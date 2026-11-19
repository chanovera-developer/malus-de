use gio::prelude::*;
use gtk4::gdk::prelude::DisplayExt;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardAction {
    Copy,
    Cut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload {
    pub action: ClipboardAction,
    pub paths: Vec<PathBuf>,
}

thread_local! {
    static FILE_CLIPBOARD: RefCell<Option<ClipboardPayload>> = const { RefCell::new(None) };
}

pub fn encode_gnome_copied_files(action: &ClipboardAction, paths: &[PathBuf]) -> String {
    let op = match action {
        ClipboardAction::Copy => "copy",
        ClipboardAction::Cut => "cut",
    };
    let mut lines = Vec::with_capacity(paths.len() + 1);
    lines.push(op.to_string());
    for p in paths {
        lines.push(gio::File::for_path(p).uri().to_string());
    }
    lines.join("\n")
}

pub fn parse_gnome_copied_files(text: &str) -> Option<ClipboardPayload> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let op = lines.next()?;
    let action = match op {
        "copy" => ClipboardAction::Copy,
        "cut" => ClipboardAction::Cut,
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines
        .filter_map(|uri| {
            if uri.starts_with('#') {
                return None;
            }
            if uri.contains("://") {
                gio::File::for_uri(uri).path()
            } else {
                Some(PathBuf::from(uri))
            }
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(ClipboardPayload { action, paths })
    }
}

pub fn parse_uri_list(text: &str) -> Option<ClipboardPayload> {
    let paths: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            if line.contains("://") {
                gio::File::for_uri(line).path()
            } else if line.starts_with('/') {
                Some(PathBuf::from(line))
            } else {
                None
            }
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(ClipboardPayload {
            action: ClipboardAction::Copy,
            paths,
        })
    }
}

pub fn set_clipboard(action: ClipboardAction, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let gnome_payload = encode_gnome_copied_files(&action, &paths);
    let uri_list: String = paths
        .iter()
        .map(|p| format!("{}\r\n", gio::File::for_path(p).uri()))
        .collect();

    FILE_CLIPBOARD.with(|slot| {
        *slot.borrow_mut() = Some(ClipboardPayload {
            action,
            paths,
        });
    });

    if let Some(display) = gtk4::gdk::Display::default() {
        let provider = gtk4::gdk::ContentProvider::new_union(&[
            gtk4::gdk::ContentProvider::for_bytes(
                "x-special/gnome-copied-files",
                &glib::Bytes::from_owned(gnome_payload),
            ),
            gtk4::gdk::ContentProvider::for_bytes(
                "text/uri-list",
                &glib::Bytes::from_owned(uri_list),
            ),
        ]);
        let _ = display.clipboard().set_content(Some(&provider));
    }
}

pub fn get_clipboard_payload() -> Option<ClipboardPayload> {
    FILE_CLIPBOARD.with(|slot| slot.borrow().clone())
}

pub fn clear_clipboard() {
    let had_local = FILE_CLIPBOARD.with(|slot| slot.borrow_mut().take().is_some());
    if had_local {
        if let Some(display) = gtk4::gdk::Display::default() {
            let formats = display.clipboard().formats();
            if formats.contain_mime_type("x-special/gnome-copied-files") {
                let _ = display
                    .clipboard()
                    .set_content(None::<&gtk4::gdk::ContentProvider>);
            }
        }
    }
}

pub fn has_clipboard_files() -> bool {
    if let Some(display) = gtk4::gdk::Display::default() {
        let formats = display.clipboard().formats();
        if formats.contain_mime_type("x-special/gnome-copied-files")
            || formats.contain_mime_type("text/uri-list")
        {
            return true;
        }
        return false;
    }
    get_clipboard_payload().is_some()
}

pub fn read_clipboard_payload_async<F: FnOnce(Option<ClipboardPayload>) + 'static>(on_payload: F) {
    let Some(display) = gtk4::gdk::Display::default() else {
        on_payload(get_clipboard_payload());
        return;
    };

    let clipboard = display.clipboard();
    let formats = clipboard.formats();
    let has_gnome = formats.contain_mime_type("x-special/gnome-copied-files");
    let has_uri = formats.contain_mime_type("text/uri-list");
    if !has_gnome && !has_uri {
        on_payload(None);
        return;
    }

    if clipboard.is_local() {
        on_payload(get_clipboard_payload());
        return;
    }

    glib::MainContext::default().spawn_local(async move {
        let payload = if let Ok((stream, mime)) = clipboard
            .read_future(
                &["x-special/gnome-copied-files", "text/uri-list"],
                glib::Priority::DEFAULT,
            )
            .await
        {
            let mut raw = Vec::new();
            loop {
                let buf = vec![0u8; 65536];
                match stream.read_future(buf, glib::Priority::DEFAULT).await {
                    Ok((chunk, n)) if n > 0 => {
                        raw.extend_from_slice(&chunk[..n]);
                        if raw.len() >= 4 * 1024 * 1024 {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            let text = String::from_utf8_lossy(&raw);
            if mime.as_str() == "x-special/gnome-copied-files" {
                parse_gnome_copied_files(&text)
            } else {
                parse_uri_list(&text)
            }
        } else {
            None
        };
        on_payload(payload);
    });
}

#[allow(dead_code)]
pub fn paste_from_clipboard_async<F: Fn(bool) + 'static>(dest_dir: PathBuf, on_done: F) {
    read_clipboard_payload_async(move |payload| {
        if let Some(payload) = payload {
            let is_cut = payload.action == ClipboardAction::Cut;
            let pasted = paste_payload_into(&payload, &dest_dir).unwrap_or_default();
            if is_cut && !pasted.is_empty() {
                clear_clipboard();
            }
            on_done(!pasted.is_empty());
        } else {
            on_done(false);
        }
    });
}

pub fn is_path_cut(path: &Path) -> bool {
    FILE_CLIPBOARD.with(|slot| {
        slot.borrow().as_ref().is_some_and(|p| {
            p.action == ClipboardAction::Cut && p.paths.iter().any(|cut_p| cut_p == path)
        })
    })
}

/// Enviar elemento a la Papelera mediante GIO (cumple especificación FreeDesktop Trash)
pub fn trash_item(path: &Path) -> Result<(), String> {
    let file = gio::File::for_path(path);
    file.trash(gio::Cancellable::NONE)
        .map_err(|e| format!("Error al mover a papelera: {}", e))
}

/// Directorio base FreeDesktop de la Papelera (`$XDG_DATA_HOME/Trash` o `~/.local/share/Trash`).
pub fn trash_base_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("Trash");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".local/share/Trash")
}

pub fn trash_files_dir() -> PathBuf {
    trash_base_dir().join("files")
}

pub fn is_in_trash_dir(path: &Path) -> bool {
    let files_dir = trash_files_dir();
    path == files_dir || path.starts_with(&files_dir) || path.ends_with("Trash/files")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashInfo {
    pub original_path: PathBuf,
    pub deletion_date: Option<String>,
}

fn percent_decode_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_trash_info_in(trash_base: &Path, trash_item_path: &Path) -> Option<TrashInfo> {
    let name = trash_item_path.file_name()?.to_string_lossy();
    let info_path = trash_base.join("info").join(format!("{}.trashinfo", name));
    let content = fs::read_to_string(&info_path).ok()?;
    let mut in_section = false;
    let mut path_val: Option<String> = None;
    let mut date_val: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[Trash Info]" {
            in_section = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_section = false;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Path=") {
            path_val = Some(percent_decode_path(rest));
        } else if let Some(rest) = trimmed.strip_prefix("DeletionDate=") {
            date_val = Some(rest.to_string());
        }
    }

    let decoded = path_val?;
    let p = PathBuf::from(&decoded);
    let original_path = if p.is_absolute() {
        p
    } else if let Some(data_home) = trash_base.parent() {
        data_home.join(p)
    } else {
        p
    };

    Some(TrashInfo {
        original_path,
        deletion_date: date_val,
    })
}

