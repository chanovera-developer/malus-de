mod app_window;
mod db;
mod folder_pane;
mod fs_backend;
mod info_window;
mod job_ui;
mod jobs;
mod network;
mod operations;
mod path_bar;
mod quick_look;
mod sidebar;
mod state;
mod status_bar;
mod views;

use app_window::BranchesWindow;
use clap::Parser;
use gtk4::gdk::Display;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{Application, CssProvider};
use malus_de_common::logging::init_logging;
use std::path::{Path, PathBuf};

use quick_look::QuickLook;
use fs_backend::{percent_decode_file_uri, FileItem};

const APP_ID: &str = "dev.malus.Branches";

#[derive(Parser, Debug, Clone)]
#[command(name = "malus-de-branches")]
#[command(about = "Branches - Explorador de Archivos Nativo de Malus DE (inspirado fielmente en macOS Finder)")]
struct Args {
    /// Abrir previsualización Quick Look para un archivo específico
    #[arg(short = 'q', long = "quick-look", value_name = "ARCHIVO")]
    quick_look: Option<PathBuf>,

    /// Abrir una nueva ventana de Branches aunque ya exista una instancia abierta
    #[arg(long = "new-window")]
    new_window: bool,

    /// Ruta o URI inicial a abrir (ej: /ruta, file:///..., trash:///, por defecto: $HOME)
    #[arg(value_name = "RUTA")]
    path: Option<String>,
}

fn trash_files_dir(home: &Path, xdg_data_home: Option<&Path>) -> PathBuf {
    let data_home = xdg_data_home
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".local/share"));
    data_home.join("Trash/files")
}

fn is_trash_uri(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.eq_ignore_ascii_case("trash://")
        || trimmed.eq_ignore_ascii_case("trash:///")
        || trimmed.to_ascii_lowercase().starts_with("trash:///")
}

fn resolve_cli_target_path(raw: &str, home: &Path, xdg_data_home: Option<&Path>) -> PathBuf {
    let trimmed = raw.trim();
    if is_trash_uri(trimmed) {
        return trash_files_dir(home, xdg_data_home);
    }
    if trimmed.starts_with("file://") {
        if let Some(decoded) = percent_decode_file_uri(trimmed) {
            return decoded;
        }
    }
    if trimmed == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(trimmed)
}

fn main() {
    init_logging("malus_de_branches");
    let _ = malus_de_common::config::setup_isolated_session_environment();

    let _ = gtk4::init();
    if let Err(err) = libadwaita::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let args = Args::parse();

    if let Some(ql_path) = args.quick_look {
        let app = Application::builder()
            .application_id("dev.malus.Branches.QuickLook")
            .build();
        app.connect_startup(|_| {
            load_custom_css();
        });
        app.connect_activate(move |app| {
            if let Some(item) = FileItem::from_path(&ql_path) {
                let ql = QuickLook::new();
                ql.show_standalone(app, &item);
            } else {
                tracing::warn!("No se pudo crear vista Quick Look para {:?}", ql_path);
            }
        });
        app.run_with_args::<&str>(&[]);
        return;
    }

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let requested_arg = if let Some(raw) = args.path.as_deref() {
        let resolved = resolve_cli_target_path(raw, &home, xdg_data_home.as_deref());
        if is_trash_uri(raw) {
            let _ = std::fs::create_dir_all(&resolved);
        }
        Some(resolved)
    } else if args.new_window {
        Some(home.clone())
    } else {
        None
    };

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let default_home = home.clone();
    app.connect_startup(|_| {
        load_custom_css();
    });

    app.connect_activate(move |app| {
        let window = BranchesWindow::new(app, default_home.clone());
        window.present();
    });

    let open_home = home;
    let open_data_home = xdg_data_home;
    app.connect_open(move |app, files, _hint| {
        for file in files {
            let uri = file.uri();
            let resolved = if is_trash_uri(uri.as_str()) {
                let trash = trash_files_dir(&open_home, open_data_home.as_deref());
                let _ = std::fs::create_dir_all(&trash);
                trash
            } else if let Some(path) = file.path() {
                path
            } else {
                resolve_cli_target_path(uri.as_str(), &open_home, open_data_home.as_deref())
            };
            let window = BranchesWindow::new(app, resolved);
            window.present();
        }
    });

    if let Some(target_path) = requested_arg {
        let arg_str = target_path.to_string_lossy().to_string();
        app.run_with_args(&["malus-de-branches", arg_str.as_str()]);
    } else {
        app.run_with_args::<&str>(&[]);
    }
}

