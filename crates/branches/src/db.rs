use gio::prelude::FileExt;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagDefinition {
    pub id: &'static str,
    pub color_hex: &'static str,
    pub label_es: &'static str,
    pub label_en: &'static str,
}

pub const TAG_CATALOG: [TagDefinition; 7] = [
    TagDefinition {
        id: "red",
        color_hex: "#ff3b30",
        label_es: "Rojo",
        label_en: "Red",
    },
    TagDefinition {
        id: "orange",
        color_hex: "#ff9500",
        label_es: "Naranja",
        label_en: "Orange",
    },
    TagDefinition {
        id: "yellow",
        color_hex: "#ffcc00",
        label_es: "Amarillo",
        label_en: "Yellow",
    },
    TagDefinition {
        id: "green",
        color_hex: "#34c759",
        label_es: "Verde",
        label_en: "Green",
    },
    TagDefinition {
        id: "blue",
        color_hex: "#007aff",
        label_es: "Azul",
        label_en: "Blue",
    },
    TagDefinition {
        id: "purple",
        color_hex: "#af52de",
        label_es: "Púrpura",
        label_en: "Purple",
    },
    TagDefinition {
        id: "gray",
        color_hex: "#8e8e93",
        label_es: "Gris",
        label_en: "Gray",
    },
];

pub fn canonical_tag_id(raw: &str) -> String {
    let norm = raw.trim().to_lowercase();
    match norm.as_str() {
        "red" | "rojo" => "red".to_string(),
        "orange" | "naranja" => "orange".to_string(),
        "yellow" | "amarillo" => "yellow".to_string(),
        "green" | "verde" => "green".to_string(),
        "blue" | "azul" => "blue".to_string(),
        "purple" | "púrpura" | "purpura" | "morado" => "purple".to_string(),
        "gray" | "grey" | "gris" => "gray".to_string(),
        _ => norm,
    }
}

pub fn tag_definition(id_or_name: &str) -> Option<&'static TagDefinition> {
    let cid = canonical_tag_id(id_or_name);
    TAG_CATALOG.iter().find(|t| t.id == cid)
}

pub fn tag_display_label(id_or_name: &str, es: bool) -> String {
    if let Some(def) = tag_definition(id_or_name) {
        if es {
            def.label_es.to_string()
        } else {
            def.label_en.to_string()
        }
    } else {
        id_or_name.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Auto,
    SnapToGrid,
    Freeform,
}

impl LayoutMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            LayoutMode::Auto => "auto",
            LayoutMode::SnapToGrid => "snap_to_grid",
            LayoutMode::Freeform => "freeform",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "snap_to_grid" => LayoutMode::SnapToGrid,
            "freeform" => LayoutMode::Freeform,
            _ => LayoutMode::Auto,
        }
    }
}

pub fn get_db_path() -> PathBuf {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
            home.join(".local/share")
        });
    let malus_dir = data_dir.join("malus-de");
    let _ = fs::create_dir_all(&malus_dir);
    malus_dir.join("branches_views.sqlite")
}

