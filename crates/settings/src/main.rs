use clap::Parser;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, CssProvider, DropDown,
    DrawingArea, Entry, EventControllerFocus, FileDialog, FileFilter, FlowBox, GestureClick,
    GestureDrag, Grid, Image, Label, Orientation, Overlay, PackType, Paned, Picture, PolicyType,
    Scale, ScrolledWindow, SearchEntry, Stack, StackTransitionType, Switch, WindowControls,
    WindowHandle,
};
use malus_de_common::config::{
    AppSwitcherStyle, DesktopIconsConfig, DisplayModeConfig, DisplaysConfig, DockConfig, InputConfig,
};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::logging::init_logging;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

mod wifi_pane;
mod bluetooth_pane;
mod network_pane;
mod vpn_pane;
mod battery_pane;

const APP_ID: &str = "dev.malus.Settings";
const SETTINGS_SIDEBAR_WIDTH: i32 = 212;
const SIDEBAR_SEARCH_MAX_HEIGHT: i32 = 36;
const SIDEBAR_SEARCH_HEIGHT: i32 = 34;
// Con superelipse n=4.2, r=17 px iguala la profundidad visual de esquina (~2.6 px) de `border-radius: 9px`.
const SIDEBAR_SEARCH_SQUIRCLE_RADIUS: f64 = 17.0;
const SQUIRCLE_EXPONENT: f64 = 4.2;
const SQUIRCLE_SEGMENTS: usize = 20;

#[derive(Parser, Debug, Clone)]
#[command(name = "malus-de-settings")]
#[command(about = "Ajustes del Sistema de Malus DE (macOS System Settings)")]
struct Args {
    /// Panel inicial a mostrar: wifi, bluetooth, network, network-firewall, vpn, battery, displays, wallpaper, keyboard, appearance, dock, spotlight, sound, about
    #[arg(short, long, default_value = "spotlight")]
    panel: String,

    /// Establecer tema GTK directamente (ej: WhiteSur-Dark, WhiteSur-Light, Adwaita) y salir
    #[arg(long)]
    theme: Option<String>,
}

fn main() {
    init_logging("malus_de_settings");
    let _ = malus_de_common::config::setup_isolated_session_environment();

    let _ = gtk4::init();
    if let Err(err) = libadwaita::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let args = Args::parse();
    if let Some(theme) = args.theme {
        set_gtk_theme(&theme);
        println!("✓ Tema GTK '{}' aplicado con éxito a GTK3, GTK4 y Libadwaita.", theme);
        return;
    }

    libadwaita::StyleManager::default().set_color_scheme(
        if is_dark_mode_active() {
            libadwaita::ColorScheme::ForceDark
        } else {
            libadwaita::ColorScheme::ForceLight
        }
    );

    // Escuchar reactivamente cambios en el modo de apariencia desde el Centro de Control o scripts
    let gsettings = Box::leak(Box::new(gtk4::gio::Settings::new("org.gnome.desktop.interface")));
    gsettings.connect_changed(Some("color-scheme"), move |settings, _| {
        let scheme = settings.string("color-scheme");
        let to_dark = scheme.contains("dark");
        tracing::info!("🎨 Detección reactiva de cambio de apariencia: {} (to_dark: {})", scheme, to_dark);
        libadwaita::StyleManager::default().set_color_scheme(
            if to_dark {
                libadwaita::ColorScheme::ForceDark
            } else {
                libadwaita::ColorScheme::ForceLight
            }
        );
    });

    let initial_panel = args.panel.clone();

    let app = Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_startup(|_| {
        load_macos_settings_css();
    });

    app.connect_activate(move |app| {
        if let Some(win) = app.active_window().or_else(|| app.windows().into_iter().next()) {
            win.present();
            return;
        }
        build_settings_window(app, &initial_panel);
    });

    app.run_with_args::<&str>(&[]);
}

fn load_macos_settings_css() {
    let provider = CssProvider::new();
    let css = include_str!("styles/macos-settings.css");
    provider.load_from_string(css);
    if let Some(display) = gtk4::gdk::Display::default() {
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

pub(crate) fn squircle_path(context: &gtk4::cairo::Context, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(1.0);
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }
    context.close_path();
}

fn build_sidebar_search_entry() -> SearchEntry {
    let clamped_height = SIDEBAR_SEARCH_HEIGHT.min(SIDEBAR_SEARCH_MAX_HEIGHT);
    SearchEntry::builder()
        .placeholder_text("Buscar")
        .height_request(clamped_height)
        .valign(Align::Center)
        .vexpand(false)
        .hexpand(true)
        .css_classes(["sidebar-search-input"])
        .build()
}

pub(crate) fn make_system_glyph_badge(
    icon_name: &str,
    css_classes: &[&str],
    box_size: i32,
    icon_px: i32,
) -> GtkBox {
    let badge = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::Center)
        .valign(Align::Center)
        .width_request(box_size)
        .height_request(box_size)
        .build();
    for cls in css_classes {
        badge.add_css_class(cls);
    }
    let glyph = Image::builder()
        .icon_name(icon_name)
        .pixel_size(icon_px)
        .halign(Align::Center)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();
    badge.append(&glyph);
    badge
}

const SIDEBAR_ROW_HEIGHT: i32 = 32;

fn build_sidebar_nav_button(icon_name: &str, label_txt: &str) -> Button {
    let btn = Button::builder()
        .css_classes(["sidebar-btn"])
        .has_frame(false)
        .height_request(SIDEBAR_ROW_HEIGHT)
        .halign(Align::Fill)
        .valign(Align::Start)
        .hexpand(true)
        .vexpand(false)
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_start(4)
        .margin_end(4)
        .halign(Align::Fill)
        .valign(Align::Center)
        .hexpand(true)
        .vexpand(false)
        .build();

    let glyph = Image::builder()
        .icon_name(icon_name)
        .pixel_size(16)
        .css_classes(["settings-sidebar-icon"])
        .halign(Align::Start)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();

    let lbl = Label::builder()
        .label(label_txt)
        .xalign(0.0)
        .halign(Align::Start)
        .valign(Align::Center)
        .hexpand(true)
        .vexpand(false)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    row.append(&glyph);
    row.append(&lbl);
    btn.set_child(Some(&row));
    btn
}

fn create_squircle_search_shell(entry: &SearchEntry) -> Overlay {
    let clamped_height = SIDEBAR_SEARCH_HEIGHT.min(SIDEBAR_SEARCH_MAX_HEIGHT);
    let shell = Overlay::builder()
        .height_request(clamped_height)
        .valign(Align::Center)
        .vexpand(false)
        .hexpand(true)
        .css_classes(["sidebar-search-shell"])
        .build();

    let bg = DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .build();

    let focus_ctrl = EventControllerFocus::new();
    let focus_state = focus_ctrl.clone();
    let bg_draw = bg.clone();
    focus_ctrl.connect_enter(move |_| bg_draw.queue_draw());
    let bg_draw_leave = bg.clone();
    focus_ctrl.connect_leave(move |_| bg_draw_leave.queue_draw());
    entry.add_controller(focus_ctrl);

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    bg.set_draw_func(move |_, cr, w, h| {
        let w = w as f64;
        let h = h as f64;
        if w <= 2.0 || h <= 2.0 {
            return;
        }
        let is_dark = libadwaita::StyleManager::default().is_dark();
        let focused = focus_state.contains_focus();

        if cr.save().is_ok() {
            squircle_path(cr, w, h, SIDEBAR_SEARCH_SQUIRCLE_RADIUS);
            if is_dark {
                cr.set_source_rgba(1.0, 1.0, 1.0, if focused { 0.11 } else { 0.08 });
            } else {
                cr.set_source_rgba(0.0, 0.0, 0.0, if focused { 0.075 } else { 0.055 });
            }
            let _ = cr.fill();
            let _ = cr.restore();
        }

        if cr.save().is_ok() {
            let inset = 0.75;
            cr.translate(inset, inset);
            squircle_path(
                cr,
                (w - inset * 2.0).max(1.0),
                (h - inset * 2.0).max(1.0),
                (SIDEBAR_SEARCH_SQUIRCLE_RADIUS - inset).max(2.0),
            );
            if focused {
                if is_dark {
                    cr.set_source_rgba(0.04, 0.52, 1.0, 0.85);
                } else {
                    cr.set_source_rgba(0.0, 0.48, 1.0, 0.80);
                }
                cr.set_line_width(1.5);
            } else {
                if is_dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.15);
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.13);
                }
                cr.set_line_width(1.0);
            }
            let _ = cr.stroke();
            let _ = cr.restore();
        }
    });

    shell.set_child(Some(&bg));
    shell.add_overlay(entry);
    shell.set_measure_overlay(entry, true);
    shell
}