pub fn restore_from_trash_in(trash_base: &Path, trash_item_path: &Path) -> Result<PathBuf, String> {
    let info = parse_trash_info_in(trash_base, trash_item_path)
        .ok_or_else(|| "No se encontró metadato .trashinfo para restaurar el elemento".to_string())?;
    let mut target = info.original_path;
    let parent = target
        .parent()
        .ok_or_else(|| "Ruta original inválida".to_string())?;
    if !parent.exists() {
        fs::create_dir_all(parent).map_err(|e| format!("No se pudo crear carpeta destino: {}", e))?;
    }
    if target.exists() || target.symlink_metadata().is_ok() {
        target = unique_destination_for_copy(&target, parent)?;
    }

    let src_file = gio::File::for_path(trash_item_path);
    let dst_file = gio::File::for_path(&target);
    src_file
        .move_(
            &dst_file,
            gio::FileCopyFlags::NONE,
            gio::Cancellable::NONE,
            None,
        )
        .or_else(|_| fs::rename(trash_item_path, &target))
        .map_err(|e| format!("Error al restaurar desde Papelera: {}", e))?;

    if let Some(name) = trash_item_path.file_name().map(|n| n.to_string_lossy()) {
        let info_path = trash_base.join("info").join(format!("{}.trashinfo", name));
        let _ = fs::remove_file(info_path);
    }

    Ok(target)
}

