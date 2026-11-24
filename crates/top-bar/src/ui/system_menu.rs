use gtk4::prelude::*;
use gtk4::{
    ApplicationWindow, Box as GtkBox, Button, Image, Label, MenuButton, Orientation, Popover,
    Separator,
};
use libadwaita as adw;
use libadwaita::prelude::*;
use tracing::info;

const MENU_LOGO_SVG: &str = include_str!("../../../../data/icons/menu-logo.svg");
const PREVIOUS_DEFAULT_LOGO_MARK: &str = "M11.182.008C11.148-.03";

thread_local! {
    static SYSTEM_MENU_LOGO_IMG: std::cell::RefCell<Option<glib::WeakRef<Image>>> =
        const { std::cell::RefCell::new(None) };
    static SYSTEM_MENU_LOGO_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

pub fn refresh_system_menu_logo() {
    let show_bg = malus_de_common::config::MenuBarConfig::load().show_background;
    let dark = malus_de_common::config::is_dark_mode_active();
    let hex_fg = if show_bg && !dark { "#1d1d1f" } else { "#ffffff" };
    SYSTEM_MENU_LOGO_IMG.with(|img_cell| {
        let Some(img) = img_cell.borrow().as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        let raw_svg = SYSTEM_MENU_LOGO_PATH.with(|path_cell| {
            path_cell
                .borrow()
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
        })
        .unwrap_or_else(|| MENU_LOGO_SVG.to_string());
        let tinted_svg = raw_svg
            .replace("fill=\"#ffffff\"", &format!("fill=\"{hex_fg}\""))
            .replace("fill=\"#FFFFFF\"", &format!("fill=\"{hex_fg}\""))
            .replace("fill=\"currentColor\"", &format!("fill=\"{hex_fg}\""));
        if let Ok(loader) = gtk4::gdk_pixbuf::PixbufLoader::with_type("svg") {
            let _ = loader.set_size(16, 16);
            if loader.write(tinted_svg.as_bytes()).is_ok() && loader.close().is_ok() {
                if let Some(pb) = loader.pixbuf() {
                    let texture = gtk4::gdk::Texture::for_pixbuf(&pb);
                    img.set_paintable(Some(&texture));
                }
            }
        }
    });
}

pub fn create_system_menu() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["menu-logo-btn", "topbar-btn"])
        .tooltip_text("Menú Malus DE")
        .has_frame(false)
        .always_show_arrow(false)
        .valign(gtk4::Align::Center)
        .build();

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let svg_path = std::path::PathBuf::from(&home).join(".config/malus-de/icons/menu-logo.svg");

    let candidate_paths = [
        std::path::PathBuf::from(&home).join("Descargas/menu-logo.svg"),
        std::path::PathBuf::from(&home).join("Downloads/menu-logo.svg"),
    ];

    for candidate in &candidate_paths {
        if candidate.exists() {
            if let Ok(content) = std::fs::read_to_string(candidate) {
                let white_content = if content.contains("fill=\"currentColor\"") {
                    content.replace("fill=\"currentColor\"", "fill=\"#ffffff\"")
                } else if !content.contains("fill=") {
                    content.replace("<svg ", "<svg fill=\"#ffffff\" ")
                } else {
                    content
                };
                if let Some(parent) = svg_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&svg_path, white_content);
                break;
            }
        }
    }

    let update_default_logo = if svg_path.exists() {
        match std::fs::read_to_string(&svg_path) {
            Ok(content) => content.contains(PREVIOUS_DEFAULT_LOGO_MARK),
            Err(error) => {
                tracing::warn!("No se pudo leer el logotipo del menú: {error}");
                false
            }
        }
    } else {
        true
    };

    if update_default_logo {
        if let Some(parent) = svg_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(&svg_path, MENU_LOGO_SVG) {
            tracing::warn!("No se pudo guardar el logotipo predeterminado del menú: {error}");
        }
    }

    let logo_widget: gtk4::Widget = if svg_path.exists() {
        let img = Image::builder()
            .file(svg_path.to_str().unwrap_or(""))
            .pixel_size(16)
            .valign(gtk4::Align::Center)
            .halign(gtk4::Align::Center)
            .css_classes(["menu-logo-icon"])
            .build();
        SYSTEM_MENU_LOGO_PATH.with(|cell| {
            *cell.borrow_mut() = Some(svg_path);
        });
        SYSTEM_MENU_LOGO_IMG.with(|cell| {
            *cell.borrow_mut() = Some(img.downgrade());
        });
        refresh_system_menu_logo();
        img.upcast()
    } else {
        Label::builder()
            .label("M")
            .css_classes(["menu-logo-icon"])
            .build()
            .upcast()
    };
    menu_btn.set_child(Some(&logo_widget));

    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(gtk4::Align::Start)
        .build();
    popover.set_offset(0, 4);

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .margin_top(0)
        .margin_bottom(0)
        .margin_start(0)
        .margin_end(0)
        .build();

    let es = is_spanish_locale();

    let label_about = if es {
        "Acerca de este equipo"
    } else {
        "About This Computer"
    };
    let label_settings = if es {
        "Configuración del Sistema..."
    } else {
        "System Settings..."
    };
    let label_appstore = if es { "Software" } else { "Software" };
    let label_recent = if es {
        "Elementos recientes"
    } else {
        "Recent Items"
    };
    let label_forcequit = if es {
        "Forzar salida..."
    } else {
        "Force Quit..."
    };
    let label_sleep = if es { "Reposo" } else { "Sleep" };
    let label_restart = if es { "Reiniciar..." } else { "Restart..." };
    let label_shutdown = if es {
        "Apagar equipo..."
    } else {
        "Shut Down..."
    };
    let label_lock = if es {
        "Bloquear pantalla"
    } else {
        "Lock Screen"
    };

    let user_name = std::env::var("USER").unwrap_or_else(|_| {
        if es {
            "Usuario".to_string()
        } else {
            "User".to_string()
        }
    });
    let label_logout = if es {
        format!("Cerrar sesión de {}...", user_name)
    } else {
        format!("Log Out {}...", user_name)
    };

    // Elementos del menú del sistema según idioma del sistema y estilo macOS
    // 1. About This Computer / Acerca de este equipo
    let item_about = create_menu_item("help-about-symbolic", label_about, None, false);
    let popover_clone = popover.clone();
    item_about.connect_clicked(move |_| {
        popover_clone.popdown();
        show_about_dialog();
    });
    content_box.append(&item_about);

    content_box.append(&create_divider());

    // 2. System Settings... / Configuración del Sistema...
    let item_settings = create_menu_item("apple-settings-symbolic", label_settings, None, false);
    let popover_clone = popover.clone();
    item_settings.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo Ajustes del Sistema de Malus DE (malus-de-settings)...");
        let wayland_display =
            std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
        let mut command = std::process::Command::new(&exe);
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("GDK_BACKEND", "wayland")
            .env("GSK_RENDERER", "cairo")
            .spawn();
    });
    content_box.append(&item_settings);

    // 3. Software
    let item_appstore = create_menu_item(
        "system-software-install-symbolic",
        label_appstore,
        None,
        false,
    );
    let popover_clone = popover.clone();
    item_appstore.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo tienda de aplicaciones...");
        let stores = [
            "gnome-software",
            "io.elementary.appcenter",
            "plasma-discover",
        ];
        for store in stores {
            if std::process::Command::new("which")
                .arg(store)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let _ = std::process::Command::new(store).spawn();
                return;
            }
        }
        let _ = std::process::Command::new("xdg-open")
            .arg("https://flathub.org")
            .spawn();
    });
    content_box.append(&item_appstore);

    content_box.append(&create_divider());

    // 4. Recent Items / Elementos recientes
    let item_recent = create_menu_item("apple-recent-symbolic", label_recent, None, true);
    let popover_clone = popover.clone();
    item_recent.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo elementos recientes...");
        let _ = std::process::Command::new("nautilus")
            .arg("recent:///")
            .spawn();
    });
    content_box.append(&item_recent);

    content_box.append(&create_divider());

    // 5. Force Quit... / Forzar salida...
    let item_monitor = create_menu_item(
        "apple-forcequit-symbolic",
        label_forcequit,
        Some("⌥⌘⎋"),
        false,
    );
    let popover_clone = popover.clone();
    item_monitor.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo Monitor del Sistema...");
        let mut command = std::process::Command::new("gnome-system-monitor");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.spawn();
    });
    content_box.append(&item_monitor);

    content_box.append(&create_divider());

    // 6. Sleep / Reposo
    let item_sleep = create_menu_item("apple-sleep-symbolic", label_sleep, None, false);
    let popover_clone = popover.clone();
    item_sleep.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Suspendiendo el equipo...");
        let _ = std::process::Command::new("systemctl")
            .arg("suspend")
            .spawn();
    });
    content_box.append(&item_sleep);

    // 7. Restart... / Reiniciar...
    let item_reboot = create_menu_item("apple-restart-symbolic", label_restart, None, false);
    let popover_clone = popover.clone();
    let menu_btn_clone = menu_btn.clone();
    item_reboot.connect_clicked(move |_| {
        popover_clone.popdown();
        let parent = menu_btn_clone
            .root()
            .and_then(|root| root.downcast::<ApplicationWindow>().ok());
        show_power_action_dialog(
            malus_de_dialogs::PowerDialogKind::Restart,
            parent.as_ref(),
            |_reopen_windows| {
                info!("Reiniciando el sistema...");
                malus_de_common::crash::mark_clean_session_exit();
                let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            },
        );
    });
    content_box.append(&item_reboot);

    // 8. Shut Down... / Apagar equipo...
    let item_shutdown = create_menu_item("apple-shutdown-symbolic", label_shutdown, None, false);
    let popover_clone = popover.clone();
    let menu_btn_clone = menu_btn.clone();
    item_shutdown.connect_clicked(move |_| {
        popover_clone.popdown();
        let parent = menu_btn_clone
            .root()
            .and_then(|root| root.downcast::<ApplicationWindow>().ok());
        show_power_action_dialog(
            malus_de_dialogs::PowerDialogKind::Shutdown,
            parent.as_ref(),
            |_reopen_windows| {
                info!("Apagando el sistema...");
                malus_de_common::crash::mark_clean_session_exit();
                let _ = std::process::Command::new("systemctl")
                    .arg("poweroff")
                    .spawn();
            },
        );
    });
    content_box.append(&item_shutdown);

    content_box.append(&create_divider());

    // 9. Lock Screen / Bloquear pantalla
    let item_lock = create_menu_item("apple-lock-symbolic", label_lock, Some("^⌘Q"), false);
    let popover_clone = popover.clone();
    item_lock.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Bloqueando pantalla con malus-de-session-lock...");
        let wayland_display =
            std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-session-lock")))
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-session-lock"));

        let mut command = std::process::Command::new(&exe);
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("GDK_BACKEND", "wayland")
            .spawn();
    });
    content_box.append(&item_lock);

    // 10. Log Out / Cerrar sesión
    let item_logout = create_menu_item("apple-logout-symbolic", &label_logout, Some("⇧⌘Q"), false);
    let popover_clone = popover.clone();
    let menu_btn_clone = menu_btn.clone();
    item_logout.connect_clicked(move |_| {
        popover_clone.popdown();
        let parent = menu_btn_clone
            .root()
            .and_then(|root| root.downcast::<ApplicationWindow>().ok());
        show_power_action_dialog(
            malus_de_dialogs::PowerDialogKind::Logout,
            parent.as_ref(),
            |_reopen_windows| {
                info!("Cerrando sesión de Malus DE...");
                malus_de_common::crash::mark_clean_session_exit();
                let _ = std::process::Command::new("pkill")
                    .args(["-TERM", "-f", "malus-de-compositor"])
                    .status();
                let _ = std::process::Command::new("killall")
                    .args(["-TERM", "malus-de-compositor"])
                    .spawn();
            },
        );
    });
    content_box.append(&item_logout);

    let recent_btn_c = item_recent.clone();
    popover.connect_show(move |_| {
        let limit = malus_de_common::config::MenuBarConfig::load().recent_items_count;
        recent_btn_c.set_visible(limit > 0);
    });
    popover.set_child(Some(&content_box));
    crate::ui::theme::register_popover(&popover);
    menu_btn.set_popover(Some(&popover));

    menu_btn
}

