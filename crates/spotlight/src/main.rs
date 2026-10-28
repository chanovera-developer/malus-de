use gtk4::gdk::Key;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Entry, EventControllerKey,
    Grid, Image, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, Stack, ToggleButton,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::logging::init_logging;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;
use tracing::info;

mod calculator;
mod indexer;

#[derive(Clone, Copy)]
enum SystemAction {
    Logout,
    Reboot,
    PowerOff,
}

#[derive(Clone)]
enum SpotlightAction {
    LaunchApp(String),
    OpenUri(String),
    System(SystemAction),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowseMode {
    Search,
    Applications,
    RecentFiles,
}

const SPOTLIGHT_CSS: &str = r#"
window.malus-spotlight-window {
    background-color: transparent;
}

.spotlight-component-window,
.spotlight-component-window.background,
.spotlight-component-window decoration {
    background-color: transparent;
    background: transparent;
    border: none;
    box-shadow: none;
    overflow: visible;
}

.spotlight-card {
    background-color: transparent;
    border: none;
    border-radius: 26px;
    overflow: visible;
    padding: 0;
}

.spotlight-backdrop {
    background-color: alpha(@window_bg_color, 0.42);
    border: 1px solid alpha(@window_fg_color, 0.16);
}

.spotlight-card-expanded {
    background-color: transparent;
    border: none;
    padding: 0;
}

.spotlight-entry,
entry.spotlight-entry,
entry.spotlight-entry text {
    background-color: transparent;
    border: 0;
    color: @window_fg_color;
    caret-color: @accent_bg_color;
    font-size: 21px;
    font-weight: 500;
    box-shadow: none;
    padding: 0 12px;
    outline-style: none;
    outline-width: 0;
}

entry.spotlight-entry text selection {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
}

entry.spotlight-entry placeholder {
    color: alpha(@window_fg_color, 0.45);
}

.spotlight-entry:focus,
entry.spotlight-entry:focus {
    background-color: transparent;
    border: 0;
    box-shadow: none;
    outline-style: none;
    outline-width: 0;
}

.spotlight-input-surface,
.spotlight-mode-button {
    background-color: alpha(@window_bg_color, 0.42);
    border: 1px solid alpha(@window_fg_color, 0.12);
    border-radius: 999px;
    box-shadow: 0 8px 18px rgba(0, 0, 0, 0.28), 0 2px 6px rgba(0, 0, 0, 0.18);
    min-height: 54px;
    overflow: visible;
}

.spotlight-input-surface {
    padding: 0 12px;
}

.spotlight-input-surface.spotlight-component-active,
.spotlight-mode-button.spotlight-component-active {
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
}

@media (prefers-color-scheme: dark) {
    .spotlight-input-surface,
    .spotlight-mode-button {
        border-color: alpha(@window_fg_color, 0.14);
    }
}

.spotlight-search-icon {
    margin: 0 8px 0 2px;
}

.spotlight-search-icon,
.spotlight-icon {
    color: alpha(@window_fg_color, 0.66);
}

.spotlight-mode-button {
    padding: 0;
    min-width: 54px;
    margin: 0;
    transition: background-color 140ms ease, box-shadow 140ms ease, transform 140ms ease;
}

.spotlight-mode-button:hover {
    background-color: alpha(@window_bg_color, 0.58);
}

.spotlight-mode-button:checked {
    border-color: alpha(@accent_bg_color, 0.42);
}

.spotlight-mode-button.spotlight-component-active:checked {
    border-color: transparent;
}

.spotlight-browse-heading {
    color: alpha(@window_fg_color, 0.7);
    font-size: 13px;
    font-weight: 600;
    margin: 0;
}

.spotlight-app-card {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 16px;
    padding: 12px 6px 10px;
    transition: background-color 120ms ease, border-color 120ms ease;
}

.spotlight-app-card:hover, .spotlight-app-card:focus, .spotlight-app-card:active {
    background-color: alpha(@window_fg_color, 0.1);
    border-color: alpha(@window_fg_color, 0.12);
}

.spotlight-app-name {
    font-size: 12px;
    color: @window_fg_color;
}

.spotlight-results-list {
    background: transparent;
    border: none;
}

.spotlight-row {
    background: transparent;
    border-radius: 12px;
    padding: 9px 12px;
    margin: 2px 4px;
    transition: background-color 100ms ease;
}

.spotlight-row:selected, .spotlight-row:hover {
    background-color: alpha(@accent_bg_color, 0.2);
}

.result-title {
    font-size: 15px;
    font-weight: 600;
    color: @window_fg_color;
}

.result-subtitle {
    font-size: 12px;
    color: alpha(@window_fg_color, 0.68);
}

.spotlight-row:selected .result-subtitle {
    color: alpha(@window_fg_color, 0.78);
}

.calc-result-title {
    font-size: 20px;
    font-weight: 700;
    color: @accent_bg_color;
}

.spotlight-row:selected .calc-result-title {
    color: @accent_fg_color;
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

fn create_spotlight_layer_window(app: &Application, width: i32, height: i32) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .default_width(width)
        .default_height(height)
        .css_classes(["spotlight-component-window"])
        .build();
    window.set_size_request(width, height);
    window
}

fn configure_spotlight_layer_window(
    window: &ApplicationWindow,
    namespace: &str,
    left: i32,
    top: i32,
    keyboard_mode: KeyboardMode,
    monitor: Option<&gtk4::gdk::Monitor>,
) {
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(namespace);
        if let Some(monitor) = monitor {
            window.set_monitor(monitor);
        }
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_margin(Edge::Top, top);
        window.set_margin(Edge::Left, left);
        window.set_keyboard_mode(keyboard_mode);
    }
}

