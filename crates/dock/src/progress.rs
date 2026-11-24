use gtk4::cairo::{Context, FontSlant, FontWeight, LineCap, LineJoin, LinearGradient};
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{DrawingArea, Overlay};
use libadwaita as adw;
use malus_de_common::config::{DockAppConfig, NotificationsConfig};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tracing::debug;

const COMPLETION_HOLD_MS: f64 = 240.0;
const COMPLETION_FADE_MS: f64 = 420.0;
const COMPLETION_TOTAL_MS: f64 = COMPLETION_HOLD_MS + COMPLETION_FADE_MS;
const BADGE_POP_ANIM_MS: f64 = 240.0;
/// Margen exterior transparente del `DrawingArea` de overlays para que el globo rojo push
/// (y las sombras de los indicadores) sobresalgan limpiamente fuera del límite del icono sin recortarse.
pub const OVERLAY_BLEED_PX: i32 = 18;
const STALE_TEMP_FILE_SECS: u64 = 600;
const IDLE_GROWTH_TIMEOUT_SECS: u64 = 90;
const TOTAL_BYTES_FAST_RETRY_MS: u64 = 500;
const TOTAL_BYTES_SLOW_RETRY_MS: u64 = 1500;
const MAX_TOTAL_BYTES_ATTEMPTS: u8 = 120;