fn create_menu_item(
    icon_name: &str,
    label_text: &str,
    shortcut: Option<&str>,
    has_submenu: bool,
) -> Button {
    let btn = Button::builder()
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let icon = Image::builder()
        .icon_name(icon_name)
        .pixel_size(15)
        .valign(gtk4::Align::Center)
        .css_classes(["menu-item-icon"])
        .build();
    row.append(&icon);

    let label = Label::builder()
        .label(label_text)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["menu-row-label"])
        .build();
    row.append(&label);

    if has_submenu {
        let chevron = Label::builder()
            .label("›")
            .xalign(1.0)
            .halign(gtk4::Align::End)
            .css_classes(["menu-shortcut-hint", "menu-submenu-chevron"])
            .build();
        row.append(&chevron);
    } else if let Some(sc) = shortcut {
        let sc_label = Label::builder()
            .label(sc)
            .xalign(1.0)
            .halign(gtk4::Align::End)
            .css_classes(["menu-shortcut-hint"])
            .build();
        row.append(&sc_label);
    }

    btn.set_child(Some(&row));
    attach_squircle_menu_hover(&btn);
    btn
}

pub(crate) fn squircle_path(
    context: &gtk4::cairo::Context,
    width: f64,
    height: f64,
    radius: f64,
) {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(1.0);
    let power = 2.0 / 4.2;
    let quarter_turn = std::f64::consts::FRAC_PI_2;
    const SEGMENTS: usize = 20;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }
    context.close_path();
}