fn build_settings_window(app: &Application, initial_panel: &str) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Ajustes del Sistema")
        .default_width(940)
        .default_height(680)
        .decorated(true)
        .css_classes(["macos-settings-window"])
        .build();

    // Reemplaza la barra superior de GNOME por un contenedor invisible,
    // preservando los márgenes CSD de redimensión interactiva y la sombra perimetral.
    let dummy_titlebar = GtkBox::builder()
        .visible(false)
        .build();
    window.set_titlebar(Some(&dummy_titlebar));

    // ==========================================
    // 1. Barra Lateral Estilo macOS / Branches (212 px)
    // ==========================================
    let sidebar = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-sidebar"])
        .hexpand(false)
        .vexpand(true)
        .build();

    // Controles de ventana nativos (Cerrar, Minimizar, Maximizar) según el tema activo
    let window_controls_start = WindowControls::builder()
        .side(PackType::Start)
        .valign(Align::Center)
        .build();

    let controls_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sidebar-windowcontrols"])
        .valign(Align::Center)
        .build();
    controls_box.append(&window_controls_start);

    let traffic_handle = WindowHandle::builder()
        .child(&controls_box)
        .build();
    sidebar.append(&traffic_handle);

    let sidebar_body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .valign(Align::Start)
        .vexpand(false)
        .css_classes(["sidebar-body"])
        .build();

    // Buscador de Ajustes (altura máx. 36 px con squircle real n=4.2)
    let search_entry = build_sidebar_search_entry();
    let search_shell = create_squircle_search_shell(&search_entry);
    sidebar_body.append(&search_shell);

    // Tarjeta de Perfil de Usuario estilo Apple ID
    let profile_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["profile-card"])
        .build();

    let user_name = std::env::var("USER").unwrap_or_else(|_| "cristian".to_string());
    let initial_char = user_name
        .chars()
        .next()
        .unwrap_or('C')
        .to_uppercase()
        .to_string();

    let avatar = Label::builder()
        .label(&initial_char)
        .css_classes(["avatar-circle"])
        .valign(Align::Center)
        .halign(Align::Center)
        .build();

    let profile_text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .spacing(2)
        .hexpand(true)
        .build();
    let profile_name = Label::builder()
        .label(&format!("{}{}", &initial_char, &user_name.chars().skip(1).collect::<String>()))
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    let profile_sub = Label::builder()
        .label("Malus DE · Ecosistema Activo")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    profile_text_box.append(&profile_name);
    profile_text_box.append(&profile_sub);

    profile_card.append(&avatar);
    profile_card.append(&profile_text_box);
    sidebar_body.append(&profile_card);

    // Items de navegación de la barra lateral con glifos del sistema (estilo Branches)
    let nav_items = [
        ("wifi", "network-wireless-symbolic", "Wi-Fi"),
        ("bluetooth", "bluetooth-active-symbolic", "Bluetooth"),
        ("network", "network-workgroup-symbolic", "Red"),
        ("vpn", "network-vpn-symbolic", "VPN"),
        ("battery", "battery-level-100-symbolic", "Batería"),
        ("appearance", "preferences-desktop-appearance-symbolic", "Aspecto"),
        ("dock", "user-desktop-symbolic", "Escritorio y Dock"),
        ("wallpaper", "preferences-desktop-wallpaper-symbolic", "Fondo de pantalla"),
        ("displays", "video-display-symbolic", "Pantallas"),
        ("spotlight", "system-search-symbolic", "Spotlight"),
        ("sound", "audio-volume-high-symbolic", "Sonido"),
        ("keyboard", "input-keyboard-symbolic", "Teclado"),
        ("about", "preferences-system-symbolic", "General"),
    ];

    let stack = Stack::builder()
        .transition_type(StackTransitionType::Crossfade)
        .transition_duration(160)
        .hexpand(true)
        .vexpand(true)
        .build();

    let mut sidebar_buttons: Vec<(String, Button)> = Vec::new();

    for (id, icon_name, label_txt) in nav_items {
        let btn = build_sidebar_nav_button(icon_name, label_txt);
        sidebar_body.append(&btn);
        sidebar_buttons.push((id.to_string(), btn));
    }

    let sidebar_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .propagate_natural_height(true)
        .child(&sidebar_body)
        .hexpand(true)
        .vexpand(true)
        .build();
    sidebar.append(&sidebar_scroll);

    // Filtro interactivo de búsqueda de la barra lateral
    let btns_for_search = sidebar_buttons.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_lowercase();
        for (id, btn) in &btns_for_search {
            if query.is_empty() {
                btn.set_visible(true);
            } else {
                let matches = id.contains(&query) || match id.as_str() {
                    "wifi" => "wi-fi wifi red inalambrica internet redes",
                    "bluetooth" => "bluetooth dispositivos audio auriculares teclado raton inalambrico emparejar",
                    "network" => "red network ethernet puente thunderbolt firewall firewalld servicios ubicaciones",
                    "vpn" => "vpn wireguard openvpn ipsec ikev2 l2tp cisco red privada",
                    "battery" => "bateria battery energia recarga condicion ciclos ahorro sdr hdr reposo",
                    "displays" => "pantallas monitor resolucion ppi truesize",
                    "wallpaper" => "fondo pantalla wallpaper imagen",
                    "keyboard" => "teclado distribucion lenguaje idioma xkb",
                    "appearance" => "aspecto tema oscuro claro cursor",
                    "dock" => "escritorio dock iconos aladino genie",
                    "spotlight" => "spotlight busqueda aplicaciones archivos",
                    "sound" => "sonido volumen audio",
                    "about" => "acerca sistema procesador ram version general",
                    _ => "",
                }.contains(&query);
                btn.set_visible(matches);
            }
        }
    });

    // ==========================================
    // 2. Construcción de los Paneles de Contenido
    // ==========================================
    let (wifi_pane, refresh_wifi_pane) = wifi_pane::create_wifi_pane(&window);
    stack.add_named(&wifi_pane, Some("wifi"));

    let (bluetooth_pane, refresh_bluetooth_pane) = bluetooth_pane::create_bluetooth_pane(&window);
    stack.add_named(&bluetooth_pane, Some("bluetooth"));

    let nav_dispatch: Rc<RefCell<Option<Rc<dyn Fn(&str, bool)>>>> = Rc::new(RefCell::new(None));
    let nav_wifi = {
        let d = nav_dispatch.clone();
        Rc::new(move || {
            if let Some(f) = d.borrow().as_ref() {
                f("wifi", true);
            }
        })
    };
    let nav_fw = {
        let d = nav_dispatch.clone();
        Rc::new(move || {
            if let Some(f) = d.borrow().as_ref() {
                f("network-firewall", true);
            }
        })
    };
    let ((network_pane, refresh_network_pane), (firewall_pane, refresh_firewall_pane)) =
        network_pane::create_network_panes(&window, nav_wifi, nav_fw);
    stack.add_named(&network_pane, Some("network"));
    stack.add_named(&firewall_pane, Some("network-firewall"));

    let (vpn_pane, refresh_vpn_pane) = vpn_pane::create_vpn_pane(&window);
    stack.add_named(&vpn_pane, Some("vpn"));

    let header_title_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let active_pane_title = Label::builder()
        .label("Spotlight")
        .css_classes(["content-header-title"])
        .xalign(0.0)
        .build();
    let header_sub_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(5)
        .visible(false)
        .build();
    let header_sub_icon = Image::builder()
        .icon_name("battery-level-100-symbolic")
        .pixel_size(12)
        .css_classes(["bat-header-sub-icon"])
        .build();
    let header_sub_label = Label::builder()
        .label("")
        .css_classes(["bat-header-sub-label"])
        .xalign(0.0)
        .build();
    header_sub_row.append(&header_sub_icon);
    header_sub_row.append(&header_sub_label);
    header_title_col.append(&active_pane_title);
    header_title_col.append(&header_sub_row);

    let battery_header_cb: battery_pane::BatteryHeaderCallback = {
        let icon_c = header_sub_icon.clone();
        let lbl_c = header_sub_label.clone();
        Rc::new(move |icon_name: &str, text: &str| {
            icon_c.set_icon_name(Some(icon_name));
            lbl_c.set_label(text);
        })
    };
    let (battery_pane, refresh_battery_pane) =
        battery_pane::create_battery_pane(&window, battery_header_cb);
    stack.add_named(&battery_pane, Some("battery"));

    let (displays_pane, refresh_displays_pane) = create_displays_pane();
    stack.add_named(&displays_pane, Some("displays"));

    let (wallpaper_pane, refresh_wallpaper_pane) = create_wallpaper_pane(&window);
    stack.add_named(&wallpaper_pane, Some("wallpaper"));

    let (keyboard_pane, refresh_keyboard_pane) = create_keyboard_pane();
    stack.add_named(&keyboard_pane, Some("keyboard"));

    let (dock_pane, refresh_dock_pane) = create_dock_pane();
    stack.add_named(&dock_pane, Some("dock"));

    let (spotlight_pane, refresh_spotlight_pane) = create_spotlight_pane();
    stack.add_named(&spotlight_pane, Some("spotlight"));

    let sound_pane = create_sound_pane();
    stack.add_named(&sound_pane, Some("sound"));

    let about_pane = create_about_pane();
    stack.add_named(&about_pane, Some("about"));

    // Header Toolbar macOS (‹ › chevrons + título de la vista actual)
    let content_area = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["settings-content-pane"])
        .hexpand(true)
        .vexpand(true)
        .build();

    let header_toolbar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["content-header-toolbar"])
        .spacing(12)
        .valign(Align::Center)
        .build();

    // 1. Botones de navegación [ < | > ] agrupados en cápsula estilo Branches
    let nav_btn_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .css_classes(["branches-toolbar-pill", "branches-nav-group", "linked"])
        .valign(Align::Center)
        .build();

    let btn_back = Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Atrás (Alt+Izquierda)")
        .css_classes(["branches-nav-btn"])
        .has_frame(false)
        .sensitive(false)
        .valign(Align::Center)
        .build();

    let nav_divider = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["branches-pill-divider"])
        .valign(Align::Center)
        .build();

    let btn_forward = Button::builder()
        .icon_name("go-next-symbolic")
        .tooltip_text("Adelante (Alt+Derecha)")
        .css_classes(["branches-nav-btn"])
        .has_frame(false)
        .sensitive(false)
        .valign(Align::Center)
        .build();

    nav_btn_box.append(&btn_back);
    nav_btn_box.append(&nav_divider);
    nav_btn_box.append(&btn_forward);
    header_toolbar.append(&nav_btn_box);

    header_toolbar.append(&header_title_col);

    let window_controls_end = WindowControls::builder()
        .side(PackType::End)
        .valign(Align::Center)
        .build();
    header_toolbar.append(&window_controls_end);

    let header_handle = WindowHandle::builder()
        .child(&header_toolbar)
        .hexpand(true)
        .build();
    content_area.append(&header_handle);
    content_area.append(&stack);

    let goto_action: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let goto_action_c = goto_action.clone();
    let goto_wallpaper = Rc::new(move || {
        if let Some(act) = goto_action_c.borrow().as_ref() {
            act();
        }
    });

    let (appearance_pane, refresh_appearance_pane) = create_appearance_pane(&window, Some(goto_wallpaper));
    stack.add_named(&appearance_pane, Some("appearance"));

    // Historial de navegación para botones atrás / adelante (estilo Branches)
    let start_id = match initial_panel {
        "red" => "network",
        "firewall" => "network-firewall",
        "bateria" | "batería" => "battery",
        "wifi" | "bluetooth" | "network" | "network-firewall" | "vpn" | "battery"
        | "wallpaper" | "keyboard" | "appearance" | "sound" | "about" | "dock" | "spotlight"
        | "displays" => initial_panel,
        _ => "spotlight",
    };

    let all_btns = Rc::new(sidebar_buttons.clone());
    let nav_history: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![start_id.to_string()]));
    let nav_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));

    let select_panel = {
        let btns = all_btns.clone();
        let stack_c = stack.clone();
        let title_lbl = active_pane_title.clone();
        let sub_row_c = header_sub_row.clone();
        let ref_wifi = refresh_wifi_pane.clone();
        let ref_bt = refresh_bluetooth_pane.clone();
        let ref_net = refresh_network_pane.clone();
        let ref_fw = refresh_firewall_pane.clone();
        let ref_vpn = refresh_vpn_pane.clone();
        let ref_bat = refresh_battery_pane.clone();
        let ref_disp = refresh_displays_pane.clone();
        let ref_wall = refresh_wallpaper_pane.clone();
        let ref_kb = refresh_keyboard_pane.clone();
        let ref_app = refresh_appearance_pane.clone();
        let ref_dock = refresh_dock_pane.clone();
        let ref_spot = refresh_spotlight_pane.clone();
        let history_c = nav_history.clone();
        let idx_c = nav_index.clone();
        let btn_b_c = btn_back.clone();
        let btn_f_c = btn_forward.clone();

        Rc::new(move |target: &str, push_history: bool| {
            let sidebar_id = if target == "network-firewall" {
                "network"
            } else {
                target
            };
            for (id, btn) in btns.iter() {
                if id == sidebar_id {
                    btn.add_css_class("sidebar-btn-active");
                } else {
                    btn.remove_css_class("sidebar-btn-active");
                }
            }
            sub_row_c.set_visible(target == "battery");
            if target == "wifi" {
                ref_wifi();
            } else if target == "bluetooth" {
                ref_bt();
            } else if target == "network" {
                ref_net();
            } else if target == "network-firewall" {
                ref_fw();
            } else if target == "vpn" {
                ref_vpn();
            } else if target == "battery" {
                ref_bat();
            } else if target == "displays" {
                ref_disp();
            } else if target == "wallpaper" {
                ref_wall();
            } else if target == "keyboard" {
                ref_kb();
            } else if target == "appearance" {
                ref_app();
            } else if target == "dock" {
                ref_dock();
            } else if target == "spotlight" {
                ref_spot();
            }
            let display_title = match target {
                "wifi" => "Wi-Fi",
                "bluetooth" => "Bluetooth",
                "network" => "Red",
                "network-firewall" => "Firewall",
                "vpn" => "VPN",
                "battery" => "Batería",
                "displays" => "Pantallas",
                "wallpaper" => "Fondo de pantalla",
                "keyboard" => "Teclado",
                "appearance" => "Aspecto",
                "dock" => "Escritorio y Dock",
                "spotlight" => "Spotlight",
                "sound" => "Sonido",
                "about" => "General",
                _ => "Ajustes",
            };
            title_lbl.set_label(display_title);
            stack_c.set_visible_child_name(target);

            if push_history {
                let mut hist = history_c.borrow_mut();
                let mut cur = idx_c.borrow_mut();
                if hist.get(*cur).map(|s| s.as_str()) != Some(target) {
                    hist.truncate(*cur + 1);
                    hist.push(target.to_string());
                    *cur = hist.len() - 1;
                }
            }
            let cur = *idx_c.borrow();
            let total = history_c.borrow().len();
            btn_b_c.set_sensitive(cur > 0);
            btn_f_c.set_sensitive(cur + 1 < total);
        })
    };

    let sel_for_dispatch = select_panel.clone();
    *nav_dispatch.borrow_mut() = Some(Rc::new(move |target: &str, push: bool| {
        sel_for_dispatch(target, push);
    }));

    let sel_for_goto = select_panel.clone();
    *goto_action.borrow_mut() = Some(Rc::new(move || {
        sel_for_goto("wallpaper", true);
    }));

    let sel_back = select_panel.clone();
    let hist_b = nav_history.clone();
    let idx_b = nav_index.clone();
    btn_back.connect_clicked(move |_| {
        let mut cur = idx_b.borrow_mut();
        if *cur > 0 {
            *cur -= 1;
            let target = hist_b.borrow()[*cur].clone();
            drop(cur);
            sel_back(&target, false);
        }
    });

    let sel_fwd = select_panel.clone();
    let hist_f = nav_history.clone();
    let idx_f = nav_index.clone();
    btn_forward.connect_clicked(move |_| {
        let mut cur = idx_f.borrow_mut();
        let total = hist_f.borrow().len();
        if *cur + 1 < total {
            *cur += 1;
            let target = hist_f.borrow()[*cur].clone();
            drop(cur);
            sel_fwd(&target, false);
        }
    });

    // Conectar navegación de la barra lateral
    for (id, btn) in &sidebar_buttons {
        let target_id = id.clone();
        let sel = select_panel.clone();
        btn.connect_clicked(move |_| {
            sel(&target_id, true);
        });
    }

    // Seleccionar panel inicial
    select_panel(start_id, false);

    let main_paned = Paned::builder()
        .orientation(Orientation::Horizontal)
        .position(SETTINGS_SIDEBAR_WIDTH)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["settings-main-paned"])
        .build();
    main_paned.connect_position_notify(|paned| {
        if paned.position() > SETTINGS_SIDEBAR_WIDTH {
            paned.set_position(SETTINGS_SIDEBAR_WIDTH);
        }
    });
    main_paned.set_start_child(Some(&sidebar));
    main_paned.set_end_child(Some(&content_area));

    let root_overlay = Overlay::new();
    root_overlay.set_child(Some(&main_paned));
    let modal_scrim = GtkBox::builder()
        .name("settings-modal-scrim")
        .css_classes(["settings-modal-scrim"])
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .visible(false)
        .build();
    root_overlay.add_overlay(&modal_scrim);

    window.set_child(Some(&root_overlay));
    window.present();
}

pub(crate) fn set_modal_scrim_visible(window: &ApplicationWindow, visible: bool) {
    let Some(overlay) = window.child().and_then(|c| c.downcast::<Overlay>().ok()) else {
        return;
    };
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == "settings-modal-scrim" {
            widget.set_visible(visible);
            break;
        }
        child = widget.next_sibling();
    }
}

pub(crate) fn attach_modal_scrim(dialog: &gtk4::Window, parent: &ApplicationWindow) {
    set_modal_scrim_visible(parent, true);
    let p1 = parent.clone();
    dialog.connect_close_request(move |_| {
        set_modal_scrim_visible(&p1, false);
        glib::Propagation::Proceed
    });
    let p2 = parent.clone();
    dialog.connect_destroy(move |_| {
        set_modal_scrim_visible(&p2, false);
    });
}

// ============================================================================
// PANEL 1: PANTALLAS (macOS System Settings > Displays)
// ============================================================================

#[derive(Clone, Debug)]
struct CanvasInteractionState {
    selected_output_idx: usize,
    dragging_idx: Option<usize>,
    drag_offset_x: f64,
    drag_offset_y: f64,
    monitor_rects: Vec<(usize, f64, f64, f64, f64)>,
}

