use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub name: String,
    pub exec: String,
    pub desktop_file: String,
    pub icon: String,
    pub comment: String,
}

pub fn index_applications() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let dirs = [
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        dirs_home_apps(),
    ];

    for dir in &dirs {
        if dir.exists() {
            scan_dir(dir, &mut apps);
        }
    }

    // Ordenar alfabéticamente por nombre
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

fn dirs_home_apps() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/applications")
    } else {
        PathBuf::from("/tmp")
    }
}

fn scan_dir(dir: &Path, apps: &mut Vec<AppEntry>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                if let Some(app) = parse_desktop_file(&path) {
                    apps.push(app);
                }
            }
        }
    }
}

fn parse_desktop_file(path: &Path) -> Option<AppEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut in_desktop_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut comment = None;
    let mut nodisplay = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }

        if !in_desktop_entry || line.starts_with('#') || !line.contains('=') {
            continue;
        }

        let mut parts = line.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();

        match key {
            "Name" if name.is_none() => name = Some(value.to_string()),
            "Exec" if exec.is_none() => {
                let clean_exec = value
                    .split_whitespace()
                    .filter(|part| !part.starts_with('%'))
                    .collect::<Vec<_>>()
                    .join(" ");
                exec = Some(clean_exec);
            }
            "Icon" if icon.is_none() => icon = Some(value.to_string()),
            "Comment" if comment.is_none() => comment = Some(value.to_string()),
            "NoDisplay" => nodisplay = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    if nodisplay {
        return None;
    }

    let name = name?;
    let exec = exec?;
    let icon = icon.unwrap_or_else(|| "application-x-executable".to_string());
    let comment = comment.unwrap_or_default();

    Some(AppEntry {
        name,
        exec,
        desktop_file: path.to_string_lossy().into_owned(),
        icon,
        comment,
    })
}
