use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider,
    EventControllerMotion, GestureClick, Image, Label, Orientation, Overlay, Popover,
    Separator, Window,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::config::{DockAppConfig, DockConfig};
use malus_de_common::ipc::{ipc_socket_path, send_ipc_message, IpcMessage, MinimizedWindowInfo};
use malus_de_common::logging::init_logging;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::rc::Rc;
use tracing::{error, info, warn};

const DOCK_CSS: &str = r#"
window.malus-dock-window,
window.malus-dock-window.background,
window.malus-dock-window decoration {
    background-color: transparent;
    background: transparent;
    border: none;
    outline: none;
    box-shadow: none;
}

.dock-container {
    background-color: rgba(24, 24, 27, 0.72);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 32px;
    padding: 6px 10px;
    box-shadow: none;
    overflow: visible;
}

.dock-item-box {
    padding: 0px 4px;
}

.dock-item-btn {
    background: transparent;
    border: none;
    border-radius: 14px;
    padding: 4px;
    transition: background-color 180ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
    overflow: visible;
}

.dock-item-btn:hover {
    background-color: rgba(255, 255, 255, 0.14);
}

.dock-item-btn:active {
    background-color: rgba(255, 255, 255, 0.22);
}

.dock-indicator {
    font-size: 8px;
    color: rgba(255, 255, 255, 0.85);
    margin-top: -2px;
    margin-bottom: -2px;
}

.dock-separator {
    background-color: rgba(255, 255, 255, 0.18);
    min-width: 1px;
    min-height: 44px;
    margin: 4px 6px 10px 6px;
}

/* Tarjeta miniatura de ventana minimizada (macOS Dock Window Thumbnail) */
.dock-window-card {
    padding: 1px 3px;
    margin: 0px 2px 8px 2px;
}

.dock-mini-window {
    background: linear-gradient(180deg, rgba(45, 45, 55, 0.90) 0%, rgba(25, 25, 32, 0.95) 100%);
    border: 1px solid rgba(255, 255, 255, 0.22);
    border-radius: 8px;
    min-width: 54px;
    min-height: 42px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.50);
    transition: all 180ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
    padding: 0;
}

.dock-mini-window:hover {
    background: linear-gradient(180deg, rgba(58, 58, 72, 0.95) 0%, rgba(32, 32, 42, 0.98) 100%);
    border-color: rgba(255, 255, 255, 0.45);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.60), 0 0 8px rgba(59, 130, 246, 0.4);
}

.dock-mini-window-titlebar {
    background: rgba(255, 255, 255, 0.08);
    border-bottom: 1px solid rgba(255, 255, 255, 0.10);
    padding: 2px 4px;
    border-top-left-radius: 7px;
    border-top-right-radius: 7px;
}

.dock-mini-dot {
    font-size: 6px;
    line-height: 6px;
    margin-right: 2px;
}