/// Envuelve el contenido de un botón de menú en un Overlay con un squircle Cairo del color de acento.
pub fn attach_squircle_menu_hover(btn: &Button) {
    let Some(inner) = btn.child() else {
        return;
    };
    btn.add_css_class("malus-squircle-menu-item");
    btn.set_child(gtk4::Widget::NONE);
    inner.set_margin_top(3);
    inner.set_margin_bottom(3);
    inner.set_margin_start(8);
    inner.set_margin_end(8);

    let bg = gtk4::DrawingArea::new();
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
        if flags.contains(gtk4::StateFlags::INSENSITIVE) {
            return;
        }
        let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
        let active = flags.contains(gtk4::StateFlags::ACTIVE);
        if !hovered && !active {
            return;
        }
        let _ = cr.save();
        squircle_path(cr, width, height, 14.0);
        let (red, green, blue) = malus_de_common::config::load_accent_color();
        if active {
            cr.set_source_rgba(red * 0.78, green * 0.78, blue * 0.78, 0.86);
        } else {
            cr.set_source_rgba(red, green, blue, 0.74);
        }
        let _ = cr.fill();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = gtk4::Overlay::builder().hexpand(true).build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner);
    overlay.set_measure_overlay(&inner, true);
    btn.set_child(Some(&overlay));
}

