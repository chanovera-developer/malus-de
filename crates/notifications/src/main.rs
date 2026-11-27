mod history;

use anyhow::Result;
use gio::prelude::ApplicationExtManual;
use glib::ControlFlow;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea,
    EventControllerKey, EventControllerMotion, GestureClick, Image, Label, Orientation, Overlay,
    PolicyType, ScrolledWindow,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use history::{
    extract_hint_image, format_current_hhmm, notification_group_key, parse_notification_actions,
    resolve_expire_timeout_ms, should_hide_preview, NotificationAction, NotificationHistory,
    NotificationImage, NotificationRecord,
};
use libadwaita as adw;
use malus_de_common::ipc::{ipc_socket_path, send_ipc_message, IpcMessage};
use malus_de_common::logging::init_logging;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::f64::consts::FRAC_PI_2;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tracing::info;
use zbus::zvariant::Value;

static NOTIFICATION_COUNTER: AtomicU32 = AtomicU32::new(1);

// 8 px de margen superior/izquierdo reservados para el botón circular ✕ que sobresale de la esquina.
// El radio (44 px) y exponente (2.4) coinciden exactamente con `NOTIFICATION_CORNER_RADIUS` / `NOTIFICATION_CORNER_EXPONENT` del compositor.
const CLOSE_OVERHANG_PX: i32 = 8;
const CARD_SQUIRCLE_RADIUS: f64 = 44.0;
const CARD_SQUIRCLE_EXPONENT: f64 = 2.4;
const CENTER_SQUIRCLE_RADIUS: f64 = 38.0;
const SQUIRCLE_EXPONENT: f64 = 4.2;

const BANNER_TOP_BASE: i32 = 34;
const BANNER_RIGHT_MARGIN: i32 = 14;
const BANNER_STACK_GAP: i32 = 10;
const BANNER_DEFAULT_WIDTH: i32 = 380;
const BANNER_DEFAULT_HEIGHT: i32 = 96;
const BANNER_OFFSCREEN_START_RIGHT: f64 = -410.0;
const ANIM_SPEED_SLIDE_IN: f64 = 15.0;
const ANIM_SPEED_SLIDE_OUT: f64 = 16.0;
const ANIM_SPEED_REFLOW: f64 = 14.0;
const ANIM_EPSILON_PX: f64 = 0.65;
// Coincide con `BLACK_HOLE_DURATION` (680 ms) del compositor para que los globos inferiores
// esperen a que termine el efecto Agujero Negro antes de deslizarse hacia arriba.
const BLACK_HOLE_SLOT_HOLD_MS: u64 = 660;

struct BannerEntry {
    id: u32,
    group_key: String,
    grouped_ids: Vec<u32>,
    window: Option<ApplicationWindow>,
    bg: Option<DrawingArea>,
    held_card_height: i32,
    current_top: f64,
    target_top: f64,
    last_applied_top: i32,
    current_right: f64,
    target_right: f64,
    last_applied_right: i32,
    closing_slide_out: bool,
    slot_hold_until: Option<Instant>,
}

struct NotificationCenterRuntime {
    window: ApplicationWindow,
    bg: DrawingArea,
    list_box: GtkBox,
    focus_bar: GtkBox,
    focus_lbl: Label,
    clear_btn: Button,
}

thread_local! {
    static NOTIF_HOLD_GUARD: RefCell<Option<gio::ApplicationHoldGuard>> =
        const { RefCell::new(None) };
    static NOTIF_THEME_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static NOTIF_GLASS_MONITOR: RefCell<Option<gtk4::gio::FileMonitor>> =
        const { RefCell::new(None) };
    static NOTIF_DARK_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static NOTIF_OPACITY: Cell<f64> = const { Cell::new(0.55) };
    static SESSION_LOCKED: Cell<bool> = const { Cell::new(false) };
    static ACTIVE_BANNERS: RefCell<Vec<BannerEntry>> = const { RefCell::new(Vec::new()) };
    static NOTIFICATION_HISTORY: RefCell<NotificationHistory> =
        RefCell::new(NotificationHistory::default());
    static EXPANDED_CENTER_GROUPS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static NOTIFICATION_CENTER: RefCell<Option<NotificationCenterRuntime>> =
        const { RefCell::new(None) };
    static RUNNING_APPS_CACHE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static DBUS_SIGNAL_TX: RefCell<Option<async_channel::Sender<DbusSignalCmd>>> =
        const { RefCell::new(None) };
    static ANIM_RUNNING: Cell<bool> = const { Cell::new(false) };
    static ANIM_LAST_TICK: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[derive(Debug, Clone)]
enum DbusSignalCmd {
    ActionInvoked { id: u32, action_key: String },
    NotificationClosed { id: u32, reason: u32 },
}

fn squircle_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    squircle_path_with_exponent(cr, x, y, w, h, radius, SQUIRCLE_EXPONENT);
}

fn squircle_path_with_exponent(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    exponent: f64,
) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.5 {
        cr.rectangle(x, y, w, h);
        return;
    }

    let steps = 18usize;
    let inv_n = 1.0 / exponent;
    let corners = [
        (x + w - r, y + r, 1.0, -1.0, false),
        (x + w - r, y + h - r, 1.0, 1.0, true),
        (x + r, y + h - r, -1.0, 1.0, false),
        (x + r, y + r, -1.0, -1.0, true),
    ];

    cr.new_sub_path();
    let mut first = true;
    for (cx, cy, sx, sy, reverse) in corners {
        for i in 0..=steps {
            let t = if reverse {
                1.0 - (i as f64 / steps as f64)
            } else {
                i as f64 / steps as f64
            };
            let theta = t * FRAC_PI_2;
            let px = cx + sx * r * theta.sin().powf(inv_n);
            let py = cy + sy * r * theta.cos().powf(inv_n);
            if first {
                cr.move_to(px, py);
                first = false;
            } else {
                cr.line_to(px, py);
            }
        }
    }
    cr.close_path();
}

fn build_notification_theme_css(dark: bool) -> String {
    if dark {
        r#"
.notif-title,
.notif-center-title,
.notif-center-group-app {
    color: rgba(255, 255, 255, 0.95);
}
.notif-body,
.notif-meta-time,
.notif-center-empty,
.notif-center-focus-label {
    color: rgba(235, 238, 246, 0.86);
}
.notif-app-icon,
button.notif-close-btn,
button.notif-close-btn image,
button.notif-center-item-close,
button.notif-center-item-close image {
    color: rgba(255, 255, 255, 0.95);
}
button.notif-close-btn {
    background-color: rgba(52, 54, 64, 0.92);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.36);
}
button.notif-close-btn:hover {
    background-color: rgba(72, 75, 88, 0.96);
}
button.notif-close-btn:active {
    background-color: rgba(40, 42, 50, 0.96);
}
button.notif-action-btn,
button.notif-center-clear-btn,
button.notif-stack-pill {
    background-color: rgba(255, 255, 255, 0.14);
    color: rgba(255, 255, 255, 0.95);
}
button.notif-action-btn:hover,
button.notif-center-clear-btn:hover,
button.notif-stack-pill:hover {
    background-color: rgba(255, 255, 255, 0.22);
}
.notif-center-focus-bar {
    background-color: rgba(114, 108, 245, 0.24);
}
.notif-center-group-card {
    background-color: rgba(255, 255, 255, 0.08);
}
.notif-center-item-sep {
    background-color: rgba(255, 255, 255, 0.10);
}
.notif-center-silenced-badge {
    background-color: rgba(136, 128, 255, 0.28);
    color: rgba(236, 236, 255, 0.95);
}
button.notif-center-item-close {
    background-color: rgba(255, 255, 255, 0.10);
}
button.notif-center-item-close:hover {
    background-color: rgba(255, 255, 255, 0.20);
}
"#
        .to_string()
    } else {
        r#"
.notif-title,
.notif-center-title,
.notif-center-group-app {
    color: rgba(14, 16, 22, 0.94);
}
.notif-body,
.notif-meta-time,
.notif-center-empty,
.notif-center-focus-label {
    color: rgba(18, 20, 26, 0.86);
}
.notif-app-icon,
button.notif-close-btn,
button.notif-close-btn image,
button.notif-center-item-close,
button.notif-center-item-close image {
    color: rgba(14, 16, 22, 0.94);
}
button.notif-close-btn {
    background-color: rgba(236, 240, 248, 0.94);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.16);
}
button.notif-close-btn:hover {
    background-color: rgba(248, 250, 255, 0.98);
}
button.notif-close-btn:active {
    background-color: rgba(218, 224, 236, 0.96);
}
button.notif-action-btn,
button.notif-center-clear-btn,
button.notif-stack-pill {
    background-color: rgba(14, 18, 28, 0.08);
    color: rgba(14, 16, 22, 0.92);
}
button.notif-action-btn:hover,
button.notif-center-clear-btn:hover,
button.notif-stack-pill:hover {
    background-color: rgba(14, 18, 28, 0.14);
}
.notif-center-focus-bar {
    background-color: rgba(88, 86, 214, 0.15);
}
.notif-center-group-card {
    background-color: rgba(255, 255, 255, 0.44);
}
.notif-center-item-sep {
    background-color: rgba(14, 16, 22, 0.09);
}
.notif-center-silenced-badge {
    background-color: rgba(88, 86, 214, 0.16);
    color: rgba(54, 52, 163, 0.96);
}
button.notif-center-item-close {
    background-color: rgba(14, 18, 28, 0.07);
}
button.notif-center-item-close:hover {
    background-color: rgba(14, 18, 28, 0.14);
}
"#
        .to_string()
    }
}

