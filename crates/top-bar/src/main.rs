use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::logging::init_logging;
use tracing::info;

pub mod services;
mod ui;

const STYLE_CSS: &str = include_str!("styles/style.css");

fn main() {
    init_logging("malus_de_top_bar");
    info!("Iniciando Malus DE Top Bar (GTK4 / Layer Shell)...");

    // Inicializar entorno aislado de Malus DE y limpiar variables residuales
    let _ = malus_de_common::config::setup_isolated_session_environment();

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }
    adw::StyleManager::default().set_color_scheme(
        if malus_de_common::config::is_dark_mode_active() {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        },
    );

    let app = Application::builder()
        .application_id("org.malus_de.TopBar")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_ui);

    app.run();
}

thread_local! {
    static TOPBAR_GLASS_PROVIDER: std::cell::RefCell<Option<CssProvider>> =
        const { std::cell::RefCell::new(None) };
    static TOPBAR_ACCENT_PROVIDER: std::cell::RefCell<Option<CssProvider>> =
        const { std::cell::RefCell::new(None) };
    static TOPBAR_APPEARANCE_MONITOR: std::cell::RefCell<Option<gtk4::gio::FileMonitor>> =
        const { std::cell::RefCell::new(None) };
    static TOPBAR_MENUBAR_MONITOR: std::cell::RefCell<Option<gtk4::gio::FileMonitor>> =
        const { std::cell::RefCell::new(None) };
    static TOPBAR_SYNC_VIS_CALLBACK: std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
}

