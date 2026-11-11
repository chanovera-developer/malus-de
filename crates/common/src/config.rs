use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Configuración global de Malus DE
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalusConfig {
    pub appearance: AppearanceConfig,
    pub dock: DockConfig,
    pub top_bar: TopBarConfig,
    pub input: InputConfig,
    pub tiling: TilingConfig,
    pub link: LinkConfig,
    pub power: PowerConfig,
    pub displays: DisplaysConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    pub theme: String,
    pub dark_mode: bool,
    pub button_layout_left: bool,
    pub acrylic_blur: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "default-light".to_string(),
            dark_mode: false,
            button_layout_left: true,
            acrylic_blur: true,
        }
    }
}

impl AppearanceConfig {
    /// Formato de botones de ventana para GNOME / GTK:
    /// - Izquierda (macOS): `close,minimize,maximize:`
    /// - Derecha (Windows/GNOME): `:minimize,maximize,close`
    pub fn button_layout_string(&self) -> &'static str {
        if self.button_layout_left {
            "close,minimize,maximize:"
        } else {
            ":minimize,maximize,close"
        }
    }
}

/// Reglas CSS de controles de ventana macOS (Traffic Lights) para tema Claro
pub const MALUS_GTK_CSS_WINDOWCONTROLS_LIGHT: &str = include_str!("styles/windowcontrols-light.css");

/// Reglas CSS de controles de ventana macOS (Traffic Lights) para tema Oscuro
pub const MALUS_GTK_CSS_WINDOWCONTROLS_DARK: &str = include_str!("styles/windowcontrols-dark.css");

/// Reglas CSS de Malus DE para controles de ventana estilo macOS y recortes squircle
pub const MALUS_GTK_CSS_SQUIRCLE_RULES: &str = include_str!("styles/squircle-rules.css");

fn malus_gtk_css_popup(dark: bool) -> &'static str {
    if dark {
        include_str!("styles/popup-dark.css")
    } else {
        include_str!("styles/popup-light.css")
    }
}

pub const MALUS_GTK_CSS_SIDEBAR_COMMON: &str = include_str!("styles/sidebar-common.css");

pub const MALUS_GTK_CSS_SIDEBAR_LIGHT: &str = include_str!("styles/sidebar-light.css");

pub const MALUS_GTK_CSS_SIDEBAR_DARK: &str = include_str!("styles/sidebar-dark.css");

/// Reglas CSS de Mozilla Firefox para controles de ventana estilo semáforo macOS (`userChrome.css`)
pub const MALUS_FIREFOX_USER_CHROME_CSS: &str = include_str!("styles/firefox-userchrome.css");

/// Genera la hoja de estilos CSS completa de Malus DE (controles macOS + recortes squircle + paneles translúcidos) para GTK 4 / Libadwaita
pub fn get_malus_gtk_css(dark: bool) -> String {
    let (controls, sidebar) = if dark {
        (
            MALUS_GTK_CSS_WINDOWCONTROLS_DARK,
            MALUS_GTK_CSS_SIDEBAR_DARK,
        )
    } else {
        (
            MALUS_GTK_CSS_WINDOWCONTROLS_LIGHT,
            MALUS_GTK_CSS_SIDEBAR_LIGHT,
        )
    };
    format!(
        "{}\n{}\n{}\n{}\n{}",
        controls,
        MALUS_GTK_CSS_SQUIRCLE_RULES,
        MALUS_GTK_CSS_SIDEBAR_COMMON,
        malus_gtk_css_popup(dark),
        sidebar
    )
}

/// Hoja de estilos CSS nativa y válida para GTK 3 (Firefox, LibreOffice, GIMP, diálogos GTK3)
pub fn get_malus_gtk3_css(dark: bool) -> String {
    let (
        bg_color,
        fg_color,
        sidebar_bg,
        border_color,
        popup_bg,
        popup_fg,
        popup_sep,
    ) = if dark {
        (
            "rgb(30, 30, 34)",
            "rgb(255, 255, 255)",
            "rgb(30, 30, 34)",
            "rgba(255, 255, 255, 0.08)",
            "rgba(30, 30, 36, 0.78)",
            "#f4f4f5",
            "rgba(255, 255, 255, 0.10)",
        )
    } else {
        (
            "rgb(246, 246, 248)",
            "rgb(0, 0, 0)",
            "rgb(246, 246, 248)",
            "rgba(0, 0, 0, 0.08)",
            "rgba(244, 246, 250, 0.76)",
            "#1d1d1f",
            "rgba(0, 0, 0, 0.08)",
        )
    };

    include_str!("styles/gtk3.css")
        .replace("__BG_COLOR__", bg_color)
        .replace("__FG_COLOR__", fg_color)
        .replace("__SIDEBAR_BG__", sidebar_bg)
        .replace("__BORDER_COLOR__", border_color)
        .replace("__POPUP_BG__", popup_bg)
        .replace("__POPUP_FG__", popup_fg)
        .replace("__POPUP_SEP__", popup_sep)
}

fn set_gtk_decoration_layout(config_dir: &Path) -> std::io::Result<()> {
    const LAYOUT: &str = "gtk-decoration-layout=close,minimize,maximize:";

    for version in ["gtk-3.0", "gtk-4.0"] {
        let settings_path = config_dir.join(version).join("settings.ini");
        let content = match std::fs::read_to_string(&settings_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();

        let settings_start = lines.iter().position(|line| line.trim() == "[Settings]");
        let Some(mut start) = settings_start else {
            if !lines.is_empty() && !lines.last().map(|line| line.is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push("[Settings]".to_string());
            lines.push(LAYOUT.to_string());
            std::fs::write(settings_path, lines.join("\n") + "\n")?;
            continue;
        };

        start += 1;
        let end = lines[start..]
            .iter()
            .position(|line| line.trim().starts_with('['))
            .map(|offset| start + offset)
            .unwrap_or(lines.len());
        if let Some(index) = (start..end).find(|&index| {
            lines[index].trim_start().starts_with("gtk-decoration-layout=")
        }) {
            lines[index] = LAYOUT.to_string();
        } else {
            lines.insert(end, LAYOUT.to_string());
        }
        std::fs::write(settings_path, lines.join("\n") + "\n")?;
    }

    Ok(())
}

static ATOMIC_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Escribe un archivo de forma atómica (temporal en el mismo directorio + `rename`)
/// únicamente si el contenido difiere del actual. Devuelve `true` si el archivo cambió.
pub fn write_file_atomic_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return Ok(false);
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let seq = ATOMIC_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("malus");
    let tmp_path = path.with_file_name(format!(".{file_name}.tmp.{}.{seq}", std::process::id()));

    if let Err(err) = std::fs::write(&tmp_path, content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    Ok(true)
}

/// Actualiza o inserta pares `clave=valor` por sección dentro de un archivo INI/keyfile
/// y lo persiste atómicamente solo cuando hubo cambios reales.
pub fn upsert_keyfile_entries(
    keyfile_path: &Path,
    updates: &[(&str, &[(&str, &str)])],
) -> std::io::Result<bool> {
    let existing = match std::fs::read_to_string(keyfile_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };

    let mut sections: Vec<(Option<String>, Vec<String>)> = Vec::new();
    let mut current_header: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
            if current_header.is_some() || !current_lines.is_empty() {
                sections.push((current_header.take(), std::mem::take(&mut current_lines)));
            }
            current_header = Some(trimmed[1..trimmed.len() - 1].to_string());
        } else {
            current_lines.push(line.to_string());
        }
    }
    if current_header.is_some() || !current_lines.is_empty() {
        sections.push((current_header, current_lines));
    }

    for &(section_name, kv_pairs) in updates {
        let target_idx = sections
            .iter()
            .position(|(hdr, _)| hdr.as_deref() == Some(section_name));

        let idx = match target_idx {
            Some(i) => i,
            None => {
                if let Some((_, prev_lines)) = sections.last_mut() {
                    if !prev_lines.last().is_some_and(|l| l.trim().is_empty()) {
                        prev_lines.push(String::new());
                    }
                }
                sections.push((Some(section_name.to_string()), Vec::new()));
                sections.len() - 1
            }
        };

        let lines = &mut sections[idx].1;
        for &(key, val) in kv_pairs {
            let formatted = format!("{key}={val}");
            if let Some(pos) = lines.iter().position(|l| {
                let t = l.trim_start();
                t.strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with('='))
            }) {
                lines[pos] = formatted;
            } else {
                let insert_at = lines
                    .iter()
                    .rposition(|l| !l.trim().is_empty())
                    .map_or(0, |last_non_empty| last_non_empty + 1);
                lines.insert(insert_at, formatted);
            }
        }
    }

    let mut out = Vec::new();
    for (hdr, lines) in sections {
        if let Some(name) = hdr {
            out.push(format!("[{name}]"));
        }
        out.extend(lines);
    }

    let content = if out.is_empty() {
        String::new()
    } else {
        out.join("\n") + "\n"
    };

    write_file_atomic_if_changed(keyfile_path, &content)
}

fn update_gtk_ini_appearance(
    ini_path: &Path,
    dark_val: &str,
    gtk_theme: &str,
) -> std::io::Result<bool> {
    upsert_keyfile_entries(
        ini_path,
        &[(
            "Settings",
            &[
                ("gtk-decoration-layout", "close,minimize,maximize:"),
                ("gtk-application-prefer-dark-theme", dark_val),
                ("gtk-theme-name", gtk_theme),
            ],
        )],
    )
}

fn propagate_isolated_portal_environment_once(malus_config_dir: &Path) {
    if std::env::var_os("MALUS_ISOLATED_ENV_READY").is_some() {
        return;
    }

    let sentinel_path = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join("malus-isolated-env-ready"));

    if sentinel_path.as_ref().is_some_and(|p| p.exists()) {
        std::env::set_var("MALUS_ISOLATED_ENV_READY", "1");
        return;
    }

    std::env::set_var("MALUS_ISOLATED_ENV_READY", "1");
    if let Some(ref sentinel) = sentinel_path {
        let _ = std::fs::write(sentinel, b"1\n");
    }

    let config_dir = malus_config_dir.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("malus-portal-env".to_string())
        .spawn(move || {
            let _ = std::process::Command::new("dbus-update-activation-environment")
                .args([
                    "--systemd",
                    &format!("XDG_CONFIG_HOME={}", config_dir.display()),
                    "GSETTINGS_BACKEND=keyfile",
                ])
                .status();
            let _ = std::process::Command::new("systemctl")
                .args([
                    "--user",
                    "import-environment",
                    "XDG_CONFIG_HOME",
                    "GSETTINGS_BACKEND",
                ])
                .status();
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "unset-environment", "GTK_USE_PORTAL"])
                .status();
            let _ = std::process::Command::new("systemctl")
                .args([
                    "--user",
                    "restart",
                    "xdg-desktop-portal-gtk",
                    "xdg-desktop-portal",
                ])
                .status();
        });
}

pub fn ensure_keyfile_button_layout(keyfile_path: &Path) -> std::io::Result<()> {
    if !keyfile_path.exists() {
        return Ok(());
    }
    upsert_keyfile_entries(
        keyfile_path,
        &[(
            "org/gnome/desktop/wm/preferences",
            &[("button-layout", "'close,minimize,maximize:'")],
        )],
    )
    .map(|_| ())
}