fn create_displays_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["content-pane"])
        .spacing(16)
        .build();

    // Cabecera de Pantallas
    let header_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    let title = Label::builder()
        .label("Pantallas")
        .css_classes(["pane-header-title"])
        .xalign(0.0)
        .build();
    let subtitle = Label::builder()
        .label("Arrastra los monitores en el lienzo para organizar su disposición física, ajusta la escala de cada pantalla o enlázalas 1:1 pixel a pixel por PPI.")
        .css_classes(["pane-header-subtitle"])
        .wrap(true)
        .xalign(0.0)
        .build();
    header_box.append(&title);
    header_box.append(&subtitle);
    pane.append(&header_box);

    // ------------------------------------------------------------------------
    // SECCIÓN A: Lienzo Interactivo de Disposición de Pantallas (macOS Arrange)
    // ------------------------------------------------------------------------
    let canvas_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-canvas-card"])
        .spacing(10)
        .build();

    let canvas_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let canvas_title = Label::builder()
        .label("Disposición Interactiva de Pantallas (Arrastra con el ratón para mover o intercambiar)")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let canvas_coords_lbl = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .build();
    canvas_header.append(&canvas_title);
    canvas_header.append(&canvas_coords_lbl);
    canvas_card.append(&canvas_header);

    let drawing_area = DrawingArea::builder()
        .content_height(225)
        .hexpand(true)
        .build();
    canvas_card.append(&drawing_area);

    // Barra de herramientas de disposición debajo del lienzo
    let arrange_toolbar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let btn_swap = Button::builder()
        .label("Invertir Izquierda / Derecha")
        .css_classes(["macos-action-btn-primary"])
        .tooltip_text("Intercambia la posición horizontal de ambos monitores")
        .has_frame(false)
        .build();

    let btn_align_top = Button::builder()
        .label("Alinear Arriba")
        .css_classes(["macos-action-btn"])
        .tooltip_text("Alinea los bordes superiores (y = 0)")
        .has_frame(false)
        .build();

    let btn_align_center = Button::builder()
        .label("Centrar")
        .css_classes(["macos-action-btn"])
        .tooltip_text("Alinea los centros verticales de ambas pantallas")
        .has_frame(false)
        .build();

    let btn_align_bottom = Button::builder()
        .label("Alinear Abajo")
        .css_classes(["macos-action-btn"])
        .tooltip_text("Alinea los bordes inferiores")
        .has_frame(false)
        .build();

    let btn_align_desk = Button::builder()
        .label("Escritorio (y=519)")
        .css_classes(["macos-action-btn"])
        .tooltip_text("Alineación ergonómica de escritorio (24\" desplazado a y=519)")
        .has_frame(false)
        .build();

    arrange_toolbar.append(&btn_swap);
    arrange_toolbar.append(&btn_align_top);
    arrange_toolbar.append(&btn_align_center);
    arrange_toolbar.append(&btn_align_bottom);
    arrange_toolbar.append(&btn_align_desk);
    canvas_card.append(&arrange_toolbar);

    pane.append(&canvas_card);

    // ------------------------------------------------------------------------
    // SECCIÓN B: Botón Maestro de Enlace 1:1 Pixel a Pixel (TrueSize PPI Link)
    // ------------------------------------------------------------------------
    let truesize_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["truesize-banner-active"])
        .spacing(10)
        .build();

    let truesize_top_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();

    let truesize_text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .spacing(3)
        .build();

    let truesize_title = Label::builder()
        .label("Enlazar Resolución Física 1:1 Pixel a Pixel (TrueSize PPI)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let truesize_sub = Label::builder()
        .label("Sincroniza la escala de las pantallas conectadas según su densidad de píxeles para conservar un tamaño físico coherente al mover ventanas entre monitores.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    truesize_text_col.append(&truesize_title);
    truesize_text_col.append(&truesize_sub);

    let truesize_toggle_btn = Button::builder()
        .label("Enlazadas 1:1")
        .css_classes(["macos-action-btn-primary"])
        .valign(Align::Center)
        .has_frame(false)
        .build();

    truesize_top_row.append(&truesize_text_col);
    truesize_top_row.append(&truesize_toggle_btn);

    // Presets de escala enlazada
    let truesize_presets_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .homogeneous(true)
        .build();

    let ts_preset1_btn = Button::builder()
        .css_classes(["truesize-preset-btn"])
        .has_frame(false)
        .build();
    let ts_p1_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(10)
        .margin_end(10)
        .build();
    let ts_p1_title = Label::builder()
        .label("Ajuste 1 · Equilibrado")
        .css_classes(["card-section-title"])
        .xalign(0.5)
        .build();
    let ts_p1_sub = Label::builder()
        .label("Escala base de 150% en la pantalla principal")
        .css_classes(["card-section-sub"])
        .xalign(0.5)
        .build();
    ts_p1_box.append(&ts_p1_title);
    ts_p1_box.append(&ts_p1_sub);
    ts_preset1_btn.set_child(Some(&ts_p1_box));

    let ts_preset2_btn = Button::builder()
        .css_classes(["truesize-preset-btn"])
        .has_frame(false)
        .build();
    let ts_p2_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(10)
        .margin_end(10)
        .build();
    let ts_p2_title = Label::builder()
        .label("Ajuste 2 · Más espacio")
        .css_classes(["card-section-title"])
        .xalign(0.5)
        .build();
    let ts_p2_sub = Label::builder()
        .label("Escala base de 133% en la pantalla principal")
        .css_classes(["card-section-sub"])
        .xalign(0.5)
        .build();
    ts_p2_box.append(&ts_p2_title);
    ts_p2_box.append(&ts_p2_sub);
    ts_preset2_btn.set_child(Some(&ts_p2_box));

    truesize_presets_row.append(&ts_preset1_btn);
    truesize_presets_row.append(&ts_preset2_btn);

    let truesize_math_lbl = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();

    truesize_card.append(&truesize_top_row);
    truesize_card.append(&truesize_presets_row);
    truesize_card.append(&truesize_math_lbl);
    pane.append(&truesize_card);

    // ------------------------------------------------------------------------
    // SECCIÓN C: Tarjetas dinámicas para las pantallas detectadas por DRM
    // ------------------------------------------------------------------------
    let monitors_grid = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .build();
    let monitor_widgets: Rc<RefCell<Vec<MonitorCardWidgets>>> = Rc::new(RefCell::new(Vec::new()));
    pane.append(&monitors_grid);

    // ------------------------------------------------------------------------
    // SECCIÓN D: Curvatura de Esquinas (Border-Radius de los Escritorios)
    // ------------------------------------------------------------------------
    let radius_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let radius_top_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();

    let radius_text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let radius_title = Label::builder()
        .label("Curvatura de Esquinas del Escritorio (Squircle Display Border-Radius)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let radius_sub = Label::builder()
        .label("Simula el bisel redondeado superelíptico orgánico (squircle n=4.2) en las 4 esquinas de los escritorios y pantallas.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    radius_text_col.append(&radius_title);
    radius_text_col.append(&radius_sub);

    let radius_val_lbl = Label::builder()
        .label("24 px")
        .css_classes(["badge-blue", "sidebar-icon-badge"])
        .build();

    radius_top_row.append(&radius_text_col);
    radius_top_row.append(&radius_val_lbl);
    radius_card.append(&radius_top_row);

    let radius_controls_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();

    let radius_slider = Scale::with_range(Orientation::Horizontal, 0.0, 48.0, 1.0);
    radius_slider.set_hexpand(true);
    radius_slider.set_draw_value(false);

    let preset_0 = Button::builder().label("0 px (Recto)").css_classes(["macos-action-btn"]).has_frame(false).build();
    let preset_16 = Button::builder().label("16 px").css_classes(["macos-action-btn"]).has_frame(false).build();
    let preset_20 = Button::builder().label("20 px").css_classes(["macos-action-btn"]).has_frame(false).build();
    let preset_24 = Button::builder().label("24 px (Recomendado)").css_classes(["macos-action-btn-primary"]).has_frame(false).build();
    let preset_32 = Button::builder().label("32 px").css_classes(["macos-action-btn"]).has_frame(false).build();

    radius_controls_row.append(&radius_slider);
    radius_controls_row.append(&preset_0);
    radius_controls_row.append(&preset_16);
    radius_controls_row.append(&preset_20);
    radius_controls_row.append(&preset_24);
    radius_controls_row.append(&preset_32);
    radius_card.append(&radius_controls_row);

    pane.append(&radius_card);

    // Tarjeta de Radio de Curvatura de Ventanas
    let win_radius_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let win_radius_top_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();

    let win_radius_text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let win_radius_title = Label::builder()
        .label("Curvatura de Esquinas de Ventanas (Squircle Window Border-Radius)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let win_radius_sub = Label::builder()
        .label("Aplica el bisel redondeado superelíptico orgánico (squircle n=4.2) en las 4 esquinas de las ventanas de aplicaciones.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    win_radius_text_col.append(&win_radius_title);
    win_radius_text_col.append(&win_radius_sub);

    let win_radius_val_lbl = Label::builder()
        .label("32 px")
        .css_classes(["badge-blue", "sidebar-icon-badge"])
        .build();

    win_radius_top_row.append(&win_radius_text_col);
    win_radius_top_row.append(&win_radius_val_lbl);
    win_radius_card.append(&win_radius_top_row);

    let win_radius_controls_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();

    let win_radius_slider = Scale::with_range(Orientation::Horizontal, 0.0, 48.0, 1.0);
    win_radius_slider.set_hexpand(true);
    win_radius_slider.set_draw_value(false);

    let win_preset_0 = Button::builder().label("0 px (Recto)").css_classes(["macos-action-btn"]).has_frame(false).build();
    let win_preset_16 = Button::builder().label("16 px").css_classes(["macos-action-btn"]).has_frame(false).build();
    let win_preset_24 = Button::builder().label("24 px").css_classes(["macos-action-btn"]).has_frame(false).build();
    let win_preset_32 = Button::builder().label("32 px (Recomendado)").css_classes(["macos-action-btn-primary"]).has_frame(false).build();
    let win_preset_40 = Button::builder().label("40 px").css_classes(["macos-action-btn"]).has_frame(false).build();

    win_radius_controls_row.append(&win_radius_slider);
    win_radius_controls_row.append(&win_preset_0);
    win_radius_controls_row.append(&win_preset_16);
    win_radius_controls_row.append(&win_preset_24);
    win_radius_controls_row.append(&win_preset_32);
    win_radius_controls_row.append(&win_preset_40);
    win_radius_card.append(&win_radius_controls_row);

    pane.append(&win_radius_card);

    scrolled.set_child(Some(&pane));

    // ==========================================
    // Lógica de Dibujo y Sincronización en Vivo
    // ==========================================
    let canvas_state = Rc::new(RefCell::new(CanvasInteractionState {
        selected_output_idx: 0,
        dragging_idx: None,
        drag_offset_x: 0.0,
        drag_offset_y: 0.0,
        monitor_rects: Vec::new(),
    }));

    let cs_draw = canvas_state.clone();
    drawing_area.set_draw_func(move |_area, cr, width, height| {
        let cfg = DisplaysConfig::load();
        let mut st = cs_draw.borrow_mut();
        st.monitor_rects.clear();

        // Fondo del lienzo de disposición con cuadrícula sutil (adaptativo claro/oscuro)
        let is_dark = is_dark_mode_active();
        if is_dark {
            cr.set_source_rgb(0.078, 0.078, 0.088);
        } else {
            cr.set_source_rgb(0.91, 0.91, 0.93);
        }
        let _ = cr.paint();

        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.04);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.05);
        }
        cr.set_line_width(1.0);
        let mut gx = 0.0;
        while gx < width as f64 {
            cr.move_to(gx, 0.0);
            cr.line_to(gx, height as f64);
            gx += 32.0;
        }
        let mut gy = 0.0;
        while gy < height as f64 {
            cr.move_to(0.0, gy);
            cr.line_to(width as f64, gy);
            gy += 32.0;
        }
        let _ = cr.stroke();

        if cfg.outputs.is_empty() {
            return;
        }

        // Calcular bounding box total del espacio lógico para escalar dentro del lienzo
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for o in &cfg.outputs {
            let (lw, lh) = o.logical_size();
            min_x = min_x.min(o.x);
            min_y = min_y.min(o.y);
            max_x = max_x.max(o.x + lw);
            max_y = max_y.max(o.y + lh);
        }

        let total_w = (max_x - min_x).max(1000) as f64;
        let total_h = (max_y - min_y).max(800) as f64;

        let pad = 34.0;
        let avail_w = (width as f64 - pad * 2.0).max(200.0);
        let avail_h = (height as f64 - pad * 2.0).max(120.0);
        let zoom = (avail_w / total_w).min(avail_h / total_h);

        let origin_x = (width as f64 - total_w * zoom) / 2.0;
        let origin_y = (height as f64 - total_h * zoom) / 2.0;

        for (idx, o) in cfg.outputs.iter().enumerate() {
            let (lw, lh) = o.logical_size();
            let mut rx = origin_x + (o.x - min_x) as f64 * zoom;
            let mut ry = origin_y + (o.y - min_y) as f64 * zoom;
            let rw = lw as f64 * zoom;
            let rh = lh as f64 * zoom;

            if st.dragging_idx == Some(idx) {
                rx += st.drag_offset_x;
                ry += st.drag_offset_y;
            }

            st.monitor_rects.push((idx, rx, ry, rw, rh));

            // Dibujar rectángulo redondeado del monitor (estilo macOS Sonoma)
            let radius = 8.0;
            cr.new_sub_path();
            cr.arc(rx + rw - radius, ry + radius, radius, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(rx + rw - radius, ry + rh - radius, radius, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(rx + radius, ry + rh - radius, radius, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(rx + radius, ry + radius, radius, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();

            if idx == 0 {
                cr.set_source_rgb(0.12, 0.28, 0.52);
            } else {
                cr.set_source_rgb(0.18, 0.22, 0.44);
            }
            let _ = cr.fill_preserve();

            if is_dark {
                if st.selected_output_idx == idx || st.dragging_idx == Some(idx) {
                    cr.set_source_rgb(0.04, 0.52, 1.0);
                    cr.set_line_width(2.5);
                } else {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
                    cr.set_line_width(1.5);
                }
            } else {
                if st.selected_output_idx == idx || st.dragging_idx == Some(idx) {
                    cr.set_source_rgb(0.0, 0.48, 1.0);
                    cr.set_line_width(2.5);
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.22);
                    cr.set_line_width(1.5);
                }
            }
            let _ = cr.stroke();

            // Si es el monitor primario, dibujar la barra de menús blanca superior estilo macOS
            if o.primary {
                cr.rectangle(rx + 2.0, ry + 2.0, rw - 4.0, 7.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.88);
                let _ = cr.fill();
            }

            // Etiquetas dentro del monitor
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_font_size(12.5);
            let title_str = format!("{} {} ({})", idx + 1, o.name, o.connector);
            cr.move_to(rx + 12.0, ry + 28.0);
            let _ = cr.show_text(&title_str);

            cr.set_source_rgba(0.90, 0.90, 0.95, 0.90);
            cr.set_font_size(11.0);
            let res_str = format!("{} × {} ({:.1}%)", lw, lh, o.scale * 100.0);
            cr.move_to(rx + 12.0, ry + 45.0);
            let _ = cr.show_text(&res_str);

            cr.set_source_rgba(0.65, 0.82, 1.0, 0.90);
            cr.set_font_size(10.5);
            let ppi_str = o
                .ppi()
                .map(|p| format!("{p:.1} PPI"))
                .unwrap_or_else(|| "PPI desconocido".to_string());
            let pos_str = format!(
                "pos=({}, {}) · {}{}",
                o.x,
                o.y,
                ppi_str,
                if o.primary { " · Principal" } else { "" }
            );
            cr.move_to(rx + 12.0, ry + 61.0);
            let _ = cr.show_text(&pos_str);
        }
    });

    let is_syncing = Rc::new(RefCell::new(false));
    let refresh_all_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let da_ref = drawing_area.clone();
    let coords_ref = canvas_coords_lbl.clone();
    let ts_card_ref = truesize_card.clone();
    let ts_btn_ref = truesize_toggle_btn.clone();
    let ts_p1_ref = ts_preset1_btn.clone();
    let ts_p2_ref = ts_preset2_btn.clone();
    let ts_math_ref = truesize_math_lbl.clone();
    let monitors_grid_ref = monitors_grid.clone();
    let monitor_widgets_ref = monitor_widgets.clone();
    let refresh_slot_ref = refresh_all_slot.clone();
    let rad_lbl_ref = radius_val_lbl.clone();
    let rad_sld_ref = radius_slider.clone();
    let win_rad_lbl_ref = win_radius_val_lbl.clone();
    let win_rad_sld_ref = win_radius_slider.clone();
    let sync_lock = is_syncing.clone();

    let refresh_all: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = DisplaysConfig::load();
        *sync_lock.borrow_mut() = true;

        // Actualizar texto de coordenadas en el lienzo
        let mut sorted: Vec<_> = cfg.outputs.iter().collect();
        sorted.sort_by_key(|o| o.x);
        let summary: Vec<String> = sorted
            .iter()
            .map(|o| {
                let (lw, lh) = o.logical_size();
                format!("{} [{}×{} @({}, {})]", o.connector, lw, lh, o.x, o.y)
            })
            .collect();
        coords_ref.set_label(&summary.join("  |  "));

        // Actualizar banner de Enlace 1:1 TrueSize
        if cfg.linked_physical_1to1 {
            ts_card_ref.remove_css_class("truesize-banner-inactive");
            ts_card_ref.add_css_class("truesize-banner-active");
            ts_btn_ref.set_label("Enlazadas 1:1 (Activo)");
            ts_btn_ref.add_css_class("macos-action-btn-primary");
            ts_btn_ref.remove_css_class("macos-action-btn");
        } else {
            ts_card_ref.remove_css_class("truesize-banner-active");
            ts_card_ref.add_css_class("truesize-banner-inactive");
            ts_btn_ref.set_label("Escala Independiente (Enlazar 1:1)");
            ts_btn_ref.remove_css_class("macos-action-btn-primary");
            ts_btn_ref.add_css_class("macos-action-btn");
        }
        ts_card_ref.set_visible(cfg.outputs.len() > 1);

        // Actualizar estado de los botones de Ajuste 1 y Ajuste 2
        let curr_preset = cfg.current_linked_preset();
        if curr_preset == Some(1) {
            ts_p1_ref.add_css_class("truesize-preset-btn-active");
            ts_p1_ref.remove_css_class("truesize-preset-btn");
            ts_p2_ref.remove_css_class("truesize-preset-btn-active");
            ts_p2_ref.add_css_class("truesize-preset-btn");
        } else if curr_preset == Some(2) {
            ts_p2_ref.add_css_class("truesize-preset-btn-active");
            ts_p2_ref.remove_css_class("truesize-preset-btn");
            ts_p1_ref.remove_css_class("truesize-preset-btn-active");
            ts_p1_ref.add_css_class("truesize-preset-btn");
        } else {
            ts_p1_ref.remove_css_class("truesize-preset-btn-active");
            ts_p1_ref.add_css_class("truesize-preset-btn");
            ts_p2_ref.remove_css_class("truesize-preset-btn-active");
            ts_p2_ref.add_css_class("truesize-preset-btn");
        }

        if cfg.outputs.len() >= 2 {
            let o0 = &cfg.outputs[0];
            let o1 = &cfg.outputs[1];
            if let (Some(ppi0), Some(ppi1)) = (o0.ppi(), o1.ppi()) {
                let mm0 = (100.0 * o0.scale / ppi0) * 25.4;
                let mm1 = (100.0 * o1.scale / ppi1) * 25.4;
                ts_math_ref.set_label(&format!(
                    "Regla Física Actual: 100 pt lógicos = {:.1} mm en {} ({:.1}%)  |  {:.1} mm en {} ({:.1}%)",
                    mm0,
                    o0.connector,
                    o0.scale * 100.0,
                    mm1,
                    o1.connector,
                    o1.scale * 100.0
                ));
            } else {
                ts_math_ref.set_label(
                    "Regla Física Actual: falta el tamaño físico real (EDID/pulgadas) en al menos una pantalla para enlazar 1:1",
                );
            }
        }

        let needs_rebuild = {
            let widgets = monitor_widgets_ref.borrow();
            monitors_grid_ref.first_child().is_none()
                || widgets.len() != cfg.outputs.len()
                || widgets
                    .iter()
                    .zip(cfg.outputs.iter())
                    .any(|(w, o)| w.connector != o.connector)
        };
        if needs_rebuild {
            if let Some(ref refresh_fn) = *refresh_slot_ref.borrow() {
                rebuild_monitors_grid(
                    &monitors_grid_ref,
                    &monitor_widgets_ref,
                    &cfg,
                    &sync_lock,
                    refresh_fn,
                );
            }
        }

        for (idx, widgets) in monitor_widgets_ref.borrow().iter().enumerate() {
            if let Some(output) = cfg.outputs.get(idx) {
                widgets.card_box.set_visible(true);
                update_monitor_card_ui(widgets, output);
            } else {
                widgets.card_box.set_visible(false);
            }
        }

        // Actualizar Curvatura de Esquinas (Border-Radius Pantallas)
        rad_lbl_ref.set_label(&format!("{:.0} px", cfg.screen_corner_radius));
        rad_sld_ref.set_value(cfg.screen_corner_radius);

        // Actualizar Curvatura de Esquinas (Border-Radius Ventanas)
        win_rad_lbl_ref.set_label(&format!("{:.0} px", cfg.window_corner_radius));
        win_rad_sld_ref.set_value(cfg.window_corner_radius);

        da_ref.queue_draw();
        *sync_lock.borrow_mut() = false;
    });

    *refresh_all_slot.borrow_mut() = Some(refresh_all.clone());
    refresh_all();

    if let Some(display) = gtk4::gdk::Display::default() {
        let ref_mon = refresh_all.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            let ref_later = ref_mon.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                ref_later();
            });
        });
    }
    if let Ok(home) = std::env::var("HOME") {
        let toml_path = std::path::PathBuf::from(home).join(".config/malus-de/monitors.toml");
        if let Ok(file_mon) = gio::File::for_path(toml_path)
            .monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let ref_file = refresh_all.clone();
            file_mon.connect_changed(move |_, _, _, ev| {
                if matches!(
                    ev,
                    gio::FileMonitorEvent::Changed
                        | gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::ChangesDoneHint
                ) {
                    ref_file();
                }
            });
            Box::leak(Box::new(file_mon));
        }
    }

    let save_and_reload = |cfg: DisplaysConfig, ref_fn: &Rc<dyn Fn()>| {
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig);
        ref_fn();
    };

    // Eventos de controles de Curvatura de Esquinas (Border-Radius Pantallas)
    let ref_r0 = refresh_all.clone();
    preset_0.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.screen_corner_radius = 0.0;
        save_and_reload(cfg, &ref_r0);
    });

    let ref_r16 = refresh_all.clone();
    preset_16.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.screen_corner_radius = 16.0;
        save_and_reload(cfg, &ref_r16);
    });

    let ref_r20 = refresh_all.clone();
    preset_20.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.screen_corner_radius = 20.0;
        save_and_reload(cfg, &ref_r20);
    });

    let ref_r24 = refresh_all.clone();
    preset_24.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.screen_corner_radius = 24.0;
        save_and_reload(cfg, &ref_r24);
    });

    let ref_r32 = refresh_all.clone();
    preset_32.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.screen_corner_radius = 32.0;
        save_and_reload(cfg, &ref_r32);
    });

    let sync_rad = is_syncing.clone();
    let ref_rad_sld = refresh_all.clone();
    radius_slider.connect_value_changed(move |sld| {
        if *sync_rad.borrow() {
            return;
        }
        let val = sld.value().round();
        let mut cfg = DisplaysConfig::load();
        cfg.screen_corner_radius = val;
        save_and_reload(cfg, &ref_rad_sld);
    });

    // Eventos de controles de Curvatura de Esquinas (Border-Radius Ventanas)
    let ref_w0 = refresh_all.clone();
    win_preset_0.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.window_corner_radius = 0.0;
        save_and_reload(cfg, &ref_w0);
    });

    let ref_w16 = refresh_all.clone();
    win_preset_16.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.window_corner_radius = 16.0;
        save_and_reload(cfg, &ref_w16);
    });

    let ref_w24 = refresh_all.clone();
    win_preset_24.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.window_corner_radius = 24.0;
        save_and_reload(cfg, &ref_w24);
    });

    let ref_w32 = refresh_all.clone();
    win_preset_32.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.window_corner_radius = 32.0;
        save_and_reload(cfg, &ref_w32);
    });

    let ref_w40 = refresh_all.clone();
    win_preset_40.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.window_corner_radius = 40.0;
        save_and_reload(cfg, &ref_w40);
    });

    let sync_win_rad = is_syncing.clone();
    let ref_win_rad_sld = refresh_all.clone();
    win_radius_slider.connect_value_changed(move |sld| {
        if *sync_win_rad.borrow() {
            return;
        }
        let val = sld.value().round();
        let mut cfg = DisplaysConfig::load();
        cfg.window_corner_radius = val;
        save_and_reload(cfg, &ref_win_rad_sld);
    });

    // Gestos de Clic y Arrastre en el Lienzo Interactivo
    let click_gesture = GestureClick::new();
    let cs_click = canvas_state.clone();
    let da_click = drawing_area.clone();
    click_gesture.connect_pressed(move |_, _, x, y| {
        let mut st = cs_click.borrow_mut();
        if let Some(&(idx, _, _, _, _)) = st
            .monitor_rects
            .iter()
            .find(|&&(_, rx, ry, rw, rh)| x >= rx && x <= rx + rw && y >= ry && y <= ry + rh)
        {
            st.selected_output_idx = idx;
            da_click.queue_draw();
        }
    });
    drawing_area.add_controller(click_gesture);

    let drag_gesture = GestureDrag::new();
    let cs_drag_begin = canvas_state.clone();
    drag_gesture.connect_drag_begin(move |_, x, y| {
        let mut st = cs_drag_begin.borrow_mut();
        if let Some(&(idx, _, _, _, _)) = st
            .monitor_rects
            .iter()
            .find(|&&(_, rx, ry, rw, rh)| x >= rx && x <= rx + rw && y >= ry && y <= ry + rh)
        {
            st.selected_output_idx = idx;
            st.dragging_idx = Some(idx);
            st.drag_offset_x = 0.0;
            st.drag_offset_y = 0.0;
        }
    });

    let cs_drag_upd = canvas_state.clone();
    let da_drag_upd = drawing_area.clone();
    drag_gesture.connect_drag_update(move |_, offset_x, offset_y| {
        let mut st = cs_drag_upd.borrow_mut();
        if st.dragging_idx.is_some() {
            st.drag_offset_x = offset_x;
            st.drag_offset_y = offset_y;
            da_drag_upd.queue_draw();
        }
    });

    let cs_drag_end = canvas_state.clone();
    let ref_drag_end = refresh_all.clone();
    drag_gesture.connect_drag_end(move |_, offset_x, offset_y| {
        let mut st = cs_drag_end.borrow_mut();
        let Some(drag_idx) = st.dragging_idx.take() else { return };
        st.drag_offset_x = 0.0;
        st.drag_offset_y = 0.0;

        let mut cfg = DisplaysConfig::load();
        if cfg.outputs.len() >= 2 {
            let other_idx = if drag_idx == 0 { 1 } else { 0 };
            // Verificar si el usuario arrastró el monitor horizontalmente al otro lado del otro monitor
            let rect_drag = st.monitor_rects.iter().find(|r| r.0 == drag_idx).copied();
            let rect_other = st.monitor_rects.iter().find(|r| r.0 == other_idx).copied();
            drop(st);

            if let (Some((_, rx, _, rw, _)), Some((_, ox, _, ow, _))) = (rect_drag, rect_other) {
                let new_center_x = rx + offset_x + rw / 2.0;
                let other_center_x = ox + ow / 2.0;
                let was_left = cfg.outputs[drag_idx].x <= cfg.outputs[other_idx].x;
                let now_left = new_center_x < other_center_x;
                if was_left != now_left {
                    cfg.swap_horizontal_order();
                }
            }

            // Aplicar desplazamiento vertical proporcional (~8x de conversión lienzo -> espacio lógico)
            let logical_dy = (offset_y * 8.0).round() as i32;
            if logical_dy.abs() >= 15 {
                cfg.adjust_vertical_offset(drag_idx, logical_dy);
            }
            save_and_reload(cfg, &ref_drag_end);
        }
    });
    drawing_area.add_controller(drag_gesture);

    // Conectar botones de barra de herramientas de disposición
    let ref_swap = refresh_all.clone();
    btn_swap.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.swap_horizontal_order();
        save_and_reload(cfg, &ref_swap);
    });

    for (btn, mode) in [
        (&btn_align_top, "top"),
        (&btn_align_center, "center"),
        (&btn_align_bottom, "bottom"),
        (&btn_align_desk, "desk"),
    ] {
        let ref_align = refresh_all.clone();
        let m = mode.to_string();
        btn.connect_clicked(move |_| {
            let mut cfg = DisplaysConfig::load();
            cfg.set_vertical_alignment(&m);
            save_and_reload(cfg, &ref_align);
        });
    }

    // Conectar botones de Ajustes Predefinidos 1 y 2 de Enlace 1:1
    let ref_p1 = refresh_all.clone();
    ts_preset1_btn.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.apply_linked_preset(1);
        save_and_reload(cfg, &ref_p1);
    });

    let ref_p2 = refresh_all.clone();
    ts_preset2_btn.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.apply_linked_preset(2);
        save_and_reload(cfg, &ref_p2);
    });

    // Conectar botón de Enlace 1:1 Pixel a Pixel
    let ref_ts = refresh_all.clone();
    truesize_toggle_btn.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.linked_physical_1to1 = !cfg.linked_physical_1to1;
        if cfg.linked_physical_1to1 {
            if cfg.current_linked_preset().is_none() {
                cfg.apply_linked_preset(1);
            } else {
                cfg.apply_1to1_link();
            }
        } else {
            cfg.recalculate_layout();
        }
        save_and_reload(cfg, &ref_ts);
    });

    (scrolled, refresh_all)
}