fn apply_notification_theme_now(dark_override: Option<bool>) {
    let dark = dark_override.unwrap_or_else(malus_de_common::config::is_dark_mode_active);
    let opacity = malus_de_common::config::load_liquid_glass_opacity();
    NOTIF_DARK_ACTIVE.with(|c| c.set(dark));
    NOTIF_OPACITY.with(|c| c.set(opacity));

    let css = build_notification_theme_css(dark);
    NOTIF_THEME_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
    ACTIVE_BANNERS.with(|cell| {
        for entry in cell.borrow().iter() {
            if let Some(ref bg) = entry.bg {
                bg.queue_draw();
            }
        }
    });
    NOTIFICATION_CENTER.with(|cell| {
        if let Some(ref rt) = *cell.borrow() {
            rt.bg.queue_draw();
        }
    });
}

fn step_damped(current: f64, target: f64, dt_secs: f64, speed: f64) -> f64 {
    let diff = target - current;
    if diff.abs() <= ANIM_EPSILON_PX {
        return target;
    }
    let alpha = 1.0 - (-dt_secs.clamp(0.001, 0.05) * speed).exp();
    let next = current + diff * alpha;
    if (target - next).abs() <= ANIM_EPSILON_PX {
        target
    } else {
        next
    }
}

fn entry_card_height(entry: &BannerEntry) -> i32 {
    if let Some(ref win) = entry.window {
        let h = win.height();
        if h > CLOSE_OVERHANG_PX + 20 {
            return (h - CLOSE_OVERHANG_PX).max(60);
        }
    }
    entry
        .held_card_height
        .max((BANNER_DEFAULT_HEIGHT - CLOSE_OVERHANG_PX).max(60))
}

fn recompute_banner_targets_locked(list: &mut [BannerEntry]) {
    let mut y = BANNER_TOP_BASE;
    for entry in list.iter_mut() {
        let card_h = entry_card_height(entry);
        entry.held_card_height = card_h;
        if entry.window.is_some() && !entry.closing_slide_out {
            entry.target_top = y as f64;
        }
        y += card_h + BANNER_STACK_GAP;
    }
}

fn recompute_banner_targets() {
    ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|entry| {
            entry.slot_hold_until.is_some()
                || entry
                    .window
                    .as_ref()
                    .is_some_and(|w| w.is_visible() || !entry.closing_slide_out)
        });
        recompute_banner_targets_locked(&mut list);
    });
    ensure_animation_loop();
}

fn ensure_animation_loop() {
    let already = ANIM_RUNNING.with(|c| {
        if c.get() {
            true
        } else {
            c.set(true);
            false
        }
    });
    if already {
        return;
    }
    ANIM_LAST_TICK.with(|t| t.set(Some(Instant::now())));

    glib::timeout_add_local(Duration::from_millis(16), || {
        let now = Instant::now();
        let dt = ANIM_LAST_TICK.with(|t| {
            let prev = t.get().unwrap_or(now);
            t.set(Some(now));
            now.saturating_duration_since(prev)
                .as_secs_f64()
                .clamp(0.004, 0.05)
        });

        let mut to_close: Vec<ApplicationWindow> = Vec::new();
        let mut any_animating = false;

        ACTIVE_BANNERS.with(|cell| {
            let mut list = cell.borrow_mut();

            let before_len = list.len();
            list.retain(|entry| {
                if let Some(until) = entry.slot_hold_until {
                    now < until
                } else {
                    true
                }
            });
            if list.len() != before_len {
                any_animating = !list.is_empty();
            }

            recompute_banner_targets_locked(&mut list);

            let supported = gtk4_layer_shell::is_supported();
            let mut finished_ids: Vec<u32> = Vec::new();

            for entry in list.iter_mut() {
                if entry.slot_hold_until.is_some() {
                    any_animating = true;
                    continue;
                }
                let Some(ref win) = entry.window else {
                    continue;
                };

                let speed_x = if entry.closing_slide_out {
                    ANIM_SPEED_SLIDE_OUT
                } else {
                    ANIM_SPEED_SLIDE_IN
                };
                entry.current_right =
                    step_damped(entry.current_right, entry.target_right, dt, speed_x);
                entry.current_top =
                    step_damped(entry.current_top, entry.target_top, dt, ANIM_SPEED_REFLOW);

                let desired_right = entry.current_right.round() as i32;
                let desired_top = entry.current_top.round() as i32;

                if supported {
                    if desired_right != entry.last_applied_right {
                        win.set_margin(Edge::Right, desired_right);
                        entry.last_applied_right = desired_right;
                    }
                    if desired_top != entry.last_applied_top {
                        win.set_margin(Edge::Top, desired_top);
                        entry.last_applied_top = desired_top;
                    }
                }

                let settled_x = (entry.current_right - entry.target_right).abs() <= ANIM_EPSILON_PX;
                let settled_y = (entry.current_top - entry.target_top).abs() <= ANIM_EPSILON_PX;

                if entry.closing_slide_out && settled_x {
                    finished_ids.push(entry.id);
                    to_close.push(win.clone());
                } else if !settled_x || !settled_y {
                    any_animating = true;
                }
            }

            if !finished_ids.is_empty() {
                list.retain(|entry| !finished_ids.contains(&entry.id));
                recompute_banner_targets_locked(&mut list);
                if !list.is_empty() {
                    any_animating = true;
                }
            }
        });

        for win in to_close {
            win.close();
        }

        if any_animating {
            ControlFlow::Continue
        } else {
            ANIM_RUNNING.with(|c| c.set(false));
            ANIM_LAST_TICK.with(|t| t.set(None));
            ControlFlow::Break
        }
    });
}

fn begin_slide_out_close(id: u32) {
    let triggered = ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        let mut found = false;
        for entry in list.iter_mut() {
            if (entry.id == id || entry.grouped_ids.contains(&id))
                && !entry.closing_slide_out
                && entry.window.is_some()
            {
                let w = entry
                    .window
                    .as_ref()
                    .map(|win| win.width().max(BANNER_DEFAULT_WIDTH))
                    .unwrap_or(BANNER_DEFAULT_WIDTH);
                entry.closing_slide_out = true;
                entry.target_right = -(w + 48) as f64;
                found = true;
                break;
            }
        }
        found
    });
    if triggered {
        ensure_animation_loop();
    }
}

fn close_banner_immediately(id: u32, hold_slot_for_black_hole: bool) {
    let target = ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        if let Some(pos) = list
            .iter()
            .position(|entry| entry.id == id || entry.grouped_ids.contains(&id))
        {
            if hold_slot_for_black_hole {
                let card_h = entry_card_height(&list[pos]);
                list[pos].held_card_height = card_h;
                list[pos].bg = None;
                list[pos].closing_slide_out = true;
                list[pos].slot_hold_until =
                    Some(Instant::now() + Duration::from_millis(BLACK_HOLE_SLOT_HOLD_MS));
                list[pos].window.take()
            } else {
                let removed = list.remove(pos);
                recompute_banner_targets_locked(&mut list);
                removed.window
            }
        } else {
            None
        }
    });
    if let Some(win) = target {
        win.close();
        ensure_animation_loop();
    }
}

fn close_banners_for_group(group_key: &str) {
    let windows: Vec<ApplicationWindow> = ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        let mut closed = Vec::new();
        list.retain_mut(|entry| {
            if entry.group_key == group_key {
                if let Some(w) = entry.window.take() {
                    closed.push(w);
                }
                false
            } else {
                true
            }
        });
        recompute_banner_targets_locked(&mut list);
        closed
    });
    for win in windows {
        win.close();
    }
    ensure_animation_loop();
}

