mod app_window;
mod db;
mod folder_pane;
mod fs_backend;
mod path_bar;
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

const APP_ID: &str = "dev.malus.Branches";

#[derive(Parser, Debug, Clone)]
#[command(name = "malus-de-branches")]
#[command(about = "Branches - Explorador de Archivos Nativo de Malus DE (inspirado fielmente en macOS Finder)")]
struct Args {
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