#[derive(Clone)]
struct MonitorCardWidgets {
    connector: String,
    card_box: GtkBox,
    title_lbl: Label,
    primary_btn: Button,
    details_lbl: Label,
    active_info_lbl: Label,
    mode_dropdown: DropDown,
    modes: Vec<DisplayModeConfig>,
    scale_slider: Scale,
    scale_entry: Entry,
    apply_btn: Button,
    preset_buttons: Vec<(f64, Button)>,
}

fn create_macos_monitor_scaling_card(
    title_str: &str,
    hw_specs_str: &str,
    presets: &[(String, String, f64)],
    output: &malus_de_common::config::DisplayOutputConfig,
) -> (GtkBox, MonitorCardWidgets) {
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    // Cabecera del Monitor + Botón de Pantalla Principal
    let top_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let title_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .spacing(2)
        .build();

    let title_lbl = Label::builder()
        .label(title_str)
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let specs_lbl = Label::builder()
        .label(hw_specs_str)
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    title_col.append(&title_lbl);
    title_col.append(&specs_lbl);

    let primary_btn = Button::builder()
        .label("Pantalla Principal")
        .css_classes(["macos-action-btn"])
        .valign(Align::Center)
        .has_frame(false)
        .build();

    top_row.append(&title_col);
    top_row.append(&primary_btn);
    card.append(&top_row);

    let mode_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let mode_label = Label::builder()
        .label("Resolución disponible:")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    let modes = if output.modes.is_empty() {
        vec![DisplayModeConfig {
            width: output.width,
            height: output.height,
            refresh_rate: output.refresh_rate,
        }]
    } else {
        output.modes.clone()
    };
    let mode_labels: Vec<String> = modes
        .iter()
        .map(|mode| {
            format!(
                "{} × {} @ {:.2} Hz",
                mode.width, mode.height, mode.refresh_rate
            )
        })
        .collect();
    let mode_label_refs: Vec<&str> = mode_labels.iter().map(String::as_str).collect();
    let mode_dropdown = DropDown::from_strings(&mode_label_refs);
    mode_dropdown.set_hexpand(true);
    let mut selected_mode_idx = 0;
    for (idx, mode) in modes.iter().enumerate() {
        if mode.width == output.width
            && mode.height == output.height
            && (mode.refresh_rate - output.refresh_rate).abs() < 1.0
        {
            selected_mode_idx = idx;
        }
    }
    mode_dropdown.set_selected(selected_mode_idx as u32);
    mode_row.append(&mode_label);
    mode_row.append(&mode_dropdown);
    card.append(&mode_row);

    // Miniaturas estilo macOS: "Texto más grande" <-> "Más espacio"
    let scale_header_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .build();
    let larger_txt_lbl = Label::builder()
        .label("Texto más grande")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let more_space_lbl = Label::builder()
        .label("Más espacio")
        .css_classes(["card-section-sub"])
        .xalign(1.0)
        .build();
    scale_header_row.append(&larger_txt_lbl);
    scale_header_row.append(&more_space_lbl);
    card.append(&scale_header_row);

    let thumbs_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let mut preset_buttons = Vec::new();
    for (res_label, pct_label, scale_val) in presets {
        let btn = Button::builder()
            .css_classes(["res-thumb-btn"])
            .has_frame(false)
            .build();
        let col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .halign(Align::Center)
            .build();
        let aa = Label::builder()
            .label("Aa")
            .css_classes(["res-aa-label"])
            .build();
        let dim = Label::builder()
            .label(res_label)
            .css_classes(["res-dim-label"])
            .build();
        let pct = Label::builder()
            .label(pct_label)
            .css_classes(["res-pct-label"])
            .build();
        col.append(&aa);
        col.append(&dim);
        col.append(&pct);
        btn.set_child(Some(&col));
        thumbs_row.append(&btn);
        preset_buttons.push((*scale_val, btn));
    }
    card.append(&thumbs_row);

    // Deslizador continuo de escala fraccionaria con input exacto
    let slider_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let slider_title = Label::builder()
        .label("Ajuste continuo de escala:")
        .css_classes(["card-section-sub"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let active_info_lbl = Label::builder()
        .label("150.0% · 2560 × 1440")
        .css_classes(["card-section-title"])
        .build();
    slider_header.append(&slider_title);
    slider_header.append(&active_info_lbl);
    card.append(&slider_header);

    let slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let scale_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 2.5);
    scale_slider.set_draw_value(false);
    scale_slider.set_hexpand(true);

    let scale_entry = Entry::builder()
        .placeholder_text("150.0%")
        .width_chars(7)
        .max_width_chars(10)
        .css_classes(["scale-input-entry"])
        .valign(Align::Center)
        .build();

    let apply_btn = Button::builder()
        .label("Aplicar")
        .css_classes(["macos-action-btn"])
        .valign(Align::Center)
        .has_frame(false)
        .build();

    slider_row.append(&scale_slider);
    slider_row.append(&scale_entry);
    slider_row.append(&apply_btn);
    card.append(&slider_row);

    (
        card.clone(),
        MonitorCardWidgets {
            connector: output.connector.clone(),
            card_box: card,
            title_lbl,
            primary_btn,
            details_lbl: specs_lbl,
            active_info_lbl,
            mode_dropdown,
            modes,
            scale_slider,
            scale_entry,
            apply_btn,
            preset_buttons,
        },
    )
}

fn format_monitor_specs_label(o: &malus_de_common::config::DisplayOutputConfig) -> String {
    let physical_summary = match (o.diagonal_inches, o.ppi()) {
        (Some(inches), Some(ppi)) => format!("{inches:.1}\" · {ppi:.1} PPI"),
        _ => "tamaño desconocido".to_string(),
    };
    format!(
        "{} × {} @ {:.2} Hz · {}",
        o.width, o.height, o.refresh_rate, physical_summary
    )
}

fn format_monitor_title(o: &malus_de_common::config::DisplayOutputConfig) -> String {
    if o.name.trim().is_empty() {
        o.connector.clone()
    } else {
        format!("{} · {}", o.connector, o.name)
    }
}

fn rebuild_monitors_grid(
    monitors_grid: &GtkBox,
    monitor_widgets: &Rc<RefCell<Vec<MonitorCardWidgets>>>,
    cfg: &DisplaysConfig,
    is_syncing: &Rc<RefCell<bool>>,
    refresh_all: &Rc<dyn Fn()>,
) {
    while let Some(child) = monitors_grid.first_child() {
        monitors_grid.remove(&child);
    }
    let mut widgets_guard = monitor_widgets.borrow_mut();
    widgets_guard.clear();
    if cfg.outputs.is_empty() {
        monitors_grid.append(
            &Label::builder()
                .label("No hay pantallas activas detectadas. Conecta una pantalla para configurarla.")
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .build(),
        );
        return;
    }
    for (idx, output) in cfg.outputs.iter().enumerate() {
        let title = format_monitor_title(output);
        let specs = format_monitor_specs_label(output);
        let presets: Vec<(String, String, f64)> = [2.0, 1.75, 1.5, 1.25, 1.0]
            .iter()
            .map(|scale| {
                let width = (output.width as f64 / scale).round() as i32;
                let height = (output.height as f64 / scale).round() as i32;
                (
                    format!("{width} × {height}"),
                    format!("{:.0}%", scale * 100.0),
                    *scale,
                )
            })
            .collect();
        let (card, widgets) =
            create_macos_monitor_scaling_card(&title, &specs, &presets, output);
        wire_monitor_card_events(idx, &widgets, is_syncing, refresh_all);
        monitors_grid.append(&card);
        widgets_guard.push(widgets);
    }
}

fn update_monitor_card_ui(
    w: &MonitorCardWidgets,
    o: &malus_de_common::config::DisplayOutputConfig,
) {
    let (lw, lh) = o.logical_size();
    w.title_lbl.set_label(&format_monitor_title(o));
    w.details_lbl.set_label(&format_monitor_specs_label(o));
    w.active_info_lbl
        .set_label(&format!("{:.1}% · {} × {}", o.scale * 100.0, lw, lh));
    w.scale_slider
        .set_value((o.scale * 100.0).clamp(100.0, 250.0));
    w.scale_entry
        .set_text(&format!("{:.2}%", o.scale * 100.0));
    let selected_mode = w.modes.iter().position(|mode| {
        mode.width == o.width
            && mode.height == o.height
            && (mode.refresh_rate - o.refresh_rate).abs() < 1.0
    });
    w.mode_dropdown
        .set_selected(selected_mode.map(|idx| idx as u32).unwrap_or(u32::MAX));

    if o.primary {
        w.primary_btn.set_label("Pantalla Principal");
        w.primary_btn.add_css_class("macos-action-btn-primary");
        w.primary_btn.remove_css_class("macos-action-btn");
    } else {
        w.primary_btn.set_label("Hacer Principal");
        w.primary_btn.remove_css_class("macos-action-btn-primary");
        w.primary_btn.add_css_class("macos-action-btn");
    }

    for (preset_scale, btn) in &w.preset_buttons {
        if (o.scale - *preset_scale).abs() < 0.02 {
            btn.add_css_class("res-thumb-btn-active");
        } else {
            btn.remove_css_class("res-thumb-btn-active");
        }
    }
}

fn wire_monitor_card_events(
    output_idx: usize,
    w: &MonitorCardWidgets,
    is_syncing: &Rc<RefCell<bool>>,
    refresh_all: &Rc<dyn Fn()>,
) {
    let modes = w.modes.clone();
    let sync_mode = is_syncing.clone();
    let ref_mode = refresh_all.clone();
    w.mode_dropdown.connect_selected_notify(move |dropdown| {
        if *sync_mode.borrow() {
            return;
        }
        let Some(mode) = modes.get(dropdown.selected() as usize).copied() else {
            return;
        };

        let mut cfg = DisplaysConfig::load();
        let Some(output) = cfg.outputs.get_mut(output_idx) else {
            tracing::error!("No existe la pantalla de índice {} en la configuración", output_idx);
            return;
        };
        if output.width == mode.width
            && output.height == mode.height
            && (output.refresh_rate - mode.refresh_rate).abs() < 1.0
        {
            return;
        }
        let old_logical_width = output.width as f64 / output.scale.max(0.5);
        output.width = mode.width;
        output.height = mode.height;
        output.refresh_rate = mode.refresh_rate;
        output.scale = (mode.width as f64 / old_logical_width).clamp(0.5, 3.0);
        if output.primary {
            cfg.master_scale = output.scale;
        }
        cfg.linked_physical_1to1 = false;
        cfg.recalculate_layout();
        if let Err(err) = cfg.save() {
            tracing::error!("No se pudo guardar la resolución de la pantalla: {}", err);
            return;
        }
        if let Err(err) = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig) {
            tracing::error!("No se pudo aplicar la nueva resolución en el compositor: {}", err);
            return;
        }
        ref_mode();
    });

    // Botón Hacer Principal
    let ref_prim = refresh_all.clone();
    w.primary_btn.connect_clicked(move |_| {
        let mut cfg = DisplaysConfig::load();
        cfg.set_primary_output(output_idx);
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig);
        ref_prim();
    });

    // Presets estilo macOS
    for (target_scale, btn) in &w.preset_buttons {
        let s = *target_scale;
        let ref_p = refresh_all.clone();
        btn.connect_clicked(move |_| {
            let mut cfg = DisplaysConfig::load();
            cfg.set_output_scale_independent(output_idx, s);
            let _ = cfg.save();
            let _ = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig);
            ref_p();
        });
    }

    // Slider continuo de escala
    let sync_c = is_syncing.clone();
    let ref_sld = refresh_all.clone();
    w.scale_slider.connect_value_changed(move |sld| {
        if *sync_c.borrow() {
            return;
        }
        let pct = (sld.value() / 2.5).round() * 2.5;
        let new_scale = (pct / 100.0).clamp(0.5, 3.0);
        let mut cfg = DisplaysConfig::load();
        cfg.set_output_scale_independent(output_idx, new_scale);
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig);
        ref_sld();
    });

    // Input numérico directo y exacto de escala
    let parse_and_apply_scale = {
        let entry_c = w.scale_entry.clone();
        let ref_apply = refresh_all.clone();
        let is_sync_c = is_syncing.clone();
        Rc::new(move || {
            if *is_sync_c.borrow() {
                return;
            }
            let text = entry_c.text().trim().trim_end_matches('%').trim().to_string();
            if let Ok(mut val) = text.parse::<f64>() {
                if val <= 0.0 {
                    return;
                }
                // Permite introducir decimal (ej. 1.6667) o porcentaje (ej. 166.67 o 170)
                if val > 4.0 {
                    val /= 100.0;
                }
                let new_scale = (val.clamp(0.5, 3.0) * 10000.0).round() / 10000.0;
                let mut cfg = DisplaysConfig::load();
                cfg.set_output_scale_independent(output_idx, new_scale);
                let _ = cfg.save();
                let _ = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig);
                ref_apply();
            }
        })
    };

    let p_activate = parse_and_apply_scale.clone();
    w.scale_entry.connect_activate(move |_| {
        p_activate();
    });

    let p_click = parse_and_apply_scale.clone();
    w.apply_btn.connect_clicked(move |_| {
        p_click();
    });
}

