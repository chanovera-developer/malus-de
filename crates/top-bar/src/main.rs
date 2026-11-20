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

    // Inicializar entorno aislado de Malus DE y limpiar variables residuales
    let _ = malus_de_common::config::setup_isolated_session_environment();

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

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
        .application_id("org.malus_de.TopBar")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_ui);

    app.run();
}

thread_local! {
    static TOPBAR_GLASS_PROVIDER: std::cell::RefCell<Option<CssProvider>> =
        const { std::cell::RefCell::new(None) };
}

fn apply_topbar_liquid_glass_opacity(opacity: f64) {
    let scale = malus_de_common::config::liquid_glass_scale_for(opacity);
    let dark_pop = malus_de_common::config::scaled_liquid_glass_alpha(0.78, scale, 0.14, 0.96);
    let dark_cc = malus_de_common::config::scaled_liquid_glass_alpha(0.82, scale, 0.16, 0.96);
    let light_pop = malus_de_common::config::scaled_liquid_glass_alpha(0.76, scale, 0.14, 0.96);
    let light_cc = malus_de_common::config::scaled_liquid_glass_alpha(0.78, scale, 0.14, 0.96);
    let css = format!(
        "popover.malus-popover-menu>contents {{ background-color: rgba(30, 30, 36, {dark_pop:.3}); }} \
         popover.control-center-popover>contents {{ background-color: rgba(30, 30, 36, {dark_cc:.3}); }} \
         popover.light-popover.malus-popover-menu>contents, \
         popover.light-popover>contents, \
         .light-mode popover.malus-popover-menu>contents {{ background-color: rgba(244, 246, 250, {light_pop:.3}); }} \
         popover.light-popover.control-center-popover>contents, \
         .light-mode popover.control-center-popover>contents {{ background-color: rgba(244, 246, 250, {light_cc:.3}); }}"
    );
    TOPBAR_GLASS_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(STYLE_CSS);
    let glass_provider = CssProvider::new();

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );
        gtk4::style_context_add_provider_for_display(
            &display,
            &glass_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        let icon_theme = gtk4::IconTheme::for_display(&display);
        for icon_dir in malus_de_common::apps::extra_icon_search_paths() {
            if icon_dir.is_dir() {
                icon_theme.add_search_path(&icon_dir);
            }
        }
        icon_theme.add_search_path("/usr/local/share/malus-de/icons");
        icon_theme.add_search_path("/usr/share/malus-de/icons");
        if let Ok(home) = std::env::var("HOME") {
            let config_icons = std::path::PathBuf::from(&home).join(".config/malus-de/icons");
            let local_icons = std::path::PathBuf::from(&home).join(".local/share/icons");
            icon_theme.add_search_path(&config_icons);
            icon_theme.add_search_path(&local_icons);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                icon_theme.add_search_path(parent.join("assets/icons"));
                icon_theme.add_search_path(parent.join("../../assets/icons"));
                icon_theme.add_search_path(parent.join("../../data/icons"));
            }
        }
    }

    TOPBAR_GLASS_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(glass_provider);
    });
    apply_topbar_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());
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
        if let Some(display) = gtk4::gdk::Display::default() {
            let displays_cfg = malus_de_common::config::DisplaysConfig::load();
            if let Some(primary_mon) = find_primary_gdk_monitor(&display, &displays_cfg) {
                window.set_monitor(&primary_mon);
            }
        }
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

    adw::StyleManager::default().connect_dark_notify(|sm| {
        let is_dark = sm.is_dark();
        ui::theme::update_all_popovers_theme(is_dark);
    });

    let win_ipc = window.clone();
    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                malus_de_common::ipc::IpcMessage::ActiveAppChanged { app_id, title, window_id } => {
                    let display = format_app_name(app_id.as_deref(), title.as_deref());
                    title_label.set_label(&display);
                    let es = std::env::var("LC_ALL")
                        .or_else(|_| std::env::var("LC_MESSAGES"))
                        .or_else(|_| std::env::var("LANG"))
                        .unwrap_or_default()
                        .to_lowercase()
                        .starts_with("es");
                    let hide_txt = if es { format!("Ocultar {}", display) } else { format!("Hide {}", display) };
                    let quit_txt = if es { format!("Salir de {}", display) } else { format!("Quit {}", display) };
                    ui::global_menu::update_menu_row_label(&hide_btn, &hide_txt);
                    ui::global_menu::update_menu_row_label(&quit_btn, &quit_txt);
                    ui::global_menu::update_global_menu(&handles_clone, app_id.as_deref(), title.as_deref(), window_id);
                }
                malus_de_common::ipc::IpcMessage::ThemeChanged { is_dark, .. } => {
                    adw::StyleManager::default().set_color_scheme(if is_dark {
                        adw::ColorScheme::ForceDark
                    } else {
                        adw::ColorScheme::ForceLight
                    });
                    ui::theme::update_all_popovers_theme(is_dark);
                }
                malus_de_common::ipc::IpcMessage::LiquidGlassOpacityChanged { opacity } => {
                    info!("✨ [TopBar] IPC LiquidGlassOpacityChanged recibido: opacity={:.2}", opacity);
                    apply_topbar_liquid_glass_opacity(opacity);
                    win_ipc.queue_draw();
                }
                malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig => {
                    info!("🖥 [TopBar] IPC ReloadDisplaysConfig recibido — reajustando top bar");
                    win_ipc.queue_resize();
                    win_ipc.queue_draw();
                }
                _ => {}
            }
        }
    });

    if let Some(display) = gtk4::gdk::Display::default() {
        let win_mon = window.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            win_mon.queue_resize();
            win_mon.queue_draw();
        });
    }

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