fn apply_topbar_accent_color(index: usize) {
    let css = malus_de_common::config::accent_color_css(index);
    TOPBAR_ACCENT_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn build_topbar_glass_css(opacity: f64, show_background: bool) -> String {
    let scale = malus_de_common::config::liquid_glass_scale_for(opacity);
    // Misma transparencia Liquid Glass que las barras laterales de Ajustes y Branches (`0.72` base, `0.12..0.96`).
    let menu_a = malus_de_common::config::scaled_liquid_glass_alpha(0.72, scale, 0.12, 0.96);
    let side_tint = malus_de_common::config::sidebar_tint_background_image_css(
        malus_de_common::config::load_tint_windows_with_wallpaper(),
    );
    let bar_bg = if show_background {
        let bar_a = malus_de_common::config::scaled_liquid_glass_alpha(0.46, scale, 0.16, 0.86);
        format!(
            "window.malus-topbar, \
             window.malus-topbar.background, \
             window.malus-topbar decoration {{ \
                 background-image: none; \
                 background-color: alpha(@window_bg_color, {bar_a:.3}); \
                 border-bottom: 1px solid alpha(@borders, 0.22); \
                 color: @window_fg_color; \
                 text-shadow: none; \
                 -gtk-icon-shadow: none; \
             }} \
             window.malus-topbar button.topbar-btn, \
             window.malus-topbar menubutton.topbar-btn>button, \
             window.malus-topbar menubutton>button, \
             window.malus-topbar label.clock-label, \
             window.malus-topbar .active-app-name, \
             window.malus-topbar .global-menu-item, \
             window.malus-topbar .clock-label, \
             window.malus-topbar button.control-center-toggle image, \
             window.malus-topbar menubutton.control-center-toggle image, \
             window.malus-topbar .topbar-content label, \
             window.malus-topbar .topbar-content image, \
             window.malus-topbar .topbar-content>box>image, \
             window.malus-topbar .topbar-content>box>button>image, \
             window.malus-topbar .topbar-content>box>menubutton>button image, \
             window.malus-topbar button.topbar-btn>image, \
             window.malus-topbar menubutton.topbar-btn>button image {{ \
                 color: @window_fg_color; \
                 text-shadow: none; \
                 -gtk-icon-shadow: none; \
             }} \
             window.malus-topbar button.topbar-btn:hover, \
             window.malus-topbar menubutton.topbar-btn>button:hover, \
             window.malus-topbar menubutton>button:hover, \
             window.malus-topbar label.clock-label:hover {{ \
                 background-color: alpha(@window_fg_color, 0.08); \
             }} \
             window.malus-topbar button.topbar-btn:active, \
             window.malus-topbar button.topbar-btn:checked, \
             window.malus-topbar menubutton.topbar-btn>button:active, \
             window.malus-topbar menubutton.topbar-btn>button:checked {{ \
                 background-color: alpha(@window_fg_color, 0.12); \
             }}"
        )
    } else {
        "window.malus-topbar, \
         window.malus-topbar.background, \
         window.malus-topbar decoration { \
             background: transparent; \
             background-color: transparent; \
             border-bottom: none; \
         }"
        .to_string()
    };
    format!(
        "{bar_bg} \
         popover.malus-popover-menu>contents, \
         popover.control-center-popover>contents {{ \
             background-color: rgba(30, 30, 36, {menu_a:.3}); \
             background-image: {side_tint}; \
         }} \
         popover.light-popover.malus-popover-menu>contents, \
         popover.light-popover>contents, \
         .light-mode popover.malus-popover-menu>contents, \
         popover.light-popover.control-center-popover>contents, \
         .light-mode popover.control-center-popover>contents {{ \
             background-color: rgba(244, 246, 250, {menu_a:.3}); \
             background-image: {side_tint}; \
         }}"
    )
}

fn apply_topbar_liquid_glass_opacity(opacity: f64) {
    let show_background = malus_de_common::config::MenuBarConfig::load().show_background;
    let css = build_topbar_glass_css(opacity, show_background);
    TOPBAR_GLASS_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(STYLE_CSS);
    let glass_provider = CssProvider::new();
    let accent_provider = CssProvider::new();

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );
        gtk4::style_context_add_provider_for_display(
            &display,
            &glass_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );
        gtk4::style_context_add_provider_for_display(
            &display,
            &accent_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 2,
        );

        let icon_theme = gtk4::IconTheme::for_display(&display);
        for icon_dir in malus_de_common::apps::extra_icon_search_paths() {
            if icon_dir.is_dir() {
                icon_theme.add_search_path(&icon_dir);
            }
        }
        icon_theme.add_search_path("/usr/local/share/malus-de/icons");
        icon_theme.add_search_path("/usr/share/malus-de/icons");
        if let Ok(home) = std::env::var("HOME") {
            let config_icons = std::path::PathBuf::from(&home).join(".config/malus-de/icons");
            let local_icons = std::path::PathBuf::from(&home).join(".local/share/icons");
            icon_theme.add_search_path(&config_icons);
            icon_theme.add_search_path(&local_icons);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                icon_theme.add_search_path(parent.join("assets/icons"));
                icon_theme.add_search_path(parent.join("../../assets/icons"));
                icon_theme.add_search_path(parent.join("../../data/icons"));
            }
        }
    }

    TOPBAR_GLASS_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(glass_provider);
    });
    TOPBAR_ACCENT_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(accent_provider);
    });
    apply_topbar_accent_color(malus_de_common::config::load_accent_color_index());
    apply_topbar_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());

    let config_file =
        gtk4::gio::File::for_path(malus_de_common::config::appearance_settings_path());
    if let Ok(monitor) = config_file.monitor_file(
        gtk4::gio::FileMonitorFlags::NONE,
        gtk4::gio::Cancellable::NONE,
    ) {
        monitor.connect_changed(|_, _, _, _| {
            malus_de_common::config::invalidate_appearance_settings_cache();
            apply_topbar_accent_color(malus_de_common::config::load_accent_color_index());
            apply_topbar_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());
        });
        TOPBAR_APPEARANCE_MONITOR.with(|cell| {
            *cell.borrow_mut() = Some(monitor);
        });
    }

    let menubar_file =
        gtk4::gio::File::for_path(malus_de_common::config::MenuBarConfig::config_path());
    if let Ok(mb_monitor) = menubar_file.monitor_file(
        gtk4::gio::FileMonitorFlags::NONE,
        gtk4::gio::Cancellable::NONE,
    ) {
        mb_monitor.connect_changed(|_, _, _, _| {
            malus_de_common::config::invalidate_appearance_settings_cache();
            apply_topbar_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());
            ui::status_tray::refresh_tray_visibility();
            ui::power_profile::refresh_power_profile_visibility();
            ui::auto_tiling::refresh_auto_tiling_visibility();
            ui::clock::refresh_clock_now();
            ui::system_menu::refresh_system_menu_logo();
            TOPBAR_SYNC_VIS_CALLBACK.with(|cell| {
                if let Some(ref f) = *cell.borrow() {
                    f();
                }
            });
        });
        TOPBAR_MENUBAR_MONITOR.with(|cell| {
            *cell.borrow_mut() = Some(mb_monitor);
        });
    }
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus DE Top Bar")
        .default_height(36)
        .css_classes(["malus-topbar"])
        .build();

    window.set_size_request(-1, 36);
    window.set_default_size(-1, 36);

    window.set_decorated(false);

    if gtk4_layer_shell::is_supported() {
        info!("Protocolo Layer Shell soportado. Anclando Top Bar en capa Top...");
        window.init_layer_shell();
        if let Some(display) = gtk4::gdk::Display::default() {
            let displays_cfg = malus_de_common::config::DisplaysConfig::load();
            if let Some(primary_mon) = find_primary_gdk_monitor(&display, &displays_cfg) {
                window.set_monitor(&primary_mon);
            }
        }
        window.set_layer(Layer::Top);
        window.set_namespace("malus-top-bar");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.auto_exclusive_zone_enable();
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    } else {
        info!("Layer Shell no detectado en este display; ejecutando ventana flotante.");
    }

    let root_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["topbar-content"])
        .valign(gtk4::Align::Center)
        .spacing(8)
        .build();

    // 1. Zona Izquierda: Menú del sistema + Global Menu (con soporte com.canonical.AppMenu.Registrar)
    let left_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(gtk4::Align::Center)
        .spacing(4)
        .build();

    let (dbus_menu_service, registrar) = crate::services::dbusmenu::DbusMenuService::new();
    crate::services::dbusmenu::DbusMenuService::start_registrar_server(registrar);
    let dbus_service_arc = std::sync::Arc::new(dbus_menu_service);

    let menu_handles = ui::global_menu::create_global_menu_widget(dbus_service_arc.clone());
    left_box.append(&ui::system_menu::create_system_menu());
    left_box.append(&menu_handles.container);

    // Escuchar eventos de registro de ventanas con menús D-Bus
    let handles_for_reg = menu_handles.clone();
    let reg_rx = dbus_service_arc.update_rx.clone();
    glib::spawn_future_local(async move {
        while let Ok(reg) = reg_rx.recv().await {
            let (app_id, title, window_id) = {
                let cur = handles_for_reg.current_active.borrow();
                (cur.0.clone(), cur.1.clone(), cur.2)
            };
            let should_update = match window_id {
                Some(wid) => (wid as u32) == reg.window_id,
                None => true,
            };
            if should_update {
                info!(
                    "🔄 [TopBar] Actualizando menú dinámico tras registro D-Bus de ventana {}",
                    reg.window_id
                );
                ui::global_menu::update_global_menu(
                    &handles_for_reg,
                    app_id.as_deref(),
                    title.as_deref(),
                    window_id,
                );
            }
        }
    });

    // Escuchar eventos IPC del compositor para actualizar el nombre de la aplicación activa y acciones
    let (tx, rx) = async_channel::unbounded::<malus_de_common::ipc::IpcMessage>();
    let title_label = menu_handles.app_title_label.clone();
    let hide_btn = menu_handles.hide_btn.clone();
    let quit_btn = menu_handles.quit_btn.clone();
    let handles_clone = menu_handles.clone();

    adw::StyleManager::default().connect_dark_notify(|sm| {
        let is_dark = sm.is_dark();
        ui::theme::update_all_popovers_theme(is_dark);
    });

    let has_active_app = std::rc::Rc::new(std::cell::Cell::new(false));
    let sync_bar_slot: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    let win_ipc = window.clone();
    let active_app_ipc = has_active_app.clone();
    let sync_slot_ipc = sync_bar_slot.clone();
    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                malus_de_common::ipc::IpcMessage::ActiveAppChanged {
                    app_id,
                    title,
                    window_id,
                } => {
                    active_app_ipc.set(app_id.as_ref().is_some_and(|id| !id.is_empty()));
                    if let Some(ref f) = *sync_slot_ipc.borrow() {
                        f();
                    }
                    let display = format_app_name(app_id.as_deref(), title.as_deref());
                    title_label.set_label(&display);
                    let active_for_sys = if app_id.as_ref().is_some_and(|id| !id.is_empty()) {
                        Some(display.as_str())
                    } else {
                        None
                    };
                    ui::system_menu::update_active_app_for_system_menu(active_for_sys);
                    let es = std::env::var("LC_ALL")
                        .or_else(|_| std::env::var("LC_MESSAGES"))
                        .or_else(|_| std::env::var("LANG"))
                        .unwrap_or_default()
                        .to_lowercase()
                        .starts_with("es");
                    let hide_txt = if es {
                        format!("Ocultar {}", display)
                    } else {
                        format!("Hide {}", display)
                    };
                    let quit_txt = if es {
                        format!("Salir de {}", display)
                    } else {
                        format!("Quit {}", display)
                    };
                    ui::global_menu::update_menu_row_label(&hide_btn, &hide_txt);
                    ui::global_menu::update_menu_row_label(&quit_btn, &quit_txt);
                    ui::global_menu::update_global_menu(
                        &handles_clone,
                        app_id.as_deref(),
                        title.as_deref(),
                        window_id,
                    );
                }
                malus_de_common::ipc::IpcMessage::ThemeChanged { is_dark, .. } => {
                    adw::StyleManager::default().set_color_scheme(if is_dark {
                        adw::ColorScheme::ForceDark
                    } else {
                        adw::ColorScheme::ForceLight
                    });
                    ui::theme::update_all_popovers_theme(is_dark);
                    ui::system_menu::refresh_system_menu_logo();
                }
                malus_de_common::ipc::IpcMessage::AccentColorChanged { index } => {
                    apply_topbar_accent_color(index);
                    win_ipc.queue_draw();
                }
                malus_de_common::ipc::IpcMessage::LiquidGlassOpacityChanged { opacity } => {
                    info!(
                        "✨ [TopBar] IPC LiquidGlassOpacityChanged recibido: opacity={:.2}",
                        opacity
                    );
                    apply_topbar_liquid_glass_opacity(opacity);
                    win_ipc.queue_draw();
                }
                malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig => {
                    info!("🖥 [TopBar] IPC ReloadDisplaysConfig recibido — reajustando top bar");
                    win_ipc.queue_resize();
                    win_ipc.queue_draw();
                }
                malus_de_common::ipc::IpcMessage::ReloadMenuBarConfig => {
                    info!("🎛 [TopBar] IPC ReloadMenuBarConfig recibido — aplicando Barra de menús");
                    apply_topbar_liquid_glass_opacity(
                        malus_de_common::config::load_liquid_glass_opacity(),
                    );
                    ui::status_tray::refresh_tray_visibility();
                    ui::power_profile::refresh_power_profile_visibility();
                    ui::auto_tiling::refresh_auto_tiling_visibility();
                    ui::clock::refresh_clock_now();
                    ui::system_menu::refresh_system_menu_logo();
                    if let Some(ref f) = *sync_slot_ipc.borrow() {
                        f();
                    }
                    win_ipc.queue_draw();
                }
                malus_de_common::ipc::IpcMessage::AutoTilingChanged { enabled } => {
                    ui::auto_tiling::set_auto_tiling_visual(enabled);
                    win_ipc.queue_draw();
                }
                malus_de_common::ipc::IpcMessage::KeyboardLayoutChanged { .. }
                | malus_de_common::ipc::IpcMessage::ReloadInputConfig => {
                    ui::status_tray::refresh_keyboard_layout_badge();
                }
                _ => {}
            }
        }
    });

    if let Some(display) = gtk4::gdk::Display::default() {
        let win_mon = window.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            win_mon.queue_resize();
            win_mon.queue_draw();
        });
    }

    std::thread::spawn(move || {
        let sock_path = malus_de_common::ipc::ipc_socket_path();
        loop {
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let reader = std::io::BufReader::new(stream);
                for line in std::io::BufRead::lines(reader) {
                    if let Ok(line) = line {
                        if let Ok(msg) =
                            serde_json::from_str::<malus_de_common::ipc::IpcMessage>(&line)
                        {
                            let _ = tx.send_blocking(msg);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // 2. Zona Central: Espaciador expansible
    let center_spacer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();

    // 3. Zona Derecha: Auto-Tiling + Power Mode + Tray + Clock
    let right_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(gtk4::Align::Center)
        .spacing(6)
        .build();
    right_box.append(&ui::auto_tiling::create_auto_tiling_widget());
    right_box.append(&ui::power_profile::create_power_profile_widget());
    right_box.append(&ui::status_tray::create_status_tray_widget());
    right_box.append(&ui::clock::create_clock_widget());

    root_box.append(&left_box);
    root_box.append(&center_spacer);
    root_box.append(&right_box);

    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideDown)
        .transition_duration(180)
        .reveal_child(true)
        .child(&root_box)
        .build();

    let bar_hovered = std::rc::Rc::new(std::cell::Cell::new(false));
    let last_applied_mode = std::rc::Rc::new(std::cell::Cell::new(None::<u8>));
    let last_input_region = std::rc::Rc::new(std::cell::Cell::new(None::<(i32, i32)>));
    let has_custom_input_region = std::rc::Rc::new(std::cell::Cell::new(false));
    let sync_bar_vis: std::rc::Rc<dyn Fn()> = {
        let win_c = window.clone();
        let rev_c = revealer.clone();
        let hov_c = bar_hovered.clone();
        let act_c = has_active_app;
        let last_mode_c = last_applied_mode;
        let last_reg_c = last_input_region;
        let custom_reg_c = has_custom_input_region;
        std::rc::Rc::new(move || {
            let mode = malus_de_common::config::MenuBarConfig::load().auto_hide_mode;
            if last_mode_c.get() != Some(mode) {
                if gtk4_layer_shell::is_supported() {
                    match mode {
                        3 => {
                            win_c.set_layer(Layer::Overlay);
                            win_c.auto_exclusive_zone_enable();
                        }
                        1 | 2 => {
                            win_c.set_layer(Layer::Top);
                            win_c.set_exclusive_zone(0);
                        }
                        _ => {
                            win_c.set_layer(Layer::Top);
                            win_c.auto_exclusive_zone_enable();
                        }
                    }
                }
                last_mode_c.set(Some(mode));
            }
            let should_show = match mode {
                1 => hov_c.get(),
                2 => !act_c.get() || hov_c.get(),
                _ => true,
            };
            if rev_c.reveals_child() != should_show {
                rev_c.set_reveal_child(should_show);
            }
            // Solo restringir input_region cuando auto-hide está activo o recuperándose de un recorte,
            // y nunca aplicar una región de 1 px antes del layout inicial (width <= 1).
            let needs_region_update = !should_show || custom_reg_c.get();
            if needs_region_update {
                let w = win_c.width();
                if w > 1 {
                    if let Some(surface) = win_c.surface() {
                        let h = if should_show {
                            win_c.height().max(36)
                        } else {
                            2
                        };
                        if last_reg_c.get() != Some((w, h)) {
                            let region = gtk4::cairo::Region::create_rectangle(
                                &gtk4::cairo::RectangleInt::new(0, 0, w, h),
                            );
                            surface.set_input_region(&region);
                            last_reg_c.set(Some((w, h)));
                            custom_reg_c.set(true);
                            // Forzar commit Wayland para que el nuevo input_region surta efecto de inmediato.
                            win_c.queue_draw();
                        }
                    }
                }
            }
        })
    };
    *sync_bar_slot.borrow_mut() = Some(sync_bar_vis.clone());
    TOPBAR_SYNC_VIS_CALLBACK.with(|cell| {
        *cell.borrow_mut() = Some(sync_bar_vis.clone());
    });

    let motion = gtk4::EventControllerMotion::new();
    {
        let hov_enter = bar_hovered.clone();
        let sync_enter = sync_bar_vis.clone();
        motion.connect_enter(move |_, _, _| {
            hov_enter.set(true);
            sync_enter();
        });
    }
    {
        let hov_leave = bar_hovered;
        let sync_leave = sync_bar_vis.clone();
        motion.connect_leave(move |_| {
            hov_leave.set(false);
            sync_leave();
        });
    }
    window.add_controller(motion);

    window.set_child(Some(&revealer));
    window.present();
    sync_bar_vis();
}

fn find_primary_gdk_monitor(
    display: &gtk4::gdk::Display,
    cfg: &malus_de_common::config::DisplaysConfig,
) -> Option<gtk4::gdk::Monitor> {
    let monitors = display.monitors();
    let monitor_list: Vec<gtk4::gdk::Monitor> = (0..monitors.n_items())
        .filter_map(|i| monitors.item(i))
        .filter_map(|m| m.downcast::<gtk4::gdk::Monitor>().ok())
        .collect();
    if let Some(primary_cfg) = cfg.primary_output() {
        if let Some(matched) = monitor_list.iter().find(|monitor| {
            let geo = monitor.geometry();
            primary_cfg.matches_monitor_identity(
                monitor.connector().as_deref(),
                monitor.model().as_deref(),
                monitor.description().as_deref(),
                geo.x(),
                geo.y(),
            )
        }) {
            return Some(matched.clone());
        }
    }
    monitor_list.into_iter().next()
}

fn format_app_name(app_id: Option<&str>, title: Option<&str>) -> String {
    let es = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es");

    if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.contains("firefox") || lower.contains("mozilla") {
            return "Firefox".to_string();
        } else if lower.contains("libreoffice") || lower.contains("soffice") {
            if lower.contains("writer") {
                return "LibreOffice Writer".to_string();
            } else if lower.contains("calc") {
                return "LibreOffice Calc".to_string();
            } else if lower.contains("impress") {
                return "LibreOffice Impress".to_string();
            } else {
                return "LibreOffice".to_string();
            }
        } else if lower.contains("nautilus") || lower.contains("archivos") {
            return if es { "Archivos" } else { "Files" }.to_string();
        } else if lower.contains("calculator") || lower.contains("calculadora") {
            return if es { "Calculadora" } else { "Calculator" }.to_string();
        } else if lower.contains("ptyxis") || lower.contains("terminal") {
            return "Terminal".to_string();
        } else if lower.contains("texteditor")
            || lower.contains("text-editor")
            || lower.contains("gedit")
        {
            return if es { "Editor de Texto" } else { "Text Editor" }.to_string();
        } else if lower.contains("control-center") || lower.contains("settings") {
            return if es { "Configuración" } else { "Settings" }.to_string();
        }
    }

    if let Some(t) = title {
        let trimmed = t.trim();
        let lower = trimmed.to_lowercase();
        if lower.contains("firefox") {
            return "Firefox".to_string();
        } else if lower.contains("libreoffice") || lower.contains("writer") {
            return "LibreOffice Writer".to_string();
        } else if lower.contains("calc") {
            return "LibreOffice Calc".to_string();
        } else if lower.contains("impress") {
            return "LibreOffice Impress".to_string();
        }
        if !trimmed.is_empty() {
            if let Some((_, last)) = trimmed.rsplit_once(" — ") {
                return last.trim().to_string();
            }
            if let Some((_, last)) = trimmed.rsplit_once(" - ") {
                return last.trim().to_string();
            }
            return trimmed.to_string();
        }
    }

    if let Some(id) = app_id {
        if let Some(last) = id.rsplit('.').next() {
            if !last.is_empty() {
                return last.to_string();
            }
        }
    }

    "Malus DE".to_string()
}

#[cfg(test)]
mod tests {
    use super::build_topbar_glass_css;

    #[test]
    fn topbar_background_removes_shadows_and_uses_theme_foreground_color() {
        let off = build_topbar_glass_css(0.65, false);
        assert!(off.contains("background-color: transparent;"));
        assert!(!off.contains("text-shadow: none;"));

        let on = build_topbar_glass_css(0.65, true);
        assert!(on.contains("background-color: alpha(@window_bg_color,"));
        assert!(on.contains("color: @window_fg_color;"));
        assert!(on.contains("text-shadow: none;"));
        assert!(on.contains("-gtk-icon-shadow: none;"));
    }
}