pub fn open_db() -> Result<Connection, rusqlite::Error> {
    let path = get_db_path();
    let conn = Connection::open(path)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS folder_positions (
            folder_path TEXT NOT NULL,
            file_name TEXT NOT NULL,
            pos_x INTEGER NOT NULL,
            pos_y INTEGER NOT NULL,
            PRIMARY KEY (folder_path, file_name)
        );
        CREATE TABLE IF NOT EXISTS folder_settings (
            folder_path TEXT PRIMARY KEY,
            layout_mode TEXT NOT NULL DEFAULT 'auto',
            icon_size INTEGER
        );
        CREATE TABLE IF NOT EXISTS file_tags (
            file_path TEXT NOT NULL,
            tag_name TEXT NOT NULL,
            tag_color TEXT NOT NULL,
            PRIMARY KEY (file_path, tag_name)
        );
        CREATE TABLE IF NOT EXISTS sidebar_bookmarks (
            folder_path TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    // Migración no destructiva para bases existentes sin columna icon_size
    let _ = conn.execute("ALTER TABLE folder_settings ADD COLUMN icon_size INTEGER", []);

    // Migración v1: carpetas con snap_to_grid implícito de versiones previas pasan a freeform
    // para que los iconos permanezcan exactamente donde se sueltan (el usuario puede elegir SnapToGrid si lo desea).
    let user_ver: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    if user_ver < 1 {
        let _ = conn.execute(
            "UPDATE folder_settings SET layout_mode = 'freeform' WHERE layout_mode = 'snap_to_grid'",
            [],
        );
        let _ = conn.execute("PRAGMA user_version = 1", []);
    }

    // Migración v2: normalizar etiquetas localizadas previas ("Rojo", "Verde", "Azul"...) a IDs canónicos.
    if user_ver < 2 {
        migrate_legacy_tags(&conn);
        let _ = conn.execute("PRAGMA user_version = 2", []);
    }

    Ok(conn)
}

fn migrate_legacy_tags(conn: &Connection) {
    let rows: Vec<(String, String, String)> = {
        let Ok(mut stmt) = conn.prepare("SELECT file_path, tag_name, tag_color FROM file_tags") else {
            return;
        };
        let mapped = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        mapped.map(|it| it.flatten().collect()).unwrap_or_default()
    };

    for (file_path, old_name, old_color) in rows {
        let canonical = canonical_tag_id(&old_name);
        let color = tag_definition(&canonical)
            .map(|d| d.color_hex.to_string())
            .unwrap_or(old_color);
        if canonical != old_name {
            let _ = conn.execute(
                "DELETE FROM file_tags WHERE file_path = ?1 AND tag_name = ?2",
                params![file_path, old_name],
            );
            let _ = conn.execute(
                "INSERT INTO file_tags (file_path, tag_name, tag_color)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(file_path, tag_name) DO UPDATE SET tag_color = excluded.tag_color",
                params![file_path, canonical, color],
            );
        }
    }
}

pub fn get_positions(conn: &Connection, folder: &Path) -> HashMap<String, (i32, i32)> {
    let folder_str = folder.to_string_lossy();
    let mut map = HashMap::new();

    let mut stmt = match conn.prepare(
        "SELECT file_name, pos_x, pos_y FROM folder_positions WHERE folder_path = ?1",
    ) {
        Ok(s) => s,
        Err(_) => return map,
    };

    let rows = stmt.query_map(params![folder_str], |row| {
        let name: String = row.get(0)?;
        let x: i32 = row.get(1)?;
        let y: i32 = row.get(2)?;
        Ok((name, (x, y)))
    });

    if let Ok(iter) = rows {
        for item in iter.flatten() {
            map.insert(item.0, item.1);
        }
    }

    map
}

pub fn save_position(
    conn: &Connection,
    folder: &Path,
    file_name: &str,
    x: i32,
    y: i32,
) -> Result<(), rusqlite::Error> {
    let folder_str = folder.to_string_lossy();
    conn.execute(
        "INSERT INTO folder_positions (folder_path, file_name, pos_x, pos_y)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(folder_path, file_name) DO UPDATE SET pos_x = excluded.pos_x, pos_y = excluded.pos_y",
        params![folder_str, file_name, x, y],
    )?;
    Ok(())
}

pub fn save_all_positions(
    conn: &mut Connection,
    folder: &Path,
    positions: &HashMap<String, (i32, i32)>,
) -> Result<(), rusqlite::Error> {
    let folder_str = folder.to_string_lossy();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO folder_positions (folder_path, file_name, pos_x, pos_y)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(folder_path, file_name) DO UPDATE SET pos_x = excluded.pos_x, pos_y = excluded.pos_y",
        )?;

        for (name, (x, y)) in positions {
            stmt.execute(params![folder_str, name, x, y])?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[allow(dead_code)]
pub fn clear_positions(conn: &Connection, folder: &Path) -> Result<(), rusqlite::Error> {
    let folder_str = folder.to_string_lossy();
    conn.execute(
        "DELETE FROM folder_positions WHERE folder_path = ?1",
        params![folder_str],
    )?;
    Ok(())
}

pub fn get_layout_mode(conn: &Connection, folder: &Path) -> LayoutMode {
    let folder_str = folder.to_string_lossy();
    let mut stmt = match conn.prepare("SELECT layout_mode FROM folder_settings WHERE folder_path = ?1") {
        Ok(s) => s,
        Err(_) => return LayoutMode::Auto,
    };

    let mode_str: Result<String, _> = stmt.query_row(params![folder_str], |row| row.get(0));
    match mode_str {
        Ok(s) => LayoutMode::from_str(&s),
        Err(_) => LayoutMode::Auto,
    }
}

pub fn set_layout_mode(conn: &Connection, folder: &Path, mode: LayoutMode) -> Result<(), rusqlite::Error> {
    let folder_str = folder.to_string_lossy();
    conn.execute(
        "INSERT INTO folder_settings (folder_path, layout_mode)
         VALUES (?1, ?2)
         ON CONFLICT(folder_path) DO UPDATE SET layout_mode = excluded.layout_mode",
        params![folder_str, mode.as_str()],
    )?;
    Ok(())
}

/// Obtiene un tamaño de icono personalizado por carpeta (si el usuario lo sobrescribió respecto a `BranchesConfig`).
pub fn get_folder_icon_size(conn: &Connection, folder: &Path) -> Option<i32> {
    let folder_str = folder.to_string_lossy();
    let mut stmt = conn
        .prepare("SELECT icon_size FROM folder_settings WHERE folder_path = ?1")
        .ok()?;
    stmt.query_row(params![folder_str], |row| row.get::<_, Option<i32>>(0))
        .ok()
        .flatten()
}

/// Guarda o limpia (`None`) el tamaño de icono personalizado por carpeta en `folder_settings`.
#[allow(dead_code)]
pub fn set_folder_icon_size(
    conn: &Connection,
    folder: &Path,
    icon_size: Option<i32>,
) -> Result<(), rusqlite::Error> {
    let folder_str = folder.to_string_lossy();
    conn.execute(
        "INSERT INTO folder_settings (folder_path, icon_size)
         VALUES (?1, ?2)
         ON CONFLICT(folder_path) DO UPDATE SET icon_size = excluded.icon_size",
        params![folder_str, icon_size],
    )?;
    Ok(())
}

fn sync_xattr_tags(conn: &Connection, file_path: &Path) {
    if file_path.symlink_metadata().is_err() {
        return;
    }
    let tags = get_file_tags(conn, file_path);
    let mut ids: Vec<String> = tags.into_iter().map(|(id, _)| id).collect();
    ids.sort();
    ids.dedup();
    let val = ids.join(",");
    let gfile = gio::File::for_path(file_path);
    let _ = gfile.set_attribute_string(
        "xattr::user.malus.tags",
        &val,
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    );
}

#[allow(dead_code)]
pub fn read_xattr_tags(file_path: &Path) -> Vec<String> {
    if file_path.symlink_metadata().is_err() {
        return Vec::new();
    }
    let gfile = gio::File::for_path(file_path);
    let Ok(info) = gfile.query_info(
        "xattr::user.malus.tags",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) else {
        return Vec::new();
    };
    info.attribute_string("xattr::user.malus.tags")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(canonical_tag_id)
                .collect()
        })
        .unwrap_or_default()
}