fn build_spotlight_ui(app: &Application) {
    let all_apps = Rc::new(indexer::index_applications());
    info!(
        "Spotlight indexó exitosamente {} aplicaciones del sistema.",
        all_apps.len()
    );

    const SEARCH_WIDTH: i32 = 430;
    const BUTTON_SIZE: i32 = 54;
    const CONTROL_GAP: i32 = 10;
    const SHADOW_GUTTER: i32 = 28;
    const SEARCH_TOP: i32 = 140;
    const RESULTS_WIDTH: i32 = SEARCH_WIDTH + BUTTON_SIZE * 2 + CONTROL_GAP * 2;
    const RESULTS_HEIGHT: i32 = 460;
    const RESULTS_PANEL_TOP_GAP: i32 = 70;

    let monitor = gtk4::gdk::Display::default().and_then(|display| {
        display
            .monitors()
            .item(0)
            .and_then(|monitor| monitor.downcast::<gtk4::gdk::Monitor>().ok())
    });
    let monitor_width = monitor
        .as_ref()
        .map(|monitor| monitor.geometry().width())
        .unwrap_or(1920);
    let controls_width = RESULTS_WIDTH;
    let controls_left = ((monitor_width - controls_width) / 2).max(0);
    let results_left = ((monitor_width - RESULTS_WIDTH) / 2).max(0);

    let backdrop_window =
        create_spotlight_layer_window(app, RESULTS_WIDTH, RESULTS_HEIGHT + RESULTS_PANEL_TOP_GAP);
    let backdrop = GtkBox::builder()
        .hexpand(true)
        .vexpand(true)
        .css_classes(["spotlight-backdrop"])
        .build();
    backdrop_window.set_child(Some(&backdrop));
    configure_spotlight_layer_window(
        &backdrop_window,
        "malus-spotlight-backdrop",
        results_left,
        SEARCH_TOP,
        KeyboardMode::None,
        monitor.as_ref(),
    );
    backdrop_window.set_layer(Layer::Bottom);
    backdrop_window.set_visible(false);

    let window =
        create_spotlight_layer_window(app, RESULTS_WIDTH, RESULTS_HEIGHT + RESULTS_PANEL_TOP_GAP);
    window.set_title(Some("Malus Spotlight"));

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["spotlight-card"])
        .build();
    let controls_spacer = GtkBox::builder()
        .height_request(RESULTS_PANEL_TOP_GAP)
        .build();
    card.append(&controls_spacer);
    card.set_size_request(RESULTS_WIDTH, RESULTS_HEIGHT);

    let input_window = create_spotlight_layer_window(
        app,
        SEARCH_WIDTH + SHADOW_GUTTER * 2,
        BUTTON_SIZE + SHADOW_GUTTER * 2,
    );
    let input_surface = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .valign(gtk4::Align::Center)
        .margin_start(SHADOW_GUTTER)
        .margin_end(SHADOW_GUTTER)
        .margin_top(SHADOW_GUTTER)
        .margin_bottom(SHADOW_GUTTER)
        .css_classes(["spotlight-input-surface"])
        .build();
    let search_icon = Image::builder()
        .icon_name("system-search-symbolic")
        .pixel_size(20)
        .css_classes(["spotlight-search-icon"])
        .build();
    input_surface.append(&search_icon);

    let search_entry = Entry::builder()
        .css_classes(["spotlight-entry"])
        .placeholder_text("Buscar aplicaciones, archivos y más")
        .has_frame(false)
        .hexpand(true)
        .build();
    input_surface.append(&search_entry);
    input_window.set_child(Some(&input_surface));
    configure_spotlight_layer_window(
        &input_window,
        "malus-spotlight-input",
        controls_left - SHADOW_GUTTER,
        SEARCH_TOP - SHADOW_GUTTER,
        KeyboardMode::Exclusive,
        monitor.as_ref(),
    );

    let selected_mode = Rc::new(Cell::new(BrowseMode::Search));
    let mut mode_buttons = Vec::new();
    let mut mode_windows = Vec::new();
    for (index, (tooltip, icon_name, mode)) in [
        (
            "Aplicaciones",
            "view-app-grid-symbolic",
            BrowseMode::Applications,
        ),
        (
            "Archivos recientes",
            "folder-symbolic",
            BrowseMode::RecentFiles,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let button_window = create_spotlight_layer_window(
            app,
            BUTTON_SIZE + SHADOW_GUTTER * 2,
            BUTTON_SIZE + SHADOW_GUTTER * 2,
        );
        let button = ToggleButton::builder()
            .tooltip_text(tooltip)
            .css_classes(["spotlight-mode-button"])
            .margin_start(SHADOW_GUTTER)
            .margin_end(SHADOW_GUTTER)
            .margin_top(SHADOW_GUTTER)
            .margin_bottom(SHADOW_GUTTER)
            .build();
        button.set_size_request(BUTTON_SIZE, BUTTON_SIZE);
        let icon = Image::builder()
            .icon_name(icon_name)
            .pixel_size(24)
            .css_classes(["spotlight-icon"])
            .build();
        button.set_child(Some(&icon));
        button_window.set_child(Some(&button));
        configure_spotlight_layer_window(
            &button_window,
            if index == 0 {
                "malus-spotlight-apps"
            } else {
                "malus-spotlight-files"
            },
            controls_left
                + SEARCH_WIDTH
                + CONTROL_GAP
                + index as i32 * (BUTTON_SIZE + CONTROL_GAP)
                - SHADOW_GUTTER,
            SEARCH_TOP - SHADOW_GUTTER,
            KeyboardMode::None,
            monitor.as_ref(),
        );
        mode_windows.push(button_window);
        mode_buttons.push((button, icon, mode, icon_name, tooltip));
    }

    let browse_heading = Label::builder()
        .label("Aplicaciones")
        .xalign(0.0)
        .css_classes(["spotlight-browse-heading"])
        .build();
    card.append(&browse_heading);

    let browse_view = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    let app_grid = Grid::builder()
        .column_spacing(8)
        .row_spacing(4)
        .column_homogeneous(true)
        .build();
    populate_app_grid(&app_grid, &all_apps, "", app);
    let app_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .max_content_height(300)
        .propagate_natural_height(true)
        .child(&app_grid)
        .build();
    browse_view.append(&app_scroll);

    // Lista de resultados
    let list_box = ListBox::builder()
        .css_classes(["spotlight-results-list"])
        .selection_mode(gtk4::SelectionMode::Single)
        .build();
    let empty_state = Label::builder()
        .label("No hay archivos recientes para mostrar.")
        .css_classes(["result-subtitle"])
        .margin_top(20)
        .margin_bottom(20)
        .build();
    list_box.set_placeholder(Some(&empty_state));

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .max_content_height(340)
        .propagate_natural_height(true)
        .child(&list_box)
        .build();
    let content_stack = Stack::builder()
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .transition_duration(120)
        .build();
    content_stack.add_named(&browse_view, Some("applications"));
    content_stack.add_named(&scroll, Some("results"));
    card.append(&content_stack);

    // Almacenamiento de acciones ejecutables asociadas a cada fila de la lista
    let row_actions: Rc<RefCell<Vec<SpotlightAction>>> = Rc::new(RefCell::new(Vec::new()));
    let actions_for_file_open = row_actions.clone();
    let app_for_file_open = app.clone();
    list_box.connect_row_activated(move |_, row| {
        let action = actions_for_file_open
            .borrow()
            .get(row.index() as usize)
            .cloned();
        if let Some(SpotlightAction::OpenUri(uri)) = action {
            if let Err(error) = gtk4::gio::AppInfo::launch_default_for_uri(
                &uri,
                None::<&gtk4::gio::AppLaunchContext>,
            ) {
                tracing::error!("No se pudo abrir el archivo reciente {uri}: {error}");
            }
            app_for_file_open.quit();
        }
    });

    // Filtrar resultados al escribir
    let apps_ref = all_apps.clone();
    let list_clone = list_box.clone();
    let actions_clone = row_actions.clone();
    let card_clone = card.clone();
    let content_stack_clone = content_stack.clone();
    let heading_clone = browse_heading.clone();
    let empty_state_clone = empty_state.clone();
    let app_grid_clone = app_grid.clone();
    let app_clone = app.clone();
    let selected_mode_for_results = selected_mode.clone();
    let input_surface_for_update = input_surface.clone();
    let backdrop_window_for_update = backdrop_window.clone();
    let mode_buttons_for_update: Vec<ToggleButton> = mode_buttons
        .iter()
        .map(|(button, _, _, _, _)| button.clone())
        .collect();
    let mode_windows_for_update = mode_windows.clone();
    let recent_manager = gtk4::RecentManager::default();
    let results_window_for_update = window.clone();

    let update_results = move |query: &str| {
        // Limpiar filas anteriores
        while let Some(child) = list_clone.first_child() {
            list_clone.remove(&child);
        }
        actions_clone.borrow_mut().clear();

        let q = query.trim().to_lowercase();
        let mode = selected_mode_for_results.get();
        let is_active = mode != BrowseMode::Search || !q.is_empty();
        results_window_for_update.set_visible(is_active);
        backdrop_window_for_update.set_visible(is_active);
        input_surface_for_update.set_css_classes(if is_active {
            &["spotlight-input-surface", "spotlight-component-active"]
        } else {
            &["spotlight-input-surface"]
        });
        for (index, (button, button_window)) in mode_buttons_for_update
            .iter()
            .zip(&mode_windows_for_update)
            .enumerate()
        {
            let visible = mode == BrowseMode::Search
                || (mode == BrowseMode::Applications && index == 0)
                || (mode == BrowseMode::RecentFiles && index == 1);
            button.set_visible(visible);
            button_window.set_visible(visible);
            if is_active {
                button.add_css_class("spotlight-component-active");
            } else {
                button.remove_css_class("spotlight-component-active");
            }
        }
        if !is_active {
            card_clone.remove_css_class("spotlight-card-expanded");
        } else {
            card_clone.add_css_class("spotlight-card-expanded");
        }
        empty_state_clone.set_label(if mode == BrowseMode::RecentFiles {
            "No hay archivos recientes que coincidan."
        } else {
            "No se encontraron resultados."
        });
        heading_clone.set_visible(mode != BrowseMode::Search);
        content_stack_clone.set_visible(mode != BrowseMode::Search || !q.is_empty());
        if mode == BrowseMode::Applications {
            content_stack_clone.set_visible_child_name("applications");
            heading_clone.set_label("Aplicaciones");
            populate_app_grid(&app_grid_clone, &apps_ref, &q, &app_clone);
            return;
        }
        content_stack_clone.set_visible_child_name("results");
        if mode == BrowseMode::RecentFiles {
            heading_clone.set_label("Archivos recientes");
            populate_recent_files(&list_clone, &actions_clone, &recent_manager, &q);
            if let Some(first_row) = list_clone.row_at_index(0) {
                list_clone.select_row(Some(&first_row));
            }
            return;
        }

        // 1. Comprobar si es un cálculo matemático (ej. 25 * 4)
        if let Some(calc_res) = calculator::try_calculate(query) {
            let row = ListBoxRow::builder().css_classes(["spotlight-row"]).build();
            let row_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .build();
            let icon = Image::builder()
                .icon_name("accessories-calculator-symbolic")
                .pixel_size(32)
                .css_classes(["spotlight-icon"])
                .build();
            let text_box = GtkBox::builder().orientation(Orientation::Vertical).build();
            let title = Label::builder()
                .label(&format!("= {}", calc_res))
                .css_classes(["calc-result-title"])
                .xalign(0.0)
                .build();
            let subtitle = Label::builder()
                .label("Resultado de cálculo")
                .css_classes(["result-subtitle"])
                .xalign(0.0)
                .build();
            text_box.append(&title);
            text_box.append(&subtitle);
            row_box.append(&icon);
            row_box.append(&text_box);
            row.set_child(Some(&row_box));
            list_clone.append(&row);
            actions_clone
                .borrow_mut()
                .push(SpotlightAction::LaunchApp(format!("echo {}", calc_res)));
        }

        // 2. Acciones de Malus DE
        if !q.is_empty() {
            let system_actions = [
                (
                    SystemAction::Logout,
                    "Cerrar sesión",
                    "Cierra la sesión actual de Malus DE",
                    "system-log-out-symbolic",
                    &[
                        "cerrar sesión",
                        "cerrar sesion",
                        "sesión",
                        "sesion",
                        "logout",
                        "salir",
                    ][..],
                ),
                (
                    SystemAction::Reboot,
                    "Reiniciar",
                    "Reinicia el equipo",
                    "system-reboot-symbolic",
                    &["reiniciar", "reinicio", "restart", "reboot"][..],
                ),
                (
                    SystemAction::PowerOff,
                    "Apagar",
                    "Apaga el equipo",
                    "system-shutdown-symbolic",
                    &["apagar", "apagado", "shutdown", "power off", "poweroff"][..],
                ),
            ];

            for (action, name, description, icon_name, keywords) in system_actions {
                if keywords.iter().any(|keyword| keyword.contains(&q)) {
                    let row = ListBoxRow::builder().css_classes(["spotlight-row"]).build();
                    let row_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(12)
                        .build();
                    let icon = Image::builder()
                        .icon_name(icon_name)
                        .pixel_size(32)
                        .css_classes(["spotlight-icon"])
                        .build();
                    let text_box = GtkBox::builder().orientation(Orientation::Vertical).build();
                    let title = Label::builder()
                        .label(name)
                        .css_classes(["result-title"])
                        .xalign(0.0)
                        .build();
                    let subtitle = Label::builder()
                        .label(description)
                        .css_classes(["result-subtitle"])
                        .xalign(0.0)
                        .build();
                    text_box.append(&title);
                    text_box.append(&subtitle);
                    row_box.append(&icon);
                    row_box.append(&text_box);
                    row.set_child(Some(&row_box));
                    list_clone.append(&row);
                    actions_clone
                        .borrow_mut()
                        .push(SpotlightAction::System(action));
                }
            }
        }

        // 3. Filtrar aplicaciones
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

                    let icon = Image::builder()
                        .icon_name(&app.icon)
                        .pixel_size(32)
                        .css_classes(["spotlight-icon"])
                        .build();
                    let text_box = GtkBox::builder().orientation(Orientation::Vertical).build();
                    let title = Label::builder()
                        .label(&app.name)
                        .css_classes(["result-title"])
                        .xalign(0.0)
                        .build();
                    let subtitle = Label::builder()
                        .label(&app.comment)
                        .css_classes(["result-subtitle"])
                        .xalign(0.0)
                        .build();
                    text_box.append(&title);
                    text_box.append(&subtitle);

                    row_box.append(&icon);
                    row_box.append(&text_box);
                    row.set_child(Some(&row_box));

                    list_clone.append(&row);
                    actions_clone
                        .borrow_mut()
                        .push(SpotlightAction::LaunchApp(app.exec.clone()));
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
    let mode_button_group: Vec<ToggleButton> = mode_buttons
        .iter()
        .map(|(button, _, _, _, _)| button.clone())
        .collect();
    for (index, (button, icon, mode, icon_name, tooltip)) in mode_buttons.into_iter().enumerate() {
        let selected_mode = selected_mode.clone();
        let update = update_clone.clone();
        let entry = search_entry.clone();
        let buttons = mode_button_group.clone();
        let button_for_tooltip = button.clone();
        button.connect_clicked(move |_| {
            let next_mode = if selected_mode.get() == mode {
                BrowseMode::Search
            } else {
                mode
            };
            selected_mode.set(next_mode);
            let is_active_mode = next_mode == mode;
            icon.set_icon_name(Some(if is_active_mode {
                "go-previous-symbolic"
            } else {
                icon_name
            }));
            button_for_tooltip.set_tooltip_text(Some(if is_active_mode {
                "Volver"
            } else {
                tooltip
            }));

            for (button_index, other) in buttons.iter().enumerate() {
                let is_selected = next_mode == mode && button_index == index;
                other.set_active(is_selected);
                other.set_visible(next_mode == BrowseMode::Search || button_index == index);
            }
            update(entry.text().as_str());
            entry.grab_focus();
        });
    }
    update_clone("");

    // Control de teclas (Escape para cerrar, Enter para lanzar, flechas para navegar)
    let key_controller = EventControllerKey::new();
    let win_close = window.clone();
    let list_nav = list_box.clone();
    let actions_exec = row_actions.clone();

    key_controller.connect_key_pressed(move |_, keyval, _, _| match keyval {
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
            let action = if let Some(selected_row) = list_nav.selected_row() {
                let idx = selected_row.index() as usize;
                actions_exec.borrow().get(idx).cloned()
            } else {
                None
            };

            match action {
                Some(SpotlightAction::LaunchApp(cmd)) => {
                    info!("Lanzando desde Spotlight: {}", cmd);
                    launch_application(&cmd);
                    if let Some(app) = win_close.application() {
                        app.quit();
                    } else {
                        win_close.close();
                    }
                }
                Some(SpotlightAction::OpenUri(uri)) => {
                    if let Err(error) = gtk4::gio::AppInfo::launch_default_for_uri(
                        &uri,
                        None::<&gtk4::gio::AppLaunchContext>,
                    ) {
                        tracing::error!("No se pudo abrir el archivo reciente {uri}: {error}");
                    }
                    if let Some(app) = win_close.application() {
                        app.quit();
                    } else {
                        win_close.close();
                    }
                }
                Some(SpotlightAction::System(action)) => {
                    show_system_action_confirmation(&win_close, action);
                }
                None => {
                    if let Some(app) = win_close.application() {
                        app.quit();
                    } else {
                        win_close.close();
                    }
                }
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
    });

    input_window.add_controller(key_controller);

    window.set_child(Some(&card));
    configure_spotlight_layer_window(
        &window,
        "malus-spotlight-results",
        results_left,
        SEARCH_TOP,
        KeyboardMode::None,
        monitor.as_ref(),
    );
    input_window.present();
    for button_window in &mode_windows {
        button_window.present();
    }
    search_entry.grab_focus();
}

fn populate_app_grid(
    grid: &Grid,
    apps: &[indexer::AppEntry],
    query: &str,
    application: &Application,
) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }

    let matches = apps.iter().filter(|app_entry| {
        query.is_empty()
            || app_entry.name.to_lowercase().contains(query)
            || app_entry.comment.to_lowercase().contains(query)
    });
    for (index, app_entry) in matches.enumerate() {
        let app_button = Button::builder()
            .tooltip_text(&app_entry.name)
            .css_classes(["spotlight-app-card"])
            .build();
        let content = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .build();
        let icon = Image::builder()
            .icon_name(&app_entry.icon)
            .pixel_size(48)
            .css_classes(["spotlight-icon"])
            .build();
        let name = Label::builder()
            .label(&app_entry.name)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(14)
            .css_classes(["spotlight-app-name"])
            .build();
        content.append(&icon);
        content.append(&name);
        app_button.set_child(Some(&content));

        let command = app_entry.exec.clone();
        let application = application.clone();
        app_button.connect_clicked(move |_| {
            launch_application(&command);
            application.quit();
        });
        grid.attach(&app_button, (index % 5) as i32, (index / 5) as i32, 1, 1);
    }
}