#[derive(Clone, Debug, PartialEq)]
pub struct LauncherEntryState {
    pub app_uri: String,
    pub sender: Option<String>,
    pub progress: f64,
    pub indeterminate: bool,
    pub progress_visible: bool,
    pub completed_at: Option<Instant>,
    pub count: i64,
    pub count_visible: bool,
    pub urgent: bool,
    pub badge_updated_at: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PushBadgeState {
    pub desktop_entry: String,
    pub app_name: String,
    pub app_icon: String,
    pub count: u64,
    pub urgent: bool,
    pub updated_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressVisual {
    pub progress: f64,
    pub indeterminate: bool,
    pub phase: f64,
    pub completed: bool,
    pub alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BadgeVisual {
    pub count: u64,
    pub scale: f64,
    pub urgent: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct UnityUpdatePayload {
    app_uri: String,
    progress: Option<f64>,
    progress_visible: Option<bool>,
    count: Option<i64>,
    count_visible: Option<bool>,
    urgent: Option<bool>,
}

#[derive(Clone, Debug)]
struct TrackedFsDownload {
    path: PathBuf,
    target_path: Option<PathBuf>,
    app_uri: String,
    current_bytes: u64,
    total_bytes: Option<u64>,
    last_growth: Instant,
    last_total_lookup: Option<Instant>,
    total_lookup_attempts: u8,
    lookup_in_flight: bool,
}

#[derive(Clone, Debug)]
struct TotalBytesLookupResult {
    path: PathBuf,
    total_bytes: Option<u64>,
    target_path: Option<PathBuf>,
}

#[derive(Default)]
struct ProgressStore {
    entries: HashMap<String, LauncherEntryState>,
    fs_entries: HashMap<String, LauncherEntryState>,
    push_badges: HashMap<String, PushBadgeState>,
    active_app_identities: Vec<String>,
    notif_config: NotificationsConfig,
    notif_config_loaded: bool,
    notif_config_monitor: Option<gio::FileMonitor>,
    fs_downloads: HashMap<PathBuf, TrackedFsDownload>,
    fs_monitors: Vec<gio::FileMonitor>,
    download_dirs: Vec<PathBuf>,
    overlays: Vec<glib::WeakRef<DrawingArea>>,
    dbus_conn: Option<gio::DBusConnection>,
    dbus_subs: Vec<gio::SignalSubscriptionId>,
    anim_timer_active: bool,
    fs_poll_active: bool,
    started_at: Option<Instant>,
    pending_total_results: Option<Arc<Mutex<Vec<TotalBytesLookupResult>>>>,
}

thread_local! {
    static PROGRESS_STORE: RefCell<ProgressStore> = RefCell::new(ProgressStore::default());
}

/// Normaliza un URI `application://foo.desktop` a su identificador base en minúsculas.
pub fn normalize_launcher_uri(app_uri: &str) -> String {
    let trimmed = app_uri.trim();
    let without_scheme = trimmed
        .strip_prefix("application://")
        .or_else(|| trimmed.strip_prefix("APPLICATION://"))
        .unwrap_or(trimmed);
    let without_ext = without_scheme
        .strip_suffix(".desktop")
        .or_else(|| without_scheme.strip_suffix(".DESKTOP"))
        .unwrap_or(without_scheme);
    without_ext.trim().to_ascii_lowercase()
}

fn is_generic_uri_segment(seg: &str) -> bool {
    matches!(
        seg,
        "" | "com" | "org" | "net" | "io" | "dev" | "app" | "stable" | "browser" | "mozilla"
            | "gnome" | "kde" | "linux" | "desktop"
    )
}

/// Extrae tokens candidatos de coincidencia a partir de un identificador `.desktop` normalizado.
fn identity_tokens(normalized_id: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    if normalized_id.is_empty() {
        return tokens;
    }
    let mut push_unique = |val: &str| {
        if !val.is_empty() && !tokens.iter().any(|t: &String| t == val) {
            tokens.push(val.to_string());
        }
    };
    push_unique(normalized_id);
    for dot_seg in normalized_id.split('.') {
        if !is_generic_uri_segment(dot_seg) && dot_seg.len() >= 3 {
            push_unique(dot_seg);
        }
        if let Some(first_dash) = dot_seg.split('-').next() {
            if first_dash.len() >= 3 && !is_generic_uri_segment(first_dash) {
                push_unique(first_dash);
            }
        }
    }
    if let Some(first_dash) = normalized_id.split('-').next() {
        if first_dash.len() >= 3 && !is_generic_uri_segment(first_dash) {
            push_unique(first_dash);
        }
    }
    tokens
}

fn command_candidates(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return out;
    }
    // Soportar "flatpak run org.mozilla.firefox" además de ejecutables directos
    let primary = if parts
        .first()
        .is_some_and(|p| p.ends_with("flatpak"))
        && parts.get(1).copied() == Some("run")
    {
        parts
            .iter()
            .skip(2)
            .find(|tok| !tok.starts_with('-'))
            .copied()
            .unwrap_or("")
    } else {
        parts[0]
    };
    let basename = primary.rsplit('/').next().unwrap_or(primary);
    let low = basename.trim().to_ascii_lowercase();
    if !low.is_empty() {
        out.extend(identity_tokens(&low));
    }
    out
}

/// Comprueba si una entrada `com.canonical.Unity.LauncherEntry` o notificación corresponde a un icono del Dock.
pub fn entry_matches_dock_app(
    app_uri: &str,
    matchers: &[String],
    app_name: &str,
    command: &str,
    icon_name: &str,
) -> bool {
    let norm_uri = normalize_launcher_uri(app_uri);
    if norm_uri.is_empty() {
        return false;
    }
    let uri_tokens = identity_tokens(&norm_uri);

    let mut targets: Vec<String> = Vec::new();
    for m in matchers {
        let low = normalize_launcher_uri(m);
        if !low.is_empty() {
            targets.extend(identity_tokens(&low));
        }
    }
    let name_low = app_name.trim().to_ascii_lowercase();
    if !name_low.is_empty() {
        targets.push(name_low.clone());
        for word in name_low.split(|c: char| !c.is_ascii_alphanumeric()) {
            if word.len() >= 3 && !is_generic_uri_segment(word) {
                targets.push(word.to_string());
            }
        }
    }
    targets.extend(command_candidates(command));
    let icon_low = normalize_launcher_uri(icon_name);
    if !icon_low.is_empty() && !icon_low.starts_with('/') {
        targets.extend(identity_tokens(&icon_low));
    }

    for target in &targets {
        if target.len() < 3 || is_generic_uri_segment(target) {
            continue;
        }
        for tok in &uri_tokens {
            if tok == target || norm_uri == *target {
                return true;
            }
            if tok.len() >= 4 && target.len() >= 4 && (tok.contains(target) || target.contains(tok))
            {
                return true;
            }
        }
    }
    false
}

fn push_badge_matches_dock_app(
    badge: &PushBadgeState,
    matchers: &[String],
    app_name: &str,
    command: &str,
    icon_name: &str,
) -> bool {
    [
        badge.desktop_entry.as_str(),
        badge.app_name.as_str(),
        badge.app_icon.as_str(),
    ]
    .into_iter()
    .filter(|candidate| !candidate.trim().is_empty())
    .any(|candidate| entry_matches_dock_app(candidate, matchers, app_name, command, icon_name))
}

fn identity_list_matches_push_source(
    active_identities: &[String],
    desktop_entry: &str,
    app_name: &str,
    app_icon: &str,
) -> bool {
    if active_identities.is_empty() {
        return false;
    }
    let candidates = [desktop_entry, app_name, app_icon];
    for active in active_identities {
        if active.trim().is_empty() {
            continue;
        }
        let active_slice = [active.clone()];
        for cand in candidates {
            if !cand.trim().is_empty()
                && entry_matches_dock_app(cand, &active_slice, active, active, "")
            {
                return true;
            }
        }
    }
    false
}

/// Extrae propiedades de un parámetro D-Bus `(sa{sv})` de `com.canonical.Unity.LauncherEntry.Update`.
fn parse_unity_update_variant(params: &glib::Variant) -> Option<UnityUpdatePayload> {
    if params.n_children() < 2 {
        return None;
    }
    let app_uri = params
        .try_child_get::<String>(0)
        .ok()
        .flatten()
        .or_else(|| params.child_value(0).str().map(str::to_string))?;
    let props_var = params.child_value(1);
    let props = glib::VariantDict::new(Some(&props_var));
    let progress = props
        .lookup::<f64>("progress")
        .ok()
        .flatten()
        .or_else(|| props.lookup::<i64>("progress").ok().flatten().map(|v| v as f64));
    let progress_visible = props.lookup::<bool>("progress-visible").ok().flatten();
    let count = props
        .lookup::<i64>("count")
        .ok()
        .flatten()
        .or_else(|| props.lookup::<i32>("count").ok().flatten().map(|v| v as i64))
        .or_else(|| props.lookup::<u64>("count").ok().flatten().map(|v| v as i64))
        .or_else(|| props.lookup::<u32>("count").ok().flatten().map(|v| v as i64));
    let count_visible = props.lookup::<bool>("count-visible").ok().flatten();
    let urgent = props.lookup::<bool>("urgent").ok().flatten();
    if progress.is_none()
        && progress_visible.is_none()
        && count.is_none()
        && count_visible.is_none()
        && urgent.is_none()
    {
        return None;
    }
    Some(UnityUpdatePayload {
        app_uri,
        progress,
        progress_visible,
        count,
        count_visible,
        urgent,
    })
}

/// Extrae `(desktop_entry, app_name, app_icon, is_replacement, urgency)` de la señal
/// D-Bus `dev.malus.Notifications.AppNotificationReceived` (`(sssby)`).
fn parse_push_notification_variant(
    params: &glib::Variant,
) -> Option<(String, String, String, bool, u8)> {
    if params.n_children() < 5 {
        return None;
    }
    let desktop_entry = params
        .try_child_get::<String>(0)
        .ok()
        .flatten()
        .unwrap_or_default();
    let app_name = params
        .try_child_get::<String>(1)
        .ok()
        .flatten()
        .unwrap_or_default();
    let app_icon = params
        .try_child_get::<String>(2)
        .ok()
        .flatten()
        .unwrap_or_default();
    let is_replacement = params
        .try_child_get::<bool>(3)
        .ok()
        .flatten()
        .unwrap_or(false);
    let urgency = params.try_child_get::<u8>(4).ok().flatten().unwrap_or(1);
    if desktop_entry.trim().is_empty() && app_name.trim().is_empty() && app_icon.trim().is_empty() {
        return None;
    }
    Some((desktop_entry, app_name, app_icon, is_replacement, urgency))
}

/// Aplica una actualización `Update(app_uri, props)` sobre el mapa de estado.
/// Devuelve `true` si se inició animación (fade-out de descarga o pop-in del globo push).
fn apply_launcher_update(
    entries: &mut HashMap<String, LauncherEntryState>,
    payload: &UnityUpdatePayload,
    sender: Option<String>,
    now: Instant,
) -> bool {
    let key = normalize_launcher_uri(&payload.app_uri);
    if key.is_empty() {
        return false;
    }
    let prev = entries.get(&key).cloned();
    let was_progress_visible = prev.as_ref().is_some_and(|e| e.progress_visible);
    let prev_progress = prev.as_ref().map(|e| e.progress).unwrap_or(0.0);
    let prev_count = prev.as_ref().map(|e| e.count).unwrap_or(0);
    let prev_count_visible = prev.as_ref().is_some_and(|e| e.count_visible);
    let prev_urgent = prev.as_ref().is_some_and(|e| e.urgent);

    let next_progress = payload
        .progress
        .map(|p| p.clamp(0.0, 1.0))
        .unwrap_or(prev_progress);
    let next_progress_visible = payload.progress_visible.unwrap_or(was_progress_visible);

    let completed_at = if !next_progress_visible && was_progress_visible && next_progress >= 0.90 {
        Some(now)
    } else if next_progress_visible {
        None
    } else {
        prev.as_ref().and_then(|e| e.completed_at)
    };

    let next_count = payload.count.unwrap_or(prev_count).max(0);
    let next_count_visible = payload.count_visible.unwrap_or(prev_count_visible);
    let next_urgent = payload.urgent.unwrap_or(prev_urgent);

    let badge_changed =
        next_count_visible && next_count > 0 && (!prev_count_visible || next_count != prev_count);
    let badge_updated_at = if badge_changed {
        Some(now)
    } else if next_count_visible && next_count > 0 {
        prev.as_ref().and_then(|e| e.badge_updated_at)
    } else {
        None
    };

    let has_any_visual = next_progress_visible
        || completed_at.is_some()
        || (next_count_visible && next_count > 0)
        || next_urgent;

    if !has_any_visual {
        entries.remove(&key);
        return false;
    }

    entries.insert(
        key,
        LauncherEntryState {
            app_uri: payload.app_uri.clone(),
            sender: sender.or_else(|| prev.and_then(|e| e.sender)),
            progress: next_progress,
            indeterminate: false,
            progress_visible: next_progress_visible,
            completed_at,
            count: next_count,
            count_visible: next_count_visible,
            urgent: next_urgent,
            badge_updated_at,
        },
    );
    completed_at.is_some() || badge_changed
}

fn push_badge_key(desktop_entry: &str, app_name: &str, app_icon: &str) -> String {
    let primary = if !desktop_entry.trim().is_empty() {
        desktop_entry
    } else if !app_name.trim().is_empty() {
        app_name
    } else {
        app_icon
    };
    normalize_launcher_uri(primary)
}

fn apply_push_notification(
    push_badges: &mut HashMap<String, PushBadgeState>,
    active_identities: &[String],
    notif_config: &NotificationsConfig,
    desktop_entry: &str,
    app_name: &str,
    app_icon: &str,
    is_replacement: bool,
    urgency: u8,
    now: Instant,
) -> bool {
    let key = push_badge_key(desktop_entry, app_name, app_icon);
    if key.is_empty() {
        return false;
    }
    if identity_list_matches_push_source(active_identities, desktop_entry, app_name, app_icon) {
        return false;
    }
    let rule_key = if !app_name.trim().is_empty() {
        app_name
    } else {
        desktop_entry
    };
    let rule = notif_config.rule_for_app(rule_key);
    if !rule.enabled || !rule.badge_icon {
        return false;
    }

    let entry = push_badges.entry(key).or_insert_with(|| PushBadgeState {
        desktop_entry: desktop_entry.to_string(),
        app_name: app_name.to_string(),
        app_icon: app_icon.to_string(),
        count: 0,
        urgent: false,
        updated_at: now,
    });
    if !desktop_entry.trim().is_empty() {
        entry.desktop_entry = desktop_entry.to_string();
    }
    if !app_name.trim().is_empty() {
        entry.app_name = app_name.to_string();
    }
    if !app_icon.trim().is_empty() {
        entry.app_icon = app_icon.to_string();
    }
    if entry.count == 0 || !is_replacement {
        entry.count = entry.count.saturating_add(1);
    }
    if urgency >= 2 {
        entry.urgent = true;
    }
    entry.updated_at = now;
    true
}

fn clear_entries_for_sender(
    entries: &mut HashMap<String, LauncherEntryState>,
    sender: &str,
) -> bool {
    let before = entries.len();
    entries.retain(|_, state| state.sender.as_deref() != Some(sender));
    entries.len() != before
}

fn badge_pop_scale(elapsed_ms: f64) -> f64 {
    if elapsed_ms < 0.0 || elapsed_ms >= BADGE_POP_ANIM_MS {
        return 1.0;
    }
    let t = (elapsed_ms / BADGE_POP_ANIM_MS).clamp(0.0, 1.0);
    if t < 0.58 {
        // Rebote rápido de entrada: 0.35 -> 1.14
        let u = t / 0.58;
        let eased = (u * FRAC_PI_2).sin();
        0.35 + (1.14 - 0.35) * eased
    } else {
        // Asentamiento suave: 1.14 -> 1.00
        let u = (t - 0.58) / 0.42;
        let eased = 0.5 - 0.5 * (u * PI).cos();
        1.14 - 0.14 * eased
    }
}

pub fn format_badge_count(count: u64) -> String {
    if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    }
}

fn app_allows_dock_badge(notif_config: &NotificationsConfig, app_name: &str, matchers: &[String]) -> bool {
    let primary = notif_config.rule_for_app(app_name);
    if !primary.enabled || !primary.badge_icon {
        return false;
    }
    for m in matchers {
        if m.trim().is_empty() {
            continue;
        }
        let key = NotificationsConfig::normalize_app_key(m);
        if let Some(rule) = notif_config.app_rules.get(&key) {
            if !rule.enabled || !rule.badge_icon {
                return false;
            }
        }
    }
    true
}

fn compute_badge_visual_for_app(
    entries: &HashMap<String, LauncherEntryState>,
    push_badges: &HashMap<String, PushBadgeState>,
    notif_config: &NotificationsConfig,
    matchers: &[String],
    app_name: &str,
    command: &str,
    icon_name: &str,
    now: Instant,
) -> Option<BadgeVisual> {
    if !app_allows_dock_badge(notif_config, app_name, matchers) {
        return None;
    }

    // 1. Prioridad: contador explícito reportado por la app vía `com.canonical.Unity.LauncherEntry`
    let mut best_unity: Option<(u64, Option<Instant>, bool)> = None;
    for state in entries.values() {
        if !state.count_visible || state.count <= 0 {
            continue;
        }
        if !entry_matches_dock_app(&state.app_uri, matchers, app_name, command, icon_name) {
            continue;
        }
        let c = state.count as u64;
        if best_unity.as_ref().is_none_or(|(prev_c, _, _)| c >= *prev_c) {
            best_unity = Some((c, state.badge_updated_at, state.urgent));
        }
    }
    if let Some((count, updated_at, urgent)) = best_unity {
        let scale = updated_at
            .map(|t| badge_pop_scale(now.saturating_duration_since(t).as_secs_f64() * 1000.0))
            .unwrap_or(1.0);
        return Some(BadgeVisual {
            count,
            scale,
            urgent,
        });
    }

    // 2. Contador acumulado desde `malus-de-notifications` (`dev.malus.Notifications`)
    let mut total_push = 0u64;
    let mut latest_update: Option<Instant> = None;
    let mut any_urgent = false;
    for badge in push_badges.values() {
        if badge.count == 0 {
            continue;
        }
        if !push_badge_matches_dock_app(badge, matchers, app_name, command, icon_name) {
            continue;
        }
        total_push = total_push.saturating_add(badge.count);
        any_urgent |= badge.urgent;
        latest_update = match latest_update {
            None => Some(badge.updated_at),
            Some(prev) => Some(prev.max(badge.updated_at)),
        };
    }

    if total_push > 0 {
        let scale = latest_update
            .map(|t| badge_pop_scale(now.saturating_duration_since(t).as_secs_f64() * 1000.0))
            .unwrap_or(1.0);
        return Some(BadgeVisual {
            count: total_push,
            scale,
            urgent: any_urgent,
        });
    }

    None
}

fn compute_visual_from_sources<'a>(
    sources: impl Iterator<Item = &'a LauncherEntryState>,
    matchers: &[String],
    app_name: &str,
    command: &str,
    icon_name: &str,
    now: Instant,
    phase: f64,
) -> Option<ProgressVisual> {
    let mut best: Option<ProgressVisual> = None;
    for state in sources {
        if !entry_matches_dock_app(&state.app_uri, matchers, app_name, command, icon_name) {
            continue;
        }
        let candidate = if state.progress_visible {
            Some(ProgressVisual {
                progress: state.progress.clamp(0.0, 1.0),
                indeterminate: state.indeterminate,
                phase,
                completed: false,
                alpha: 1.0,
            })
        } else if let Some(done_at) = state.completed_at {
            let elapsed_ms = now.saturating_duration_since(done_at).as_secs_f64() * 1000.0;
            if elapsed_ms < COMPLETION_TOTAL_MS {
                let alpha = if elapsed_ms <= COMPLETION_HOLD_MS {
                    1.0
                } else {
                    (1.0 - (elapsed_ms - COMPLETION_HOLD_MS) / COMPLETION_FADE_MS).clamp(0.0, 1.0)
                };
                Some(ProgressVisual {
                    progress: 1.0,
                    indeterminate: false,
                    phase: 0.0,
                    completed: true,
                    alpha,
                })
            } else {
                None
            }
        } else {
            None
        };

        if let Some(vis) = candidate {
            best = match best {
                None => Some(vis),
                Some(cur) => {
                    if !vis.completed && cur.completed {
                        Some(vis)
                    } else if vis.completed == cur.completed {
                        // Preferir progreso determinado sobre indeterminado
                        if !vis.indeterminate && cur.indeterminate {
                            Some(vis)
                        } else if vis.indeterminate == cur.indeterminate
                            && vis.progress >= cur.progress
                        {
                            Some(vis)
                        } else {
                            Some(cur)
                        }
                    } else {
                        Some(cur)
                    }
                }
            };
        }
    }
    best
}

fn queue_redraw_all_overlays() {
    PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        store.overlays.retain(|w| {
            if let Some(area) = w.upgrade() {
                area.queue_draw();
                true
            } else {
                false
            }
        });
    });
}