pub fn set_file_tag(
    conn: &Connection,
    file_path: &Path,
    tag_name: &str,
    tag_color: &str,
) -> Result<(), rusqlite::Error> {
    let p_str = file_path.to_string_lossy();
    let canonical = canonical_tag_id(tag_name);
    let color = tag_definition(&canonical)
        .map(|d| d.color_hex)
        .unwrap_or(tag_color);
    conn.execute(
        "INSERT INTO file_tags (file_path, tag_name, tag_color)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(file_path, tag_name) DO UPDATE SET tag_color = excluded.tag_color",
        params![p_str, canonical, color],
    )?;
    sync_xattr_tags(conn, file_path);
    Ok(())
}

pub fn remove_file_tag(
    conn: &Connection,
    file_path: &Path,
    tag_name: &str,
) -> Result<(), rusqlite::Error> {
    let p_str = file_path.to_string_lossy();
    let canonical = canonical_tag_id(tag_name);
    conn.execute(
        "DELETE FROM file_tags WHERE file_path = ?1 AND (tag_name = ?2 OR tag_name = ?3)",
        params![p_str, canonical, tag_name],
    )?;
    sync_xattr_tags(conn, file_path);
    Ok(())
}

/// Alterna una etiqueta canónica en el archivo y devuelve `true` si quedó activada.
pub fn toggle_file_tag(
    conn: &Connection,
    file_path: &Path,
    tag_name: &str,
) -> Result<bool, rusqlite::Error> {
    let canonical = canonical_tag_id(tag_name);
    let existing = get_file_tags(conn, file_path);
    let has_tag = existing.iter().any(|(id, _)| id == &canonical);
    if has_tag {
        remove_file_tag(conn, file_path, &canonical)?;
        Ok(false)
    } else {
        let color = tag_definition(&canonical)
            .map(|d| d.color_hex)
            .unwrap_or("#8e8e93");
        set_file_tag(conn, file_path, &canonical, color)?;
        Ok(true)
    }
}

