use crate::state::{SortBy, SortOrder};
use gio::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileItem {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_hidden: bool,
    #[allow(dead_code)]
    pub is_symlink: bool,
    pub size: u64,
    pub size_str: String,
    pub modified: Option<SystemTime>,
    pub modified_str: String,
    pub kind_str: String,
    pub gicon: gio::Icon,
}

impl FileItem {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_string();
        let is_hidden = name.starts_with('.');
        
        let metadata = fs::symlink_metadata(path).ok()?;
        let is_symlink = metadata.file_type().is_symlink();
        
        // Si es symlink, intentar resolver el metadata final para saber si apunta a dir
        let target_metadata = if is_symlink {
            fs::metadata(path).ok().unwrap_or_else(|| metadata.clone())
        } else {
            metadata.clone()
        };

        let is_dir = target_metadata.is_dir();
        let size = if is_dir { 0 } else { target_metadata.len() };
        let size_str = if is_dir {
            "--".to_string()
        } else {
            format_size(size)
        };

        let modified = target_metadata.modified().ok();
        let modified_str = modified.map(format_modified).unwrap_or_else(|| "--".to_string());

        let (kind_str, gicon) = resolve_kind_and_icon(path, is_dir, &name);

        Some(Self {
            name,
            path: path.to_path_buf(),
            is_dir,
            is_hidden,
            is_symlink,
            size,
            size_str,
            modified,
            modified_str,
            kind_str,
            gicon,
        })
    }
}

pub fn load_directory(
    path: &Path,
    show_hidden: bool,
    sort_by: SortBy,
    sort_order: SortOrder,
    filter_query: &str,
) -> Result<Vec<FileItem>, std::io::Error> {
    let mut items = Vec::new();

    let entries = fs::read_dir(path)?;
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(item) = FileItem::from_path(&p) {
            if !show_hidden && item.is_hidden {
                continue;
            }
            if !filter_query.is_empty()
                && !item.name.to_lowercase().contains(&filter_query.to_lowercase())
            {
                continue;
            }
            items.push(item);
        }
    }

    // Ordenar elementos: siempre carpetas primero salvo requerimiento especial
    items.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            return b.is_dir.cmp(&a.is_dir); // Carpetas primero
        }

        let ordering = match sort_by {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::DateModified => {
                let a_time = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
                let b_time = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
                a_time.cmp(&b_time)
            }
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Kind => a.kind_str.cmp(&b.kind_str).then(a.name.cmp(&b.name)),
        };

        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
    });

    Ok(items)
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

pub fn format_modified(time: SystemTime) -> String {
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;

    let dt = glib::DateTime::from_unix_local(secs).unwrap_or_else(|_| glib::DateTime::now_local().unwrap());
    let now = glib::DateTime::now_local().unwrap();

    let diff_days = (now.to_unix() - dt.to_unix()) / 86400;

    if diff_days == 0 && now.day_of_month() == dt.day_of_month() {
        format!("Hoy a las {}", dt.format("%H:%M").unwrap_or_default())
    } else if diff_days <= 1 {
        format!("Ayer a las {}", dt.format("%H:%M").unwrap_or_default())
    } else if diff_days < 7 {
        format!("{}", dt.format("%a %H:%M").unwrap_or_default())
    } else {
        format!("{}", dt.format("%d/%m/%Y %H:%M").unwrap_or_default())
    }
}

pub fn get_free_space(path: &Path) -> String {
    // Usar GIO File para consultar espacio libre del sistema de archivos de forma no bloqueante
    let gfile = gio::File::for_path(path);
    if let Ok(info) = gfile.query_filesystem_info("filesystem::free", gio::Cancellable::NONE) {
        let free = info.attribute_uint64("filesystem::free");
        if free > 0 {
            return format!("{} disponibles", format_size(free));
        }
    }
    "".to_string()
}

pub fn open_file(path: &Path) -> Result<(), String> {
    let uri = format!("file://{}", path.display());
    gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        .map_err(|e| format!("Error al abrir archivo: {}", e))
}

pub fn open_in_terminal(path: &Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };

    let terminals = ["ghostty", "ptyxis", "gnome-terminal", "alacritty", "kitty", "xterm"];
    for term in terminals {
        let mut cmd = std::process::Command::new(term);
        cmd.current_dir(target);
        malus_de_common::config::apply_isolated_environment(&mut cmd);
        if cmd.spawn().is_ok() {
            return;
        }
    }
}

