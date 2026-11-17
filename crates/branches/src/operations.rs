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
        let src_file = gio::File::for_path(src);
        let dest_file = gio::File::for_path(&target);
        let ok = src_file
            .move_(
                &dest_file,
                gio::FileCopyFlags::NONE,
                gio::Cancellable::NONE,
                None,
            )
            .is_ok()
            || fs::rename(src, &target).is_ok();

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
        create_new_folder, create_new_text_document, move_paths_into_folder, rename_item,
        rename_selection_end, validate_rename_target,
    };
    use std::fs;

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
}