fn ensure_animation_timer() {
    let should_start = PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        if store.started_at.is_none() {
            store.started_at = Some(Instant::now());
        }
        if store.anim_timer_active {
            false
        } else {
            store.anim_timer_active = true;
            true
        }
    });
    if !should_start {
        return;
    }

    glib::timeout_add_local(Duration::from_millis(24), || {
        let now = Instant::now();
        let keep_running = PROGRESS_STORE.with(|cell| {
            let mut store = cell.borrow_mut();
            let retain_fn = |_: &String, state: &mut LauncherEntryState| {
                let keep_done = state.completed_at.is_some_and(|done_at| {
                    now.saturating_duration_since(done_at).as_secs_f64() * 1000.0
                        < COMPLETION_TOTAL_MS
                });
                if !keep_done {
                    state.completed_at = None;
                }
                state.progress_visible
                    || state.completed_at.is_some()
                    || (state.count_visible && state.count > 0)
                    || state.urgent
            };
            store.entries.retain(retain_fn);
            store.fs_entries.retain(retain_fn);

            let progress_needs_anim = store
                .entries
                .values()
                .chain(store.fs_entries.values())
                .any(|s| {
                    (!s.progress_visible && s.completed_at.is_some())
                        || (s.progress_visible && s.indeterminate)
                });
            let unity_badge_anim = store.entries.values().any(|s| {
                s.count_visible
                    && s.count > 0
                    && s.badge_updated_at.is_some_and(|t| {
                        now.saturating_duration_since(t).as_secs_f64() * 1000.0 < BADGE_POP_ANIM_MS
                    })
            });
            let push_badge_anim = store.push_badges.values().any(|b| {
                b.count > 0
                    && now.saturating_duration_since(b.updated_at).as_secs_f64() * 1000.0
                        < BADGE_POP_ANIM_MS
            });
            let needs_anim = progress_needs_anim || unity_badge_anim || push_badge_anim;
            if !needs_anim {
                store.anim_timer_active = false;
            }
            needs_anim
        });
        queue_redraw_all_overlays();
        if keep_running {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

/// Actualiza la ventana enfocada actualmente y limpia los globos push acumulados de esa app
/// (comportamiento nativo macOS al cambiar o enfocar la aplicación).
pub fn notify_active_app_changed(active_identities: &[String]) {
    let changed = PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        store.active_app_identities = active_identities.to_vec();
        let before = store.push_badges.len();
        store.push_badges.retain(|_, badge| {
            !identity_list_matches_push_source(
                active_identities,
                &badge.desktop_entry,
                &badge.app_name,
                &badge.app_icon,
            )
        });
        store.push_badges.len() != before
    });
    if changed {
        queue_redraw_all_overlays();
    }
}

/// Limpia los globos de notificaciones push de una aplicación concreta del Dock (al hacer clic o desde el menú).
pub fn clear_push_badges_for_dock_app(app_cfg: &DockAppConfig) {
    let changed = PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        let before_push = store.push_badges.len();
        store.push_badges.retain(|_, badge| {
            !push_badge_matches_dock_app(
                badge,
                &app_cfg.matchers,
                &app_cfg.name,
                &app_cfg.command,
                &app_cfg.icon,
            )
        });
        let mut unity_cleared = false;
        for state in store.entries.values_mut() {
            if state.count_visible
                && entry_matches_dock_app(
                    &state.app_uri,
                    &app_cfg.matchers,
                    &app_cfg.name,
                    &app_cfg.command,
                    &app_cfg.icon,
                )
            {
                state.count = 0;
                state.count_visible = false;
                state.urgent = false;
                unity_cleared = true;
            }
        }
        store.push_badges.len() != before_push || unity_cleared
    });
    if changed {
        queue_redraw_all_overlays();
    }
}

/// Devuelve el contador actual del globo de notificaciones visible para una app del Dock.
pub fn badge_count_for_dock_app(app_cfg: &DockAppConfig) -> u64 {
    PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        if !store.notif_config_loaded {
            store.notif_config = NotificationsConfig::load();
            store.notif_config_loaded = true;
        }
        compute_badge_visual_for_app(
            &store.entries,
            &store.push_badges,
            &store.notif_config,
            &app_cfg.matchers,
            &app_cfg.name,
            &app_cfg.command,
            &app_cfg.icon,
            Instant::now(),
        )
        .map(|b| b.count)
        .unwrap_or(0)
    })
}

fn ensure_notifications_config_monitor() {
    PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        if !store.notif_config_loaded {
            store.notif_config = NotificationsConfig::load();
            store.notif_config_loaded = true;
        }
        if store.notif_config_monitor.is_some() {
            return;
        }
        let path = NotificationsConfig::config_path();
        let gfile = gio::File::for_path(&path);
        if let Ok(monitor) =
            gfile.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            monitor.connect_changed(|_, _, _, _| {
                PROGRESS_STORE.with(|c| {
                    let mut st = c.borrow_mut();
                    st.notif_config = NotificationsConfig::load();
                    st.notif_config_loaded = true;
                });
                queue_redraw_all_overlays();
            });
            store.notif_config_monitor = Some(monitor);
        }
    });
}

// ============================================================================
// Detección nativa de descargas activas de navegadores (.crdownload / .part)
// ============================================================================

fn parse_xdg_download_dir_from_file(path: &Path, home: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("XDG_DOWNLOAD_DIR=") {
            let unquoted = val.trim().trim_matches('"').trim_matches('\'');
            let resolved = if let Some(rest) = unquoted.strip_prefix("$HOME/") {
                home.join(rest)
            } else if unquoted == "$HOME" {
                home.to_path_buf()
            } else {
                PathBuf::from(unquoted)
            };
            if resolved.is_dir() && resolved != home {
                return Some(resolved);
            }
        }
    }
    None
}

fn discover_download_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if p.is_dir() && !dirs.iter().any(|existing: &PathBuf| existing == &p) {
            dirs.push(p);
        }
    };

    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(special) = glib::user_special_dir(glib::UserDirectory::Downloads) {
        if home.as_deref() != Some(special.as_path()) {
            push_unique(special);
        }
    }
    if let Some(ref home_dir) = home {
        for user_dirs in [
            home_dir.join(".config/user-dirs.dirs"),
            home_dir.join(".config/malus-de/user-dirs.dirs"),
        ] {
            if let Some(parsed) = parse_xdg_download_dir_from_file(&user_dirs, home_dir) {
                push_unique(parsed);
            }
        }
        push_unique(home_dir.join("Descargas"));
        push_unique(home_dir.join("Downloads"));
    }
    dirs
}

fn is_browser_temp_download_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".crdownload") || lower.ends_with(".part")
}

fn browser_uri_from_process_identity(identity: &str) -> Option<&'static str> {
    let low = identity.to_ascii_lowercase();
    if low.contains("brave-origin") || low.contains("brave_origin") {
        Some("application://brave-origin.desktop")
    } else if low.contains("brave") {
        Some("application://brave-browser.desktop")
    } else if low.contains("firefox") {
        Some("application://org.mozilla.firefox.desktop")
    } else if low.contains("librewolf") {
        Some("application://io.gitlab.librewolf-community.desktop")
    } else if low.contains("zen-bin") || low.contains("zen-browser") {
        Some("application://app.zen_browser.zen.desktop")
    } else if low.contains("google-chrome") {
        Some("application://google-chrome.desktop")
    } else if low.contains("chromium") {
        Some("application://chromium.desktop")
    } else if low.contains("vivaldi") {
        Some("application://vivaldi-stable.desktop")
    } else if low.contains("msedge") || low.contains("microsoft-edge") {
        Some("application://microsoft-edge.desktop")
    } else if low.contains("opera") {
        Some("application://opera.desktop")
    } else {
        None
    }
}