pub struct NotificationServer {
    sender: async_channel::Sender<NotificationEvent>,
}

#[derive(Debug, Clone)]
pub enum NotificationEvent {
    Show {
        msg: NotificationMsg,
        record: NotificationRecord,
    },
    RecordOnly(NotificationRecord),
    Close(u32),
    ThemeChanged(bool),
    RunningAppsChanged(Vec<String>),
    ToggleCenter,
    SessionLockChanged(bool),
}

#[derive(Debug, Clone)]
pub struct NotificationMsg {
    pub id: u32,
    pub group_key: String,
    pub desktop_entry: String,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub sound: bool,
    pub has_default_action: bool,
    pub actions: Vec<NotificationAction>,
    pub image: Option<NotificationImage>,
    pub expire_timeout: i32,
}

fn has_default_notification_action(actions: &[String]) -> bool {
    actions
        .chunks(2)
        .any(|pair| pair.first().is_some_and(|key| key == "default"))
}

fn normalize_app_token(raw: &str) -> String {
    raw.trim()
        .strip_prefix("application://")
        .unwrap_or(raw.trim())
        .trim_end_matches(".desktop")
        .trim()
        .to_ascii_lowercase()
}

fn resolve_target_running_app(
    running_apps: &[String],
    desktop_entry: &str,
    app_name: &str,
    app_icon: &str,
) -> Option<String> {
    let raw_candidates = [desktop_entry, app_name, app_icon];
    let mut normalized_candidates: Vec<String> = Vec::new();
    let mut push_unique = |val: String| {
        if val.len() >= 2 && !normalized_candidates.iter().any(|c| c == &val) {
            normalized_candidates.push(val);
        }
    };

    for raw in raw_candidates {
        let norm = normalize_app_token(raw);
        if norm.is_empty() || norm.starts_with('/') {
            continue;
        }
        push_unique(norm.clone());
        if let Some(last_dot) = norm.rsplit('.').next() {
            if last_dot.len() >= 3 && last_dot != "desktop" {
                push_unique(last_dot.to_string());
            }
        }
        let dashed = norm.replace(' ', "-");
        if dashed != norm {
            push_unique(dashed);
        }
    }

    for cand in &normalized_candidates {
        if let Some(found) = running_apps
            .iter()
            .find(|r| normalize_app_token(r) == *cand)
        {
            return Some(found.clone());
        }
    }

    for cand in &normalized_candidates {
        if cand.len() < 3 {
            continue;
        }
        if let Some(found) = running_apps.iter().find(|r| {
            let r_norm = normalize_app_token(r);
            r_norm.len() >= 3 && (r_norm.contains(cand.as_str()) || cand.contains(r_norm.as_str()))
        }) {
            return Some(found.clone());
        }
    }

    None
}

fn launch_desktop_app_for_notification(desktop_entry: &str, app_name: &str, app_icon: &str) -> bool {
    let mut lookup_ids: Vec<String> = Vec::new();
    for raw in [desktop_entry, app_name, app_icon] {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('/') {
            continue;
        }
        let norm = normalize_app_token(trimmed);
        for cand in [
            format!("{trimmed}.desktop"),
            format!("{norm}.desktop"),
            trimmed.to_string(),
        ] {
            if !lookup_ids.contains(&cand) {
                lookup_ids.push(cand);
            }
        }
    }

    for cand in &lookup_ids {
        if let Some(info) = gio::DesktopAppInfo::new(cand) {
            if info.launch(&[], gio::AppLaunchContext::NONE).is_ok() {
                info!("🚀 Aplicación lanzada desde notificación: {}", info.name());
                return true;
            }
        }
    }

    let targets: Vec<String> = [desktop_entry, app_name, app_icon]
        .into_iter()
        .map(normalize_app_token)
        .filter(|s| s.len() >= 3 && !s.starts_with('/'))
        .collect();
    if targets.is_empty() {
        return false;
    }

    for app_info in gio::AppInfo::all() {
        let id_norm = app_info
            .id()
            .map(|id| normalize_app_token(&id))
            .unwrap_or_default();
        let name_norm = normalize_app_token(&app_info.name());
        let matches = targets.iter().any(|t| {
            id_norm == *t
                || name_norm == *t
                || (!id_norm.is_empty()
                    && (id_norm.contains(t.as_str()) || t.contains(id_norm.as_str())))
                || (!name_norm.is_empty() && name_norm == *t)
        });
        if matches && app_info.launch(&[], gio::AppLaunchContext::NONE).is_ok() {
            info!(
                "🚀 Aplicación lanzada desde notificación (AppInfo): {}",
                app_info.name()
            );
            return true;
        }
    }

    false
}

fn emit_dbus_action(notif_id: u32, action_key: &str) {
    DBUS_SIGNAL_TX.with(|cell| {
        if let Some(tx) = cell.borrow().as_ref() {
            let _ = tx.send_blocking(DbusSignalCmd::ActionInvoked {
                id: notif_id,
                action_key: action_key.to_string(),
            });
        }
    });
}

fn emit_dbus_closed(notif_id: u32, reason: u32) {
    DBUS_SIGNAL_TX.with(|cell| {
        if let Some(tx) = cell.borrow().as_ref() {
            let _ = tx.send_blocking(DbusSignalCmd::NotificationClosed {
                id: notif_id,
                reason,
            });
        }
    });
}

fn activate_notification_source_app(
    notif_id: u32,
    desktop_entry: &str,
    app_name: &str,
    app_icon: &str,
    has_default_action: bool,
) {
    if has_default_action {
        emit_dbus_action(notif_id, "default");
    }

    let running = RUNNING_APPS_CACHE.with(|cell| cell.borrow().clone());
    if let Some(running_id) =
        resolve_target_running_app(&running, desktop_entry, app_name, app_icon)
    {
        info!(
            "🔔 Clic en notificación [id={}]: activando app en ejecución '{}'",
            notif_id, running_id
        );
        let _ = send_ipc_message(&IpcMessage::ActivateApp { app_id: running_id });
        return;
    }

    let fallback_target = [desktop_entry, app_name, app_icon]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty() && !s.starts_with('/'));

    if running.is_empty() {
        if let Some(target) = fallback_target {
            info!(
                "🔔 Clic en notificación [id={}]: solicitando ActivateApp para '{}'",
                notif_id, target
            );
            let _ = send_ipc_message(&IpcMessage::ActivateApp {
                app_id: target.to_string(),
            });
            return;
        }
    }

    if !launch_desktop_app_for_notification(desktop_entry, app_name, app_icon) {
        if let Some(target) = fallback_target {
            let _ = send_ipc_message(&IpcMessage::ActivateApp {
                app_id: target.to_string(),
            });
        }
    }
}

