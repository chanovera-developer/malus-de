use gtk4::gdk::Key;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, CssProvider, DrawingArea,
    EventControllerKey, Expander, FileDialog, Label, Orientation, PolicyType, ScrolledWindow,
    TextView, Window, WrapMode,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

thread_local! {
    static CONFIRMATION_CSS_LOADED: Cell<bool> = const { Cell::new(false) };
    static DIALOG_GLASS_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static ACTIVE_POWER_DIALOG: RefCell<Option<glib::WeakRef<ApplicationWindow>>> =
        const { RefCell::new(None) };
}

const CONFIRMATION_CSS: &str = include_str!("styles/confirmation.css");
pub const POWER_DIALOG_COUNTDOWN_SECS: u32 = 60;
const POWER_CARD_BUTTON_INSET_PX: f64 = 20.0;
const POWER_BUTTON_HEIGHT_PX: i32 = 28;
const SQUIRCLE_EXPONENT: f64 = 4.2;

pub fn concentric_power_button_radius(
    window_corner_radius: f64,
    inset: f64,
    button_height: f64,
) -> f64 {
    let max_r = (button_height * 0.5).max(1.0);
    let raw = (window_corner_radius - inset).max(6.0);
    raw.min(max_r)
}

fn squircle_path(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let radius = radius.min(width * 0.5).min(height * 0.5).max(1.0);
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;
    const SEGMENTS: usize = 20;

    cr.move_to(x + radius, y);
    cr.line_to(x + width - radius, y);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        cr.line_to(
            x + width - radius + radius * angle.sin().powf(power),
            y + radius - radius * angle.cos().powf(power),
        );
    }
    cr.line_to(x + width, y + height - radius);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        cr.line_to(
            x + width - radius + radius * angle.cos().powf(power),
            y + height - radius + radius * angle.sin().powf(power),
        );
    }
    cr.line_to(x + radius, y + height);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        cr.line_to(
            x + radius - radius * angle.sin().powf(power),
            y + height - radius + radius * angle.cos().powf(power),
        );
    }
    cr.line_to(x, y + radius);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        cr.line_to(
            x + radius - radius * angle.cos().powf(power),
            y + radius - radius * angle.sin().powf(power),
        );
    }
    cr.close_path();
}

fn create_power_squircle_button(
    label_text: &str,
    is_primary: bool,
    dark: bool,
    window_corner_radius: f64,
) -> Button {
    let btn = Button::builder()
        .has_frame(false)
        .height_request(POWER_BUTTON_HEIGHT_PX)
        .css_classes([if is_primary {
            "malus-power-primary"
        } else {
            "malus-power-cancel"
        }])
        .build();

    let label = Label::builder()
        .label(label_text)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(14)
        .margin_end(14)
        .css_classes([
            "malus-power-btn-label",
            if is_primary {
                "malus-power-btn-label-primary"
            } else {
                "malus-power-btn-label-cancel"
            },
        ])
        .build();

    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let Some(b) = btn_weak.upgrade() else {
            return;
        };
        let flags = b.state_flags();
        let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
        let active = flags.contains(gtk4::StateFlags::ACTIVE);
        let focused = flags.contains(gtk4::StateFlags::FOCUS_VISIBLE);

        let r = concentric_power_button_radius(
            window_corner_radius,
            POWER_CARD_BUTTON_INSET_PX,
            height,
        );

        let _ = cr.save();
        squircle_path(cr, 0.0, 0.0, width, height, r);
        if is_primary {
            if active {
                cr.set_source_rgba(0.0, 0.400, 0.839, 1.0);
            } else if hovered {
                cr.set_source_rgba(0.086, 0.525, 1.0, 1.0);
            } else {
                cr.set_source_rgba(0.0, 0.478, 1.0, 1.0);
            }
        } else if dark {
            if active {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
            } else if hovered {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.19);
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.13);
            }
        } else if active {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.19);
        } else if hovered {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.09);
        }
        let _ = cr.fill();

        // Fino bisel interior concéntrico para consistencia con Liquid Glass
        squircle_path(cr, 0.5, 0.5, width - 1.0, height - 1.0, (r - 0.5).max(1.0));
        cr.set_line_width(1.0);
        if focused {
            cr.set_source_rgba(0.0, 0.478, 1.0, 0.72);
        } else if is_primary {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
        } else if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.24);
        }
        let _ = cr.stroke();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = gtk4::Overlay::builder()
        .hexpand(true)
        .height_request(POWER_BUTTON_HEIGHT_PX)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&label);
    overlay.set_measure_overlay(&label, true);
    btn.set_child(Some(&overlay));
    btn
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerDialogKind {
    Shutdown,
    Logout,
    Restart,
}

