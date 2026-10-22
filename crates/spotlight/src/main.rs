use gtk4::gdk::Key;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, CssProvider, Entry, EventControllerKey,
    Image, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::logging::init_logging;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use tracing::info;

mod calculator;
mod indexer;

const SPOTLIGHT_CSS: &str = r#"
window.malus-spotlight-window {
    background-color: transparent;
}

.spotlight-card {
    background-color: rgba(24, 24, 27, 0.88);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 18px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.65);
    padding: 10px;
    min-width: 640px;
}

.spotlight-search-box {
    background-color: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 12px;
    padding: 8px 12px;
    margin-bottom: 8px;
}

.spotlight-entry,
entry.spotlight-entry,
entry.spotlight-entry text {
    background: transparent;
    border: none;
    color: #ffffff;
    caret-color: #3b82f6;
    font-size: 18px;
    font-weight: 500;
    box-shadow: none;
}

entry.spotlight-entry text selection {
    background-color: #2563eb;
    color: #ffffff;
}

entry.spotlight-entry placeholder {
    color: rgba(255, 255, 255, 0.45);
}

.spotlight-entry:focus,
entry.spotlight-entry:focus {
    box-shadow: none;
}

.spotlight-search-icon {
    margin-right: 10px;
    color: rgba(255, 255, 255, 0.6);
}

.spotlight-results-list {
    background: transparent;
    border: none;
}

.spotlight-row {
    background: transparent;
    border-radius: 10px;
    padding: 6px 10px;
    margin: 2px 0px;
    transition: background-color 100ms ease;
}

.spotlight-row:selected, .spotlight-row:hover {
    background-color: #2563eb;
    color: #ffffff;
}

.result-title {
    font-size: 15px;
    font-weight: 600;
    color: #f4f4f5;
}

.result-subtitle {
    font-size: 12px;
    color: #a1a1aa;
}

.spotlight-row:selected .result-subtitle {
    color: rgba(255, 255, 255, 0.85);
}

.calc-result-title {
    font-size: 20px;
    font-weight: 700;
    color: #60a5fa;
}

.spotlight-row:selected .calc-result-title {
    color: #ffffff;
}
"#;

