use crate::state::{SortBy, SortOrder};
use gio::prelude::*;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    pub accessed: Option<SystemTime>,
    pub added: Option<SystemTime>,
    pub created: Option<SystemTime>,
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
        let accessed = target_metadata.accessed().ok();
        let created = target_metadata.created().ok().or(modified);
        let added = {
            use std::os::unix::fs::MetadataExt;
            let secs = target_metadata.ctime();
            if secs >= 0 {
                std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs as u64))
            } else {
                modified
            }
        };
        let modified_str = modified
            .map(format_modified)
            .unwrap_or_else(|| "--".to_string());

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
            accessed,
            added,
            created,
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
                && !item
                    .name
                    .to_lowercase()
                    .contains(&filter_query.to_lowercase())
            {
                continue;
            }
            items.push(item);
        }
    }

    sort_file_items(&mut items, sort_by, sort_order);

    Ok(items)
}

fn ctime_from_meta(meta: &fs::Metadata) -> Option<SystemTime> {
    use std::os::unix::fs::MetadataExt;
    let secs = meta.ctime();
    if secs >= 0 {
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs as u64))
    } else {
        None
    }
}

fn time_bucket_rank_label(time: Option<SystemTime>, now: SystemTime, es: bool) -> (u8, String) {
    let Some(t) = time else {
        return (
            6,
            if es { "Sin fecha" } else { "No Date" }.to_string(),
        );
    };
    let secs = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    if secs < 86_400 {
        (0, if es { "Hoy" } else { "Today" }.to_string())
    } else if secs < 2 * 86_400 {
        (1, if es { "Ayer" } else { "Yesterday" }.to_string())
    } else if secs < 7 * 86_400 {
        (
            2,
            if es { "Últimos 7 días" } else { "Previous 7 Days" }.to_string(),
        )
    } else if secs < 30 * 86_400 {
        (
            3,
            if es { "Últimos 30 días" } else { "Previous 30 Days" }.to_string(),
        )
    } else if secs < 365 * 86_400 {
        (
            4,
            if es { "Este año" } else { "This Year" }.to_string(),
        )
    } else {
        (5, if es { "Anterior" } else { "Earlier" }.to_string())
    }
}

fn item_group_key(
    item: &FileItem,
    sort_by: SortBy,
    tags_map: &std::collections::HashMap<PathBuf, Vec<(String, String)>>,
    now: SystemTime,
    es: bool,
) -> (u8, String) {
    match sort_by {
        SortBy::Name => {
            if item.is_dir {
                (0, if es { "Carpetas" } else { "Folders" }.to_string())
            } else {
                let first = item
                    .name
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().collect::<String>())
                    .unwrap_or_else(|| "#".to_string());
                let rank = if first.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    1
                } else {
                    2
                };
                (rank, first)
            }
        }
        SortBy::Kind => {
            if item.is_dir {
                (0, if es { "Carpetas" } else { "Folders" }.to_string())
            } else {
                (1, item.kind_str.clone())
            }
        }
        SortBy::DateLastOpened => time_bucket_rank_label(item.accessed.or(item.modified), now, es),
        SortBy::DateAdded => time_bucket_rank_label(item.added.or(item.modified), now, es),
        SortBy::DateModified => time_bucket_rank_label(item.modified, now, es),
        SortBy::DateCreated => time_bucket_rank_label(item.created.or(item.modified), now, es),
        SortBy::Size => {
            if item.is_dir {
                (0, if es { "Carpetas" } else { "Folders" }.to_string())
            } else if item.size >= 100 * 1024 * 1024 {
                (
                    1,
                    if es {
                        "Grande (> 100 MB)"
                    } else {
                        "Large (> 100 MB)"
                    }
                    .to_string(),
                )
            } else if item.size >= 1024 * 1024 {
                (
                    2,
                    if es {
                        "Mediano (1 - 100 MB)"
                    } else {
                        "Medium (1 - 100 MB)"
                    }
                    .to_string(),
                )
            } else {
                (
                    3,
                    if es {
                        "Pequeño (< 1 MB)"
                    } else {
                        "Small (< 1 MB)"
                    }
                    .to_string(),
                )
            }
        }
        SortBy::Tags => {
            if let Some((first_tag, _)) = tags_map.get(&item.path).and_then(|t| t.first()) {
                let label = crate::db::tag_definition(first_tag)
                    .map(|d| if es { d.label_es } else { d.label_en }.to_string())
                    .unwrap_or_else(|| first_tag.clone());
                (0, label)
            } else {
                (
                    1,
                    if es { "Sin etiqueta" } else { "No Tags" }.to_string(),
                )
            }
        }
    }
}