fn browser_display_meta(app_uri: &str) -> (&'static str, &'static str, &'static str) {
    let norm = normalize_launcher_uri(app_uri);
    if norm.contains("brave-origin") {
        ("Brave Origin", "brave-origin", "brave-origin")
    } else if norm.contains("brave") {
        ("Brave", "brave-browser", "brave-browser")
    } else if norm.contains("firefox") {
        ("Firefox", "firefox", "org.mozilla.firefox")
    } else if norm.contains("librewolf") {
        ("LibreWolf", "librewolf", "io.gitlab.librewolf-community")
    } else if norm.contains("zen") {
        ("Zen Browser", "zen", "app.zen_browser.zen")
    } else if norm.contains("google-chrome") {
        ("Google Chrome", "google-chrome", "google-chrome")
    } else if norm.contains("chromium") {
        ("Chromium", "chromium", "chromium")
    } else if norm.contains("vivaldi") {
        ("Vivaldi", "vivaldi", "vivaldi-stable")
    } else if norm.contains("edge") {
        ("Microsoft Edge", "microsoft-edge", "microsoft-edge")
    } else if norm.contains("opera") {
        ("Opera", "opera", "opera")
    } else {
        ("Descargas", "folder-download-symbolic", "malus-de-dock")
    }
}

fn format_download_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// Inspecciona `/proc/<pid>/fd` únicamente en procesos de navegadores del usuario actual
/// para identificar con precisión qué navegador mantiene abierto `target_path`.
fn detect_browser_uri_holding_file(target_path: &Path) -> Option<String> {
    let target_canon =
        std::fs::canonicalize(target_path).unwrap_or_else(|_| target_path.to_path_buf());
    let proc_entries = std::fs::read_dir("/proc").ok()?;
    for entry in proc_entries.flatten() {
        let fname = entry.file_name();
        let Some(pid_str) = fname.to_str() else {
            continue;
        };
        if !pid_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let proc_dir = entry.path();
        let comm = std::fs::read_to_string(proc_dir.join("comm")).unwrap_or_default();
        let cmdline_bytes = std::fs::read(proc_dir.join("cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline_bytes).replace('\0', " ");
        let combined = format!("{comm} {cmdline}");
        let Some(uri) = browser_uri_from_process_identity(&combined) else {
            continue;
        };
        let Ok(fd_dir) = std::fs::read_dir(proc_dir.join("fd")) else {
            continue;
        };
        for fd in fd_dir.flatten() {
            if let Ok(link) = std::fs::read_link(fd.path()) {
                if link == target_path || link == target_canon {
                    return Some(uri.to_string());
                }
            }
        }
    }
    None
}

fn detect_running_chromium_family_uri() -> Option<String> {
    let proc_entries = std::fs::read_dir("/proc").ok()?;
    for entry in proc_entries.flatten() {
        let fname = entry.file_name();
        let Some(pid_str) = fname.to_str() else {
            continue;
        };
        if !pid_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let proc_dir = entry.path();
        let cmdline_bytes = std::fs::read(proc_dir.join("cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline_bytes).replace('\0', " ");
        if let Some(uri) = browser_uri_from_process_identity(&cmdline) {
            if !uri.contains("firefox") && !uri.contains("librewolf") && !uri.contains("zen") {
                return Some(uri.to_string());
            }
        }
    }
    None
}

fn infer_browser_uri_for_download(path: &Path) -> String {
    if let Some(uri) = detect_browser_uri_holding_file(path) {
        return uri;
    }
    let lower = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if lower.ends_with(".part") {
        "application://org.mozilla.firefox.desktop".to_string()
    } else {
        detect_running_chromium_family_uri()
            .unwrap_or_else(|| "application://brave-browser.desktop".to_string())
    }
}

fn push_chromium_profile_histories(base: &Path, dbs: &mut Vec<PathBuf>) {
    if !base.is_dir() {
        return;
    }
    let default_hist = base.join("Default/History");
    if default_hist.is_file() && !dbs.contains(&default_hist) {
        dbs.push(default_hist);
    }
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if s.starts_with("Profile ") {
                let hist = entry.path().join("History");
                if hist.is_file() && !dbs.contains(&hist) {
                    dbs.push(hist);
                }
            }
        }
    }
}

fn candidate_chromium_history_dbs() -> Vec<PathBuf> {
    let mut dbs = Vec::new();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return dbs;
    };
    let mut config_roots = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        config_roots.push(xdg);
    }
    for extra in [home.join(".config/malus-de"), home.join(".config")] {
        if !config_roots.contains(&extra) {
            config_roots.push(extra);
        }
    }

    // 1. Subcarpetas dinámicas dentro de BraveSoftware/* (Brave-Origin, Brave-Browser, etc.)
    for root in &config_roots {
        let brave_vendor = root.join("BraveSoftware");
        if let Ok(products) = std::fs::read_dir(&brave_vendor) {
            for prod in products.flatten() {
                push_chromium_profile_histories(&prod.path(), &mut dbs);
            }
        }
        for direct in [
            "google-chrome",
            "google-chrome-beta",
            "google-chrome-unstable",
            "chromium",
            "vivaldi",
            "vivaldi-snapshot",
            "microsoft-edge",
            "opera",
            "thorium",
        ] {
            push_chromium_profile_histories(&root.join(direct), &mut dbs);
        }
    }

    // 2. Perfiles Flatpak de navegadores Chromium
    let flatpak_roots = [
        home.join(".var/app/com.brave.Browser/config/BraveSoftware"),
        home.join(".var/app/com.google.Chrome/config"),
        home.join(".var/app/org.chromium.Chromium/config"),
    ];
    for fp_root in &flatpak_roots {
        if let Ok(products) = std::fs::read_dir(fp_root) {
            for prod in products.flatten() {
                push_chromium_profile_histories(&prod.path(), &mut dbs);
            }
        }
    }
    dbs
}

/// Consulta `(total_bytes, target_path)` en la base `History` de navegadores Chromium/Brave.
/// Intenta primero lectura `mode=ro` (con WAL / transacciones recién confirmadas) y hace fallback
/// a `mode=ro&immutable=1` si la base estuviera momentáneamente bloqueada.
fn query_chromium_download_info(download_path: &Path) -> (Option<u64>, Option<PathBuf>) {
    let dbs = candidate_chromium_history_dbs();
    if dbs.is_empty() {
        return (None, None);
    }
    let Some(path_str) = download_path.to_str() else {
        return (None, None);
    };
    let target_str = path_str.strip_suffix(".crdownload").unwrap_or(path_str);
    let basename = download_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let py = r#"
import json, sqlite3, sys
cur_path, tgt_path, base = sys.argv[1], sys.argv[2], sys.argv[3]
like_pat = "%/" + base if base else ""
for db in sys.argv[4:]:
    for suffix in ("mode=ro", "mode=ro&immutable=1"):
        con = None
        try:
            con = sqlite3.connect(f"file:{db}?{suffix}", uri=True, timeout=0.15)
            row = con.execute(
                "SELECT total_bytes, target_path FROM downloads "
                "WHERE current_path = ? OR target_path = ? OR (? != '' AND current_path LIKE ?) "
                "ORDER BY (state = 0) DESC, id DESC LIMIT 1",
                (cur_path, tgt_path, base, like_pat),
            ).fetchone()
            if not row or not row[0] or int(row[0]) <= 0:
                row = con.execute(
                    "SELECT total_bytes, target_path FROM downloads "
                    "WHERE state = 0 AND total_bytes > 0 "
                    "ORDER BY start_time DESC, id DESC LIMIT 1"
                ).fetchone()
            if row:
                tb = int(row[0]) if row[0] and int(row[0]) > 0 else None
                tp = str(row[1]) if row[1] else None
                if tb is not None or tp is not None:
                    print(json.dumps({"total_bytes": tb, "target_path": tp}))
                    sys.exit(0)
            break
        except Exception:
            continue
        finally:
            if con is not None:
                try:
                    con.close()
                except Exception:
                    pass
"#;
    let mut cmd = Command::new("python3");
    cmd.arg("-c")
        .arg(py)
        .arg(path_str)
        .arg(target_str)
        .arg(basename);
    for db in &dbs {
        cmd.arg(db);
    }
    let Ok(output) = cmd.output() else {
        return (None, None);
    };
    if !output.status.success() {
        return (None, None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Ok(val) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        return (None, None);
    };
    let total_bytes = val
        .get("total_bytes")
        .and_then(|v| v.as_u64())
        .filter(|&n| n > 0);
    let target_path = val
        .get("target_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    (total_bytes, target_path)
}

fn candidate_firefox_downloads_jsons() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return files;
    };
    let profile_parents = [
        home.join(".mozilla/firefox"),
        home.join(".config/mozilla/firefox"),
        home.join(".config/malus-de/mozilla/firefox"),
        home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
        home.join(".librewolf"),
        home.join(".zen"),
    ];
    for parent in &profile_parents {
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join("downloads.json");
            if candidate.is_file() && !files.contains(&candidate) {
                files.push(candidate);
            }
        }
    }
    files
}

fn parse_firefox_downloads_json(
    raw: &str,
    part_path: &Path,
) -> Option<(Option<u64>, Option<PathBuf>)> {
    let parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let list = parsed.get("list")?.as_array()?;
    let part_str = part_path.to_str()?;
    let stripped_str = part_str
        .strip_suffix(".part")
        .or_else(|| part_str.strip_suffix(".PART"))
        .unwrap_or(part_str);
    let part_name = part_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    for item in list.iter().rev() {
        let target = item.get("target");
        let item_part = target
            .and_then(|t| t.get("partFilePath"))
            .and_then(|v| v.as_str());
        let item_target = target.and_then(|t| t.get("path")).and_then(|v| v.as_str());
        let matches = item_part == Some(part_str)
            || item_target == Some(stripped_str)
            || item_part.is_some_and(|p| !part_name.is_empty() && p.ends_with(part_name));
        if matches {
            let total_bytes = item
                .get("totalBytes")
                .and_then(|v| v.as_u64())
                .filter(|&n| n > 0);
            let target_path = item_target.map(PathBuf::from);
            if total_bytes.is_some() || target_path.is_some() {
                return Some((total_bytes, target_path));
            }
        }
    }
    None
}