fn extract_hint_str(hints: &HashMap<String, Value<'_>>, key: &str) -> Option<String> {
    hints
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_hint_bool(hints: &HashMap<String, Value<'_>>, key: &str) -> bool {
    hints
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

fn extract_hint_urgency(hints: &HashMap<String, Value<'_>>) -> u8 {
    hints
        .get("urgency")
        .and_then(|v| {
            u8::try_from(v)
                .ok()
                .or_else(|| u32::try_from(v).ok().map(|n| n.min(2) as u8))
                .or_else(|| i32::try_from(v).ok().map(|n| n.clamp(0, 2) as u8))
        })
        .unwrap_or(1)
}

fn should_skip_dock_push_badge(
    app_name: &str,
    desktop_entry: &str,
    category: Option<&str>,
    transient: bool,
) -> bool {
    if transient {
        return true;
    }
    let name_low = app_name.trim().to_ascii_lowercase();
    let entry_low = desktop_entry.trim().to_ascii_lowercase();
    if name_low == "malus de"
        || name_low == "malus-de"
        || entry_low == "malus-de-dock"
        || entry_low == "malus-de-top-bar"
    {
        return true;
    }
    if let Some(cat) = category {
        let cat_low = cat.to_ascii_lowercase();
        if cat_low.starts_with("device")
            || cat_low.starts_with("x-malus-osd")
            || cat_low == "transfer.complete"
        {
            return true;
        }
    }
    false
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    async fn notify(
        &mut self,
        #[zbus(signal_context)] ctxt: zbus::SignalContext<'_>,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            NOTIFICATION_COUNTER.fetch_add(1, Ordering::SeqCst)
        };

        let desktop_entry = extract_hint_str(&hints, "desktop-entry").unwrap_or_default();
        let category = extract_hint_str(&hints, "category");
        let transient = extract_hint_bool(&hints, "transient");
        let urgency = extract_hint_urgency(&hints);
        let has_default_action = has_default_notification_action(&actions);
        let parsed_actions = parse_notification_actions(&actions);
        let image = extract_hint_image(&hints);
        let group_key = notification_group_key(&desktop_entry, &app_name, &app_icon);

        let notif_cfg = malus_de_common::config::NotificationsConfig::load();
        let rule_key = if !app_name.trim().is_empty() {
            app_name.as_str()
        } else {
            desktop_entry.as_str()
        };
        let rule = notif_cfg.rule_for_app(rule_key);

        info!(
            "Notificación recibida [id={}] de '{}' (desktop_entry='{}'): '{}' - '{}'",
            id, app_name, desktop_entry, summary, body
        );

        if !rule.enabled {
            info!(
                "Notificación silenciada por preferencia de usuario para '{}'",
                rule_key
            );
            return id;
        }

        let focus_cfg = malus_de_common::config::FocusConfig::load();
        let (focus_silence_banner, focus_silence_badge) =
            focus_cfg.should_silence_notification(rule_key, &desktop_entry, urgency);

        if rule.badge_icon
            && !focus_silence_badge
            && !should_skip_dock_push_badge(
                &app_name,
                &desktop_entry,
                category.as_deref(),
                transient,
            )
        {
            let is_replacement = replaces_id > 0;
            let _ = ctxt
                .connection()
                .emit_signal(
                    None::<&str>,
                    "/org/freedesktop/Notifications",
                    "dev.malus.Notifications",
                    "AppNotificationReceived",
                    &(
                        desktop_entry.as_str(),
                        app_name.as_str(),
                        app_icon.as_str(),
                        is_replacement,
                        urgency,
                    ),
                )
                .await;
        }

        let record = NotificationRecord {
            id,
            group_key: group_key.clone(),
            desktop_entry: desktop_entry.clone(),
            app_name: app_name.clone(),
            app_icon: app_icon.clone(),
            summary: summary.clone(),
            body: body.clone(),
            actions: parsed_actions.clone(),
            has_default_action,
            image: image.clone(),
            timestamp_label: format_current_hhmm(),
            silenced_by_focus: focus_silence_banner,
            urgency,
        };

        if focus_silence_banner {
            info!(
                "Notificación silenciada por modo de Enfoque activo para '{}' (archivada en Centro de Notificaciones)",
                rule_key
            );
            if !transient {
                let _ = self.sender.send(NotificationEvent::RecordOnly(record)).await;
            }
            return id;
        }

        if rule.style != malus_de_common::config::NotificationAlertStyle::None {
            let msg = NotificationMsg {
                id,
                group_key,
                desktop_entry,
                app_name,
                app_icon,
                summary,
                body,
                sound: rule.sound
                    && (hints.contains_key("sound-name") || !hints.contains_key("suppress-sound")),
                has_default_action,
                actions: parsed_actions,
                image,
                expire_timeout,
            };

            let _ = self
                .sender
                .send(NotificationEvent::Show { msg, record })
                .await;
        } else if !transient {
            let _ = self.sender.send(NotificationEvent::RecordOnly(record)).await;
        }

        id
    }

    async fn close_notification(&mut self, id: u32) {
        info!("Cierre de notificación solicitado para id={}", id);
        let _ = self.sender.send(NotificationEvent::Close(id)).await;
    }

    async fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["body", "actions", "icon-static", "persistence", "sound"]
    }

    async fn get_server_information(
        &self,
    ) -> (&'static str, &'static str, &'static str, &'static str) {
        ("Malus Notification Daemon", "Malus DE", "0.1.0", "1.2")
    }
}

fn main() -> Result<()> {
    init_logging("malus_de_notifications");
    let _ = malus_de_common::config::setup_isolated_session_environment();
    info!("Iniciando Malus DE Notification Daemon (D-Bus / GTK4 Layer Shell)...");

    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let (sender, receiver) = async_channel::unbounded::<NotificationEvent>();
    let (dbus_sig_tx, dbus_sig_rx) = async_channel::unbounded::<DbusSignalCmd>();
    DBUS_SIGNAL_TX.with(|cell| {
        *cell.borrow_mut() = Some(dbus_sig_tx);
    });

    // Servidor D-Bus en hilo de fondo
    let s_clone = sender.clone();
    std::thread::spawn(move || {
        let rt = zbus::block_on(async {
            let connection = zbus::connection::Builder::session()?
                .serve_at(
                    "/org/freedesktop/Notifications",
                    NotificationServer { sender: s_clone },
                )?
                .build()
                .await?;

            connection
                .request_name_with_flags(
                    "org.freedesktop.Notifications",
                    zbus::fdo::RequestNameFlags::ReplaceExisting
                        | zbus::fdo::RequestNameFlags::AllowReplacement,
                )
                .await?;

            info!("Servicio D-Bus org.freedesktop.Notifications registrado con éxito.");
            while let Ok(cmd) = dbus_sig_rx.recv().await {
                match cmd {
                    DbusSignalCmd::ActionInvoked { id, action_key } => {
                        let _ = connection
                            .emit_signal(
                                None::<&str>,
                                "/org/freedesktop/Notifications",
                                "org.freedesktop.Notifications",
                                "ActionInvoked",
                                &(id, action_key.as_str()),
                            )
                            .await;
                    }
                    DbusSignalCmd::NotificationClosed { id, reason } => {
                        let _ = connection
                            .emit_signal(
                                None::<&str>,
                                "/org/freedesktop/Notifications",
                                "org.freedesktop.Notifications",
                                "NotificationClosed",
                                &(id, reason),
                            )
                            .await;
                    }
                }
            }
            Ok::<(), zbus::Error>(())
        });
        if let Err(e) = rt {
            tracing::warn!("Aviso al enlazar D-Bus Notifications: {:?}", e);
        }
    });

    // Escuchar eventos IPC del compositor (tema, opacidad, apps abiertas, bloqueo y Centro de Notificaciones)
    let ipc_tx = sender.clone();
    std::thread::spawn(move || loop {
        if let Ok(stream) = UnixStream::connect(ipc_socket_path()) {
            let reader = BufReader::new(stream);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                    match msg {
                        IpcMessage::ThemeChanged { is_dark, .. } => {
                            let _ = ipc_tx.send_blocking(NotificationEvent::ThemeChanged(is_dark));
                        }
                        IpcMessage::LiquidGlassOpacityChanged { .. } => {
                            let is_dark = malus_de_common::config::is_dark_mode_active();
                            let _ = ipc_tx.send_blocking(NotificationEvent::ThemeChanged(is_dark));
                        }
                        IpcMessage::RunningAppsChanged { running_apps } => {
                            let _ = ipc_tx
                                .send_blocking(NotificationEvent::RunningAppsChanged(running_apps));
                        }
                        IpcMessage::ToggleNotificationCenter => {
                            let _ = ipc_tx.send_blocking(NotificationEvent::ToggleCenter);
                        }
                        IpcMessage::SessionLockChanged { locked } => {
                            let _ =
                                ipc_tx.send_blocking(NotificationEvent::SessionLockChanged(locked));
                        }
                        _ => {}
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    });

    let app = Application::builder()
        .application_id("org.malus.de.Notifications")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|app| {
        NOTIF_HOLD_GUARD.with(|cell| {
            *cell.borrow_mut() = Some(app.hold());
        });
        let provider = CssProvider::new();
        provider.load_from_string(include_str!("styles/banner.css"));
        let theme_provider = CssProvider::new();
        if let Some(display) = gtk4::gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            for icon_dir in malus_de_common::apps::extra_icon_search_paths() {
                if icon_dir.is_dir() {
                    icon_theme.add_search_path(&icon_dir);
                }
            }
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk4::style_context_add_provider_for_display(
                &display,
                &theme_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
        }
        NOTIF_THEME_PROVIDER.with(|cell| {
            *cell.borrow_mut() = Some(theme_provider);
        });
        apply_notification_theme_now(None);

        adw::StyleManager::default().connect_dark_notify(|mgr| {
            apply_notification_theme_now(Some(mgr.is_dark()));
        });

        let cfg_file =
            gtk4::gio::File::for_path(malus_de_common::config::appearance_settings_path());
        if let Ok(monitor) = cfg_file.monitor_file(
            gtk4::gio::FileMonitorFlags::NONE,
            gtk4::gio::Cancellable::NONE,
        ) {
            monitor.connect_changed(|_, _, _, _| {
                malus_de_common::config::invalidate_appearance_settings_cache();
                apply_notification_theme_now(None);
            });
            NOTIF_GLASS_MONITOR.with(|cell| {
                *cell.borrow_mut() = Some(monitor);
            });
        }
    });

    app.connect_activate(|_| {});

    let app_handle = app.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(ev) = receiver.recv().await {
            match ev {
                NotificationEvent::Show { msg, record } => {
                    NOTIFICATION_HISTORY.with(|cell| cell.borrow_mut().push(record));
                    refresh_notification_center_if_open(&app_handle);
                    display_banner(&app_handle, msg);
                }
                NotificationEvent::RecordOnly(record) => {
                    NOTIFICATION_HISTORY.with(|cell| cell.borrow_mut().push(record));
                    refresh_notification_center_if_open(&app_handle);
                }
                NotificationEvent::Close(id) => {
                    NOTIFICATION_HISTORY.with(|cell| {
                        cell.borrow_mut().remove(id);
                    });
                    begin_slide_out_close(id);
                    refresh_notification_center_if_open(&app_handle);
                }
                NotificationEvent::ThemeChanged(is_dark) => {
                    malus_de_common::config::invalidate_appearance_settings_cache();
                    apply_notification_theme_now(Some(is_dark));
                }
                NotificationEvent::RunningAppsChanged(running_apps) => {
                    RUNNING_APPS_CACHE.with(|cell| {
                        *cell.borrow_mut() = running_apps;
                    });
                }
                NotificationEvent::ToggleCenter => {
                    toggle_notification_center(&app_handle, None);
                }
                NotificationEvent::SessionLockChanged(locked) => {
                    SESSION_LOCKED.with(|c| c.set(locked));
                    if locked {
                        hide_notification_center();
                    }
                }
            }
        }
    });

    app.run_with_args(&["malus-de-notifications"]);
    Ok(())
}

