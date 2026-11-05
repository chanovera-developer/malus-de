// ==============================================================================
// Malus DE: Binario Ejecutable de la Barra Superior Nativa (WGPU / Layer-Shell)
// ==============================================================================

use anyhow::{Context, Result};
use calloop::channel::{channel, Channel, Event};
use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use malus_de_common::ipc::{ipc_socket_path, IpcMessage};
use malus_de_common::logging::init_logging;
use malus_de_render::dbus_menu::{DbusMenuCategory, DbusMenuService};
use malus_de_render::status_notifier::SniService;
use malus_de_render::{StatusItem, TopBarConfig, TopBarModel, WaylandTopBarApp};
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::info;
use wayland_client::globals::registry_queue_init;
use wayland_client::Connection;

/// Mensaje de actualización dinámica recibido por la barra superior.
#[derive(Debug, Clone)]
pub struct MenuUpdate {
    pub app_name: String,
    pub menus: Vec<String>,
    pub categories: Vec<DbusMenuCategory>,
}

/// Mensaje de actualización periódica de reloj y elementos de estado del sistema.
#[derive(Debug, Clone)]
pub struct StatusUpdate {
    pub clock: String,
    pub status_items: Vec<StatusItem>,
    pub is_dark: bool,
}

fn main() -> Result<()> {
    init_logging("malus_wgpu_topbar");
    info!("🍏 Iniciando Malus DE Top Bar Nativa (WGPU / Layer-Shell / D-Bus Global Menu)...");

    // 1. Conexión a la sesión Wayland activa
    let conn = Connection::connect_to_env()
        .context("No se pudo conectar a la sesión gráfica Wayland. Asegúrate de ejecutar dentro de una sesión activa (Malus DE, GNOME o compositor Wayland)")?;

    let (globals, event_queue) = registry_queue_init(&conn)
        .context("Error al inicializar la cola de eventos y registros de Wayland")?;
    let qh = event_queue.handle();

    // 2. EventLoop de Calloop
    let mut event_loop: EventLoop<WaylandTopBarApp> = EventLoop::try_new()
        .context("Error al inicializar el bucle de eventos Calloop")?;
    let loop_handle = event_loop.handle();

    // Insertar origen de eventos de Wayland en Calloop
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .context("Error al insertar WaylandSource en Calloop")?;

    // 3. Canal para recibir actualizaciones de Menú Global (D-Bus e IPC)
    let (menu_tx, menu_channel): (_, Channel<MenuUpdate>) = channel();
    loop_handle
        .insert_source(menu_channel, |event, _, app| {
            if let Event::Msg(update) = event {
                info!("🔄 [TopBar] Aplicando actualización de menú: {} ({:?})", update.app_name, update.menus);
                let _ = app.update_active_app(update.app_name, update.menus);
                app.update_dbus_categories(update.categories);
                let _ = app.render();
            }
        })
        .map_err(|e| anyhow::anyhow!("Error al insertar canal de menús en Calloop: {:?}", e))?;

    // 3b. Canal para recibir actualizaciones de estado del sistema (Reloj, Batería, WiFi, etc.)
    let (status_tx, status_channel): (_, Channel<StatusUpdate>) = channel();
    loop_handle
        .insert_source(status_channel, |event, _, app| {
            if let Event::Msg(update) = event {
                let mut changed = false;
                if app.model.clock_text != update.clock {
                    app.model.clock_text = update.clock;
                    changed = true;
                }
                if app.model.status_items != update.status_items {
                    app.model.status_items = update.status_items;
                    changed = true;
                }
                if app.model.is_dark_background != update.is_dark {
                    info!("🎨 [TopBar] Paleta adaptativa actualizada por tema/fondo: is_dark={}", update.is_dark);
                    app.model.is_dark_background = update.is_dark;
                    changed = true;
                }
                if changed {
                    let _ = app.rebuild_ui();
                    let _ = app.render();
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("Error al insertar canal de status en Calloop: {:?}", e))?;

    // 4. Iniciar servicio D-Bus com.canonical.AppMenu.Registrar y org.kde.StatusNotifierWatcher
    let (dbus_service, registrar) = DbusMenuService::new();
    let dbus_service = Arc::new(dbus_service);
    DbusMenuService::start_registrar(registrar);

    let (_sni_service, sni_watcher) = SniService::new();
    SniService::start_watcher(sni_watcher);

    // 4b. Hilo de sondeo periódico para reloj e indicadores de estado del sistema (1 Hz)
    {
        let status_tx = status_tx.clone();
        std::thread::spawn(move || {
            loop {
                let clock = current_clock_string();
                let status_items = fetch_current_status_items();
                let is_dark = malus_de_render::luminance::detect_system_is_dark();
                let _ = status_tx.send(StatusUpdate { clock, status_items, is_dark });
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    // 5. Estado compartido de la ventana activa actualmente enfocada
    let active_window_state = Arc::new(Mutex::new((
        Option::<String>::None, // app_id
        Option::<String>::None, // title
        Option::<u64>::None,    // window_id
    )));

    // 6. Hilo listener para registros D-Bus dinámicos entrantes
    {
        let dbus_service = dbus_service.clone();
        let active_state = active_window_state.clone();
        let menu_tx = menu_tx.clone();
        std::thread::spawn(move || {
            let rx = dbus_service.update_rx.clone();
            while let Ok(reg) = rx.recv_blocking() {
                let (cur_app, cur_title, cur_win) = {
                    let g = active_state.lock().unwrap();
                    (g.0.clone(), g.1.clone(), g.2)
                };

                let should_update = match cur_win {
                    Some(wid) => (wid as u32) == reg.window_id,
                    None => true,
                };

                if should_update {
                    let display_name = format_app_name(cur_app.as_deref(), cur_title.as_deref());
                    let srv = reg.service.clone();
                    let pth = reg.path.clone();

                    let tx = menu_tx.clone();
                    std::thread::spawn(move || {
                        if let Ok(categories) = futures_lite::future::block_on(DbusMenuService::fetch_layout(&srv, &pth)) {
                            let titles: Vec<String> = categories.iter().map(|c| c.title.clone()).collect();
                            let _ = tx.send(MenuUpdate {
                                app_name: display_name,
                                menus: titles,
                                categories,
                            });
                        }
                    });
                }
            }
        });
    }

    // 7. Hilo listener de eventos IPC del Compositor Malus DE
    {
        let dbus_service = dbus_service.clone();
        let active_state = active_window_state.clone();
        let menu_tx = menu_tx.clone();
        let status_tx = status_tx.clone();

        std::thread::spawn(move || {
            let sock_path = ipc_socket_path();
            // Reintentar conexión al socket IPC del compositor
            loop {
                if let Ok(stream) = UnixStream::connect(&sock_path) {
                    info!("🔌 [TopBar] Conectado al socket IPC del compositor: {:?}", sock_path);
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                            match msg {
                                IpcMessage::ActiveAppChanged { app_id, title, window_id } => {
                                    {
                                        let mut g = active_state.lock().unwrap();
                                        *g = (app_id.clone(), title.clone(), window_id);
                                    }

                                    let display_name = format_app_name(app_id.as_deref(), title.as_deref());

                                    // Buscar si hay menú D-Bus para esta ventana
                                    if let Some((srv, pth)) = dbus_service.find_menu(window_id, app_id.as_deref()) {
                                        let tx = menu_tx.clone();
                                        let d_name = display_name.clone();
                                        std::thread::spawn(move || {
                                            if let Ok(categories) = futures_lite::future::block_on(DbusMenuService::fetch_layout(&srv, &pth)) {
                                                let titles: Vec<String> = categories.iter().map(|c| c.title.clone()).collect();
                                                let _ = tx.send(MenuUpdate {
                                                    app_name: d_name,
                                                    menus: titles,
                                                    categories,
                                                });
                                                return;
                                            }
                                            let _ = tx.send(MenuUpdate {
                                                app_name: d_name,
                                                menus: default_menus_for_app(&app_id),
                                                categories: Vec::new(),
                                            });
                                        });
                                    } else {
                                        let _ = menu_tx.send(MenuUpdate {
                                            app_name: display_name,
                                            menus: default_menus_for_app(&app_id),
                                            categories: Vec::new(),
                                        });
                                    }
                                }
                                IpcMessage::ThemeChanged { is_dark, .. } => {
                                    info!("🎨 [TopBar] IPC ThemeChanged recibido: is_dark={}", is_dark);
                                    let clock = current_clock_string();
                                    let status_items = fetch_current_status_items();
                                    let _ = status_tx.send(StatusUpdate { clock, status_items, is_dark });
                                }
                                _ => {}
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    // 8. Crear la aplicación de barra superior (100% transparente y adaptativa)
    let config = TopBarConfig {
        height: 32.0,
        is_floating_pill: false,
        margin_x: 0.0,
        margin_top: 0.0,
        bg_color: [0.0, 0.0, 0.0, 0.0],
        border_color: [0.0, 0.0, 0.0, 0.0],
        border_width: 0.0,
    };
    let model = TopBarModel {
        app_name: "Finder".to_string(),
        menus: default_menus_for_app(&None),
        status_items: fetch_current_status_items(),
        clock_text: current_clock_string(),
        is_dark_background: malus_de_render::luminance::detect_system_is_dark(),
        active_popover: None,
    };

    info!("Inicializando superficie layer-shell y contexto WGPU...");
    let mut app = WaylandTopBarApp::new(&conn, &qh, &globals, config, model)
        .context("Error al inicializar WaylandTopBarApp")?;

    info!("✨ Barra superior nativa activa y escuchando eventos Wayland / D-Bus");

    // 9. Bucle principal impulsado por Calloop
    while !app.exit {
        event_loop
            .dispatch(None, &mut app)
            .context("Error durante el despacho de eventos de Calloop")?;
    }

    info!("Finalizando Malus DE Top Bar limpiamente");
    Ok(())
}

fn format_app_name(app_id: Option<&str>, title: Option<&str>) -> String {
    if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.contains("firefox") || lower.contains("mozilla") {
            return "Firefox".to_string();
        } else if lower.contains("brave") {
            return "Brave".to_string();
        } else if lower.contains("nautilus") || lower.contains("archivos") {
            return "Archivos".to_string();
        } else if lower.contains("code") || lower.contains("vscode") {
            return "Visual Studio Code".to_string();
        } else if lower.contains("terminal") || lower.contains("ptyxis") {
            return "Terminal".to_string();
        } else if lower.contains("calculator") || lower.contains("calculadora") {
            return "Calculadora".to_string();
        } else if lower.contains("settings") || lower.contains("control-center") {
            return "Configuración".to_string();
        }
    }

    if let Some(t) = title {
        let trimmed = t.trim();
        if !trimmed.is_empty() && trimmed != "Finder" {
            // Si el título contiene un guión (ej: "Documento - Editor"), tomar la app
            if let Some((_, app_part)) = trimmed.rsplit_once(" — ") {
                return app_part.trim().to_string();
            }
            if let Some((_, app_part)) = trimmed.rsplit_once(" - ") {
                return app_part.trim().to_string();
            }
            return trimmed.to_string();
        }
    }

    "Finder".to_string()
}

fn default_menus_for_app(app_id: &Option<String>) -> Vec<String> {
    match app_id.as_deref().map(str::to_lowercase) {
        Some(ref id) if id.contains("terminal") || id.contains("ptyxis") => vec![
            "Terminal".to_string(),
            "Shell".to_string(),
            "Edición".to_string(),
            "Ver".to_string(),
            "Ventana".to_string(),
            "Ayuda".to_string(),
        ],
        Some(ref id) if id.contains("nautilus") || id.contains("archivos") => vec![
            "Archivo".to_string(),
            "Edición".to_string(),
            "Visualización".to_string(),
            "Ir".to_string(),
            "Ventana".to_string(),
            "Ayuda".to_string(),
        ],
        _ => vec![
            "Archivo".to_string(),
            "Edición".to_string(),
            "Visualización".to_string(),
            "Ventana".to_string(),
            "Ayuda".to_string(),
        ],
    }
}

fn current_clock_string() -> String {
    // Formato estilo macOS en español (ej. "Jue 5 Nov  00:23")
    if let Ok(out) = std::process::Command::new("date")
        .arg("+%a %-d %b  %H:%M")
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            let mut chars = s.chars();
            if let Some(first) = chars.next() {
                return first.to_uppercase().collect::<String>() + chars.as_str();
            }
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (now / 3600) % 24;
    let minutes = (now / 60) % 60;
    format!("{:02}:{:02}", hours, minutes)
}

fn fetch_current_status_items() -> Vec<StatusItem> {
    let mut items = Vec::new();

    // 0. Bandeja del sistema (expansor estilo Bartender)
    items.push(StatusItem::new("status:tray", "> +4").with_size(11.5).bold());

    // 1. Selector rápido de distribución de teclado (ES, LAT, US)
    let kb = malus_de_common::config::InputConfig::load().layout_short_badge();
    items.push(StatusItem::new("status:keyboard", kb).with_size(11.0).bold());

    // 2. Wi-Fi (Vectorial Font Awesome 6 Free Solid)
    let _wifi_up = std::fs::read_dir("/sys/class/net")
        .ok()
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("wl") {
                    let operstate_path = e.path().join("operstate");
                    std::fs::read_to_string(operstate_path)
                        .map(|s| s.trim() == "up")
                        .unwrap_or(false)
                } else {
                    false
                }
            })
        })
        .unwrap_or(false);

    items.push(StatusItem::new("status:wifi", "\u{F1EB}").with_size(13.0));

    // 3. Salida de audio / Volumen
    items.push(StatusItem::new("status:volume", "\u{F028}").with_size(13.0));

    // 4. Batería (Dispositivos HID o equipo portátil)
    let battery_pct = std::fs::read_dir("/sys/class/power_supply")
        .ok()
        .and_then(|entries| {
            for entry in entries.flatten() {
                let cap_path = entry.path().join("capacity");
                if let Ok(s) = std::fs::read_to_string(cap_path) {
                    if let Ok(val) = s.trim().parse::<u32>() {
                        return Some(val);
                    }
                }
            }
            None
        });

    if let Some(pct) = battery_pct {
        items.push(StatusItem::new("status:battery", format!("\u{F240} {}%", pct)).with_size(12.0));
    } else {
        items.push(StatusItem::new("status:battery", "\u{F240}").with_size(13.0));
    }

    // 5. Spotlight Search (Lupa macOS)
    items.push(StatusItem::new("status:spotlight", "\u{F002}").with_size(12.0));

    items
}