pub fn item_group_label(
    item: &FileItem,
    sort_by: SortBy,
    tags_map: &std::collections::HashMap<PathBuf, Vec<(String, String)>>,
    es: bool,
) -> String {
    item_group_key(item, sort_by, tags_map, SystemTime::now(), es).1
}

pub fn item_matches_kind_filter(item: &FileItem, kind_filter: Option<&str>) -> bool {
    let Some(filter) = kind_filter else {
        return true;
    };
    if filter.is_empty() || filter == "all" {
        return true;
    }
    if filter == "folders" {
        return item.is_dir;
    }
    if item.is_dir {
        return false;
    }
    let ext = item
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match filter {
        "images" => matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "ico" | "tif" | "tiff" | "avif" | "heic"
        ),
        "documents" => matches!(
            ext.as_str(),
            "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "csv" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp"
        ),
        "videos" => matches!(
            ext.as_str(),
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "flv" | "wmv" | "m4v"
        ),
        "audio" => matches!(
            ext.as_str(),
            "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" | "wma"
        ),
        "archives" => matches!(
            ext.as_str(),
            "zip" | "tar" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar"
        ),
        _ => true,
    }
}

pub fn sort_file_items(items: &mut [FileItem], sort_by: SortBy, sort_order: SortOrder) {
    let empty_tags = std::collections::HashMap::new();
    sort_file_items_with_tags(items, sort_by, sort_order, &empty_tags, false);
}

pub fn sort_file_items_with_tags(
    items: &mut [FileItem],
    sort_by: SortBy,
    sort_order: SortOrder,
    tags_map: &std::collections::HashMap<PathBuf, Vec<(String, String)>>,
    use_groups: bool,
) {
    let now = SystemTime::now();
    let es = malus_de_common::is_spanish_locale();
    items.sort_by(|a, b| {
        if use_groups {
            let ga = item_group_key(a, sort_by, tags_map, now, es);
            let gb = item_group_key(b, sort_by, tags_map, now, es);
            if ga != gb {
                return ga.cmp(&gb);
            }
        }

        if a.is_dir != b.is_dir {
            return b.is_dir.cmp(&a.is_dir); // Carpetas primero
        }

        let cmp_opt_time = |ta: Option<SystemTime>, tb: Option<SystemTime>| {
            let ua = ta.unwrap_or(SystemTime::UNIX_EPOCH);
            let ub = tb.unwrap_or(SystemTime::UNIX_EPOCH);
            ua.cmp(&ub)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        };

        let ordering = match sort_by {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Kind => a
                .kind_str
                .to_lowercase()
                .cmp(&b.kind_str.to_lowercase())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortBy::DateLastOpened => {
                cmp_opt_time(a.accessed.or(a.modified), b.accessed.or(b.modified))
            }
            SortBy::DateAdded => cmp_opt_time(a.added.or(a.modified), b.added.or(b.modified)),
            SortBy::DateModified => cmp_opt_time(a.modified, b.modified),
            SortBy::DateCreated => cmp_opt_time(a.created.or(a.modified), b.created.or(b.modified)),
            SortBy::Size => a
                .size
                .cmp(&b.size)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortBy::Tags => {
                let ta = tags_map.get(&a.path).and_then(|v| v.first()).map(|t| &t.0);
                let tb = tags_map.get(&b.path).and_then(|v| v.first()).map(|t| &t.0);
                match (ta, tb) {
                    (Some(sa), Some(sb)) => sa
                        .cmp(sb)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                }
            }
        };

        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
    });
}