fn load_custom_css() {
    let provider = CssProvider::new();
    let css = include_str!("styles/macos-branches.css");
    provider.load_from_string(css);

    if let Some(display) = Display::default() {
        let icon_theme = gtk4::IconTheme::for_display(&display);
        for icon_dir in malus_de_common::apps::extra_icon_search_paths() {
            if icon_dir.is_dir() {
                icon_theme.add_search_path(&icon_dir);
            }
        }
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_branches_macos_drag_and_drop_styles() {
        let css = include_str!("styles/macos-branches.css");
        assert!(css.contains(".branches-icon-dragging"));
        assert!(css.contains("opacity: 0.0;"));
        assert!(css.contains("transition: none;"));
        assert!(!css.contains("!important"));
        assert!(css.contains(".folder-drop-hover"));
        assert!(css.contains(".branches-icon-item.landing"));
        assert!(css.contains("@keyframes macos-drop-landing"));
        assert!(css.contains("cubic-bezier(0.16, 1, 0.3, 1)"));
        assert!(css.contains(".branches-icon-canvas:drop(active)"));
        assert!(css.contains(".branches-sidebar-scroll:drop(active)"));
        let sidebar_hover_block = css
            .split(".branches-sidebar-row.folder-drop-hover {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir .branches-sidebar-row.folder-drop-hover");
        assert!(sidebar_hover_block.contains("transform: none;"));
        assert!(sidebar_hover_block.contains("border: none;"));
        assert!(css.contains(".branches-folder-pane.branches-split-mode.branches-pane-active"));
        assert!(css.contains("border: 1.5px solid alpha(@accent_bg_color, 0.55);"));
        assert!(!css.contains("outline: 2px solid alpha(@accent_bg_color, 0.55);"));
    }

    #[test]
    fn test_branches_finder_sidebar_and_toolbar_pills() {
        let css = include_str!("styles/macos-branches.css");
        // Secciones de barra lateral en Title Case (sin text-transform: uppercase)
        let title_block = css
            .split(".branches-sidebar-section-title")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir .branches-sidebar-section-title");
        assert!(!title_block.contains("text-transform: uppercase"));

        // Colores de iconos en barra lateral (Favoritos azul macOS, Ubicaciones gris neutro)
        assert!(css.contains(".branches-sidebar-row-favorite .branches-sidebar-icon"));
        assert!(css.contains("#0a84ff"));
        assert!(css.contains(".branches-sidebar-row-location .branches-sidebar-icon"));

        // Contenedores squircle afilados en barra superior con altura uniforme + input 999px
        let pill_block = css
            .split(".branches-toolbar-pill {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir .branches-toolbar-pill");
        assert!(pill_block.contains("border-radius: 8px;"));

        let search_btn_block = css
            .split("button.branches-search-btn {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir button.branches-search-btn");
        assert!(search_btn_block.contains("border-radius: 8px;"));

        let search_entry_block = css
            .split(".branches-search-entry {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir .branches-search-entry");
        assert!(search_entry_block.contains("border-radius: 999px;"));

        assert!(css.contains("border-radius: 5px;"));
        assert!(css.contains("menubutton.branches-tool-btn"));
        assert!(!css.contains("\nbutton.branches-tool-btn"));

        let nav_btn_block = css
            .split("button.branches-nav-btn {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir button.branches-nav-btn");
        assert!(nav_btn_block.contains("min-height: 26px;"));
        assert!(nav_btn_block.contains("min-width: 26px;"));
        assert_eq!(nav_btn_block.contains("padding: 0;"), true);

        assert!(css.contains(".branches-quicklook-backdrop"));
        assert!(css.contains(".branches-quicklook-container.ql-open"));
        assert!(css.contains(".branches-quicklook-container.ql-closing"));
        assert!(css.contains("transform: scale(0.84);"));
        assert!(css.contains("transform: scale(1.0);"));
    }

    #[test]
    fn test_resolve_cli_target_path_handles_trash_file_uri_and_tilde() {
        use clap::Parser;
        use super::resolve_cli_target_path;
        use std::path::{Path, PathBuf};

        let home = Path::new("/home/cristian");
        let custom_data = Path::new("/tmp/xdg-data");

        assert_eq!(
            resolve_cli_target_path("trash://", home, None),
            PathBuf::from("/home/cristian/.local/share/Trash/files")
        );
        assert_eq!(
            resolve_cli_target_path("trash:///", home, Some(custom_data)),
            PathBuf::from("/tmp/xdg-data/Trash/files")
        );
        assert_eq!(
            resolve_cli_target_path("file:///home/cristian/Mi%20Carpeta", home, None),
            PathBuf::from("/home/cristian/Mi Carpeta")
        );
        assert_eq!(
            resolve_cli_target_path("~/Descargas", home, None),
            PathBuf::from("/home/cristian/Descargas")
        );

        let parsed = super::Args::try_parse_from(["malus-de-branches", "--new-window"])
            .expect("--new-window debe ser aceptado por clap");
        assert!(parsed.new_window);
        assert!(parsed.path.is_none());
    }

    #[test]
    fn test_branches_tab_bar_styles_and_context_menu_wiring() {
        let css = include_str!("styles/macos-branches.css");
        assert!(css.contains(".branches-hidden-item"));
        assert!(css.contains(".branches-cut-item"));
        assert!(css.contains(".branches-tab-bar"));
        assert!(css.contains(".branches-tabs-strip"));
        assert!(css.contains(".branches-tab.branches-tab-active"));
        assert!(css.contains("button.branches-tab-close-btn"));
        assert!(css.contains("button.branches-tab-add-btn"));
        // El botón de nueva pestaña debe tener las mismas dimensiones que el botón de búsqueda (30x30px)
        assert!(css.contains("button.branches-tab-add-btn {\n    min-width: 30px;\n    min-height: 30px;"));
        // Las pestañas deben tener la misma altura que los botones internos de vista (26px)
        assert!(css.contains(".branches-tab {\n    min-height: 26px;"));
        // El botón de cerrar pestaña debe ser compacto (14x14px), oculto por defecto y visible en hover
        assert!(css.contains("button.branches-tab-close-btn {\n    min-width: 14px;\n    min-height: 14px;"));
        assert!(css.contains(".branches-tab:hover button.branches-tab-close-btn"));
        assert!(css.contains(".branches-tab-spacer"));

        let app_window_src = include_str!("app_window.rs");
        assert!(app_window_src.contains("\"open_in_new_tab\""));
        assert!(app_window_src.contains("\"new_tab\""));
        assert!(app_window_src.contains("open_target_in_new_tab"));
        assert!(app_window_src.contains("\"copy_item\""));
        assert!(app_window_src.contains("\"cut_item\""));
        assert!(app_window_src.contains("\"paste_into\""));
        assert!(app_window_src.contains("refresh_all_cut_states"));
        assert!(app_window_src.contains("SUPER_MASK"));
        assert!(app_window_src.contains("META_MASK"));

        let views_src = include_str!("views/mod.rs");
        assert!(views_src.contains("Abrir en nueva pestaña"));
        assert!(views_src.contains("win.open_in_new_tab"));
        assert!(views_src.contains("win.copy_item"));
        assert!(views_src.contains("win.cut_item"));
        assert!(views_src.contains("win.paste_into"));

        let sidebar_src = include_str!("sidebar.rs");
        assert!(sidebar_src.contains("Abrir en nueva pestaña"));
        assert!(sidebar_src.contains("win.open_in_new_tab"));
        assert!(sidebar_src.contains("@recents"));
        assert!(sidebar_src.contains("@applications"));
    }
}