fn query_firefox_download_info(part_path: &Path) -> (Option<u64>, Option<PathBuf>) {
    for json_path in candidate_firefox_downloads_jsons() {
        if let Ok(raw) = std::fs::read_to_string(&json_path) {
            if let Some(found) = parse_firefox_downloads_json(&raw, part_path) {
                return found;
            }
        }
    }
    (None, None)
}

fn spawn_total_bytes_lookup(path: PathBuf, sink: Arc<Mutex<Vec<TotalBytesLookupResult>>>) {
    std::thread::spawn(move || {
        let lower_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let (total_bytes, target_path) = if lower_name.ends_with(".crdownload") {
            query_chromium_download_info(&path)
        } else if lower_name.ends_with(".part") {
            query_firefox_download_info(&path)
        } else {
            (None, None)
        };
        if let Ok(mut guard) = sink.lock() {
            guard.push(TotalBytesLookupResult {
                path,
                total_bytes,
                target_path,
            });
        }
    });
}

/// Si la descarga temporal desapareció porque terminó correctamente, devuelve la ruta y tamaño
/// del archivo final en disco. Devuelve `None` si fue cancelada.
fn resolve_finished_download_file(
    temp_path: &Path,
    known_target: Option<&Path>,
    now_sys: SystemTime,
) -> Option<(PathBuf, u64)> {
    // 1. Ruta destino exacta obtenida desde History / downloads.json
    if let Some(target) = known_target {
        if let Ok(meta) = std::fs::metadata(target) {
            if meta.is_file() && meta.len() > 0 {
                return Some((target.to_path_buf(), meta.len()));
            }
        }
    }

    // 2. `archivo.iso.part` o `archivo.iso.crdownload` -> `archivo.iso`
    if let Some(file_str) = temp_path.to_str() {
        for suffix in [".crdownload", ".CRDOWNLOAD", ".part", ".PART"] {
            if let Some(stripped) = file_str.strip_suffix(suffix) {
                let final_path = Path::new(stripped);
                if let Ok(meta) = std::fs::metadata(final_path) {
                    if meta.is_file() && meta.len() > 0 {
                        return Some((final_path.to_path_buf(), meta.len()));
                    }
                }
            }
        }
    }

    // 3. Chromium usa `Unconfirmed 12345.crdownload` y lo renombra al archivo final en la misma carpeta
    let parent = temp_path.parent()?;
    let entries = std::fs::read_dir(parent).ok()?;
    let mut newest: Option<(SystemTime, PathBuf, u64)> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if is_browser_temp_download_file(&p) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() == 0 {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if now_sys
            .duration_since(modified)
            .map(|d| d.as_secs() <= 4)
            .unwrap_or(false)
            && newest.as_ref().is_none_or(|(prev_t, _, _)| modified > *prev_t)
        {
            newest = Some((modified, p, meta.len()));
        }
    }
    newest.map(|(_, path, size)| (path, size))
}

fn send_download_completed_notification(app_uri: &str, final_path: &Path, size_bytes: u64) {
    let (app_name, app_icon, desktop_entry) = browser_display_meta(app_uri);
    let filename = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Archivo descargado");
    let body = if size_bytes > 0 {
        format!("{} ({})", filename, format_download_size(size_bytes))
    } else {
        filename.to_string()
    };

    let bus = PROGRESS_STORE
        .with(|cell| cell.borrow().dbus_conn.clone())
        .or_else(|| gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok());
    let Some(bus) = bus else {
        return;
    };

    let hints = glib::VariantDict::new(None);
    hints.insert("desktop-entry", desktop_entry);
    hints.insert("category", "transfer.complete");
    let actions: Vec<String> = Vec::new();
    let params = glib::Variant::tuple_from_iter([
        app_name.to_variant(),
        0u32.to_variant(),
        app_icon.to_variant(),
        "Descarga completada".to_variant(),
        body.to_variant(),
        actions.to_variant(),
        hints.end(),
        5000i32.to_variant(),
    ]);

    bus.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
        |_| {},
    );
}

/// Reconstruye el mapa `fs_entries` a partir de las descargas activas en disco.
fn rebuild_fs_entries_from_tracked(
    fs_downloads: &HashMap<PathBuf, TrackedFsDownload>,
    fs_entries: &mut HashMap<String, LauncherEntryState>,
    completed_uris: &[String],
    now: Instant,
) -> bool {
    let mut needs_anim = false;
    let mut grouped: HashMap<String, (String, u64, u64, bool)> = HashMap::new();

    for dl in fs_downloads.values() {
        let key = normalize_launcher_uri(&dl.app_uri);
        if key.is_empty() {
            continue;
        }
        let entry = grouped
            .entry(key)
            .or_insert_with(|| (dl.app_uri.clone(), 0, 0, true));
        entry.1 = entry.1.saturating_add(dl.current_bytes);
        if let Some(tot) = dl.total_bytes.filter(|&t| t > 0) {
            entry.2 = entry.2.saturating_add(tot);
        } else {
            entry.3 = false;
        }
    }

    // Actualizar o crear entradas activas
    let active_keys: HashSet<String> = grouped.keys().cloned().collect();
    for (key, (app_uri, cur, tot, all_known)) in grouped {
        let (progress, indeterminate) = if all_known && tot > 0 {
            ((cur as f64 / tot as f64).clamp(0.0, 1.0), false)
        } else {
            needs_anim = true;
            (0.0, true)
        };
        fs_entries.insert(
            key,
            LauncherEntryState {
                app_uri,
                sender: None,
                progress,
                indeterminate,
                progress_visible: true,
                completed_at: None,
                count: 0,
                count_visible: false,
                urgent: false,
                badge_updated_at: None,
            },
        );
    }

    // Marcar completadas las URIs que terminaron y ya no tienen descargas activas
    for uri in completed_uris {
        let key = normalize_launcher_uri(uri);
        if key.is_empty() || active_keys.contains(&key) {
            continue;
        }
        fs_entries.insert(
            key,
            LauncherEntryState {
                app_uri: uri.clone(),
                sender: None,
                progress: 1.0,
                indeterminate: false,
                progress_visible: false,
                completed_at: Some(now),
                count: 0,
                count_visible: false,
                urgent: false,
                badge_updated_at: None,
            },
        );
        needs_anim = true;
    }

    // Limpiar entradas que ya no están activas ni en fase de fade-out de completado
    fs_entries.retain(|key, state| {
        if active_keys.contains(key) {
            return true;
        }
        if let Some(done_at) = state.completed_at {
            if now.saturating_duration_since(done_at).as_secs_f64() * 1000.0 < COMPLETION_TOTAL_MS {
                needs_anim = true;
                return true;
            }
        }
        false
    });

    needs_anim
}

fn scan_browser_downloads_once() -> (bool, bool) {
    let now = Instant::now();
    let now_sys = SystemTime::now();

    let (has_active, needs_anim, completed_notifications) = PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();

        // 1. Aplicar resultados asíncronos de consultas `total_bytes` / `target_path`
        if let Some(sink) = store.pending_total_results.clone() {
            if let Ok(mut results) = sink.lock() {
                for res in results.drain(..) {
                    if let Some(dl) = store.fs_downloads.get_mut(&res.path) {
                        dl.lookup_in_flight = false;
                        if res.total_bytes.is_some() {
                            dl.total_bytes = res.total_bytes;
                        }
                        if res.target_path.is_some() {
                            dl.target_path = res.target_path;
                        }
                    }
                }
            }
        }

        // 2. Descubrir archivos temporales activos en los directorios de descargas
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        let dirs = store.download_dirs.clone();
        for dir in &dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_browser_temp_download_file(&path) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let is_fresh = meta
                    .modified()
                    .ok()
                    .and_then(|m| now_sys.duration_since(m).ok())
                    .map(|age| age.as_secs() <= STALE_TEMP_FILE_SECS)
                    .unwrap_or(false);
                if !is_fresh && !store.fs_downloads.contains_key(&path) {
                    continue;
                }
                let size = meta.len();
                seen_paths.insert(path.clone());

                if let Some(dl) = store.fs_downloads.get_mut(&path) {
                    if size != dl.current_bytes {
                        dl.current_bytes = size;
                        dl.last_growth = now;
                    }
                } else {
                    let app_uri = infer_browser_uri_for_download(&path);
                    debug!(
                        "Descarga de navegador detectada: {:?} -> {}",
                        path, app_uri
                    );
                    store.fs_downloads.insert(
                        path.clone(),
                        TrackedFsDownload {
                            path,
                            target_path: None,
                            app_uri,
                            current_bytes: size,
                            total_bytes: None,
                            last_growth: now,
                            last_total_lookup: None,
                            total_lookup_attempts: 0,
                            lookup_in_flight: false,
                        },
                    );
                }
            }
        }

        // 3. Detectar descargas que desaparecieron (completadas o canceladas) o quedaron abandonadas
        let mut removed_paths = Vec::new();
        let mut completed_uris = Vec::new();
        let mut completed_notifications: Vec<(String, PathBuf, u64)> = Vec::new();
        for (path, dl) in &store.fs_downloads {
            if !seen_paths.contains(path) {
                removed_paths.push(path.clone());
                if let Some((final_path, final_size)) =
                    resolve_finished_download_file(path, dl.target_path.as_deref(), now_sys)
                {
                    completed_uris.push(dl.app_uri.clone());
                    completed_notifications.push((dl.app_uri.clone(), final_path, final_size));
                }
            } else if now.saturating_duration_since(dl.last_growth).as_secs()
                > IDLE_GROWTH_TIMEOUT_SECS
                && detect_browser_uri_holding_file(path).is_none()
            {
                removed_paths.push(path.clone());
            }
        }
        for p in removed_paths {
            store.fs_downloads.remove(&p);
        }

        // 4. Lanzar consulta en segundo plano de `total_bytes` para `.crdownload` y `.part`
        let sink = store
            .pending_total_results
            .get_or_insert_with(|| Arc::new(Mutex::new(Vec::new())))
            .clone();
        for dl in store.fs_downloads.values_mut() {
            if dl.total_bytes.is_some()
                || dl.lookup_in_flight
                || dl.total_lookup_attempts >= MAX_TOTAL_BYTES_ATTEMPTS
            {
                continue;
            }
            let retry_ms = if dl.total_lookup_attempts < 24 {
                TOTAL_BYTES_FAST_RETRY_MS
            } else {
                TOTAL_BYTES_SLOW_RETRY_MS
            };
            let due = dl.last_total_lookup.is_none_or(|t| {
                now.saturating_duration_since(t).as_millis() >= retry_ms as u128
            });
            if due {
                dl.lookup_in_flight = true;
                dl.last_total_lookup = Some(now);
                dl.total_lookup_attempts += 1;
                spawn_total_bytes_lookup(dl.path.clone(), sink.clone());
            }
        }

        let ProgressStore {
            fs_downloads,
            fs_entries,
            ..
        } = &mut *store;
        let needs_anim =
            rebuild_fs_entries_from_tracked(fs_downloads, fs_entries, &completed_uris, now);
        (!fs_downloads.is_empty(), needs_anim, completed_notifications)
    });

    for (app_uri, final_path, final_size) in completed_notifications {
        send_download_completed_notification(&app_uri, &final_path, final_size);
    }

    queue_redraw_all_overlays();
    if needs_anim {
        ensure_animation_timer();
    }
    (has_active, needs_anim)
}