pub fn restore_from_trash(trash_item_path: &Path) -> Result<PathBuf, String> {
    restore_from_trash_in(&trash_base_dir(), trash_item_path)
}

pub fn empty_trash_in(trash_base: &Path) -> Result<usize, String> {
    let files_dir = trash_base.join("files");
    let info_dir = trash_base.join("info");
    let mut count = 0usize;

    if files_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&files_dir) {
            for entry in entries.flatten() {
                delete_permanent(&entry.path())?;
                count += 1;
            }
        }
    }
    if info_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&info_dir) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(count)
}

pub fn empty_trash() -> Result<usize, String> {
    empty_trash_in(&trash_base_dir())
}

/// Eliminar permanentemente (sin pasar por la papelera, limpiando también `.trashinfo` si existe)
pub fn delete_permanent(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("No se pudo inspeccionar elemento a eliminar: {}", e))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|e| format!("Error al eliminar carpeta: {}", e))?;
    } else {
        fs::remove_file(path).map_err(|e| format!("Error al eliminar archivo: {}", e))?;
    }
    if is_in_trash_dir(path) {
        if let Some(name) = path.file_name().map(|n| n.to_string_lossy()) {
            let info_path = trash_base_dir()
                .join("info")
                .join(format!("{}.trashinfo", name));
            let _ = fs::remove_file(info_path);
        }
    }
    Ok(())
}

/// Calcula una ruta destino única al duplicar o resolver "Mantener ambos" ("nombre copia.ext", "nombre copia 2.ext").
pub fn unique_destination_for_copy(src: &Path, dest_folder: &Path) -> Result<PathBuf, String> {
    let file_name = src
        .file_name()
        .ok_or_else(|| "Nombre de archivo inválido".to_string())?;
    let is_dir = src
        .symlink_metadata()
        .map(|m| m.is_dir())
        .unwrap_or_else(|_| src.is_dir());

    let (stem, ext) = if is_dir {
        (file_name.to_string_lossy().to_string(), String::new())
    } else {
        (
            src.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file_name.to_string_lossy().to_string()),
            src.extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default(),
        )
    };

    let mut counter = 2usize;
    let mut dest = dest_folder.join(format!("{} copia{}", stem, ext));
    while dest.exists() || dest.symlink_metadata().is_ok() {
        dest = dest_folder.join(format!("{} copia {}{}", stem, counter, ext));
        counter += 1;
    }
    Ok(dest)
}

/// Calcula la siguiente ruta de duplicado para un elemento existente en su misma carpeta.
pub fn next_duplicate_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "No se encontró directorio padre".to_string())?;
    unique_destination_for_copy(path, parent)
}