fn is_spanish_locale() -> bool {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es")
}

fn reopen_windows_pref_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".config/malus-de/session-reopen.conf"))
}

pub fn load_reopen_windows_pref() -> bool {
    let Some(path) = reopen_windows_pref_path() else {
        return true;
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return true;
    };
    for line in content.lines() {
        if let Some(val) = line.trim().strip_prefix("reopen_windows=") {
            return val.trim().eq_ignore_ascii_case("true") || val.trim() == "1";
        }
    }
    true
}

pub fn save_reopen_windows_pref(reopen: bool) {
    let Some(path) = reopen_windows_pref_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("reopen_windows={}\n", reopen));
}

pub fn power_dialog_copy(
    kind: PowerDialogKind,
    remaining_secs: u32,
    spanish: bool,
) -> (&'static str, String, &'static str, &'static str, &'static str) {
    if spanish {
        let secs_word = if remaining_secs == 1 {
            "segundo"
        } else {
            "segundos"
        };
        match kind {
            PowerDialogKind::Shutdown => (
                "¿Quieres apagar la computadora?",
                format!(
                    "Si no haces nada, la computadora se apagará automáticamente dentro de {remaining_secs} {secs_word}."
                ),
                "Volver a abrir ventanas al iniciar la sesión",
                "Cancelar",
                "Apagar",
            ),
            PowerDialogKind::Logout => (
                "¿Quieres salir de todas las aplicaciones y cerrar la sesión?",
                format!(
                    "Si no haces nada, la sesión se cerrará automáticamente dentro de {remaining_secs} {secs_word}."
                ),
                "Volver a abrir ventanas al iniciar la sesión",
                "Cancelar",
                "Cerrar sesión",
            ),
            PowerDialogKind::Restart => (
                "¿Quieres reiniciar la computadora?",
                format!(
                    "Si no haces nada, la computadora se reiniciará automáticamente dentro de {remaining_secs} {secs_word}."
                ),
                "Volver a abrir ventanas al iniciar la sesión",
                "Cancelar",
                "Reiniciar",
            ),
        }
    } else {
        let secs_word = if remaining_secs == 1 {
            "second"
        } else {
            "seconds"
        };
        match kind {
            PowerDialogKind::Shutdown => (
                "Are you sure you want to shut down your computer now?",
                format!(
                    "If you do nothing, the computer will shut down automatically in {remaining_secs} {secs_word}."
                ),
                "Reopen windows when logging back in",
                "Cancel",
                "Shut Down",
            ),
            PowerDialogKind::Logout => (
                "Are you sure you want to quit all applications and log out now?",
                format!(
                    "If you do nothing, you will be logged out automatically in {remaining_secs} {secs_word}."
                ),
                "Reopen windows when logging back in",
                "Cancel",
                "Log Out",
            ),
            PowerDialogKind::Restart => (
                "Are you sure you want to restart your computer now?",
                format!(
                    "If you do nothing, the computer will restart automatically in {remaining_secs} {secs_word}."
                ),
                "Reopen windows when logging back in",
                "Cancel",
                "Restart",
            ),
        }
    }
}