/// Radio squircle (n=4.2) para filas interactivas de los paneles de la bandeja y Centro de Control.
const TRAY_ROW_SQUIRCLE_RADIUS: f64 = 12.0;

/// Envuelve el contenido de un botón de la bandeja (Bluetooth, Wi-Fi, Sonido) en un Overlay
/// con fondo squircle Cairo (n=4.2) sin alterar el color del texto en hover.
pub(crate) fn attach_tray_squircle_button(
    btn: &Button,
    is_active_row: bool,
    show_idle_tint: bool,
    pad_v: i32,
    pad_h: i32,
) {
    let Some(inner) = btn.child() else {
        return;
    };
    btn.add_css_class("malus-tray-squircle-row");
    btn.set_halign(gtk4::Align::Fill);
    btn.set_valign(gtk4::Align::Start);
    btn.set_hexpand(true);
    btn.set_vexpand(false);
    btn.set_child(gtk4::Widget::NONE);

    inner.set_margin_top(pad_v);
    inner.set_margin_bottom(pad_v);
    inner.set_margin_start(pad_h);
    inner.set_margin_end(pad_h);
    inner.set_halign(gtk4::Align::Fill);
    inner.set_hexpand(true);
    inner.set_vexpand(false);

    let bg = gtk4::DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(false);
    bg.set_valign(gtk4::Align::Fill);

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
        if flags.contains(gtk4::StateFlags::INSENSITIVE) {
            return;
        }
        let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
        let pressed = flags.contains(gtk4::StateFlags::ACTIVE);
        let dark = libadwaita::StyleManager::default().is_dark();

        if !is_active_row && !show_idle_tint && !hovered && !pressed {
            return;
        }

        let _ = cr.save();
        cr.translate(0.5, 0.5);
        squircle_path(cr, width - 1.0, height - 1.0, TRAY_ROW_SQUIRCLE_RADIUS);

        if is_active_row {
            let (r, g, b_col) = malus_de_common::config::load_accent_color();
            let fill_alpha = if pressed {
                if dark { 0.32 } else { 0.25 }
            } else if hovered {
                if dark { 0.28 } else { 0.22 }
            } else if dark {
                0.20
            } else {
                0.15
            };
            cr.set_source_rgba(r, g, b_col, fill_alpha);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(r, g, b_col, if dark { 0.40 } else { 0.35 });
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        } else {
            let fill_rgba = if pressed {
                if dark {
                    (1.0, 1.0, 1.0, 0.16)
                } else {
                    (0.0, 0.0, 0.0, 0.13)
                }
            } else if hovered {
                if dark {
                    (1.0, 1.0, 1.0, 0.10)
                } else {
                    (0.0, 0.0, 0.0, 0.08)
                }
            } else if dark {
                (1.0, 1.0, 1.0, 0.05)
            } else {
                (0.0, 0.0, 0.0, 0.04)
            };
            cr.set_source_rgba(fill_rgba.0, fill_rgba.1, fill_rgba.2, fill_rgba.3);
            if show_idle_tint {
                let _ = cr.fill_preserve();
                let stroke_a = if hovered {
                    if dark { 0.08 } else { 0.10 }
                } else if dark {
                    0.04
                } else {
                    0.05
                };
                if dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, stroke_a);
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, stroke_a);
                }
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            } else {
                let _ = cr.fill();
            }
        }
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    let overlay = gtk4::Overlay::builder()
        .halign(gtk4::Align::Fill)
        .valign(gtk4::Align::Start)
        .hexpand(true)
        .vexpand(false)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner);
    overlay.set_measure_overlay(&inner, true);
    btn.set_child(Some(&overlay));
}

