mod auth;
mod blur_bg;
mod intrusion;
mod lock_protocol;
mod scale_fade;

use anyhow::Result;
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, ContentFit, CssProvider,
    DrawingArea, EventControllerKey, EventControllerMotion, GestureClick, Image, Label,
    Orientation, Overlay, PasswordEntry, Picture, Spinner,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::{
    config::{DisplaysConfig, LockScreenConfig},
    is_spanish_locale,
    logging::init_logging,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use tracing::{error, info};

const LOCK_CSS: &str = include_str!("styles/lock.css");
const CONTROL_HEIGHT: i32 = 34;
const MAX_CONTROL_HEIGHT: i32 = 37;
const AVATAR_START_SIZE: f64 = 38.0;
const AVATAR_END_SIZE: f64 = 58.0;
const SQUIRCLE_EXPONENT: f64 = 4.2;
const SQUIRCLE_SEGMENTS: usize = 24;
const CONTROL_SQUIRCLE_RADIUS: f64 = 11.0;

struct LockSessionState {
    unlocked: Cell<bool>,
    verifying: Cell<bool>,
    revealed: Cell<bool>,
    cooldown_timer_active: Cell<bool>,
    windows: RefCell<Vec<ApplicationWindow>>,
    backdrop_overlays: RefCell<Vec<Overlay>>,
    primary_entry: RefCell<Option<PasswordEntry>>,
    reveal_login_cb: RefCell<Option<Rc<dyn Fn()>>>,
    protocol_lock: RefCell<Option<lock_protocol::SessionLockGuard>>,
    intrusion: RefCell<intrusion::IntrusionState>,
}

impl LockSessionState {
    fn new(
        protocol_lock: Option<lock_protocol::SessionLockGuard>,
        intrusion: intrusion::IntrusionState,
    ) -> Rc<Self> {
        let has_intrusion = intrusion.failed_attempts > 0;
        Rc::new(Self {
            unlocked: Cell::new(false),
            verifying: Cell::new(false),
            revealed: Cell::new(has_intrusion),
            cooldown_timer_active: Cell::new(false),
            windows: RefCell::new(Vec::new()),
            backdrop_overlays: RefCell::new(Vec::new()),
            primary_entry: RefCell::new(None),
            reveal_login_cb: RefCell::new(None),
            protocol_lock: RefCell::new(protocol_lock),
            intrusion: RefCell::new(intrusion),
        })
    }

    fn close_all_windows(&self) {
        self.backdrop_overlays.borrow_mut().clear();
        for win in self.windows.borrow_mut().drain(..) {
            win.close();
        }
        *self.primary_entry.borrow_mut() = None;
        *self.reveal_login_cb.borrow_mut() = None;
    }
}

fn main() -> Result<()> {
    init_logging("malus_de_session_lock");
    info!("Iniciando Malus DE Session Lock (Lock Screen)...");

    let protocol_lock = match lock_protocol::SessionLockGuard::acquire() {
        Ok(guard) => guard,
        Err(lock_protocol::LockRejection::AlreadyLocked) => {
            tracing::warn!(
                "La sesión ya cuenta con un bloqueo ext-session-lock-v1 activo; saliendo."
            );
            return Ok(());
        }
        Err(lock_protocol::LockRejection::ConnectionFailed(err)) => {
            tracing::warn!("No se pudo adquirir ext-session-lock-v1 ({err}); continuando con respaldo layer-shell");
            None
        }
    };
    let initial_intrusion = intrusion::load_state_from(&intrusion::intrusion_state_path());

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }
    adw::StyleManager::default().set_color_scheme(
        if malus_de_common::config::is_dark_mode_active() {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        },
    );

    let app = Application::builder()
        .application_id("org.malus_de.SessionLock")
        .build();

    app.connect_startup(|_| {
        let provider = CssProvider::new();
        provider.load_from_string(LOCK_CSS);
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    let state = LockSessionState::new(protocol_lock, initial_intrusion);
    let state_act = state.clone();
    app.connect_activate(move |application| {
        if !state_act.windows.borrow().is_empty() {
            if let Some(entry) = state_act.primary_entry.borrow().as_ref() {
                if state_act.revealed.get() {
                    entry.grab_focus();
                }
            }
            return;
        }
        build_lock_ui(application, &state_act);
    });

    app.run_with_args(&["malus-de-session-lock"]);
    Ok(())
}

fn enumerate_monitors(display: &gtk4::gdk::Display) -> (Vec<gtk4::gdk::Monitor>, usize) {
    let model = display.monitors();
    let monitors: Vec<gtk4::gdk::Monitor> = (0..model.n_items())
        .filter_map(|index| model.item(index))
        .filter_map(|monitor| monitor.downcast().ok())
        .collect();

    let displays_config = DisplaysConfig::load();
    let primary_config = displays_config.outputs.iter().find(|output| output.primary);
    let primary_index = primary_config
        .and_then(|config| {
            monitors.iter().position(|monitor| {
                if monitor
                    .connector()
                    .is_some_and(|connector| connector.as_str() == config.connector)
                {
                    return true;
                }
                if monitor.model().is_some_and(|model| {
                    model.as_str() == config.name || model.as_str() == config.connector
                }) {
                    return true;
                }
                if monitor.description().is_some_and(|description| {
                    description.contains(&config.name) || description.contains(&config.connector)
                }) {
                    return true;
                }
                let geometry = monitor.geometry();
                geometry.x() == config.x && geometry.y() == config.y
            })
        })
        .unwrap_or(0);

    (monitors, primary_index)
}

fn build_lock_ui(app: &Application, state: &Rc<LockSessionState>) {
    let Some(display) = gtk4::gdk::Display::default() else {
        build_lock_window(app, None, true, state);
        return;
    };

    rebuild_lock_windows(app, &display, state);

    let app_weak = app.downgrade();
    let state_mon = state.clone();
    let display_mon = display.clone();
    display.monitors().connect_items_changed(move |_, _, _, _| {
        if state_mon.unlocked.get() {
            return;
        }
        if let Some(app) = app_weak.upgrade() {
            rebuild_lock_windows(&app, &display_mon, &state_mon);
        }
    });
}

fn rebuild_lock_windows(
    app: &Application,
    display: &gtk4::gdk::Display,
    state: &Rc<LockSessionState>,
) {
    state.close_all_windows();
    let (monitors, primary_index) = enumerate_monitors(display);
    if monitors.is_empty() {
        build_lock_window(app, None, true, state);
        return;
    }

    for (idx, monitor) in monitors.iter().enumerate() {
        if idx != primary_index {
            build_lock_window(app, Some(monitor), false, state);
        }
    }
    if let Some(primary) = monitors.get(primary_index) {
        build_lock_window(app, Some(primary), true, state);
    }
}

fn squircle_path(context: &gtk4::cairo::Context, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }
    context.close_path();
}

/// Pinta la región correspondiente del wallpaper difuminado debajo del control squircle
/// para lograr un efecto vidrio/blur real independiente de las ventanas detrás.
fn paint_frosted_backdrop(
    cr: &gtk4::cairo::Context,
    widget: &impl IsA<gtk4::Widget>,
    root_overlay: &Overlay,
    assets: Option<&blur_bg::LockWallpaperAssets>,
) {
    let Some(assets) = assets else { return };
    let Some(bounds) = widget.compute_bounds(root_overlay) else {
        return;
    };
    let root_w = root_overlay.width().max(1) as f64;
    let root_h = root_overlay.height().max(1) as f64;
    let sx = root_w / (assets.blur_width.max(1) as f64);
    let sy = root_h / (assets.blur_height.max(1) as f64);

    if cr.save().is_ok() {
        cr.translate(-(bounds.x() as f64), -(bounds.y() as f64));
        cr.scale(sx, sy);
        if cr
            .set_source_surface(&assets.blurred_surface, 0.0, 0.0)
            .is_ok()
        {
            cr.source().set_filter(gtk4::cairo::Filter::Bilinear);
            let _ = cr.paint();
        }
        let _ = cr.restore();
    }
}

fn create_squircle_entry_shell(
    entry: &PasswordEntry,
    is_error: &Rc<Cell<bool>>,
    root_overlay: &Overlay,
    assets: Option<blur_bg::LockWallpaperAssets>,
) -> Overlay {
    debug_assert!(CONTROL_HEIGHT <= MAX_CONTROL_HEIGHT);
    let shell = Overlay::builder()
        .width_request(232)
        .height_request(CONTROL_HEIGHT)
        .halign(Align::Center)
        .valign(Align::Center)
        .vexpand(false)
        .css_classes(["lock-squircle-shell"])
        .build();

    let bg = DrawingArea::builder().hexpand(true).vexpand(true).build();
    let entry_focus = entry.clone();
    let err_flag = is_error.clone();
    let shell_weak = shell.downgrade();
    let root_weak = root_overlay.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let w = w as f64;
        let h = h as f64;
        if w <= 2.0 || h <= 2.0 {
            return;
        }
        let focused = entry_focus.has_focus();
        let has_err = err_flag.get();

        if cr.save().is_ok() {
            squircle_path(cr, w, h, CONTROL_SQUIRCLE_RADIUS);
            cr.clip();
            if let (Some(sh), Some(root)) = (shell_weak.upgrade(), root_weak.upgrade()) {
                paint_frosted_backdrop(cr, &sh, &root, assets.as_ref());
            }
            // Velo claro translúcido tipo macOS Vibrancy + leve tinte oscuro de contraste
            cr.set_source_rgba(0.12, 0.14, 0.20, 0.22);
            let _ = cr.paint();
            if focused {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
            }
            let _ = cr.paint();
            let _ = cr.restore();
        }

        if cr.save().is_ok() {
            let inset = 0.75;
            cr.translate(inset, inset);
            squircle_path(
                cr,
                (w - inset * 2.0).max(1.0),
                (h - inset * 2.0).max(1.0),
                (CONTROL_SQUIRCLE_RADIUS - inset).max(2.0),
            );
            if has_err {
                cr.set_source_rgba(1.0, 0.32, 0.28, 0.95);
                cr.set_line_width(1.6);
            } else if focused {
                cr.set_source_rgba(0.28, 0.65, 1.0, 0.95);
                cr.set_line_width(1.5);
            } else {
                let border_grad = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
                border_grad.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.52);
                border_grad.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.18);
                let _ = cr.set_source(&border_grad);
                cr.set_line_width(1.1);
            }
            let _ = cr.stroke();
            let _ = cr.restore();
        }
    });

    let bg_focus = bg.clone();
    entry.connect_has_focus_notify(move |_| {
        bg_focus.queue_draw();
    });

    shell.set_child(Some(&bg));
    shell.add_overlay(entry);
    shell
}

