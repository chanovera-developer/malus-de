use gio::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardAction {
    Copy,
    Cut,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ClipboardPayload {
    pub action: ClipboardAction,
    pub paths: Vec<PathBuf>,
}

/// Enviar elemento a la Papelera mediante GIO (cumple especificación FreeDesktop Trash)
pub fn trash_item(path: &Path) -> Result<(), String> {
    let file = gio::File::for_path(path);
    file.trash(gio::Cancellable::NONE)
        .map_err(|e| format!("Error al mover a papelera: {}", e))
}

#[allow(dead_code)]
/// Eliminar permanentemente (sin pasar por la papelera)
pub fn delete_permanent(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("Error al eliminar carpeta: {}", e))
    } else {
        fs::remove_file(path).map_err(|e| format!("Error al eliminar archivo: {}", e))
    }
}

/// Duplicar archivo o carpeta al estilo macOS Finder ("nombre copia.ext", "nombre copia 2.ext")
pub fn duplicate_item(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "No se encontró directorio padre".to_string())?;

    let file_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "archivo".to_string());

    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let mut candidate = parent.join(format!("{} copia{}", file_stem, ext));
    let mut counter = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{} copia {}{}", file_stem, counter, ext));
        counter += 1;
    }

    if path.is_dir() {
        copy_dir_recursive(path, &candidate)?;
    } else {
        fs::copy(path, &candidate)
            .map_err(|e| format!("Error al duplicar archivo: {}", e))?;
    }

    Ok(candidate)
}

#[allow(dead_code)]
/// Copiar archivo o carpeta a una carpeta destino
pub fn copy_into(src: &Path, dest_folder: &Path) -> Result<PathBuf, String> {
    let file_name = src
        .file_name()
        .ok_or_else(|| "Nombre de archivo inválido".to_string())?;

    let mut dest = dest_folder.join(file_name);
    if dest.exists() {
        // Si ya existe en destino, aplicar sufijo copia
        let file_stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = src
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut counter = 2;
        dest = dest_folder.join(format!("{} copia{}", file_stem, ext));
        while dest.exists() {
            dest = dest_folder.join(format!("{} copia {}{}", file_stem, counter, ext));
            counter += 1;
        }
    }

    if src.is_dir() {
        copy_dir_recursive(src, &dest)?;
    } else {
        fs::copy(src, &dest).map_err(|e| format!("Error al copiar: {}", e))?;
    }

    Ok(dest)
}

#[allow(dead_code)]
/// Mover archivo o carpeta a una carpeta destino
pub fn move_into(src: &Path, dest_folder: &Path) -> Result<PathBuf, String> {
    let file_name = src
        .file_name()
        .ok_or_else(|| "Nombre de archivo inválido".to_string())?;

    let dest = dest_folder.join(file_name);
    fs::rename(src, &dest).map_err(|e| format!("Error al mover: {}", e))?;
    Ok(dest)
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
