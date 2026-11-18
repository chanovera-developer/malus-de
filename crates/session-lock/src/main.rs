use anyhow::Result;
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Image, Label, Orientation,
    PasswordEntry,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::{config::DisplaysConfig, logging::init_logging};
use std::time::Duration;
use tracing::info;

const LOCK_CSS: &str = include_str!("styles/lock.css");

fn main() -> Result<()> {
    init_logging("malus_de_session_lock");
    info!("Iniciando Malus DE Session Lock (Lock Screen)...");

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

    app.connect_activate(build_lock_ui);

    app.run_with_args(&["malus-de-session-lock"]);
    Ok(())
}

fn build_lock_ui(app: &Application) {
    let monitor = gtk4::gdk::Display::default().and_then(|display| {
        let monitors: Vec<gtk4::gdk::Monitor> = {
            let monitors = display.monitors();
            (0..monitors.n_items())
                .filter_map(|index| monitors.item(index))
                .filter_map(|monitor| monitor.downcast().ok())
                .collect()
        };
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
                        description.contains(&config.name)
                            || description.contains(&config.connector)
                    }) {
                        return true;
                    }
                    let geometry = monitor.geometry();
                    geometry.x() == config.x && geometry.y() == config.y
                })
            })
            .unwrap_or(0);
        monitors.get(primary_index).cloned()
    });

    build_lock_window(app, monitor.as_ref());
}

fn build_lock_window(app: &Application, monitor: Option<&gtk4::gdk::Monitor>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bloqueo de Sesión")
        .css_classes(["malus-lock-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        info!("Bloqueando pantalla completa con foco exclusivo en Overlay...");
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
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_exclusive_zone(-1);
    } else {
        if let Some(monitor) = monitor {
            window.set_monitor(monitor);
        }
        window.set_decorated(false);
        window.maximize();
    }

    let root_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::Center)
        .spacing(8)
        .build();

    // 1. Reloj Gigante
    let clock_label = Label::builder().css_classes(["giant-clock"]).build();
    let date_label = Label::builder().css_classes(["lock-date"]).build();

    update_lock_time(&clock_label, &date_label);
    let c_clone = clock_label.clone();
    let d_clone = date_label.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        update_lock_time(&c_clone, &d_clone);
        ControlFlow::Continue
    });

    root_box.append(&clock_label);
    root_box.append(&date_label);

    // 2. Avatar de Usuario
    let avatar_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .css_classes(["avatar-circle"])
        .build();
    let avatar_icon = Image::builder()
        .icon_name("avatar-default-symbolic")
        .pixel_size(64)
        .css_classes(["avatar-icon"])
        .build();
    avatar_box.append(&avatar_icon);
    root_box.append(&avatar_box);

    let user_name = std::env::var("USER").unwrap_or_else(|_| "Usuario".to_string());
    let user_label = Label::builder()
        .label(&user_name)
        .css_classes(["username-label"])
        .build();
    root_box.append(&user_label);

    // 3. Campo de Contraseña
    let input_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(gtk4::Align::Center)
        .spacing(6)
        .build();

    let password_entry = PasswordEntry::builder()
        .placeholder_text("Introduce la contraseña")
        .css_classes(["lock-password-entry"])
        .show_peek_icon(true)
        .build();
    input_box.append(&password_entry);

    let unlock_btn = Button::builder()
        .icon_name("go-next-symbolic")
        .css_classes(["unlock-btn", "suggested-action"])
        .build();
    input_box.append(&unlock_btn);
    root_box.append(&input_box);

    // Desbloqueo al pulsar Enter o el botón
    let app_unlock = app.clone();
    let pwd_clone = password_entry.clone();
    let attempt_unlock = move || {
        let text = pwd_clone.text();
        info!("Intento de desbloqueo recibido (longitud={})", text.len());
        // En entorno de desarrollo o con cualquier tecla pulsada desbloquea
        info!("Desbloqueando sesión de Malus DE...");
        app_unlock.quit();
    };

    let u_btn = attempt_unlock.clone();
    unlock_btn.connect_clicked(move |_| {
        u_btn();
    });

    let u_entry = attempt_unlock.clone();
    password_entry.connect_activate(move |_| {
        u_entry();
    });

    window.set_child(Some(&root_box));
    window.present();
    password_entry.grab_focus();
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