.dot-red { color: #ff5f56; }
.dot-yellow { color: #ffbd2e; }
.dot-green { color: #27c93f; }

.dock-mini-window-body {
    padding: 4px 6px;
}

.dock-mini-content-line {
    background-color: rgba(255, 255, 255, 0.18);
    border-radius: 2px;
    min-height: 3px;
    margin-bottom: 3px;
}

.dock-mini-content-line-short {
    background-color: rgba(255, 255, 255, 0.10);
    border-radius: 2px;
    min-height: 3px;
    margin-right: 14px;
}

.dock-mini-badge {
    background: rgba(24, 24, 28, 0.95);
    border: 1.5px solid rgba(255, 255, 255, 0.35);
    border-radius: 9999px;
    padding: 2px;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.65);
    margin-right: -3px;
    margin-bottom: -3px;
}

.malus-popover-menu {
    background-color: alpha(@window_bg_color, 0.94);
    border: 1px solid alpha(@window_fg_color, 0.14);
    border-radius: 14px;
    box-shadow: 0 10px 25px rgba(0, 0, 0, 0.45);
    padding: 6px;
}

.popover-menu-header {
    color: alpha(@window_fg_color, 0.65);
    font-size: 11px;
    font-weight: 600;
    padding: 4px 10px 2px 10px;
    letter-spacing: 0.3px;
}

.popover-separator {
    background-color: alpha(@window_fg_color, 0.12);
    min-height: 1px;
    margin: 4px 4px;
}

.popover-menu-item {
    background: transparent;
    border: none;
    border-radius: 8px;
    padding: 6px 14px;
    color: @window_fg_color;
    font-size: 13px;
    font-weight: 500;
}

.popover-menu-item:hover {
    background-color: #2563eb;
    color: #ffffff;
}

.popover-menu-item-danger:hover {
    background-color: #dc2626;
    color: #ffffff;
}

/* Selector Visual de Aplicaciones estilo macOS (⌘Tab App Switcher HUD) */
window.malus-switcher-window {
    background-color: transparent;
}

.switcher-container {
    background-color: rgba(26, 26, 32, 0.90);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 22px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.70);
    padding: 12px 16px;
}

.switcher-card {
    background-color: transparent;
    border: 1.5px solid transparent;
    border-radius: 14px;
    padding: 10px 14px;
    opacity: 0.65;
    transition: all 120ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
}

.switcher-card.selected {
    background-color: rgba(255, 255, 255, 0.16);
    border-color: #2563eb;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45), 0 0 10px rgba(37, 99, 235, 0.35);
    opacity: 1.0;
}

.switcher-title {
    color: #f4f4f5;
    font-size: 12px;
    font-weight: 600;
    margin-top: 6px;
}

.switcher-card.selected .switcher-title {
    color: #ffffff;
    font-weight: 700;
}
"#;

fn main() {
    init_logging("malus_de_dock");
    info!("Iniciando Malus DE Floating Dock (GTK4 / Layer Shell)...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let app = Application::builder()
        .application_id("org.malus_de.Dock")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_dock_ui);

    app.run();
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(DOCK_CSS);

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn check_trash_full() -> bool {
    let trash = gio::File::for_uri("trash:///");
    if let Ok(enumerator) = trash.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        enumerator.into_iter().next().is_some()
    } else {
        false
    }
}

fn should_ignore_app(identifier: &str) -> bool {
    let lower = identifier.to_lowercase();
    let ignored = [
        "top-bar", "layer-shell", "desktop", "gdm", "compositor",
        "xwayland", "malus-session", "xsettings", "pipewire", "wireplumber",
        "gnome-shell", "unknown", "malus-de-top-bar", "malus-de-dock",
        "malus-de-wallpaper", "malus-de-link", "malus-de-spotlight",
        "malus-de-notifications"
    ];
    ignored.iter().any(|ign| lower.contains(ign))
}

fn resolve_desktop_app_info(identifier: &str) -> (String, String, String, Vec<String>) {
    let lower = identifier.to_lowercase();
    let candidates = [
        format!("{}.desktop", identifier),
        format!("{}.desktop", lower),
        format!("org.gnome.{}.desktop", identifier),
        format!("org.gnome.{}.desktop", lower),
        identifier.to_string(),
        lower.clone(),
    ];
    for cand in &candidates {
        if let Some(info) = gio::DesktopAppInfo::new(cand) {
            let name = info.name().to_string();
            let icon = info.icon()
                .and_then(|i| i.to_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| identifier.to_string());
            let exec = info.executable().to_string_lossy().to_string();
            let matchers = vec![identifier.to_string(), lower.clone(), name.to_lowercase()];
            return (name, icon, exec, matchers);
        }
    }

    let name = capitalize(identifier);
    let icon = identifier.to_string();
    let exec = identifier.to_string();
    let matchers = vec![identifier.to_string(), lower];
    (name, icon, exec, matchers)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn build_dock_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus DE Dock")
        .decorated(false)
        .css_classes(["malus-dock-window"])
        .build();

    window.set_decorated(false);

    // 1. Cargar configuración persistida desde dock.toml
    let dock_config = Rc::new(RefCell::new(DockConfig::load_or_create_default()));
    let initial_cfg = dock_config.borrow().clone();

    if gtk4_layer_shell::is_supported() {
        info!("Layer Shell soportado. Configurando Dock flotante centrado inferior...");
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace("malus-dock");
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, initial_cfg.floating_gap as i32);
        window.auto_exclusive_zone_enable();
    } else {
        info!("Layer Shell no detectado; ejecutando Dock en ventana flotante de desarrollo.");
    }

    let dock_root = GtkBox::new(Orientation::Vertical, 0);
    dock_root.set_overflow(gtk4::Overflow::Visible);
    let magnification_headroom = GtkBox::new(Orientation::Horizontal, 0);
    magnification_headroom.set_size_request(
        -1,
        initial_cfg
            .magnification_max_size
            .saturating_sub(initial_cfg.icon_size) as i32,
    );
    dock_root.append(&magnification_headroom);

    let dock_bar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["dock-container"])
        .spacing(4)
        .valign(gtk4::Align::End)
        .halign(gtk4::Align::Center)
        .build();
    dock_bar.set_overflow(gtk4::Overflow::Visible);

    // Contenedor modular de aplicaciones (fijadas y dinámicas en ejecución)
    let apps_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .valign(gtk4::Align::End)
        .build();
    apps_box.set_overflow(gtk4::Overflow::Visible);
    dock_bar.append(&apps_box);

    // Separador de cristal translúcido
    let separator = Separator::new(Orientation::Vertical);
    separator.add_css_class("dock-separator");
    separator.set_valign(gtk4::Align::End);
    dock_bar.append(&separator);

    // Contenedor dinámico de miniaturas de ventanas minimizadas (Bloque Derecho)
    let minimized_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .valign(gtk4::Align::End)
        .build();
    minimized_box.set_overflow(gtk4::Overflow::Visible);
    minimized_box.set_visible(false);
    dock_bar.append(&minimized_box);

    // Papelera de Reciclaje (Trash) con estado reactivo y menú contextual
    let (trash_widget, trash_img) = create_trash_item(initial_cfg.icon_size as i32);
    dock_bar.append(&trash_widget);
    dock_root.append(&dock_bar);

    // Ventana modal Overlay del Selector Visual de Aplicaciones (⌘Tab App Switcher HUD)
    let switcher_window = Window::builder()
        .title("Malus DE App Switcher")
        .css_classes(["malus-switcher-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        switcher_window.init_layer_shell();
        switcher_window.set_layer(Layer::Overlay);
        switcher_window.set_namespace("malus-app-switcher");
        switcher_window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
        switcher_window.set_exclusive_zone(0);
    } else {
        switcher_window.set_decorated(false);
    }

    let switcher_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["switcher-container"])
        .spacing(8)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();

    let win_dismiss_click = GestureClick::new();
    win_dismiss_click.connect_released(|_, _, _, _| {
        let _ = send_ipc_message(&IpcMessage::HideAppSwitcher);
    });
    switcher_box.set_visible(false);
    switcher_window.set_child(Some(&switcher_box));

    // Canales y estado compartido
    let running_cache = Rc::new(RefCell::new(Vec::<String>::new()));
    let dock_indicators: Rc<RefCell<Vec<(Vec<String>, Label)>>> = Rc::new(RefCell::new(Vec::new()));

    // Canal de actualización del layout de aplicaciones del Dock
    let (refresh_tx, refresh_rx) = async_channel::unbounded::<()>();
    let r_tx = refresh_tx.clone();
    let on_refresh: Rc<dyn Fn()> = Rc::new(move || {
        let _ = r_tx.send_blocking(());
    });

    // Render inicial de aplicaciones desde dock.toml
    render_dock_apps(
        &apps_box,
        &dock_config,
        &running_cache,
        &dock_indicators,
        &on_refresh,
    );

    // Loop de refresco reactivo de las aplicaciones
    let apps_box_rf = apps_box.clone();
    let dock_cfg_rf = dock_config.clone();
    let run_cache_rf = running_cache.clone();
    let indicators_rf = dock_indicators.clone();
    let on_rf_loop = on_refresh.clone();
    let dock_bar_resize = dock_bar.clone();
    let win_resize = window.clone();

    glib::spawn_future_local(async move {
        while let Ok(()) = refresh_rx.recv().await {
            render_dock_apps(
                &apps_box_rf,
                &dock_cfg_rf,
                &run_cache_rf,
                &indicators_rf,
                &on_rf_loop,
            );
            dock_bar_resize.queue_resize();
            win_resize.queue_resize();
        }
    });

    // Monitoreo reactivo de cambios en dock.toml en disco (GFileMonitor)
    let config_path = DockConfig::config_path();
    let gfile = gio::File::for_path(&config_path);
    if let Ok(monitor) = gfile.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        let cfg_clone = dock_config.clone();
        let on_rf_mon = on_refresh.clone();
        let win_mon = window.clone();
        let headroom_mon = magnification_headroom.clone();
        monitor.connect_changed(move |_, _, _, event_type| {
            if event_type == gio::FileMonitorEvent::ChangesDoneHint || event_type == gio::FileMonitorEvent::Created {
                info!("🔄 dock.toml modificado externamente en disco. Recargando...");
                let new_cfg = DockConfig::load_or_create_default();
                if gtk4_layer_shell::is_supported() {
                    win_mon.set_margin(Edge::Bottom, new_cfg.floating_gap as i32);
                }
                headroom_mon.set_size_request(
                    -1,
                    new_cfg
                        .magnification_max_size
                        .saturating_sub(new_cfg.icon_size) as i32,
                );
                *cfg_clone.borrow_mut() = new_cfg;
                on_rf_mon();
            }
        });
    }

    // Magnificación Analítica macOS (Efecto zoom suave con curva gaussiana)
    let apps_box_motion = apps_box.clone();
    let min_box_motion = minimized_box.clone();
    let trash_widget_motion = trash_widget.clone();
    let dock_bar_motion = dock_bar.clone();
    let dock_root_motion = dock_root.clone();
    let cfg_motion = dock_config.clone();

    let motion_controller = EventControllerMotion::new();
    motion_controller.connect_motion(move |_, x, y| {
        let cfg = cfg_motion.borrow();
        if !cfg.magnification_enabled {
            return;
        }
        let Some(pointer_in_dock) = dock_root_motion.compute_point(
            &dock_bar_motion,
            &gtk4::graphene::Point::new(x as f32, y as f32),
        ) else {
            return;
        };
        let x = pointer_in_dock.x() as f64;
        let base_size = cfg.icon_size as f64;
        let max_size = cfg.magnification_max_size as f64;
        let max_scale_diff = (max_size - base_size).max(0.0);
        let max_zoom_dist = 110.0;

        let update_item = |widget: &gtk4::Widget| {
            if let Some(p) = widget.compute_point(&dock_bar_motion, &gtk4::graphene::Point::zero()) {
                let cx = p.x() as f64;
                let cw = widget.width() as f64;
                let center_x = cx + cw / 2.0;
                let dist = (x - center_x).abs();
                let factor = if dist < max_zoom_dist {
                    let norm = dist / max_zoom_dist;
                    (-norm * norm * 2.2).exp()
                } else {
                    0.0
                };
                let target_size = (base_size + factor * max_scale_diff) as i32;
                update_icon_size_recursive(widget, target_size, base_size as i32);
            }
        };

        // Magnificar cada app en apps_box
        let mut child = apps_box_motion.first_child();
        while let Some(c) = child {
            update_item(&c);
            child = c.next_sibling();
        }

        // Magnificar cada miniatura en minimized_box si está visible
        if min_box_motion.is_visible() {
            let mut min_child = min_box_motion.first_child();
            while let Some(c) = min_child {
                update_item(&c);
                min_child = c.next_sibling();
            }
        }

        // Magnificar papelera
        update_item(trash_widget_motion.upcast_ref());
    });

    let apps_box_leave = apps_box.clone();
    let min_box_leave = minimized_box.clone();
    let trash_widget_leave = trash_widget.clone();
    let cfg_leave = dock_config.clone();

    motion_controller.connect_leave(move |_| {
        let base_size = cfg_leave.borrow().icon_size as i32;
        let mut child = apps_box_leave.first_child();
        while let Some(c) = child {
            update_icon_size_recursive(&c, base_size, base_size);
            child = c.next_sibling();
        }
        if min_box_leave.is_visible() {
            let mut min_child = min_box_leave.first_child();
            while let Some(c) = min_child {
                update_icon_size_recursive(&c, base_size, base_size);
                min_child = c.next_sibling();
            }
        }
        update_icon_size_recursive(trash_widget_leave.upcast_ref(), base_size, base_size);
    });
    dock_root.add_controller(motion_controller);

    // Escucha de eventos IPC del compositor
    let (tx, rx) = async_channel::unbounded::<IpcMessage>();
    let cache_ipc = running_cache.clone();
    let min_box = minimized_box.clone();
    let dock_bar_ref = dock_bar.clone();
    let min_map: Rc<RefCell<HashMap<u64, gtk4::Widget>>> = Rc::new(RefCell::new(HashMap::new()));
    let win_ref = window.clone();
    let indicators_ipc = dock_indicators.clone();
    let cfg_ipc = dock_config.clone();
    let on_rf_ipc = on_refresh.clone();
    let headroom_ipc = magnification_headroom.clone();
    let switcher_win_ipc = switcher_window.clone();
    let switcher_box_ipc = switcher_box.clone();

    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                IpcMessage::RunningAppsChanged { running_apps } => {
                    let prev_apps = cache_ipc.borrow().clone();
                    *cache_ipc.borrow_mut() = running_apps.clone();

                    // 1. Actualizar indicadores de las apps visibles
                    for (matchers, indicator) in indicators_ipc.borrow().iter() {
                        let is_running = running_apps.iter().any(|running| {
                            let r_lower = running.to_lowercase();
                            matchers.iter().any(|m| r_lower.contains(&m.to_lowercase()))
                        });
                        indicator.set_opacity(if is_running { 1.0 } else { 0.0 });
                    }

                    // 2. Si la lista de aplicaciones ejecutándose cambió, refrescar apps unpinned
                    if prev_apps != running_apps {
                        on_rf_ipc();
                    }
                }
                IpcMessage::PinAppToDock { name, icon, command, matchers } => {
                    info!("📌 IPC PinAppToDock recibido para: {}", name);
                    let app_cfg = DockAppConfig { name, icon, command, matchers };
                    let mut cfg = cfg_ipc.borrow_mut();
                    if cfg.pin_app(app_cfg) {
                        let _ = cfg.save();
                        drop(cfg);
                        on_rf_ipc();
                    }
                }
                IpcMessage::UnpinAppFromDock { name } => {
                    info!("📌 IPC UnpinAppFromDock recibido para: {}", name);
                    let mut cfg = cfg_ipc.borrow_mut();
                    if cfg.unpin_app(&name) {
                        let _ = cfg.save();
                        drop(cfg);
                        on_rf_ipc();
                    }
                }
                IpcMessage::ReloadDockConfig => {
                    info!("🔄 IPC ReloadDockConfig recibido");
                    let new_cfg = DockConfig::load_or_create_default();
                    if gtk4_layer_shell::is_supported() {
                        win_ref.set_margin(Edge::Bottom, new_cfg.floating_gap as i32);
                    }
                    headroom_ipc.set_size_request(
                        -1,
                        new_cfg
                            .magnification_max_size
                            .saturating_sub(new_cfg.icon_size) as i32,
                    );
                    *cfg_ipc.borrow_mut() = new_cfg;
                    on_rf_ipc();
                }
                IpcMessage::MinimizedWindowsChanged { windows } => {
                    let mut current_map = min_map.borrow_mut();
                    let new_ids: HashSet<u64> = windows.iter().map(|w| w.window_id).collect();

                    // Eliminar miniaturas de ventanas ya restauradas o cerradas
                    let to_remove: Vec<u64> = current_map.keys().filter(|id| !new_ids.contains(id)).copied().collect();
                    for id in to_remove {
                        if let Some(widget) = current_map.remove(&id) {
                            min_box.remove(&widget);
                            info!("🗑️ Miniatura removida del Dock: window_id={}", id);
                        }
                    }

                    // Agregar o mantener miniaturas activas
                    for win_info in windows {
                        if !current_map.contains_key(&win_info.window_id) {
                            let (widget, id) = create_minimized_window_item(&win_info, &win_ref);
                            min_box.append(&widget);
                            current_map.insert(id, widget);
                            info!("🖼️ Miniatura agregada al Dock: window_id={} ('{}')", id, win_info.title);
                        }
                    }

                    // Actualizar visibilidad del contenedor para colapsar espacio si está vacío
                    let has_minimized = !current_map.is_empty();
                    min_box.set_visible(has_minimized);
                    dock_bar_ref.queue_resize();
                    win_ref.queue_resize();
                }
                IpcMessage::ShowAppSwitcher { items, selected_index } => {
                    while let Some(c) = switcher_box_ipc.first_child() {
                        switcher_box_ipc.remove(&c);
                    }
                    for (i, item) in items.iter().enumerate() {
                        let card = GtkBox::builder()
                            .orientation(Orientation::Vertical)
                            .css_classes(["switcher-card"])
                            .spacing(6)
                            .halign(gtk4::Align::Center)
                            .valign(gtk4::Align::Center)
                            .build();

                        if i == selected_index {
                            card.add_css_class("selected");
                        }

                        let icon_name = item.icon_name.as_deref().unwrap_or("application-x-executable");
                        let icon = Image::builder()
                            .icon_name(icon_name)
                            .pixel_size(64)
                            .halign(gtk4::Align::Center)
                            .build();
                        card.append(&icon);

                        let title_label = Label::builder()
                            .label(&item.title)
                            .css_classes(["switcher-title"])
                            .ellipsize(gtk4::pango::EllipsizeMode::End)
                            .max_width_chars(14)
                            .halign(gtk4::Align::Center)
                            .build();
                        card.append(&title_label);

                        // Conmutar a la ventana al hacer clic sobre su tarjeta
                        let target_wid = item.window_id;
                        let card_click = GestureClick::new();
                        card_click.connect_released(move |_, _, _, _| {
                            let _ = send_ipc_message(&IpcMessage::SwitchToWindow { window_id: target_wid });
                        });
                        card.add_controller(card_click);

                        switcher_box_ipc.append(&card);
                    }
                    switcher_box_ipc.set_visible(true);
                    switcher_win_ipc.set_visible(true);
                    switcher_win_ipc.present();
                }
                IpcMessage::UpdateAppSwitcher { selected_index } => {
                    let mut idx = 0;
                    let mut child = switcher_box_ipc.first_child();
                    while let Some(c) = child {
                        if idx == selected_index {
                            c.add_css_class("selected");
                        } else {
                            c.remove_css_class("selected");
                        }
                        idx += 1;
                        child = c.next_sibling();
                    }
                }
                IpcMessage::HideAppSwitcher => {
                    switcher_box_ipc.set_visible(false);
                    while let Some(c) = switcher_box_ipc.first_child() {
                        switcher_box_ipc.remove(&c);
                    }
                }
                _ => {}
            }
        }
    });

    // Hilo lector del socket IPC del compositor
    std::thread::spawn(move || {
        let sock_path = ipc_socket_path();
        loop {
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let reader = std::io::BufReader::new(stream);
                for line in std::io::BufRead::lines(reader) {
                    if let Ok(line) = line {
                        if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                            let _ = tx.send_blocking(msg);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // Monitor periódico para actualizar icono de papelera (vacía vs llena)
    let trash_img_ref = trash_img;
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let is_full = check_trash_full();
        trash_img_ref.set_icon_name(Some(if is_full {
            "user-trash-full"
        } else {
            "user-trash"
        }));
        glib::ControlFlow::Continue
    });

    window.set_child(Some(&dock_root));
    window.present();
}

fn render_dock_apps(
    apps_box: &GtkBox,
    dock_config: &Rc<RefCell<DockConfig>>,
    running_cache: &Rc<RefCell<Vec<String>>>,
    dock_indicators: &Rc<RefCell<Vec<(Vec<String>, Label)>>>,
    on_refresh: &Rc<dyn Fn()>,
) {
    while let Some(child) = apps_box.first_child() {
        apps_box.remove(&child);
    }
    dock_indicators.borrow_mut().clear();

    let config = dock_config.borrow().clone();
    let running = running_cache.borrow().clone();
    let icon_size = config.icon_size as i32;
    let total_pinned = config.apps.len();

    // 1. Aplicaciones fijadas (Pinned Apps)
    for (idx, app) in config.apps.iter().enumerate() {
        let is_running = running.iter().any(|r| app.matches(r));
        let (widget, ind) = create_dock_item(
            app,
            true,
            idx,
            total_pinned,
            icon_size,
            is_running,
            running_cache.clone(),
            dock_config.clone(),
            on_refresh.clone(),
        );
        apps_box.append(&widget);
        dock_indicators.borrow_mut().push((app.matchers.clone(), ind));
    }

    // 2. Aplicaciones en ejecución no fijadas (Unpinned Running Apps)
    let mut unpinned_candidates: Vec<String> = Vec::new();
    for r in &running {
        if should_ignore_app(r) {
            continue;
        }
        if config.apps.iter().any(|a| a.matches(r)) {
            continue;
        }
        let r_lower = r.to_lowercase();
        if !unpinned_candidates.iter().any(|c| c.to_lowercase() == r_lower) {
            unpinned_candidates.push(r.clone());
        }
    }

    for cand in unpinned_candidates {
        let (name, icon, command, matchers) = resolve_desktop_app_info(&cand);
        let app_cfg = DockAppConfig {
            name,
            icon,
            command,
            matchers,
        };
        let (widget, ind) = create_dock_item(
            &app_cfg,
            false,
            0,
            0,
            icon_size,
            true,
            running_cache.clone(),
            dock_config.clone(),
            on_refresh.clone(),
        );
        apps_box.append(&widget);
        dock_indicators.borrow_mut().push((app_cfg.matchers.clone(), ind));
    }
}

fn update_icon_size_recursive(widget: &gtk4::Widget, size: i32, base_size: i32) {
    if let Ok(img) = widget.clone().downcast::<Image>() {
        // Solo ajustar iconos principales del dock, preservando insignias secundarias
        if img.pixel_size() >= (base_size - 10) && img.pixel_size() != size {
            img.set_pixel_size(size);
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        update_icon_size_recursive(&c, size, base_size);
        child = c.next_sibling();
    }
}

fn create_magnifiable_icon(icon: &Image, icon_size: i32) -> Overlay {
    let placeholder = GtkBox::new(Orientation::Horizontal, 0);
    placeholder.set_size_request(icon_size, icon_size);

    let overlay = Overlay::new();
    overlay.set_child(Some(&placeholder));
    overlay.add_overlay(icon);
    overlay.set_measure_overlay(icon, false);
    overlay.set_overflow(gtk4::Overflow::Visible);

    icon.set_halign(gtk4::Align::Center);
    icon.set_valign(gtk4::Align::End);
    icon.set_overflow(gtk4::Overflow::Visible);

    overlay
}

fn create_minimized_window_item(
    info: &MinimizedWindowInfo,
    dock_window: &ApplicationWindow,
) -> (gtk4::Widget, u64) {
    let window_id = info.window_id;

    // Mini ventana: barra superior con 3 semáforos y cuerpo con simulación de contenido
    let mini_win = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-mini-window"])
        .valign(gtk4::Align::End)
        .build();

    // 1. Barra superior con puntitos estilo semáforo macOS (rojo, amarillo, verde)
    let titlebar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["dock-mini-window-titlebar"])
        .spacing(2)
        .build();

    let dot_red = Label::builder().label("●").css_classes(["dock-mini-dot", "dot-red"]).build();
    let dot_yellow = Label::builder().label("●").css_classes(["dock-mini-dot", "dot-yellow"]).build();
    let dot_green = Label::builder().label("●").css_classes(["dock-mini-dot", "dot-green"]).build();

    titlebar.append(&dot_red);
    titlebar.append(&dot_yellow);
    titlebar.append(&dot_green);
    mini_win.append(&titlebar);

    // 2. Cuerpo de la ventana simulada con líneas de contenido
    let body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-mini-window-body"])
        .valign(gtk4::Align::Center)
        .build();

    let line1 = GtkBox::builder().css_classes(["dock-mini-content-line"]).build();
    let line2 = GtkBox::builder().css_classes(["dock-mini-content-line-short"]).build();
    body.append(&line1);
    body.append(&line2);
    mini_win.append(&body);

    // 3. Insignia circular de la app en la esquina inferior derecha (macOS style)
    let icon_name = info.icon_name.as_deref().unwrap_or("application-x-executable");
    let badge_img = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .build();

    let badge_box = GtkBox::builder()
        .css_classes(["dock-mini-badge"])
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::End)
        .build();
    badge_box.append(&badge_img);

    // Contenedor Overlay para superponer la insignia sobre la mini ventana
    let overlay = Overlay::builder()
        .child(&mini_win)
        .css_classes(["dock-window-card"])
        .tooltip_text(&info.title)
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();
    overlay.add_overlay(&badge_box);

    // Clic para restaurar la ventana vía Genie Effect inverso
    let gesture = GestureClick::new();
    let id = window_id;
    gesture.connect_pressed(move |_, _, _, _| {
        info!("✨ Clic en miniatura de Dock: restaurando ventana id={}", id);
        let _ = send_ipc_message(&IpcMessage::AnimateRestore { window_id: id });
    });
    overlay.add_controller(gesture);

    // Registro de coordenadas del slot para la animación Genie del compositor
    let overlay_clone = overlay.clone();
    let win_weak = dock_window.downgrade();
    overlay.connect_realize(move |_| {
        let card = overlay_clone.clone();
        let win_weak = win_weak.clone();

        for delay in [60, 180] {
            let card_c = card.clone();
            let win_w = win_weak.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
                if let Some(win) = win_w.upgrade() {
                    if let Some(p) = card_c.compute_point(&win, &gtk4::graphene::Point::zero()) {
                        let rel_x = p.x().max(0.0) as i32;
                        let rel_y = p.y().max(0.0) as i32;
                        let w = card_c.width().max(48) as u32;
                        let h = card_c.height().max(42) as u32;

                        info!("📍 Registrando slot de miniatura en Dock: id={}, pos=({}, {}, {}, {})", id, rel_x, rel_y, w, h);
                        let _ = send_ipc_message(&IpcMessage::RegisterDockSlot {
                            window_id: id,
                            x: rel_x,
                            y: rel_y,
                            width: w,
                            height: h,
                        });
                    }
                }
            });
        }
    });

    (overlay.upcast::<gtk4::Widget>(), window_id)
}