pub fn clear_file_tags(conn: &Connection, file_path: &Path) -> Result<(), rusqlite::Error> {
    let p_str = file_path.to_string_lossy();
    conn.execute("DELETE FROM file_tags WHERE file_path = ?1", params![p_str])?;
    sync_xattr_tags(conn, file_path);
    Ok(())
}

pub fn get_file_tags(conn: &Connection, file_path: &Path) -> Vec<(String, String)> {
    let p_str = file_path.to_string_lossy();
    let mut stmt = match conn.prepare("SELECT tag_name, tag_color FROM file_tags WHERE file_path = ?1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![p_str], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    });
    rows.map(|iter| iter.flatten().collect()).unwrap_or_default()
}

pub fn get_all_file_tags_map(conn: &Connection) -> HashMap<PathBuf, Vec<(String, String)>> {
    let mut map: HashMap<PathBuf, Vec<(String, String)>> = HashMap::new();
    let mut stmt = match conn.prepare("SELECT file_path, tag_name, tag_color FROM file_tags") {
        Ok(s) => s,
        Err(_) => return map,
    };
    if let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        for (path_str, tag_id, color) in rows.flatten() {
            let entry = map.entry(PathBuf::from(path_str)).or_default();
            if !entry.iter().any(|(id, _)| id == &tag_id) {
                entry.push((tag_id, color));
            }
        }
    }
    map
}

pub fn get_files_by_tag(conn: &Connection, tag_name: &str) -> Vec<PathBuf> {
    let canonical = canonical_tag_id(tag_name);
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT file_path FROM file_tags WHERE tag_name = ?1 OR lower(tag_name) = lower(?2)",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![canonical, tag_name], |row| {
        let p: String = row.get(0)?;
        Ok(PathBuf::from(p))
    });
    rows.map(|iter| iter.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkEntry {
    pub folder_path: PathBuf,
    pub title: String,
}

fn default_bookmark_title(folder: &Path) -> String {
    if folder == Path::new("/") {
        "Malus SSD".to_string()
    } else {
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| folder.to_string_lossy().into_owned())
    }
}

