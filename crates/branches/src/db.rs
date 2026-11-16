use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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

    Ok(conn)
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

pub fn set_file_tag(
    conn: &Connection,
    file_path: &Path,
    tag_name: &str,
    tag_color: &str,
) -> Result<(), rusqlite::Error> {
    let p_str = file_path.to_string_lossy();
    conn.execute(
        "INSERT INTO file_tags (file_path, tag_name, tag_color)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(file_path, tag_name) DO UPDATE SET tag_color = excluded.tag_color",
        params![p_str, tag_name, tag_color],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn remove_file_tag(
    conn: &Connection,
    file_path: &Path,
    tag_name: &str,
) -> Result<(), rusqlite::Error> {
    let p_str = file_path.to_string_lossy();
    conn.execute(
        "DELETE FROM file_tags WHERE file_path = ?1 AND tag_name = ?2",
        params![p_str, tag_name],
    )?;
    Ok(())
}

#[allow(dead_code)]
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

pub fn get_files_by_tag(conn: &Connection, tag_name: &str) -> Vec<PathBuf> {
    let mut stmt = match conn.prepare("SELECT file_path FROM file_tags WHERE tag_name = ?1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![tag_name], |row| {
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
        "Macintosh HD".to_string()
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
        add_bookmark, get_bookmarks, get_folder_icon_size, get_layout_mode, get_positions,
        is_bookmark, open_db, remove_bookmark, rename_bookmark, rename_item_metadata,
        save_position, set_folder_icon_size, set_layout_mode, LayoutMode,
    };
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
}