fn create_squircle_button_shell(
    btn: &Button,
    root_overlay: &Overlay,
    assets: Option<blur_bg::LockWallpaperAssets>,
) -> Overlay {
    debug_assert!(CONTROL_HEIGHT <= MAX_CONTROL_HEIGHT);
    let shell = Overlay::builder()
        .width_request(CONTROL_HEIGHT)
        .height_request(CONTROL_HEIGHT)
        .halign(Align::Center)
        .valign(Align::Center)
        .vexpand(false)
        .css_classes(["lock-squircle-shell"])
        .build();

    let hovered = Rc::new(Cell::new(false));
    let bg = DrawingArea::builder().hexpand(true).vexpand(true).build();
    let hov_draw = hovered.clone();
    let btn_draw = btn.clone();
    let shell_weak = shell.downgrade();
    let root_weak = root_overlay.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let w = w as f64;
        let h = h as f64;
        if w <= 2.0 || h <= 2.0 {
            return;
        }
        let sensitive = btn_draw.is_sensitive();
        let active = btn_draw.state_flags().contains(gtk4::StateFlags::ACTIVE);
        if cr.save().is_ok() {
            squircle_path(cr, w, h, CONTROL_SQUIRCLE_RADIUS);
            cr.clip();
            if let (Some(sh), Some(root)) = (shell_weak.upgrade(), root_weak.upgrade()) {
                paint_frosted_backdrop(cr, &sh, &root, assets.as_ref());
            }
            if !sensitive {
                cr.set_source_rgba(0.14, 0.52, 0.98, 0.45);
            } else if active {
                cr.set_source_rgba(0.05, 0.42, 0.90, 0.88);
            } else if hov_draw.get() {
                cr.set_source_rgba(0.16, 0.58, 1.0, 0.88);
            } else {
                cr.set_source_rgba(0.10, 0.50, 0.98, 0.78);
            }
            let _ = cr.paint();
            let _ = cr.restore();
        }

        if cr.save().is_ok() {
            let inset = 0.6;
            cr.translate(inset, inset);
            squircle_path(
                cr,
                (w - inset * 2.0).max(1.0),
                (h - inset * 2.0).max(1.0),
                (CONTROL_SQUIRCLE_RADIUS - inset).max(2.0),
            );
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.32);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let _ = cr.restore();
        }
    });

    let motion = EventControllerMotion::new();
    let hov_enter = hovered.clone();
    let bg_enter = bg.clone();
    motion.connect_enter(move |_, _, _| {
        hov_enter.set(true);
        bg_enter.queue_draw();
    });
    let hov_leave = hovered;
    let bg_leave = bg.clone();
    motion.connect_leave(move |_| {
        hov_leave.set(false);
        bg_leave.queue_draw();
    });
    btn.add_controller(motion);

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    shell.set_child(Some(&bg));
    shell.add_overlay(btn);
    shell
}

