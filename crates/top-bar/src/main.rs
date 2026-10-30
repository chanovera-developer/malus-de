use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::logging::init_logging;
use tracing::info;

mod ui;
pub mod services;

const STYLE_CSS: &str = include_str!("styles/style.css");

fn main() {
    init_logging("malus_de_top_bar");
    info!("Iniciando Malus DE Top Bar (GTK4 / Layer Shell)...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let app = Application::builder()
        .application_id("org.malus_de.TopBar")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_ui);

    app.run();
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(STYLE_CSS);

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );

        let icon_theme = gtk4::IconTheme::for_display(&display);
        icon_theme.add_search_path("/usr/local/share/malus-de/icons");
        icon_theme.add_search_path("/usr/share/malus-de/icons");
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                icon_theme.add_search_path(parent.join("assets/icons"));
                icon_theme.add_search_path(parent.join("../../assets/icons"));
            }
        }
    }
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus DE Top Bar")
        .default_height(36)
        .css_classes(["malus-topbar"])
        .build();

    window.set_size_request(-1, 36);
    window.set_default_size(-1, 36);

    window.set_decorated(false);

    if gtk4_layer_shell::is_supported() {
        info!("Protocolo Layer Shell soportado. Anclando Top Bar en capa Top...");
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace("malus-top-bar");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.auto_exclusive_zone_enable();
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    } else {
        info!("Layer Shell no detectado en este display; ejecutando ventana flotante.");
    }

    let root_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["topbar-content"])
        .valign(gtk4::Align::Center)
        .spacing(8)
        .build();

    // 1. Zona Izquierda: Menú del sistema + Global Menu (con soporte com.canonical.AppMenu.Registrar)
    let left_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(gtk4::Align::Center)
        .spacing(4)
        .build();

    let (dbus_menu_service, registrar) = crate::services::dbusmenu::DbusMenuService::new();
    crate::services::dbusmenu::DbusMenuService::start_registrar_server(registrar);
    let dbus_service_arc = std::sync::Arc::new(dbus_menu_service);

    let menu_handles = ui::global_menu::create_global_menu_widget(dbus_service_arc.clone());
    left_box.append(&ui::system_menu::create_system_menu());
    left_box.append(&menu_handles.container);

    // Escuchar eventos de registro de ventanas con menús D-Bus
    let handles_for_reg = menu_handles.clone();
    let reg_rx = dbus_service_arc.update_rx.clone();
    glib::spawn_future_local(async move {
        while let Ok(reg) = reg_rx.recv().await {
            let (app_id, title, window_id) = {
                let cur = handles_for_reg.current_active.borrow();
                (cur.0.clone(), cur.1.clone(), cur.2)
            };
            let should_update = match window_id {
                Some(wid) => (wid as u32) == reg.window_id,
                None => true,
            };
            if should_update {
                info!("🔄 [TopBar] Actualizando menú dinámico tras registro D-Bus de ventana {}", reg.window_id);
                ui::global_menu::update_global_menu(
                    &handles_for_reg,
                    app_id.as_deref(),
                    title.as_deref(),
                    window_id,
                );
            }
        }
    });

    // Escuchar eventos IPC del compositor para actualizar el nombre de la aplicación activa y acciones
    let (tx, rx) = async_channel::unbounded::<malus_de_common::ipc::IpcMessage>();
    let title_label = menu_handles.app_title_label.clone();
    let hide_btn = menu_handles.hide_btn.clone();
    let quit_btn = menu_handles.quit_btn.clone();
    let handles_clone = menu_handles.clone();

    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                malus_de_common::ipc::IpcMessage::ActiveAppChanged { app_id, title, window_id } => {
                    let display = format_app_name(app_id.as_deref(), title.as_deref());
                    title_label.set_label(&display);
                    hide_btn.set_label(&format!("Ocultar {}          ⌘H", display));
                    quit_btn.set_label(&format!("Salir de {}        ⌘Q", display));
                    ui::global_menu::update_global_menu(&handles_clone, app_id.as_deref(), title.as_deref(), window_id);
                }
                malus_de_common::ipc::IpcMessage::ThemeChanged { is_dark, .. } => {
                    ui::theme::update_all_popovers_theme(is_dark);
                }
                _ => {}
            }
        }
    });

    std::thread::spawn(move || {
        let sock_path = malus_de_common::ipc::ipc_socket_path();
        loop {
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let reader = std::io::BufReader::new(stream);
                for line in std::io::BufRead::lines(reader) {
                    if let Ok(line) = line {
                        if let Ok(msg) = serde_json::from_str::<malus_de_common::ipc::IpcMessage>(&line) {
                            let _ = tx.send_blocking(msg);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // 2. Zona Central: Espaciador expansible
    let center_spacer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();

    // 3. Zona Derecha: Auto-Tiling + Power Mode + Tray + Clock
    let right_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(gtk4::Align::Center)
        .spacing(6)
        .build();
    right_box.append(&ui::auto_tiling::create_auto_tiling_widget());
    right_box.append(&ui::power_profile::create_power_profile_widget());
    right_box.append(&ui::status_tray::create_status_tray_widget());
    right_box.append(&ui::clock::create_clock_widget());

    root_box.append(&left_box);
    root_box.append(&center_spacer);
    root_box.append(&right_box);

    window.set_child(Some(&root_box));
    window.present();
}

fn format_app_name(app_id: Option<&str>, title: Option<&str>) -> String {
    if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.contains("firefox") || lower.contains("mozilla") {
            return "Firefox".to_string();
        } else if lower.contains("nautilus") || lower.contains("archivos") {
            return "Archivos".to_string();
        } else if lower.contains("calculator") || lower.contains("calculadora") {
            return "Calculadora".to_string();
        } else if lower.contains("ptyxis") || lower.contains("terminal") {
            return "Terminal".to_string();
        } else if lower.contains("texteditor") || lower.contains("text-editor") || lower.contains("gedit") {
            return "Editor de Texto".to_string();
        } else if lower.contains("control-center") || lower.contains("settings") {
            return "Configuración".to_string();
        }
    }

    if let Some(t) = title {
        let trimmed = t.trim();
        let lower = trimmed.to_lowercase();
        if lower.contains("firefox") {
            return "Firefox".to_string();
        }
        if !trimmed.is_empty() {
            if let Some((_, last)) = trimmed.rsplit_once(" — ") {
                return last.trim().to_string();
            }
            if let Some((_, last)) = trimmed.rsplit_once(" - ") {
                return last.trim().to_string();
            }
            return trimmed.to_string();
        }
    }

    if let Some(id) = app_id {
        if let Some(last) = id.rsplit('.').next() {
            if !last.is_empty() {
                return last.to_string();
            }
        }
    }

    "Malus DE".to_string()
}
