use std::ffi::OsStr;
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
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS");
    let dirs = application_dirs_from(
        home.as_deref(),
        xdg_data_home.as_deref(),
        xdg_data_dirs.as_deref(),
    );

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

fn application_dirs_from(
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    xdg_data_dirs: Option<&OsStr>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let user_data_home = xdg_data_home
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|path| path.join(".local/share")));

    if let Some(data_home) = user_data_home {
        dirs.push(data_home.join("applications"));
    }

    if let Some(home) = home {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }

    let mut data_dirs: Vec<PathBuf> = xdg_data_dirs
        .filter(|dirs| !dirs.is_empty())
        .map(std::env::split_paths)
        .map(Iterator::collect)
        .unwrap_or_default();
    data_dirs.retain(|path| path.is_absolute());
    if data_dirs.is_empty() {
        data_dirs = vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ];
    }

    dirs.extend(data_dirs.into_iter().map(|path| path.join("applications")));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    let mut unique_dirs = Vec::with_capacity(dirs.len());
    for dir in dirs {
        if !unique_dirs.contains(&dir) {
            unique_dirs.push(dir);
        }
    }
    unique_dirs
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

#[cfg(test)]
mod tests {
    use super::application_dirs_from;
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    #[test]
    fn includes_flatpak_exports_and_xdg_application_dirs() {
        let home = Path::new("/home/test");
        let data_dirs = std::env::join_paths([
            "/opt/apps/share",
            "/usr/share",
            "/var/lib/flatpak/exports/share",
        ])
        .expect("valid XDG data directories");
        let dirs = application_dirs_from(
            Some(home),
            Some(Path::new("/home/test/.data")),
            Some(data_dirs.as_os_str()),
        );

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/home/test/.data/applications"),
                PathBuf::from("/home/test/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/opt/apps/share/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            ]
        );
    }

    #[test]
    fn uses_xdg_defaults_when_environment_values_are_missing_or_invalid() {
        let dirs = application_dirs_from(
            Some(Path::new("/home/test")),
            Some(Path::new("relative-data-home")),
            Some(OsStr::new("relative-data-dir")),
        );

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/home/test/.local/share/applications"),
                PathBuf::from("/home/test/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            ]
        );
    }
}