fn load_avatar_pixbuf(path: &std::path::Path, target_px: i32) -> Option<gdk_pixbuf::Pixbuf> {
    let loaded = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let pixbuf = loaded.apply_embedded_orientation().unwrap_or(loaded);
    let (sw, sh, cx, cy) =
        blur_bg::compute_cover_crop_geometry(pixbuf.width(), pixbuf.height(), target_px, target_px);
    let scaled = pixbuf.scale_simple(sw, sh, gdk_pixbuf::InterpType::Bilinear)?;
    scaled.new_subpixbuf(cx, cy, target_px, target_px).copy()
}

fn create_animated_avatar_widget(username: &str, scale_progress: &Rc<Cell<f64>>) -> DrawingArea {
    let px = (AVATAR_END_SIZE.round() as i32) * 2;
    let pixbuf = find_user_avatar_path(username).and_then(|p| load_avatar_pixbuf(&p, px));
    let initial = AVATAR_START_SIZE.round() as i32;
    let area = DrawingArea::builder()
        .width_request(initial)
        .height_request(initial)
        .halign(Align::Center)
        .valign(Align::Center)
        .vexpand(false)
        .hexpand(false)
        .build();

    let prog = scale_progress.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let size = (w.min(h) as f64).max(2.0);
        let cx = w as f64 * 0.5;
        let cy = h as f64 * 0.5;
        let radius = (size * 0.5) - 1.5;
        if radius <= 1.0 {
            return;
        }
        let _ = prog.get();

        if cr.save().is_ok() {
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            cr.clip();
            if let Some(ref pb) = pixbuf {
                let scale = (radius * 2.0) / (pb.width().max(1) as f64);
                cr.translate(cx - radius, cy - radius);
                cr.scale(scale, scale);
                cr.set_source_pixbuf(pb, 0.0, 0.0);
                cr.source().set_filter(gtk4::cairo::Filter::Bilinear);
                let _ = cr.paint();
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                let _ = cr.paint();
                // Silueta simple si no hay foto configurada
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.90);
                cr.arc(
                    cx,
                    cy - radius * 0.20,
                    radius * 0.28,
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
                cr.arc(
                    cx,
                    cy + radius * 0.72,
                    radius * 0.56,
                    std::f64::consts::PI,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
            }
            let _ = cr.restore();
        }

        if cr.save().is_ok() {
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
            let _ = cr.restore();
        }
    });
    area
}

/// Curva elástica amortiguada estilo macOS Sequoia (ligero sobreimpulso orgánico ~1.04).
fn sequoia_spring(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let raw = 1.0 - (-7.2 * t).exp() * (8.6 * t).cos();
    let end = 1.0 - (-7.2_f64).exp() * (8.6_f64).cos();
    raw / end
}

/// Entrada estilo macOS Sequoia: el fondo hace cross-dissolve limpio (`scale = 1.0`),
/// mientras la UI del reloj entra con un leve asentamiento (`scale 1.04 → 1.0` + `opacity 0 → 1`).
fn animate_lock_enter(
    window: &ApplicationWindow,
    backdrop: &Overlay,
    ui_container: Option<&scale_fade::ScaleFadeBox>,
) {
    backdrop.set_opacity(0.0);
    if let Some(ui) = ui_container {
        ui.set_scale_and_opacity(1.04, 0.0);
    }
    let start_time: Cell<Option<i64>> = Cell::new(None);
    let bg_tick = backdrop.clone();
    let ui_tick = ui_container.cloned();
    window.add_tick_callback(move |_, clock| {
        const BG_US: f64 = 240_000.0;
        const UI_US: f64 = 320_000.0;
        let now = clock.frame_time();
        let t0 = match start_time.get() {
            Some(v) => v,
            None => {
                start_time.set(Some(now));
                now
            }
        };
        let dt = (now - t0) as f64;
        let bg_t = (dt / BG_US).clamp(0.0, 1.0);
        let bg_eased = 1.0 - (1.0 - bg_t).powi(3);
        bg_tick.set_opacity(bg_eased);

        let ui_t = (dt / UI_US).clamp(0.0, 1.0);
        let ui_eased = 1.0 - (1.0 - ui_t).powi(3);
        if let Some(ref ui) = ui_tick {
            let scale = 1.04 - 0.04 * (ui_eased as f32);
            ui.set_scale_and_opacity(scale, ui_eased);
        }

        if ui_t >= 1.0 {
            bg_tick.set_opacity(1.0);
            if let Some(ref ui) = ui_tick {
                ui.set_scale_and_opacity(1.0, 1.0);
            }
            ControlFlow::Break
        } else {
            ControlFlow::Continue
        }
    });
}