#[allow(dead_code)]
/// Duplicar archivo o carpeta al estilo macOS Finder ("nombre copia.ext", "nombre copia 2.ext")
pub fn duplicate_item(path: &Path) -> Result<PathBuf, String> {
    let candidate = next_duplicate_path(path)?;

    if path.is_dir() {
        copy_dir_recursive(path, &candidate)?;
    } else {
        fs::copy(path, &candidate)
            .map_err(|e| format!("Error al duplicar archivo: {}", e))?;
    }

    Ok(candidate)
}

/// Calcula una ruta destino única al copiar o mover sin sobrescribir archivos/carpetas existentes.
fn unique_destination_in_folder(src: &Path, dest_folder: &Path) -> Result<PathBuf, String> {
    let file_name = src
        .file_name()
        .ok_or_else(|| "Nombre de archivo inválido".to_string())?;

    let dest = dest_folder.join(file_name);
    if !dest.exists() && dest.symlink_metadata().is_err() {
        return Ok(dest);
    }
    unique_destination_for_copy(src, dest_folder)
}

/// Copiar archivo o carpeta a una carpeta destino
pub fn copy_into(src: &Path, dest_folder: &Path) -> Result<PathBuf, String> {
    if src == dest_folder || (src.is_dir() && dest_folder.starts_with(src)) {
        return Err("No se puede copiar una carpeta dentro de sí misma.".to_string());
    }
    let dest = unique_destination_in_folder(src, dest_folder)?;

    if src.is_dir() {
        copy_dir_recursive(src, &dest)?;
    } else {
        fs::copy(src, &dest).map_err(|e| format!("Error al copiar: {}", e))?;
    }

    Ok(dest)
}

/// Mover archivo o carpeta a una carpeta destino (con respaldo entre sistemas de archivos y metadatos SQLite)
pub fn move_into(src: &Path, dest_folder: &Path) -> Result<PathBuf, String> {
    if src == dest_folder || src.parent() == Some(dest_folder) || (src.is_dir() && dest_folder.starts_with(src)) {
        return Err("No se puede mover aquí.".to_string());
    }
    let dest = unique_destination_in_folder(src, dest_folder)?;
    let is_dir = src
        .symlink_metadata()
        .map(|m| m.is_dir())
        .unwrap_or_else(|_| src.is_dir());

    let src_file = gio::File::for_path(src);
    let dest_file = gio::File::for_path(&dest);
    src_file
        .move_(
            &dest_file,
            gio::FileCopyFlags::NONE,
            gio::Cancellable::NONE,
            None,
        )
        .or_else(|_| fs::rename(src, &dest))
        .map_err(|e| format!("Error al mover: {}", e))?;

    if let Ok(mut conn) = crate::db::open_db() {
        let _ = crate::db::rename_item_metadata(&mut conn, src, &dest, is_dir);
    }

    Ok(dest)
}

/// Ejecuta Pegar (`Copy` o `Cut`) de un `ClipboardPayload` sobre `dest_folder`, devolviendo las rutas resultantes.
pub fn paste_payload_into(payload: &ClipboardPayload, dest_folder: &Path) -> Result<Vec<PathBuf>, String> {
    if !dest_folder.is_dir() {
        return Err("La carpeta de destino no es válida.".to_string());
    }

    let mut pasted = Vec::new();
    for src in &payload.paths {
        if !src.exists() && src.symlink_metadata().is_err() {
            continue;
        }
        if src == dest_folder || (src.is_dir() && dest_folder.starts_with(src)) {
            continue;
        }
        match payload.action {
            ClipboardAction::Copy => {
                if let Ok(dst) = copy_into(src, dest_folder) {
                    pasted.push(dst);
                }
            }
            ClipboardAction::Cut => {
                if src.parent() == Some(dest_folder) {
                    continue;
                }
                if let Ok(dst) = move_into(src, dest_folder) {
                    pasted.push(dst);
                }
            }
        }
    }

    Ok(pasted)
}

/// Calcula el índice de carácter final a preseleccionar al renombrar (omite la extensión en archivos).
pub fn rename_selection_end(name: &str, is_dir: bool) -> i32 {
    if !is_dir {
        if let Some((stem, _ext)) = name.rsplit_once('.') {
            if !stem.is_empty() {
                return stem.chars().count() as i32;
            }
        }
    }
    name.chars().count() as i32
}

