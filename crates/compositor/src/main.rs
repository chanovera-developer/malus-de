pub mod backend;
pub mod blur;
pub mod capture;
mod cpu_priority;
pub mod grabs;
pub mod shadow;
pub mod state;
pub mod theme_transition;
pub mod window_corners;

use malus_de_common::logging::init_logging;
use std::process::{Child, Command};
use tracing::info;

pub struct ClipboardManagerProcess(Child);

impl Drop for ClipboardManagerProcess {
    fn drop(&mut self) {
        if let Err(error) = self.0.kill() {
            tracing::debug!("No se pudo detener el gestor de portapapeles: {error}");
        }
        if let Err(error) = self.0.wait() {
            tracing::warn!("No se pudo esperar al gestor de portapapeles: {error}");
        }
    }
}

pub fn start_clipboard_manager(socket_name: &str) -> Option<ClipboardManagerProcess> {
    let local_binary = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("malus-de-clipboard")))
        .filter(|path| path.is_file());
    let mut command = local_binary
        .map(Command::new)
        .unwrap_or_else(|| Command::new("malus-de-clipboard"));
    command.env("WAYLAND_DISPLAY", socket_name);
    match command.spawn() {
        Ok(child) => {
            info!("Gestor de portapapeles iniciado (PID {})", child.id());
            Some(ClipboardManagerProcess(child))
        }
        Err(error) => {
            tracing::warn!("No se pudo iniciar malus-de-clipboard: {error}");
            None
        }
    }
}

fn main() -> anyhow::Result<()> {
    init_logging("malus_de_compositor");
    info!("Iniciando Malus DE Compositor (Smithay Wayland)...");

    // Limpiar sockets huérfanos de sesiones Wayland previas antes de inicializar backends
    malus_de_common::wayland::cleanup_stale_wayland_sockets();

    if let Err(e) = malus_de_common::config::setup_isolated_session_environment() {
        tracing::warn!("Aviso al inicializar entorno aislado de configuración: {:?}", e);
    }

    let args: Vec<String> = std::env::args().collect();
    let default_backend = if std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok() {
        "winit"
    } else {
        "drm"
    };

    let backend = args.iter().position(|r| r == "--backend")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or(default_backend);

    info!("Modo de backend seleccionado: {}", backend);

    match backend {
        "winit" => {
            info!("Iniciando backend Winit para pruebas en ventana anidada...");
            crate::backend::winit::run_winit()?;
        }
        "drm" => {
            info!("Iniciando backend DRM/KMS nativo con soporte libinput...");
            crate::backend::drm::run_drm()?;
        }
        other => {
            eprintln!("Backend no reconocido: {other}. Usa 'winit' o 'drm'.");
            std::process::exit(1);
        }
    }

    Ok(())
}
