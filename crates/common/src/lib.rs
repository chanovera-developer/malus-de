//! Biblioteca compartida para Malus DE (`malus-de-common`)
//!
//! Contiene estructuras de configuración serializables, tipos de mensajes IPC
//! para sockets Unix, gestión de temas y configuración de logging estructurado.

pub mod apps;
pub mod config;
pub mod crash;
#[cfg(feature = "gtk-ui")]
pub mod folder_icon;
pub mod ipc;
pub mod logging;
#[cfg(feature = "gtk-ui")]
pub mod mac_switch;
#[cfg(feature = "gtk-ui")]
pub mod sidebar_style;
pub mod theme;
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
    write_drag_anchors(&[(path.to_path_buf(), (offset_x, offset_y))]);
}

/// Guarda temporalmente los puntos de agarre de uno o varios elementos arrastrados simultáneamente,
/// preservando sus desplazamientos relativos entre procesos (Branches <-> Escritorio).
pub fn write_drag_anchors(anchors: &[(std::path::PathBuf, (f64, f64))]) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut payload = format!("v2:{}\n", now_ms);
    for (path, (ox, oy)) in anchors {
        payload.push_str(&format!("{},{}\t{}\n", ox, oy, path.to_string_lossy()));
    }
    for candidate in drag_anchor_candidate_paths() {
        if std::fs::write(&candidate, &payload).is_ok() {
            break;
        }
    }
}

fn paths_match(stored_path: &str, expected_path: &std::path::Path) -> bool {
    if std::path::Path::new(stored_path) == expected_path {
        return true;
    }
    let can_stored = std::fs::canonicalize(stored_path).ok();
    let can_expected = std::fs::canonicalize(expected_path).ok();
    can_stored.is_some() && can_stored == can_expected
}

/// Lee el punto de agarre compartido si corresponde a `expected_path` y fue registrado recientemente (< 60 s).
pub fn read_drag_anchor(expected_path: &std::path::Path) -> Option<(f64, f64)> {
    let raw = drag_anchor_candidate_paths()
        .into_iter()
        .filter_map(|candidate| std::fs::read_to_string(candidate).ok())
        .next()?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    if let Some(rest) = raw.strip_prefix("v2:") {
        let mut lines = rest.lines();
        let ts_ms: u128 = lines.next()?.trim().parse().ok()?;
        if now_ms.saturating_sub(ts_ms) > 60_000 {
            return None;
        }
        for line in lines {
            let (coords_part, stored_path) = line.split_once('\t')?;
            if paths_match(stored_path.trim(), expected_path) {
                let mut xy = coords_part.split(',');
                let ox: f64 = xy.next()?.trim().parse().ok()?;
                let oy: f64 = xy.next()?.trim().parse().ok()?;
                return Some((ox, oy));
            }
        }
        return None;
    }

    let mut lines = raw.splitn(3, '\n');
    let ts_ms: u128 = lines.next()?.trim().parse().ok()?;
    let coords_line = lines.next()?;
    let stored_path = lines.next()?.trim();

    if now_ms.saturating_sub(ts_ms) > 60_000 {
        return None;
    }
    if !paths_match(stored_path, expected_path) {
        return None;
    }

    let mut xy = coords_line.split(',');
    let ox: f64 = xy.next()?.trim().parse().ok()?;
    let oy: f64 = xy.next()?.trim().parse().ok()?;
    Some((ox, oy))
}

/// Formatea la fecha y hora local al estilo macOS (p. ej. `"Jue 5 Nov  00:23"`)
/// directamente en proceso vía `libc::localtime_r` + `libc::strftime`, sin lanzar `date`.
pub fn format_macos_clock_now() -> String {
    let mut now: libc::time_t = 0;
    unsafe {
        libc::time(&mut now);
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&now, &mut tm).is_null() };
    if ok {
        let mut buf = [0u8; 64];
        let fmt = c"%a %-d %b  %H:%M";
        let len = unsafe {
            libc::strftime(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                fmt.as_ptr(),
                &tm,
            )
        };
        if len > 0 {
            let s = String::from_utf8_lossy(&buf[..len]).trim().to_string();
            let mut chars = s.chars();
            if let Some(first) = chars.next() {
                return first.to_uppercase().collect::<String>() + chars.as_str();
            }
        }
        return format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_drag_anchor_roundtrips_single_and_multi_paths() {
        let p1 = std::path::PathBuf::from("/tmp/malus_drag_anchor_test_1.txt");
        let p2 = std::path::PathBuf::from("/tmp/malus_drag_anchor_test_2.txt");
        write_drag_anchors(&[(p1.clone(), (38.0, 44.0)), (p2.clone(), (-78.0, -56.0))]);
        assert_eq!(read_drag_anchor(&p1), Some((38.0, 44.0)));
        assert_eq!(read_drag_anchor(&p2), Some((-78.0, -56.0)));
        assert_eq!(
            read_drag_anchor(std::path::Path::new("/tmp/other_file.txt")),
            None
        );
    }
}