// ============================================================================
// PANEL 2: TECLADO E IDIOMA (macOS System Settings > Keyboard)
// ============================================================================

fn create_keyboard_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["content-pane"])
        .spacing(16)
        .build();

    let header_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    let title = Label::builder()
        .label("Teclado y Fuentes de Entrada")
        .css_classes(["pane-header-title"])
        .xalign(0.0)
        .build();
    let subtitle = Label::builder()
        .label("Selecciona la distribución de teclado XKB activa en toda la sesión Wayland (por defecto Español Latinoamericano 'latam').")
        .css_classes(["pane-header-subtitle"])
        .wrap(true)
        .xalign(0.0)
        .build();
    header_box.append(&title);
    header_box.append(&subtitle);
    pane.append(&header_box);

    let layout_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let active_status_lbl = Label::builder()
        .label("Distribución activa: Español (Latinoamérica)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    layout_card.append(&active_status_lbl);

    let layouts_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .homogeneous(true)
        .build();

    let create_layout_btn = |flag_title: &str, code_sub: &str, desc: &str| -> Button {
        let btn = Button::builder()
            .css_classes(["res-thumb-btn"])
            .has_frame(false)
            .build();
        let col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let t = Label::builder()
            .label(flag_title)
            .css_classes(["card-section-title"])
            .build();
        let c = Label::builder()
            .label(code_sub)
            .css_classes(["res-dim-label"])
            .build();
        let d = Label::builder()
            .label(desc)
            .css_classes(["res-pct-label"])
            .wrap(true)
            .build();
        col.append(&t);
        col.append(&c);
        col.append(&d);
        btn.set_child(Some(&col));
        btn
    };

    let btn_latam = create_layout_btn(
        "Español (Latinoamérica)",
        "XKB: latam (Predeterminado)",
        "Tecla ñ, acentos muertos (áéíóú), ¿¡ y @ en AltGr+Q",
    );
    let btn_es = create_layout_btn(
        "Español (España)",
        "XKB: es",
        "Distribución ISO española con ç, ñ y @ en AltGr+2",
    );
    let btn_us = create_layout_btn(
        "English (US)",
        "XKB: us",
        "Distribución ANSI estadounidense estándar",
    );

    layouts_row.append(&btn_latam);
    layouts_row.append(&btn_es);
    layouts_row.append(&btn_us);
    layout_card.append(&layouts_row);

    // Campo de prueba en vivo
    let test_lbl = Label::builder()
        .label("Prueba tu teclado en vivo aquí (escribe ñ, á, é, í, ó, ú, ¿, ¡, @):")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    let test_entry = Entry::builder()
        .placeholder_text("Escribe aquí para probar: niño, canción, pingüino, ¿qué tal?...")
        .css_classes(["sidebar-search"])
        .build();
    layout_card.append(&test_lbl);
    layout_card.append(&test_entry);

    pane.append(&layout_card);

    // Tarjeta de Trackpad / Ratón
    let input_opts_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(10)
        .build();

    let nat_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    let nat_lbl = Label::builder()
        .label("Desplazamiento natural (el contenido sigue el movimiento de los dedos)")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let nat_switch = Switch::builder()
        .valign(Align::Center)
        .active(InputConfig::load().natural_scrolling)
        .build();
    nat_row.append(&nat_lbl);
    nat_row.append(&nat_switch);
    input_opts_card.append(&nat_row);

    nat_switch.connect_state_set(|_, active| {
        let mut cfg = InputConfig::load();
        cfg.natural_scrolling = active;
        let _ = cfg.save();
        glib::Propagation::Proceed
    });

    pane.append(&input_opts_card);
    scrolled.set_child(Some(&pane));

    let b_latam = btn_latam.clone();
    let b_es = btn_es.clone();
    let b_us = btn_us.clone();
    let st_lbl = active_status_lbl.clone();

    let refresh_kb: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = InputConfig::load();
        st_lbl.set_label(&format!("Distribución activa: {}", cfg.layout_display_name()));
        b_latam.remove_css_class("res-thumb-btn-active");
        b_es.remove_css_class("res-thumb-btn-active");
        b_us.remove_css_class("res-thumb-btn-active");
        match cfg.keyboard_layout.as_str() {
            "latam" => b_latam.add_css_class("res-thumb-btn-active"),
            "es" => b_es.add_css_class("res-thumb-btn-active"),
            "us" => b_us.add_css_class("res-thumb-btn-active"),
            _ => b_latam.add_css_class("res-thumb-btn-active"),
        }
    });

    refresh_kb();

    for (btn, code) in [(&btn_latam, "latam"), (&btn_es, "es"), (&btn_us, "us")] {
        let c = code.to_string();
        let ref_c = refresh_kb.clone();
        btn.connect_clicked(move |_| {
            let mut cfg = InputConfig::load();
            cfg.keyboard_layout = c.clone();
            cfg.keyboard_variant = String::new();
            let _ = cfg.save();
            let _ = IpcClient::send_command(IpcMessage::SetKeyboardLayout {
                layout: c.clone(),
                variant: String::new(),
            });
            ref_c();
        });
    }

    (scrolled, refresh_kb)
}

// ============================================================================
// PANEL 3: APARIENCIA Y DOCK
// ============================================================================

fn get_installed_cursor_themes() -> Vec<(String, String, &'static str)> {
    let mut themes: Vec<(String, String, &'static str)> = Vec::new();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dirs = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/cursors").to_string(),
        "/usr/local/share/malus-de/cursors".to_string(),
        format!("{}/.icons", home),
        format!("{}/.local/share/icons", home),
        "/usr/share/icons".to_string(),
    ];

    let mut found_names = std::collections::HashSet::new();

    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("cursors").exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name != "default" && found_names.insert(name.to_string()) {
                            let (label, desc) = match name {
                                "macOS" => ("macOS Original", "Flecha negra clásica con contorno blanco Apple"),
                                "macOS-White" => ("macOS Blanco", "Variante blanca oficial de Apple"),
                                "Adwaita" => ("Adwaita", "Tema estándar de GNOME Linux"),
                                "AdwaitaLegacy" => ("Adwaita Legacy", "Tema clásico de GNOME"),
                                other => (other, "Tema instalado por el usuario"),
                            };
                            themes.push((name.to_string(), label.to_string(), desc));
                        }
                    }
                }
            }
        }
    }

    if themes.is_empty() {
        themes.push(("macOS".to_string(), "macOS Original".to_string(), "Flecha negra clásica con contorno blanco Apple"));
        themes.push(("macOS-White".to_string(), "macOS Blanco".to_string(), "Variante blanca oficial de Apple"));
        themes.push(("Adwaita".to_string(), "Adwaita".to_string(), "Tema estándar de GNOME Linux"));
    }

    // Ordenar con temas de macOS al principio
    themes.sort_by(|a, b| {
        let is_a_mac = a.0.starts_with("macOS");
        let is_b_mac = b.0.starts_with("macOS");
        match (is_a_mac, is_b_mac) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.0.cmp(&b.0),
        }
    });

    themes
}

fn get_current_cursor_config() -> (String, u32) {
    let out_theme = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-theme"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_matches('\'').to_string())
        .unwrap_or_else(|| "macOS".to_string());

    let out_size = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "cursor-size"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(24);

    (out_theme, out_size)
}

fn is_dark_mode_active() -> bool {
    malus_de_common::config::is_dark_mode_active()
}

fn get_installed_gtk_themes() -> Vec<(String, String, &'static str)> {
    vec![
        (
            "Adwaita-Dark".to_string(),
            "Adwaita macOS Oscuro".to_string(),
            "Estilo macOS Sonoma oscuro con semáforos integrados y recortes Squircle",
        ),
        (
            "Adwaita-Light".to_string(),
            "Adwaita macOS Claro".to_string(),
            "Estilo macOS Sonoma claro con semáforos integrados y recortes Squircle",
        ),
    ]
}

fn get_current_gtk_theme() -> String {
    if is_dark_mode_active() {
        "Adwaita-Dark".to_string()
    } else {
        "Adwaita-Light".to_string()
    }
}

fn set_gtk_theme(theme_name: &str) {
    tracing::info!("🎨 Aplicando tema GTK y controles: {}", theme_name);
    let to_dark = theme_name.contains("Dark")
        || theme_name.contains("dark")
        || (!theme_name.contains("Light") && is_dark_mode_active());
    set_system_appearance_mode(to_dark);
}

fn set_system_appearance_mode(to_dark: bool) {
    let target = if to_dark { "prefer-dark" } else { "prefer-light" };
    tracing::info!("🎨 Cambiando modo de apariencia de Malus DE: {}", target);

    match IpcClient::prepare_theme_transition() {
        Ok(true) => tracing::info!("✨ Fotograma anterior guardado para la transición de tema"),
        Ok(false) => tracing::warn!(
            "No se preparó la transición de tema; se aplicará el cambio sin animación"
        ),
        Err(error) => tracing::warn!(
            "No se pudo preparar la transición de tema: {}; se aplicará el cambio sin animación",
            error
        ),
    }

    malus_de_common::config::sync_system_appearance(to_dark);

    libadwaita::StyleManager::default().set_color_scheme(
        if to_dark {
            libadwaita::ColorScheme::ForceDark
        } else {
            libadwaita::ColorScheme::ForceLight
        }
    );

    let _ = IpcClient::send_command(IpcMessage::ThemeChanged {
        theme_name: if to_dark { "Adwaita-Dark".to_string() } else { "Adwaita-Light".to_string() },
        is_dark: to_dark,
    });
}

fn set_cursor_theme_and_size(theme: &str, size: u32) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = PathBuf::from(&home).join(".config/malus-de");
    let _ = Command::new("gsettings")
        .env("XDG_CONFIG_HOME", &malus_config_dir)
        .env("GSETTINGS_BACKEND", "keyfile")
        .args(["set", "org.gnome.desktop.interface", "cursor-theme", theme])
        .status();
    let _ = Command::new("gsettings")
        .env("XDG_CONFIG_HOME", &malus_config_dir)
        .env("GSETTINGS_BACKEND", "keyfile")
        .args(["set", "org.gnome.desktop.interface", "cursor-size", &size.to_string()])
        .status();
    let dark_active = is_dark_mode_active();
    let dark_num = if dark_active { "1" } else { "0" };
    let cur_gtk_theme = get_current_gtk_theme();
    let icon_theme_name = if dark_active {
        malus_de_common::config::DEFAULT_ICON_THEME_DARK
    } else {
        malus_de_common::config::DEFAULT_ICON_THEME_LIGHT
    };

    // Actualizar los settings.ini aislados de GTK3/GTK4 preservando el tema GTK y de iconos.
    for ini_path in [
        format!("{}/.config/malus-de/gtk-3.0/settings.ini", home),
        format!("{}/.config/malus-de/gtk-4.0/settings.ini", home),
    ] {
        let p = PathBuf::from(ini_path);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = format!(
            "[Settings]\ngtk-decoration-layout=close,minimize,maximize:\ngtk-application-prefer-dark-theme={}\ngtk-cursor-theme-name={}\ngtk-cursor-theme-size={}\ngtk-theme-name={}\ngtk-icon-theme-name={}\n",
            dark_num, theme, size, cur_gtk_theme, icon_theme_name
        );
        let _ = std::fs::write(p, body);
    }

    let keyfile_path = PathBuf::from(&home).join(".config/malus-de/glib-2.0/settings/keyfile");
    if keyfile_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&keyfile_path) {
            let mut new_lines = Vec::new();
            for line in content.lines() {
                if line.starts_with("cursor-theme=") {
                    new_lines.push(format!("cursor-theme='{}'", theme));
                } else if line.starts_with("cursor-size=") {
                    new_lines.push(format!("cursor-size={}", size));
                } else {
                    new_lines.push(line.to_string());
                }
            }
            let _ = std::fs::write(&keyfile_path, new_lines.join("\n") + "\n");
        }
    }

    match Command::new("dbus-update-activation-environment")
        .args(["--systemd", "XCURSOR_THEME", "XCURSOR_SIZE"])
        .env("XCURSOR_THEME", theme)
        .env("XCURSOR_SIZE", size.to_string())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(
            "No se pudo actualizar el entorno de activación D-Bus para el cursor (código: {:?})",
            status.code()
        ),
        Err(error) => tracing::warn!(
            "No se pudo actualizar el entorno de activación D-Bus para el cursor: {error}"
        ),
    }
}

