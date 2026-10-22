use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Image, Label, MenuButton, Orientation, Popover, Separator,
};
use libadwaita as adw;
use libadwaita::prelude::*;
use tracing::info;

const APPLE_LOGO_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" fill="#ffffff" class="bi bi-apple" viewBox="0 0 16 16">
  <path d="M11.182.008C11.148-.03 9.923.023 8.857 1.18c-1.066 1.156-.902 2.482-.878 2.516s1.52.087 2.475-1.258.762-2.391.728-2.43m3.314 11.733c-.048-.096-2.325-1.234-2.113-3.422s1.675-2.789 1.698-2.854-.597-.79-1.254-1.157a3.7 3.7 0 0 0-1.563-.434c-.108-.003-.483-.095-1.254.116-.508.139-1.653.589-1.968.607-.316.018-1.256-.522-2.267-.665-.647-.125-1.333.131-1.824.328-.49.196-1.422.754-2.074 2.237-.652 1.482-.311 3.83-.067 4.56s.625 1.924 1.273 2.796c.576.984 1.34 1.667 1.659 1.899s1.219.386 1.843.067c.502-.308 1.408-.485 1.766-.472.357.013 1.061.154 1.782.539.571.197 1.111.115 1.652-.105.541-.221 1.324-1.059 2.238-2.758q.52-1.185.473-1.282"/>
</svg>"##;

pub fn create_apple_menu() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["apple-logo-btn", "topbar-btn"])
        .tooltip_text("Menú Malus DE")
        .has_frame(false)
        .always_show_arrow(false)
        .valign(gtk4::Align::Center)
        .build();

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let svg_path = std::path::PathBuf::from(&home).join(".config/malus-de/icons/apple-logo.svg");

    // Check if user has an apple.svg in Downloads or Descargas to auto-import
    let candidate_paths = [
        std::path::PathBuf::from(&home).join("Descargas/apple.svg"),
        std::path::PathBuf::from(&home).join("Downloads/apple.svg"),
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

    if !svg_path.exists() {
        if let Some(parent) = svg_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&svg_path, APPLE_LOGO_SVG);
    }

    let logo_widget: gtk4::Widget = if svg_path.exists() {
        Image::builder()
            .file(svg_path.to_str().unwrap_or(""))
            .pixel_size(16)
            .valign(gtk4::Align::Center)
            .halign(gtk4::Align::Center)
            .css_classes(["apple-icon"])
            .build()
            .upcast()
    } else {
        Label::builder()
            .label("")
            .css_classes(["apple-icon"])
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
    crate::ui::theme::register_popover(&popover);

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    // Elementos del Menú Apple
    let item_about = create_menu_item("Acerca de Malus DE", None, true);
    let popover_clone = popover.clone();
    item_about.connect_clicked(move |_| {
        popover_clone.popdown();
        show_about_dialog();
    });
    content_box.append(&item_about);

    content_box.append(&create_divider());

    let item_settings = create_menu_item("Preferencias del Sistema...", Some("⌘,"), false);
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

    let item_monitor = create_menu_item("Forzar salida de aplicaciones...", Some("⌥⌘⎋"), false);
    let popover_clone = popover.clone();
    item_monitor.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo Monitor del Sistema...");
        let mut command = std::process::Command::new("gnome-system-monitor");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.spawn();
    });
    content_box.append(&item_monitor);

    let item_reload = create_menu_item("Recargar Shell (Hot-Reload)", Some("⌥⌘R"), false);
    let popover_clone = popover.clone();
    item_reload.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Recargando componentes de interfaz de Malus DE (Hot-Reload)...");
        let bin_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin"));
        let ctl_path = if bin_dir.join("malus-de-ctl").exists() {
            bin_dir.join("malus-de-ctl")
        } else {
            bin_dir.join("malus-ctl")
        };
        let cmd = if ctl_path.exists() {
            ctl_path.to_str().unwrap().to_string()
        } else {
            "malus-de-ctl".to_string()
        };
        let _ = std::process::Command::new(&cmd).arg("reload").spawn();
    });
    content_box.append(&item_reload);

    let item_lock = create_menu_item("Bloquear Pantalla", Some("⌃⌘L"), false);
    let popover_clone = popover.clone();
    item_lock.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Bloqueando pantalla con malus-de-session-lock...");
        let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
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

    content_box.append(&create_divider());

    let item_reboot = create_menu_item("Reiniciar...", None, false);
    let popover_clone = popover.clone();
    item_reboot.connect_clicked(move |_| {
        popover_clone.popdown();
        show_confirm_dialog(
            "¿Reiniciar el equipo?",
            "El sistema cerrará todas las aplicaciones y reiniciará el equipo. Cualquier cambio no guardado se perderá.",
            "Reiniciar",
            || {
                info!("Reiniciando el sistema...");
                let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            },
        );
    });
    content_box.append(&item_reboot);

    let item_shutdown = create_menu_item("Apagar el Equipo...", None, false);
    let popover_clone = popover.clone();
    item_shutdown.connect_clicked(move |_| {
        popover_clone.popdown();
        show_confirm_dialog(
            "¿Apagar el equipo?",
            "El sistema cerrará todas las aplicaciones y apagará el equipo de forma segura.",
            "Apagar",
            || {
                info!("Apagando el sistema...");
                let _ = std::process::Command::new("systemctl").arg("poweroff").spawn();
            },
        );
    });
    content_box.append(&item_shutdown);

    content_box.append(&create_divider());

    let user_name = std::env::var("USER").unwrap_or_else(|_| "Usuario".to_string());
    let logout_label = format!("Cerrar Sesión de {}...", user_name);
    let item_logout = create_menu_item(&logout_label, Some("⇧⌘Q"), false);
    let popover_clone = popover.clone();
    item_logout.connect_clicked(move |_| {
        popover_clone.popdown();
        show_confirm_dialog(
            "¿Cerrar la sesión actual?",
            "Se cerrarán todas las aplicaciones abiertas en Malus DE y volverás a la pantalla de inicio de sesión.",
            "Cerrar Sesión",
            || {
                info!("Cerrando sesión de Malus DE...");
                let _ = std::process::Command::new("pkill")
                    .args(["-TERM", "-x", "malus-de-compositor"])
                    . status();
                let _ = std::process::Command::new("killall")
                    .args(["-TERM", "malus-de-compositor"])
                    .spawn();
            },
        );
    });
    content_box.append(&item_logout);

    popover.set_child(Some(&content_box));
    menu_btn.set_popover(Some(&popover));

    menu_btn
}