/// Envuelve una fila activa no interactiva (como la red Wi-Fi actual) en un fondo squircle Cairo (n=4.2).
pub(crate) fn wrap_tray_squircle_active_card(
    inner: &GtkBox,
    pad_v: i32,
    pad_h: i32,
) -> gtk4::Overlay {
    inner.set_margin_top(pad_v);
    inner.set_margin_bottom(pad_v);
    inner.set_margin_start(pad_h);
    inner.set_margin_end(pad_h);
    inner.set_halign(gtk4::Align::Fill);
    inner.set_hexpand(true);
    inner.set_vexpand(false);

    let bg = gtk4::DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(false);
    bg.set_valign(gtk4::Align::Fill);

    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let dark = libadwaita::StyleManager::default().is_dark();
        let (r, g, b_col) = malus_de_common::config::load_accent_color();
        let _ = cr.save();
        cr.translate(0.5, 0.5);
        squircle_path(cr, width - 1.0, height - 1.0, TRAY_ROW_SQUIRCLE_RADIUS);
        cr.set_source_rgba(r, g, b_col, if dark { 0.20 } else { 0.15 });
        let _ = cr.fill_preserve();
        cr.set_source_rgba(r, g, b_col, if dark { 0.40 } else { 0.35 });
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let _ = cr.restore();
    });

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    let overlay = gtk4::Overlay::builder()
        .halign(gtk4::Align::Fill)
        .valign(gtk4::Align::Start)
        .hexpand(true)
        .vexpand(false)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(inner);
    overlay.set_measure_overlay(inner, true);
    overlay
}

fn create_divider() -> Separator {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("menu-separator");
    sep
}

fn is_spanish_locale() -> bool {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es")
}

fn show_about_dialog() {
    let es = is_spanish_locale();
    let comments = if es {
        "Entorno de Escritorio Wayland Autónomo y Ultraligero escrito en Rust y Smithay. Fusión de diseño macOS y gestión de ventanas avanzada."
    } else {
        "Autonomous, ultra-lightweight Wayland Desktop Environment written in Rust and Smithay. Blending macOS design with advanced window management."
    };
    let dev = if es {
        "Equipo Malus DE"
    } else {
        "Malus DE Team"
    };

    let about_window = adw::AboutDialog::builder()
        .application_name("Malus Desktop Environment")
        .version("0.1.0-alpha")
        .developer_name(dev)
        .comments(comments)
        .website("https://github.com/cristian/malus-de")
        .issue_url("https://github.com/cristian/malus-de/issues")
        .license_type(gtk4::License::Gpl30)
        .build();

    about_window.present(None as Option<&gtk4::Window>);
}

fn show_power_action_dialog(
    kind: malus_de_dialogs::PowerDialogKind,
    parent: Option<&ApplicationWindow>,
    on_confirm: impl FnOnce(bool) + 'static,
) {
    if let Some(app) = parent.and_then(|parent| parent.application()) {
        let parent = parent.map(|parent| parent.upcast_ref());
        malus_de_dialogs::show_power_dialog(&app, parent, kind, on_confirm, || {});
        return;
    }

    let (title, body, _, cancel_label, confirm_label) = malus_de_dialogs::power_dialog_copy(
        kind,
        malus_de_dialogs::POWER_DIALOG_COUNTDOWN_SECS,
        is_spanish_locale(),
    );
    let dialog = adw::MessageDialog::builder()
        .title(if is_spanish_locale() {
            "Confirmación de acción del sistema"
        } else {
            "System Action Confirmation"
        })
        .heading(title)
        .body(&body)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }
    dialog.add_response("cancel", cancel_label);
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("confirm"));
    dialog.set_close_response("cancel");

    let on_confirm = std::cell::RefCell::new(Some(on_confirm));
    dialog.connect_response(None, move |d, resp| {
        if resp == "confirm" {
            if let Some(callback) = on_confirm.borrow_mut().take() {
                callback(malus_de_dialogs::load_reopen_windows_pref());
            }
        }
        d.close();
    });
    dialog.present();
}
