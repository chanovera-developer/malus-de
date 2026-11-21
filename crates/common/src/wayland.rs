use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Obtiene la ruta del directorio de ejecución de usuario (XDG_RUNTIME_DIR)
pub fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/run/user/1000")
    }
}

/// Espera de forma activa y robusta a que el compositor Wayland esté realmente
/// listo y aceptando conexiones en su socket UNIX.
pub fn wait_for_wayland(timeout_secs: u64) -> bool {
    let r_dir = runtime_dir();
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
    let socket_path = r_dir.join(&display);

    let start = Instant::now();
    let max_dur = Duration::from_secs(timeout_secs);

    tracing::info!(
        "⏳ Esperando a que el compositor Wayland esté listo en {:?}...",
        socket_path
    );

    while start.elapsed() < max_dur {
        if let Ok(_stream) = UnixStream::connect(&socket_path) {
            // El compositor está escuchando y ha aceptado la conexión de prueba.
            std::thread::sleep(Duration::from_millis(50));
            tracing::info!(
                "✅ Compositor Wayland activo y aceptando conexiones en {:?}",
                socket_path
            );
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    tracing::warn!(
        "⚠️ Tiempo de espera agotado esperando socket Wayland {:?}",
        socket_path
    );
    false
}

/// Limpia sockets y cerraduras de Wayland huérfanos de sesiones anteriores que hayan
/// terminado abruptamente, evitando falsos positivos al iniciar una nueva sesión.
pub fn cleanup_stale_wayland_sockets() {
    let r_dir = runtime_dir();
    let path = Path::new(&r_dir);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if filename.starts_with("wayland-") && !filename.ends_with(".lock") {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_socket() {
                        let socket_path = entry.path();
                        // Si no se puede conectar al socket, es un socket huérfano muerto
                        if UnixStream::connect(&socket_path).is_err() {
                            tracing::info!(
                                "🧹 Eliminando socket Wayland huérfano: {:?}",
                                socket_path
                            );
                            let _ = std::fs::remove_file(&socket_path);
                            let lock_path = socket_path.with_extension("lock");
                            if lock_path.exists() {
                                let _ = std::fs::remove_file(&lock_path);
                            }
                        }
                    }
                }
            }
        }
    }
}