/// Valida y resuelve la ruta destino para renombrar un archivo o carpeta.
/// Devuelve `Ok(None)` si el nombre no cambió.
pub fn validate_rename_target(old_path: &Path, raw_new_name: &str) -> Result<Option<PathBuf>, String> {
    let trimmed = raw_new_name.trim();
    if trimmed.is_empty() {
        return Err("El nombre no puede estar vacío.".to_string());
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains('/') || trimmed.contains('\0') {
        return Err("El nombre contiene caracteres no válidos.".to_string());
    }

    let parent = old_path
        .parent()
        .ok_or_else(|| "No se encontró el directorio padre.".to_string())?;
    let current_name = old_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if trimmed == current_name {
        return Ok(None);
    }

    let dest = parent.join(trimmed);
    if dest.symlink_metadata().is_ok() {
        return Err(format!("Ya existe «{}» en esta carpeta.", trimmed));
    }

    Ok(Some(dest))
}

/// Renombra un archivo, carpeta o icono y migra sus metadatos en SQLite (posición espacial, vistas y etiquetas).
pub fn rename_item(old_path: &Path, raw_new_name: &str) -> Result<PathBuf, String> {
    let Some(dest) = validate_rename_target(old_path, raw_new_name)? else {
        return Ok(old_path.to_path_buf());
    };

    let is_dir = old_path
        .symlink_metadata()
        .map(|m| m.is_dir())
        .unwrap_or_else(|_| old_path.is_dir());

    fs::rename(old_path, &dest).map_err(|e| format!("No se pudo renombrar: {}", e))?;

    if let Ok(mut conn) = crate::db::open_db() {
        let _ = crate::db::rename_item_metadata(&mut conn, old_path, &dest, is_dir);
    }

    Ok(dest)
}

/// Calcula una ruta única en `folder` sin sobrescribir archivos, carpetas o enlaces existentes.
pub fn unique_target_path(folder: &Path, stem: &str, ext: Option<&str>) -> PathBuf {
    let format_name = |suffix: Option<usize>| -> String {
        let numbered = match suffix {
            Some(n) => format!("{} {}", stem, n),
            None => stem.to_string(),
        };
        match ext {
            Some(e) if !e.is_empty() => format!("{}.{}", numbered, e),
            _ => numbered,
        }
    };

    let mut candidate = folder.join(format_name(None));
    let mut counter = 2usize;
    while candidate.exists() || candidate.symlink_metadata().is_ok() {
        candidate = folder.join(format_name(Some(counter)));
        counter += 1;
    }
    candidate
}

/// Crea una carpeta nueva con nombre único dentro de `folder`.
pub fn create_new_folder(folder: &Path, base_name: &str) -> Result<PathBuf, String> {
    let target = unique_target_path(folder, base_name, None);
    fs::create_dir(&target).map_err(|e| format!("No se pudo crear la carpeta: {}", e))?;
    Ok(target)
}

/// Crea un documento de texto plano vacío (`.txt`) con nombre único dentro de `folder`.
pub fn create_new_text_document(folder: &Path, base_stem: &str) -> Result<PathBuf, String> {
    let target = unique_target_path(folder, base_stem, Some("txt"));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| format!("No se pudo crear el documento de texto: {}", e))?;
    Ok(target)
}

