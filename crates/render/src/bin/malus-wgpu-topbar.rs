// ==============================================================================
// Malus DE: Binario Ejecutable de la Barra Superior Nativa (WGPU / Layer-Shell)
// ==============================================================================

use anyhow::{Context, Result};
use calloop::channel::{channel, Channel, Event};
use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use malus_de_common::ipc::{ipc_socket_path, IpcMessage};
use malus_de_common::logging::init_logging;
use malus_de_render::dbus_menu::DbusMenuService;
use malus_de_render::{TopBarConfig, TopBarModel, WaylandTopBarApp};
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
                let _ = app.render();
            }
        })
        .map_err(|e| anyhow::anyhow!("Error al insertar canal de menús en Calloop: {:?}", e))?;

    // 4. Iniciar servicio D-Bus com.canonical.AppMenu.Registrar
    let (dbus_service, registrar) = DbusMenuService::new();
    let dbus_service = Arc::new(dbus_service);
    DbusMenuService::start_registrar(registrar);

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
                            let titles: Vec<String> = categories.into_iter().map(|c| c.title).collect();
                            let _ = tx.send(MenuUpdate {
                                app_name: display_name,
                                menus: titles,
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

        std::thread::spawn(move || {
            let sock_path = ipc_socket_path();
            // Reintentar conexión al socket IPC del compositor
            loop {
                if let Ok(stream) = UnixStream::connect(&sock_path) {
                    info!("🔌 [TopBar] Conectado al socket IPC del compositor: {:?}", sock_path);
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if let Ok(IpcMessage::ActiveAppChanged { app_id, title, window_id }) =
                            serde_json::from_str::<IpcMessage>(&line)
                        {
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
                                        let titles: Vec<String> = categories.into_iter().map(|c| c.title).collect();
                                        let _ = tx.send(MenuUpdate {
                                            app_name: d_name,
                                            menus: titles,
                                        });
                                        return;
                                    }
                                    let _ = tx.send(MenuUpdate {
                                        app_name: d_name,
                                        menus: default_menus_for_app(&app_id),
                                    });
                                });
                            } else {
                                let _ = menu_tx.send(MenuUpdate {
                                    app_name: display_name,
                                    menus: default_menus_for_app(&app_id),
                                });
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    // 8. Crear la aplicación de barra superior
    let config = TopBarConfig {
        height: 32.0,
        is_floating_pill: true,
        margin_x: 16.0,
        margin_top: 6.0,
        bg_color: [0.12, 0.12, 0.15, 0.88],
        border_color: [1.0, 1.0, 1.0, 0.20],
        border_width: 1.0,
    };
    let model = TopBarModel {
        app_name: "Finder".to_string(),
        menus: default_menus_for_app(&None),
        status_items: vec!["WiFi".to_string(), "100%".to_string()],
        clock_text: current_clock_string(),
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
    // Formato breve estilo macOS (ej. "Mié 4 Nov 21:55")
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (now / 3600) % 24;
    let minutes = (now / 60) % 60;
    format!("{:02}:{:02}", hours, minutes)
}