fn resolve_notification_icon(app_icon: &str, app_name: &str, pixel_size: i32) -> Image {
    let icon_from_path = |path: std::path::PathBuf| -> Image {
        let file = gio::File::for_path(path);
        let gicon = gio::FileIcon::new(&file);
        Image::from_gicon(&gicon)
    };
    let img = if !app_icon.is_empty() {
        if let Some(path) = malus_de_common::apps::resolve_icon_file_path(app_icon) {
            icon_from_path(path)
        } else {
            Image::from_icon_name(app_icon)
        }
    } else if !app_name.is_empty() {
        let lower = app_name.trim().to_ascii_lowercase();
        if let Some(path) = malus_de_common::apps::resolve_icon_file_path(&lower) {
            icon_from_path(path)
        } else {
            Image::from_icon_name("preferences-system-notifications-symbolic")
        }
    } else {
        Image::from_icon_name("preferences-system-notifications-symbolic")
    };
    img.set_pixel_size(pixel_size);
    img.set_size_request(pixel_size, pixel_size);
    img.set_halign(Align::Center);
    img.set_valign(Align::Center);
    img.add_css_class("notif-app-icon");
    img
}

fn build_thumbnail_widget(image: &NotificationImage) -> Option<DrawingArea> {
    let pixbuf = match image {
        NotificationImage::Path(path) => {
            if !path.is_file() {
                return None;
            }
            gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 96, 96, true).ok()?
        }
        NotificationImage::RawRgba {
            width,
            height,
            rowstride,
            has_alpha,
            data,
        } => {
            let bytes = glib::Bytes::from(data);
            gtk4::gdk_pixbuf::Pixbuf::from_bytes(
                &bytes,
                gtk4::gdk_pixbuf::Colorspace::Rgb,
                *has_alpha,
                8,
                *width,
                *height,
                *rowstride,
            )
        }
    };
    let pw = pixbuf.width() as f64;
    let ph = pixbuf.height() as f64;
    if pw <= 0.0 || ph <= 0.0 {
        return None;
    }

    let area = DrawingArea::builder()
        .content_width(44)
        .content_height(44)
        .halign(Align::End)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();
    area.set_size_request(44, 44);
    area.set_draw_func(move |_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        if wf <= 1.0 || hf <= 1.0 {
            return;
        }
        let scale = (wf / pw).max(hf / ph);
        let draw_w = pw * scale;
        let draw_h = ph * scale;
        let ox = (wf - draw_w) * 0.5;
        let oy = (hf - draw_h) * 0.5;

        squircle_path(cr, 0.0, 0.0, wf, hf, 10.0);
        cr.clip();
        cr.translate(ox, oy);
        cr.scale(scale, scale);
        cr.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        let _ = cr.paint();
    });
    Some(area)
}

/// Dibuja en Cairo el fondo squircle (`n = 4.2`, `r = 38`) y, cuando `stacked_count > 1`,
/// añade tarjetas translúcidas apiladas detrás al estilo de las pilas de macOS.
fn build_squircle_bg(stacked_count: usize) -> DrawingArea {
    let bg = DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .build();

    let extra_layers = stacked_count.saturating_sub(1).min(2);
    bg.set_draw_func(move |_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        let cx = CLOSE_OVERHANG_PX as f64;
        let cy = CLOSE_OVERHANG_PX as f64;
        let cw = wf - CLOSE_OVERHANG_PX as f64;
        let stack_pad = (extra_layers as f64) * 5.0;
        let ch = hf - CLOSE_OVERHANG_PX as f64 - stack_pad;
        if cw <= 4.0 || ch <= 4.0 {
            return;
        }

        let dark = NOTIF_DARK_ACTIVE.with(|c| c.get());
        let opacity = NOTIF_OPACITY.with(|c| c.get());
        let scale = malus_de_common::config::liquid_glass_scale_for(opacity);

        // Tarjetas apiladas detrás (de más profunda a más cercana)
        for depth in (1..=extra_layers).rev() {
            let inset_x = depth as f64 * 8.0;
            let offset_y = depth as f64 * 5.0;
            if cw > inset_x * 2.0 + 16.0 {
                squircle_path_with_exponent(
                    cr,
                    cx + inset_x,
                    cy + offset_y,
                    cw - inset_x * 2.0,
                    ch,
                    (CARD_SQUIRCLE_RADIUS - depth as f64 * 4.0).max(20.0),
                    CARD_SQUIRCLE_EXPONENT,
                );
                let factor = if depth == 2 { 0.42 } else { 0.64 };
                if dark {
                    let a = malus_de_common::config::scaled_liquid_glass_alpha(
                        0.66 * factor,
                        scale,
                        0.12,
                        0.72,
                    );
                    cr.set_source_rgba(30.0 / 255.0, 32.0 / 255.0, 40.0 / 255.0, a);
                } else {
                    let a = malus_de_common::config::scaled_liquid_glass_alpha(
                        0.62 * factor,
                        scale,
                        0.12,
                        0.72,
                    );
                    cr.set_source_rgba(232.0 / 255.0, 236.0 / 255.0, 244.0 / 255.0, a);
                }
                let _ = cr.fill();
            }
        }

        squircle_path_with_exponent(
            cr,
            cx,
            cy,
            cw,
            ch,
            CARD_SQUIRCLE_RADIUS,
            CARD_SQUIRCLE_EXPONENT,
        );
        if dark {
            let bg_a = malus_de_common::config::scaled_liquid_glass_alpha(0.66, scale, 0.24, 0.96);
            cr.set_source_rgba(34.0 / 255.0, 36.0 / 255.0, 44.0 / 255.0, bg_a);
        } else {
            let bg_a = malus_de_common::config::scaled_liquid_glass_alpha(0.62, scale, 0.22, 0.96);
            cr.set_source_rgba(244.0 / 255.0, 247.0 / 255.0, 252.0 / 255.0, bg_a);
        }
        let _ = cr.fill();
    });

    bg
}

fn build_center_squircle_bg() -> DrawingArea {
    let bg = DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .build();

    bg.set_draw_func(|_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        if wf <= 4.0 || hf <= 4.0 {
            return;
        }
        let dark = NOTIF_DARK_ACTIVE.with(|c| c.get());
        let opacity = NOTIF_OPACITY.with(|c| c.get());
        let scale = malus_de_common::config::liquid_glass_scale_for(opacity);

        squircle_path(cr, 0.0, 0.0, wf, hf, CENTER_SQUIRCLE_RADIUS);
        if dark {
            let bg_a = malus_de_common::config::scaled_liquid_glass_alpha(0.72, scale, 0.28, 0.96);
            cr.set_source_rgba(28.0 / 255.0, 30.0 / 255.0, 38.0 / 255.0, bg_a);
        } else {
            let bg_a = malus_de_common::config::scaled_liquid_glass_alpha(0.68, scale, 0.26, 0.96);
            cr.set_source_rgba(244.0 / 255.0, 247.0 / 255.0, 252.0 / 255.0, bg_a);
        }
        let _ = cr.fill();
    });

    bg
}