/// Búsqueda recursiva BFS acotada y cancelable sobre `root` devolviendo rutas `Send`.
pub fn search_paths_recursive(
    root: &Path,
    filter_query: &str,
    show_hidden: bool,
    max_results: usize,
    cancel: &AtomicBool,
) -> Vec<PathBuf> {
    let needle = filter_query.trim().to_lowercase();
    let mut results = Vec::new();
    if needle.is_empty() || max_results == 0 {
        return results;
    }

    let mut queue = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0usize));
    const MAX_DEPTH: usize = 8;

    while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) || results.len() >= max_results {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if cancel.load(Ordering::Relaxed) || results.len() >= max_results {
                break;
            }
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_hidden = name.starts_with('.');
            if !show_hidden && is_hidden {
                continue;
            }
            if name.to_lowercase().contains(&needle) {
                results.push(p.clone());
            }
            if let Ok(meta) = fs::symlink_metadata(&p) {
                let is_dir = meta.is_dir();
                let is_symlink = meta.file_type().is_symlink();
                if is_dir && !is_symlink && depth < MAX_DEPTH {
                    let skip_heavy = matches!(name.as_str(), ".git" | "node_modules" | "target");
                    if !skip_heavy {
                        queue.push_back((p, depth + 1));
                    }
                }
            }
        }
    }

    results
}

/// Búsqueda recursiva BFS acotada y cancelable sobre `root`.
#[cfg(test)]
pub fn search_directory_recursive(
    root: &Path,
    filter_query: &str,
    show_hidden: bool,
    sort_by: SortBy,
    sort_order: SortOrder,
    max_results: usize,
    cancel: &AtomicBool,
) -> Vec<FileItem> {
    let mut results: Vec<FileItem> =
        search_paths_recursive(root, filter_query, show_hidden, max_results, cancel)
            .into_iter()
            .filter_map(|p| FileItem::from_path(&p))
            .collect();
    sort_file_items(&mut results, sort_by, sort_order);
    results
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

/// Convierte fechas (año/mes/dias) a numeros absolutos de dias (Rata Die)
fn date_to_days(year: i32, month: i32, day: i32) -> i64 {
    let leap_month_adjustment = (14 - month) / 12;
    let adjusted_year = year + 4800 - leap_month_adjustment;
    let adjusted_month = month + 12 * leap_month_adjustment - 3;

    day as i64
        + (153 * adjusted_month as i64 + 2) / 5
        + 365 * adjusted_year as i64
        + adjusted_year as i64 / 4
        - adjusted_year as i64 / 100
        + adjusted_year as i64 / 400
        - 32045
}

/// Lógica interna que permite inyectar el tiempo actual (`now`) para pruebas deterministas.
fn format_modified_with_now(time: SystemTime, now: &glib::DateTime) -> String {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as i64;

    let dt = glib::DateTime::from_unix_local(secs).unwrap_or_else(|_| now.clone());

    if dt > *now {
        return dt.format("%d/%m/%Y %H:%M").unwrap_or_default().to_string();
    }

    let now_days = date_to_days(now.year(), now.month(), now.day_of_month());
    let dt_days = date_to_days(dt.year(), dt.month(), dt.day_of_month());
    let diff_days = now_days - dt_days;

    if diff_days == 0 {
        format!("Hoy a las {}", dt.format("%H:%M").unwrap_or_default())
    } else if diff_days == 1 {
        format!("Ayer a las {}", dt.format("%H:%M").unwrap_or_default())
    } else if diff_days < 7 {
        format!("{}", dt.format("%a %H:%M").unwrap_or_default())
    } else {
        format!("{}", dt.format("%d/%m/%Y %H:%M").unwrap_or_default())
    }
}

pub fn format_modified(time: SystemTime) -> String {
    let now = glib::DateTime::now_local().unwrap();
    format_modified_with_now(time, &now)
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
    if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
        return launch_desktop_file(path);
    }
    let uri = format!("file://{}", path.display());
    gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        .map_err(|e| format!("Error al abrir archivo: {}", e))
}