/// Salida estilo macOS Sequoia: el reloj y el login se elevan y desvanecen (`scale 1.0 → 1.045` + `opacity 1 → 0`),
/// y el wallpaper hace cross-dissolve puro hacia el escritorio (`opacity 1 → 0`).
fn animate_unlock_and_quit(
    app: &Application,
    state: &Rc<LockSessionState>,
    primary_win: &ApplicationWindow,
    primary_ui: &scale_fade::ScaleFadeBox,
) {
    state.unlocked.set(true);
    let windows: Vec<ApplicationWindow> = state.windows.borrow().clone();
    let backdrops: Vec<Overlay> = state.backdrop_overlays.borrow().clone();
    for win in &windows {
        if gtk4_layer_shell::is_supported() {
            win.set_keyboard_mode(KeyboardMode::None);
        }
    }

    let finished = Rc::new(Cell::new(false));
    let finish_app = app.clone();
    let finish_state = state.clone();
    let finish_flag = finished.clone();
    let finish_fn: Rc<dyn Fn()> = Rc::new(move || {
        if finish_flag.replace(true) {
            return;
        }
        if let Some(mut guard) = finish_state.protocol_lock.borrow_mut().take() {
            guard.unlock_and_destroy();
        }
        finish_state.close_all_windows();
        finish_app.quit();
    });

    let fallback_done = finish_fn.clone();
    glib::timeout_add_local_once(Duration::from_millis(340), move || {
        fallback_done();
    });

    let start_time: Cell<Option<i64>> = Cell::new(None);
    let ui = primary_ui.clone();
    let tick_done = finish_fn;
    primary_win.add_tick_callback(move |_, clock| {
        const UI_US: f64 = 190_000.0;
        const BG_US: f64 = 240_000.0;
        let now = clock.frame_time();
        let t0 = match start_time.get() {
            Some(v) => v,
            None => {
                start_time.set(Some(now));
                now
            }
        };
        let dt = (now - t0) as f64;
        let ui_t = (dt / UI_US).clamp(0.0, 1.0);
        let ui_eased = 1.0 - (1.0 - ui_t).powi(2);
        let ui_scale = 1.0 + 0.045 * (ui_eased as f32);
        let ui_alpha = (1.0 - ui_eased).clamp(0.0, 1.0);
        ui.set_scale_and_opacity(ui_scale, ui_alpha);

        let bg_t = (dt / BG_US).clamp(0.0, 1.0);
        let bg_eased = 1.0 - (1.0 - bg_t).powi(3);
        let bg_alpha = (1.0 - bg_eased).clamp(0.0, 1.0);
        for bg in &backdrops {
            bg.set_opacity(bg_alpha);
        }

        if bg_t >= 1.0 {
            tick_done();
            ControlFlow::Break
        } else {
            ControlFlow::Continue
        }
    });
}