fn create_menu_item(label_text: &str, shortcut: Option<&str>, is_bold: bool) -> Button {
    let btn = Button::builder()
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(16)
        .build();

    let label = Label::builder()
        .label(label_text)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["menu-row-label"])
        .build();

    if is_bold {
        label.add_css_class("menu-item-bold");
    }
    row.append(&label);

    if let Some(sc) = shortcut {
        let sc_label = Label::builder()
            .label(sc)
            .xalign(1.0)
            .halign(gtk4::Align::End)
            .css_classes(["menu-shortcut-hint"])
            .build();
        row.append(&sc_label);
    }

    btn.set_child(Some(&row));
    btn
}

fn create_divider() -> Separator {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("menu-separator");
    sep
}

fn show_about_dialog() {
    let about_window = adw::AboutDialog::builder()
        .application_name("Malus Desktop Environment")
        .version("0.1.0-alpha")
        .developer_name("Equipo Malus DE")
        .comments("Entorno de Escritorio Wayland Autónomo y Ultraligero escrito en Rust y Smithay. Fusión de diseño macOS y gestión de ventanas avanzada.")
        .website("https://github.com/cristian/malus-de")
        .issue_url("https://github.com/cristian/malus-de/issues")
        .license_type(gtk4::License::MitX11)
        .build();

    about_window.present(None as Option<&gtk4::Window>);
}

fn show_confirm_dialog(title: &str, body: &str, confirm_label: &str, on_confirm: impl FnOnce() + 'static) {
    let dialog = adw::MessageDialog::builder()
        .heading(title)
        .body(body)
        .build();
    dialog.add_response("cancel", "Cancelar");
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let on_confirm = std::cell::RefCell::new(Some(on_confirm));
    dialog.connect_response(None, move |d, resp| {
        if resp == "confirm" {
            if let Some(cb) = on_confirm.borrow_mut().take() {
                cb();
            }
        }
        d.close();
    });
    dialog.present();
}