fn launch_desktop_file(path: &Path) -> Result<(), String> {
    if malus_de_common::apps::spawn_desktop_entry_path(path).is_ok() {
        return Ok(());
    }

    if let Some(app_info) = gio::DesktopAppInfo::from_filename(path) {
        app_info
            .launch(&[], None::<&gio::AppLaunchContext>)
            .map_err(|e| format!("Error al iniciar aplicación: {}", e))?;
        return Ok(());
    }

    malus_de_common::apps::spawn_desktop_entry_path(path)
}

fn resolve_desktop_gicon(icon: &str) -> gio::Icon {
    let trimmed = icon.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return gio::ThemedIcon::new("application-x-executable").upcast();
    }
    let path = Path::new(trimmed);
    if path.is_absolute() && path.is_file() {
        return gio::FileIcon::new(&gio::File::for_path(path)).upcast();
    }
    if !trimmed.ends_with("-symbolic") {
        if let Some(resolved_path) = malus_de_common::apps::resolve_icon_file_path(trimmed) {
            return gio::FileIcon::new(&gio::File::for_path(&resolved_path)).upcast();
        }
    }
    gio::ThemedIcon::from_names(&[trimmed, "application-x-executable"]).upcast()
}

/// Convierte una entrada de aplicación de escritorio al modelo `FileItem` de Branches.
pub fn file_item_from_desktop_entry(app: &malus_de_common::apps::DesktopAppEntry) -> FileItem {
    let metadata = fs::symlink_metadata(&app.desktop_file).ok();
    let is_symlink = metadata
        .as_ref()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    let target_meta = fs::metadata(&app.desktop_file).ok().or(metadata);
    let size = target_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified = target_meta.as_ref().and_then(|m| m.modified().ok());
    let accessed = target_meta.as_ref().and_then(|m| m.accessed().ok());
    let created = target_meta.as_ref().and_then(|m| m.created().ok()).or(modified);
    let added = target_meta.as_ref().and_then(ctime_from_meta).or(modified);
    let modified_str = modified
        .map(format_modified)
        .unwrap_or_else(|| "--".to_string());
    let kind_str = if malus_de_common::is_spanish_locale() {
        "Aplicación".to_string()
    } else {
        "Application".to_string()
    };

    FileItem {
        name: app.name.clone(),
        path: app.desktop_file.clone(),
        is_dir: false,
        is_hidden: false,
        is_symlink,
        size,
        size_str: if size > 0 {
            format_size(size)
        } else {
            "--".to_string()
        },
        modified,
        accessed,
        added,
        created,
        modified_str,
        kind_str,
        gicon: resolve_desktop_gicon(&app.icon),
    }
}

/// Carga las aplicaciones visibles indexadas igual que la sección Aplicaciones de Spotlight.
pub fn load_applications_items(filter_query: &str) -> Vec<FileItem> {
    let query = filter_query.trim().to_lowercase();
    malus_de_common::apps::index_applications()
        .into_iter()
        .filter(|app| {
            query.is_empty()
                || app.name.to_lowercase().contains(&query)
                || app.comment.to_lowercase().contains(&query)
                || app.search_terms.contains(&query)
        })
        .map(|app| file_item_from_desktop_entry(&app))
        .collect()
}