fn create_trash_item(icon_size: i32) -> (GtkBox, Image) {
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(1)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let btn = Button::builder()
        .css_classes(["dock-item-btn"])
        .tooltip_text("Papelera")
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();

    let trash_icon_name = if check_trash_full() {
        "user-trash-full"
    } else {
        "user-trash"
    };

    let trash_img = Image::builder()
        .icon_name(trash_icon_name)
        .pixel_size(icon_size)
        .valign(gtk4::Align::End)
        .build();
    btn.set_child(Some(&create_magnifiable_icon(&trash_img, icon_size)));

    btn.connect_clicked(|_| {
        let _ = Command::new("nautilus").arg("trash://").spawn();
    });

    // Menú Popover de Clic Derecho para la Papelera
    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(true)
        .build();

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();

    let header = Label::builder()
        .label("Papelera")
        .css_classes(["popover-menu-header"])
        .halign(gtk4::Align::Start)
        .build();
    menu_box.append(&header);

    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("popover-separator");
    menu_box.append(&sep);

    let btn_open = Button::builder()
        .label("Abrir en Archivos")
        .css_classes(["popover-menu-item"])
        .build();
    let p_open = popover.clone();
    btn_open.connect_clicked(move |_| {
        p_open.popdown();
        let _ = Command::new("nautilus").arg("trash://").spawn();
    });

    let btn_empty = Button::builder()
        .label("Vaciar Papelera")
        .css_classes(["popover-menu-item"])
        .build();
    let img_clone = trash_img.clone();
    let popover_clone = popover.clone();
    btn_empty.connect_clicked(move |_| {
        info!("🗑️ Vaciando papelera...");
        let _ = Command::new("gio").args(["trash", "--empty"]).status();
        img_clone.set_icon_name(Some("user-trash"));
        popover_clone.popdown();
    });

    menu_box.append(&btn_open);
    menu_box.append(&btn_empty);
    popover.set_child(Some(&menu_box));
    popover.set_parent(&btn);
    let p_unroot = popover.clone();
    btn.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let right_click = GestureClick::new();
    right_click.set_button(3);
    let p = popover.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        p.popup();
    });
    btn.add_controller(right_click);

    item_container.append(&btn);

    let spacer = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(0.0)
        .valign(gtk4::Align::End)
        .build();
    item_container.append(&spacer);

    (item_container, trash_img)
}