fn populate_recent_files(
    list: &ListBox,
    actions: &RefCell<Vec<SpotlightAction>>,
    manager: &gtk4::RecentManager,
    query: &str,
) {
    let mut recent_files: Vec<_> = manager
        .items()
        .into_iter()
        .filter(|item| item.exists())
        .collect();
    recent_files.sort_by_key(|item| std::cmp::Reverse(item.modified().to_unix()));

    for recent in recent_files {
        let name = recent.display_name();
        let uri = recent.uri();
        let mime_type = recent.mime_type();
        if !query.is_empty()
            && !name.to_lowercase().contains(query)
            && !uri.to_lowercase().contains(query)
        {
            continue;
        }

        let icon_name = if mime_type.starts_with("image/") {
            "image-x-generic-symbolic"
        } else if mime_type == "application/pdf" {
            "application-pdf-symbolic"
        } else if mime_type.starts_with("text/") {
            "text-x-generic-symbolic"
        } else {
            "unknown-symbolic"
        };
        let row = ListBoxRow::builder().css_classes(["spotlight-row"]).build();
        let row_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        let icon = Image::builder()
            .icon_name(icon_name)
            .pixel_size(32)
            .css_classes(["spotlight-icon"])
            .build();
        let text_box = GtkBox::builder().orientation(Orientation::Vertical).build();
        let title = Label::builder()
            .label(name.to_string())
            .css_classes(["result-title"])
            .xalign(0.0)
            .build();
        let location = recent
            .uri_display()
            .map(|display| display.to_string())
            .unwrap_or_else(|| uri.to_string());
        let subtitle = Label::builder()
            .label(&location)
            .css_classes(["result-subtitle"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .build();
        text_box.append(&title);
        text_box.append(&subtitle);
        row_box.append(&icon);
        row_box.append(&text_box);
        row.set_child(Some(&row_box));
        list.append(&row);
        actions
            .borrow_mut()
            .push(SpotlightAction::OpenUri(uri.to_string()));
    }
}

fn launch_application(command_line: &str) {
    let parts: Vec<&str> = command_line.split_whitespace().collect();
    let Some((exe, args)) = parts.split_first() else {
        tracing::error!("No se pudo lanzar una aplicación: el comando está vacío.");
        return;
    };

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
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => tracing::error!("No se pudo lanzar la aplicación {final_exe}: {error}"),
    }
}

fn show_system_action_confirmation(window: &ApplicationWindow, action: SystemAction) {
    let (heading, body, confirm_label) = match action {
        SystemAction::Logout => (
            "¿Cerrar la sesión actual?",
            "Se cerrarán las aplicaciones abiertas en Malus DE y volverás a la pantalla de inicio de sesión.",
            "Cerrar sesión",
        ),
        SystemAction::Reboot => (
            "¿Reiniciar el equipo?",
            "El sistema cerrará las aplicaciones y reiniciará el equipo. Cualquier cambio no guardado se perderá.",
            "Reiniciar",
        ),
        SystemAction::PowerOff => (
            "¿Apagar el equipo?",
            "El sistema cerrará las aplicaciones y apagará el equipo.",
            "Apagar",
        ),
    };

    let Some(app) = window.application() else {
        tracing::error!(
            "No se pudo mostrar la confirmación: Spotlight no tiene una aplicación asociada."
        );
        return;
    };

    let app_on_confirm = app.clone();
    let app_on_cancel = app.clone();
    let parent = window.upcast_ref();
    malus_de_dialogs::show_confirmation(
        &app,
        Some(parent),
        heading,
        body,
        confirm_label,
        move || {
            execute_system_action(action);
            app_on_confirm.quit();
        },
        move || app_on_cancel.quit(),
    );
}

fn execute_system_action(action: SystemAction) {
    let mut command = match action {
        SystemAction::Logout => {
            let sibling_ctl = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|dir| dir.join("malus-de-ctl")))
                .filter(|path| path.is_file());
            let mut command = sibling_ctl
                .map(Command::new)
                .unwrap_or_else(|| Command::new("malus-de-ctl"));
            command.arg("logout");
            command
        }
        SystemAction::Reboot => {
            let mut command = Command::new("systemctl");
            command.arg("reboot");
            command
        }
        SystemAction::PowerOff => {
            let mut command = Command::new("systemctl");
            command.arg("poweroff");
            command
        }
    };

    if let Err(error) = command.spawn() {
        tracing::error!("No se pudo ejecutar la acción del sistema: {error}");
    }
}