fn find_user_avatar_path(username: &str) -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("/var/lib/AccountsService/icons").join(username)];
    if let Ok(home) = std::env::var("HOME") {
        let h = PathBuf::from(home);
        candidates.push(h.join(".face"));
        candidates.push(h.join(".face.icon"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn trigger_shake(widget: &impl IsA<gtk4::Widget>) {
    let widget = widget.as_ref().clone();
    let offsets = [12, -12, 9, -9, 5, -5, 2, 0];
    let step = Rc::new(Cell::new(0usize));
    glib::timeout_add_local(Duration::from_millis(32), move || {
        let idx = step.get();
        if idx >= offsets.len() {
            widget.set_margin_start(0);
            widget.set_margin_end(0);
            return ControlFlow::Break;
        }
        let x = offsets[idx];
        if x >= 0 {
            widget.set_margin_start(x);
            widget.set_margin_end(0);
        } else {
            widget.set_margin_start(0);
            widget.set_margin_end(-x);
        }
        step.set(idx + 1);
        ControlFlow::Continue
    });
}

fn build_lock_window(
    app: &Application,
    monitor: Option<&gtk4::gdk::Monitor>,
    is_primary: bool,
    state: &Rc<LockSessionState>,
) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bloqueo de Sesión")
        .css_classes(["malus-lock-window"])
        .build();

    let state_close = state.clone();
    window.connect_close_request(move |_| {
        if state_close.unlocked.get() {
            glib::Propagation::Proceed
        } else {
            glib::Propagation::Stop
        }
    });

    if gtk4_layer_shell::is_supported() {
        info!(
            "Bloqueando pantalla (principal={}) con Layer::Overlay...",
            is_primary
        );
        window.init_layer_shell();
        if let Some(monitor) = monitor {
            window.set_monitor(monitor);
        }
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-lock");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_keyboard_mode(if is_primary {
            KeyboardMode::Exclusive
        } else {
            KeyboardMode::None
        });
        window.set_exclusive_zone(-1);
    } else {
        if let Some(monitor) = monitor {
            window.set_monitor(monitor);
        }
        window.set_decorated(false);
        window.maximize();
    }

    let (mon_w, mon_h) = monitor
        .map(|m| {
            let g = m.geometry();
            let scale = m.scale_factor().max(1);
            (g.width().max(1) * scale, g.height().max(1) * scale)
        })
        .unwrap_or((1920, 1080));

    let root_overlay = Overlay::builder().hexpand(true).vexpand(true).build();

    let assets = blur_bg::build_lock_wallpaper_assets(mon_w, mon_h);
    if let Some(ref wp) = assets {
        let bg_pic = Picture::builder()
            .paintable(&wp.crisp_texture)
            .content_fit(ContentFit::Cover)
            .hexpand(true)
            .vexpand(true)
            .can_shrink(true)
            .build();
        root_overlay.set_child(Some(&bg_pic));
    }

    if is_primary {
        // Viñeta inferior sutil estilo Sequoia: solo aparece al despertar el login para dar contraste
        let bottom_scrim = DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .can_target(false)
            .opacity(if state.revealed.get() { 1.0 } else { 0.0 })
            .build();
        bottom_scrim.set_draw_func(|_, cr, w, h| {
            let w = w as f64;
            let h = h as f64;
            let grad = gtk4::cairo::LinearGradient::new(0.0, h * 0.55, 0.0, h);
            grad.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
            grad.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.34);
            let _ = cr.set_source(&grad);
            let _ = cr.rectangle(0.0, h * 0.55, w, h * 0.45);
            let _ = cr.fill();
        });
        root_overlay.add_overlay(&bottom_scrim);

        let es = is_spanish_locale();
        let lock_cfg = LockScreenConfig::load();
        let root_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .valign(Align::Fill)
            .halign(Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["lock-primary-layout"])
            .build();

        // 1. Parte superior: Fecha y Reloj grande (visible desde el inicio)
        let top_clock_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .halign(Align::Center)
            .valign(Align::Start)
            .vexpand(true)
            .spacing(2)
            .css_classes(["lock-top-clock"])
            .build();

        let date_label = Label::builder().css_classes(["lock-date"]).build();
        let clock_label = Label::builder().css_classes(["giant-clock"]).build();

        update_lock_time(&clock_label, &date_label);
        fn schedule_lock_clock_tick(clock: Label, date: Label) {
            let delay_secs = glib::DateTime::now_local()
                .map(|now| (60 - now.second().clamp(0, 59)) as u64)
                .unwrap_or(60)
                .max(1);
            glib::timeout_add_local_once(Duration::from_secs(delay_secs), move || {
                if clock.root().is_none() {
                    return;
                }
                update_lock_time(&clock, &date);
                schedule_lock_clock_tick(clock, date);
            });
        }
        schedule_lock_clock_tick(clock_label.clone(), date_label.clone());

        top_clock_box.append(&date_label);
        top_clock_box.append(&clock_label);
        root_box.append(&top_clock_box);

        // 2. Bloque inferior de login: oculto al inicio; aparece animado al presionar la pantalla
        let bottom_login_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .halign(Align::Center)
            .valign(Align::End)
            .spacing(6)
            .visible(state.revealed.get())
            .opacity(if state.revealed.get() { 1.0 } else { 0.0 })
            .css_classes(["lock-bottom-login"])
            .build();

        let user_name = auth::current_username();
        let avatar_progress = Rc::new(Cell::new(if state.revealed.get() { 1.0 } else { 0.0 }));
        let avatar_widget = create_animated_avatar_widget(&user_name, &avatar_progress);
        if state.revealed.get() {
            let end_sz = AVATAR_END_SIZE.round() as i32;
            avatar_widget.set_size_request(end_sz, end_sz);
        }
        bottom_login_box.append(&avatar_widget);

        let user_label = Label::builder()
            .label(&user_name)
            .css_classes(["username-label"])
            .build();
        bottom_login_box.append(&user_label);

        // 3. Input + Botón squircle (<= 37px) con blur detrás del fondo
        let input_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .halign(Align::Center)
            .valign(Align::Center)
            .spacing(8)
            .build();

        let is_error = Rc::new(Cell::new(false));
        let password_entry = PasswordEntry::builder()
            .placeholder_text(if es {
                "Introduce la contraseña"
            } else {
                "Enter password"
            })
            .css_classes(["lock-password-entry"])
            .show_peek_icon(true)
            .valign(Align::Center)
            .vexpand(false)
            .build();

        let unlock_spinner = Spinner::builder()
            .spinning(false)
            .width_request(15)
            .height_request(15)
            .build();
        let unlock_arrow = Image::from_icon_name("go-next-symbolic");
        let unlock_btn = Button::builder()
            .child(&unlock_arrow)
            .css_classes(["unlock-btn"])
            .valign(Align::Center)
            .vexpand(false)
            .tooltip_text(if es { "Desbloquear" } else { "Unlock" })
            .build();

        let entry_shell =
            create_squircle_entry_shell(&password_entry, &is_error, &root_overlay, assets.clone());
        let btn_shell = create_squircle_button_shell(&unlock_btn, &root_overlay, assets);
        input_row.append(&entry_shell);
        input_row.append(&btn_shell);
        bottom_login_box.append(&input_row);

        let error_label = Label::builder()
            .css_classes(["lock-error-label"])
            .halign(Align::Center)
            .visible(false)
            .build();
        bottom_login_box.append(&error_label);

        let hint_label = Label::builder()
            .css_classes(["lock-hint-label"])
            .halign(Align::Center)
            .visible(false)
            .build();
        bottom_login_box.append(&hint_label);

        let intrusion_label = Label::builder()
            .css_classes(["lock-intrusion-warning"])
            .halign(Align::Center)
            .visible(false)
            .build();
        bottom_login_box.append(&intrusion_label);
        root_box.append(&bottom_login_box);

        // 2b. Mensaje personalizado en pantalla bloqueada (si está habilitado en Ajustes)
        if let Some(custom_msg) = lock_cfg.effective_lock_message() {
            let msg_lbl = Label::builder()
                .label(custom_msg)
                .css_classes(["lock-custom-message"])
                .halign(Align::Center)
                .justify(gtk4::Justification::Center)
                .wrap(true)
                .max_width_chars(68)
                .build();
            root_box.append(&msg_lbl);
        }

        // 2c. Botones Reposo, Reiniciar y Apagar (controlados por show_power_buttons)
        if lock_cfg.show_power_buttons {
            let power_bar = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .halign(Align::Center)
                .valign(Align::End)
                .spacing(28)
                .css_classes(["lock-power-bar"])
                .build();

            let make_power_btn = |icon: &str, text: &str, action: &'static str| -> GtkBox {
                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .halign(Align::Center)
                    .spacing(6)
                    .build();
                let btn = Button::builder()
                    .icon_name(icon)
                    .css_classes(["lock-power-circle-btn"])
                    .halign(Align::Center)
                    .valign(Align::Center)
                    .tooltip_text(text)
                    .build();
                btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("systemctl").arg(action).spawn();
                });
                let lbl = Label::builder()
                    .label(text)
                    .css_classes(["lock-power-label"])
                    .halign(Align::Center)
                    .build();
                col.append(&btn);
                col.append(&lbl);
                col
            };

            power_bar.append(&make_power_btn(
                "weather-clear-night-symbolic",
                if es { "Reposo" } else { "Sleep" },
                "suspend",
            ));
            power_bar.append(&make_power_btn(
                "system-reboot-symbolic",
                if es { "Reiniciar" } else { "Restart" },
                "reboot",
            ));
            power_bar.append(&make_power_btn(
                "system-shutdown-symbolic",
                if es { "Apagar" } else { "Shut Down" },
                "poweroff",
            ));
            root_box.append(&power_bar);
        }

        // Animación estilo Sequoia al despertar: viñeta sutil + resorte orgánico en login y avatar
        let anim_gen = Rc::new(Cell::new(0u64));
        let reveal_login: Rc<dyn Fn()> = {
            let state_rev = state.clone();
            let box_rev = bottom_login_box.clone();
            let scrim_rev = bottom_scrim.clone();
            let avatar_rev = avatar_widget.clone();
            let avatar_prog = avatar_progress.clone();
            let pwd_rev = password_entry.clone();
            let entry_bg = entry_shell.child();
            let btn_bg = btn_shell.child();
            let gen_slot = anim_gen.clone();
            Rc::new(move || {
                if state_rev.revealed.get() {
                    pwd_rev.grab_focus();
                    return;
                }
                state_rev.revealed.set(true);
                box_rev.set_visible(true);
                box_rev.set_opacity(0.0);
                box_rev.set_margin_bottom(0);
                scrim_rev.set_opacity(0.0);
                avatar_prog.set(0.0);
                let start_sz = AVATAR_START_SIZE.round() as i32;
                avatar_rev.set_size_request(start_sz, start_sz);

                let gen = gen_slot.get().wrapping_add(1);
                gen_slot.set(gen);
                let start_time: Cell<Option<i64>> = Cell::new(None);
                let gen_tick = gen_slot.clone();
                let scrim_tick = scrim_rev.clone();
                let avatar_tick = avatar_rev.clone();
                let prog_tick = avatar_prog.clone();
                let pwd_tick = pwd_rev.clone();
                let entry_bg_tick = entry_bg.clone();
                let btn_bg_tick = btn_bg.clone();

                box_rev.add_tick_callback(move |b, clock| {
                    const DURATION_US: f64 = 380_000.0;
                    if gen_tick.get() != gen {
                        return ControlFlow::Break;
                    }
                    let now = clock.frame_time();
                    let t0 = match start_time.get() {
                        Some(v) => v,
                        None => {
                            start_time.set(Some(now));
                            now
                        }
                    };
                    let t = ((now - t0) as f64 / DURATION_US).clamp(0.0, 1.0);
                    let spring = sequoia_spring(t);
                    let fade = (1.0 - (1.0 - (t * 1.4).min(1.0)).powi(3)).clamp(0.0, 1.0);

                    b.set_opacity(fade);
                    scrim_tick.set_opacity(fade);
                    let lift = (spring * 14.0).round().max(0.0) as i32;
                    b.set_margin_bottom(lift);

                    // El avatar crece con resorte Sequoia de 38px a 58px
                    let cur_sz = (AVATAR_START_SIZE
                        + (AVATAR_END_SIZE - AVATAR_START_SIZE) * spring)
                        .round()
                        .max(AVATAR_START_SIZE) as i32;
                    avatar_tick.set_size_request(cur_sz, cur_sz);
                    prog_tick.set(spring);
                    avatar_tick.queue_draw();

                    if let Some(ref w) = entry_bg_tick {
                        w.queue_draw();
                    }
                    if let Some(ref w) = btn_bg_tick {
                        w.queue_draw();
                    }

                    if t >= 1.0 {
                        b.set_opacity(1.0);
                        scrim_tick.set_opacity(1.0);
                        b.set_margin_bottom(14);
                        let end_sz = AVATAR_END_SIZE.round() as i32;
                        avatar_tick.set_size_request(end_sz, end_sz);
                        pwd_tick.grab_focus();
                        ControlFlow::Break
                    } else {
                        ControlFlow::Continue
                    }
                });
            })
        };
        *state.reveal_login_cb.borrow_mut() = Some(reveal_login.clone());

        // Presionar en cualquier parte de la pantalla o cualquier tecla despierta el login
        let screen_click = GestureClick::new();
        screen_click.set_button(0);
        let rev_click = reveal_login.clone();
        let state_click = state.clone();
        screen_click.connect_pressed(move |_, _, _, _| {
            if !state_click.revealed.get() {
                rev_click();
            }
        });
        window.add_controller(screen_click);

        let key_ctrl = EventControllerKey::new();
        let rev_key = reveal_login;
        let state_key = state.clone();
        let pwd_key = password_entry.clone();
        let box_key = bottom_login_box.clone();
        let scrim_key = bottom_scrim;
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if !state_key.revealed.get() {
                rev_key();
                return glib::Propagation::Proceed;
            }
            if key == gtk4::gdk::Key::Escape
                && pwd_key.text().is_empty()
                && !state_key.verifying.get()
            {
                state_key.revealed.set(false);
                box_key.set_visible(false);
                box_key.set_opacity(0.0);
                scrim_key.set_opacity(0.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_ctrl);

        // Limpiar el borde de error cuando el usuario vuelve a escribir
        let err_clear = is_error.clone();
        let lbl_clear = error_label.clone();
        let shell_clear = entry_shell.clone();
        password_entry.connect_changed(move |_| {
            if err_clear.get() {
                err_clear.set(false);
                lbl_clear.set_visible(false);
                if let Some(bg) = shell_clear.child() {
                    bg.queue_draw();
                }
            }
        });

        let app_unlock = app.clone();
        let state_unlock = state.clone();
        let pwd_clone = password_entry.clone();
        let btn_clone = unlock_btn.clone();
        let arrow_clone = unlock_arrow;
        let spin_clone = unlock_spinner;
        let err_lbl_clone = error_label;
        let hint_lbl_clone = hint_label;
        let configured_hint: Option<String> =
            lock_cfg.effective_password_hint().map(str::to_string);
        let intr_lbl_clone = intrusion_label;
        let err_flag_clone = is_error;
        let entry_shell_clone = entry_shell;
        let input_row_clone = input_row;
        let ui_scale_box = scale_fade::ScaleFadeBox::new();
        ui_scale_box.set_child(&root_box);
        let auth_user = user_name;
        let win_attempt = window.clone();
        let ui_attempt = ui_scale_box.clone();

        let update_intrusion_banner: Rc<dyn Fn()> = {
            let st = state_unlock.clone();
            let lbl = intr_lbl_clone.clone();
            Rc::new(move || {
                let intr = st.intrusion.borrow();
                if intr.failed_attempts == 0 {
                    lbl.set_visible(false);
                    return;
                }
                let text = match (es, intr.last_failed_local.as_deref()) {
                    (true, Some(ts)) => format!(
                        "⚠ {} intento(s) fallido(s) (último: {})",
                        intr.failed_attempts, ts
                    ),
                    (true, None) => {
                        format!("⚠ {} intento(s) fallido(s)", intr.failed_attempts)
                    }
                    (false, Some(ts)) => format!(
                        "⚠ {} failed attempt(s) (last: {})",
                        intr.failed_attempts, ts
                    ),
                    (false, None) => {
                        format!("⚠ {} failed attempt(s)", intr.failed_attempts)
                    }
                };
                lbl.set_text(&text);
                lbl.set_visible(true);
            })
        };

        let start_cooldown_countdown: Rc<dyn Fn()> = {
            let st = state_unlock.clone();
            let pwd = pwd_clone.clone();
            let btn = btn_clone.clone();
            let err_lbl = err_lbl_clone.clone();
            let err_flag = err_flag_clone.clone();
            let shell = entry_shell_clone.clone();
            Rc::new(move || {
                let rem = intrusion::remaining_cooldown_secs(
                    &st.intrusion.borrow(),
                    intrusion::now_unix_secs(),
                );
                if rem == 0 {
                    return;
                }
                pwd.set_sensitive(false);
                btn.set_sensitive(false);
                err_flag.set(true);
                err_lbl.set_text(&if es {
                    format!("Demasiados intentos fallidos. Espera {rem} s")
                } else {
                    format!("Too many failed attempts. Wait {rem} s")
                });
                err_lbl.set_visible(true);
                if let Some(bg) = shell.child() {
                    bg.queue_draw();
                }

                if st.cooldown_timer_active.replace(true) {
                    return;
                }
                let st_t = st.clone();
                let pwd_t = pwd.clone();
                let btn_t = btn.clone();
                let err_lbl_t = err_lbl.clone();
                let err_flag_t = err_flag.clone();
                let shell_t = shell.clone();
                glib::timeout_add_local(Duration::from_secs(1), move || {
                    if st_t.unlocked.get() || pwd_t.root().is_none() {
                        st_t.cooldown_timer_active.set(false);
                        return ControlFlow::Break;
                    }
                    let rem = intrusion::remaining_cooldown_secs(
                        &st_t.intrusion.borrow(),
                        intrusion::now_unix_secs(),
                    );
                    if rem == 0 {
                        st_t.cooldown_timer_active.set(false);
                        err_flag_t.set(false);
                        err_lbl_t.set_visible(false);
                        if !st_t.verifying.get() {
                            pwd_t.set_sensitive(true);
                            btn_t.set_sensitive(true);
                            pwd_t.grab_focus();
                        }
                        if let Some(bg) = shell_t.child() {
                            bg.queue_draw();
                        }
                        return ControlFlow::Break;
                    }
                    err_lbl_t.set_text(&if es {
                        format!("Demasiados intentos fallidos. Espera {rem} s")
                    } else {
                        format!("Too many failed attempts. Wait {rem} s")
                    });
                    ControlFlow::Continue
                });
            })
        };

        update_intrusion_banner();
        start_cooldown_countdown();

        let attempt_unlock = Rc::new(move || {
            if state_unlock.verifying.get() || state_unlock.unlocked.get() {
                return;
            }
            let rem = intrusion::remaining_cooldown_secs(
                &state_unlock.intrusion.borrow(),
                intrusion::now_unix_secs(),
            );
            if rem > 0 {
                start_cooldown_countdown();
                trigger_shake(&input_row_clone);
                return;
            }
            let password = pwd_clone.text().to_string();
            if password.is_empty() {
                err_flag_clone.set(true);
                err_lbl_clone.set_text(if es {
                    "Introduce tu contraseña"
                } else {
                    "Enter your password"
                });
                err_lbl_clone.set_visible(true);
                if let Some(bg) = entry_shell_clone.child() {
                    bg.queue_draw();
                }
                trigger_shake(&input_row_clone);
                pwd_clone.grab_focus();
                return;
            }

            state_unlock.verifying.set(true);
            pwd_clone.set_sensitive(false);
            btn_clone.set_sensitive(false);
            spin_clone.set_spinning(true);
            btn_clone.set_child(Some(&spin_clone));
            err_lbl_clone.set_visible(false);
            err_flag_clone.set(false);
            if let Some(bg) = entry_shell_clone.child() {
                bg.queue_draw();
            }

            let user = auth_user.clone();
            let app_res = app_unlock.clone();
            let state_res = state_unlock.clone();
            let win_res = win_attempt.clone();
            let ui_res = ui_attempt.clone();
            let pwd_res = pwd_clone.clone();
            let btn_res = btn_clone.clone();
            let arrow_res = arrow_clone.clone();
            let spin_res = spin_clone.clone();
            let err_lbl_res = err_lbl_clone.clone();
            let hint_lbl_res = hint_lbl_clone.clone();
            let hint_text_res = configured_hint.clone();
            let err_flag_res = err_flag_clone.clone();
            let shell_res = entry_shell_clone.clone();
            let row_res = input_row_clone.clone();
            let banner_res = update_intrusion_banner.clone();
            let cooldown_res = start_cooldown_countdown.clone();

            glib::MainContext::default().spawn_local(async move {
                let auth_result = gio::spawn_blocking(move || {
                    auth::verify_user_password(&user, &password)
                })
                .await;

                state_res.verifying.set(false);
                spin_res.set_spinning(false);
                btn_res.set_child(Some(&arrow_res));

                match auth_result {
                    Ok(Ok(true)) => {
                        info!("Desbloqueo autenticado mediante PAM; animando salida del lockscreen...");
                        let prev = intrusion::clear_state_at(&intrusion::intrusion_state_path());
                        *state_res.intrusion.borrow_mut() = intrusion::IntrusionState::default();
                        if prev.failed_attempts > 0 {
                            let last_ts = prev.last_failed_local.clone();
                            let count = prev.failed_attempts;
                            gio::spawn_blocking(move || {
                                intrusion::dispatch_intrusion_alert(
                                    count,
                                    last_ts.as_deref(),
                                    es,
                                );
                            });
                        }
                        animate_unlock_and_quit(&app_res, &state_res, &win_res, &ui_res);
                    }
                    Ok(Ok(false)) => {
                        let local_ts = glib::DateTime::now_local()
                            .ok()
                            .and_then(|dt| dt.format("%H:%M:%S").ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let wait = {
                            let mut intr = state_res.intrusion.borrow_mut();
                            intrusion::record_failed_attempt_at(
                                &intrusion::intrusion_state_path(),
                                &mut intr,
                                intrusion::now_unix_secs(),
                                &local_ts,
                            )
                        };
                        banner_res();
                        err_flag_res.set(true);
                        pwd_res.set_text("");
                        if let Some(ref hint) = hint_text_res {
                            hint_lbl_res.set_text(&if es {
                                format!("Pista: {hint}")
                            } else {
                                format!("Hint: {hint}")
                            });
                            hint_lbl_res.set_visible(true);
                        }
                        if wait > 0 {
                            cooldown_res();
                        } else {
                            err_lbl_res.set_text(if es {
                                "Contraseña incorrecta"
                            } else {
                                "Incorrect password"
                            });
                            err_lbl_res.set_visible(true);
                            pwd_res.set_sensitive(true);
                            btn_res.set_sensitive(true);
                            if let Some(bg) = shell_res.child() {
                                bg.queue_draw();
                            }
                            pwd_res.grab_focus();
                        }
                        trigger_shake(&row_res);
                    }
                    Ok(Err(err)) => {
                        error!("Error interno de PAM al verificar contraseña: {}", err);
                        err_flag_res.set(true);
                        err_lbl_res.set_text(if es {
                            "Error de autenticación PAM"
                        } else {
                            "PAM authentication error"
                        });
                        err_lbl_res.set_visible(true);
                        pwd_res.set_text("");
                        pwd_res.set_sensitive(true);
                        btn_res.set_sensitive(true);
                        if let Some(bg) = shell_res.child() {
                            bg.queue_draw();
                        }
                        trigger_shake(&row_res);
                        pwd_res.grab_focus();
                    }
                    Err(join_err) => {
                        error!("Fallo en tarea de autenticación PAM: {:?}", join_err);
                        pwd_res.set_sensitive(true);
                        btn_res.set_sensitive(true);
                        pwd_res.grab_focus();
                    }
                }
            });
        });

        let u_btn = attempt_unlock.clone();
        unlock_btn.connect_clicked(move |_| {
            u_btn();
        });

        let u_entry = attempt_unlock;
        password_entry.connect_activate(move |_| {
            u_entry();
        });

        root_overlay.add_overlay(&ui_scale_box);
        *state.primary_entry.borrow_mut() = Some(password_entry);
        window.set_child(Some(&root_overlay));
        window.present();
        animate_lock_enter(&window, &root_overlay, Some(&ui_scale_box));
        state.backdrop_overlays.borrow_mut().push(root_overlay);
    } else {
        // Monitores secundarios: solo wallpaper activo; al tocar cualquier parte despierta el login en el principal
        let sec_click = GestureClick::new();
        sec_click.set_button(0);
        let state_sec = state.clone();
        sec_click.connect_pressed(move |_, _, _, _| {
            if let Some(cb) = state_sec.reveal_login_cb.borrow().as_ref() {
                cb();
            }
        });
        window.add_controller(sec_click);
        window.set_child(Some(&root_overlay));
        window.present();
        animate_lock_enter(&window, &root_overlay, None);
        state.backdrop_overlays.borrow_mut().push(root_overlay);
    }

    state.windows.borrow_mut().push(window);
}

