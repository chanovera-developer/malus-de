// ==============================================================================
// Malus DE: Binario Ejecutable de la Barra Superior Nativa (WGPU / Layer-Shell)
// ==============================================================================

use anyhow::{Context, Result};
use malus_de_common::logging::init_logging;
use malus_de_render::{TopBarConfig, TopBarModel, WaylandTopBarApp};
use tracing::info;
use wayland_client::globals::registry_queue_init;
use wayland_client::Connection;

fn main() -> Result<()> {
    init_logging("malus_wgpu_topbar");
    info!("🍏 Iniciando Malus DE Top Bar Nativa (WGPU / Layer-Shell)...");

    // Conectar al socket de sesión Wayland activo
    let conn = Connection::connect_to_env()
        .context("No se pudo conectar a la sesión gráfica Wayland. Asegúrate de ejecutar dentro de una sesión activa (Malus DE, GNOME o compositor Wayland)")?;

    let (globals, mut event_queue) = registry_queue_init(&conn)
        .context("Error al inicializar la cola de eventos y registros de Wayland")?;
    let qh = event_queue.handle();

    let config = TopBarConfig {
        height: 32.0,
        is_floating_pill: true,
        margin_x: 16.0,
        margin_top: 6.0,
        bg_color: [0.12, 0.12, 0.15, 0.88],
        border_color: [1.0, 1.0, 1.0, 0.20],
        border_width: 1.0,
    };
    let model = TopBarModel::default();

    info!("Inicializando superficie layer-shell y contexto WGPU...");
    let mut app = WaylandTopBarApp::new(&conn, &qh, &globals, config, model)
        .context("Error al inicializar WaylandTopBarApp")?;

    info!("✨ Barra superior activa y escuchando eventos Wayland");

    // Bucle principal de eventos Wayland
    while !app.exit {
        event_queue
            .blocking_dispatch(&mut app)
            .context("Error durante el despacho de eventos de Wayland")?;
    }

    info!("Finalizando Malus DE Top Bar limpiamente");
    Ok(())
}