fn create_appearance_pane(
    _window: &ApplicationWindow,
    on_goto_wallpaper: Option<Rc<dyn Fn()>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["content-pane"])
        .spacing(16)
        .build();

    let title = Label::builder()
        .label("Aspecto")
        .css_classes(["pane-header-title"])
        .xalign(0.0)
        .build();
    pane.append(&title);

    // ------------------------------------------------------------------------
    // SECCIÓN 1: Modo de Apariencia del Sistema (Claro / Oscuro)
    // ------------------------------------------------------------------------
    let appearance_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(14)
        .build();

    let app_header = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .build();

    let app_title = Label::builder()
        .label("Modo de Apariencia del Sistema")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let app_desc = Label::builder()
        .label("Selecciona el aspecto visual general (claro u oscuro) para las aplicaciones y ventanas del sistema. La barra superior mantiene su diseño base transparente con texto blanco.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    app_header.append(&app_title);
    app_header.append(&app_desc);
    appearance_card.append(&app_header);

    let modes_grid = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .homogeneous(true)
        .build();

    let btn_light = Button::builder()
        .css_classes(["appearance-mode-btn"])
        .has_frame(false)
        .build();
    let light_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .halign(Align::Center)
        .build();
    let light_icon = Image::builder()
        .icon_name("weather-clear-symbolic")
        .pixel_size(24)
        .css_classes(["cursor-preview-icon"])
        .build();
    let light_lbl = Label::builder()
        .label("Claro")
        .css_classes(["cursor-theme-name"])
        .build();
    light_box.append(&light_icon);
    light_box.append(&light_lbl);
    btn_light.set_child(Some(&light_box));

    let btn_dark = Button::builder()
        .css_classes(["appearance-mode-btn"])
        .has_frame(false)
        .build();
    let dark_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .halign(Align::Center)
        .build();
    let dark_icon = Image::builder()
        .icon_name("weather-clear-night-symbolic")
        .pixel_size(24)
        .css_classes(["cursor-preview-icon"])
        .build();
    let dark_lbl = Label::builder()
        .label("Oscuro")
        .css_classes(["cursor-theme-name"])
        .build();
    dark_box.append(&dark_icon);
    dark_box.append(&dark_lbl);
    btn_dark.set_child(Some(&dark_box));

    modes_grid.append(&btn_light);
    modes_grid.append(&btn_dark);
    appearance_card.append(&modes_grid);

    pane.append(&appearance_card);

    // ------------------------------------------------------------------------
    // SECCIÓN 2: Temas de Aplicaciones y Ventanas (GTK / Libadwaita)
    // ------------------------------------------------------------------------
    let theme_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(14)
        .build();

    let theme_header = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .build();

    let theme_title = Label::builder()
        .label("Tema de Aplicaciones y Ventanas (GTK)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let theme_desc = Label::builder()
        .label("Selecciona el tema de controles, botones y estilo de ventanas para las aplicaciones GTK3, GTK4 y Libadwaita de la sesión Malus DE.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    theme_header.append(&theme_title);
    theme_header.append(&theme_desc);
    theme_card.append(&theme_header);

    let gtk_themes_list = get_installed_gtk_themes();
    let gtk_themes_grid = Grid::builder()
        .column_spacing(10)
        .row_spacing(10)
        .column_homogeneous(true)
        .build();

    let gtk_theme_buttons: Rc<RefCell<Vec<(String, Button)>>> = Rc::new(RefCell::new(Vec::new()));

    let gtk_status_hint_lbl = Label::builder()
        .label("✓ Tema aplicado en vivo para las aplicaciones de Malus DE.")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();

    let total_themes = gtk_themes_list.len();
    for (idx, (theme_id, label_txt, desc_txt)) in gtk_themes_list.iter().enumerate() {
        let btn = Button::builder()
            .css_classes(["theme-select-btn"])
            .has_frame(false)
            .build();

        let inner_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(10)
            .margin_end(10)
            .build();

        let badge_style = if theme_id.contains("Dark") {
            "badge-dark"
        } else if theme_id.contains("Light") {
            "badge-blue"
        } else if theme_id == "Adwaita" {
            "badge-teal"
        } else {
            "badge-indigo"
        };
        let icon_badge = make_system_glyph_badge(
            "preferences-desktop-appearance-symbolic",
            &["spotlight-banner-icon", badge_style],
            44,
            22,
        );
        icon_badge.set_valign(Align::Center);

        let text_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();

        let t_lbl = Label::builder()
            .label(label_txt)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build();

        let d_lbl = Label::builder()
            .label(*desc_txt)
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .wrap(true)
            .build();

        text_col.append(&t_lbl);
        text_col.append(&d_lbl);

        inner_box.append(&icon_badge);
        inner_box.append(&text_col);
        btn.set_child(Some(&inner_box));

        let col = (idx % 2) as i32;
        let row = (idx / 2) as i32;
        if idx == total_themes - 1 && idx % 2 == 0 {
            gtk_themes_grid.attach(&btn, 0, row, 2, 1);
        } else {
            gtk_themes_grid.attach(&btn, col, row, 1, 1);
        }

        gtk_theme_buttons.borrow_mut().push((theme_id.clone(), btn));
    }

    theme_card.append(&gtk_themes_grid);
    theme_card.append(&gtk_status_hint_lbl);
    pane.append(&theme_card);

    // ------------------------------------------------------------------------
    // SECCIÓN 3: Tema y Estilo del Cursor del Ratón (Puntero macOS)
    // ------------------------------------------------------------------------
    let cursor_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(14)
        .build();

    let cursor_title_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .spacing(3)
        .build();

    let cursor_title = Label::builder()
        .label("Tema y Estilo del Puntero / Cursor")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let cursor_desc = Label::builder()
        .label("Selecciona el diseño y tamaño del cursor para todas las ventanas, aplicaciones y el escritorio. Compatible con temas instalados en ~/.icons.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    cursor_title_col.append(&cursor_title);
    cursor_title_col.append(&cursor_desc);
    cursor_card.append(&cursor_title_col);

    let themes_list = get_installed_cursor_themes();
    let themes_grid = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .homogeneous(true)
        .build();

    let theme_buttons: Rc<RefCell<Vec<(String, Button)>>> = Rc::new(RefCell::new(Vec::new()));
    let size_buttons: Rc<RefCell<Vec<(u32, Button)>>> = Rc::new(RefCell::new(Vec::new()));

    let (active_theme, active_size) = get_current_cursor_config();
    let state_theme = Rc::new(RefCell::new(active_theme));
    let state_size = Rc::new(RefCell::new(active_size));

    let cursor_status_hint_lbl = Label::builder()
        .label("✓ Tema aplicado en vivo para nuevas ventanas y aplicaciones.")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();

    for (theme_id, label_txt, desc_txt) in &themes_list {
        let btn = Button::builder()
            .css_classes(["cursor-theme-btn"])
            .has_frame(false)
            .build();

        let inner_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(3)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(8)
            .margin_end(8)
            .build();

        let t_lbl = Label::builder()
            .label(label_txt)
            .css_classes(["card-section-title"])
            .xalign(0.5)
            .build();

        let d_lbl = Label::builder()
            .label(*desc_txt)
            .css_classes(["card-section-sub"])
            .xalign(0.5)
            .wrap(true)
            .build();

        inner_box.append(&t_lbl);
        inner_box.append(&d_lbl);
        btn.set_child(Some(&inner_box));

        themes_grid.append(&btn);
        theme_buttons.borrow_mut().push((theme_id.clone(), btn));
    }
    cursor_card.append(&themes_grid);

    // Fila de selección de tamaño del cursor
    let size_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();

    let size_prompt_lbl = Label::builder()
        .label("Tamaño del cursor:")
        .css_classes(["card-section-title"])
        .valign(Align::Center)
        .build();
    size_box.append(&size_prompt_lbl);

    let size_pills_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    for &(sz, sz_label) in &[
        (24, "24 px (Estándar)"),
        (32, "32 px (Mediano)"),
        (48, "48 px (Grande / 4K)"),
    ] {
        let p_btn = Button::builder()
            .css_classes(["macos-action-btn"])
            .label(sz_label)
            .has_frame(false)
            .build();
        size_pills_box.append(&p_btn);
        size_buttons.borrow_mut().push((sz, p_btn));
    }
    size_box.append(&size_pills_box);
    cursor_card.append(&size_box);
    cursor_card.append(&cursor_status_hint_lbl);

    pane.append(&cursor_card);

    // ------------------------------------------------------------------------
    // SECCIÓN 4: Opciones del Dock
    // ------------------------------------------------------------------------
    let dock_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let dock_cfg = DockConfig::load_or_create_default();

    let dock_header_lbl = Label::builder()
        .label("Dock de Aplicaciones")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    dock_card.append(&dock_header_lbl);

    let size_lbl = Label::builder()
        .label(&format!("Tamaño de iconos del Dock: {} px", dock_cfg.icon_size))
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    let size_slider = Scale::with_range(Orientation::Horizontal, 36.0, 72.0, 2.0);
    size_slider.set_value(dock_cfg.icon_size as f64);
    size_slider.set_draw_value(false);

    let size_lbl_c = size_lbl.clone();
    size_slider.connect_value_changed(move |s| {
        let val = s.value().round() as u32;
        size_lbl_c.set_label(&format!("Tamaño de iconos del Dock: {} px", val));
        let mut cfg = DockConfig::load_or_create_default();
        cfg.icon_size = val;
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDockConfig);
    });

    dock_card.append(&size_lbl);
    dock_card.append(&size_slider);
    pane.append(&dock_card);

    // ------------------------------------------------------------------------
    // SECCIÓN 4B: Superficie de Escritorio (Archivos e Iconos)
    // ------------------------------------------------------------------------
    let desktop_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let desktop_header_lbl = Label::builder()
        .label("Superficie de Escritorio (Archivos e Iconos)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    desktop_card.append(&desktop_header_lbl);

    let desk_cfg = DesktopIconsConfig::load();

    // 1. Mostrar archivos en el escritorio
    let show_row = GtkBox::new(Orientation::Horizontal, 10);
    let show_lbl = Label::builder()
        .label("Mostrar archivos en el escritorio")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let show_switch = Switch::builder()
        .valign(Align::Center)
        .active(desk_cfg.show_icons)
        .build();
    show_row.append(&show_lbl);
    show_row.append(&show_switch);
    desktop_card.append(&show_row);

    show_switch.connect_state_set(|_, active| {
        let mut cfg = DesktopIconsConfig::load();
        cfg.show_icons = active;
        let _ = cfg.save();
        glib::Propagation::Proceed
    });

    // 2. Usar Pilas en el escritorio (macOS Stacks)
    let stacks_row = GtkBox::new(Orientation::Horizontal, 10);
    let stacks_lbl = Label::builder()
        .label("Usar Pilas en el escritorio (macOS Stacks)")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let stacks_switch = Switch::builder()
        .valign(Align::Center)
        .active(desk_cfg.use_stacks)
        .build();
    stacks_row.append(&stacks_lbl);
    stacks_row.append(&stacks_switch);
    desktop_card.append(&stacks_row);

    stacks_switch.connect_state_set(|_, active| {
        let mut cfg = DesktopIconsConfig::load();
        cfg.use_stacks = active;
        let _ = cfg.save();
        glib::Propagation::Proceed
    });

    // 3. Imantar a la cuadrícula (Snap to Grid)
    let snap_row = GtkBox::new(Orientation::Horizontal, 10);
    let snap_lbl = Label::builder()
        .label("Imantar iconos a la cuadrícula (Snap to Grid)")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let snap_switch = Switch::builder()
        .valign(Align::Center)
        .active(desk_cfg.snap_to_grid)
        .build();
    snap_row.append(&snap_lbl);
    snap_row.append(&snap_switch);
    desktop_card.append(&snap_row);

    snap_switch.connect_state_set(|_, active| {
        let mut cfg = DesktopIconsConfig::load();
        cfg.snap_to_grid = active;
        let _ = cfg.save();
        glib::Propagation::Proceed
    });

    // 4. Alineación de la cuadrícula
    let align_row = GtkBox::new(Orientation::Horizontal, 10);
    let align_lbl = Label::builder()
        .label("Alineación de cuadrícula de iconos")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let align_dropdown = DropDown::from_strings(&[
        "Columna Derecha (Estilo macOS)",
        "Columna Izquierda",
    ]);
    align_dropdown.set_selected(if desk_cfg.align_left { 1 } else { 0 });
    align_row.append(&align_lbl);
    align_row.append(&align_dropdown);
    desktop_card.append(&align_row);

    align_dropdown.connect_selected_notify(|dd| {
        let mut cfg = DesktopIconsConfig::load();
        cfg.align_left = dd.selected() == 1;
        let _ = cfg.save();
    });

    // 5. Tamaño de iconos del escritorio
    let desk_size_lbl = Label::builder()
        .label(&format!("Tamaño de iconos del escritorio: {} px", desk_cfg.clamped_icon_size()))
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    let desk_size_slider = Scale::with_range(Orientation::Horizontal, 32.0, 96.0, 4.0);
    desk_size_slider.set_value(desk_cfg.clamped_icon_size() as f64);
    desk_size_slider.set_draw_value(false);

    let desk_size_lbl_c = desk_size_lbl.clone();
    desk_size_slider.connect_value_changed(move |s| {
        let val = s.value().round() as u32;
        desk_size_lbl_c.set_label(&format!("Tamaño de iconos del escritorio: {} px", val));
        let mut cfg = DesktopIconsConfig::load();
        cfg.icon_size = val;
        let _ = cfg.save();
    });

    desktop_card.append(&desk_size_lbl);
    desktop_card.append(&desk_size_slider);
    pane.append(&desktop_card);

    // ------------------------------------------------------------------------
    // SECCIÓN 5: Fondo de Pantalla (Enlace a la sección dedicada)
    // ------------------------------------------------------------------------
    let wp_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(10)
        .build();

    let wp_header_lbl = Label::builder()
        .label("Fondo de Pantalla del Escritorio")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let wp_desc_lbl = Label::builder()
        .label("Personaliza el fondo de escritorio con colecciones exclusivas de macOS, fotos de tu galería o degradados dinámicos.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    wp_card.append(&wp_header_lbl);
    wp_card.append(&wp_desc_lbl);

    if let Some(goto_wp) = on_goto_wallpaper {
        let btn_open_wp = Button::builder()
            .label("Personalizar fondos de pantalla...")
            .css_classes(["macos-action-btn"])
            .halign(Align::Start)
            .has_frame(false)
            .build();
        btn_open_wp.connect_clicked(move |_| {
            goto_wp();
        });
        wp_card.append(&btn_open_wp);
    }

    pane.append(&wp_card);

    scrolled.set_child(Some(&pane));

    // Función para refrescar la selección visual activa de temas y tamaños
    let gtk_tb_ref = gtk_theme_buttons.clone();
    let tb_ref = theme_buttons.clone();
    let sb_ref = size_buttons.clone();
    let st_theme = state_theme.clone();
    let st_size = state_size.clone();
    let btn_l_ref = btn_light.clone();
    let btn_d_ref = btn_dark.clone();

    let refresh_appearance: Rc<dyn Fn()> = Rc::new(move || {
        let current_gtk = get_current_gtk_theme();
        let (current_t, current_s) = get_current_cursor_config();
        *st_theme.borrow_mut() = current_t.clone();
        *st_size.borrow_mut() = current_s;

        // Modo Claro / Oscuro
        if is_dark_mode_active() {
            btn_d_ref.add_css_class("appearance-mode-btn-active");
            btn_l_ref.remove_css_class("appearance-mode-btn-active");
        } else {
            btn_l_ref.add_css_class("appearance-mode-btn-active");
            btn_d_ref.remove_css_class("appearance-mode-btn-active");
        }

        // Temas GTK
        for (id, btn) in gtk_tb_ref.borrow().iter() {
            if id == &current_gtk {
                btn.add_css_class("theme-select-btn-active");
                btn.remove_css_class("theme-select-btn");
            } else {
                btn.remove_css_class("theme-select-btn-active");
                btn.add_css_class("theme-select-btn");
            }
        }

        // Temas de cursor
        for (id, btn) in tb_ref.borrow().iter() {
            if id == &current_t {
                btn.add_css_class("cursor-theme-btn-active");
                btn.remove_css_class("cursor-theme-btn");
            } else {
                btn.remove_css_class("cursor-theme-btn-active");
                btn.add_css_class("cursor-theme-btn");
            }
        }

        // Tamaños de cursor
        for (sz, btn) in sb_ref.borrow().iter() {
            if *sz == current_s {
                btn.add_css_class("macos-action-btn-primary");
                btn.remove_css_class("macos-action-btn");
            } else {
                btn.remove_css_class("macos-action-btn-primary");
                btn.add_css_class("macos-action-btn");
            }
        }
    });

    refresh_appearance();

    // Conectar eventos del modo de apariencia
    let ref_app_1 = refresh_appearance.clone();
    btn_light.connect_clicked(move |_| {
        let cur = get_current_gtk_theme();
        if cur == "WhiteSur-Dark" {
            set_gtk_theme("WhiteSur-Light");
        } else if cur == "WhiteSur-Dark-solid" {
            set_gtk_theme("WhiteSur-Light-solid");
        } else {
            set_system_appearance_mode(false);
        }
        ref_app_1();
    });

    let ref_app_2 = refresh_appearance.clone();
    btn_dark.connect_clicked(move |_| {
        let cur = get_current_gtk_theme();
        if cur == "WhiteSur-Light" {
            set_gtk_theme("WhiteSur-Dark");
        } else if cur == "WhiteSur-Light-solid" {
            set_gtk_theme("WhiteSur-Dark-solid");
        } else {
            set_system_appearance_mode(true);
        }
        ref_app_2();
    });

    let ref_app_notify = refresh_appearance.clone();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        ref_app_notify();
    });

    // Conectar eventos de clic de los botones de tema GTK
    for (t_id, btn) in gtk_theme_buttons.borrow().iter() {
        let target_theme = t_id.clone();
        let ref_fn = refresh_appearance.clone();
        let hint_lbl_c = gtk_status_hint_lbl.clone();

        btn.connect_clicked(move |_| {
            set_gtk_theme(&target_theme);
            hint_lbl_c.set_label(&format!("✓ Tema '{}' aplicado en vivo para las aplicaciones de Malus DE.", target_theme));
            ref_fn();
        });
    }

    // Conectar eventos de clic de los botones de tema de cursor
    for (t_id, btn) in theme_buttons.borrow().iter() {
        let target_theme = t_id.clone();
        let st_size_c = state_size.clone();
        let ref_fn = refresh_appearance.clone();

        btn.connect_clicked(move |_| {
            let sz = *st_size_c.borrow();
            set_cursor_theme_and_size(&target_theme, sz);
            ref_fn();
        });
    }

    // Conectar eventos de clic de los botones de tamaño de cursor
    for (sz, btn) in size_buttons.borrow().iter() {
        let target_size = *sz;
        let st_theme_c = state_theme.clone();
        let ref_fn = refresh_appearance.clone();

        btn.connect_clicked(move |_| {
            let current_t = st_theme_c.borrow().clone();
            set_cursor_theme_and_size(&current_t, target_size);
            ref_fn();
        });
    }

    (scrolled, refresh_appearance)
}

// ============================================================================
// PANEL 3.5: FONDO DE PANTALLA (WALLPAPER)
// ============================================================================

#[derive(Clone)]
#[allow(dead_code)]
struct WallpaperPreset {
    id: String,
    title: String,
    subtitle: String,
    path: Option<PathBuf>,
}

fn get_current_wallpaper_file() -> Option<PathBuf> {
    let malus_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/malus-de")
        });
    let candidates = [
        malus_dir.join("wallpaper.png"),
        malus_dir.join("wallpaper.jpg"),
        malus_dir.join("wallpaper.jpeg"),
        malus_dir.join("wallpaper.webp"),
        malus_dir.join("wallpaper.svg"),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists() && std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .or_else(|| {
            if malus_dir.join(".wallpaper-gradient").exists() {
                None
            } else {
                malus_de_common::config::default_bundled_wallpaper_path()
            }
        })
}

fn apply_system_wallpaper(source: Option<&std::path::Path>) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".config/malus-de"));
    let _ = std::fs::create_dir_all(&malus_dir);
    let gradient_marker = malus_dir.join(".wallpaper-gradient");

    for ext in &["png", "jpg", "jpeg", "webp", "svg"] {
        let p = malus_dir.join(format!("wallpaper.{}", ext));
        let _ = std::fs::remove_file(&p);
    }

    if let Some(src) = source {
        let _ = std::fs::remove_file(&gradient_marker);
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();
        let target = malus_dir.join(format!("wallpaper.{}", ext));
        let _ = std::fs::copy(src, &target);

        let uri = format!("file://{}", target.to_str().unwrap_or(""));
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", &malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["set", "org.gnome.desktop.background", "picture-uri", &uri])
            .status();
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", &malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["set", "org.gnome.desktop.background", "picture-uri-dark", &uri])
            .status();
    } else {
        let _ = std::fs::write(&gradient_marker, b"1");
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", &malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["reset", "org.gnome.desktop.background", "picture-uri"])
            .status();
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", &malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["reset", "org.gnome.desktop.background", "picture-uri-dark"])
            .status();
    }

    // Reiniciar malus-de-wallpaper
    let _ = Command::new("killall").args(["-9", "malus-de-wallpaper"]).status();
    let wayland_disp = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());

    let mut bin_candidates = [
        std::env::var_os("MALUS_DE_BIN_DIR")
            .map(|dir| PathBuf::from(dir).join("malus-de-wallpaper")),
        Some(PathBuf::from(&home).join("Proyectos/malus-de/target/release/malus-de-wallpaper")),
        Some(PathBuf::from(&home).join("Apps/malus-de/target/release/malus-de-wallpaper")),
        Some(PathBuf::from("/usr/local/bin/malus-de-wallpaper")),
    ]
    .into_iter()
    .flatten();
    let exec_cmd = bin_candidates
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "malus-de-wallpaper".to_string());

    let mut cmd = Command::new(&exec_cmd);
    cmd.env("WAYLAND_DISPLAY", &wayland_disp);
    cmd.env("GDK_BACKEND", "wayland");
    cmd.env("XDG_CONFIG_HOME", format!("{}/.config/malus-de", home));
    let _ = cmd.spawn();
}