pub fn open_in_terminal(path: &Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };

    let terminals = [
        "ghostty",
        "ptyxis",
        "gnome-terminal",
        "alacritty",
        "kitty",
        "xterm",
    ];
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
            "public" | "público" | "publico" => "folder-publicshare",
            "projects" | "proyectos" => "folder-projects",
            "templates" | "plantillas" => "folder-templates",
            "trash" | "papelera" => "user-trash",
            _ => "folder",
        };
        let icon = gio::ThemedIcon::new(folder_icon_name);
        return ("Carpeta".to_string(), icon.upcast());
    }

    if !is_dir && path.extension().and_then(|e| e.to_str()) == Some("desktop") {
        if let Some(entry) = malus_de_common::apps::parse_desktop_file_any(path) {
            let kind = if malus_de_common::is_spanish_locale() {
                "Aplicación".to_string()
            } else {
                "Application".to_string()
            };
            return (kind, resolve_desktop_gicon(&entry.icon));
        }
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
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
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

pub(crate) fn percent_decode_file_uri(uri: &str) -> Option<PathBuf> {
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
        b_time
            .cmp(&a_time)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    items.truncate(limit);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{load_recent_files, percent_decode_file_uri, resolve_kind_and_icon, FileItem};
    use gio::prelude::{Cast, FileExt};
    use std::path::{Path, PathBuf};

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

    #[test]
    fn test_desktop_file_resolves_app_gicon() {
        let temp_dir = std::env::temp_dir().join("malus_branches_desktop_icon_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let icon_file = temp_dir.join("demo-app.svg");
        std::fs::write(&icon_file, b"<svg/>").unwrap();
        let desktop_file = temp_dir.join("demo.desktop");
        std::fs::write(
            &desktop_file,
            format!(
                "[Desktop Entry]\nType=Application\nName=Demo\nExec=/bin/true\nIcon={}\n",
                icon_file.display()
            ),
        )
        .unwrap();

        let item = FileItem::from_path(&desktop_file).expect("desktop item");
        let file_icon = item
            .gicon
            .downcast_ref::<gio::FileIcon>()
            .expect("debe usar FileIcon para icono propio de .desktop");
        assert_eq!(file_icon.file().path(), Some(icon_file));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_special_folder_themed_icons() {
        let cases = [
            ("Público", "folder-publicshare"),
            ("Publico", "folder-publicshare"),
            ("Public", "folder-publicshare"),
            ("Proyectos", "folder-projects"),
            ("Projects", "folder-projects"),
            ("Plantillas", "folder-templates"),
            ("Templates", "folder-templates"),
            ("Escritorio", "user-desktop"),
            ("Desktop", "user-desktop"),
        ];

        for (name, expected_icon) in cases {
            let (kind, gicon) = resolve_kind_and_icon(Path::new(name), true, name);
            assert_eq!(kind, "Carpeta");
            let themed = gicon
                .downcast_ref::<gio::ThemedIcon>()
                .expect("las carpetas deben resolver a ThemedIcon");
            assert_eq!(
                themed.names().first().map(|s| s.as_str()),
                Some(expected_icon)
            );
        }
    }

    #[test]
    fn test_malus_icon_theme_resolves_color_folders_and_documents_not_symbolic() {
        let icons_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
        assert!(icons_root.is_dir(), "assets/icons debe existir");

        let folder_icons = [
            "folder",
            "folder-documents",
            "folder-projects",
            "folder-publicshare",
            "folder-templates",
            "user-desktop",
        ];

        for theme_name in ["Malus-Dark", "Malus"] {
            let theme_dir = icons_root.join(theme_name);
            let index_content = std::fs::read_to_string(theme_dir.join("index.theme"))
                .expect("index.theme debe existir");
            assert!(
                !index_content.contains("places/16"),
                "{theme_name} no debe incluir places/16 monocromo sobre places/scalable"
            );

            for folder_icon in folder_icons {
                let p = theme_dir
                    .join("places/scalable")
                    .join(format!("{folder_icon}.svg"));
                assert!(
                    p.is_file(),
                    "{theme_name} debe resolver {} en places/scalable",
                    p.display()
                );
            }
            for doc_icon in ["text-plain", "text-x-generic", "application-pdf"] {
                let p = theme_dir
                    .join("mimetypes/scalable")
                    .join(format!("{doc_icon}.svg"));
                assert!(
                    p.is_file(),
                    "{theme_name} debe resolver {} en mimetypes/scalable",
                    p.display()
                );
            }
            for sym_rel in [
                "places/symbolic/folder-documents-symbolic.svg",
                "actions/symbolic/go-previous-symbolic.svg",
                "status/symbolic/control-center-symbolic.svg",
            ] {
                let p = theme_dir.join(sym_rel);
                assert!(p.is_file(), "{theme_name} debe incluir {}", p.display());
            }
        }

        if gtk4::init().is_err() {
            return;
        }

        for theme_name in ["Malus-Dark", "Malus"] {
            let theme = gtk4::IconTheme::new();
            theme.set_search_path(&[&icons_root]);
            theme.set_theme_name(Some(theme_name));

            for folder_icon in folder_icons {
                let gicon = gio::ThemedIcon::new(folder_icon);
                for size in [20, 64] {
                    let paintable = theme.lookup_by_gicon(
                        &gicon,
                        size,
                        1,
                        gtk4::TextDirection::Ltr,
                        gtk4::IconLookupFlags::empty(),
                    );
                    assert!(
                        !paintable.is_symbolic(),
                        "{theme_name} {folder_icon} ({size}px) no debe resolverse como simbólico"
                    );
                    let path = paintable
                        .file()
                        .and_then(|f| f.path())
                        .expect("debe resolver a archivo SVG");
                    let path_str = path.to_string_lossy();
                    assert!(
                        path_str.contains("/places/scalable/"),
                        "{theme_name} {folder_icon} ({size}px) resolvió a ruta inesperada: {path_str}"
                    );
                }
            }

            for mime in ["text/plain", "application/pdf"] {
                let gicon = gio::content_type_get_icon(mime);
                let paintable = theme.lookup_by_gicon(
                    &gicon,
                    64,
                    1,
                    gtk4::TextDirection::Ltr,
                    gtk4::IconLookupFlags::empty(),
                );
                assert!(
                    !paintable.is_symbolic(),
                    "{theme_name} {mime} no debe resolverse como simbólico"
                );
                let path = paintable
                    .file()
                    .and_then(|f| f.path())
                    .expect("debe resolver a archivo SVG");
                let path_str = path.to_string_lossy();
                assert!(
                    (path_str.contains("/mimetypes/scalable/")
                        || path_str.contains("/apps/scalable/"))
                        && !path_str.contains("/symbolic"),
                    "{theme_name} {mime} resolvió a ruta inesperada: {path_str}"
                );
            }

            for sym_icon in [
                "folder-documents-symbolic",
                "go-previous-symbolic",
                "control-center-symbolic",
            ] {
                let gicon = gio::ThemedIcon::new(sym_icon);
                let paintable = theme.lookup_by_gicon(
                    &gicon,
                    16,
                    1,
                    gtk4::TextDirection::Ltr,
                    gtk4::IconLookupFlags::empty(),
                );
                assert!(
                    paintable.is_symbolic(),
                    "{theme_name} {sym_icon} sí debe ser simbólico"
                );
            }
        }
    }

    #[test]
    fn test_date_to_days_calculation() {
        let d1 = date_to_days(2026, 1, 1);
        let d2 = date_to_days(2026, 1, 2);
        assert_eq!(d2 - d1, 1);
    }

    #[test]
    fn test_format_modified_today() {
        let now = glib::DateTime::now_local().unwrap();
        let elapsed_secs_today =
            now.hour() as i64 * 3600 + now.minute() as i64 * 60 + now.second() as i64;

        // Si estamos muy cerca de medianoche, usamos un desfase seguro de 1 minuto, sino 1 hora
        let offset = if elapsed_secs_today > 120 { 60 } else { 1 };
        let time_secs = now.to_unix() - offset;
        let time =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time_secs as u64);

        let result = super::format_modified_with_now(time, &now);
        assert!(result.starts_with("Hoy a las"));
    }

    #[test]
    fn test_format_modified_yesterday() {
        let now = glib::DateTime::now_local().unwrap();
        let elapsed_secs_today =
            now.hour() as i64 * 3600 + now.minute() as i64 * 60 + now.second() as i64;

        // Restamos los segundos transcurridos hoy más 6 horas para caer siempre en la tarde de ayer
        let time_secs = now.to_unix() - elapsed_secs_today - 6 * 3600;
        let time =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time_secs as u64);

        let result = super::format_modified_with_now(time, &now);
        assert!(result.starts_with("Ayer a las"));
    }

    #[test]
    fn test_modified_future_date() {
        let now = glib::DateTime::now_local().unwrap();
        // 10 días en el futuro
        let time_secs = now.to_unix() + 864000;
        let time =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time_secs as u64);

        let result = super::format_modified_with_now(time, &now);

        assert!(result.contains('/'));
        assert!(!result.contains("Hoy"));
        assert!(!result.contains("Ayer"));
    }

    #[test]
    fn split_desktop_exec_args_handles_quotes_and_escapes() {
        assert_eq!(
            malus_de_common::apps::split_desktop_exec_args(
                r#""/opt/My App/bin" --title="Hello World" 'single arg'"#
            ),
            vec!["/opt/My App/bin", "--title=Hello World", "single arg",]
        );
        assert_eq!(
            malus_de_common::apps::split_desktop_exec_args(
                r#"env FOO=bar /usr/bin/app\ name --flag"#
            ),
            vec!["env", "FOO=bar", "/usr/bin/app name", "--flag"]
        );
    }

    #[test]
    fn read_desktop_launch_metadata_detects_terminal_and_path() {
        let dir = std::env::temp_dir().join("malus_branches_launch_meta_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let desktop_file = dir.join("term.desktop");
        std::fs::write(
            &desktop_file,
            format!(
                "[Desktop Entry]\nType=Application\nName=TermTool\nExec=btop\nTerminal=true\nPath={}\n",
                dir.display()
            ),
        )
        .unwrap();

        let (term, wd) = malus_de_common::apps::read_desktop_launch_metadata(&desktop_file);
        assert!(term);
        assert_eq!(wd.as_deref(), Some(dir.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recursive_search_finds_nested_matches_skips_heavy_dirs_and_honors_cancel() {
        let root = std::env::temp_dir().join("malus_branches_rec_search_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();

        fs::write(root.join("a/reporte_anual.txt"), b"x").unwrap();
        fs::write(root.join("a/b/otro_reporte.md"), b"y").unwrap();
        fs::write(root.join(".git/reporte_oculto.txt"), b"z").unwrap();
        fs::write(root.join("node_modules/reporte_pkg.js"), b"w").unwrap();

        let cancel = AtomicBool::new(false);
        let found = search_directory_recursive(
            &root,
            "reporte",
            false,
            SortBy::Name,
            SortOrder::Ascending,
            50,
            &cancel,
        );
        let names: Vec<String> = found.into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["otro_reporte.md", "reporte_anual.txt"]);

        cancel.store(true, Ordering::Relaxed);
        let cancelled = search_directory_recursive(
            &root,
            "reporte",
            false,
            SortBy::Name,
            SortOrder::Ascending,
            50,
            &cancel,
        );
        assert!(cancelled.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn grouping_sort_and_kind_filter_cover_finder_options() {
        let mk = |name: &str, is_dir: bool, size: u64, kind: &str| FileItem {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/g/{name}")),
            is_dir,
            is_hidden: false,
            is_symlink: false,
            size,
            size_str: format!("{size} B"),
            modified: Some(SystemTime::now()),
            accessed: None,
            added: None,
            created: None,
            modified_str: "Hoy".to_string(),
            kind_str: kind.to_string(),
            gicon: gio::ThemedIcon::new("text-x-generic").upcast(),
        };

        let img = mk("foto.png", false, 200, "Imagen PNG");
        let doc = mk("informe.pdf", false, 1500, "Documento PDF");
        let dir = mk("Proyectos", true, 0, "Carpeta");

        assert!(item_matches_kind_filter(&img, Some("images")));
        assert!(!item_matches_kind_filter(&doc, Some("images")));
        assert!(item_matches_kind_filter(&doc, Some("documents")));
        assert!(item_matches_kind_filter(&dir, Some("folders")));
        assert!(!item_matches_kind_filter(&dir, Some("images")));

        let mut tags = std::collections::HashMap::new();
        tags.insert(doc.path.clone(), vec![("red".to_string(), "#ff3b30".to_string())]);

        let mut items = vec![img.clone(), doc.clone(), dir.clone()];
        sort_file_items_with_tags(&mut items, SortBy::Tags, SortOrder::Ascending, &tags, true);
        assert_eq!(item_group_label(&doc, SortBy::Tags, &tags, true), "Rojo");
        assert_eq!(item_group_label(&img, SortBy::Tags, &tags, true), "Sin etiqueta");
    }
}