fn find_primary_gdk_monitor(
    display: &gtk4::gdk::Display,
    cfg: &malus_de_common::config::DisplaysConfig,
) -> Option<gtk4::gdk::Monitor> {
    let monitors = display.monitors();
    let monitor_list: Vec<gtk4::gdk::Monitor> = (0..monitors.n_items())
        .filter_map(|i| monitors.item(i))
        .filter_map(|m| m.downcast::<gtk4::gdk::Monitor>().ok())
        .collect();
    if let Some(primary_cfg) = cfg.primary_output() {
        if let Some(matched) = monitor_list.iter().find(|monitor| {
            let geo = monitor.geometry();
            primary_cfg.matches_monitor_identity(
                monitor.connector().as_deref(),
                monitor.model().as_deref(),
                monitor.description().as_deref(),
                geo.x(),
                geo.y(),
            )
        }) {
            return Some(matched.clone());
        }
    }
    monitor_list.into_iter().next()
}

fn format_app_name(app_id: Option<&str>, title: Option<&str>) -> String {
    let es = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es");

    if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.contains("firefox") || lower.contains("mozilla") {
            return "Firefox".to_string();
        } else if lower.contains("libreoffice") || lower.contains("soffice") {
            if lower.contains("writer") {
                return "LibreOffice Writer".to_string();
            } else if lower.contains("calc") {
                return "LibreOffice Calc".to_string();
            } else if lower.contains("impress") {
                return "LibreOffice Impress".to_string();
            } else {
                return "LibreOffice".to_string();
            }
        } else if lower.contains("nautilus") || lower.contains("archivos") {
            return if es { "Archivos" } else { "Files" }.to_string();
        } else if lower.contains("calculator") || lower.contains("calculadora") {
            return if es { "Calculadora" } else { "Calculator" }.to_string();
        } else if lower.contains("ptyxis") || lower.contains("terminal") {
            return "Terminal".to_string();
        } else if lower.contains("texteditor") || lower.contains("text-editor") || lower.contains("gedit") {
            return if es { "Editor de Texto" } else { "Text Editor" }.to_string();
        } else if lower.contains("control-center") || lower.contains("settings") {
            return if es { "Configuración" } else { "Settings" }.to_string();
        }
    }

    if let Some(t) = title {
        let trimmed = t.trim();
        let lower = trimmed.to_lowercase();
        if lower.contains("firefox") {
            return "Firefox".to_string();
        } else if lower.contains("libreoffice") || lower.contains("writer") {
            return "LibreOffice Writer".to_string();
        } else if lower.contains("calc") {
            return "LibreOffice Calc".to_string();
        } else if lower.contains("impress") {
            return "LibreOffice Impress".to_string();
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