pub fn set_gsettings_button_layout(config_dir: &Path) {
    let keyfile_path = config_dir.join("glib-2.0/settings/keyfile");
    if let Err(error) = ensure_keyfile_button_layout(&keyfile_path) {
        tracing::warn!("No se pudo escribir el layout de botones en keyfile: {error}");
    }
}

/// Inicializa el entorno de configuración aislado para Malus DE en `$HOME/.config/malus-de`.
/// Configura el backend `keyfile` de GSettings y los archivos `settings.ini` de GTK3/GTK4
/// con el layout de botones a la macOS (`close,minimize,maximize:`) sin alterar dconf del host.
pub fn setup_isolated_session_environment() -> std::io::Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = PathBuf::from(&home).join(".config").join("malus-de");

    std::env::set_var("XDG_CONFIG_HOME", &malus_config_dir);
    std::env::set_var("GSETTINGS_BACKEND", "keyfile");
    std::env::remove_var("GTK_USE_PORTAL");

    // 1. Directorio glib-2.0/settings para el backend GSettings keyfile (preservar si ya existe)
    let keyfile_dir = malus_config_dir.join("glib-2.0").join("settings");
    std::fs::create_dir_all(&keyfile_dir)?;
    let keyfile_path = keyfile_dir.join("keyfile");
    if !keyfile_path.exists() {
        let keyfile_content = r#"[org/gnome/desktop/wm/preferences]
button-layout='close,minimize,maximize:'

[org/gnome/desktop/interface]
color-scheme='prefer-dark'
cursor-theme='macOS'
cursor-size=24
icon-theme='Adwaita'
gtk-theme='Adwaita'

[org/gnome/settings-daemon/plugins/xsettings]
overrides={'Gtk/ShellShowsAppMenu': <1>, 'Gtk/ShellShowsMenubar': <1>}
"#;
        write_file_atomic_if_changed(&keyfile_path, keyfile_content)?;
    } else {
        let _ = ensure_keyfile_button_layout(&keyfile_path);
    }

    // 1.1. Configuración de portales para Malus DE (~/.config/malus-de/xdg-desktop-portal/portals.conf)
    // Dirigir Settings hacia xdg-desktop-portal-gtk para evitar contaminación desde el host GNOME.
    let portal_dir = malus_config_dir.join("xdg-desktop-portal");
    let _ = std::fs::create_dir_all(&portal_dir);
    let portal_content = r#"[preferred]
default=gtk;gnome;
org.freedesktop.impl.portal.Settings=gtk;gnome;
"#;
    let _ = write_file_atomic_if_changed(&portal_dir.join("portals.conf"), portal_content);
    let _ = write_file_atomic_if_changed(&portal_dir.join("malus-portals.conf"), portal_content);
    let _ = write_file_atomic_if_changed(&portal_dir.join("Malus-portals.conf"), portal_content);

    // 1.2. Propagar variables de entorno aisladas a D-Bus/systemd solo una vez por sesión y sin bloquear UI
    propagate_isolated_portal_environment_once(&malus_config_dir);

    let (cursor_theme, cursor_size) = load_cursor_config();

    // 2. Archivos settings.ini para GTK3 y GTK4 (preservar configuraciones existentes del usuario)
    let gtk3_dir = malus_config_dir.join("gtk-3.0");
    std::fs::create_dir_all(&gtk3_dir)?;
    let gtk3_ini = gtk3_dir.join("settings.ini");
    if !gtk3_ini.exists() {
        let gtk_ini_content = format!(
            "[Settings]\ngtk-decoration-layout=close,minimize,maximize:\ngtk-application-prefer-dark-theme=1\ngtk-cursor-theme-name={}\ngtk-cursor-theme-size={}\ngtk-theme-name=Adwaita\n",
            cursor_theme, cursor_size
        );
        write_file_atomic_if_changed(&gtk3_ini, &gtk_ini_content)?;
    }

    let gtk4_dir = malus_config_dir.join("gtk-4.0");
    std::fs::create_dir_all(&gtk4_dir)?;
    let gtk4_ini = gtk4_dir.join("settings.ini");
    if !gtk4_ini.exists() {
        let gtk_ini_content = format!(
            "[Settings]\ngtk-decoration-layout=close,minimize,maximize:\ngtk-application-prefer-dark-theme=1\ngtk-cursor-theme-name={}\ngtk-cursor-theme-size={}\ngtk-theme-name=Adwaita\n",
            cursor_theme, cursor_size
        );
        write_file_atomic_if_changed(&gtk4_ini, &gtk_ini_content)?;
    }

    set_gtk_decoration_layout(&malus_config_dir)?;
    set_gsettings_button_layout(&malus_config_dir);

    // 2.1. Hojas de estilo GTK exclusivas de Malus DE (generar y escribir solo la activa)
    let is_dark = is_dark_mode_active();
    let active_gtk3 = get_malus_gtk3_css(is_dark);
    let active_css = get_malus_gtk_css(is_dark);

    // GTK 3
    let _ = write_file_atomic_if_changed(&gtk3_dir.join("gtk.css"), &active_gtk3);

    // GTK 4
    let _ = write_file_atomic_if_changed(&gtk4_dir.join("gtk.css"), &active_css);
    let gtk4_dark_path = gtk4_dir.join("gtk-dark.css");
    if is_dark {
        let _ = write_file_atomic_if_changed(&gtk4_dark_path, &active_css);
    } else if !gtk4_dark_path.exists() {
        let _ = write_file_atomic_if_changed(&gtk4_dark_path, &get_malus_gtk_css(true));
    }

    // 3. Configuración de monitores y escalado fraccionario (~/.config/malus-de/monitors.toml)
    let monitors_path = malus_config_dir.join("monitors.toml");
    if !monitors_path.exists() {
        let displays_cfg = DisplaysConfig::load();
        if let Ok(toml_str) = toml::to_string_pretty(&displays_cfg) {
            let header = "# Configuración de Pantallas y Monitores para Malus DE\n# Soporte nativo para múltiples pantallas 4K con escalado fraccionario independiente.\n\n";
            let _ = std::fs::write(&monitors_path, format!("{}{}", header, toml_str));
        }
    }

    // 4. Configuración del Dock (~/.config/malus-de/dock.toml)
    let dock_path = malus_config_dir.join("dock.toml");
    if !dock_path.exists() {
        let dock_cfg = DockConfig::load_or_create_default();
        let _ = dock_cfg.save();
    }

    // 5. Configuración de Teclado y Entrada (~/.config/malus-de/input.toml)
    let input_path = malus_config_dir.join("input.toml");
    if !input_path.exists() {
        let input_cfg = InputConfig::load();
        let _ = input_cfg.save();
    }

    // 6. Configuración de editores Electron (Antigravity IDE, Code) para controles a la izquierda
    setup_electron_ide_configurations(&malus_config_dir);

    // 7. Configuración de Mozilla Firefox (controles semáforo macOS vía userChrome.css)
    setup_firefox_configuration(&malus_config_dir);

    // 8. Asociaciones MIME por defecto (paquetes .rpm, flatpak, etc. hacia GNOME Software)
    setup_default_mime_applications(&malus_config_dir);

    // 9. Asegurar que el tema de cursores macOS integrado esté disponible en el entorno del usuario
    let bundled_cursor_dirs = [
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/cursors")),
        PathBuf::from("/usr/local/share/malus-de/cursors"),
    ];
    let user_icons_dir = PathBuf::from(&home).join(".local/share/icons");
    let dot_icons_dir = PathBuf::from(&home).join(".icons");
    let _ = std::fs::create_dir_all(&user_icons_dir);
    let _ = std::fs::create_dir_all(&dot_icons_dir);
    for variant in ["macOS", "macOS-White"] {
        if let Some(src_dir) = bundled_cursor_dirs
            .iter()
            .map(|base| base.join(variant))
            .find(|p| p.join("cursors").is_dir())
        {
            for target_base in [&user_icons_dir, &dot_icons_dir] {
                let dst = target_base.join(variant);
                if !dst.exists() {
                    #[cfg(unix)]
                    let _ = std::os::unix::fs::symlink(&src_dir, &dst);
                }
            }
        }
    }

    // Exportar variables de cursor para los procesos iniciados desde Malus DE
    let xcursor_path = format!(
        "{}:{}:/usr/local/share/malus-de/cursors:{}:/usr/share/icons",
        user_icons_dir.display(),
        dot_icons_dir.display(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/cursors")
    );
    std::env::set_var("XCURSOR_PATH", xcursor_path);
    std::env::set_var("XCURSOR_THEME", &cursor_theme);
    std::env::set_var("XCURSOR_SIZE", cursor_size.to_string());

    Ok(malus_config_dir)
}

/// Configura los editores basados en VS Code / Electron (Antigravity IDE, Code) para que utilicen
/// la barra personalizada pero con controles custom (`"window.controlsStyle": "custom"`), permitiendo
/// que los controles de ventana respeten la posición y estética de macOS en Malus DE.
pub fn setup_electron_ide_configurations(malus_config_dir: &Path) {
    let ide_names = ["Antigravity IDE", "Code", "Code - OSS", "VSCodium"];
    for ide_name in ide_names {
        let user_dir = malus_config_dir.join(ide_name).join("User");
        let settings_path = user_dir.join("settings.json");

        if !settings_path.exists() {
            let _ = std::fs::create_dir_all(&user_dir);
            let default_settings = serde_json::json!({
                "window.titleBarStyle": "custom",
                "window.controlsStyle": "custom"
            });
            if let Ok(content) = serde_json::to_string_pretty(&default_settings) {
                let _ = std::fs::write(&settings_path, content);
            }
        } else if let Ok(existing) = std::fs::read_to_string(&settings_path) {
            if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&existing) {
                let mut modified = false;
                if let Some(obj) = json.as_object_mut() {
                    if obj.get("window.controlsStyle").and_then(|v| v.as_str()) != Some("custom") {
                        obj.insert(
                            "window.controlsStyle".to_string(),
                            serde_json::Value::String("custom".to_string()),
                        );
                        modified = true;
                    }
                    if obj.get("window.titleBarStyle").and_then(|v| v.as_str()) != Some("custom") {
                        obj.insert(
                            "window.titleBarStyle".to_string(),
                            serde_json::Value::String("custom".to_string()),
                        );
                        modified = true;
                    }
                }
                if modified {
                    if let Ok(content) = serde_json::to_string_pretty(&json) {
                        let _ = std::fs::write(&settings_path, content);
                    }
                }
            }
        }
    }
}

