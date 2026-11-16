//! Biblioteca compartida para Malus DE (`malus-de-common`)
//!
//! Contiene estructuras de configuración serializables, tipos de mensajes IPC
//! para sockets Unix, gestión de temas y configuración de logging estructurado.

pub mod apps;
pub mod config;
pub mod crash;
pub mod ipc;
pub mod theme;
pub mod logging;
pub mod wayland;

/// Detecta si el locale activo del sistema corresponde a idioma español (`es_*`).
pub fn is_spanish_locale() -> bool {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es")
}

fn drag_anchor_candidate_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::with_capacity(2);
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        paths.push(std::path::PathBuf::from(runtime_dir).join("malus-de-drag-anchor.txt"));
    }
    let tmp_path = std::env::temp_dir().join("malus-de-drag-anchor.txt");
    if !paths.contains(&tmp_path) {
        paths.push(tmp_path);
    }
    paths
}

/// Guarda temporalmente el punto de agarre del arrastre actual para alinear con precisión
/// sueltas entre procesos (p. ej. Branches <-> Escritorio).
pub fn write_drag_anchor(path: &std::path::Path, offset_x: f64, offset_y: f64) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let payload = format!("{}\n{},{}\n{}", now_ms, offset_x, offset_y, path.to_string_lossy());
    for candidate in drag_anchor_candidate_paths() {
        if std::fs::write(&candidate, &payload).is_ok() {
            break;
        }
    }
}

/// Lee el punto de agarre compartido si corresponde a `expected_path` y fue registrado recientemente (< 60 s).
pub fn read_drag_anchor(expected_path: &std::path::Path) -> Option<(f64, f64)> {
    let raw = drag_anchor_candidate_paths()
        .into_iter()
        .filter_map(|candidate| std::fs::read_to_string(candidate).ok())
        .next()?;
    let mut lines = raw.splitn(3, '\n');
    let ts_ms: u128 = lines.next()?.trim().parse().ok()?;
    let coords_line = lines.next()?;
    let stored_path = lines.next()?.trim();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if now_ms.saturating_sub(ts_ms) > 60_000 {
        return None;
    }

    if std::path::Path::new(stored_path) != expected_path {
        let can_stored = std::fs::canonicalize(stored_path).ok();
        let can_expected = std::fs::canonicalize(expected_path).ok();
        if can_stored.is_none() || can_stored != can_expected {
            return None;
        }
    }

    let mut xy = coords_line.split(',');
    let ox: f64 = xy.next()?.trim().parse().ok()?;
    let oy: f64 = xy.next()?.trim().parse().ok()?;
    Some((ox, oy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_drag_anchor_roundtrips_matching_path() {
        let p = std::path::Path::new("/tmp/malus_drag_anchor_test.txt");
        write_drag_anchor(p, 38.0, 44.0);
        assert_eq!(read_drag_anchor(p), Some((38.0, 44.0)));
        assert_eq!(
            read_drag_anchor(std::path::Path::new("/tmp/other_file.txt")),
            None
        );
    }
}

