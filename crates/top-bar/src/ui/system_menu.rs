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
        Image::builder()
            .file(svg_path.to_str().unwrap_or(""))
            .pixel_size(16)
            .valign(gtk4::Align::Center)
            .halign(gtk4::Align::Center)
            .css_classes(["menu-logo-icon"])
            .build()
            .upcast()
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
        .margin_top(2)
        .margin_bottom(2)
        .margin_start(2)
        .margin_end(2)
        .build();

    let es = is_spanish_locale();

    let label_about = if es { "Acerca de este equipo" } else { "About This Computer" };
    let label_settings = if es { "Configuración del Sistema..." } else { "System Settings..." };
    let label_appstore = if es { "Software" } else { "Software" };
    let label_recent = if es { "Elementos recientes" } else { "Recent Items" };
    let label_forcequit = if es { "Forzar salida..." } else { "Force Quit..." };
    let label_sleep = if es { "Reposo" } else { "Sleep" };
    let label_restart = if es { "Reiniciar..." } else { "Restart..." };
    let label_shutdown = if es { "Apagar equipo..." } else { "Shut Down..." };
    let label_lock = if es { "Bloquear pantalla" } else { "Lock Screen" };

    let user_name = std::env::var("USER").unwrap_or_else(|_| if es { "Usuario".to_string() } else { "User".to_string() });
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
    let item_appstore = create_menu_item("system-software-install-symbolic", label_appstore, None, false);
    let popover_clone = popover.clone();
    item_appstore.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo tienda de aplicaciones...");
        let stores = ["gnome-software", "io.elementary.appcenter", "plasma-discover"];
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
        let _ = std::process::Command::new("xdg-open").arg("https://flathub.org").spawn();
    });
    content_box.append(&item_appstore);

    content_box.append(&create_divider());

    // 4. Recent Items / Elementos recientes
    let item_recent = create_menu_item("apple-recent-symbolic", label_recent, None, true);
    let popover_clone = popover.clone();
    item_recent.connect_clicked(move |_| {
        popover_clone.popdown();
        info!("Abriendo elementos recientes...");
        let _ = std::process::Command::new("nautilus").arg("recent:///").spawn();
    });
    content_box.append(&item_recent);

    content_box.append(&create_divider());

    // 5. Force Quit... / Forzar salida...
    let item_monitor = create_menu_item("apple-forcequit-symbolic", label_forcequit, Some("⌥⌘⎋"), false);
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
        let _ = std::process::Command::new("systemctl").arg("suspend").spawn();
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
        let (dlg_title, dlg_body, dlg_confirm, dlg_cancel) = if is_spanish_locale() {
            ("¿Reiniciar el equipo?", "El sistema cerrará todas las aplicaciones y reiniciará el equipo. Cualquier cambio no guardado se perderá.", "Reiniciar", "Cancelar")
        } else {
            ("Restart the computer?", "The system will close all applications and restart the computer. Any unsaved changes will be lost.", "Restart", "Cancel")
        };
        show_confirm_dialog(
            dlg_title,
            dlg_body,
            dlg_confirm,
            dlg_cancel,
            parent.as_ref(),
            || {
                info!("Reiniciando el sistema...");
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
        let (dlg_title, dlg_body, dlg_confirm, dlg_cancel) = if is_spanish_locale() {
            ("¿Apagar el equipo?", "El sistema cerrará todas las aplicaciones y apagará el equipo de forma segura.", "Apagar", "Cancelar")
        } else {
            ("Shut down the computer?", "The system will close all applications and safely power off the computer.", "Shut Down", "Cancel")
        };
        show_confirm_dialog(
            dlg_title,
            dlg_body,
            dlg_confirm,
            dlg_cancel,
            parent.as_ref(),
            || {
                info!("Apagando el sistema...");
                let _ = std::process::Command::new("systemctl").arg("poweroff").spawn();
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

    // 10. Log Out / Cerrar sesión
    let item_logout = create_menu_item("apple-logout-symbolic", &label_logout, Some("⇧⌘Q"), false);
    let popover_clone = popover.clone();
    let menu_btn_clone = menu_btn.clone();
    item_logout.connect_clicked(move |_| {
        popover_clone.popdown();
        let parent = menu_btn_clone
            .root()
            .and_then(|root| root.downcast::<ApplicationWindow>().ok());
        let (dlg_title, dlg_body, dlg_confirm, dlg_cancel) = if is_spanish_locale() {
            ("¿Cerrar la sesión actual?", "Se cerrarán todas las aplicaciones abiertas en Malus DE y volverás a la pantalla de inicio de sesión.", "Cerrar Sesión", "Cancelar")
        } else {
            ("Log out of current session?", "All open applications in Malus DE will be closed and you will return to the login screen.", "Log Out", "Cancel")
        };
        show_confirm_dialog(
            dlg_title,
            dlg_body,
            dlg_confirm,
            dlg_cancel,
            parent.as_ref(),
            || {
                info!("Cerrando sesión de Malus DE...");
                let _ = std::process::Command::new("pkill")
                    .args(["-TERM", "-x", "malus-de-compositor"])
                    .status();
                let _ = std::process::Command::new("killall")
                    .args(["-TERM", "malus-de-compositor"])
                    .spawn();
            },
        );
    });
    content_box.append(&item_logout);

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
    btn
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
    let dev = if es { "Equipo Malus DE" } else { "Malus DE Team" };

    let about_window = adw::AboutDialog::builder()
        .application_name("Malus Desktop Environment")
        .version("0.1.0-alpha")
        .developer_name(dev)
        .comments(comments)
        .website("https://github.com/cristian/malus-de")
        .issue_url("https://github.com/cristian/malus-de/issues")
        .license_type(gtk4::License::MitX11)
        .build();

    about_window.present(None as Option<&gtk4::Window>);
}

fn show_confirm_dialog(
    title: &str,
    body: &str,
    confirm_label: &str,
    cancel_label: &str,
    parent: Option<&ApplicationWindow>,
    on_confirm: impl FnOnce() + 'static,
) {
    let app = parent.and_then(|parent| parent.application());
    if let Some(app) = app {
        let parent = parent.map(|parent| parent.upcast_ref());
        malus_de_dialogs::show_confirmation(
            &app,
            parent,
            title,
            body,
            confirm_label,
            on_confirm,
            || {},
        );
        return;
    }

    let dialog = adw::MessageDialog::builder()
        .title(if is_spanish_locale() { "Confirmación de acción del sistema" } else { "System Action Confirmation" })
        .heading(title)
        .body(body)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }
    dialog.add_response("cancel", cancel_label);
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let on_confirm = std::cell::RefCell::new(Some(on_confirm));
    dialog.connect_response(None, move |d, resp| {
        if resp == "confirm" {
            if let Some(callback) = on_confirm.borrow_mut().take() {
                callback();
            }
        }
        d.close();
    });
    dialog.present();
}
