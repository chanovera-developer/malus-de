mod app_window;
mod db;
mod folder_pane;
mod fs_backend;
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
use gtk4::prelude::*;
use gtk4::{Application, CssProvider};
use malus_de_common::logging::init_logging;
use std::path::PathBuf;

use quick_look::QuickLook;
use fs_backend::FileItem;

const APP_ID: &str = "dev.malus.Branches";

#[derive(Parser, Debug, Clone)]
#[command(name = "malus-de-branches")]
#[command(about = "Branches - Explorador de Archivos Nativo de Malus DE (inspirado fielmente en macOS Finder)")]
struct Args {
    /// Abrir previsualización Quick Look para un archivo específico
    #[arg(short = 'q', long = "quick-look", value_name = "ARCHIVO")]
    quick_look: Option<PathBuf>,

    /// Ruta del directorio inicial a abrir (por defecto: $HOME)
    #[arg(value_name = "RUTA")]
    path: Option<PathBuf>,
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

    let initial_path = args.path.unwrap_or_else(|| {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"))
    });

    let app = Application::builder()
        .application_id(APP_ID)
        .build();

    let init_path_clone = initial_path.clone();
    app.connect_startup(|_| {
        load_custom_css();
    });

    app.connect_activate(move |app| {
        let window = BranchesWindow::new(app, init_path_clone.clone());
        window.present();
    });

    app.run_with_args::<&str>(&[]);
}

fn load_custom_css() {
    let provider = CssProvider::new();
    let css = include_str!("styles/macos-branches.css");
    provider.load_from_string(css);

    if let Some(display) = Display::default() {
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
        assert!(css.contains("opacity: 0.0 !important;"));
        assert!(css.contains("transition: none !important;"));
        assert!(css.contains(".folder-drop-hover"));
        assert!(css.contains(".branches-icon-item.landing"));
        assert!(css.contains("@keyframes macos-drop-landing"));
        assert!(css.contains("cubic-bezier(0.16, 1, 0.3, 1)"));
    }
}