pub fn get_bookmarks(conn: &Connection) -> Vec<BookmarkEntry> {
    let mut stmt = match conn.prepare(
        "SELECT folder_path, title FROM sidebar_bookmarks ORDER BY sort_order ASC, rowid ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |row| {
        let p: String = row.get(0)?;
        let title: String = row.get(1)?;
        Ok(BookmarkEntry {
            folder_path: PathBuf::from(p),
            title,
        })
    });
    rows.map(|iter| iter.flatten().collect()).unwrap_or_default()
}

pub fn is_bookmark(conn: &Connection, folder: &Path) -> bool {
    let p_str = folder.to_string_lossy();
    conn.query_row(
        "SELECT 1 FROM sidebar_bookmarks WHERE folder_path = ?1 LIMIT 1",
        params![p_str],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn add_bookmark(
    conn: &Connection,
    folder: &Path,
    title: Option<&str>,
) -> Result<bool, rusqlite::Error> {
    let clean_title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_bookmark_title(folder));
    let p_str = folder.to_string_lossy();
    let next_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM sidebar_bookmarks",
            [],
            |row| row.get(0),
        )
        .unwrap_or(1);
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO sidebar_bookmarks (folder_path, title, sort_order)
         VALUES (?1, ?2, ?3)",
        params![p_str, clean_title, next_order],
    )?;
    Ok(inserted > 0)
}

pub fn remove_bookmark(conn: &Connection, folder: &Path) -> Result<bool, rusqlite::Error> {
    let p_str = folder.to_string_lossy();
    let removed = conn.execute(
        "DELETE FROM sidebar_bookmarks WHERE folder_path = ?1",
        params![p_str],
    )?;
    Ok(removed > 0)
}

/// Cambia únicamente el título visible de un marcador en el sidebar sin renombrar el elemento en disco.
pub fn rename_bookmark(
    conn: &Connection,
    folder: &Path,
    new_title: &str,
) -> Result<bool, rusqlite::Error> {
    let clean = new_title.trim();
    if clean.is_empty() {
        return Ok(false);
    }
    let p_str = folder.to_string_lossy();
    let updated = conn.execute(
        "UPDATE sidebar_bookmarks SET title = ?1 WHERE folder_path = ?2",
        params![clean, p_str],
    )?;
    Ok(updated > 0)
}