fn display_banner(app: &Application, msg: NotificationMsg) {
    let notif_cfg = malus_de_common::config::NotificationsConfig::load();
    let app_rule = notif_cfg.rule_for_app(&msg.app_name);
    if !app_rule.enabled {
        info!(
            "Notificación suprimida por preferencias del usuario para '{}'",
            msg.app_name
        );
        return;
    }

    if msg.sound && app_rule.sound {
        if let Some(display) = gtk4::gdk::Display::default() {
            display.beep();
        } else {
            tracing::warn!(
                "No se pudo emitir el sonido de notificación: no hay pantalla GTK disponible"
            );
        }
    }

    if app_rule.style == malus_de_common::config::NotificationAlertStyle::None {
        return;
    }

    apply_notification_theme_now(None);

    let preview_mode = notif_cfg.effective_preview_mode(&msg.app_name);
    let hide_preview = should_hide_preview(preview_mode, SESSION_LOCKED.get());

    // Agrupar si ya existe un banner activo visible de la misma aplicación
    let mut grouped_ids: Vec<u32> = Vec::new();
    let existing_group_win: Option<(ApplicationWindow, f64)> = ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        if let Some(pos) = list.iter().position(|e| {
            !e.closing_slide_out && e.window.is_some() && e.group_key == msg.group_key
        }) {
            let old = list.remove(pos);
            grouped_ids = old.grouped_ids;
            recompute_banner_targets_locked(&mut list);
            old.window.map(|w| (w, old.current_top))
        } else {
            None
        }
    });
    if let Some((old_win, _)) = existing_group_win.as_ref() {
        old_win.close();
    }
    if !grouped_ids.contains(&msg.id) {
        grouped_ids.push(msg.id);
    }
    let stacked_count = grouped_ids.len();

    close_banner_immediately(msg.id, false);

    let top_y = if let Some((_, prev_top)) = existing_group_win {
        prev_top.round() as i32
    } else {
        ACTIVE_BANNERS.with(|cell| {
            let list = cell.borrow();
            let mut y = BANNER_TOP_BASE;
            for entry in list.iter() {
                let card_h = entry_card_height(entry);
                y += card_h + BANNER_STACK_GAP;
            }
            y
        })
    };

    let initial_right = if existing_group_win.is_some() {
        BANNER_RIGHT_MARGIN
    } else {
        BANNER_OFFSCREEN_START_RIGHT.round() as i32
    };

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Notificación")
        .css_classes(["malus-notification-window"])
        .resizable(false)
        .build();

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-notifications");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, top_y);
        window.set_margin(Edge::Right, initial_right);
    } else {
        window.set_decorated(false);
    }

    let stack_bottom_pad = (stacked_count.saturating_sub(1).min(2) as i32) * 5;
    let banner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["notification-banner"])
        .spacing(0)
        .margin_start(CLOSE_OVERHANG_PX)
        .margin_top(CLOSE_OVERHANG_PX)
        .margin_bottom(stack_bottom_pad)
        .build();

    let main_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();

    main_row.append(&resolve_notification_icon(&msg.app_icon, &msg.app_name, 38));

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .valign(Align::Center)
        .hexpand(true)
        .build();

    let title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let title_text = if !msg.summary.trim().is_empty() {
        msg.summary.as_str()
    } else if !msg.app_name.trim().is_empty() {
        msg.app_name.as_str()
    } else {
        "Notificación"
    };
    let title = Label::builder()
        .label(title_text)
        .css_classes(["notif-title"])
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .max_width_chars(30)
        .build();
    title_row.append(&title);

    if stacked_count > 1 {
        let extra = stacked_count - 1;
        let stack_btn = Button::builder()
            .label(&format!("+{extra} más"))
            .css_classes(["notif-stack-pill"])
            .has_frame(false)
            .valign(Align::Start)
            .tooltip_text("Expandir notificaciones agrupadas")
            .build();
        let app_c = app.clone();
        let gk_c = msg.group_key.clone();
        stack_btn.connect_clicked(move |_| {
            close_banners_for_group(&gk_c);
            toggle_notification_center(&app_c, Some(gk_c.clone()));
        });
        title_row.append(&stack_btn);
    }
    text_col.append(&title_row);

    if hide_preview {
        let body = Label::builder()
            .label("Notificación")
            .css_classes(["notif-body"])
            .xalign(0.0)
            .build();
        text_col.append(&body);
    } else if !msg.body.trim().is_empty() {
        let body = Label::builder()
            .label(&msg.body)
            .css_classes(["notif-body"])
            .wrap(true)
            .xalign(0.0)
            .max_width_chars(34)
            .build();
        text_col.append(&body);
    } else if !msg.app_name.trim().is_empty() && msg.app_name.trim() != title_text.trim() {
        let body = Label::builder()
            .label(&msg.app_name)
            .css_classes(["notif-body"])
            .xalign(0.0)
            .build();
        text_col.append(&body);
    }

    main_row.append(&text_col);

    if !hide_preview {
        if let Some(ref image) = msg.image {
            if let Some(thumb) = build_thumbnail_widget(image) {
                main_row.append(&thumb);
            }
        }
    }

    banner.append(&main_row);

    let notif_id = msg.id;

    // Botones de acción interactivos (org.freedesktop.Notifications `actions`)
    if !hide_preview && !msg.actions.is_empty() {
        let actions_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .css_classes(["notif-actions-row"])
            .halign(Align::End)
            .build();
        for action in &msg.actions {
            let btn = Button::builder()
                .label(&action.label)
                .css_classes(["notif-action-btn"])
                .has_frame(false)
                .build();
            let action_key = action.key.clone();
            let app_c = app.clone();
            btn.connect_clicked(move |_| {
                emit_dbus_action(notif_id, &action_key);
                emit_dbus_closed(notif_id, 2);
                NOTIFICATION_HISTORY.with(|cell| {
                    cell.borrow_mut().remove(notif_id);
                });
                close_banner_immediately(notif_id, false);
                refresh_notification_center_if_open(&app_c);
            });
            actions_row.append(&btn);
        }
        banner.append(&actions_row);
    }

    // Botón circular ✕ flotando sobre la esquina superior izquierda del squircle (solo visible en hover)
    let close_icon = Image::builder()
        .icon_name("window-close-symbolic")
        .pixel_size(11)
        .build();
    let close_btn = Button::builder()
        .child(&close_icon)
        .css_classes(["notif-close-btn"])
        .has_frame(false)
        .halign(Align::Start)
        .valign(Align::Start)
        .can_target(false)
        .tooltip_text(if stacked_count > 1 {
            "Cerrar pila de notificaciones"
        } else {
            "Cerrar notificación"
        })
        .build();

    let bg = build_squircle_bg(stacked_count);
    let overlay = Overlay::new();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&banner);
    overlay.set_measure_overlay(&banner, true);
    overlay.add_overlay(&close_btn);

    let is_hovered = Rc::new(Cell::new(false));
    let motion = EventControllerMotion::new();
    {
        let btn_c = close_btn.clone();
        let hov_c = is_hovered.clone();
        motion.connect_enter(move |_, _, _| {
            hov_c.set(true);
            btn_c.add_css_class("visible");
            btn_c.set_can_target(true);
        });
    }
    {
        let btn_c = close_btn.clone();
        let hov_c = is_hovered.clone();
        motion.connect_leave(move |_| {
            hov_c.set(false);
            btn_c.remove_css_class("visible");
            btn_c.set_can_target(false);
        });
    }
    overlay.add_controller(motion);

    let has_target_app = !msg.desktop_entry.trim().is_empty()
        || !msg.app_name.trim().is_empty()
        || !msg.app_icon.trim().is_empty();
    if has_target_app {
        main_row.set_cursor_from_name(Some("pointer"));
        let click_gesture = GestureClick::new();
        click_gesture.set_button(1);
        let click_desktop = msg.desktop_entry.clone();
        let click_app_name = msg.app_name.clone();
        let click_app_icon = msg.app_icon.clone();
        let click_has_default = msg.has_default_action;
        let app_c = app.clone();
        click_gesture.connect_released(move |_, _, _, _| {
            activate_notification_source_app(
                notif_id,
                &click_desktop,
                &click_app_name,
                &click_app_icon,
                click_has_default,
            );
            emit_dbus_closed(notif_id, 2);
            NOTIFICATION_HISTORY.with(|cell| {
                cell.borrow_mut().remove(notif_id);
            });
            close_banner_immediately(notif_id, false);
            refresh_notification_center_if_open(&app_c);
        });
        main_row.add_controller(click_gesture);
    }

    {
        let ids_to_close = grouped_ids.clone();
        close_btn.connect_clicked(move |_| {
            for cid in &ids_to_close {
                emit_dbus_closed(*cid, 2);
            }
            close_banner_immediately(notif_id, true);
        });
    }
    window.connect_close_request(move |_| {
        ACTIVE_BANNERS.with(|cell| {
            let mut list = cell.borrow_mut();
            list.retain(|entry| entry.id != notif_id || entry.slot_hold_until.is_some());
            recompute_banner_targets_locked(&mut list);
        });
        glib::idle_add_local_once(ensure_animation_loop);
        glib::Propagation::Proceed
    });

    window.set_child(Some(&overlay));
    ACTIVE_BANNERS.with(|cell| {
        cell.borrow_mut().push(BannerEntry {
            id: notif_id,
            group_key: msg.group_key.clone(),
            grouped_ids,
            window: Some(window.clone()),
            bg: Some(bg),
            held_card_height: (BANNER_DEFAULT_HEIGHT - CLOSE_OVERHANG_PX).max(60),
            current_top: top_y as f64,
            target_top: top_y as f64,
            last_applied_top: top_y,
            current_right: initial_right as f64,
            target_right: BANNER_RIGHT_MARGIN as f64,
            last_applied_right: initial_right,
            closing_slide_out: false,
            slot_hold_until: None,
        });
    });
    window.present();
    ensure_animation_loop();
    glib::timeout_add_local_once(Duration::from_millis(32), recompute_banner_targets);

    if let Some(timeout_ms) = resolve_expire_timeout_ms(msg.expire_timeout, app_rule.style) {
        let win_auto = window.clone();
        let hov_auto = is_hovered.clone();
        glib::timeout_add_local(Duration::from_millis(timeout_ms), move || {
            if !win_auto.is_visible() {
                return ControlFlow::Break;
            }
            if hov_auto.get() {
                return ControlFlow::Continue;
            }
            begin_slide_out_close(notif_id);
            ControlFlow::Break
        });
    }
}