fn draw_power_dialog_icon(
    cr: &gtk4::cairo::Context,
    width: f64,
    height: f64,
    kind: PowerDialogKind,
    dark: bool,
) {
    let size = width.min(height);
    if size <= 4.0 {
        return;
    }
    let cx = width * 0.5;
    let cy = height * 0.5;
    let r = size * 0.46875;

    let _ = cr.save();
    // Círculo gris neutro estilo macOS
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    if dark {
        cr.set_source_rgba(0.580, 0.592, 0.616, 0.88);
    } else {
        cr.set_source_rgba(0.553, 0.561, 0.580, 0.92);
    }
    let _ = cr.fill();

    cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
    let stroke = (size * 0.038).max(1.8);

    match kind {
        PowerDialogKind::Shutdown => {
            cr.set_line_width(stroke);
            cr.set_line_cap(gtk4::cairo::LineCap::Round);
            let arc_cy = cy + size * 0.022;
            let arc_r = r * 0.485;
            cr.new_sub_path();
            cr.arc(
                cx,
                arc_cy,
                arc_r,
                (-50.0_f64).to_radians(),
                (230.0_f64).to_radians(),
            );
            let _ = cr.stroke();

            cr.move_to(cx, cy - r * 0.55);
            cr.line_to(cx, cy + r * 0.01);
            let _ = cr.stroke();
        }
        PowerDialogKind::Logout => {
            // Silueta blanca de usuario recortada con margen circular gris interior
            let inner_clip_r = r * 0.845;
            cr.arc(cx, cy, inner_clip_r, 0.0, std::f64::consts::TAU);
            cr.clip();

            // Cabeza
            cr.arc(cx, cy - r * 0.25, r * 0.34, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();

            // Hombros / torso
            let _ = cr.save();
            cr.translate(cx, cy + r * 0.82);
            cr.scale(1.0, 0.80);
            cr.arc(0.0, 0.0, r * 0.74, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            let _ = cr.restore();
        }
        PowerDialogKind::Restart => {
            cr.set_line_width(stroke);
            cr.set_line_cap(gtk4::cairo::LineCap::Round);
            cr.set_line_join(gtk4::cairo::LineJoin::Round);
            // Triángulo equilátero hueco apuntando a la izquierda, centrado ópticamente
            let left_x = cx - r * 0.50;
            let right_x = cx + r * 0.28;
            let half_h = r * 0.45;
            cr.move_to(left_x, cy);
            cr.line_to(right_x, cy - half_h);
            cr.line_to(right_x, cy + half_h);
            cr.close_path();
            let _ = cr.stroke();
        }
    }
    let _ = cr.restore();
}

fn refresh_dialog_liquid_glass_opacity() {
    let dark = malus_de_common::config::is_dark_mode_active();
    libadwaita::StyleManager::default().set_color_scheme(if dark {
        libadwaita::ColorScheme::ForceDark
    } else {
        libadwaita::ColorScheme::ForceLight
    });
    let opacity = malus_de_common::config::load_liquid_glass_opacity();
    let scale = malus_de_common::config::liquid_glass_scale_for(opacity);
    let card_a = malus_de_common::config::scaled_liquid_glass_alpha(0.82, scale, 0.16, 0.96);
    let power_bg = if dark {
        let a = malus_de_common::config::scaled_liquid_glass_alpha(0.72, scale, 0.26, 0.94);
        format!(
            "background-color: rgba(38, 40, 44, {a:.3}); \
             background-image: linear-gradient(180deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0.02));"
        )
    } else {
        let a = malus_de_common::config::scaled_liquid_glass_alpha(0.68, scale, 0.22, 0.94);
        format!(
            "background-color: rgba(246, 246, 248, {a:.3}); \
             background-image: linear-gradient(180deg, rgba(255, 255, 255, 0.26), rgba(255, 255, 255, 0.06));"
        )
    };
    let css = format!(
        ".malus-confirmation-card {{ background-color: alpha(@window_bg_color, {card_a:.3}); }}\n\
         .malus-power-card {{ {power_bg} }}"
    );
    DIALOG_GLASS_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn ensure_dialog_css_loaded() {
    let _ = libadwaita::init();
    CONFIRMATION_CSS_LOADED.with(|loaded| {
        if loaded.get() {
            refresh_dialog_liquid_glass_opacity();
            return;
        }
        if let Some(display) = gtk4::gdk::Display::default() {
            let provider = CssProvider::new();
            provider.load_from_string(CONFIRMATION_CSS);
            let glass_provider = CssProvider::new();
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk4::style_context_add_provider_for_display(
                &display,
                &glass_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
            DIALOG_GLASS_PROVIDER.with(|cell| {
                *cell.borrow_mut() = Some(glass_provider);
            });
            loaded.set(true);
            refresh_dialog_liquid_glass_opacity();
        }
    });
}

pub fn show_confirmation(
    app: &Application,
    parent: Option<&Window>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    on_confirm: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    ensure_dialog_css_loaded();

    let dialog = ApplicationWindow::builder()
        .application(app)
        .title("Confirmación de acción del sistema")
        .css_classes(["malus-confirmation-window"])
        .modal(true)
        .decorated(false)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .css_classes(["malus-confirmation-card"])
        .build();
    let heading = Label::builder()
        .label(heading)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-heading"])
        .build();
    let body = Label::builder()
        .label(body)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-body"])
        .build();
    card.append(&heading);
    card.append(&body);

    let buttons = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk4::Align::End)
        .build();
    let cancel_button = Button::with_label("Cancelar");
    cancel_button.add_css_class("malus-confirmation-cancel");
    let confirm_button = Button::with_label(confirm_label);
    confirm_button.add_css_class("destructive-action");
    buttons.append(&cancel_button);
    buttons.append(&confirm_button);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let on_cancel = Rc::new(RefCell::new(Some(on_cancel)));
    dialog.connect_close_request({
        let on_cancel = on_cancel.clone();
        move |_| {
            if let Some(callback) = on_cancel.borrow_mut().take() {
                callback();
            }
            glib::Propagation::Proceed
        }
    });

    let dialog_on_cancel = dialog.clone();
    cancel_button.connect_clicked(move |_| dialog_on_cancel.close());

    let dialog_on_confirm = dialog.clone();
    let on_cancel_confirm = on_cancel;
    let on_confirm = RefCell::new(Some(on_confirm));
    confirm_button.connect_clicked(move |_| {
        on_cancel_confirm.borrow_mut().take();
        if let Some(callback) = on_confirm.borrow_mut().take() {
            callback();
        }
        dialog_on_confirm.close();
    });

    let key_controller = EventControllerKey::new();
    let dialog_on_escape = dialog.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == Key::Escape {
            dialog_on_escape.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    dialog.add_controller(key_controller);
    dialog.present();
    cancel_button.grab_focus();
}

/// Presenta el cuadro de diálogo de sistema estilo macOS para Apagar, Reiniciar o Cerrar sesión,
/// con icono circular vectorial, cuenta regresiva de 60 segundos, casilla para reabrir ventanas
/// y botones redondeados en píldora.
pub fn show_power_dialog(
    app: &Application,
    parent: Option<&Window>,
    kind: PowerDialogKind,
    on_confirm: impl FnOnce(bool) + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    ensure_dialog_css_loaded();

    // Reemplazar cualquier diálogo de energía/sesión previo abierto para evitar cuentas duplicadas.
    ACTIVE_POWER_DIALOG.with(|slot| {
        if let Some(existing) = slot.borrow_mut().take().and_then(|w| w.upgrade()) {
            existing.close();
        }
    });

    let spanish = is_spanish_locale();
    let dark = malus_de_common::config::is_dark_mode_active();
    let (heading_text, initial_body, reopen_text, cancel_text, confirm_text) =
        power_dialog_copy(kind, POWER_DIALOG_COUNTDOWN_SECS, spanish);

    let dialog = ApplicationWindow::builder()
        .application(app)
        .title("Confirmación de acción del sistema")
        .css_classes(["malus-confirmation-window", "malus-power-window"])
        .modal(true)
        .decorated(false)
        .resizable(false)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }

    ACTIVE_POWER_DIALOG.with(|slot| {
        *slot.borrow_mut() = Some(dialog.downgrade());
    });

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["malus-power-card"])
        .build();

    let icon_area = DrawingArea::builder()
        .content_width(64)
        .content_height(64)
        .halign(gtk4::Align::Start)
        .valign(gtk4::Align::Start)
        .css_classes(["malus-power-icon"])
        .build();
    icon_area.set_draw_func(move |_, cr, w, h| {
        draw_power_dialog_icon(cr, w as f64, h as f64, kind, dark);
    });

    let heading = Label::builder()
        .label(heading_text)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(34)
        .css_classes(["malus-power-heading"])
        .build();

    let body = Label::builder()
        .label(&initial_body)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(38)
        .css_classes(["malus-power-body"])
        .build();

    let reopen_check = CheckButton::builder()
        .label(reopen_text)
        .active(load_reopen_windows_pref())
        .halign(gtk4::Align::Start)
        .css_classes(["malus-power-checkbox"])
        .build();
    reopen_check.connect_toggled(|cb| {
        save_reopen_windows_pref(cb.is_active());
    });

    let buttons = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .homogeneous(true)
        .hexpand(true)
        .css_classes(["malus-power-actions"])
        .build();

    let window_corner_radius = malus_de_common::config::DisplaysConfig::load().window_corner_radius;
    let cancel_button =
        create_power_squircle_button(cancel_text, false, dark, window_corner_radius);
    let confirm_button =
        create_power_squircle_button(confirm_text, true, dark, window_corner_radius);

    buttons.append(&cancel_button);
    buttons.append(&confirm_button);

    card.append(&icon_area);
    card.append(&heading);
    card.append(&body);
    card.append(&reopen_check);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let finished = Rc::new(Cell::new(false));
    let on_confirm = Rc::new(RefCell::new(Some(on_confirm)));
    let on_cancel = Rc::new(RefCell::new(Some(on_cancel)));

    let trigger_confirm: Rc<dyn Fn()> = {
        let finished = finished.clone();
        let on_confirm = on_confirm.clone();
        let on_cancel = on_cancel.clone();
        let reopen_check = reopen_check.clone();
        let dialog_weak = dialog.downgrade();
        Rc::new(move || {
            if finished.replace(true) {
                return;
            }
            on_cancel.borrow_mut().take();
            let reopen = reopen_check.is_active();
            save_reopen_windows_pref(reopen);
            if let Some(callback) = on_confirm.borrow_mut().take() {
                callback(reopen);
            }
            if let Some(win) = dialog_weak.upgrade() {
                win.close();
            }
        })
    };

    dialog.connect_close_request({
        let finished = finished.clone();
        let on_confirm = on_confirm.clone();
        let on_cancel = on_cancel.clone();
        move |win| {
            ACTIVE_POWER_DIALOG.with(|slot| {
                let mut guard = slot.borrow_mut();
                if guard
                    .as_ref()
                    .and_then(|w| w.upgrade())
                    .is_some_and(|w| w == *win)
                {
                    *guard = None;
                }
            });
            if !finished.replace(true) {
                on_confirm.borrow_mut().take();
                if let Some(callback) = on_cancel.borrow_mut().take() {
                    callback();
                }
            }
            glib::Propagation::Proceed
        }
    });

    {
        let dialog_on_cancel = dialog.clone();
        cancel_button.connect_clicked(move |_| dialog_on_cancel.close());
    }

    {
        let trigger = trigger_confirm.clone();
        confirm_button.connect_clicked(move |_| trigger());
    }

    let key_controller = EventControllerKey::new();
    {
        let dialog_on_key = dialog.clone();
        let cancel_btn_key = cancel_button.clone();
        let trigger = trigger_confirm.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, _| match keyval {
            Key::Escape => {
                dialog_on_key.close();
                glib::Propagation::Stop
            }
            Key::Return | Key::KP_Enter => {
                if cancel_btn_key.has_focus() {
                    dialog_on_key.close();
                } else {
                    trigger();
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    dialog.add_controller(key_controller);

    // Cuenta regresiva automática de 60 segundos (idéntica a macOS)
    let remaining = Cell::new(POWER_DIALOG_COUNTDOWN_SECS);
    let finished_timer = finished.clone();
    let body_weak = body.downgrade();
    let trigger_timer = trigger_confirm.clone();
    glib::timeout_add_seconds_local(1, move || {
        if finished_timer.get() {
            return glib::ControlFlow::Break;
        }
        let next = remaining.get().saturating_sub(1);
        remaining.set(next);
        if next == 0 {
            trigger_timer();
            return glib::ControlFlow::Break;
        }
        if let Some(lbl) = body_weak.upgrade() {
            let (_, updated_body, _, _, _) = power_dialog_copy(kind, next, spanish);
            lbl.set_label(&updated_body);
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });

    dialog.present();
    reopen_check.grab_focus();
}

/// Presenta un diálogo compacto estilo macOS tras un cierre inesperado, con botón directo
/// para enviar el reporte a `malus-de@chano.dev` y detalles técnicos desplegables opcionales.
pub fn show_crash_report_dialog(
    app: &Application,
    report_path: PathBuf,
    report_text: String,
    _extra_archived_count: usize,
) {
    ensure_dialog_css_loaded();

    let component =
        extract_report_field(&report_text, "Componente:").unwrap_or_else(|| "Malus DE".to_string());
    let summary = extract_report_summary(&report_text);

    let dialog = ApplicationWindow::builder()
        .application(app)
        .title("Informe de error — Malus DE")
        .css_classes(["malus-confirmation-window", "malus-crash-window"])
        .decorated(false)
        .resizable(false)
        .build();

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .css_classes(["malus-confirmation-card", "malus-crash-card"])
        .build();

    let heading = Label::builder()
        .label(format!("{component} se cerró inesperadamente"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-heading"])
        .build();

    let body = Label::builder()
        .label("¿Quieres enviar el informe de error para ayudar a solucionar este problema?")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-body"])
        .build();

    let text_view = TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(WrapMode::WordChar)
        .css_classes(["malus-crash-text"])
        .build();
    text_view.buffer().set_text(&report_text);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .css_classes(["malus-crash-scroll"])
        .child(&text_view)
        .build();

    let details_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .build();
    let save_btn = Button::with_label("Guardar .txt...");
    save_btn.add_css_class("malus-confirmation-cancel");
    save_btn.set_halign(gtk4::Align::Start);
    details_box.append(&scroll);
    details_box.append(&save_btn);

    let details_expander = Expander::builder()
        .label("Ver detalles")
        .expanded(false)
        .css_classes(["malus-crash-expander"])
        .child(&details_box)
        .build();

    let status_label = Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["malus-crash-status"])
        .build();

    let buttons = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk4::Align::End)
        .build();

    let close_btn = Button::with_label("Cerrar");
    close_btn.add_css_class("malus-confirmation-cancel");

    let send_btn = Button::with_label("Enviar reporte");
    send_btn.add_css_class("malus-crash-primary");

    buttons.append(&close_btn);
    buttons.append(&send_btn);

    card.append(&heading);
    card.append(&body);
    card.append(&details_expander);
    card.append(&status_label);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let dialog_close = dialog.clone();
    close_btn.connect_clicked(move |_| dialog_close.close());

    let report_for_save = report_text.clone();
    let default_name = report_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("malus-crash-report.txt")
        .to_string();
    let dialog_for_save = dialog.clone();
    let status_for_save = status_label.clone();
    save_btn.connect_clicked(move |_| {
        let chooser = FileDialog::builder()
            .title("Guardar informe de error (.txt)")
            .initial_name(&default_name)
            .build();
        if let Some(folder) = default_export_folder() {
            chooser.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        let text_to_write = report_for_save.clone();
        let status_done = status_for_save.clone();
        chooser.save(Some(&dialog_for_save), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(dest_path) = file.path() {
                    status_done.set_visible(true);
                    match std::fs::write(&dest_path, text_to_write.as_bytes()) {
                        Ok(()) => {
                            status_done.set_text(&format!("✓ Guardado en {}", dest_path.display()))
                        }
                        Err(err) => status_done.set_text(&format!("No se pudo guardar: {err}")),
                    }
                }
            }
        });
    });

    let report_for_send = report_text;
    let path_for_send = report_path;
    let component_for_send = component;
    let summary_for_send = summary;
    let status_for_send = status_label;
    let send_btn_self = send_btn.clone();
    let expander_on_error = details_expander;
    send_btn.connect_clicked(move |_| {
        send_btn_self.set_sensitive(false);
        send_btn_self.set_label("Enviando...");
        status_for_send.set_visible(true);
        status_for_send.set_text("Enviando reporte...");

        let endpoint = malus_de_common::crash::crash_report_endpoint();
        let component_bg = component_for_send.clone();
        let summary_bg = summary_for_send.clone();
        let path_bg = path_for_send.clone();
        let report_bg = report_for_send.clone();

        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let result = malus_de_common::crash::send_report_via_endpoint(
                &endpoint,
                &component_bg,
                &summary_bg,
                &path_bg,
                &report_bg,
            );
            let _ = tx.send(result);
        });

        let btn_ui = send_btn_self.clone();
        let status_ui = status_for_send.clone();
        let exp_ui = expander_on_error.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(ok_msg)) => {
                    btn_ui.set_label("Enviado ✓");
                    btn_ui.set_sensitive(false);
                    status_ui.set_text(&format!("✓ {ok_msg}"));
                    glib::ControlFlow::Break
                }
                Ok(Err(err_msg)) => {
                    btn_ui.set_label("Reintentar");
                    btn_ui.set_sensitive(true);
                    exp_ui.set_expanded(true);
                    status_ui.set_text(&format!(
                        "No se pudo enviar ({err_msg}). Puedes guardar el .txt abajo."
                    ));
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    btn_ui.set_label("Reintentar");
                    btn_ui.set_sensitive(true);
                    exp_ui.set_expanded(true);
                    status_ui.set_text("Error al enviar. Puedes guardar el .txt abajo.");
                    glib::ControlFlow::Break
                }
            }
        });
    });

    let key_controller = EventControllerKey::new();
    let dialog_on_escape = dialog.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == Key::Escape {
            dialog_on_escape.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    dialog.add_controller(key_controller);
    dialog.present();
    send_btn.grab_focus();
}

fn default_export_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    for candidate in ["Escritorio", "Desktop", "Descargas", "Downloads"] {
        let p = home.join(candidate);
        if p.is_dir() {
            return Some(p);
        }
    }
    Some(home)
}