fn ensure_fs_poll_loop() {
    let should_start = PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        if store.fs_poll_active {
            false
        } else {
            store.fs_poll_active = true;
            true
        }
    });
    if !should_start {
        return;
    }

    glib::timeout_add_local(Duration::from_millis(200), || {
        let (has_active, _) = scan_browser_downloads_once();
        if has_active {
            glib::ControlFlow::Continue
        } else {
            PROGRESS_STORE.with(|cell| {
                cell.borrow_mut().fs_poll_active = false;
            });
            glib::ControlFlow::Break
        }
    });
}

fn start_browser_download_monitors() {
    let dirs = discover_download_directories();
    PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        if !store.fs_monitors.is_empty() {
            return;
        }
        store.download_dirs = dirs.clone();
        for dir in dirs {
            let gfile = gio::File::for_path(&dir);
            if let Ok(monitor) =
                gfile.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            {
                monitor.connect_changed(|_, file, _, _| {
                    let relevant = file.path().is_none_or(|p| {
                        is_browser_temp_download_file(&p)
                            || PROGRESS_STORE.with(|c| !c.borrow().fs_downloads.is_empty())
                    });
                    if relevant {
                        let (has_active, _) = scan_browser_downloads_once();
                        if has_active {
                            ensure_fs_poll_loop();
                        }
                    }
                });
                store.fs_monitors.push(monitor);
            }
        }
    });

    // Escaneo inicial por si ya había una descarga en curso al iniciar el Dock
    let (has_active, _) = scan_browser_downloads_once();
    if has_active {
        ensure_fs_poll_loop();
    }
}

/// Inicia la suscripción D-Bus de sesión a `com.canonical.Unity.LauncherEntry` (`Update`),
/// `dev.malus.Notifications` (`AppNotificationReceived`) y los monitores de descargas
/// para mostrar progreso y globos push en tiempo real sobre los iconos del Dock.
pub fn ensure_launcher_entry_listener() {
    ensure_notifications_config_monitor();

    PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        if store.dbus_conn.is_some() {
            return;
        }
        let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
            return;
        };

        let update_sub = bus.signal_subscribe(
            None,
            Some("com.canonical.Unity.LauncherEntry"),
            Some("Update"),
            None,
            None,
            gio::DBusSignalFlags::NONE,
            |_conn, sender, _path, _iface, _signal, params| {
                let Some(payload) = parse_unity_update_variant(params) else {
                    return;
                };
                let sender_name = (!sender.is_empty()).then(|| sender.to_string());
                let needs_anim = PROGRESS_STORE.with(|c| {
                    let mut st = c.borrow_mut();
                    apply_launcher_update(&mut st.entries, &payload, sender_name, Instant::now())
                });
                queue_redraw_all_overlays();
                if needs_anim {
                    ensure_animation_timer();
                }
            },
        );

        // Notificaciones push retransmitidas por `malus-de-notifications`
        let push_sub = bus.signal_subscribe(
            None,
            Some("dev.malus.Notifications"),
            Some("AppNotificationReceived"),
            Some("/org/freedesktop/Notifications"),
            None,
            gio::DBusSignalFlags::NONE,
            |_conn, _sender, _path, _iface, _signal, params| {
                let Some((desktop_entry, app_name, app_icon, is_replacement, urgency)) =
                    parse_push_notification_variant(params)
                else {
                    return;
                };
                let updated = PROGRESS_STORE.with(|c| {
                    let mut st = c.borrow_mut();
                    if !st.notif_config_loaded {
                        st.notif_config = NotificationsConfig::load();
                        st.notif_config_loaded = true;
                    }
                    let ProgressStore {
                        push_badges,
                        active_app_identities,
                        notif_config,
                        ..
                    } = &mut *st;
                    apply_push_notification(
                        push_badges,
                        active_app_identities,
                        notif_config,
                        &desktop_entry,
                        &app_name,
                        &app_icon,
                        is_replacement,
                        urgency,
                        Instant::now(),
                    )
                });
                if updated {
                    queue_redraw_all_overlays();
                    ensure_animation_timer();
                }
            },
        );

        // Limpiar progreso y contadores `LauncherEntry` automáticamente si la app emisora cierra D-Bus
        let owner_sub = bus.signal_subscribe(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            |_conn, _sender, _path, _iface, _signal, params| {
                let old_owner = params
                    .try_child_get::<String>(1)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let new_owner = params
                    .try_child_get::<String>(2)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if old_owner.is_empty() || !new_owner.is_empty() {
                    return;
                }
                let changed = PROGRESS_STORE.with(|c| {
                    let mut st = c.borrow_mut();
                    clear_entries_for_sender(&mut st.entries, &old_owner)
                });
                if changed {
                    queue_redraw_all_overlays();
                }
            },
        );

        store.dbus_subs.push(update_sub);
        store.dbus_subs.push(push_sub);
        store.dbus_subs.push(owner_sub);
        store.dbus_conn = Some(bus);
    });

    start_browser_download_monitors();
}

/// Adjunta una capa superior `DrawingArea` escalable con el icono del Dock para dibujar
/// tanto el globo rojo de notificaciones push (esquina superior derecha) como el indicador
/// circular de descarga Liquid Glass (esquina inferior derecha).
pub fn attach_download_progress_overlay(
    icon_container: &Overlay,
    app_cfg: &DockAppConfig,
    icon_size: i32,
) {
    let area = DrawingArea::new();
    area.add_css_class("dock-progress-overlay");
    let overlay_size = icon_size + OVERLAY_BLEED_PX * 2;
    area.set_size_request(overlay_size, overlay_size);
    area.set_margin_top(-OVERLAY_BLEED_PX);
    area.set_margin_bottom(-OVERLAY_BLEED_PX);
    area.set_margin_start(-OVERLAY_BLEED_PX);
    area.set_margin_end(-OVERLAY_BLEED_PX);
    area.set_halign(gtk4::Align::Center);
    area.set_valign(gtk4::Align::End);
    area.set_can_target(false);
    area.set_overflow(gtk4::Overflow::Visible);

    let matchers = app_cfg.matchers.clone();
    let app_name = app_cfg.name.clone();
    let command = app_cfg.command.clone();
    let icon_name = app_cfg.icon.clone();

    area.set_draw_func(move |_, cr, w, h| {
        let now = Instant::now();
        let (visual, badge) = PROGRESS_STORE.with(|cell| {
            let mut store = cell.borrow_mut();
            if !store.notif_config_loaded {
                store.notif_config = NotificationsConfig::load();
                store.notif_config_loaded = true;
            }
            let elapsed = store
                .started_at
                .map(|t| now.saturating_duration_since(t).as_secs_f64())
                .unwrap_or(0.0);
            let phase = (elapsed / 1.15).fract();
            let prog = compute_visual_from_sources(
                store.entries.values().chain(store.fs_entries.values()),
                &matchers,
                &app_name,
                &command,
                &icon_name,
                now,
                phase,
            );
            let bdg = compute_badge_visual_for_app(
                &store.entries,
                &store.push_badges,
                &store.notif_config,
                &matchers,
                &app_name,
                &command,
                &icon_name,
                now,
            );
            (prog, bdg)
        });
        if let Some(vis) = visual {
            if vis.alpha > 0.01 {
                draw_download_progress_circle(cr, w as f64, h as f64, vis);
            }
        }
        if let Some(bdg) = badge {
            draw_notification_badge_pill(cr, w as f64, h as f64, bdg);
        }
    });

    PROGRESS_STORE.with(|cell| {
        let mut store = cell.borrow_mut();
        store.overlays.retain(|w| w.upgrade().is_some());
        store.overlays.push(area.downgrade());
    });

    icon_container.add_overlay(&area);
    icon_container.set_measure_overlay(&area, false);
}

fn capsule_path(cr: &Context, x: f64, y: f64, w: f64, h: f64) {
    let r = (h * 0.5).min(w * 0.5);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, FRAC_PI_2);
    cr.arc(x + r, y + r, r, FRAC_PI_2, FRAC_PI_2 * 3.0);
    cr.close_path();
}

