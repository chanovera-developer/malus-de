//! Indexado y análisis compartido de archivos `.desktop` (XDG / Flatpak) para Spotlight, Branches y Dock.

use crate::config::{DockAppConfig, DockIconAppearance};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopAppEntry {
    pub name: String,
    pub exec: String,
    pub desktop_file: PathBuf,
    pub icon: String,
    pub comment: String,
    pub categories: String,
    pub search_terms: String,
    pub matchers: Vec<String>,
}

impl DesktopAppEntry {
    pub fn to_dock_app(&self) -> DockAppConfig {
        DockAppConfig {
            name: self.name.clone(),
            icon: self.icon.clone(),
            command: self.exec.clone(),
            matchers: self.matchers.clone(),
            icon_appearance: DockIconAppearance::Auto,
        }
    }
}

pub fn session_locale_codes() -> (String, String) {
    let raw = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let without_encoding = raw.split('.').next().unwrap_or("");
    let full = without_encoding.split('@').next().unwrap_or("").trim();
    let lang = full.split('_').next().unwrap_or("").trim();
    (full.to_string(), lang.to_string())
}

pub fn application_dirs_from(
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

pub fn build_app_matchers(name: &str, command: &str, desktop_file: &Path) -> Vec<String> {
    let mut matchers = Vec::new();
    let mut push_unique = |val: String| {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() && !matchers.iter().any(|m: &String| m.eq_ignore_ascii_case(&trimmed)) {
            matchers.push(trimmed);
        }
    };

    if let Some(stem) = desktop_file.file_stem().and_then(|s| s.to_str()) {
        push_unique(stem.to_string());
        push_unique(stem.to_lowercase());
    }
    if let Some(first_bin) = command
        .split_whitespace()
        .next()
        .and_then(|bin| Path::new(bin).file_name())
        .and_then(|s| s.to_str())
    {
        push_unique(first_bin.to_lowercase());
    }
    push_unique(name.to_lowercase());
    matchers
}

fn clean_exec_line(raw: &str) -> String {
    raw.split_whitespace()
        .filter(|part| !part.starts_with('%'))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_desktop_file_inner(path: &Path, include_hidden: bool) -> Option<DesktopAppEntry> {
    let content = fs::read_to_string(path).ok()?;
    let (full_locale, lang_code) = session_locale_codes();
    let name_full_key = format!("Name[{full_locale}]");
    let name_lang_key = format!("Name[{lang_code}]");
    let comment_full_key = format!("Comment[{full_locale}]");
    let comment_lang_key = format!("Comment[{lang_code}]");
    let keywords_full_key = format!("Keywords[{full_locale}]");
    let keywords_lang_key = format!("Keywords[{lang_code}]");

    let mut in_desktop_entry = false;
    let mut entry_type: Option<String> = None;
    let mut name_default = None;
    let mut name_lang = None;
    let mut name_full = None;
    let mut exec = None;
    let mut icon = None;
    let mut comment_default = None;
    let mut comment_lang = None;
    let mut comment_full = None;
    let mut categories = String::new();
    let mut search_terms = Vec::new();
    let mut nodisplay = false;
    let mut hidden = false;

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

        if key == "Type" && entry_type.is_none() {
            entry_type = Some(value.to_string());
        } else if key == "Name" && name_default.is_none() {
            name_default = Some(value.to_string());
            search_terms.push(value.to_lowercase());
        } else if !full_locale.is_empty() && key == name_full_key {
            name_full = Some(value.to_string());
            search_terms.push(value.to_lowercase());
        } else if !lang_code.is_empty() && key == name_lang_key {
            name_lang = Some(value.to_string());
            search_terms.push(value.to_lowercase());
        } else if key == "Exec" && exec.is_none() {
            exec = Some(clean_exec_line(value));
        } else if key == "Icon" && icon.is_none() {
            icon = Some(value.to_string());
        } else if key == "Comment" && comment_default.is_none() {
            comment_default = Some(value.to_string());
        } else if !full_locale.is_empty() && key == comment_full_key {
            comment_full = Some(value.to_string());
        } else if !lang_code.is_empty() && key == comment_lang_key {
            comment_lang = Some(value.to_string());
        } else if key == "Keywords"
            || (!full_locale.is_empty() && key == keywords_full_key)
            || (!lang_code.is_empty() && key == keywords_lang_key)
        {
            search_terms.push(value.to_lowercase());
        } else if key == "Categories" {
            categories = value.to_string();
        } else if key == "NoDisplay" {
            nodisplay = value.eq_ignore_ascii_case("true");
        } else if key == "Hidden" {
            hidden = value.eq_ignore_ascii_case("true");
        }
    }

    if let Some(t) = entry_type {
        if t != "Application" {
            return None;
        }
    }
    if !include_hidden && (nodisplay || hidden) {
        return None;
    }

    let name = name_full.or(name_lang).or(name_default)?;
    let exec = exec?;
    let icon = icon
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "application-x-executable".to_string());
    let comment = comment_full.or(comment_lang).or(comment_default).unwrap_or_default();
    let matchers = build_app_matchers(&name, &exec, path);

    Some(DesktopAppEntry {
        name,
        exec,
        desktop_file: path.to_path_buf(),
        icon,
        comment,
        categories,
        search_terms: search_terms.join(" "),
        matchers,
    })
}