fn extract_report_field(report: &str, prefix: &str) -> Option<String> {
    for line in report.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_report_summary(report: &str) -> String {
    let mut after_marker = false;
    for line in report.lines() {
        if line.contains("--- MENSAJE DEL ERROR ---") {
            after_marker = true;
            continue;
        }
        if after_marker {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(160).collect();
            }
        }
    }
    extract_report_field(report, "Ubicación:")
        .or_else(|| extract_report_field(report, "Señal / Código:"))
        .unwrap_or_else(|| "Cierre inesperado".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_report_field_and_summary_parse_dump_text() {
        let dump = "\
Componente:  malus-de-compositor\n\
Ubicación:   state.rs:10:5\n\
\n\
--- MENSAJE DEL ERROR ---\n\
assertion failed: something went wrong\n";
        assert_eq!(
            extract_report_field(dump, "Componente:"),
            Some("malus-de-compositor".to_string())
        );
        assert_eq!(
            extract_report_summary(dump),
            "assertion failed: something went wrong"
        );
    }

    #[test]
    fn crash_window_and_card_have_solid_background_in_css() {
        let win_block = CONFIRMATION_CSS
            .split("window.malus-crash-window.background {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir regla para window.malus-crash-window");
        assert!(win_block.contains("background-color: @window_bg_color;"));
        assert!(!win_block.contains("transparent"));
        assert!(!win_block.contains("alpha("));

        let card_block = CONFIRMATION_CSS
            .split(".malus-crash-card {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir regla para .malus-crash-card");
        assert!(card_block.contains("background-color: @window_bg_color;"));
        assert!(!card_block.contains("transparent"));
        assert!(!card_block.contains("alpha("));
    }

    #[test]
    fn power_dialog_copy_matches_spanish_and_english_countdown_variants() {
        let (h_off, b_off, reopen, cancel, confirm) =
            power_dialog_copy(PowerDialogKind::Shutdown, 54, true);
        assert_eq!(h_off, "¿Quieres apagar la computadora?");
        assert_eq!(
            b_off,
            "Si no haces nada, la computadora se apagará automáticamente dentro de 54 segundos."
        );
        assert_eq!(reopen, "Volver a abrir ventanas al iniciar la sesión");
        assert_eq!(cancel, "Cancelar");
        assert_eq!(confirm, "Apagar");

        let (h_out, b_out_1, _, _, confirm_out) =
            power_dialog_copy(PowerDialogKind::Logout, 1, true);
        assert_eq!(
            h_out,
            "¿Quieres salir de todas las aplicaciones y cerrar la sesión?"
        );
        assert_eq!(
            b_out_1,
            "Si no haces nada, la sesión se cerrará automáticamente dentro de 1 segundo."
        );
        assert_eq!(confirm_out, "Cerrar sesión");

        let (h_res, b_res, _, _, confirm_res) =
            power_dialog_copy(PowerDialogKind::Restart, 55, true);
        assert_eq!(h_res, "¿Quieres reiniciar la computadora?");
        assert_eq!(
            b_res,
            "Si no haces nada, la computadora se reiniciará automáticamente dentro de 55 segundos."
        );
        assert_eq!(confirm_res, "Reiniciar");

        let (_, b_en, _, _, confirm_en) = power_dialog_copy(PowerDialogKind::Restart, 1, false);
        assert!(b_en.ends_with("1 second."));
        assert_eq!(confirm_en, "Restart");
    }

    #[test]
    fn concentric_power_button_radius_matches_window_squircle_and_clamps() {
        // Ventana por defecto (32 px) con padding 20 px -> radio concéntrico exacto de 12 px
        assert!((concentric_power_button_radius(32.0, 20.0, 28.0) - 12.0).abs() < 1e-6);
        // Radio de ventana grande se acota a la mitad de la altura del botón (14 px)
        assert!((concentric_power_button_radius(40.0, 20.0, 28.0) - 14.0).abs() < 1e-6);
        // Radio de ventana pequeño mantiene mínimo legible de 6 px
        assert!((concentric_power_button_radius(22.0, 20.0, 28.0) - 6.0).abs() < 1e-6);
    }
}