fn main() {
    init_logging("malus_de_spotlight");
    info!("Iniciando Malus DE Spotlight Search (GTK4 / Layer Shell)...");

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let app = Application::builder()
        .application_id("org.malus_de.Spotlight")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_spotlight_ui);

    app.run();
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(SPOTLIGHT_CSS);

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn build_spotlight_ui(app: &Application) {
    let all_apps = Rc::new(indexer::index_applications());
    info!("Spotlight indexó exitosamente {} aplicaciones del sistema.", all_apps.len());

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus Spotlight")
        .css_classes(["malus-spotlight-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        info!("Layer Shell soportado. Configurando Spotlight como Overlay con foco exclusivo...");
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-spotlight");
        window.set_anchor(Edge::Top, true);
        window.set_margin(Edge::Top, 140);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    } else {
        info!("Layer Shell no detectado; ejecutando Spotlight en ventana flotante.");
        window.set_decorated(false);
    }

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["spotlight-card"])
        .build();

    // Barra de búsqueda con icono
    let search_bar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["spotlight-search-box"])
        .build();

    let search_icon = Image::builder()
        .icon_name("system-search-symbolic")
        .pixel_size(20)
        .css_classes(["spotlight-search-icon"])
        .build();
    search_bar.append(&search_icon);

    let search_entry = Entry::builder()
        .css_classes(["spotlight-entry"])
        .placeholder_text("Buscar apps, archivos, cálculos matemáticos...")
        .hexpand(true)
        .build();
    search_bar.append(&search_entry);
    card.append(&search_bar);

    // Lista de resultados
    let list_box = ListBox::builder()
        .css_classes(["spotlight-results-list"])
        .selection_mode(gtk4::SelectionMode::Single)
        .build();

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .max_content_height(340)
        .propagate_natural_height(true)
        .child(&list_box)
        .build();
    card.append(&scroll);

    // Almacenamiento de acciones ejecutables asociadas a cada fila de la lista
    let row_actions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    // Filtrar resultados al escribir
    let apps_ref = all_apps.clone();
    let list_clone = list_box.clone();
    let actions_clone = row_actions.clone();

    let update_results = move |query: &str| {
        // Limpiar filas anteriores
        while let Some(child) = list_clone.first_child() {
            list_clone.remove(&child);
        }
        actions_clone.borrow_mut().clear();

        let q = query.trim().to_lowercase();

        // 1. Comprobar si es un cálculo matemático (ej. 25 * 4)
        if let Some(calc_res) = calculator::try_calculate(query) {
            let row = ListBoxRow::builder().css_classes(["spotlight-row"]).build();
            let row_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .build();
            let icon = Image::builder().icon_name("accessories-calculator-symbolic").pixel_size(32).build();
            let text_box = GtkBox::builder().orientation(Orientation::Vertical).build();
            let title = Label::builder().label(&format!("= {}", calc_res)).css_classes(["calc-result-title"]).xalign(0.0).build();
            let subtitle = Label::builder().label("Resultado de cálculo").css_classes(["result-subtitle"]).xalign(0.0).build();
            text_box.append(&title);
            text_box.append(&subtitle);
            row_box.append(&icon);
            row_box.append(&text_box);
            row.set_child(Some(&row_box));
            list_clone.append(&row);
            actions_clone.borrow_mut().push(format!("echo {}", calc_res));
        }

        // 2. Filtrar aplicaciones
        if !q.is_empty() {
            let mut matches = 0;
            for app in apps_ref.iter() {
                if matches >= 8 {
                    break;
                }
                if app.name.to_lowercase().contains(&q) || app.comment.to_lowercase().contains(&q) {
                    let row = ListBoxRow::builder().css_classes(["spotlight-row"]).build();
                    let row_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(12)
                        .build();

                    let icon = Image::builder().icon_name(&app.icon).pixel_size(32).build();
                    let text_box = GtkBox::builder().orientation(Orientation::Vertical).build();
                    let title = Label::builder().label(&app.name).css_classes(["result-title"]).xalign(0.0).build();
                    let subtitle = Label::builder().label(&app.comment).css_classes(["result-subtitle"]).xalign(0.0).build();
                    text_box.append(&title);
                    text_box.append(&subtitle);

                    row_box.append(&icon);
                    row_box.append(&text_box);
                    row.set_child(Some(&row_box));

                    list_clone.append(&row);
                    actions_clone.borrow_mut().push(app.exec.clone());
                    matches += 1;
                }
            }
        }

        // Seleccionar automáticamente la primera fila
        if let Some(first_row) = list_clone.row_at_index(0) {
            list_clone.select_row(Some(&first_row));
        }
    };

    let update_clone = Rc::new(update_results);
    let u_entry = update_clone.clone();
    search_entry.connect_changed(move |entry| {
        u_entry(entry.text().as_str());
    });

    // Control de teclas (Escape para cerrar, Enter para lanzar, flechas para navegar)
    let key_controller = EventControllerKey::new();
    let win_close = window.clone();
    let list_nav = list_box.clone();
    let actions_exec = row_actions.clone();

    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        match keyval {
            Key::Escape => {
                info!("Cerrando Spotlight...");
                if let Some(app) = win_close.application() {
                    app.quit();
                } else {
                    win_close.close();
                }
                glib::Propagation::Stop
            }
            Key::Return | Key::KP_Enter => {
                if let Some(selected_row) = list_nav.selected_row() {
                    let idx = selected_row.index() as usize;
                    let actions = actions_exec.borrow();
                    if let Some(cmd) = actions.get(idx) {
                        info!("Lanzando desde Spotlight: {}", cmd);
                        let parts: Vec<&str> = cmd.split_whitespace().collect();
                        if let Some((exe, args)) = parts.split_first() {
                            let (final_exe, final_args) = if *exe == "nautilus" {
                                ("nautilus", vec!["--new-window"])
                            } else if *exe == "ptyxis" && !args.contains(&"-s") {
                                ("ptyxis", vec!["-s"])
                            } else if *exe == "gnome-text-editor" && !args.contains(&"-s") {
                                ("gnome-text-editor", vec!["-s"])
                            } else {
                                (*exe, args.to_vec())
                            };

                            let mut command = Command::new(final_exe);
                            command.args(final_args);
                            malus_de_common::config::set_cursor_environment(&mut command);
                            if let Ok(wayland_disp) = std::env::var("WAYLAND_DISPLAY") {
                                command.env("WAYLAND_DISPLAY", wayland_disp);
                            }
                            command.env("XDG_CURRENT_DESKTOP", "Malus");
                            command.env("GDK_BACKEND", "wayland");
                            if let Ok(xdg_rt) = std::env::var("XDG_RUNTIME_DIR") {
                                command.env("XDG_RUNTIME_DIR", xdg_rt);
                            }
                            if let Ok(mut child) = command.spawn() {
                                std::thread::spawn(move || {
                                    let _ = child.wait();
                                });
                            }
                        }
                    }
                }
                if let Some(app) = win_close.application() {
                    app.quit();
                } else {
                    win_close.close();
                }
                glib::Propagation::Stop
            }
            Key::Down => {
                if let Some(sel) = list_nav.selected_row() {
                    let next_idx = sel.index() + 1;
                    if let Some(next_row) = list_nav.row_at_index(next_idx) {
                        list_nav.select_row(Some(&next_row));
                    }
                }
                glib::Propagation::Stop
            }
            Key::Up => {
                if let Some(sel) = list_nav.selected_row() {
                    let prev_idx = (sel.index() - 1).max(0);
                    if let Some(prev_row) = list_nav.row_at_index(prev_idx) {
                        list_nav.select_row(Some(&prev_row));
                    }
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });

    window.add_controller(key_controller);

    window.set_child(Some(&card));
    window.present();
    search_entry.grab_focus();
}