pub fn parse_desktop_file(path: &Path) -> Option<DesktopAppEntry> {
    parse_desktop_file_inner(path, false)
}

pub fn parse_desktop_file_any(path: &Path) -> Option<DesktopAppEntry> {
    parse_desktop_file_inner(path, true)
}

pub fn desktop_file_to_dock_app(path: &Path) -> Option<DockAppConfig> {
    if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
        return None;
    }
    parse_desktop_file_any(path).map(|entry| entry.to_dock_app())
}

pub fn index_applications() -> Vec<DesktopAppEntry> {
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
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

fn push_unique_path(dirs: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !dirs.contains(&candidate) {
        dirs.push(candidate);
    }
}

pub fn extra_icon_search_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS");
    icon_root_dirs_from(home.as_deref(), xdg_data_home.as_deref(), xdg_data_dirs.as_deref())
}

fn icon_root_dirs_from(
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    xdg_data_dirs: Option<&OsStr>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        push_unique_path(&mut dirs, home.join(".local/share/icons"));
        push_unique_path(
            &mut dirs,
            home.join(".local/share/flatpak/exports/share/icons"),
        );
        push_unique_path(&mut dirs, home.join(".icons"));
    }
    if let Some(data_home) = xdg_data_home.filter(|p| p.is_absolute()) {
        push_unique_path(&mut dirs, data_home.join("icons"));
    }

    let mut data_dirs: Vec<PathBuf> = xdg_data_dirs
        .filter(|d| !d.is_empty())
        .map(std::env::split_paths)
        .map(Iterator::collect)
        .unwrap_or_default();
    data_dirs.retain(|p| p.is_absolute());
    if data_dirs.is_empty() {
        data_dirs = vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ];
    }
    for dir in data_dirs {
        push_unique_path(&mut dirs, dir.join("icons"));
    }
    push_unique_path(
        &mut dirs,
        PathBuf::from("/var/lib/flatpak/exports/share/icons"),
    );
    push_unique_path(&mut dirs, PathBuf::from("/usr/local/share/icons"));
    push_unique_path(&mut dirs, PathBuf::from("/usr/share/icons"));
    dirs
}

fn pixmap_dirs_from(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        push_unique_path(&mut dirs, home.join(".local/share/pixmaps"));
    }
    push_unique_path(&mut dirs, PathBuf::from("/usr/local/share/pixmaps"));
    push_unique_path(&mut dirs, PathBuf::from("/usr/share/pixmaps"));
    dirs
}