fn create_dock_item(
    app_cfg: &DockAppConfig,
    is_pinned: bool,
    idx: usize,
    total_pinned: usize,
    icon_size: i32,
    is_running: bool,
    running_cache: Rc<RefCell<Vec<String>>>,
    dock_config: Rc<RefCell<DockConfig>>,
    on_refresh: Rc<dyn Fn()>,
) -> (GtkBox, Label) {
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(1)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let btn = Button::builder()
        .css_classes(["dock-item-btn"])
        .tooltip_text(&app_cfg.name)
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();
    btn.set_overflow(gtk4::Overflow::Visible);

    let icon_name = match app_cfg.icon.as_str() {
        "system-file-manager" => "org.gnome.Nautilus",
        "utilities-terminal" => "org.gnome.Ptyxis",
        "accessories-text-editor" => "org.gnome.TextEditor",
        icon_name => icon_name,
    };
    let icon = if icon_name.starts_with('/') {
        Image::builder()
            .file(icon_name)
            .pixel_size(icon_size)
            .valign(gtk4::Align::End)
            .build()
    } else {
        Image::builder()
            .icon_name(icon_name)
            .pixel_size(icon_size)
            .valign(gtk4::Align::End)
            .build()
    };
    btn.set_child(Some(&create_magnifiable_icon(&icon, icon_size)));

    let cmd = app_cfg.command.clone();
    let name = app_cfg.name.clone();
    let matchers = app_cfg.matchers.clone();
    let cache = running_cache;

    let activate_or_spawn = {
        let cmd = cmd.clone();
        let name = name.clone();
        let matchers = matchers.clone();
        let cache = cache.clone();
        Rc::new(move || {
            let running = cache.borrow();
            let is_currently_running = running.iter().any(|r| {
                let r_lower = r.to_lowercase();
                matchers.iter().any(|m| r_lower.contains(&m.to_lowercase()))
            });
            drop(running);

            if is_currently_running && !matchers.is_empty() {
                info!("App ya en ejecución, enviando IPC ActivateApp para {}", name);
                let _ = send_ipc_message(&IpcMessage::ActivateApp {
                    app_id: matchers[0].clone(),
                });
                return;
            }

            info!("Lanzando aplicación desde Dock: {} (`{}`)", name, cmd);
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if let Some((exe, args)) = parts.split_first() {
                let target_exe = if *exe == "gnome-control-center" {
                    "malus-de-settings"
                } else {
                    *exe
                };
                let resolved_exe = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join(target_exe)))
                    .filter(|p| p.exists())
                    .or_else(|| {
                        let rel = std::path::PathBuf::from("/home/cristian/Apps/malus-de/target/release")
                            .join(target_exe);
                        if rel.exists() { Some(rel) } else { None }
                    })
                    .unwrap_or_else(|| std::path::PathBuf::from(target_exe));
                let mut command = Command::new(&resolved_exe);
                command.args(args);
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
                        let pid = child.id();
                        info!("🚀 Proceso iniciado para {}: PID={}", name, pid);
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    Err(err) => {
                        error!("❌ Error al lanzar aplicación {} desde Dock: {:?}", name, err);
                    }
                }
            }
        })
    };

    let act_click = activate_or_spawn.clone();
    btn.connect_clicked(move |_| {
        act_click();
    });

    // Menú Popover de Clic Derecho (macOS Style Context Menu)
    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(true)
        .build();

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();

    // 1. Título de la app en la cabecera
    let header = Label::builder()
        .label(&app_cfg.name)
        .css_classes(["popover-menu-header"])
        .halign(gtk4::Align::Start)
        .build();
    menu_box.append(&header);

    let sep1 = Separator::new(Orientation::Horizontal);
    sep1.add_css_class("popover-separator");
    menu_box.append(&sep1);

    // 2. Acción Abrir / Traer al frente
    let btn_open = Button::builder()
        .label(if is_running { "Traer al frente" } else { "Abrir" })
        .css_classes(["popover-menu-item"])
        .build();
    let p_open = popover.clone();
    let act_open = activate_or_spawn;
    btn_open.connect_clicked(move |_| {
        p_open.popdown();
        act_open();
    });
    menu_box.append(&btn_open);

    // 3. Opciones de anclado / organización estilo macOS
    if is_pinned {
        if idx > 0 {
            let btn_move_left = Button::builder()
                .label("Mover a la izquierda")
                .css_classes(["popover-menu-item"])
                .build();
            let p_mv_l = popover.clone();
            let cfg_mv_l = dock_config.clone();
            let on_rf_mv_l = on_refresh.clone();
            btn_move_left.connect_clicked(move |_| {
                p_mv_l.popdown();
                let mut cfg = cfg_mv_l.borrow_mut();
                if cfg.move_app(idx, idx - 1) {
                    let _ = cfg.save();
                    drop(cfg);
                    on_rf_mv_l();
                }
            });
            menu_box.append(&btn_move_left);
        }

        if idx + 1 < total_pinned {
            let btn_move_right = Button::builder()
                .label("Mover a la derecha")
                .css_classes(["popover-menu-item"])
                .build();
            let p_mv_r = popover.clone();
            let cfg_mv_r = dock_config.clone();
            let on_rf_mv_r = on_refresh.clone();
            btn_move_right.connect_clicked(move |_| {
                p_mv_r.popdown();
                let mut cfg = cfg_mv_r.borrow_mut();
                if cfg.move_app(idx, idx + 1) {
                    let _ = cfg.save();
                    drop(cfg);
                    on_rf_mv_r();
                }
            });
            menu_box.append(&btn_move_right);
        }

        let btn_unpin = Button::builder()
            .label("Eliminar del Dock")
            .css_classes(["popover-menu-item"])
            .build();
        let p_unpin = popover.clone();
        let cfg_unpin = dock_config.clone();
        let name_unpin = app_cfg.name.clone();
        let on_rf_unpin = on_refresh.clone();
        btn_unpin.connect_clicked(move |_| {
            p_unpin.popdown();
            let mut cfg = cfg_unpin.borrow_mut();
            if cfg.unpin_app(&name_unpin) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_unpin();
            }
        });
        menu_box.append(&btn_unpin);
    } else {
        let btn_pin = Button::builder()
            .label("Mantener en el Dock")
            .css_classes(["popover-menu-item"])
            .build();
        let p_pin = popover.clone();
        let cfg_pin = dock_config.clone();
        let app_to_pin = app_cfg.clone();
        let on_rf_pin = on_refresh.clone();
        btn_pin.connect_clicked(move |_| {
            p_pin.popdown();
            let mut cfg = cfg_pin.borrow_mut();
            if cfg.pin_app(app_to_pin.clone()) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_pin();
            }
        });
        menu_box.append(&btn_pin);
    }

    // 4. Forzar salida si está en ejecución
    if is_running {
        let sep2 = Separator::new(Orientation::Horizontal);
        sep2.add_css_class("popover-separator");
        menu_box.append(&sep2);

        let btn_force_quit = Button::builder()
            .label("Forzar salida")
            .css_classes(["popover-menu-item", "popover-menu-item-danger"])
            .build();
        let p_fq = popover.clone();
        let matchers_fq = matchers.clone();
        let cmd_fq = cmd.clone();
        let app_name_fq = name.clone();
        btn_force_quit.connect_clicked(move |_| {
            p_fq.popdown();
            info!("🛑 Forzando salida de {}", app_name_fq);
            for m in &matchers_fq {
                let _ = Command::new("pkill").args(["-f", m]).status();
            }
            let parts: Vec<&str> = cmd_fq.split_whitespace().collect();
            if let Some((exe, _)) = parts.split_first() {
                let _ = Command::new("killall").arg(exe).status();
            }
        });
        menu_box.append(&btn_force_quit);
    }

    popover.set_child(Some(&menu_box));
    popover.set_parent(&btn);
    let p_unroot = popover.clone();
    btn.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let right_click = GestureClick::new();
    right_click.set_button(3); // Clic derecho
    let p_rc = popover.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        p_rc.popup();
    });
    btn.add_controller(right_click);

    item_container.append(&btn);

    // Indicador de app en ejecución (Punto iluminado `●`)
    let indicator = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(if is_running { 1.0 } else { 0.0 })
        .valign(gtk4::Align::End)
        .build();
    item_container.append(&indicator);

    (item_container, indicator)
}