/// Dibuja el globo rojo de notificaciones push estilo macOS sobresaliendo fuera de la esquina
/// superior derecha del icono (sin recortes gracias al margen transparente `OVERLAY_BLEED_PX`).
fn draw_notification_badge_pill(cr: &Context, width: f64, height: f64, badge: BadgeVisual) {
    let bleed = OVERLAY_BLEED_PX as f64;
    let icon_w = (width - bleed * 2.0).max(0.0);
    let icon_h = (height - bleed * 2.0).max(0.0);
    let side = icon_w.min(icon_h);
    if side < 16.0 || badge.count == 0 {
        return;
    }

    let text = format_badge_count(badge.count);
    let pill_h = (side * 0.43).clamp(20.0, 38.0);
    let font_size = (pill_h * 0.66).clamp(12.5, 24.0);

    let _ = cr.save();
    cr.select_font_face("SF Pro Display", FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(font_size);

    let text_w = cr
        .text_extents(&text)
        .map(|ext| ext.width().max(ext.x_advance()))
        .unwrap_or(font_size * 0.62 * text.len() as f64);
    let h_pad = (pill_h * 0.32).max(5.5);
    let pill_w = if text.len() <= 1 {
        pill_h
    } else {
        (text_w + h_pad * 2.0).max(pill_h * 1.22)
    };

    // Esquina superior derecha sobresaliendo por fuera del borde del icono (estilo macOS)
    let icon_right = bleed + icon_w;
    let icon_top = bleed;
    let max_out_x = (bleed - 2.0).max(0.0);
    let max_out_y = (bleed - 2.0).max(0.0);
    let out_x = (pill_h * 0.36).min(max_out_x);
    let out_y = (pill_h * 0.36).min(max_out_y);
    let cx = icon_right - pill_w * 0.5 + out_x;
    let cy = icon_top + pill_h * 0.5 - out_y;

    // Aplicar animación elástica pop-in centrada en el globo
    cr.translate(cx, cy);
    cr.scale(badge.scale, badge.scale);
    let x = -pill_w * 0.5;
    let y = -pill_h * 0.5;

    // 1. Sombra suave inferior
    capsule_path(cr, x, y + 1.25, pill_w, pill_h);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.32);
    let _ = cr.fill();

    // 2. Relleno rojo macOS (degradado vertical SF Red)
    capsule_path(cr, x, y, pill_w, pill_h);
    let grad = LinearGradient::new(0.0, y, 0.0, y + pill_h);
    if badge.urgent {
        grad.add_color_stop_rgba(0.0, 1.0, 0.34, 0.28, 1.0);
        grad.add_color_stop_rgba(1.0, 0.96, 0.14, 0.11, 1.0);
    } else {
        grad.add_color_stop_rgba(0.0, 1.0, 0.27, 0.23, 1.0);
        grad.add_color_stop_rgba(1.0, 0.94, 0.18, 0.14, 1.0);
    }
    let _ = cr.set_source(&grad);
    let _ = cr.fill_preserve();

    // 3. Borde especular sutil para separación sobre iconos claros y oscuros
    cr.set_line_width(0.8);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.34);
    let _ = cr.stroke();

    // 4. Número centrado ópticamente en blanco puro
    if let Ok(ext) = cr.text_extents(&text) {
        let tx = -(ext.width() * 0.5 + ext.x_bearing());
        let ty = -(ext.height() * 0.5 + ext.y_bearing());
        cr.move_to(tx, ty);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
        let _ = cr.show_text(&text);
    }

    let _ = cr.restore();
}