fn update_lock_time(clock: &Label, date: &Label) {
    if let Ok(now) = glib::DateTime::now_local() {
        if let Ok(c) = now.format("%H:%M") {
            clock.set_text(&c);
        }
        if let Ok(d) = now.format("%A, %-d de %B") {
            date.set_text(&d);
        }
    }
}

#[cfg(test)]
mod lockscreen_tests {
    use super::{
        AVATAR_END_SIZE, AVATAR_START_SIZE, CONTROL_HEIGHT, LOCK_CSS, MAX_CONTROL_HEIGHT,
        SQUIRCLE_EXPONENT,
    };

    #[test]
    fn sequoia_spring_has_gentle_overshoot_and_settles_at_one() {
        assert_eq!(super::sequoia_spring(0.0), 0.0);
        assert_eq!(super::sequoia_spring(1.0), 1.0);
        let peak = (1..100)
            .map(|i| super::sequoia_spring(i as f64 / 100.0))
            .fold(0.0_f64, f64::max);
        assert!(peak > 1.0 && peak < 1.15);
    }

    #[test]
    fn input_and_button_height_do_not_exceed_37px() {
        assert!(CONTROL_HEIGHT <= MAX_CONTROL_HEIGHT);
        assert_eq!(MAX_CONTROL_HEIGHT, 37);
        assert!(AVATAR_END_SIZE > AVATAR_START_SIZE);
        assert!((SQUIRCLE_EXPONENT - 4.2).abs() < 1e-6);
        assert!(LOCK_CSS.contains("max-height: 37px;"));
    }
}
