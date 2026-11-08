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
            layout_mode TEXT NOT NULL DEFAULT 'auto'
        );
        CREATE TABLE IF NOT EXISTS file_tags (
            file_path TEXT NOT NULL,
            tag_name TEXT NOT NULL,
            tag_color TEXT NOT NULL,
            PRIMARY KEY (file_path, tag_name)
        );",
    )?;

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