fn resolve_kind_and_icon(path: &Path, is_dir: bool, name: &str) -> (String, gio::Icon) {
    if is_dir {
        // Íconos específicos para carpetas del sistema
        let folder_icon_name = match name.to_lowercase().as_str() {
            "desktop" | "escritorio" => "user-desktop",
            "documents" | "documentos" => "folder-documents",
            "downloads" | "descargas" => "folder-download",
            "music" | "música" => "folder-music",
            "pictures" | "imágenes" => "folder-pictures",
            "videos" | "vídeos" => "folder-videos",
            "trash" | "papelera" => "user-trash",
            _ => "folder",
        };
        let icon = gio::ThemedIcon::new(folder_icon_name);
        return ("Carpeta".to_string(), icon.upcast());
    }

    // Archivo ordinario: consultar tipo MIME mediante GIO
    let gfile = gio::File::for_path(path);
    if let Ok(info) = gfile.query_info(
        "standard::content-type,standard::icon",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        let content_type = info.content_type().unwrap_or_default();
        let description = gio::content_type_get_description(&content_type).to_string();

        if let Some(icon) = info.icon() {
            return (description, icon);
        }
    }

    // Fallback según extensión
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let (desc, icon_name) = match ext.as_str() {
        "rs" | "c" | "cpp" | "py" | "js" | "ts" | "html" | "css" | "sh" => {
            ("Código Fuente", "text-x-script")
        }
        "png" | "jpg" | "jpeg" | "webp" | "svg" | "gif" => ("Imagen", "image-x-generic"),
        "mp4" | "mkv" | "mov" | "avi" | "webm" => ("Vídeo", "video-x-generic"),
        "mp3" | "flac" | "wav" | "ogg" | "m4a" => ("Audio", "audio-x-generic"),
        "pdf" => ("Documento PDF", "application-pdf"),
        "zip" | "tar" | "gz" | "xz" | "7z" => ("Archivo Comprimido", "package-x-generic"),
        "txt" | "md" => ("Documento de Texto", "text-plain"),
        _ => ("Documento", "text-x-generic"),
    };

    let icon = gio::ThemedIcon::new(icon_name);
    (desc.to_string(), icon.upcast())
}

fn percent_decode_file_uri(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((hi << 4) | lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

fn collect_recent_candidates(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if depth > 1 && !matches!(name, "target" | "node_modules" | ".git") {
                collect_recent_candidates(&path, depth - 1, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// Carga los archivos recientes del usuario combinando `recently-used.xbel`
/// y las carpetas personales recientes, ordenados del más reciente al más antiguo.
pub fn load_recent_files(home: &Path, search_dirs: &[PathBuf], limit: usize) -> Vec<FileItem> {
    use std::collections::HashSet;

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut items: Vec<FileItem> = Vec::new();

    let xbel_path = home.join(".local/share/recently-used.xbel");
    if let Ok(content) = fs::read_to_string(&xbel_path) {
        for segment in content.split("href=\"").skip(1) {
            if let Some((uri, _)) = segment.split_once('"') {
                if let Some(path) = percent_decode_file_uri(uri) {
                    if path.is_file() && seen.insert(path.clone()) {
                        if let Some(item) = FileItem::from_path(&path) {
                            if !item.is_hidden && !item.is_dir {
                                items.push(item);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut scanned = Vec::new();
    for dir in search_dirs {
        collect_recent_candidates(dir, 2, &mut scanned);
    }
    for path in scanned {
        if seen.insert(path.clone()) {
            if let Some(item) = FileItem::from_path(&path) {
                if !item.is_hidden && !item.is_dir {
                    items.push(item);
                }
            }
        }
    }

    items.sort_by(|a, b| {
        let a_time = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
        let b_time = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
        b_time.cmp(&a_time).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    items.truncate(limit);
    items
}

#[cfg(test)]
mod tests {
    use super::{load_recent_files, percent_decode_file_uri};
    use std::path::PathBuf;

    #[test]
    fn test_percent_decode_file_uri_and_load_recent_files() {
        assert_eq!(
            percent_decode_file_uri("file:///home/user/Mi%20Documento.pdf"),
            Some(PathBuf::from("/home/user/Mi Documento.pdf"))
        );

        let temp_dir = std::env::temp_dir().join("malus_branches_recents_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let docs = temp_dir.join("Docs");
        let sub = docs.join("Sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(docs.join("nota.txt"), "hola").unwrap();
        std::fs::write(sub.join("oculto"), "x").unwrap();
        std::fs::write(docs.join(".hidden"), "h").unwrap();

        let recent = load_recent_files(&temp_dir, &[docs], 10);
        let names: Vec<&str> = recent.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"nota.txt"));
        assert!(names.contains(&"oculto"));
        assert!(!names.contains(&".hidden"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