/// Mueve un conjunto de rutas hacia `dest_dir` (o hacia la Papelera si `dest_dir` termina en `Trash/files`),
/// protegiendo contra mover un elemento sobre su propio padre o dentro de sí mismo.
pub fn move_paths_into_folder(paths: &[PathBuf], dest_dir: &Path) -> bool {
    let is_trash_dest = dest_dir.ends_with("Trash/files");
    if !is_trash_dest && !dest_dir.is_dir() {
        return false;
    }

    let mut moved_any = false;
    let mut moved_pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for src in paths {
        if is_trash_dest {
            if src.parent() != Some(dest_dir) && trash_item(src).is_ok() {
                moved_any = true;
            }
            continue;
        }

        if src == dest_dir
            || src.parent() == Some(dest_dir)
            || (src.is_dir() && dest_dir.starts_with(src))
        {
            continue;
        }

        let Some(fname) = src.file_name() else {
            continue;
        };
        let target = dest_dir.join(fname);
        if target.exists() || target.symlink_metadata().is_ok() {
            continue;
        }

        let is_dir = src
            .symlink_metadata()
            .map(|m| m.is_dir())
            .unwrap_or_else(|_| src.is_dir());
        let ok = if malus_de_common::apps::is_application_desktop_file(src) {
            std::os::unix::fs::symlink(src, &target).is_ok()
        } else {
            let src_file = gio::File::for_path(src);
            let dest_file = gio::File::for_path(&target);
            src_file
                .move_(
                    &dest_file,
                    gio::FileCopyFlags::NONE,
                    gio::Cancellable::NONE,
                    None,
                )
                .is_ok()
                || fs::rename(src, &target).is_ok()
        };

        if ok {
            if let Ok(mut conn) = crate::db::open_db() {
                let _ = crate::db::rename_item_metadata(&mut conn, src, &target, is_dir);
            }
            moved_pairs.push((src.clone(), target));
            moved_any = true;
        }
    }

    // Si los elementos arrastrados provienen de una vista espacial (con anclas relativas),
    // conservar su disposición espacial relativa dentro de la carpeta destino.
    if !is_trash_dest && !moved_pairs.is_empty() {
        let anchored: Vec<(String, (f64, f64))> = moved_pairs
            .iter()
            .filter_map(|(src, dst)| {
                let name = dst.file_name()?.to_string_lossy().into_owned();
                let grab = malus_de_common::read_drag_anchor(src)?;
                Some((name, grab))
            })
            .collect();
        if anchored.len() > 1 {
            // La posición de cada icono respecto al cursor era (-grab.0, -grab.1).
            // Normalizamos respecto a la esquina superior izquierda del conjunto (min_x, min_y) + margen (24, 24).
            let min_rel_x = anchored
                .iter()
                .map(|(_, (gx, _))| -*gx)
                .fold(f64::INFINITY, f64::min);
            let min_rel_y = anchored
                .iter()
                .map(|(_, (_, gy))| -*gy)
                .fold(f64::INFINITY, f64::min);
            if let Ok(conn) = crate::db::open_db() {
                let dest_mode = crate::db::get_layout_mode(&conn, dest_dir);
                if dest_mode == crate::db::LayoutMode::Auto {
                    let _ = crate::db::set_layout_mode(
                        &conn,
                        dest_dir,
                        crate::db::LayoutMode::Freeform,
                    );
                }
                for (name, (gx, gy)) in anchored {
                    let px = (24.0 + (-gx - min_rel_x)).round().max(24.0) as i32;
                    let py = (24.0 + (-gy - min_rel_y)).round().max(24.0) as i32;
                    let _ = crate::db::save_position(&conn, dest_dir, &name, px, py);
                }
            }
        }
    }

    moved_any
}

/// Mueve archivos soltados vía GTK `DropTarget` hacia `dest_dir`.
pub fn drop_files_into_folder(files: &[gio::File], dest_dir: &Path) -> bool {
    let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
    move_paths_into_folder(&paths, dest_dir)
}

