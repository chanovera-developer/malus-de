use anyhow::Result;
use gio::prelude::ApplicationExtManual;
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea,
    EventControllerMotion, Image, Label, Orientation, Overlay,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::ipc::{ipc_socket_path, IpcMessage};
use malus_de_common::logging::init_logging;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
// El radio squircle (38 px, n=4.2) coincide exactamente con `NOTIFICATION_CORNER_RADIUS` del compositor.
const CLOSE_OVERHANG_PX: i32 = 8;
const CARD_SQUIRCLE_RADIUS: f64 = 38.0;
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

thread_local! {
    static NOTIF_HOLD_GUARD: RefCell<Option<gio::ApplicationHoldGuard>> =
        const { RefCell::new(None) };
    static NOTIF_THEME_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static NOTIF_GLASS_MONITOR: RefCell<Option<gtk4::gio::FileMonitor>> =
        const { RefCell::new(None) };
    static NOTIF_DARK_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static NOTIF_OPACITY: Cell<f64> = const { Cell::new(0.55) };
    static ACTIVE_BANNERS: RefCell<Vec<BannerEntry>> = const { RefCell::new(Vec::new()) };
    static ANIM_RUNNING: Cell<bool> = const { Cell::new(false) };
    static ANIM_LAST_TICK: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn squircle_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.5 {
        cr.rectangle(x, y, w, h);
        return;
    }

    let steps = 18usize;
    let inv_n = 1.0 / SQUIRCLE_EXPONENT;
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
.notif-title {
    color: rgba(255, 255, 255, 0.95);
}
.notif-body {
    color: rgba(235, 238, 246, 0.86);
}
.notif-app-icon,
button.notif-close-btn,
button.notif-close-btn image {
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
"#
        .to_string()
    } else {
        r#"
.notif-title {
    color: rgba(14, 16, 22, 0.94);
}
.notif-body {
    color: rgba(18, 20, 26, 0.86);
}
.notif-app-icon,
button.notif-close-btn,
button.notif-close-btn image {
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
        // Mantener reservado el espacio vertical tanto mientras un globo se desliza fuera
        // como mientras su efecto Agujero Negro sigue ejecutándose, para que los globos
        // inferiores no se muevan antes de que termine de cerrarse el globo superior.
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

            // Liberar espacios reservados cuyo efecto Agujero Negro ya concluyó
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

/// Inicia la salida deslizada suave hacia la derecha (para auto-desvanecimiento o cierre D-Bus).
/// Los globos inferiores conservan su posición hasta que éste termine de salir de pantalla.
fn begin_slide_out_close(id: u32) {
    let triggered = ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        let mut found = false;
        for entry in list.iter_mut() {
            if entry.id == id && !entry.closing_slide_out && entry.window.is_some() {
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

/// Cierra inmediatamente el globo visible (activando el efecto Agujero Negro en el compositor).
/// Si `hold_slot_for_black_hole` es `true`, mantiene reservado su hueco vertical hasta que
/// termine el efecto Agujero Negro antes de deslizar hacia arriba los globos inferiores.
fn close_banner_immediately(id: u32, hold_slot_for_black_hole: bool) {
    let target = ACTIVE_BANNERS.with(|cell| {
        let mut list = cell.borrow_mut();
        if let Some(pos) = list.iter().position(|entry| entry.id == id) {
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

pub struct NotificationServer {
    sender: async_channel::Sender<NotificationEvent>,
}

#[derive(Debug, Clone)]
pub enum NotificationEvent {
    Show(NotificationMsg),
    Close(u32),
    ThemeChanged(bool),
}

#[derive(Debug, Clone)]
pub struct NotificationMsg {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub sound: bool,
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
        _actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        _expire_timeout: i32,
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

        if rule.badge_icon
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

        if rule.style != malus_de_common::config::NotificationAlertStyle::None {
            let msg = NotificationMsg {
                id,
                app_name,
                app_icon,
                summary,
                body,
                sound: rule.sound
                    && (hints.contains_key("sound-name") || !hints.contains_key("suppress-sound")),
            };

            let _ = self.sender.send(NotificationEvent::Show(msg)).await;
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
            std::future::pending::<()>().await;
            Ok::<(), zbus::Error>(())
        });
        if let Err(e) = rt {
            tracing::warn!("Aviso al enlazar D-Bus Notifications: {:?}", e);
        }
    });

    // Escuchar cambios de tema / opacidad del compositor por IPC
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
                NotificationEvent::Show(msg) => display_banner(&app_handle, msg),
                NotificationEvent::Close(id) => begin_slide_out_close(id),
                NotificationEvent::ThemeChanged(is_dark) => {
                    malus_de_common::config::invalidate_appearance_settings_cache();
                    apply_notification_theme_now(Some(is_dark));
                }
            }
        }
    });

    app.run_with_args(&["malus-de-notifications"]);
    Ok(())
}

fn resolve_notification_icon(app_icon: &str, app_name: &str) -> Image {
    let img = if !app_icon.is_empty() {
        if let Some(path) = malus_de_common::apps::resolve_icon_file_path(app_icon) {
            Image::from_file(path)
        } else {
            Image::from_icon_name(app_icon)
        }
    } else if !app_name.is_empty() {
        let lower = app_name.trim().to_ascii_lowercase();
        if let Some(path) = malus_de_common::apps::resolve_icon_file_path(&lower) {
            Image::from_file(path)
        } else {
            Image::from_icon_name("preferences-system-notifications-symbolic")
        }
    } else {
        Image::from_icon_name("preferences-system-notifications-symbolic")
    };
    img.set_pixel_size(38);
    img.set_valign(Align::Center);
    img.add_css_class("notif-app-icon");
    img
}

/// Dibuja en Cairo el fondo squircle (`n = 4.2`, `r = 28`, idéntico al OSD de audio/brillo)
/// sin borde blanco ni brillo; la sombra GPU la renderiza el compositor con `render_menu_shadow_alpha`.
fn build_squircle_bg() -> DrawingArea {
    let bg = DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .build();

    bg.set_draw_func(|_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        let cx = CLOSE_OVERHANG_PX as f64;
        let cy = CLOSE_OVERHANG_PX as f64;
        let cw = wf - CLOSE_OVERHANG_PX as f64;
        let ch = hf - CLOSE_OVERHANG_PX as f64;
        if cw <= 4.0 || ch <= 4.0 {
            return;
        }

        let dark = NOTIF_DARK_ACTIVE.with(|c| c.get());
        let opacity = NOTIF_OPACITY.with(|c| c.get());
        let scale = malus_de_common::config::liquid_glass_scale_for(opacity);

        squircle_path(cr, cx, cy, cw, ch, CARD_SQUIRCLE_RADIUS);
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
    let hide_preview = preview_mode == malus_de_common::config::NotificationPreviewMode::Never;

    close_banner_immediately(msg.id, false);

    let top_y = ACTIVE_BANNERS.with(|cell| {
        let list = cell.borrow();
        let mut y = BANNER_TOP_BASE;
        for entry in list.iter() {
            let card_h = entry_card_height(entry);
            y += card_h + BANNER_STACK_GAP;
        }
        y
    });

    let initial_right = BANNER_OFFSCREEN_START_RIGHT.round() as i32;

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

    // Contenido horizontal del globo con margen de 8 px arriba/izquierda para el botón ✕ flotante
    let banner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["notification-banner"])
        .spacing(12)
        .margin_start(CLOSE_OVERHANG_PX)
        .margin_top(CLOSE_OVERHANG_PX)
        .build();

    banner.append(&resolve_notification_icon(&msg.app_icon, &msg.app_name));

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .valign(Align::Center)
        .hexpand(true)
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
        .wrap(true)
        .max_width_chars(34)
        .build();
    text_col.append(&title);

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
            .max_width_chars(36)
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
    banner.append(&text_col);

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
        .tooltip_text("Cerrar notificación")
        .build();

    let bg = build_squircle_bg();
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

    let notif_id = msg.id;
    close_btn.connect_clicked(move |_| {
        close_banner_immediately(notif_id, true);
    });
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
            window: Some(window.clone()),
            bg: Some(bg),
            held_card_height: (BANNER_DEFAULT_HEIGHT - CLOSE_OVERHANG_PX).max(60),
            current_top: top_y as f64,
            target_top: top_y as f64,
            last_applied_top: top_y,
            current_right: BANNER_OFFSCREEN_START_RIGHT,
            target_right: BANNER_RIGHT_MARGIN as f64,
            last_applied_right: initial_right,
            closing_slide_out: false,
            slot_hold_until: None,
        });
    });
    window.present();
    ensure_animation_loop();
    // Una vez GTK mida la altura real del contenido, recalcular los destinos de la pila con animación.
    glib::timeout_add_local_once(Duration::from_millis(32), recompute_banner_targets);

    if app_rule.style == malus_de_common::config::NotificationAlertStyle::Banners {
        let win_auto = window.clone();
        let hov_auto = is_hovered.clone();
        glib::timeout_add_local(Duration::from_secs(6), move || {
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
        assert!((CARD_SQUIRCLE_RADIUS - 38.0).abs() < f64::EPSILON);

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
}