/// Resuelve la ruta al wrapper de Malus DE para editores basados en Electron
/// (Antigravity IDE, VS Code, Code - OSS, VSCodium).
pub fn resolve_electron_ide_wrapper(exe: &str) -> Option<PathBuf> {
    let is_antigravity = exe == "antigravity-ide"
        || exe == "antigravity"
        || exe.ends_with("/antigravity-ide")
        || exe.ends_with("/antigravity");

    let is_vscode = exe == "code"
        || exe == "code-oss"
        || exe == "vscode"
        || exe == "codium"
        || exe == "vscodium"
        || exe.ends_with("/code")
        || exe.ends_with("/code-oss")
        || exe.ends_with("/codium")
        || exe.ends_with("/vscodium");

    if !is_antigravity && !is_vscode {
        return None;
    }

    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let mut candidate_paths = Vec::new();

    if is_antigravity {
        if let Some(h) = &home {
            candidate_paths.push(h.join(".local/bin/antigravity-ide-wrapper"));
        }
        candidate_paths.push(PathBuf::from("/usr/local/bin/antigravity-ide-wrapper"));
    }

    if is_vscode {
        if let Some(h) = &home {
            candidate_paths.push(h.join(".local/bin/code-wrapper"));
        }
        candidate_paths.push(PathBuf::from("/usr/local/bin/code-wrapper"));
    }

    // Wrappers unificados de Malus DE
    if let Some(h) = &home {
        candidate_paths.push(h.join(".local/bin/malus-ide-wrapper"));
    }
    candidate_paths.push(PathBuf::from("/usr/local/bin/malus-ide-wrapper"));

    for path in candidate_paths {
        if path.is_file() {
            return Some(path);
        }
    }

    None
}

/// Configura los perfiles de Mozilla Firefox para que utilicen los controles de ventana
/// semáforo estilo macOS (posición a la izquierda, círculos de colores y glifos vectoriales en hover)
/// persistiendo los estilos en `chrome/userChrome.css` y habilitando la preferencia en `user.js`.
pub fn setup_firefox_configuration(malus_config_dir: &Path) {
    let mut candidate_dirs = Vec::new();

    // 1. Perfiles bajo el XDG_CONFIG_HOME aislado de Malus DE (~/.config/malus-de/mozilla/firefox)
    let isolated_firefox_dir = malus_config_dir.join("mozilla").join("firefox");
    candidate_dirs.push(isolated_firefox_dir.clone());

    // 2. Perfiles estándar del host (~/.mozilla/firefox)
    if let Ok(home) = std::env::var("HOME") {
        let host_firefox_dir = PathBuf::from(home).join(".mozilla").join("firefox");
        if host_firefox_dir != isolated_firefox_dir {
            candidate_dirs.push(host_firefox_dir);
        }
    }

    let mut configured_any_profile = false;

    for base_dir in &candidate_dirs {
        if !base_dir.exists() {
            continue;
        }

        // Buscar perfiles definidos en profiles.ini si existe
        let mut profile_subdirs = Vec::new();
        let profiles_ini_path = base_dir.join("profiles.ini");
        if let Ok(content) = std::fs::read_to_string(&profiles_ini_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Path=") {
                    let rel_path = trimmed.trim_start_matches("Path=").trim();
                    if !rel_path.is_empty() {
                        profile_subdirs.push(base_dir.join(rel_path));
                    }
                }
            }
        }

        // Escanear también todas las carpetas hijas por si hay perfiles no registrados en profiles.ini
        if let Ok(entries) = std::fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && !profile_subdirs.contains(&path) {
                    let is_profile = path.join("prefs.js").exists()
                        || path.join("compatibility.ini").exists()
                        || path.join("times.json").exists()
                        || path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.contains(".default") || n.contains(".release"))
                            .unwrap_or(false);
                    if is_profile {
                        profile_subdirs.push(path);
                    }
                }
            }
        }

        for profile_dir in profile_subdirs {
            apply_firefox_profile_styles(&profile_dir);
            configured_any_profile = true;
        }
    }

    // 3. Si es una instalación limpia de Malus DE donde aún no se ha abierto Firefox por primera vez,
    // pre-crear un perfil default inicial con los estilos ya inyectados
    if !configured_any_profile {
        let default_profile_dir = isolated_firefox_dir.join("default-release");
        let _ = std::fs::create_dir_all(&default_profile_dir);
        apply_firefox_profile_styles(&default_profile_dir);

        let initial_profiles_ini = r#"[General]
StartWithLastProfile=1
Version=2

[Profile0]
Name=default-release
IsRelative=1
Path=default-release
Default=1

[InstallMalusDE]
Default=default-release
Locked=1
"#;
        let _ = std::fs::write(isolated_firefox_dir.join("profiles.ini"), initial_profiles_ini);
    }
}

/// Aplica o actualiza `chrome/userChrome.css` y la preferencia en `user.js` para un perfil específico de Firefox.
pub fn apply_firefox_profile_styles(profile_dir: &Path) {
    let chrome_dir = profile_dir.join("chrome");
    let _ = std::fs::create_dir_all(&chrome_dir);
    let user_chrome_path = chrome_dir.join("userChrome.css");

    let should_write_css = match std::fs::read_to_string(&user_chrome_path) {
        Ok(existing) => existing != MALUS_FIREFOX_USER_CHROME_CSS,
        Err(_) => true,
    };

    if should_write_css {
        let _ = std::fs::write(&user_chrome_path, MALUS_FIREFOX_USER_CHROME_CSS);
        tracing::info!("🎨 [Firefox] userChrome.css actualizado en: {:?}", user_chrome_path);
    }

    let user_js_path = profile_dir.join("user.js");
    let pref_line = "user_pref(\"toolkit.legacyUserProfileCustomizations.stylesheets\", true);";

    if user_js_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&user_js_path) {
            if !content.contains("toolkit.legacyUserProfileCustomizations.stylesheets") {
                let new_content = format!("{}\n{}\n", content.trim_end(), pref_line);
                let _ = std::fs::write(&user_js_path, new_content);
            }
        }
    } else {
        let content = format!("// Malus DE - Configuración de interfaz de ventana\n{}\n", pref_line);
        let _ = std::fs::write(&user_js_path, content);
    }
}

/// Configura las asociaciones MIME por defecto en `mimeapps.list` para que
/// paquetes de aplicaciones (.rpm, .flatpakref, .deb, etc.) se abran con GNOME Software al hacer doble clic.
pub fn setup_default_mime_applications(malus_config_dir: &Path) {
    let mime_path = malus_config_dir.join("mimeapps.list");
    let default_package_associations: &[(&str, &str)] = &[
        ("application/x-rpm", "gnome-software-local-file-packagekit.desktop"),
        ("application/x-redhat-package-manager", "gnome-software-local-file-packagekit.desktop"),
        ("application/vnd.flatpak", "gnome-software-local-file-flatpak.desktop"),
        ("application/vnd.flatpak.ref", "gnome-software-local-file-flatpak.desktop"),
        ("application/vnd.flatpak.repo", "gnome-software-local-file-flatpak.desktop"),
        ("application/x-deb", "gnome-software-local-file-packagekit.desktop"),
        ("application/x-app-package", "gnome-software-local-file-packagekit.desktop"),
    ];

    let content = std::fs::read_to_string(&mime_path).unwrap_or_default();
    let mut default_apps: Vec<String> = Vec::new();
    let mut added_associations: Vec<String> = Vec::new();
    let mut other_sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut current_section = "";

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed;
            if current_section != "[Default Applications]" && current_section != "[Added Associations]" {
                other_sections.push((current_section.to_string(), Vec::new()));
            }
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if current_section == "[Default Applications]" {
            default_apps.push(trimmed.to_string());
        } else if current_section == "[Added Associations]" {
            added_associations.push(trimmed.to_string());
        } else if let Some((_, list)) = other_sections.last_mut() {
            list.push(trimmed.to_string());
        }
    }

    for (mime, app) in default_package_associations {
        let prefix = format!("{}=", mime);
        if !default_apps.iter().any(|entry| entry.starts_with(&prefix)) {
            default_apps.push(format!("{}={}", mime, app));
        }
        let added_entry = format!("{}={};", mime, app);
        if !added_associations.iter().any(|entry| entry.starts_with(&prefix)) {
            added_associations.push(added_entry);
        }
    }

    let mut new_content = String::from("[Default Applications]\n");
    for entry in &default_apps {
        new_content.push_str(entry);
        new_content.push('\n');
    }
    new_content.push_str("\n[Added Associations]\n");
    for entry in &added_associations {
        new_content.push_str(entry);
        new_content.push('\n');
    }
    for (sec, list) in &other_sections {
        new_content.push('\n');
        new_content.push_str(sec);
        new_content.push('\n');
        for entry in list {
            new_content.push_str(entry);
            new_content.push('\n');
        }
    }

    let _ = std::fs::write(&mime_path, new_content);
}

pub fn load_cursor_config() -> (String, u32) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".config"));
    let settings_path = config_home.join("glib-2.0/settings/keyfile");
    let settings = std::fs::read_to_string(settings_path).unwrap_or_default();
    let (configured_theme, configured_size) = parse_cursor_config(&settings);
    let theme = configured_theme
        .filter(|value| {
            !value.is_empty()
                && value != "Adwaita"
                && value != "AdwaitaLegacy"
                && value != "default"
        })
        .or_else(|| {
            std::env::var("XCURSOR_THEME").ok().filter(|value| {
                !value.is_empty()
                    && value != "Adwaita"
                    && value != "AdwaitaLegacy"
                    && value != "default"
            })
        })
        .unwrap_or_else(|| "macOS".to_string());
    let size = configured_size
        .or_else(|| std::env::var("XCURSOR_SIZE").ok()?.parse::<u32>().ok())
        .unwrap_or(24)
        .clamp(1, 128);
    (theme, size)
}

fn parse_cursor_config(settings: &str) -> (Option<String>, Option<u32>) {
    let mut in_interface_section = false;
    let mut theme = None;
    let mut size = None;

    for line in settings.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_interface_section = line == "[org/gnome/desktop/interface]";
        } else if in_interface_section {
            if let Some(value) = line.strip_prefix("cursor-theme=") {
                theme = Some(value.trim().trim_matches('\'').to_string());
            } else if let Some(value) = line.strip_prefix("cursor-size=") {
                size = value.trim().parse::<u32>().ok();
            }
        }
    }
    (theme, size)
}

pub fn set_cursor_environment(command: &mut std::process::Command) {
    let (theme, size) = load_cursor_config();
    command
        .env("XCURSOR_THEME", theme)
        .env("XCURSOR_SIZE", size.to_string());
}

/// Aplica el entorno completamente aislado de Malus DE a cualquier subproceso o aplicación lanzada.
/// Garantiza que ninguna aplicación herede ni consulte dconf/GSettings del host GNOME.
pub fn apply_isolated_environment(command: &mut std::process::Command) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".config/malus-de"));
    set_cursor_environment(command);
    command.env("XDG_CONFIG_HOME", &malus_config_dir);
    command.env("GSETTINGS_BACKEND", "keyfile");
    command.env("XDG_CURRENT_DESKTOP", "Malus:GNOME");
    command.env("GDK_BACKEND", "wayland");
    command.env_remove("GTK_USE_PORTAL");
    if let Ok(wayland_disp) = std::env::var("WAYLAND_DISPLAY") {
        command.env("WAYLAND_DISPLAY", wayland_disp);
    }
    if let Ok(xdg_rt) = std::env::var("XDG_RUNTIME_DIR") {
        command.env("XDG_RUNTIME_DIR", xdg_rt);
    }
}