/// Copia recursiva de directorios
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("Error al crear carpeta destino: {}", e))?;
    let entries = fs::read_dir(src).map_err(|e| format!("Error al leer directorio: {}", e))?;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let target_path = dst.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &target_path)?;
        } else {
            fs::copy(&entry_path, &target_path)
                .map_err(|e| format!("Error al copiar fichero {}: {}", entry_path.display(), e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        create_new_folder, create_new_text_document, empty_trash_in, encode_gnome_copied_files,
        move_paths_into_folder, parse_gnome_copied_files, parse_trash_info_in, parse_uri_list,
        paste_payload_into, rename_item, rename_selection_end, restore_from_trash_in,
        validate_rename_target, ClipboardAction, ClipboardPayload,
    };
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn rename_selection_end_keeps_extension_unselected_for_files() {
        assert_eq!(rename_selection_end("documento.pdf", false), 9);
        assert_eq!(rename_selection_end(".gitignore", false), 10);
        assert_eq!(rename_selection_end("Mi Carpeta.v1", true), 13);
        assert_eq!(rename_selection_end("Makefile", false), 8);
    }

    #[test]
    fn validate_and_rename_item_handles_folders_and_conflicts() {
        let dir = std::env::temp_dir().join("malus_branches_rename_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Original")).unwrap();
        fs::write(dir.join("Existente.txt"), "hola").unwrap();

        assert!(validate_rename_target(&dir.join("Original"), "   ").is_err());
        assert!(validate_rename_target(&dir.join("Original"), "a/b").is_err());
        assert!(validate_rename_target(&dir.join("Original"), "Existente.txt").is_err());
        assert_eq!(
            validate_rename_target(&dir.join("Original"), "Original").unwrap(),
            None
        );

        let renamed = rename_item(&dir.join("Original"), "  Renombrada  ").unwrap();
        assert_eq!(renamed, dir.join("Renombrada"));
        assert!(!dir.join("Original").exists());
        assert!(renamed.is_dir());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_new_folder_and_text_document_generate_unique_names() {
        let dir = std::env::temp_dir().join("malus_branches_create_new_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let f1 = create_new_folder(&dir, "Nueva carpeta").unwrap();
        let f2 = create_new_folder(&dir, "Nueva carpeta").unwrap();
        assert_eq!(f1, dir.join("Nueva carpeta"));
        assert_eq!(f2, dir.join("Nueva carpeta 2"));
        assert!(f1.is_dir() && f2.is_dir());

        let d1 = create_new_text_document(&dir, "Nuevo documento").unwrap();
        let d2 = create_new_text_document(&dir, "Nuevo documento").unwrap();
        assert_eq!(d1, dir.join("Nuevo documento.txt"));
        assert_eq!(d2, dir.join("Nuevo documento 2.txt"));
        assert!(d1.is_file() && d2.is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_paths_into_folder_moves_files_and_guards_descendants() {
        let dir = std::env::temp_dir().join("malus_branches_move_drop_test");
        let _ = fs::remove_dir_all(&dir);
        let src_dir = dir.join("origen");
        let dest_dir = dir.join("destino");
        let nested = src_dir.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        let file = src_dir.join("nota.txt");
        fs::write(&file, "abc").unwrap();

        // No debe mover carpeta dentro de su propia subcarpeta ni archivo al mismo padre
        assert!(!move_paths_into_folder(&[src_dir.clone()], &nested));
        assert!(!move_paths_into_folder(&[file.clone()], &src_dir));

        // Mover archivo hacia destino debe funcionar
        assert!(move_paths_into_folder(&[file.clone()], &dest_dir));
        assert!(!file.exists());
        assert!(dest_dir.join("nota.txt").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gnome_clipboard_roundtrip_and_uri_list_parsing() {
        let paths = vec![
            PathBuf::from("/tmp/Mi Carpeta/archivo con espacios.txt"),
            PathBuf::from("/tmp/otra"),
        ];
        let encoded_cut = encode_gnome_copied_files(&ClipboardAction::Cut, &paths);
        let parsed_cut = parse_gnome_copied_files(&encoded_cut).unwrap();
        assert_eq!(parsed_cut.action, ClipboardAction::Cut);
        assert_eq!(parsed_cut.paths, paths);

        let uri_list = "file:///tmp/Mi%20Carpeta/archivo%20con%20espacios.txt\r\nfile:///tmp/otra\r\n";
        let parsed_uri = parse_uri_list(uri_list).unwrap();
        assert_eq!(parsed_uri.action, ClipboardAction::Copy);
        assert_eq!(parsed_uri.paths, paths);
    }

    #[test]
    fn paste_payload_copy_and_cut_handle_conflicts_and_self_descendants() {
        let dir = std::env::temp_dir().join("malus_branches_clipboard_paste_test");
        let _ = fs::remove_dir_all(&dir);
        let src_dir = dir.join("origen");
        let dest_dir = dir.join("destino");
        let sub_dir = src_dir.join("sub.v1");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();

        let src_file = src_dir.join("informe.txt");
        fs::write(&src_file, "contenido").unwrap();
        fs::write(sub_dir.join("interno.txt"), "dentro").unwrap();

        // Copiar archivo a la misma carpeta genera sufijo copia
        let copied_same = paste_payload_into(
            &ClipboardPayload {
                action: ClipboardAction::Copy,
                paths: vec![src_file.clone()],
            },
            &src_dir,
        )
        .unwrap();
        assert_eq!(copied_same, vec![src_dir.join("informe copia.txt")]);
        assert!(src_dir.join("informe.txt").is_file());
        assert!(src_dir.join("informe copia.txt").is_file());

        // Copiar carpeta sobre subcarpeta hija no entra en recursión infinita
        let self_copy = paste_payload_into(
            &ClipboardPayload {
                action: ClipboardAction::Copy,
                paths: vec![src_dir.clone()],
            },
            &sub_dir,
        )
        .unwrap();
        assert!(self_copy.is_empty());

        // Cortar dentro del mismo directorio padre es no-op
        let same_cut = paste_payload_into(
            &ClipboardPayload {
                action: ClipboardAction::Cut,
                paths: vec![src_file.clone()],
            },
            &src_dir,
        )
        .unwrap();
        assert!(same_cut.is_empty());
        assert!(src_file.is_file());

        // Cortar archivo y carpeta hacia destino los mueve y preserva contenido
        let moved = paste_payload_into(
            &ClipboardPayload {
                action: ClipboardAction::Cut,
                paths: vec![src_file.clone(), sub_dir.clone()],
            },
            &dest_dir,
        )
        .unwrap();
        assert_eq!(
            moved,
            vec![dest_dir.join("informe.txt"), dest_dir.join("sub.v1")]
        );
        assert!(!src_file.exists());
        assert!(!sub_dir.exists());
        assert!(dest_dir.join("informe.txt").is_file());
        assert!(dest_dir.join("sub.v1/interno.txt").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trash_info_restore_and_empty_work_correctly() {
        let root = std::env::temp_dir().join("malus_branches_trash_test");
        let _ = fs::remove_dir_all(&root);
        let trash_base = root.join("Trash");
        let files_dir = trash_base.join("files");
        let info_dir = trash_base.join("info");
        let orig_dir = root.join("docs con espacio");
        fs::create_dir_all(&files_dir).unwrap();
        fs::create_dir_all(&info_dir).unwrap();
        fs::create_dir_all(&orig_dir).unwrap();

        let trashed_file = files_dir.join("informe final.txt");
        fs::write(&trashed_file, b"recuperable").unwrap();

        let orig_target = orig_dir.join("informe final.txt");
        let encoded_orig = orig_target
            .to_string_lossy()
            .replace(' ', "%20");
        let info_content = format!(
            "[Trash Info]\nPath={}\nDeletionDate=2026-11-19T08:00:00\n",
            encoded_orig
        );
        fs::write(info_dir.join("informe final.txt.trashinfo"), info_content).unwrap();

        let parsed = parse_trash_info_in(&trash_base, &trashed_file).unwrap();
        assert_eq!(parsed.original_path, orig_target);
        assert_eq!(parsed.deletion_date.as_deref(), Some("2026-11-19T08:00:00"));

        let restored = restore_from_trash_in(&trash_base, &trashed_file).unwrap();
        assert_eq!(restored, orig_target);
        assert!(!trashed_file.exists());
        assert!(orig_target.is_file());
        assert!(!info_dir.join("informe final.txt.trashinfo").exists());

        // Probar vaciado de papelera
        fs::write(files_dir.join("otro.txt"), b"basura").unwrap();
        fs::write(info_dir.join("otro.txt.trashinfo"), "[Trash Info]\nPath=/tmp/otro.txt\n").unwrap();
        let emptied = empty_trash_in(&trash_base).unwrap();
        assert_eq!(emptied, 1);
        assert!(!files_dir.join("otro.txt").exists());
        assert!(!info_dir.join("otro.txt.trashinfo").exists());

        let _ = fs::remove_dir_all(&root);
    }
}