fn get_available_wallpaper_presets() -> Vec<WallpaperPreset> {
    let mut list = Vec::new();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

    if let Some(default_wp) = malus_de_common::config::default_bundled_wallpaper_path() {
        list.push(WallpaperPreset {
            id: "default-pacasmayo".to_string(),
            title: "Playa de Pacasmayo (Predeterminado)".to_string(),
            subtitle: "Pexels — J. Barboza Meca".to_string(),
            path: Some(default_wp),
        });
    }

    list.push(WallpaperPreset {
        id: "default-gradient".to_string(),
        title: "Degradado Aurora".to_string(),
        subtitle: "macOS Dynamic Shader".to_string(),
        path: None,
    });

    for folder_name in ["Imágenes/Wallpapers", "Imágenes/Wallpaper"] {
        let user_wp_dir = PathBuf::from(&home).join(folder_name);
        if let Ok(entries) = std::fs::read_dir(&user_wp_dir) {
            let mut user_files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    if p.file_name().and_then(|n| n.to_str())
                        == Some(malus_de_common::config::DEFAULT_WALLPAPER_FILENAME)
                    {
                        return false;
                    }
                    if let Some(ext) = p.extension().and_then(|x| x.to_str()) {
                        let lext = ext.to_lowercase();
                        lext == "jpg" || lext == "jpeg" || lext == "png" || lext == "webp"
                    } else {
                        false
                    }
                })
                .collect();
            user_files.sort();
            user_files.truncate(8);
            for p in user_files {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Fondo").to_string();
                let title = if stem.contains("beach-aerial") {
                    "Costa de California (macOS)".to_string()
                } else if stem.contains("Blueprint") {
                    "macOS Blueprint".to_string()
                } else if stem.contains("Blackprint") {
                    "macOS Blackprint".to_string()
                } else if stem.contains("do_more") {
                    "Do More (Inspirational)".to_string()
                } else if stem.contains("growing-tree") {
                    "Growing Tree 4K".to_string()
                } else if stem.len() > 24 {
                    format!("{}...", &stem[..21])
                } else {
                    stem
                };

                list.push(WallpaperPreset {
                    id: format!("user-{}", p.to_string_lossy()),
                    title,
                    subtitle: "Galería de Wallpapers".to_string(),
                    path: Some(p),
                });
            }
        }
    }

    for dir in &["/usr/share/backgrounds/fedora-workstation", "/usr/share/backgrounds/gnome"] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut sys_files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    if let Some(ext) = p.extension().and_then(|x| x.to_str()) {
                        let lext = ext.to_lowercase();
                        lext == "webp" || lext == "jpg" || lext == "png"
                    } else {
                        false
                    }
                })
                .collect();
            sys_files.sort();
            sys_files.truncate(4);
            for p in sys_files {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Fondo").to_string();
                if stem.ends_with("-d") || stem.ends_with("_dark") {
                    continue;
                }
                let clean_name = stem.trim_end_matches("-l").trim_end_matches("_light");
                list.push(WallpaperPreset {
                    id: format!("sys-{}", p.to_string_lossy()),
                    title: format!("{} (Sistema)", clean_name),
                    subtitle: "Fondo del sistema".to_string(),
                    path: Some(p),
                });
            }
        }
    }

    list
}

fn create_wallpaper_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["content-pane"])
        .spacing(16)
        .build();

    let title = Label::builder()
        .label("Fondo de pantalla")
        .css_classes(["pane-header-title"])
        .xalign(0.0)
        .build();

    let subtitle = Label::builder()
        .label("Personaliza el fondo de escritorio con colecciones auténticas de macOS, fondos dinámicos o tus fotos.")
        .css_classes(["pane-header-subtitle"])
        .xalign(0.0)
        .wrap(true)
        .build();

    pane.append(&title);
    pane.append(&subtitle);

    // ==========================================
    // TARJETA 1: Vista Previa y Fondo Actual
    // ==========================================
    let current_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let current_title = Label::builder()
        .label("Fondo de Pantalla Actual")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    current_card.append(&current_title);

    let preview_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["wallpaper-preview-box"])
        .height_request(160)
        .hexpand(true)
        .build();
    current_card.append(&preview_container);

    let current_status_lbl = Label::builder()
        .label("Cargando fondo...")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    current_card.append(&current_status_lbl);

    let actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();

    let btn_choose = Button::builder()
        .label("Seleccionar archivo...")
        .css_classes(["macos-action-btn-primary"])
        .has_frame(false)
        .build();

    let btn_reset = Button::builder()
        .label("Restablecer degradado macOS")
        .css_classes(["macos-action-btn"])
        .has_frame(false)
        .build();

    let btn_open_dir = Button::builder()
        .label("Abrir carpeta de Wallpapers")
        .css_classes(["macos-action-btn"])
        .has_frame(false)
        .build();

    actions_row.append(&btn_choose);
    actions_row.append(&btn_reset);
    actions_row.append(&btn_open_dir);
    current_card.append(&actions_row);

    pane.append(&current_card);

    // ==========================================
    // TARJETA 2: Galería de Fondos Disponibles
    // ==========================================
    let gallery_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(12)
        .build();

    let gallery_title = Label::builder()
        .label("Galería de Fondos y Colecciones")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let gallery_sub = Label::builder()
        .label("Haz clic en cualquier diseño para aplicarlo instantáneamente como fondo de pantalla.")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();

    gallery_card.append(&gallery_title);
    gallery_card.append(&gallery_sub);

    let flow = FlowBox::builder()
        .valign(Align::Start)
        .max_children_per_line(4)
        .min_children_per_line(2)
        .selection_mode(gtk4::SelectionMode::None)
        .row_spacing(12)
        .column_spacing(12)
        .build();

    let presets = get_available_wallpaper_presets();
    let preset_buttons: Rc<RefCell<Vec<(WallpaperPreset, Button)>>> = Rc::new(RefCell::new(Vec::new()));

    for preset in &presets {
        let btn = Button::builder()
            .css_classes(["wallpaper-grid-item"])
            .has_frame(false)
            .build();

        let item_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .width_request(160)
            .build();

        if let Some(ref p) = preset.path {
            let pic = if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file_at_scale(p, 160, 95, true) {
                let texture = gtk4::gdk::Texture::for_pixbuf(&pb);
                Picture::for_paintable(&texture)
            } else {
                Picture::for_filename(p)
            };
            pic.set_content_fit(gtk4::ContentFit::Cover);
            pic.set_size_request(160, 95);
            pic.set_css_classes(&["wallpaper-thumb-frame"]);
            item_box.append(&pic);
        } else {
            let grad_box = GtkBox::builder()
                .css_classes(["wallpaper-gradient-box", "wallpaper-thumb-frame"])
                .width_request(160)
                .height_request(95)
                .build();
            item_box.append(&grad_box);
        }

        let name_lbl = Label::builder()
            .label(&preset.title)
            .css_classes(["wallpaper-item-title"])
            .xalign(0.5)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        item_box.append(&name_lbl);

        btn.set_child(Some(&item_box));
        flow.append(&btn);
        preset_buttons.borrow_mut().push((preset.clone(), btn));
    }

    gallery_card.append(&flow);
    pane.append(&gallery_card);

    scrolled.set_child(Some(&pane));

    // Refrescar UI del panel de fondos
    let prev_box_c = preview_container.clone();
    let status_lbl_c = current_status_lbl.clone();
    let p_btns_c = preset_buttons.clone();

    let refresh_wallpaper_pane: Rc<dyn Fn()> = Rc::new(move || {
        let active_file = get_current_wallpaper_file();

        while let Some(child) = prev_box_c.first_child() {
            prev_box_c.remove(&child);
        }

        if let Some(ref path) = active_file {
            let pic = if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file_at_scale(path, 480, 200, true) {
                let texture = gtk4::gdk::Texture::for_pixbuf(&pb);
                Picture::for_paintable(&texture)
            } else {
                Picture::for_filename(path)
            };
            pic.set_content_fit(gtk4::ContentFit::Cover);
            pic.set_hexpand(true);
            pic.set_vexpand(true);
            prev_box_c.append(&pic);

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("wallpaper");
            status_lbl_c.set_label(&format!("Fondo activo: {} ({})", name, path.display()));
        } else {
            let grad = GtkBox::builder()
                .css_classes(["wallpaper-gradient-box"])
                .hexpand(true)
                .vexpand(true)
                .build();
            prev_box_c.append(&grad);
            status_lbl_c.set_label("Fondo activo: Degradado Aurora Dinámico (macOS Oficial)");
        }

        for (preset, btn) in p_btns_c.borrow().iter() {
            let is_match = match (&preset.path, &active_file) {
                (None, None) => true,
                (Some(p), Some(act)) => {
                    p == act || p.file_name() == act.file_name()
                }
                _ => false,
            };

            if is_match {
                btn.add_css_class("wallpaper-grid-item-active");
            } else {
                btn.remove_css_class("wallpaper-grid-item-active");
            }
        }
    });

    refresh_wallpaper_pane();

    for (preset, btn) in preset_buttons.borrow().iter() {
        let p_path = preset.path.clone();
        let ref_fn = refresh_wallpaper_pane.clone();
        btn.connect_clicked(move |_| {
            apply_system_wallpaper(p_path.as_deref());
            ref_fn();
        });
    }

    let ref_reset = refresh_wallpaper_pane.clone();
    btn_reset.connect_clicked(move |_| {
        let def_wp = malus_de_common::config::default_bundled_wallpaper_path();
        apply_system_wallpaper(def_wp.as_deref());
        ref_reset();
    });

    btn_open_dir.connect_clicked(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let p = format!("{}/Imágenes/Wallpapers", home);
        let _ = Command::new("xdg-open").arg(p).spawn();
    });

    let win_c = window.clone();
    let ref_choose = refresh_wallpaper_pane.clone();
    btn_choose.connect_clicked(move |_| {
        let filter = build_wallpaper_file_filter();
        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);

        let chooser = FileDialog::builder()
            .title("Seleccionar fondo de pantalla")
            .accept_label("Establecer Fondo")
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();

        if let Some(folder) = wallpaper_chooser_initial_folder() {
            chooser.set_initial_folder(Some(&gtk4::gio::File::for_path(folder)));
        }

        let ref_sub = ref_choose.clone();
        chooser.open(
            Some(&win_c),
            gtk4::gio::Cancellable::NONE,
            move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        apply_system_wallpaper(Some(&path));
                        ref_sub();
                    }
                }
            },
        );
    });

    (scrolled, refresh_wallpaper_pane)
}