fn ensure_notification_center_window(app: &Application) {
    let already_exists = NOTIFICATION_CENTER.with(|cell| cell.borrow().is_some());
    if already_exists {
        return;
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Centro de Notificaciones")
        .css_classes(["malus-notification-center-window"])
        .resizable(false)
        .default_width(380)
        .default_height(560)
        .build();

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-notification-center");
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, 36);
        window.set_margin(Edge::Right, 12);
    } else {
        window.set_decorated(false);
    }

    let key_ctrl = EventControllerKey::new();
    {
        let win_c = window.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                win_c.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(key_ctrl);

    window.connect_close_request(|win| {
        win.set_visible(false);
        glib::Propagation::Stop
    });

    let shell = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["notif-center-shell"])
        .spacing(12)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let title = Label::builder()
        .label("Notificaciones")
        .css_classes(["notif-center-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let clear_btn = Button::builder()
        .label("Borrar todo")
        .css_classes(["notif-center-clear-btn"])
        .has_frame(false)
        .build();
    {
        let app_c = app.clone();
        clear_btn.connect_clicked(move |_| {
            NOTIFICATION_HISTORY.with(|cell| cell.borrow_mut().clear());
            EXPANDED_CENTER_GROUPS.with(|cell| cell.borrow_mut().clear());
            rebuild_notification_center_contents(&app_c);
        });
    }
    header.append(&title);
    header.append(&clear_btn);
    shell.append(&header);

    // Barra de estado de Enfoque activo
    let focus_bar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["notif-center-focus-bar"])
        .spacing(8)
        .visible(false)
        .build();
    let focus_ico = Image::builder()
        .icon_name("weather-clear-night-symbolic")
        .pixel_size(15)
        .build();
    let focus_lbl = Label::builder()
        .css_classes(["notif-center-focus-label"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let focus_off_btn = Button::builder()
        .label("Desactivar")
        .css_classes(["notif-action-btn"])
        .has_frame(false)
        .build();
    {
        let app_c = app.clone();
        focus_off_btn.connect_clicked(move |_| {
            let mut focus_cfg = malus_de_common::config::FocusConfig::load();
            focus_cfg.active_focus_id = None;
            let _ = focus_cfg.save();
            if let Some(source) = gio::SettingsSchemaSource::default() {
                if source
                    .lookup("org.gnome.desktop.notifications", true)
                    .is_some()
                {
                    let gs = gio::Settings::new("org.gnome.desktop.notifications");
                    let _ = gs.set_boolean("show-banners", true);
                }
            }
            rebuild_notification_center_contents(&app_c);
        });
    }
    focus_bar.append(&focus_ico);
    focus_bar.append(&focus_lbl);
    focus_bar.append(&focus_off_btn);
    shell.append(&focus_bar);

    let list_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .build();
    let scroller = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(430)
        .max_content_height(520)
        .propagate_natural_height(true)
        .child(&list_box)
        .build();
    shell.append(&scroller);

    let bg = build_center_squircle_bg();
    let overlay = Overlay::new();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&shell);
    overlay.set_measure_overlay(&shell, true);
    window.set_child(Some(&overlay));

    NOTIFICATION_CENTER.with(|cell| {
        *cell.borrow_mut() = Some(NotificationCenterRuntime {
            window,
            bg,
            list_box,
            focus_bar,
            focus_lbl,
            clear_btn,
        });
    });
}

fn hide_notification_center() {
    NOTIFICATION_CENTER.with(|cell| {
        if let Some(ref rt) = *cell.borrow() {
            rt.window.set_visible(false);
        }
    });
}

fn toggle_notification_center(app: &Application, expand_group: Option<String>) {
    ensure_notification_center_window(app);
    apply_notification_theme_now(None);

    if let Some(gk) = expand_group {
        EXPANDED_CENTER_GROUPS.with(|cell| {
            cell.borrow_mut().insert(gk);
        });
        rebuild_notification_center_contents(app);
        NOTIFICATION_CENTER.with(|cell| {
            if let Some(ref rt) = *cell.borrow() {
                rt.window.present();
            }
        });
        return;
    }

    let was_visible = NOTIFICATION_CENTER.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|rt| rt.window.is_visible())
    });
    if was_visible {
        hide_notification_center();
    } else {
        rebuild_notification_center_contents(app);
        NOTIFICATION_CENTER.with(|cell| {
            if let Some(ref rt) = *cell.borrow() {
                rt.window.present();
            }
        });
    }
}

fn refresh_notification_center_if_open(app: &Application) {
    let is_open = NOTIFICATION_CENTER.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|rt| rt.window.is_visible())
    });
    if is_open {
        rebuild_notification_center_contents(app);
    }
}