/// Consulta el modo oscuro actual exclusivamente desde la configuración aislada de Malus DE.
/// NUNCA consulta dconf ni GSettings del host GNOME para evitar contaminación mutua.
pub fn is_dark_mode_active() -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".config/malus-de"));

    // 1. Keyfile aislado de Malus DE
    let keyfile_path = malus_config_dir.join("glib-2.0/settings/keyfile");
    if let Ok(content) = std::fs::read_to_string(&keyfile_path) {
        if content.contains("color-scheme='prefer-dark'") {
            return true;
        } else if content.contains("color-scheme='prefer-light'") {
            return false;
        }
    }

    // 2. Settings.ini de GTK4 / GTK3 aislado de Malus DE
    let gtk4_ini = malus_config_dir.join("gtk-4.0/settings.ini");
    if let Ok(content) = std::fs::read_to_string(&gtk4_ini) {
        if content.contains("gtk-application-prefer-dark-theme=1") {
            return true;
        } else if content.contains("gtk-application-prefer-dark-theme=0") {
            return false;
        }
    }

    // Valor predeterminado en Malus DE: modo oscuro
    true
}

/// Sincroniza el modo claro/oscuro EXCLUSIVAMENTE dentro del entorno aislado de Malus DE
/// (~/.config/malus-de). NUNCA modifica dconf del host, ni GSettings global, ni ~/.config/gtk-*.0
/// del sistema anfitrión, garantizando independencia bidireccional absoluta frente a GNOME.
pub fn sync_system_appearance(to_dark: bool) {
    let target = if to_dark { "prefer-dark" } else { "prefer-light" };
    let color_scheme_val = format!("'{}'", target);
    let dark_val = if to_dark { "1" } else { "0" };
    let gtk_theme_target = "Adwaita";
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = std::path::PathBuf::from(&home).join(".config/malus-de");

    // 1. Mantener variables de entorno aisladas en el proceso actual
    std::env::set_var("XDG_CONFIG_HOME", &malus_config_dir);
    std::env::set_var("GSETTINGS_BACKEND", "keyfile");

    // 2. Actualizar keyfile en una sola escritura atómica (dispara GLib/portal en ~0.4 ms sin subprocesos)
    let keyfile_path = malus_config_dir.join("glib-2.0/settings/keyfile");
    let _ = upsert_keyfile_entries(
        &keyfile_path,
        &[
            (
                "org/gnome/desktop/interface",
                &[
                    ("color-scheme", color_scheme_val.as_str()),
                    ("gtk-theme", "'Adwaita'"),
                ],
            ),
            (
                "org/gnome/desktop/wm/preferences",
                &[("button-layout", "'close,minimize,maximize:'")],
            ),
        ],
    );

    // 3. Actualizar settings.ini de GTK3 y GTK4 en una sola pasada atómica por archivo
    for ini_path in [
        malus_config_dir.join("gtk-3.0/settings.ini"),
        malus_config_dir.join("gtk-4.0/settings.ini"),
    ] {
        let _ = update_gtk_ini_appearance(&ini_path, dark_val, gtk_theme_target);
    }

    // 4. Generar y escribir únicamente la hoja CSS activa (y gtk-dark.css solo si hace falta)
    let active_gtk3 = get_malus_gtk3_css(to_dark);
    let active_gtk4 = get_malus_gtk_css(to_dark);

    let _ = write_file_atomic_if_changed(
        &malus_config_dir.join("gtk-3.0/gtk.css"),
        &active_gtk3,
    );
    let _ = write_file_atomic_if_changed(
        &malus_config_dir.join("gtk-4.0/gtk.css"),
        &active_gtk4,
    );
    let gtk4_dark_path = malus_config_dir.join("gtk-4.0/gtk-dark.css");
    if to_dark {
        let _ = write_file_atomic_if_changed(&gtk4_dark_path, &active_gtk4);
    } else if !gtk4_dark_path.exists() {
        let _ = write_file_atomic_if_changed(&gtk4_dark_path, &get_malus_gtk_css(true));
    }
}