fn build_wallpaper_file_filter() -> FileFilter {
    let filter = FileFilter::new();
    filter.set_name(Some("Archivos de imagen (*.jpg, *.png, *.webp, *.svg)"));
    for mime in ["image/jpeg", "image/png", "image/webp", "image/svg+xml"] {
        filter.add_mime_type(mime);
    }
    filter
}

fn wallpaper_chooser_initial_folder_in(home: &std::path::Path) -> Option<PathBuf> {
    [
        "Imágenes/Wallpapers",
        "Pictures/Wallpapers",
        "Imágenes",
        "Pictures",
    ]
    .into_iter()
    .map(|rel| home.join(rel))
    .find(|p| p.is_dir())
}

fn wallpaper_chooser_initial_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    wallpaper_chooser_initial_folder_in(&home)
}

// ============================================================================
// PANEL 4: SONIDO
// ============================================================================

fn create_sound_pane() -> ScrolledWindow {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["content-pane"])
        .spacing(16)
        .build();

    let title = Label::builder()
        .label("Sonido")
        .css_classes(["pane-header-title"])
        .xalign(0.0)
        .build();
    pane.append(&title);

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(10)
        .build();

    let vol_lbl = Label::builder()
        .label("Volumen de salida maestro (PipeWire / WirePlumber)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let vol_slider = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    vol_slider.set_value(75.0);
    vol_slider.connect_value_changed(|s| {
        let pct = s.value().round() as i32;
        let _ = Command::new("wpctl")
            .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{}%", pct)])
            .spawn();
    });

    card.append(&vol_lbl);
    card.append(&vol_slider);
    pane.append(&card);

    scrolled.set_child(Some(&pane));
    scrolled
}

// ============================================================================
// PANEL 5: ACERCA DE ESTE EQUIPO
// ============================================================================

fn create_about_pane() -> ScrolledWindow {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["content-pane"])
        .spacing(16)
        .build();

    let title = Label::builder()
        .label("Acerca de este equipo")
        .css_classes(["pane-header-title"])
        .xalign(0.0)
        .build();
    pane.append(&title);

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["macos-group-card"])
        .spacing(8)
        .build();

    for (icon_name, line) in [
        ("start-here-symbolic", "Malus Desktop Environment v1.1.0"),
        ("video-display-symbolic", "Compositor: Smithay 0.7 (Wayland Multi-CRTC DRM/KMS)"),
        ("applications-graphics-symbolic", "Gráficos: AMD Radeon Vega 7 (OpenGL ES 3.2 / GBM)"),
        (
            "preferences-desktop-display-symbolic",
            "Pantallas: detección DRM/KMS de conectores, modos de resolución y escalado fraccionario",
        ),
        ("input-keyboard-symbolic", "Teclado predeterminado: Español Latinoamericano (XKB 'latam')"),
    ] {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .build();
        let glyph = Image::builder()
            .icon_name(icon_name)
            .pixel_size(16)
            .valign(Align::Center)
            .build();
        let lbl = Label::builder()
            .label(line)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .build();
        row.append(&glyph);
        row.append(&lbl);
        card.append(&row);
    }

    pane.append(&card);
    scrolled.set_child(Some(&pane));
    scrolled
}

fn create_spotlight_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .css_classes(["content-pane"])
        .build();

    // 1. Tarjeta Banner Superior (igual a la imagen de referencia Descargas/ajustes.webp)
    let banner_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .css_classes(["macos-group-card"])
        .build();

    let icon_badge = make_system_glyph_badge(
        "system-search-symbolic",
        &["spotlight-banner-icon", "badge-blue"],
        44,
        22,
    );
    icon_badge.set_valign(Align::Start);

    let banner_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let title_lbl = Label::builder()
        .label("Spotlight")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let desc_lbl = Label::builder()
        .label("Spotlight permite realizar búsquedas rápidas en el ordenador y muestra sugerencias de internet, Música, App Store, la cartelera de cine, ubicaciones cercanas y mucho más.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    banner_text.append(&title_lbl);
    banner_text.append(&desc_lbl);
    banner_card.append(&icon_badge);
    banner_card.append(&banner_text);
    pane.append(&banner_card);

    // 2. Título de Sección: "Resultados de la búsqueda"
    let sec_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .build();

    let sec_title = Label::builder()
        .label("Resultados de la búsqueda")
        .css_classes(["section-header-title"])
        .xalign(0.0)
        .build();

    let sec_sub = Label::builder()
        .label("En los resultados de la búsqueda de Spotlight, solo se incluirán las categorías seleccionadas.")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();

    sec_box.append(&sec_title);
    sec_box.append(&sec_sub);
    pane.append(&sec_box);

    // 3. Tarjeta de Lista con Casillas de Verificación (CheckButton estilo macOS)
    let list_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["macos-group-card", "spotlight-list-card"])
        .build();

    let categories = [
        "Ajustes del Sistema",
        "Aplicaciones",
        "Calculadora",
        "Carpetas",
        "Consejos",
        "Contactos",
        "Conversión",
        "Correo y mensajes",
        "Definición",
        "Documentos",
        "Documentos PDF",
        "Eventos y recordatorios",
        "Hojas de cálculo",
        "Imágenes",
        "Música",
        "Otros",
        "Presentaciones",
        "Sitios web",
        "Sugerencias de Siri",
        "Tipos de letra",
        "Vídeos",
    ];

    for (i, cat) in categories.iter().enumerate() {
        if i > 0 {
            let sep = GtkBox::builder()
                .css_classes(["list-row-separator"])
                .build();
            list_card.append(&sep);
        }

        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["list-option-row"])
            .build();

        let chk = CheckButton::builder()
            .label(*cat)
            .active(true)
            .hexpand(true)
            .css_classes(["macos-checkbox"])
            .build();

        row.append(&chk);
        list_card.append(&row);
    }

    pane.append(&list_card);
    scrolled.set_child(Some(&pane));

    let refresh: Rc<dyn Fn()> = Rc::new(|| {});
    (scrolled, refresh)
}

fn create_dock_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .css_classes(["content-pane"])
        .build();

    let banner_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .css_classes(["macos-group-card"])
        .build();

    let icon_badge = make_system_glyph_badge(
        "user-desktop-symbolic",
        &["spotlight-banner-icon", "badge-blue"],
        44,
        22,
    );
    icon_badge.set_valign(Align::Start);

    let banner_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let title_lbl = Label::builder()
        .label("Escritorio y Dock")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let desc_lbl = Label::builder()
        .label("Personaliza la apariencia del Dock flotante, el tamaño de iconos, el efecto Genie de minimización y el auto-ocultado.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    banner_text.append(&title_lbl);
    banner_text.append(&desc_lbl);
    banner_card.append(&icon_badge);
    banner_card.append(&banner_text);
    pane.append(&banner_card);

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["macos-group-card"])
        .build();

    let cfg = DockConfig::load_or_create_default();

    // Tamaño del Dock Slider
    let size_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(16)
        .build();
    let size_lbl = Label::builder()
        .label("Tamaño:")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .width_request(130)
        .build();
    let size_slider = Scale::with_range(Orientation::Horizontal, 36.0, 96.0, 2.0);
    size_slider.set_hexpand(true);
    size_slider.set_value(cfg.icon_size as f64);
    size_slider.connect_value_changed(|s| {
        let mut cfg = DockConfig::load_or_create_default();
        cfg.icon_size = s.value().round() as u32;
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDockConfig);
    });
    size_row.append(&size_lbl);
    size_row.append(&size_slider);
    card.append(&size_row);

    // Ampliación / Magnification
    let mag_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(16)
        .build();
    let mag_lbl = Label::builder()
        .label("Ampliación:")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let mag_sw = Switch::builder()
        .active(cfg.magnification_enabled)
        .valign(Align::Center)
        .build();
    mag_sw.connect_state_set(|_, active| {
        let mut cfg = DockConfig::load_or_create_default();
        cfg.magnification_enabled = active;
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDockConfig);
        glib::Propagation::Proceed
    });
    mag_row.append(&mag_lbl);
    mag_row.append(&mag_sw);
    card.append(&mag_row);

    // Efecto de minimización
    let eff_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(16)
        .build();
    let eff_lbl = Label::builder()
        .label("Efecto al minimizar:")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let eff_val = Label::builder()
        .label("Efecto Aladino (Genie Lamp)")
        .css_classes(["card-section-sub"])
        .build();
    eff_row.append(&eff_lbl);
    eff_row.append(&eff_val);
    card.append(&eff_row);

    // Estilo del selector de aplicaciones (Alt+Tab)
    let switcher_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(16)
        .build();
    let switcher_lbl = Label::builder()
        .label("Selector de aplicaciones (Alt+Tab):")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let switcher_dropdown = DropDown::from_strings(&[
        "Tarjetas en cascada (Predeterminado)",
        "Barra clásica de iconos",
    ]);
    switcher_dropdown.set_selected(match cfg.app_switcher_style {
        AppSwitcherStyle::Deck => 0,
        AppSwitcherStyle::Classic => 1,
    });
    switcher_dropdown.connect_selected_notify(|dd| {
        let mut cfg = DockConfig::load_or_create_default();
        cfg.app_switcher_style = if dd.selected() == 1 {
            AppSwitcherStyle::Classic
        } else {
            AppSwitcherStyle::Deck
        };
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadDockConfig);
    });
    switcher_row.append(&switcher_lbl);
    switcher_row.append(&switcher_dropdown);
    card.append(&switcher_row);

    pane.append(&card);
    scrolled.set_child(Some(&pane));

    let refresh: Rc<dyn Fn()> = Rc::new(|| {});
    (scrolled, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallpaper_chooser_initial_folder_prefers_wallpapers_subdirs() {
        let temp_root = std::env::temp_dir().join("malus_settings_wallpaper_folder_test");
        let _ = std::fs::remove_dir_all(&temp_root);
        let images = temp_root.join("Imágenes");
        let wallpapers = images.join("Wallpapers");
        std::fs::create_dir_all(&wallpapers).unwrap();

        assert_eq!(
            wallpaper_chooser_initial_folder_in(&temp_root),
            Some(wallpapers.clone())
        );

        std::fs::remove_dir_all(&wallpapers).unwrap();
        assert_eq!(
            wallpaper_chooser_initial_folder_in(&temp_root),
            Some(images)
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn gtk_ui_smoke_checks() {
        if gtk4::init().is_err() {
            return;
        }
        let _ = libadwaita::init();
        load_macos_settings_css();

        // 1. Verificar filtro de selector de fondo (FileDialog)
        let filter = build_wallpaper_file_filter();
        assert_eq!(
            filter.name().as_deref(),
            Some("Archivos de imagen (*.jpg, *.png, *.webp, *.svg)")
        );
        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);
        let dialog = FileDialog::builder()
            .title("Seleccionar fondo de pantalla")
            .accept_label("Establecer Fondo")
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        assert_eq!(dialog.title(), "Seleccionar fondo de pantalla");
        assert_eq!(dialog.accept_label().as_deref(), Some("Establecer Fondo"));
        assert!(dialog.is_modal());

        // 2. Verificar alto <= 36 px y squircle del filtro lateral
        let search_entry = build_sidebar_search_entry();
        let shell = create_squircle_search_shell(&search_entry);
        let (_entry_min_h, _entry_nat_h, _, _) = search_entry.measure(Orientation::Vertical, 200);
        let (min_h, nat_h, _, _) = shell.measure(Orientation::Vertical, 200);
        assert!(
            min_h <= SIDEBAR_SEARCH_MAX_HEIGHT,
            "La altura mínima ({min_h}) supera {SIDEBAR_SEARCH_MAX_HEIGHT}px"
        );
        assert!(
            nat_h <= SIDEBAR_SEARCH_MAX_HEIGHT,
            "La altura natural ({nat_h}) supera {SIDEBAR_SEARCH_MAX_HEIGHT}px"
        );
        assert_eq!(SETTINGS_SIDEBAR_WIDTH, 212);
        let (min_w, _, _, _) = shell.measure(Orientation::Horizontal, -1);
        assert!(
            min_w <= SETTINGS_SIDEBAR_WIDTH - 20,
            "El ancho mínimo del buscador ({min_w}) excede el espacio útil de la sidebar ({SETTINGS_SIDEBAR_WIDTH}px)"
        );
        assert!(SIDEBAR_SEARCH_HEIGHT <= SIDEBAR_SEARCH_MAX_HEIGHT);

        // 3. Verificar altura fija (sin vexpand) y alineación izquierda en ítems de la sidebar
        let nav_btn = build_sidebar_nav_button("preferences-desktop-appearance-symbolic", "Aspecto");
        assert!(!nav_btn.property::<bool>("vexpand"));
        assert_eq!(nav_btn.valign(), Align::Start);
        let (btn_min_h, btn_nat_h, _, _) = nav_btn.measure(Orientation::Vertical, 200);
        assert!(
            btn_min_h <= 36 && btn_nat_h <= 36,
            "La fila de la sidebar creció verticalmente: min={btn_min_h}, nat={btn_nat_h}"
        );

        // 4. Smoke checks de widgets squircle compartidos entre Wi-Fi y Bluetooth
        let bt_badge = wifi_pane::make_squircle_badge(
            34,
            "bluetooth-active-symbolic",
            18,
            wifi_pane::WIFI_BADGE_SQUIRCLE_RADIUS,
        );
        assert_eq!(bt_badge.width_request(), 34);
        assert_eq!(bt_badge.height_request(), 34);

        let card_inner = GtkBox::new(Orientation::Vertical, 0);
        let card_wrap =
            wifi_pane::wrap_squircle_card(&card_inner, wifi_pane::WIFI_CARD_SQUIRCLE_RADIUS);
        assert!(card_wrap.child().is_some());

        let (btn_wrap, btn) = wifi_pane::make_squircle_button("Conectar", false, None);
        assert!(btn_wrap.child().is_some());
        assert_eq!(btn.label().as_deref(), Some("Conectar"));

        let fw_badge = wifi_pane::make_squircle_colored_badge(
            34,
            "security-high-symbolic",
            18,
            wifi_pane::WIFI_BADGE_SQUIRCLE_RADIUS,
            (0.988, 0.584, 0.129),
            (0.945, 0.459, 0.039),
        );
        assert_eq!(fw_badge.width_request(), 34);
        assert_eq!(fw_badge.height_request(), 34);

        let (help_wrap, _) = wifi_pane::make_help_circle_button("Ayuda");
        assert_eq!(help_wrap.width_request(), 22);
        assert_eq!(help_wrap.height_request(), 22);

        let (pill_wrap, pill_btn) =
            wifi_pane::build_squircle_chevron_menu_button("list-add-symbolic", "Añadir");
        assert_eq!(pill_btn.height_request(), 22);
        let (pill_min_w, pill_nat_w, _, _) = pill_wrap.measure(Orientation::Horizontal, -1);
        let (pill_min_h, pill_nat_h, _, _) = pill_wrap.measure(Orientation::Vertical, -1);
        assert!(
            pill_min_w <= 44 && pill_nat_w <= 44,
            "La píldora + ⌄ está demasiado ancha: min={pill_min_w}, nat={pill_nat_w}"
        );
        assert!(
            pill_min_h <= 24 && pill_nat_h <= 24,
            "La píldora + ⌄ excede altura compacta: min={pill_min_h}, nat={pill_nat_h}"
        );
    }
}