fn draw_download_progress_circle(cr: &Context, width: f64, height: f64, vis: ProgressVisual) {
    let bleed = OVERLAY_BLEED_PX as f64;
    let icon_w = (width - bleed * 2.0).max(0.0);
    let icon_h = (height - bleed * 2.0).max(0.0);
    let side = icon_w.min(icon_h);
    if side < 16.0 {
        return;
    }

    let is_dark = adw::StyleManager::default().is_dark();
    let diameter = (side * 0.44).clamp(18.0, 44.0);
    let r = diameter * 0.5;
    let out = (r * 0.16).min((bleed - 2.0).max(0.0));
    let cx = bleed + icon_w - r + out;
    let cy = bleed + icon_h - r + out;
    let stroke = (diameter * 0.125).clamp(2.0, 4.2);
    let ring_r = (r - stroke * 1.15).max(3.5);
    let alpha = vis.alpha;
    let (ar, ag, ab) = malus_de_common::config::load_accent_color();

    let _ = cr.save();

    // 1. Sombra suave de separación sobre el icono de la app
    cr.arc(cx, cy + 1.1, r + 0.6, 0.0, TAU);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.34 * alpha);
    let _ = cr.fill();

    // 2. Disco circular translúcido Liquid Glass
    cr.arc(cx, cy, r, 0.0, TAU);
    if is_dark {
        cr.set_source_rgba(0.11, 0.12, 0.15, 0.86 * alpha);
    } else {
        cr.set_source_rgba(0.98, 0.99, 1.0, 0.93 * alpha);
    }
    let _ = cr.fill_preserve();

    // Borde especular fino
    cr.set_line_width(0.85);
    if is_dark {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.26 * alpha);
    } else {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.12 * alpha);
    }
    let _ = cr.stroke();

    // 3. Pista circular de fondo
    cr.arc(cx, cy, ring_r, 0.0, TAU);
    cr.set_line_width(stroke);
    if is_dark {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.20 * alpha);
    } else {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.13 * alpha);
    }
    let _ = cr.stroke();

    // 3.5. Relleno sectorial translúcido proporcional al porcentaje cuando es determinado
    if !vis.completed && !vis.indeterminate && vis.progress > 0.01 {
        let sweep = vis.progress.clamp(0.02, 1.0);
        let fill_r = (ring_r - stroke * 0.45).max(1.5);
        cr.move_to(cx, cy);
        cr.arc(cx, cy, fill_r, -FRAC_PI_2, -FRAC_PI_2 + TAU * sweep);
        cr.close_path();
        cr.set_source_rgba(ar, ag, ab, (if is_dark { 0.28 } else { 0.20 }) * alpha);
        let _ = cr.fill();
    }

    // 4. Arco de progreso en color de acento (determinado, indeterminado rotatorio o verde al completar)
    let (start_angle, end_angle) = if vis.completed {
        (-FRAC_PI_2, -FRAC_PI_2 + TAU)
    } else if vis.indeterminate {
        let start = -FRAC_PI_2 + TAU * vis.phase;
        (start, start + TAU * 0.36)
    } else {
        let sweep = vis.progress.clamp(0.04, 1.0);
        (-FRAC_PI_2, -FRAC_PI_2 + TAU * sweep)
    };
    cr.arc(cx, cy, ring_r, start_angle, end_angle);
    cr.set_line_width(stroke);
    cr.set_line_cap(LineCap::Round);
    if vis.completed {
        cr.set_source_rgba(0.19, 0.82, 0.35, alpha);
    } else {
        cr.set_source_rgba(ar, ag, ab, alpha);
    }
    let _ = cr.stroke();

    // 5. Glifo central: flecha de descarga (↓) en curso o verificación (✓) al completar
    let glyph_r = ring_r * 0.44;
    let glyph_stroke = (diameter * 0.09).clamp(1.4, 2.8);
    cr.set_line_width(glyph_stroke);
    cr.set_line_cap(LineCap::Round);
    cr.set_line_join(LineJoin::Round);

    if vis.completed {
        cr.set_source_rgba(0.19, 0.82, 0.35, alpha);
        cr.move_to(cx - glyph_r * 0.9, cy + glyph_r * 0.05);
        cr.line_to(cx - glyph_r * 0.2, cy + glyph_r * 0.78);
        cr.line_to(cx + glyph_r * 0.95, cy - glyph_r * 0.65);
        let _ = cr.stroke();
    } else {
        if is_dark {
            cr.set_source_rgba(0.96, 0.96, 0.98, 0.92 * alpha);
        } else {
            cr.set_source_rgba(0.11, 0.12, 0.14, 0.88 * alpha);
        }
        // Tallo vertical
        cr.move_to(cx, cy - glyph_r * 0.95);
        cr.line_to(cx, cy + glyph_r * 0.85);
        let _ = cr.stroke();
        // Punta inferior de la flecha
        cr.move_to(cx - glyph_r * 0.72, cy + glyph_r * 0.18);
        cr.line_to(cx, cy + glyph_r * 0.92);
        cr.line_to(cx + glyph_r * 0.72, cy + glyph_r * 0.18);
        let _ = cr.stroke();
    }

    let _ = cr.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use malus_de_common::config::AppNotificationRule;

    #[test]
    fn normalizes_unity_launcher_uris_and_matches_browsers_and_apps() {
        assert_eq!(
            normalize_launcher_uri("application://org.mozilla.firefox.desktop"),
            "org.mozilla.firefox"
        );
        assert_eq!(
            normalize_launcher_uri("application://brave-browser.desktop"),
            "brave-browser"
        );

        assert!(entry_matches_dock_app(
            "application://org.mozilla.firefox.desktop",
            &["firefox".to_string()],
            "Firefox",
            "firefox",
            "firefox"
        ));
        assert!(entry_matches_dock_app(
            "application://brave-browser.desktop",
            &["brave".to_string()],
            "Brave",
            "brave-browser",
            "brave-browser"
        ));
        assert!(entry_matches_dock_app(
            "application://com.brave.Origin.desktop",
            &["brave-origin".to_string()],
            "Brave Origin",
            "/usr/bin/brave-origin-stable",
            "brave-origin"
        ));
        assert!(entry_matches_dock_app(
            "application://brave-origin.desktop",
            &["brave".to_string()],
            "Brave",
            "/usr/bin/brave-origin-stable",
            "brave-origin"
        ));
        assert!(entry_matches_dock_app(
            "application://google-chrome.desktop",
            &["chrome".to_string()],
            "Google Chrome",
            "/usr/bin/google-chrome-stable",
            "google-chrome"
        ));
        assert!(!entry_matches_dock_app(
            "application://org.mozilla.firefox.desktop",
            &["ptyxis".to_string()],
            "Terminal",
            "ptyxis",
            "org.gnome.Ptyxis"
        ));
    }

    #[test]
    fn parses_dbus_unity_update_variant_tuple_with_progress_and_count() {
        let dict = glib::VariantDict::new(None);
        dict.insert("progress", 0.42_f64);
        dict.insert("progress-visible", true);
        dict.insert("count", 7_i64);
        dict.insert("count-visible", true);
        dict.insert("urgent", true);
        let tuple = glib::Variant::tuple_from_iter([
            "application://brave-origin.desktop".to_variant(),
            dict.end(),
        ]);
        let payload =
            parse_unity_update_variant(&tuple).expect("debe parsear la tupla D-Bus (sa{sv})");
        assert_eq!(payload.app_uri, "application://brave-origin.desktop");
        assert!((payload.progress.unwrap() - 0.42).abs() < 1e-6);
        assert_eq!(payload.progress_visible, Some(true));
        assert_eq!(payload.count, Some(7));
        assert_eq!(payload.count_visible, Some(true));
        assert_eq!(payload.urgent, Some(true));
    }

    #[test]
    fn updates_progress_merges_partial_dicts_and_handles_completion_and_exit() {
        let mut entries = HashMap::new();
        let t0 = Instant::now();

        let fade = apply_launcher_update(
            &mut entries,
            &UnityUpdatePayload {
                app_uri: "application://firefox.desktop".to_string(),
                progress_visible: Some(true),
                ..Default::default()
            },
            Some(":1.42".to_string()),
            t0,
        );
        assert!(!fade);
        let vis = compute_visual_from_sources(
            entries.values(),
            &["firefox".to_string()],
            "Firefox",
            "firefox",
            "firefox",
            t0,
            0.0,
        )
        .expect("debe mostrarse el anillo de progreso");
        assert!(!vis.completed);
        assert!((vis.progress - 0.0).abs() < 1e-6);

        apply_launcher_update(
            &mut entries,
            &UnityUpdatePayload {
                app_uri: "application://firefox.desktop".to_string(),
                progress: Some(0.65),
                ..Default::default()
            },
            Some(":1.42".to_string()),
            t0,
        );
        let vis2 = compute_visual_from_sources(
            entries.values(),
            &["firefox".to_string()],
            "Firefox",
            "firefox",
            "firefox",
            t0,
            0.0,
        )
        .unwrap();
        assert!((vis2.progress - 0.65).abs() < 1e-6);

        apply_launcher_update(
            &mut entries,
            &UnityUpdatePayload {
                app_uri: "application://firefox.desktop".to_string(),
                progress: Some(1.0),
                progress_visible: Some(false),
                ..Default::default()
            },
            Some(":1.42".to_string()),
            t0,
        );
        let vis_done = compute_visual_from_sources(
            entries.values(),
            &["firefox".to_string()],
            "Firefox",
            "firefox",
            "firefox",
            t0,
            0.0,
        )
        .expect("debe mostrar breve check de completado");
        assert!(vis_done.completed);
        assert!((vis_done.alpha - 1.0).abs() < 1e-6);

        assert!(clear_entries_for_sender(&mut entries, ":1.42"));
        assert!(entries.is_empty());
    }

    #[test]
    fn push_notification_badges_accumulate_respect_settings_and_clear_on_focus() {
        let now = Instant::now();
        let mut entries = HashMap::new();
        let mut push_badges = HashMap::new();
        let mut notif_cfg = NotificationsConfig::default();

        // 1. Dos notificaciones para Telegram mientras otra app está enfocada -> contador = 2
        let active = vec!["org.gnome.Ptyxis".to_string()];
        assert!(apply_push_notification(
            &mut push_badges,
            &active,
            &notif_cfg,
            "org.telegram.desktop",
            "Telegram",
            "telegram",
            false,
            1,
            now,
        ));
        assert!(apply_push_notification(
            &mut push_badges,
            &active,
            &notif_cfg,
            "org.telegram.desktop",
            "Telegram",
            "telegram",
            false,
            1,
            now,
        ));
        // Una notificación que reemplaza a otra existente no duplica el contador
        assert!(apply_push_notification(
            &mut push_badges,
            &active,
            &notif_cfg,
            "org.telegram.desktop",
            "Telegram",
            "telegram",
            true,
            1,
            now,
        ));

        let bdg = compute_badge_visual_for_app(
            &entries,
            &push_badges,
            &notif_cfg,
            &["telegram".to_string()],
            "Telegram",
            "telegram-desktop",
            "telegram",
            now,
        )
        .expect("Telegram debe mostrar globo rojo con contador 2");
        assert_eq!(bdg.count, 2);
        assert_eq!(format_badge_count(bdg.count), "2");
        assert_eq!(format_badge_count(120), "99+");

        // 2. Si Telegram reporta su contador exacto vía Unity LauncherEntry, tiene prioridad
        apply_launcher_update(
            &mut entries,
            &UnityUpdatePayload {
                app_uri: "application://org.telegram.desktop.desktop".to_string(),
                count: Some(5),
                count_visible: Some(true),
                ..Default::default()
            },
            Some(":1.99".to_string()),
            now,
        );
        let bdg_unity = compute_badge_visual_for_app(
            &entries,
            &push_badges,
            &notif_cfg,
            &["telegram".to_string()],
            "Telegram",
            "telegram-desktop",
            "telegram",
            now,
        )
        .unwrap();
        assert_eq!(bdg_unity.count, 5);

        // 3. Desactivar "Contadores en el ícono de la app" en Ajustes oculta el globo
        notif_cfg.set_rule_for_app(
            "Telegram",
            AppNotificationRule {
                badge_icon: false,
                ..AppNotificationRule::default()
            },
        );
        assert!(compute_badge_visual_for_app(
            &entries,
            &push_badges,
            &notif_cfg,
            &["telegram".to_string()],
            "Telegram",
            "telegram-desktop",
            "telegram",
            now,
        )
        .is_none());

        // 4. Si la app ya tiene el foco, no acumula globos nuevos
        let notif_cfg_default = NotificationsConfig::default();
        assert!(!apply_push_notification(
            &mut push_badges,
            &["org.mozilla.firefox".to_string()],
            &notif_cfg_default,
            "org.mozilla.firefox",
            "Firefox",
            "firefox",
            false,
            1,
            now,
        ));

        // 5. Curva pop-in del globo push
        assert!((badge_pop_scale(0.0) - 0.35).abs() < 1e-6);
        assert!(badge_pop_scale(130.0) > 1.0);
        assert!((badge_pop_scale(BADGE_POP_ANIM_MS) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tracks_fs_browser_downloads_determinate_indeterminate_and_completion() {
        let now = Instant::now();
        let mut tracked = HashMap::new();
        let mut fs_entries = HashMap::new();

        tracked.insert(
            PathBuf::from("/tmp/iso.crdownload"),
            TrackedFsDownload {
                path: PathBuf::from("/tmp/iso.crdownload"),
                target_path: Some(PathBuf::from("/tmp/iso")),
                app_uri: "application://brave-origin.desktop".to_string(),
                current_bytes: 500,
                total_bytes: Some(1000),
                last_growth: now,
                last_total_lookup: Some(now),
                total_lookup_attempts: 1,
                lookup_in_flight: false,
            },
        );
        tracked.insert(
            PathBuf::from("/tmp/archive.tar.gz.part"),
            TrackedFsDownload {
                path: PathBuf::from("/tmp/archive.tar.gz.part"),
                target_path: Some(PathBuf::from("/tmp/archive.tar.gz")),
                app_uri: "application://org.mozilla.firefox.desktop".to_string(),
                current_bytes: 256,
                total_bytes: None,
                last_growth: now,
                last_total_lookup: Some(now),
                total_lookup_attempts: 1,
                lookup_in_flight: false,
            },
        );

        let needs_anim = rebuild_fs_entries_from_tracked(&tracked, &mut fs_entries, &[], now);
        assert!(needs_anim);

        let brave_vis = compute_visual_from_sources(
            fs_entries.values(),
            &["brave-origin".to_string()],
            "Brave Origin",
            "/usr/bin/brave-origin-stable",
            "brave-origin",
            now,
            0.25,
        )
        .expect("Brave Origin debe mostrar progreso determinado");
        assert!(!brave_vis.indeterminate);
        assert!(!brave_vis.completed);
        assert!((brave_vis.progress - 0.50).abs() < 1e-6);

        let ff_vis = compute_visual_from_sources(
            fs_entries.values(),
            &["firefox".to_string()],
            "Firefox",
            "firefox",
            "firefox",
            now,
            0.25,
        )
        .expect("Firefox debe mostrar progreso indeterminado animado");
        assert!(ff_vis.indeterminate);
        assert!(!ff_vis.completed);

        tracked.remove(&PathBuf::from("/tmp/archive.tar.gz.part"));
        let completed = vec!["application://org.mozilla.firefox.desktop".to_string()];
        let needs_anim_done =
            rebuild_fs_entries_from_tracked(&tracked, &mut fs_entries, &completed, now);
        assert!(needs_anim_done);
        let ff_done = compute_visual_from_sources(
            fs_entries.values(),
            &["firefox".to_string()],
            "Firefox",
            "firefox",
            "firefox",
            now,
            0.0,
        )
        .expect("Firefox debe mostrar check de completado");
        assert!(ff_done.completed);
    }

    #[test]
    fn parses_firefox_downloads_json_and_formats_sizes_and_browser_meta() {
        let json = r#"{
            "list": [
                {
                    "target": {
                        "path": "/home/cristian/Descargas/fedora.iso",
                        "partFilePath": "/home/cristian/Descargas/fedora.iso.part"
                    },
                    "totalBytes": 2147483648,
                    "currentBytes": 536870912
                }
            ]
        }"#;
        let (total, target) = parse_firefox_downloads_json(
            json,
            Path::new("/home/cristian/Descargas/fedora.iso.part"),
        )
        .expect("debe extraer totalBytes de downloads.json");
        assert_eq!(total, Some(2_147_483_648));
        assert_eq!(
            target.as_deref(),
            Some(Path::new("/home/cristian/Descargas/fedora.iso"))
        );

        assert_eq!(format_download_size(512), "512 B");
        assert_eq!(format_download_size(15 * 1024), "15 KB");
        assert_eq!(format_download_size(12 * 1024 * 1024), "12.0 MB");
        assert_eq!(format_download_size(2 * 1024 * 1024 * 1024), "2.00 GB");

        assert_eq!(
            browser_display_meta("application://brave-origin.desktop"),
            ("Brave Origin", "brave-origin", "brave-origin")
        );
        assert_eq!(
            browser_display_meta("application://org.mozilla.firefox.desktop"),
            ("Firefox", "firefox", "org.mozilla.firefox")
        );
    }
}