fn rebuild_notification_center_contents(app: &Application) {
    let groups = NOTIFICATION_HISTORY.with(|cell| cell.borrow().grouped());
    let focus_cfg = malus_de_common::config::FocusConfig::load();
    let active_focus = focus_cfg.active_mode_now().cloned();

    NOTIFICATION_CENTER.with(|cell| {
        let borrow = cell.borrow();
        let Some(ref rt) = *borrow else {
            return;
        };

        if let Some(mode) = active_focus {
            rt.focus_lbl
                .set_label(&format!("Enfoque activo: {}", mode.name));
            rt.focus_bar.set_visible(true);
        } else {
            rt.focus_bar.set_visible(false);
        }

        rt.clear_btn.set_visible(!groups.is_empty());

        while let Some(child) = rt.list_box.first_child() {
            rt.list_box.remove(&child);
        }

        if groups.is_empty() {
            let empty = Label::builder()
                .label("Sin notificaciones recientes")
                .css_classes(["notif-center-empty"])
                .halign(Align::Center)
                .build();
            rt.list_box.append(&empty);
            return;
        }

        for group in groups {
            let card = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .css_classes(["notif-center-group-card"])
                .spacing(6)
                .build();

            let is_expanded = group.items.len() <= 1
                || EXPANDED_CENTER_GROUPS.with(|s| s.borrow().contains(&group.group_key));

            // Cabecera de grupo por app
            let hdr = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .build();
            hdr.append(&resolve_notification_icon(&group.app_icon, &group.app_name, 22));
            let app_lbl = Label::builder()
                .label(&group.app_name)
                .css_classes(["notif-center-group-app"])
                .hexpand(true)
                .xalign(0.0)
                .build();
            hdr.append(&app_lbl);

            if let Some(first) = group.items.first() {
                let time_lbl = Label::builder()
                    .label(&first.timestamp_label)
                    .css_classes(["notif-meta-time"])
                    .build();
                hdr.append(&time_lbl);
            }

            if group.items.len() > 1 {
                let toggle_lbl = if is_expanded {
                    "Colapsar".to_string()
                } else {
                    format!("+{} más", group.items.len() - 1)
                };
                let expand_btn = Button::builder()
                    .label(&toggle_lbl)
                    .css_classes(["notif-stack-pill"])
                    .has_frame(false)
                    .build();
                let gk = group.group_key.clone();
                let app_c = app.clone();
                expand_btn.connect_clicked(move |_| {
                    EXPANDED_CENTER_GROUPS.with(|s| {
                        let mut set = s.borrow_mut();
                        if !set.remove(&gk) {
                            set.insert(gk.clone());
                        }
                    });
                    rebuild_notification_center_contents(&app_c);
                });
                hdr.append(&expand_btn);
            }

            let clear_group_btn = Button::builder()
                .child(
                    &Image::builder()
                        .icon_name("window-close-symbolic")
                        .pixel_size(11)
                        .build(),
                )
                .css_classes(["notif-center-item-close"])
                .has_frame(false)
                .tooltip_text("Borrar grupo")
                .build();
            {
                let gk = group.group_key.clone();
                let app_c = app.clone();
                clear_group_btn.connect_clicked(move |_| {
                    NOTIFICATION_HISTORY.with(|h| {
                        h.borrow_mut().remove_group(&gk);
                    });
                    EXPANDED_CENTER_GROUPS.with(|s| {
                        s.borrow_mut().remove(&gk);
                    });
                    close_banners_for_group(&gk);
                    rebuild_notification_center_contents(&app_c);
                });
            }
            hdr.append(&clear_group_btn);
            card.append(&hdr);

            let items_to_show = if is_expanded {
                group.items.as_slice()
            } else {
                &group.items[..1]
            };

            for (idx, rec) in items_to_show.iter().enumerate() {
                if idx > 0 {
                    let sep = GtkBox::builder()
                        .css_classes(["notif-center-item-sep"])
                        .hexpand(true)
                        .build();
                    card.append(&sep);
                }

                let item_box = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(4)
                    .build();

                let content_row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .build();
                content_row.set_cursor_from_name(Some("pointer"));

                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .build();

                let summary_row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(6)
                    .build();
                let sum_lbl = Label::builder()
                    .label(if rec.summary.trim().is_empty() {
                        rec.app_name.as_str()
                    } else {
                        rec.summary.as_str()
                    })
                    .css_classes(["notif-title"])
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .max_width_chars(30)
                    .build();
                summary_row.append(&sum_lbl);

                if rec.silenced_by_focus {
                    let badge = Label::builder()
                        .label("Enfoque")
                        .css_classes(["notif-center-silenced-badge"])
                        .valign(Align::Start)
                        .build();
                    summary_row.append(&badge);
                }
                col.append(&summary_row);

                if !rec.body.trim().is_empty() {
                    let body_lbl = Label::builder()
                        .label(&rec.body)
                        .css_classes(["notif-body"])
                        .xalign(0.0)
                        .wrap(true)
                        .max_width_chars(34)
                        .build();
                    col.append(&body_lbl);
                }
                content_row.append(&col);

                if let Some(ref image) = rec.image {
                    if let Some(thumb) = build_thumbnail_widget(image) {
                        content_row.append(&thumb);
                    }
                }

                {
                    let click = GestureClick::new();
                    click.set_button(1);
                    let rid = rec.id;
                    let de = rec.desktop_entry.clone();
                    let an = rec.app_name.clone();
                    let ai = rec.app_icon.clone();
                    let hd = rec.has_default_action;
                    let app_c = app.clone();
                    click.connect_released(move |_, _, _, _| {
                        activate_notification_source_app(rid, &de, &an, &ai, hd);
                        NOTIFICATION_HISTORY.with(|h| {
                            h.borrow_mut().remove(rid);
                        });
                        close_banner_immediately(rid, false);
                        hide_notification_center();
                        rebuild_notification_center_contents(&app_c);
                    });
                    content_row.add_controller(click);
                }

                item_box.append(&content_row);

                if !rec.actions.is_empty() {
                    let actions_row = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(6)
                        .halign(Align::End)
                        .build();
                    for action in &rec.actions {
                        let btn = Button::builder()
                            .label(&action.label)
                            .css_classes(["notif-action-btn"])
                            .has_frame(false)
                            .build();
                        let rid = rec.id;
                        let key = action.key.clone();
                        let app_c = app.clone();
                        btn.connect_clicked(move |_| {
                            emit_dbus_action(rid, &key);
                            emit_dbus_closed(rid, 2);
                            NOTIFICATION_HISTORY.with(|h| {
                                h.borrow_mut().remove(rid);
                            });
                            close_banner_immediately(rid, false);
                            rebuild_notification_center_contents(&app_c);
                        });
                        actions_row.append(&btn);
                    }
                    item_box.append(&actions_row);
                }

                card.append(&item_box);
            }

            rt.list_box.append(&card);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_css_adapts_for_light_and_dark_modes_without_white_border() {
        let light = build_notification_theme_css(false);
        assert!(light.contains("rgba(14, 16, 22, 0.94)"));
        assert!(!light.contains("border: 1px solid"));

        let dark = build_notification_theme_css(true);
        assert!(dark.contains("rgba(255, 255, 255, 0.95)"));
        assert!(!dark.contains("border: 1px solid"));
    }

    #[test]
    fn squircle_radius_and_damped_slide_animation_converge_smoothly() {
        assert!((CARD_SQUIRCLE_RADIUS - 44.0).abs() < f64::EPSILON);
        assert!((CARD_SQUIRCLE_EXPONENT - 2.4).abs() < f64::EPSILON);

        let mut x = BANNER_OFFSCREEN_START_RIGHT;
        let target = BANNER_RIGHT_MARGIN as f64;
        for _ in 0..45 {
            let next = step_damped(x, target, 0.016, ANIM_SPEED_SLIDE_IN);
            assert!(next >= x);
            assert!(next <= target);
            x = next;
        }
        assert!((x - target).abs() < 1e-6);

        let mut y = 240.0;
        let target_y = 34.0;
        for _ in 0..45 {
            let next = step_damped(y, target_y, 0.016, ANIM_SPEED_REFLOW);
            assert!(next <= y);
            assert!(next >= target_y);
            y = next;
        }
        assert!((y - target_y).abs() < 1e-6);
    }

    #[test]
    fn resolves_notification_click_target_and_default_action_cleanly() {
        assert!(has_default_notification_action(&[
            "default".to_string(),
            "Abrir".to_string(),
            "reply".to_string(),
            "Responder".to_string(),
        ]));
        assert!(!has_default_notification_action(&[
            "dismiss".to_string(),
            "Descartar".to_string(),
        ]));

        let running = vec![
            "org.mozilla.firefox".to_string(),
            "brave-origin".to_string(),
            "org.telegram.desktop".to_string(),
        ];
        assert_eq!(
            resolve_target_running_app(&running, "org.mozilla.firefox", "Firefox", "firefox")
                .as_deref(),
            Some("org.mozilla.firefox")
        );
        assert_eq!(
            resolve_target_running_app(&running, "", "Brave Origin", "brave-origin").as_deref(),
            Some("brave-origin")
        );
        assert_eq!(
            resolve_target_running_app(&running, "", "Telegram", "telegram").as_deref(),
            Some("org.telegram.desktop")
        );
        assert_eq!(
            resolve_target_running_app(&running, "", "App Inexistente", ""),
            None
        );

        assert!(should_skip_dock_push_badge("Malus DE", "", None, false));
        assert!(should_skip_dock_push_badge(
            "Firefox",
            "org.mozilla.firefox",
            Some("transfer.complete"),
            false
        ));
        assert!(!should_skip_dock_push_badge(
            "Firefox",
            "org.mozilla.firefox",
            None,
            false
        ));
    }
}