/// Migra posiciones, ajustes de carpeta y etiquetas cuando un archivo o carpeta cambia de nombre.
pub fn rename_item_metadata(
    conn: &mut Connection,
    old_path: &Path,
    new_path: &Path,
    is_dir: bool,
) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;

    if let (Some(parent), Some(old_name), Some(new_name)) = (
        old_path.parent(),
        old_path.file_name().map(|n| n.to_string_lossy().to_string()),
        new_path.file_name().map(|n| n.to_string_lossy().to_string()),
    ) {
        let parent_str = parent.to_string_lossy().to_string();
        tx.execute(
            "DELETE FROM folder_positions WHERE folder_path = ?1 AND file_name = ?2",
            params![parent_str, new_name],
        )?;
        tx.execute(
            "UPDATE folder_positions SET file_name = ?1 WHERE folder_path = ?2 AND file_name = ?3",
            params![new_name, parent_str, old_name],
        )?;
    }

    let old_str = old_path.to_string_lossy().to_string();
    let new_str = new_path.to_string_lossy().to_string();
    let old_leaf = old_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new_leaf = new_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    tx.execute(
        "UPDATE OR REPLACE file_tags SET file_path = ?1 WHERE file_path = ?2",
        params![new_str, old_str],
    )?;
    if !is_dir && old_str != new_str {
        let _ = tx.execute(
            "UPDATE OR REPLACE sidebar_bookmarks
             SET folder_path = ?1,
                 title = CASE WHEN title = ?2 THEN ?3 ELSE title END
             WHERE folder_path = ?4",
            params![new_str, old_leaf, new_leaf, old_str],
        );
    }

    if is_dir && old_str != new_str {
        let old_base = old_str.trim_end_matches('/');
        let new_base = new_str.trim_end_matches('/');
        let child_like = format!("{}/%", old_base);
        let prefix_len = old_base.chars().count() as i64;

        tx.execute(
            "UPDATE OR REPLACE folder_settings
             SET folder_path = CASE
                 WHEN folder_path = ?1 THEN ?2
                 ELSE ?2 || substr(folder_path, ?3 + 1)
             END
             WHERE folder_path = ?1 OR folder_path LIKE ?4",
            params![old_base, new_base, prefix_len, child_like],
        )?;
        tx.execute(
            "UPDATE OR REPLACE folder_positions
             SET folder_path = CASE
                 WHEN folder_path = ?1 THEN ?2
                 ELSE ?2 || substr(folder_path, ?3 + 1)
             END
             WHERE folder_path = ?1 OR folder_path LIKE ?4",
            params![old_base, new_base, prefix_len, child_like],
        )?;
        let _ = tx.execute(
            "UPDATE OR REPLACE sidebar_bookmarks
             SET folder_path = CASE
                 WHEN folder_path = ?1 THEN ?2
                 ELSE ?2 || substr(folder_path, ?3 + 1)
             END,
             title = CASE
                 WHEN folder_path = ?1 AND title = ?5 THEN ?6
                 ELSE title
             END
             WHERE folder_path = ?1 OR folder_path LIKE ?4",
            params![old_base, new_base, prefix_len, child_like, old_leaf, new_leaf],
        );
    }

    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        add_bookmark, clear_file_tags, get_bookmarks, get_file_tags, get_files_by_tag,
        get_folder_icon_size, get_layout_mode, get_positions, is_bookmark, migrate_legacy_tags,
        open_db, remove_bookmark, rename_bookmark, rename_item_metadata, save_position,
        set_folder_icon_size, set_layout_mode, toggle_file_tag, LayoutMode,
    };
    use rusqlite::{params, Connection};
    use std::fs;
    use std::path::Path;

    #[test]
    fn folder_icon_size_persists_and_clears_cleanly() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS folder_settings (
                folder_path TEXT PRIMARY KEY,
                layout_mode TEXT NOT NULL DEFAULT 'auto',
                icon_size INTEGER
            );",
        )
        .unwrap();
        let folder = Path::new("/tmp/test_branches_folder");
        assert_eq!(get_folder_icon_size(&conn, folder), None);
        set_folder_icon_size(&conn, folder, Some(80)).unwrap();
        assert_eq!(get_folder_icon_size(&conn, folder), Some(80));
        set_folder_icon_size(&conn, folder, None).unwrap();
        assert_eq!(get_folder_icon_size(&conn, folder), None);
        let _ = open_db;
    }

    #[test]
    fn rename_item_metadata_preserves_positions_and_subfolder_settings() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE folder_positions (
                folder_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                pos_x INTEGER NOT NULL,
                pos_y INTEGER NOT NULL,
                PRIMARY KEY (folder_path, file_name)
            );
            CREATE TABLE folder_settings (
                folder_path TEXT PRIMARY KEY,
                layout_mode TEXT NOT NULL DEFAULT 'auto',
                icon_size INTEGER
            );
            CREATE TABLE file_tags (
                file_path TEXT NOT NULL,
                tag_name TEXT NOT NULL,
                tag_color TEXT NOT NULL,
                PRIMARY KEY (file_path, tag_name)
            );",
        )
        .unwrap();

        let parent = Path::new("/tmp/parent");
        let old_dir = parent.join("Antes");
        let new_dir = parent.join("Después");
        save_position(&conn, parent, "Antes", 120, 240).unwrap();
        set_layout_mode(&conn, &old_dir, LayoutMode::Freeform).unwrap();
        save_position(&conn, &old_dir, "nota.txt", 40, 60).unwrap();

        rename_item_metadata(&mut conn, &old_dir, &new_dir, true).unwrap();

        let parent_positions = get_positions(&conn, parent);
        assert_eq!(parent_positions.get("Antes"), None);
        assert_eq!(parent_positions.get("Después"), Some(&(120, 240)));
        assert_eq!(get_layout_mode(&conn, &new_dir), LayoutMode::Freeform);
        assert_eq!(get_positions(&conn, &new_dir).get("nota.txt"), Some(&(40, 60)));
    }

    #[test]
    fn bookmarks_crud_and_rename_preserve_order() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE folder_positions (
                folder_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                pos_x INTEGER NOT NULL,
                pos_y INTEGER NOT NULL,
                PRIMARY KEY (folder_path, file_name)
            );
            CREATE TABLE folder_settings (
                folder_path TEXT PRIMARY KEY,
                layout_mode TEXT NOT NULL DEFAULT 'auto',
                icon_size INTEGER
            );
            CREATE TABLE file_tags (
                file_path TEXT NOT NULL,
                tag_name TEXT NOT NULL,
                tag_color TEXT NOT NULL,
                PRIMARY KEY (file_path, tag_name)
            );
            CREATE TABLE sidebar_bookmarks (
                folder_path TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();

        let p1 = Path::new("/home/cristian/Alfa");
        let p2 = Path::new("/home/cristian/Beta");
        assert!(add_bookmark(&conn, p1, None).unwrap());
        assert!(!add_bookmark(&conn, p1, None).unwrap());
        assert!(add_bookmark(&conn, p2, Some("Trabajo")).unwrap());
        assert!(is_bookmark(&conn, p1));

        let p1_renamed = Path::new("/home/cristian/Gamma");
        rename_item_metadata(&mut conn, p1, p1_renamed, true).unwrap();
        let list = get_bookmarks(&conn);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].folder_path, p1_renamed);
        assert_eq!(list[0].title, "Gamma");
        assert_eq!(list[1].folder_path, p2);
        assert_eq!(list[1].title, "Trabajo");

        assert!(rename_bookmark(&conn, p2, "  Proyecto X  ").unwrap());
        assert_eq!(get_bookmarks(&conn)[1].title, "Proyecto X");
        assert_eq!(get_bookmarks(&conn)[1].folder_path, p2);

        assert!(remove_bookmark(&conn, p1_renamed).unwrap());
        assert!(!is_bookmark(&conn, p1_renamed));
    }

    #[test]
    fn canonical_tags_migrate_legacy_rows_and_toggle_cleanly() {
        let temp_dir = std::env::temp_dir().join("malus_branches_tags_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let sample = temp_dir.join("doc.txt");
        fs::write(&sample, b"hola").unwrap();

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE file_tags (
                file_path TEXT NOT NULL,
                tag_name TEXT NOT NULL,
                tag_color TEXT NOT NULL,
                PRIMARY KEY (file_path, tag_name)
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_tags (file_path, tag_name, tag_color) VALUES (?1, 'Rojo', '#ff3b30')",
            params![sample.to_string_lossy()],
        )
        .unwrap();

        migrate_legacy_tags(&conn);
        let migrated = get_file_tags(&conn, &sample);
        assert_eq!(migrated, vec![("red".to_string(), "#ff3b30".to_string())]);

        // Activar segunda etiqueta y alternar la primera
        assert!(toggle_file_tag(&conn, &sample, "Azul").unwrap());
        assert_eq!(get_files_by_tag(&conn, "blue"), vec![sample.clone()]);
        assert_eq!(get_files_by_tag(&conn, "Azul"), vec![sample.clone()]);
        assert!(!toggle_file_tag(&conn, &sample, "Red").unwrap());

        let remaining = get_file_tags(&conn, &sample);
        assert_eq!(remaining, vec![("blue".to_string(), "#007aff".to_string())]);

        clear_file_tags(&conn, &sample).unwrap();
        assert!(get_file_tags(&conn, &sample).is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