/// Configuración de una aplicación anclada en el Dock
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DockAppConfig {
    pub name: String,
    pub icon: String,
    pub command: String,
    #[serde(default)]
    pub matchers: Vec<String>,
    #[serde(default)]
    pub icon_appearance: DockIconAppearance,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockIconAppearance {
    #[default]
    Auto,
    DockBackground,
    Native,
}

impl DockAppConfig {
    pub fn matches(&self, identifier: &str) -> bool {
        let id_lower = identifier.to_lowercase();
        if self.name.to_lowercase() == id_lower {
            return true;
        }
        for m in &self.matchers {
            if id_lower.contains(&m.to_lowercase()) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DockConfig {
    pub position: DockPosition,
    pub style: DockStyle,
    pub icon_size: u32,
    pub magnification_enabled: bool,
    pub magnification_max_size: u32,
    pub floating_gap: u32,
    pub corner_radius: u32,
    pub background_opacity: f32,
    pub blur_strength: u32,
    pub genie_effect: bool,
    pub indicator_style: DockIndicatorStyle,
    pub apps: Vec<DockAppConfig>,
}

const DOCK_BOUNCE_HEADROOM: u32 = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockPosition {
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockStyle {
    Floating,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockIndicatorStyle {
    Dot,
    Dash,
    Glow,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            position: DockPosition::Bottom,
            style: DockStyle::Floating,
            icon_size: 48,
            magnification_enabled: true,
            magnification_max_size: 62,
            floating_gap: 4,
            corner_radius: 24,
            background_opacity: 0.70,
            blur_strength: 24,
            genie_effect: true,
            indicator_style: DockIndicatorStyle::Dot,
            apps: vec![
                DockAppConfig {
                    name: "Archivos".to_string(),
                    icon: "org.gnome.Nautilus".to_string(),
                    command: "nautilus --new-window".to_string(),
                    matchers: vec!["nautilus".into(), "archivos".into(), "org.gnome.nautilus".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Terminal".to_string(),
                    icon: "org.gnome.Ptyxis".to_string(),
                    command: "ptyxis -s".to_string(),
                    matchers: vec!["ptyxis".into(), "terminal".into(), "org.gnome.ptyxis".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Editor de Texto".to_string(),
                    icon: "org.gnome.TextEditor".to_string(),
                    command: "gnome-text-editor -s".to_string(),
                    matchers: vec!["texteditor".into(), "text-editor".into(), "gedit".into(), "org.gnome.texteditor".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Calculadora".to_string(),
                    icon: "org.gnome.Calculator".to_string(),
                    command: "gnome-calculator".to_string(),
                    matchers: vec!["calculator".into(), "calculadora".into(), "org.gnome.calculator".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Ajustes del Sistema".to_string(),
                    icon: "preferences-system".to_string(),
                    command: "malus-de-settings".to_string(),
                    matchers: vec!["malus-de-settings".into(), "dev.malus.settings".into(), "ajustes".into(), "settings".into(), "configuración".into(), "configuracion".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
            ],
        }
    }
}

impl DockConfig {
    /// Height reserved above the dock for magnification and bounce animations.
    pub fn headroom_height(&self) -> u32 {
        self.magnification_max_size
            .saturating_sub(self.icon_size)
            .saturating_add(DOCK_BOUNCE_HEADROOM)
    }

    /// Determina la ruta canónica del archivo `dock.toml`
    pub fn config_path() -> PathBuf {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let p1 = PathBuf::from(&xdg).join("dock.toml");
            let p2 = PathBuf::from(&xdg).join("malus-de").join("dock.toml");
            if p1.exists() {
                return p1;
            }
            if p2.exists() {
                return p2;
            }
            if PathBuf::from(&xdg).ends_with("malus-de") {
                return p1;
            } else {
                return p2;
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config").join("malus-de").join("dock.toml")
    }

    /// Carga la configuración desde `dock.toml`, o crea el archivo con la configuración predeterminada si no existe
    pub fn load_or_create_default() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str::<DockConfig>(&content) {
                    return cfg;
                }
            }
        }
        let default_cfg = Self::default();
        let _ = default_cfg.save();
        default_cfg
    }

    /// Guarda la configuración actual en el archivo `dock.toml`
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Configuración del Dock de Malus DE\n# Personalización de tamaño de iconos, magnificación, gap flotante y aplicaciones ancladas.\n# icon_appearance por aplicación: auto, dock_background o native.\n\n";
        std::fs::write(path, format!("{}{}", header, toml_str))
    }

    /// Ancla una aplicación al final del dock si no está ya presente
    pub fn pin_app(&mut self, app: DockAppConfig) -> bool {
        self.pin_app_at(app, None)
    }

    /// Ancla una aplicación en una posición específica del dock (o la reubica si ya estaba anclada)
    pub fn pin_app_at(&mut self, app: DockAppConfig, index: Option<usize>) -> bool {
        if let Some(existing_idx) = self.apps.iter().position(|a| {
            a.name.eq_ignore_ascii_case(&app.name)
                || (!a.command.is_empty() && a.command == app.command)
        }) {
            if let Some(target) = index {
                let clamped = target.min(self.apps.len().saturating_sub(1));
                return self.move_app(existing_idx, clamped);
            }
            return false;
        }
        match index {
            Some(target) => {
                let insert_idx = target.min(self.apps.len());
                self.apps.insert(insert_idx, app);
            }
            None => {
                self.apps.push(app);
            }
        }
        true
    }

    /// Desancla una aplicación del dock por su nombre
    pub fn unpin_app(&mut self, name: &str) -> bool {
        let prev_len = self.apps.len();
        self.apps.retain(|a| !a.name.eq_ignore_ascii_case(name));
        self.apps.len() < prev_len
    }

    /// Mueve una aplicación de posición (reordenar)
    pub fn move_app(&mut self, from_idx: usize, to_idx: usize) -> bool {
        if from_idx < self.apps.len() && to_idx < self.apps.len() && from_idx != to_idx {
            let item = self.apps.remove(from_idx);
            self.apps.insert(to_idx, item);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopBarConfig {
    pub height: u32,
    pub global_menu: bool,
    pub clock_show_seconds: bool,
}

impl Default for TopBarConfig {
    fn default() -> Self {
        Self {
            height: 28,
            global_menu: true,
            clock_show_seconds: false,
        }
    }
}

/// Configuración de la superficie de escritorio activa (archivos e iconos de `~/Escritorio`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopIconsConfig {
    /// Mostrar los archivos de la carpeta de escritorio sobre el fondo de pantalla
    pub show_icons: bool,
    /// Tamaño de los iconos en píxeles lógicos (32-96)
    pub icon_size: u32,
    /// Alinear la cuadrícula a la columna izquierda (por defecto derecha, estilo macOS)
    pub align_left: bool,
    /// Imantar los iconos a una rejilla invisible al soltarlos
    pub snap_to_grid: bool,
    /// Agrupar automáticamente los archivos en pilas por categoría (macOS Stacks)
    pub use_stacks: bool,
}

impl Default for DesktopIconsConfig {
    fn default() -> Self {
        Self {
            show_icons: true,
            icon_size: 64,
            align_left: false,
            snap_to_grid: false,
            use_stacks: false,
        }
    }
}

impl DesktopIconsConfig {
    pub const MIN_ICON_SIZE: u32 = 32;
    pub const MAX_ICON_SIZE: u32 = 96;

    pub fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/desktop.toml")
    }

    /// Carga la configuración (o la predeterminada si no existe o es inválida)
    pub fn load() -> Self {
        std::fs::read_to_string(Self::config_path())
            .ok()
            .and_then(|content| toml::from_str::<DesktopIconsConfig>(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Superficie de Escritorio de Malus DE (archivos de ~/Escritorio)\n\n";
        std::fs::write(path, format!("{}{}", header, toml_str))
    }

    /// Tamaño de icono acotado al rango soportado
    pub fn clamped_icon_size(&self) -> i32 {
        self.icon_size
            .clamp(Self::MIN_ICON_SIZE, Self::MAX_ICON_SIZE) as i32
    }
}

fn default_keyboard_layout() -> String {
    "latam".to_string()
}

fn default_keyboard_model() -> String {
    "pc105".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    /// Distribución de teclado XKB activa ("latam" = Español Latinoamericano, "es" = Español España, "us" = Inglés EE. UU.)
    #[serde(default = "default_keyboard_layout")]
    pub keyboard_layout: String,
    /// Variante XKB opcional (ej. "" o "deadtilde")
    #[serde(default)]
    pub keyboard_variant: String,
    /// Modelo físico de teclado XKB (por defecto "pc105")
    #[serde(default = "default_keyboard_model")]
    pub keyboard_model: String,
    /// Opciones XKB adicionales (ej. Some("grp:alt_shift_toggle"))
    #[serde(default)]
    pub keyboard_options: Option<String>,
    pub natural_scrolling: bool,
    pub shake_to_find_cursor: bool,
    pub show_desktop_gesture: bool,
    pub hot_corners: HotCornersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotCornersConfig {
    pub enabled: bool,
    pub delay_ms: u32,
    pub top_left: HotCornerAction,
    pub top_right: HotCornerAction,
    pub bottom_left: HotCornerAction,
    pub bottom_right: HotCornerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotCornerAction {
    None,
    MissionControl,
    NotificationCenter,
    ShowDesktop,
    LockScreen,
}

impl Default for HotCornersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            delay_ms: 150,
            top_left: HotCornerAction::MissionControl,
            top_right: HotCornerAction::NotificationCenter,
            bottom_left: HotCornerAction::LockScreen,
            bottom_right: HotCornerAction::ShowDesktop,
        }
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            keyboard_layout: default_keyboard_layout(),
            keyboard_variant: String::new(),
            keyboard_model: default_keyboard_model(),
            keyboard_options: None,
            natural_scrolling: true,
            shake_to_find_cursor: true,
            show_desktop_gesture: true,
            hot_corners: HotCornersConfig::default(),
        }
    }
}

impl InputConfig {
    pub fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/input.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(mut cfg) = toml::from_str::<InputConfig>(&content) {
                    if cfg.keyboard_layout.trim().is_empty() {
                        cfg.keyboard_layout = "latam".to_string();
                    }
                    return cfg;
                }
            }
        }
        let cfg = Self::default();
        let _ = cfg.save();
        cfg
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Configuración de Teclado y Entrada para Malus DE\n# Soporte nativo para Español Latinoamericano (latam), Español (es) e Inglés (us).\n\n";
        std::fs::write(path, format!("{}{}", header, toml_str))
    }

    pub fn layout_display_name(&self) -> &'static str {
        match self.keyboard_layout.as_str() {
            "latam" => "🇲🇽 Español (Latinoamérica)",
            "es" => "🇪🇸 Español (España)",
            "us" => "🇺🇸 Inglés (EE. UU.)",
            _ => "⌨️ Personalizado",
        }
    }

    pub fn layout_short_badge(&self) -> &'static str {
        match self.keyboard_layout.as_str() {
            "latam" => "LAT",
            "es" => "ES",
            "us" => "US",
            _ => "KB",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilingConfig {
    pub enabled: bool,
    pub gaps: u32,
    pub default_layout: TilingLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TilingLayout {
    BinaryTree,
    Columns,
}

impl Default for TilingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gaps: 12,
            default_layout: TilingLayout::BinaryTree,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkConfig {
    pub device_name: String,
    pub auto_clipboard: bool,
    pub pause_on_call: bool,
    pub sync_notifications: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            device_name: "Malus Linux".to_string(),
            auto_clipboard: true,
            pause_on_call: true,
            sync_notifications: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerConfig {
    pub default_profile: PowerProfile,
    pub auto_power_saver_on_low_battery: bool,
    pub low_battery_threshold: u8,
    pub reduce_effects_on_power_saver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerProfile {
    Performance,
    Balanced,
    PowerSaver,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            default_profile: PowerProfile::Balanced,
            auto_power_saver_on_low_battery: true,
            low_battery_threshold: 20,
            reduce_effects_on_power_saver: true,
        }
    }
}

impl Default for MalusConfig {
    fn default() -> Self {
        Self {
            appearance: AppearanceConfig::default(),
            dock: DockConfig::default(),
            top_bar: TopBarConfig::default(),
            input: InputConfig::default(),
            tiling: TilingConfig::default(),
            link: LinkConfig::default(),
            power: PowerConfig::default(),
            displays: DisplaysConfig::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_master_scale() -> f64 {
    1.0
}

fn default_brightness() -> f64 {
    80.0
}

fn default_screen_corner_radius() -> f64 {
    32.0
}

fn default_window_corner_radius() -> f64 {
    32.0
}

/// Configuración multi-monitor con escalado fraccionario independiente y Enlace Físico 1:1
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplaysConfig {
    /// Activa el enlace matemático 1:1 para que ventanas y textos midan los mismos milímetros físicos en todos los monitores
    #[serde(default = "default_true")]
    pub linked_physical_1to1: bool,
    /// Escala maestra aplicada al monitor principal (1.0 = 100%, 1.666667 = 166.7%, 2.0 = 200%)
    #[serde(default = "default_master_scale")]
    pub master_scale: f64,
    /// Nivel de brillo de pantalla configurado (5.0 a 100.0)
    #[serde(default = "default_brightness")]
    pub brightness: f64,
    /// Radio de curvatura squircle de las esquinas del escritorio (en puntos lógicos, ej. 24.0 o 32.0). 0.0 desactiva el recorte.
    #[serde(default = "default_screen_corner_radius")]
    pub screen_corner_radius: f64,
    /// Radio de curvatura squircle de las ventanas (en puntos lógicos, ej. 32.0). 0.0 desactiva el recorte.
    #[serde(default = "default_window_corner_radius")]
    pub window_corner_radius: f64,
    /// Lista de pantallas configuradas
    pub outputs: Vec<DisplayOutputConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayOutputConfig {
    pub connector: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f64,
    #[serde(default)]
    pub modes: Vec<DisplayModeConfig>,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub primary: bool,
    /// Diagonal física real del panel en pulgadas (permite corregir datos genéricos de EDID)
    #[serde(default)]
    pub diagonal_inches: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayModeConfig {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f64,
}

impl DisplayOutputConfig {
    /// Obtiene la diagonal real o estimada en pulgadas
    pub fn effective_diagonal(&self) -> f64 {
        if let Some(d) = self.diagonal_inches {
            return d;
        }
        let n = self.name.to_lowercase();
        if n.contains("27") {
            27.0
        } else if n.contains("24") || n.contains("mi monitor") {
            24.0
        } else if n.contains("32") {
            32.0
        } else {
            27.0
        }
    }

    /// Calcula la densidad física de píxeles por pulgada (PPI)
    pub fn ppi(&self) -> f64 {
        let diag = self.effective_diagonal();
        if diag <= 0.0 {
            return 163.0;
        }
        let diag_px = ((self.width as f64).powi(2) + (self.height as f64).powi(2)).sqrt();
        diag_px / diag
    }

    /// Dimensiones físicas aproximadas del panel en milímetros
    pub fn physical_size_mm(&self) -> (f64, f64) {
        let ppi = self.ppi();
        let w_mm = (self.width as f64 / ppi) * 25.4;
        let h_mm = (self.height as f64 / ppi) * 25.4;
        (w_mm, h_mm)
    }

    /// Resolución lógica en el espacio virtual del compositor
    pub fn logical_size(&self) -> (i32, i32) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (self.width as f64 / s).round() as i32,
            (self.height as f64 / s).round() as i32,
        )
    }
}

impl Default for DisplaysConfig {
    fn default() -> Self {
        Self {
            linked_physical_1to1: true,
            master_scale: 1.0,
            brightness: 80.0,
            screen_corner_radius: 32.0,
            window_corner_radius: 32.0,
            outputs: Vec::new(),
        }
    }
}

impl DisplaysConfig {
    /// Carga la configuración de monitores desde `~/.config/malus-de/monitors.toml`,
    /// o como fallback parsea `~/.config/monitors.xml` de GNOME.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let toml_path = PathBuf::from(&home).join(".config/malus-de/monitors.toml");
        if toml_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                if let Ok(mut config) = toml::from_str::<DisplaysConfig>(&content) {
                    config.recalculate_layout();
                    return config;
                }
            }
        }

        // Fallback: intentar parsear ~/.config/monitors.xml
        let xml_path = PathBuf::from(&home).join(".config/monitors.xml");
        if xml_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&xml_path) {
                if let Some(mut config) = Self::from_gnome_xml(&content) {
                    if config.linked_physical_1to1 {
                        config.apply_1to1_link();
                    } else {
                        config.recalculate_layout();
                    }
                    return config;
                }
            }
        }

        Self::default()
    }

    /// Recalcula las coordenadas lógicas X e Y de las pantallas para mantenerlas adyacentes
    /// sin huecos muertos ni solapamientos cuando cambia la escala o el orden horizontal.
    pub fn recalculate_layout(&mut self) {
        if self.outputs.is_empty() {
            return;
        }

        // Ordenar índices de izquierda a derecha según su coordenada X actual
        let mut order: Vec<usize> = (0..self.outputs.len()).collect();
        order.sort_by_key(|&idx| (self.outputs[idx].x, !self.outputs[idx].primary));

        let mut cursor_x: i32 = 0;
        for idx in order {
            self.outputs[idx].x = cursor_x;
            let (lw, _) = self.outputs[idx].logical_size();
            cursor_x += lw.max(100);
        }

        // Normalizar coordenadas verticales Y para que al menos un monitor toque el borde superior (y = 0)
        if let Some(min_y) = self.outputs.iter().map(|o| o.y).min() {
            for o in &mut self.outputs {
                o.y = (o.y - min_y).max(0);
            }
        }
    }

    /// Devuelve el preset enlazado activo (Some(1), Some(2), o None)
    pub fn current_linked_preset(&self) -> Option<u8> {
        if !self.linked_physical_1to1 || self.outputs.is_empty() {
            return None;
        }
        let primary_scale = self.outputs.iter().find(|output| output.primary)?.scale;
        if (primary_scale - 1.5).abs() < 0.015 {
            Some(1)
        } else if (primary_scale - 1.333333).abs() < 0.015 {
            Some(2)
        } else {
            None
        }
    }

    /// Aplica un preset de escala enlazada a partir de la pantalla principal.
    pub fn apply_linked_preset(&mut self, preset: u8) {
        let scale = match preset {
            1 => 1.5,
            2 => 1.333333,
            _ => return,
        };
        self.linked_physical_1to1 = true;
        if self.outputs.is_empty() {
            self.master_scale = scale;
            return;
        }
        let primary_idx = self.outputs.iter().position(|output| output.primary).unwrap_or(0);
        let primary_ppi = self.outputs[primary_idx].ppi();
        self.master_scale = scale;
        for (idx, output) in self.outputs.iter_mut().enumerate() {
            output.scale = if idx == primary_idx {
                scale
            } else {
                (scale * (output.ppi() / primary_ppi)).clamp(0.75, 3.0)
            };
        }
        self.recalculate_layout();
    }

    /// Aplica el enlace físico 1:1 sincronizando la escala de todos los monitores
    /// según su densidad de píxeles (PPI) real respecto al monitor primario.
    pub fn apply_1to1_link(&mut self) {
        if !self.linked_physical_1to1 || self.outputs.is_empty() {
            self.recalculate_layout();
            return;
        }

        // Si ya coincide con un preset, mantenerlo exacto
        if let Some(preset) = self.current_linked_preset() {
            self.apply_linked_preset(preset);
            return;
        }

        let primary_idx = self.outputs.iter().position(|o| o.primary).unwrap_or(0);
        let primary_ppi = self.outputs[primary_idx].ppi();
        let primary_scale = self.master_scale.clamp(0.75, 3.0);
        self.master_scale = (primary_scale * 10000.0).round() / 10000.0;
        self.outputs[primary_idx].scale = self.master_scale;

        for (i, output) in self.outputs.iter_mut().enumerate() {
            if i == primary_idx {
                continue;
            }
            let out_ppi = output.ppi();
            // Escala 1:1 exacta en base a PPI: Escala_i = Escala_primaria * (PPI_i / PPI_primaria)
            let linked_scale = (primary_scale * (out_ppi / primary_ppi)).clamp(0.75, 3.0);
            output.scale = (linked_scale * 10000.0).round() / 10000.0;
        }

        self.recalculate_layout();
    }

    /// Actualiza la escala maestra unificada y recalcula ambas pantallas 1:1
    pub fn set_master_scale(&mut self, new_scale: f64) {
        let clamped = (new_scale.clamp(0.75, 3.0) * 10000.0).round() / 10000.0;
        self.master_scale = clamped;
        if self.linked_physical_1to1 {
            self.apply_1to1_link();
        } else {
            if let Some(p) = self.outputs.iter_mut().find(|o| o.primary) {
                p.scale = clamped;
            }
            self.recalculate_layout();
        }
    }

    /// Ajusta la escala de una pantalla específica por su índice.
    /// Si `linked_physical_1to1` está activo, ajusta proporcionalmente todas las demás pantallas según su PPI.
    pub fn set_output_scale(&mut self, output_idx: usize, new_scale: f64) {
        if output_idx >= self.outputs.len() {
            return;
        }
        let clamped = (new_scale.clamp(0.75, 3.0) * 10000.0).round() / 10000.0;
        self.outputs[output_idx].scale = clamped;

        if self.linked_physical_1to1 {
            let ref_ppi = self.outputs[output_idx].ppi();
            for i in 0..self.outputs.len() {
                if i != output_idx {
                    let i_ppi = self.outputs[i].ppi();
                    let linked = (clamped * (i_ppi / ref_ppi)).clamp(0.75, 3.0);
                    self.outputs[i].scale = (linked * 10000.0).round() / 10000.0;
                }
            }
            let primary_idx = self.outputs.iter().position(|o| o.primary).unwrap_or(0);
            self.master_scale = self.outputs[primary_idx].scale;
        } else if self.outputs[output_idx].primary {
            self.master_scale = clamped;
        }

        self.recalculate_layout();
    }

    /// Ajusta una pantalla sin modificar la escala de las demás.
    pub fn set_output_scale_independent(&mut self, output_idx: usize, new_scale: f64) {
        if output_idx >= self.outputs.len() {
            return;
        }
        self.linked_physical_1to1 = false;
        self.set_output_scale(output_idx, new_scale);
    }

    /// Intercambia el orden horizontal (Izquierda <-> Derecha) de los monitores
    pub fn swap_horizontal_order(&mut self) {
        if self.outputs.len() < 2 {
            return;
        }
        let mut order: Vec<usize> = (0..self.outputs.len()).collect();
        order.sort_by_key(|&idx| (self.outputs[idx].x, !self.outputs[idx].primary));
        order.reverse();

        let mut cursor_x = 0;
        for idx in order {
            self.outputs[idx].x = cursor_x;
            let (lw, _) = self.outputs[idx].logical_size();
            cursor_x += lw.max(100);
        }
    }

    /// Aplica un modo de alineación vertical entre las pantallas ("top", "center", "bottom", "desk")
    pub fn set_vertical_alignment(&mut self, mode: &str) {
        if self.outputs.is_empty() {
            return;
        }
        let max_h = self
            .outputs
            .iter()
            .map(|o| o.logical_size().1)
            .max()
            .unwrap_or(1080);

        match mode {
            "top" => {
                for o in &mut self.outputs {
                    o.y = 0;
                }
            }
            "center" => {
                for o in &mut self.outputs {
                    let (_, lh) = o.logical_size();
                    o.y = ((max_h - lh) / 2).max(0);
                }
            }
            "bottom" => {
                for o in &mut self.outputs {
                    let (_, lh) = o.logical_size();
                    o.y = (max_h - lh).max(0);
                }
            }
            "desk" => {
                for (i, o) in self.outputs.iter_mut().enumerate() {
                    if o.primary || i == 0 {
                        o.y = 519;
                    } else {
                        o.y = 0;
                    }
                }
            }
            _ => {}
        }
        self.recalculate_layout();
    }

    /// Desplaza verticalmente un monitor específico en `delta_y` píxeles lógicos
    pub fn adjust_vertical_offset(&mut self, output_idx: usize, delta_y: i32) {
        if output_idx >= self.outputs.len() {
            return;
        }
        self.outputs[output_idx].y += delta_y;
        if let Some(min_y) = self.outputs.iter().map(|o| o.y).min() {
            for o in &mut self.outputs {
                o.y = (o.y - min_y).clamp(0, 1200);
            }
        }
    }

    /// Define qué monitor es la pantalla principal
    pub fn set_primary_output(&mut self, primary_idx: usize) {
        if primary_idx >= self.outputs.len() {
            return;
        }
        for (i, o) in self.outputs.iter_mut().enumerate() {
            o.primary = i == primary_idx;
        }
        self.master_scale = self.outputs[primary_idx].scale;
        if self.linked_physical_1to1 {
            self.apply_1to1_link();
        } else {
            self.recalculate_layout();
        }
    }

    /// Guarda la configuración actual en `$HOME/.config/malus-de/monitors.toml`
    pub fn save(&self) -> std::io::Result<()> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let toml_path = PathBuf::from(&home).join(".config/malus-de/monitors.toml");
        if let Some(parent) = toml_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Configuración de Pantallas y Monitores para Malus DE\n# Soporte nativo para múltiples pantallas 4K con Enlace Físico 1:1.\n\n";
        std::fs::write(toml_path, format!("{}{}", header, toml_str))
    }

    /// Parsea la configuración de pantallas desde el formato XML de GNOME
    pub fn from_gnome_xml(xml: &str) -> Option<Self> {
        let config_blocks: Vec<&str> = xml.split("<configuration>").collect();
        let active_block = config_blocks.last()?;
        let parts: Vec<&str> = active_block.split("<logicalmonitor>").collect();
        if parts.len() <= 1 {
            return None;
        }

        let mut outputs = Vec::new();
        let mut master_scale = 1.666667;

        for part in &parts[1..] {
            let x = extract_tag_value(part, "x").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            let y = extract_tag_value(part, "y").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            let scale = extract_tag_value(part, "scale").and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
            let primary = extract_tag_value(part, "primary").map(|v| v == "yes").unwrap_or(false);
            let connector = extract_tag_value(part, "connector").unwrap_or_else(|| "default".to_string());
            let product = extract_tag_value(part, "product").unwrap_or_else(|| connector.clone());
            let width = extract_tag_value(part, "width").and_then(|v| v.parse::<u32>().ok()).unwrap_or(3840);
            let height = extract_tag_value(part, "height").and_then(|v| v.parse::<u32>().ok()).unwrap_or(2160);
            let rate = extract_tag_value(part, "rate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(60.0);

            let diagonal_inches = if product.contains("27") || product.contains("LG") {
                Some(27.0)
            } else if product.contains("24") || product.contains("Mi") {
                Some(24.0)
            } else {
                None
            };

            if primary {
                master_scale = scale;
            }

            outputs.push(DisplayOutputConfig {
                connector,
                name: product,
                width,
                height,
                refresh_rate: rate,
                modes: vec![DisplayModeConfig {
                    width,
                    height,
                    refresh_rate: rate,
                }],
                x,
                y,
                scale,
                primary,
                diagonal_inches,
            });
        }

        if outputs.is_empty() {
            None
        } else {
            let mut cfg = DisplaysConfig {
                linked_physical_1to1: true,
                master_scale,
                brightness: 80.0,
                screen_corner_radius: 32.0,
                window_corner_radius: 32.0,
                outputs,
            };
            cfg.apply_1to1_link();
            Some(cfg)
        }
    }

    /// Busca la configuración de un monitor por nombre de conector (p. ej. "DP-1" o "HDMI-A-1")
    pub fn find_for_connector(&self, connector: &str) -> Option<&DisplayOutputConfig> {
        let norm = connector.trim_start_matches("card1-").trim_start_matches("card0-");
        self.outputs.iter().find(|o| {
            let o_norm = o.connector.trim_start_matches("card1-").trim_start_matches("card0-");
            o_norm.eq_ignore_ascii_case(norm)
        })
    }
}

fn extract_tag_value(xml_snippet: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{}>", tag);
    let close_tag = format!("</{}>", tag);
    let start = xml_snippet.find(&open_tag)? + open_tag.len();
    let end = xml_snippet[start..].find(&close_tag)? + start;
    Some(xml_snippet[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_headroom_includes_magnification_and_bounce_space() {
        let config = DockConfig {
            icon_size: 48,
            magnification_max_size: 62,
            ..DockConfig::default()
        };

        assert_eq!(config.headroom_height(), 42);
    }

    #[test]
    fn cursor_config_reads_values_from_interface_section_only() {
        let settings = "[org/example]\ncursor-theme='wrong'\ncursor-size=99\n\
[org/gnome/desktop/interface]\ncursor-theme='Adwaita'\ncursor-size=32\n";

        assert_eq!(
            parse_cursor_config(settings),
            (Some("Adwaita".to_string()), Some(32))
        );
    }

    #[test]
    fn test_gtk_decoration_layout_is_set_left_without_dropping_settings() {
        let config_dir = std::env::temp_dir().join(format!(
            "malus-de-gtk-settings-test-{}",
            std::process::id()
        ));
        let gtk3_dir = config_dir.join("gtk-3.0");
        let gtk4_dir = config_dir.join("gtk-4.0");
        std::fs::create_dir_all(&gtk3_dir).unwrap();
        std::fs::create_dir_all(&gtk4_dir).unwrap();
        std::fs::write(
            gtk3_dir.join("settings.ini"),
            "[Settings]\ngtk-decoration-layout=:close\ncustom-option=true\n",
        )
        .unwrap();
        std::fs::write(gtk4_dir.join("settings.ini"), "[Settings]\ncustom-option=true\n").unwrap();

        set_gtk_decoration_layout(&config_dir).unwrap();

        for version in ["gtk-3.0", "gtk-4.0"] {
            let settings = std::fs::read_to_string(config_dir.join(version).join("settings.ini")).unwrap();
            assert!(settings.contains("gtk-decoration-layout=close,minimize,maximize:"));
            assert!(settings.contains("custom-option=true"));
        }
        std::fs::remove_dir_all(config_dir).unwrap();
    }

    #[test]
    fn test_nautilus_windowcontrols_preserve_traffic_lights_colors() {
        for dark in [false, true] {
            let css = get_malus_gtk_css(dark);
            assert!(css.contains("window.view .sidebar-pane windowcontrols > button.close"));
            assert!(css.contains("window.view .sidebar-pane windowcontrols > button.minimize"));
            assert!(css.contains("window.view .sidebar-pane windowcontrols > button.maximize"));
            assert!(css.contains("background-color: #ff5f56;"));
            assert!(css.contains("background-color: #ffbd2e;"));
            assert!(css.contains("background-color: #27c93f;"));
        }
    }

    #[test]
    fn test_gtk4_transparency_is_scoped_to_nautilus_and_menus_keep_their_background() {
        let css = get_malus_gtk_css(false);

        assert!(css.contains("window.view.background"));
        assert!(css.contains("window.view overlay-split-view > .sidebar-pane"));
        assert!(css.contains("window.view .sidebar-pane *"));
        assert!(css.contains("window.view popover > contents"));
        assert!(css.contains(
            "background-color: transparent;\n    border-color: transparent;\n    box-shadow: none;"
        ));
        assert!(
            css.rfind("window.view popover > contents").unwrap()
                > css.find("window.view popover > contents").unwrap()
        );
        assert!(css.contains("border-radius: 0;\n    margin: 0;\n    padding: 5px;"));
        assert!(css.contains("min-height: 24px;\n    padding: 3px 8px;"));
        assert!(!css.contains("background-color: alpha(@window_bg_color, 0.84);"));
        assert!(!css.contains("\nwindow.background {\n"));
        assert!(!css.contains("\n.sidebar-pane,\n"));
    }

    #[test]
    fn nautilus_popup_background_overrides_transparency_in_both_themes() {
        for (dark, background, opposite_media, opposite_bg) in [
            (
                true,
                "background-color: rgba(30, 30, 36, 0.78);",
                "@media (prefers-color-scheme: light)",
                "background-color: rgba(244, 246, 250, 0.76);",
            ),
            (
                false,
                "background-color: rgba(244, 246, 250, 0.76);",
                "@media (prefers-color-scheme: dark)",
                "background-color: rgba(30, 30, 36, 0.78);",
            ),
        ] {
            let css = get_malus_gtk_css(dark);
            let squircle_rule = css.find("window.view popover > contents").unwrap();
            let media_idx = css.find(opposite_media).unwrap();

            assert!(media_idx > squircle_rule);
            assert!(css[squircle_rule..media_idx].contains(background));
            assert!(css[media_idx..].contains(opposite_bg));
            assert!(css.contains("background-color: rgba(0, 122, 255, 0.70);"));
        }
    }

    #[test]
    fn popup_background_and_border_follow_the_active_theme_in_both_gtk_versions() {
        let dark_gtk4 = get_malus_gtk_css(true);
        let light_gtk4 = get_malus_gtk_css(false);
        let dark_gtk3 = get_malus_gtk3_css(true);
        let light_gtk3 = get_malus_gtk3_css(false);

        for (css, bg, fg) in [
            (&dark_gtk4, "rgba(30, 30, 36, 0.78)", "#f4f4f5"),
            (&light_gtk4, "rgba(244, 246, 250, 0.76)", "#1d1d1f"),
            (&dark_gtk3, "rgba(30, 30, 36, 0.78)", "#f4f4f5"),
            (&light_gtk3, "rgba(244, 246, 250, 0.76)", "#1d1d1f"),
        ] {
            assert!(css.contains(&format!("background-color: {bg};")));
            assert!(css.contains(&format!("color: {fg};")));
            assert!(css.contains("border: none;"));
            assert!(css.contains("box-shadow: none;"));
            assert!(css.contains("padding: 5px;"));
            assert!(css.contains("background-color: rgba(0, 122, 255, 0.70);"));
        }
    }

    #[test]
    fn test_gtk3_sidebar_background_is_opaque() {
        let css = get_malus_gtk3_css(false);

        assert!(css.contains("@define-color sidebar_bg_color rgb(246, 246, 248);"));
        assert!(css.contains(
            "placessidebar,\nplacessidebar.view {\n    background-color: rgb(246, 246, 248);"
        ));
    }

    #[test]
    fn test_dock_config_default_serialization() {
        let default_cfg = DockConfig::default();
        let toml_str = toml::to_string_pretty(&default_cfg).expect("Serialización falló");
        let parsed: DockConfig = toml::from_str(&toml_str).expect("Deserialización falló");
        assert_eq!(parsed.apps.len(), 5);
        assert_eq!(parsed.icon_size, 48);
        assert_eq!(parsed.magnification_max_size, 62);
        assert_eq!(parsed.floating_gap, 4);
        assert_eq!(parsed.position, DockPosition::Bottom);
    }

    #[test]
    fn test_dock_config_partial_deserialization() {
        let partial_toml = r#"
            icon_size = 56
            magnification_enabled = false
            floating_gap = 14

            [[apps]]
            name = "Test App"
            icon = "test-icon"
            command = "test-cmd"
            matchers = ["testapp"]
        "#;
        let parsed: DockConfig = toml::from_str(partial_toml).expect("Deserialización parcial falló");
        assert_eq!(parsed.icon_size, 56);
        assert!(!parsed.magnification_enabled);
        assert_eq!(parsed.floating_gap, 14);
        assert_eq!(parsed.corner_radius, 24); // default
        assert_eq!(parsed.apps.len(), 1);
        assert_eq!(parsed.apps[0].name, "Test App");
        assert_eq!(parsed.apps[0].icon_appearance, DockIconAppearance::Auto);
    }

    #[test]
    fn test_dock_app_icon_appearance_override() {
        let config = r#"
            [[apps]]
            name = "Native Icon"
            icon = "native-icon"
            command = "native-app"
            icon_appearance = "native"
        "#;

        let parsed: DockConfig = toml::from_str(config).expect("Deserialización falló");
        assert_eq!(parsed.apps[0].icon_appearance, DockIconAppearance::Native);

        let serialized = toml::to_string(&parsed).expect("Serialización falló");
        let reparsed: DockConfig = toml::from_str(&serialized).expect("Deserialización falló");
        assert_eq!(reparsed.apps[0].icon_appearance, DockIconAppearance::Native);
    }

    #[test]
    fn test_dock_config_pin_unpin_move() {
        let mut cfg = DockConfig::default();
        let initial_count = cfg.apps.len();

        // Pin new app
        let new_app = DockAppConfig {
            name: "Spotify".to_string(),
            icon: "spotify".to_string(),
            command: "spotify".to_string(),
            matchers: vec!["spotify".into()],
            icon_appearance: DockIconAppearance::Auto,
        };
        assert!(cfg.pin_app(new_app.clone()));
        assert_eq!(cfg.apps.len(), initial_count + 1);
        // Duplicate pin should return false
        assert!(!cfg.pin_app(new_app));

        // Move app
        let last_idx = cfg.apps.len() - 1;
        assert!(cfg.move_app(last_idx, 0));
        assert_eq!(cfg.apps[0].name, "Spotify");

        // Unpin app
        assert!(cfg.unpin_app("Spotify"));
        assert_eq!(cfg.apps.len(), initial_count);
        assert!(!cfg.unpin_app("NonExistentApp"));
    }

    #[test]
    fn test_dock_app_config_matches() {
        let app = DockAppConfig {
            name: "Terminal".to_string(),
            icon: "utilities-terminal".to_string(),
            command: "ptyxis -s".to_string(),
            matchers: vec!["ptyxis".into(), "terminal".into(), "org.gnome.ptyxis".into()],
            icon_appearance: DockIconAppearance::Auto,
        };

        assert!(app.matches("Terminal"));
        assert!(app.matches("ptyxis"));
        assert!(app.matches("org.gnome.ptyxis.something"));
        assert!(!app.matches("firefox"));
    }

    #[test]
    fn test_dock_config_load_or_create_default() {
        let cfg = DockConfig::load_or_create_default();
        let path = DockConfig::config_path();
        assert!(path.exists());
        assert!(!cfg.apps.is_empty());
        assert_eq!(DockConfig::default().apps.len(), 5);
        assert!(cfg.icon_size > 0);
    }

    #[test]
    fn test_displays_config_brightness() {
        let mut cfg = DisplaysConfig::default();
        assert_eq!(cfg.brightness, 80.0);
        cfg.brightness = 95.0;
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: DisplaysConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.brightness, 95.0);
    }

    #[test]
    fn test_displays_config_1to1_and_arrangement() {
        let mut cfg = DisplaysConfig::default();
        cfg.outputs = vec![
            DisplayOutputConfig {
                connector: "DP-1".to_string(),
                name: "Monitor 27".to_string(),
                width: 3840,
                height: 2160,
                refresh_rate: 60.0,
                modes: Vec::new(),
                x: 0,
                y: 0,
                scale: 1.5,
                primary: true,
                diagonal_inches: Some(27.0),
            },
            DisplayOutputConfig {
                connector: "HDMI-A-1".to_string(),
                name: "Monitor 24".to_string(),
                width: 3840,
                height: 2160,
                refresh_rate: 60.0,
                modes: Vec::new(),
                x: 2560,
                y: 0,
                scale: 1.5,
                primary: false,
                diagonal_inches: Some(24.0),
            },
        ];
        cfg.outputs[0].diagonal_inches = Some(27.0);
        cfg.outputs[1].diagonal_inches = Some(24.0);
        assert!(cfg.linked_physical_1to1);
        cfg.set_output_scale(0, 1.5);
        // Monitor 0 is 27" 4K (163.18 PPI) -> scale 1.50 (logical width 2560)
        // Monitor 1 is 24" 4K (183.58 PPI) -> scale 1.50 * (27/24) = 1.6875 (logical width 2276)
        assert!((cfg.outputs[0].scale - 1.5).abs() < 0.001);
        assert!((cfg.outputs[1].scale - 1.6875).abs() < 0.001);
        assert_eq!(cfg.outputs[0].x, 0);
        assert_eq!(cfg.outputs[1].x, 2560);

        // Swap horizontal order
        cfg.swap_horizontal_order();
        assert_eq!(cfg.outputs[1].x, 0);
        assert_eq!(cfg.outputs[0].x, 2276);

        // Editing an individual output automatically unlinks the scales.
        cfg.set_output_scale_independent(1, 2.0);
        assert!(!cfg.linked_physical_1to1);
        assert!((cfg.outputs[1].scale - 2.0).abs() < 0.001);
        assert!((cfg.outputs[0].scale - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_input_config_latam_default() {
        let cfg = InputConfig::default();
        assert_eq!(cfg.keyboard_layout, "latam");
        assert_eq!(cfg.layout_short_badge(), "LAT");
    }

    #[test]
    fn test_displays_config_screen_corner_radius_default() {
        let cfg = DisplaysConfig::default();
        assert!(cfg.outputs.is_empty());
        assert_eq!(cfg.screen_corner_radius, 32.0);
        assert_eq!(cfg.window_corner_radius, 32.0);
    }

    #[test]
    fn test_display_connector_matching_is_exact() {
        let mut cfg = DisplaysConfig::default();
        cfg.outputs.push(DisplayOutputConfig {
            connector: "DP-1".to_string(),
            name: "DisplayPort".to_string(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            modes: Vec::new(),
            x: 0,
            y: 0,
            scale: 1.0,
            primary: true,
            diagonal_inches: None,
        });
        assert!(cfg.find_for_connector("card0-DP-1").is_some());
        assert!(cfg.find_for_connector("DP-2").is_none());
    }

    #[test]
    fn test_legacy_display_config_without_modes_still_loads() {
        let legacy = r#"
connector = "DP-1"
name = "DisplayPort"
width = 3840
height = 2160
refresh_rate = 60.0
x = 0
y = 0
scale = 2.0
primary = true
"#;
        let output: DisplayOutputConfig = toml::from_str(legacy).unwrap();
        assert!(output.modes.is_empty());
        assert_eq!(output.width, 3840);
    }

    #[test]
    fn test_setup_electron_ide_configurations_creates_and_updates_settings() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!("malus-test-ide-{}", unique));
        let _ = std::fs::remove_dir_all(&config_dir);
        std::fs::create_dir_all(&config_dir).unwrap();

        // 1. Probar creación limpia
        setup_electron_ide_configurations(&config_dir);

        let antigravity_settings = config_dir.join("Antigravity IDE/User/settings.json");
        assert!(antigravity_settings.exists());
        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&antigravity_settings).unwrap()).unwrap();
        assert_eq!(content["window.titleBarStyle"], "custom");
        assert_eq!(content["window.controlsStyle"], "custom");

        // 2. Probar actualización preservando otras configuraciones del usuario
        let code_user_dir = config_dir.join("Code/User");
        std::fs::create_dir_all(&code_user_dir).unwrap();
        let code_settings = code_user_dir.join("settings.json");
        std::fs::write(
            &code_settings,
            r#"{"editor.fontSize": 14, "window.controlsStyle": "native"}"#,
        )
        .unwrap();

        setup_electron_ide_configurations(&config_dir);

        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&code_settings).unwrap()).unwrap();
        assert_eq!(updated["editor.fontSize"], 14);
        assert_eq!(updated["window.controlsStyle"], "custom");
        assert_eq!(updated["window.titleBarStyle"], "custom");

        let codium_settings = config_dir.join("VSCodium/User/settings.json");
        assert!(codium_settings.exists());

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn test_resolve_electron_ide_wrapper() {
        assert!(resolve_electron_ide_wrapper("firefox").is_none());
        assert!(resolve_electron_ide_wrapper("nautilus").is_none());
        // Debe reconocer comandos válidos si existe el wrapper en ~/.local/bin o /usr/local/bin
        if let Some(wrapper) = resolve_electron_ide_wrapper("antigravity-ide") {
            assert!(wrapper.to_string_lossy().contains("wrapper"));
        }
        if let Some(wrapper) = resolve_electron_ide_wrapper("code") {
            assert!(wrapper.to_string_lossy().contains("wrapper"));
        }
    }

    #[test]
    fn test_setup_firefox_configuration_creates_and_updates_profiles() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!("malus-test-firefox-{}", unique));
        let _ = std::fs::remove_dir_all(&config_dir);
        std::fs::create_dir_all(&config_dir).unwrap();

        // 1. Probar inicialización limpia (sin perfiles previos)
        setup_firefox_configuration(&config_dir);

        let default_css = config_dir.join("mozilla/firefox/default-release/chrome/userChrome.css");
        assert!(default_css.exists());
        let css_content = std::fs::read_to_string(&default_css).unwrap();
        assert!(css_content.contains("Malus DE - macOS Traffic Light Controls for Mozilla Firefox"));
        assert!(css_content.contains(".titlebar-close"));

        let default_user_js = config_dir.join("mozilla/firefox/default-release/user.js");
        assert!(default_user_js.exists());
        let user_js_content = std::fs::read_to_string(&default_user_js).unwrap();
        assert!(user_js_content.contains("toolkit.legacyUserProfileCustomizations.stylesheets"));

        // 2. Probar actualización en perfil existente
        let custom_profile = config_dir.join("mozilla/firefox/custom.profile");
        std::fs::create_dir_all(&custom_profile).unwrap();
        let custom_prefs = custom_profile.join("prefs.js");
        std::fs::write(&custom_prefs, "// Dummy prefs\n").unwrap();
        let custom_user_js = custom_profile.join("user.js");
        std::fs::write(&custom_user_js, "user_pref(\"browser.startup.page\", 3);\n").unwrap();

        setup_firefox_configuration(&config_dir);

        let custom_css = custom_profile.join("chrome/userChrome.css");
        assert!(custom_css.exists());
        let updated_user_js = std::fs::read_to_string(&custom_user_js).unwrap();
        assert!(updated_user_js.contains("browser.startup.page"));
        assert!(updated_user_js.contains("toolkit.legacyUserProfileCustomizations.stylesheets"));

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn test_setup_default_mime_applications_registers_package_managers_and_preserves_user_apps() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!("malus-test-mime-{}", unique));
        let _ = std::fs::remove_dir_all(&config_dir);
        std::fs::create_dir_all(&config_dir).unwrap();

        // 1. Inicializar con archivo preexistente que contiene preferencias de usuario
        let initial_mime = r#"[Default Applications]
text/html=brave-origin.desktop
image/png=org.gnome.Loupe.desktop

[Added Associations]
image/png=org.gnome.Loupe.desktop;
"#;
        std::fs::write(config_dir.join("mimeapps.list"), initial_mime).unwrap();

        setup_default_mime_applications(&config_dir);

        let content = std::fs::read_to_string(config_dir.join("mimeapps.list")).unwrap();
        // Verificar que preserva las asociaciones existentes del usuario
        assert!(content.contains("text/html=brave-origin.desktop"));
        assert!(content.contains("image/png=org.gnome.Loupe.desktop"));
        // Verificar que agrega las asociaciones de paquetes hacia GNOME Software
        assert!(content.contains("application/x-rpm=gnome-software-local-file-packagekit.desktop"));
        assert!(content.contains("application/x-redhat-package-manager=gnome-software-local-file-packagekit.desktop"));
        assert!(content.contains("application/vnd.flatpak=gnome-software-local-file-flatpak.desktop"));
        assert!(content.contains("application/vnd.flatpak.ref=gnome-software-local-file-flatpak.desktop"));
        assert!(content.contains("application/vnd.flatpak.repo=gnome-software-local-file-flatpak.desktop"));
        assert!(content.contains("application/x-deb=gnome-software-local-file-packagekit.desktop"));

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn test_ensure_keyfile_button_layout_enforces_left_traffic_lights() {
        let temp_dir = std::env::temp_dir().join("malus_test_keyfile_isolation");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let keyfile_path = temp_dir.join("keyfile");

        // Caso 1: Archivo con sección GNOME modificada por GNOME Tweaks (controles a la derecha)
        let gnometweaks_content = r#"[org/gnome/desktop/wm/preferences]
button-layout=':minimize,maximize,close'

[org/gnome/desktop/interface]
color-scheme='prefer-dark'
gtk-theme='Adwaita'
"#;
        std::fs::write(&keyfile_path, gnometweaks_content).unwrap();

        ensure_keyfile_button_layout(&keyfile_path).unwrap();

        let updated = std::fs::read_to_string(&keyfile_path).unwrap();
        assert!(updated.contains("button-layout='close,minimize,maximize:'"));
        assert!(!updated.contains(":minimize,maximize,close"));
        assert!(updated.contains("color-scheme='prefer-dark'"));
        assert!(updated.contains("gtk-theme='Adwaita'"));

        // Caso 2: Archivo sin sección de wm preferences
        let no_wm_content = r#"[org/gnome/desktop/interface]
color-scheme='prefer-dark'
"#;
        std::fs::write(&keyfile_path, no_wm_content).unwrap();
        ensure_keyfile_button_layout(&keyfile_path).unwrap();

        let updated2 = std::fs::read_to_string(&keyfile_path).unwrap();
        assert!(updated2.contains("[org/gnome/desktop/wm/preferences]"));
        assert!(updated2.contains("button-layout='close,minimize,maximize:'"));
        assert!(updated2.contains("color-scheme='prefer-dark'"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_write_file_atomic_if_changed_skips_unchanged_and_updates_modified() {
        let temp_dir = std::env::temp_dir().join("malus_test_atomic_write");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let file_path = temp_dir.join("sub").join("config.css");

        // Primera escritura: crea directorios padre y devuelve true
        assert!(write_file_atomic_if_changed(&file_path, "body { color: white; }\n").unwrap());
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "body { color: white; }\n"
        );

        // Contenido idéntico: no-op, devuelve false y no deja archivos temporales
        assert!(!write_file_atomic_if_changed(&file_path, "body { color: white; }\n").unwrap());

        // Contenido distinto: actualiza atómicamente y devuelve true
        assert!(write_file_atomic_if_changed(&file_path, "body { color: black; }\n").unwrap());
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "body { color: black; }\n"
        );

        let left_tmp = std::fs::read_dir(temp_dir.join("sub"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!left_tmp);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_upsert_keyfile_entries_preserves_unrelated_keys_and_skips_noop() {
        let temp_dir = std::env::temp_dir().join("malus_test_upsert_keyfile");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let keyfile_path = temp_dir.join("keyfile");

        let initial = r#"[org/gnome/desktop/wm/preferences]
button-layout='close,minimize,maximize:'
focus-mode='click'

[org/gnome/desktop/interface]
cursor-theme='macOS'
color-scheme='prefer-light'

[org/gnome/settings-daemon/plugins/xsettings]
overrides={'Gtk/ShellShowsAppMenu': <1>}
"#;
        std::fs::write(&keyfile_path, initial).unwrap();

        let changed = upsert_keyfile_entries(
            &keyfile_path,
            &[(
                "org/gnome/desktop/interface",
                &[("color-scheme", "'prefer-dark'"), ("gtk-theme", "'Adwaita'")],
            )],
        )
        .unwrap();
        assert!(changed);

        let after = std::fs::read_to_string(&keyfile_path).unwrap();
        assert!(after.contains("color-scheme='prefer-dark'"));
        assert!(after.contains("gtk-theme='Adwaita'"));
        assert!(after.contains("cursor-theme='macOS'"));
        assert!(after.contains("focus-mode='click'"));
        assert!(after.contains("overrides={'Gtk/ShellShowsAppMenu': <1>}"));

        // Segunda llamada idéntica: devuelve false (sin reescritura ni inotify extra)
        let changed_again = upsert_keyfile_entries(
            &keyfile_path,
            &[(
                "org/gnome/desktop/interface",
                &[("color-scheme", "'prefer-dark'"), ("gtk-theme", "'Adwaita'")],
            )],
        )
        .unwrap();
        assert!(!changed_again);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