pub fn resolve_icon_file_path_in_dirs(
    icon_name: &str,
    icon_roots: &[PathBuf],
    pixmap_dirs: &[PathBuf],
    home: Option<&Path>,
) -> Option<PathBuf> {
    let trimmed = icon_name.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = home {
            let expanded = home.join(rest);
            if expanded.is_file() {
                return Some(expanded);
            }
        }
    }

    let direct = Path::new(trimmed);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct.to_path_buf());
    }

    let has_raster_or_svg_ext = matches!(
        direct.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()).as_deref(),
        Some("svg" | "png")
    );
    let stem = if has_raster_or_svg_ext {
        direct.file_stem().and_then(|s| s.to_str()).unwrap_or(trimmed)
    } else {
        trimmed
    };

    let subdirs: &[&str] = if stem.ends_with("-symbolic") {
        &[
            "scalable/apps",
            "symbolic/apps",
            "scalable/categories",
            "scalable/places",
            "scalable/status",
            "symbolic/status",
        ]
    } else {
        &[
            "scalable/apps",
            "512x512/apps",
            "256x256/apps",
            "128x128/apps",
            "96x96/apps",
            "64x64/apps",
            "48x48/apps",
            "32x32/apps",
            "24x24/apps",
            "16x16/apps",
            "scalable/categories",
            "scalable/places",
            "scalable/status",
            "48x48/devices",
        ]
    };

    let themes = ["hicolor", "Adwaita"];
    for root in icon_roots {
        for theme in &themes {
            let theme_dir = root.join(theme);
            if !theme_dir.is_dir() {
                continue;
            }
            for sub in subdirs {
                let sub_dir = theme_dir.join(sub);
                if has_raster_or_svg_ext {
                    let candidate = sub_dir.join(trimmed);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
                for ext in ["svg", "png"] {
                    let candidate = sub_dir.join(format!("{stem}.{ext}"));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    for pixmap_dir in pixmap_dirs {
        if has_raster_or_svg_ext {
            let candidate = pixmap_dir.join(trimmed);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        for ext in ["svg", "png"] {
            let candidate = pixmap_dir.join(format!("{stem}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Resuelve el archivo de icono (`.svg` o `.png`) para un nombre de icono de `.desktop`.
pub fn resolve_icon_file_path(icon_name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS");
    let icon_roots = icon_root_dirs_from(home.as_deref(), xdg_data_home.as_deref(), xdg_data_dirs.as_deref());
    let pixmap_dirs = pixmap_dirs_from(home.as_deref());
    resolve_icon_file_path_in_dirs(icon_name, &icon_roots, &pixmap_dirs, home.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_dirs_include_xdg_and_flatpak_exports() {
        let home = Path::new("/home/test");
        let data_dirs = std::env::join_paths([
            "/opt/apps/share",
            "/usr/share",
            "/var/lib/flatpak/exports/share",
        ])
        .unwrap();
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
    fn parse_desktop_file_respects_nodisplay_and_builds_dock_app() {
        let temp_dir = std::env::temp_dir().join("malus_common_apps_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let visible_path = temp_dir.join("org.example.Demo.desktop");
        fs::write(
            &visible_path,
            "[Desktop Entry]\nType=Application\nName=Demo App\nExec=/usr/bin/demo --flag %U\nIcon=demo-icon\nComment=Sample\n",
        )
        .unwrap();

        let hidden_path = temp_dir.join("hidden.desktop");
        fs::write(
            &hidden_path,
            "[Desktop Entry]\nType=Application\nName=Hidden Helper\nExec=/usr/bin/helper\nNoDisplay=true\n",
        )
        .unwrap();

        assert!(parse_desktop_file(&hidden_path).is_none());
        let entry = parse_desktop_file(&visible_path).expect("visible desktop entry");
        assert_eq!(entry.name, "Demo App");
        assert_eq!(entry.exec, "/usr/bin/demo --flag");
        assert_eq!(entry.icon, "demo-icon");

        let dock_app = desktop_file_to_dock_app(&visible_path).expect("dock app config");
        assert_eq!(dock_app.name, "Demo App");
        assert_eq!(dock_app.command, "/usr/bin/demo --flag");
        assert_eq!(dock_app.icon, "demo-icon");
        assert!(dock_app.matchers.iter().any(|m| m == "org.example.Demo"));
        assert!(dock_app.matchers.iter().any(|m| m == "demo"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn resolve_icon_file_path_finds_hicolor_flatpak_and_pixmap_icons() {
        let temp_dir = std::env::temp_dir().join("malus_common_icon_resolve_test");
        let _ = fs::remove_dir_all(&temp_dir);

        let flatpak_icons = temp_dir.join("flatpak/icons");
        let hicolor_svg = flatpak_icons.join("hicolor/scalable/apps");
        let pixmaps = temp_dir.join("pixmaps");
        fs::create_dir_all(&hicolor_svg).unwrap();
        fs::create_dir_all(&pixmaps).unwrap();

        let svg_icon = hicolor_svg.join("org.example.Flatpak.svg");
        let png_pixmap = pixmaps.join("vscode.png");
        fs::write(&svg_icon, b"<svg/>").unwrap();
        fs::write(&png_pixmap, b"png").unwrap();

        assert_eq!(
            resolve_icon_file_path_in_dirs(
                "org.example.Flatpak",
                &[flatpak_icons.clone()],
                &[pixmaps.clone()],
                None,
            ),
            Some(svg_icon)
        );
        assert_eq!(
            resolve_icon_file_path_in_dirs(
                "vscode",
                &[flatpak_icons.clone()],
                &[pixmaps.clone()],
                None,
            ),
            Some(png_pixmap.clone())
        );
        assert_eq!(
            resolve_icon_file_path_in_dirs(
                "vscode.png",
                &[flatpak_icons],
                &[pixmaps],
                None,
            ),
            Some(png_pixmap)
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

