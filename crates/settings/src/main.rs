use clap::Parser;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea, Entry,
    EventControllerFocus, FlowBox, Image, Label, Orientation, Overlay, PackType, PolicyType,
    ScrolledWindow, SearchEntry, Stack, StackTransitionType, WindowControls, WindowHandle,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use malus_de_common::config::InputConfig;
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::logging::init_logging;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

mod accessibility_pane;
mod appearance_pane;
mod battery_pane;
mod bluetooth_pane;
mod canopy_pane;
mod displays_pane;
mod dock_pane;
mod general_pane;
mod menubar_pane;
mod network_pane;
mod notifications_pane;
mod siri_pane;
mod sound_pane;
mod vpn_pane;
mod wallpaper_pane;
mod wifi_pane;

const APP_ID: &str = "dev.malus.Settings";
pub(crate) const SETTINGS_SIDEBAR_WIDTH: i32 = 212;
const SETTINGS_RIGHT_PANE_MIN_WIDTH: i32 = 440;
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
        println!(
            "✓ Tema GTK '{}' aplicado con éxito a GTK3, GTK4 y Libadwaita.",
            theme
        );
        return;
    }

    libadwaita::StyleManager::default().set_color_scheme(if is_dark_mode_active() {
        libadwaita::ColorScheme::ForceDark
    } else {
        libadwaita::ColorScheme::ForceLight
    });

    // Escuchar reactivamente cambios en el modo de apariencia desde el Centro de Control o scripts
    let gsettings = Box::leak(Box::new(gtk4::gio::Settings::new(
        "org.gnome.desktop.interface",
    )));
    gsettings.connect_changed(Some("color-scheme"), move |settings, _| {
        let scheme = settings.string("color-scheme");
        let to_dark = scheme.contains("dark");
        tracing::info!(
            "🎨 Detección reactiva de cambio de apariencia: {} (to_dark: {})",
            scheme,
            to_dark
        );
        libadwaita::StyleManager::default().set_color_scheme(if to_dark {
            libadwaita::ColorScheme::ForceDark
        } else {
            libadwaita::ColorScheme::ForceLight
        });
    });

    let initial_panel = args.panel.clone();

    let app = Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        load_macos_settings_css();
    });

    app.connect_activate(move |app| {
        if let Some(win) = app
            .active_window()
            .or_else(|| app.windows().into_iter().next())
        {
            win.present();
            return;
        }
        build_settings_window(app, &initial_panel);
    });

    app.run_with_args::<&str>(&[]);
}

thread_local! {
    static SETTINGS_GLASS_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static SETTINGS_ACCENT_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static SETTINGS_ACCENT_DRAW_AREAS: RefCell<Vec<glib::WeakRef<DrawingArea>>> = const { RefCell::new(Vec::new()) };
}

static SETTINGS_REDUCE_TRANSPARENCY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn register_accent_draw_area(area: &DrawingArea) {
    SETTINGS_ACCENT_DRAW_AREAS.with(|areas| {
        let mut areas = areas.borrow_mut();
        areas.retain(|area| area.upgrade().is_some());
        areas.push(area.downgrade());
    });
}

pub(crate) fn accent_color() -> (f64, f64, f64) {
    malus_de_common::config::load_accent_color()
}

pub(crate) fn apply_liquid_glass_opacity_css(opacity: f64) {
    SETTINGS_REDUCE_TRANSPARENCY.store(
        malus_de_common::config::load_reduce_transparency(),
        std::sync::atomic::Ordering::Relaxed,
    );
    let scale = malus_de_common::config::liquid_glass_scale_for(opacity);
    let pop_a = malus_de_common::config::scaled_liquid_glass_alpha(0.78, scale, 0.14, 0.96);
    let side_a = malus_de_common::config::scaled_liquid_glass_alpha(0.72, scale, 0.12, 0.96);
    let side_tint = malus_de_common::config::sidebar_tint_background_image_css(
        malus_de_common::config::load_tint_windows_with_wallpaper(),
    );
    let css = format!(
        "window.macos-settings-window popover > contents, \
         window.macos-settings-window popover.menu > contents, \
         window.macos-settings-window popover.background > contents, \
         window.macos-settings-window menu, \
         window.macos-settings-window .menu {{ \
             background-color: alpha(@window_bg_color, {pop_a:.3}); \
         }} \
         .macos-sidebar {{ \
             background-color: alpha(@window_bg_color, {side_a:.3}); \
             background-image: {side_tint}; \
         }}"
    );
    SETTINGS_GLASS_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

pub(crate) fn apply_accent_color_css(index: usize) {
    SETTINGS_ACCENT_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&settings_accent_color_css(index));
        }
    });
    SETTINGS_ACCENT_DRAW_AREAS.with(|areas| {
        areas.borrow_mut().retain(|area| {
            if let Some(area) = area.upgrade() {
                area.queue_draw();
                true
            } else {
                false
            }
        });
    });
}

fn settings_accent_color_css(index: usize) -> String {
    format!(
        "{}\nimage.settings-sidebar-icon {{ color: @accent_bg_color; }}",
        malus_de_common::config::accent_color_css(index)
    )
}

fn load_macos_settings_css() {
    let provider = CssProvider::new();
    let css = include_str!("styles/macos-settings.css");
    provider.load_from_string(css);
    let glass_provider = CssProvider::new();
    let accent_provider = CssProvider::new();
    accent_provider.load_from_string(&settings_accent_color_css(
        malus_de_common::config::load_accent_color_index(),
    ));
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
    }
    SETTINGS_GLASS_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(glass_provider);
    });
    SETTINGS_ACCENT_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(accent_provider);
    });
    if let Some(display) = gtk4::gdk::Display::default() {
        malus_de_common::sidebar_style::install_sidebar_size_sync(&display);
    }
    apply_liquid_glass_opacity_css(malus_de_common::config::load_liquid_glass_opacity());
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

fn build_sidebar_nav_button(icon_name: &str, label_txt: &str) -> Button {
    let btn = Button::builder()
        .css_classes(["sidebar-btn", "malus-sidebar-row"])
        .has_frame(false)
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
        .css_classes(["settings-sidebar-icon", "malus-sidebar-icon"])
        .halign(Align::Start)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();

    let lbl = Label::builder()
        .label(label_txt)
        .css_classes(["settings-sidebar-label", "malus-sidebar-label"])
        .xalign(0.0)
        .halign(Align::Start)
        .valign(Align::Center)
        .hexpand(true)
        .vexpand(false)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    malus_de_common::sidebar_style::register_sidebar_row(&btn);
    malus_de_common::sidebar_style::register_sidebar_icon(&glyph);

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
    register_accent_draw_area(&bg);

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
                let (r, g, b) = accent_color();
                cr.set_source_rgba(r, g, b, if is_dark { 0.85 } else { 0.80 });
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
    let dummy_titlebar = GtkBox::builder().visible(false).build();
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

    let traffic_handle = WindowHandle::builder().child(&controls_box).build();
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
        .label(&format!(
            "{}{}",
            &initial_char,
            &user_name.chars().skip(1).collect::<String>()
        ))
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
        ("about", "preferences-system-symbolic", "General"),
        (
            "accessibility",
            "preferences-desktop-accessibility-symbolic",
            "Accesibilidad",
        ),
        (
            "appearance",
            "preferences-desktop-appearance-symbolic",
            "Aspecto",
        ),
        ("menubar", "control-center-symbolic", "Barra de menús"),
        ("dock", "user-desktop-symbolic", "Escritorio y Dock"),
        (
            "wallpaper",
            "preferences-desktop-wallpaper-symbolic",
            "Fondo de pantalla",
        ),
        ("displays", "video-display-symbolic", "Pantallas"),
        (
            siri_pane::SIRI_PANEL_ID,
            "audio-input-microphone-symbolic",
            siri_pane::SIRI_PANEL_TITLE,
        ),
        (
            canopy_pane::CANOPY_PANEL_ID,
            "system-search-symbolic",
            canopy_pane::CANOPY_PANEL_TITLE,
        ),
        (
            notifications_pane::NOTIFICATIONS_PANEL_ID,
            "preferences-system-notifications-symbolic",
            notifications_pane::NOTIFICATIONS_PANEL_TITLE,
        ),
        (
            sound_pane::SOUND_PANEL_ID,
            "audio-volume-high-symbolic",
            sound_pane::SOUND_PANEL_TITLE,
        ),
        ("keyboard", "input-keyboard-symbolic", "Teclado"),
    ];

    let stack = Stack::builder()
        .transition_type(StackTransitionType::Crossfade)
        .transition_duration(160)
        .hhomogeneous(false)
        .hexpand(true)
        .vexpand(true)
        .build();

    let mut sidebar_buttons: Vec<(String, Button)> = Vec::new();

    for (id, icon_name, label_txt) in nav_items {
        let btn = build_sidebar_nav_button(icon_name, label_txt);
        // Espaciado visual de separación de grupo antes de General y antes de Notificaciones
        if id == "about" || id == notifications_pane::NOTIFICATIONS_PANEL_ID {
            btn.set_margin_top(14);
        }
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
                    "menubar" => "barra de menus menubar centro de control reloj bateria spotlight bluetooth airdrop enfoque pantalla sonido clima",
                    "dock" => "escritorio dock organizador visual widgets navegador ventanas mosaico mission control esquinas activas atajos aladino",
                    "siri" => "pip siri asistente voz inteligencia chatgpt extension consulta visual sugerencias idioma respuestas",
                    "spotlight" => "canopy spotlight busqueda aplicaciones archivos carpetas portapapeles privacidad",
                    "notifications" => "notificaciones avisos tiras alertas banners sonidos previsualizar centro apps",
                    "sound" => "sonido volumen audio alerta salida entrada microfono bocinas balance silencio efectos",
                    "about" => "general acerca sistema procesador ram version informacion actualizacion almacenamiento airdrop compartir disco arranque inicio extensiones fecha hora idioma region time machine",
                    "accessibility" => "accesibilidad accessibility vision voiceover orca zoom pantalla contraste movimiento voz teclado puntero subtitulos audio",
                    _ => "",
                }.contains(&query);
                btn.set_visible(matches);
            }
        }
    });

    // ==========================================
    // 2. Contenedores de cabecera y cargador bajo demanda (lazy + caché en Stack)
    // ==========================================
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
    let nav_general: Rc<dyn Fn(&str)> = {
        let d = nav_dispatch.clone();
        Rc::new(move |target_id: &str| {
            if let Some(f) = d.borrow().as_ref() {
                f(target_id, true);
            }
        })
    };

    let header_title_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let active_pane_title = Label::builder()
        .label(canopy_pane::CANOPY_PANEL_TITLE)
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

    // Cargadores perezosos para grupos con subvistas (construyen cada página una sola vez)
    let load_general_pane =
        general_pane::create_general_pane_loader(&window, &stack, nav_general.clone());
    let load_accessibility_pane =
        accessibility_pane::create_accessibility_pane_loader(&window, &stack, nav_general.clone());
    let load_appearance_pane =
        appearance_pane::create_appearance_pane_loader(&stack, nav_general.clone());
    let (load_notifications_pane, notifications_title_fn) =
        notifications_pane::create_notifications_pane_loader(&stack, nav_general.clone());

    // Caché de callbacks de refresco por panel ya construido en el Stack
    let cached_refreshers: Rc<RefCell<std::collections::HashMap<String, Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));

    // Construye un panel solo si aún no existe en el Stack; si `refresh` es true,
    // actualiza también sus datos vivos (evitando doble refresco en constructores que ya lo hacen).
    let ensure_panel: Rc<dyn Fn(&str, bool)> = {
        let win_c = window.clone();
        let stack_c = stack.clone();
        let nav_wifi_c = nav_wifi.clone();
        let nav_fw_c = nav_fw.clone();
        let nav_gen_c = nav_general.clone();
        let bat_cb_c = battery_header_cb.clone();
        let load_gen = load_general_pane.clone();
        let load_a11y = load_accessibility_pane.clone();
        let load_app = load_appearance_pane.clone();
        let load_notif = load_notifications_pane.clone();
        let refreshers_c = cached_refreshers.clone();

        Rc::new(move |target: &str, refresh: bool| {
            if general_pane::is_general_subpanel(target) {
                if refresh || stack_c.child_by_name(target).is_none() {
                    load_gen(target);
                }
                return;
            }
            if accessibility_pane::is_accessibility_subpanel(target) {
                load_a11y(target);
                return;
            }
            if appearance_pane::is_appearance_subpanel(target) {
                if refresh || stack_c.child_by_name(target).is_none() {
                    load_app(target);
                }
                return;
            }
            if notifications_pane::is_notifications_subpanel(target) {
                if refresh || stack_c.child_by_name(target).is_none() {
                    load_notif(target);
                }
                return;
            }

            let mut refreshed_on_construct = false;
            if stack_c.child_by_name(target).is_none() {
                match target {
                    "wifi" => {
                        let (pane, rf) = wifi_pane::create_wifi_pane(&win_c);
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("wifi"));
                        refreshers_c.borrow_mut().insert("wifi".to_string(), rf);
                    }
                    "bluetooth" => {
                        let (pane, rf) = bluetooth_pane::create_bluetooth_pane(&win_c);
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("bluetooth"));
                        refreshers_c
                            .borrow_mut()
                            .insert("bluetooth".to_string(), rf);
                    }
                    "network" | "network-firewall" => {
                        let ((net_pane, ref_net), (fw_pane, ref_fw)) =
                            network_pane::create_network_panes(
                                &win_c,
                                nav_wifi_c.clone(),
                                nav_fw_c.clone(),
                            );
                        configure_right_pane_widget(net_pane.upcast_ref());
                        configure_right_pane_widget(fw_pane.upcast_ref());
                        stack_c.add_named(&net_pane, Some("network"));
                        stack_c.add_named(&fw_pane, Some("network-firewall"));
                        let mut map = refreshers_c.borrow_mut();
                        map.insert("network".to_string(), ref_net);
                        map.insert("network-firewall".to_string(), ref_fw);
                    }
                    "vpn" => {
                        let (pane, rf) = vpn_pane::create_vpn_pane(&win_c);
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("vpn"));
                        refreshers_c.borrow_mut().insert("vpn".to_string(), rf);
                        refreshed_on_construct = true;
                    }
                    "battery" => {
                        let (pane, rf) = battery_pane::create_battery_pane(&win_c, bat_cb_c.clone());
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("battery"));
                        refreshers_c.borrow_mut().insert("battery".to_string(), rf);
                        refreshed_on_construct = true;
                    }
                    "displays" => {
                        let (pane, rf) = displays_pane::create_displays_pane(&win_c);
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("displays"));
                        refreshers_c.borrow_mut().insert("displays".to_string(), rf);
                        refreshed_on_construct = true;
                    }
                    "wallpaper" => {
                        let (pane, rf) = wallpaper_pane::create_wallpaper_pane(&win_c);
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("wallpaper"));
                        refreshers_c
                            .borrow_mut()
                            .insert("wallpaper".to_string(), rf);
                        refreshed_on_construct = true;
                    }
                    "keyboard" => {
                        let (pane, rf) = create_keyboard_pane();
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("keyboard"));
                        refreshers_c.borrow_mut().insert("keyboard".to_string(), rf);
                        refreshed_on_construct = true;
                    }
                    "spotlight" => {
                        let (pane, rf) = canopy_pane::create_canopy_pane();
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("spotlight"));
                        refreshers_c
                            .borrow_mut()
                            .insert("spotlight".to_string(), rf);
                    }
                    "siri" => {
                        let pane = siri_pane::create_siri_pane(nav_gen_c.clone());
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("siri"));
                    }
                    "sound" => {
                        let (pane, rf) = sound_pane::create_sound_pane();
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("sound"));
                        refreshers_c.borrow_mut().insert("sound".to_string(), rf);
                        refreshed_on_construct = true;
                    }
                    "dock" => {
                        let (pane, rf) = dock_pane::create_dock_pane(&win_c, nav_gen_c.clone());
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("dock"));
                        refreshers_c.borrow_mut().insert("dock".to_string(), rf);
                    }
                    "menubar" => {
                        let (pane, rf) =
                            menubar_pane::create_menubar_pane(&win_c, nav_gen_c.clone());
                        configure_right_pane_widget(pane.upcast_ref());
                        stack_c.add_named(&pane, Some("menubar"));
                        refreshers_c.borrow_mut().insert("menubar".to_string(), rf);
                    }
                    _ => {}
                }
            }

            if refresh && !refreshed_on_construct {
                let rf = refreshers_c.borrow().get(target).cloned();
                if let Some(ref_fn) = rf {
                    ref_fn();
                }
            }
        })
    };

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
    window_controls_end.set_visible(!window_controls_end.is_empty());
    window_controls_end.connect_empty_notify(|wc| {
        wc.set_visible(!wc.is_empty());
    });
    header_toolbar.append(&window_controls_end);
    header_toolbar.set_margin_start(SETTINGS_SIDEBAR_WIDTH);

    let header_handle = WindowHandle::builder()
        .child(&header_toolbar)
        .hexpand(true)
        .build();
    content_area.append(&header_handle);
    content_area.append(&stack);

    configure_right_pane_stack(&stack);

    // Historial de navegación para botones atrás / adelante (estilo Branches)
    let start_id = match initial_panel {
        "red" => "network",
        "firewall" => "network-firewall",
        "bateria" | "batería" => "battery",
        "general" => "about",
        "accesibilidad" => "accessibility",
        "aspecto" => "appearance",
        "temas" => "appearance-themes",
        "pip" => "siri",
        "canopy" => "spotlight",
        "notificaciones" => "notifications",
        "barra de menus" | "barra de menús" => "menubar",
        p if general_pane::is_general_subpanel(p) => p,
        p if accessibility_pane::is_accessibility_subpanel(p) => p,
        p if appearance_pane::is_appearance_subpanel(p) => p,
        p if notifications_pane::is_notifications_subpanel(p) => p,
        "wifi" | "bluetooth" | "network" | "network-firewall" | "vpn" | "battery" | "wallpaper"
        | "keyboard" | "appearance" | "menubar" | "sound" | "about" | "dock" | "siri"
        | "spotlight" | "notifications" | "displays" => initial_panel,
        _ => "spotlight",
    };

    let all_btns = Rc::new(sidebar_buttons.clone());
    let nav_history: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![start_id.to_string()]));
    let nav_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    let current_panel: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let select_panel = {
        let btns = all_btns.clone();
        let stack_c = stack.clone();
        let header_tb_c = header_toolbar.clone();
        let title_lbl = active_pane_title.clone();
        let sub_row_c = header_sub_row.clone();
        let ensure_c = ensure_panel.clone();
        let history_c = nav_history.clone();
        let idx_c = nav_index.clone();
        let cur_panel_c = current_panel.clone();
        let btn_b_c = btn_back.clone();
        let btn_f_c = btn_forward.clone();
        let notif_title_c = notifications_title_fn.clone();

        Rc::new(move |target: &str, action: NavAction| {
            let prev_id = cur_panel_c.borrow().clone();
            let (trans_type, trans_ms) =
                choose_panel_transition(prev_id.as_deref(), target, action);
            stack_c.set_transition_type(trans_type);
            stack_c.set_transition_duration(trans_ms);
            *cur_panel_c.borrow_mut() = Some(target.to_string());

            let sidebar_id = if target == "network-firewall" {
                "network"
            } else if general_pane::is_general_subpanel(target) {
                "about"
            } else if accessibility_pane::is_accessibility_subpanel(target) {
                "accessibility"
            } else if appearance_pane::is_appearance_subpanel(target) {
                "appearance"
            } else if notifications_pane::is_notifications_subpanel(target) {
                notifications_pane::NOTIFICATIONS_PANEL_ID
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
            if target == "displays" {
                header_tb_c.add_css_class("disp-header-toolbar");
            } else {
                header_tb_c.remove_css_class("disp-header-toolbar");
            }
            ensure_c(target, true);
            let display_title = general_pane::general_subpanel_title(target)
                .map(str::to_string)
                .or_else(|| accessibility_pane::accessibility_subpanel_title(target).map(str::to_string))
                .or_else(|| appearance_pane::appearance_subpanel_title(target).map(str::to_string))
                .or_else(|| notif_title_c(target))
                .unwrap_or_else(|| {
                    match target {
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
                        "menubar" => "Barra de menús",
                        "dock" => "Escritorio y Dock",
                        "siri" => siri_pane::SIRI_PANEL_TITLE,
                        "spotlight" => canopy_pane::CANOPY_PANEL_TITLE,
                        "sound" => "Sonido",
                        _ => "Ajustes",
                    }
                    .to_string()
                });
            title_lbl.set_label(&display_title);
            stack_c.set_visible_child_name(target);

            if matches!(action, NavAction::Push) {
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
        sel_for_dispatch(
            target,
            if push {
                NavAction::Push
            } else {
                NavAction::Back
            },
        );
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
            sel_back(&target, NavAction::Back);
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
            sel_fwd(&target, NavAction::Forward);
        }
    });

    // Conectar navegación de la barra lateral
    for (id, btn) in &sidebar_buttons {
        let target_id = id.clone();
        let sel = select_panel.clone();
        btn.connect_clicked(move |_| {
            sel(&target_id, NavAction::Push);
        });
    }

    // Seleccionar panel inicial sin animación de entrada
    select_panel(start_id, NavAction::Initial);

    sidebar.set_halign(Align::Start);
    sidebar.set_valign(Align::Fill);
    sidebar.set_hexpand(false);
    sidebar.set_vexpand(true);
    sidebar.set_width_request(SETTINGS_SIDEBAR_WIDTH);

    let split_host = frosted_sidebar::SettingsSplitBox::new(&content_area, &sidebar, &stack);

    let root_overlay = Overlay::new();
    root_overlay.set_child(Some(&split_host));
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

    // Precalentamiento suave en tiempo muerto (solo mientras la ventana siga abierta):
    // construye de a una las secciones ligeras sin sondas de hardware/red para que
    // navegar por la barra lateral sea instantáneo sin penalizar el arranque inicial.
    let prewarm_queue: Rc<RefCell<std::collections::VecDeque<&'static str>>> = Rc::new(
        RefCell::new(std::collections::VecDeque::from([
            "about",
            "appearance",
            "menubar",
            "dock",
            "accessibility",
            "keyboard",
            "sound",
            "spotlight",
        ])),
    );
    let win_weak = window.downgrade();
    let stack_prewarm = stack.clone();
    let ensure_prewarm = ensure_panel.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
            let Some(win) = win_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !win.is_visible() {
                return glib::ControlFlow::Break;
            }
            while let Some(next_id) = prewarm_queue.borrow_mut().pop_front() {
                if stack_prewarm.child_by_name(next_id).is_none() {
                    ensure_prewarm(next_id, false);
                    return glib::ControlFlow::Continue;
                }
            }
            glib::ControlFlow::Break
        });
    });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavAction {
    Initial,
    Push,
    Back,
    Forward,
}

fn is_subsection_panel(id: &str) -> bool {
    id == "network-firewall"
        || id == notifications_pane::NOTIFICATIONS_APP_SUBPANEL_ID
        || (id != "about" && general_pane::is_general_subpanel(id))
        || (id != "accessibility" && accessibility_pane::is_accessibility_subpanel(id))
        || (id != "appearance" && appearance_pane::is_appearance_subpanel(id))
}

fn section_group(id: &str) -> &str {
    if id == "network" || id == "network-firewall" {
        "network"
    } else if general_pane::is_general_subpanel(id) {
        "about"
    } else if accessibility_pane::is_accessibility_subpanel(id) {
        "accessibility"
    } else if appearance_pane::is_appearance_subpanel(id) {
        "appearance"
    } else if notifications_pane::is_notifications_subpanel(id) {
        notifications_pane::NOTIFICATIONS_PANEL_ID
    } else {
        id
    }
}

fn same_section_group(a: &str, b: &str) -> bool {
    section_group(a) == section_group(b)
}

fn choose_panel_transition(
    prev_id: Option<&str>,
    next_id: &str,
    action: NavAction,
) -> (StackTransitionType, u32) {
    let Some(prev) = prev_id else {
        return (StackTransitionType::None, 0);
    };
    if prev == next_id || matches!(action, NavAction::Initial) {
        return (StackTransitionType::None, 0);
    }

    let prev_sub = is_subsection_panel(prev);
    let next_sub = is_subsection_panel(next_id);

    match action {
        NavAction::Back => {
            if prev_sub || next_sub || same_section_group(prev, next_id) {
                (StackTransitionType::SlideRight, 220)
            } else {
                (StackTransitionType::Crossfade, 140)
            }
        }
        NavAction::Forward | NavAction::Push => {
            if next_sub {
                (StackTransitionType::SlideLeft, 220)
            } else if prev_sub && same_section_group(prev, next_id) {
                (StackTransitionType::SlideRight, 220)
            } else {
                (StackTransitionType::Crossfade, 140)
            }
        }
        NavAction::Initial => (StackTransitionType::None, 0),
    }
}

/// Configures a single page widget added to the right-hand `Stack` so narrow panels
/// wrap cleanly and the right area can shrink to ~2/3 of its prior minimum width (~440 px).
pub(crate) fn configure_right_pane_widget(child: &gtk4::Widget) {
    let is_bleed = child.has_css_class("wp-bleed-pane");
    if !is_bleed {
        child.set_margin_start(SETTINGS_SIDEBAR_WIDTH);
    }
    if let Ok(sw) = child.clone().downcast::<ScrolledWindow>() {
        if !is_bleed {
            sw.set_hscrollbar_policy(PolicyType::Automatic);
        }
        sw.set_min_content_width(if is_bleed {
            SETTINGS_SIDEBAR_WIDTH + SETTINGS_RIGHT_PANE_MIN_WIDTH
        } else {
            SETTINGS_RIGHT_PANE_MIN_WIDTH
        });
    }
}

/// Configures every page in the right-hand `Stack` so narrow panels wrap cleanly
/// and the right area can shrink to ~2/3 of its prior minimum width (~440 px).
fn configure_right_pane_stack(stack: &Stack) {
    stack.set_hhomogeneous(false);
    let pages = stack.pages();
    for i in 0..pages.n_items() {
        if let Some(page) = pages
            .item(i)
            .and_then(|obj| obj.downcast::<gtk4::StackPage>().ok())
        {
            configure_right_pane_widget(&page.child());
        }
    }
}

mod frosted_sidebar {
    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;
    use std::cell::RefCell;

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct SettingsSplitBox {
            pub content: RefCell<Option<glib::WeakRef<gtk4::Widget>>>,
            pub sidebar: RefCell<Option<glib::WeakRef<gtk4::Widget>>>,
            pub stack: RefCell<Option<glib::WeakRef<gtk4::Stack>>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for SettingsSplitBox {
            const NAME: &'static str = "MalusSettingsSplitBox";
            type Type = super::SettingsSplitBox;
            type ParentType = gtk4::Widget;

            fn class_init(klass: &mut Self::Class) {
                klass.set_layout_manager_type::<gtk4::BinLayout>();
            }
        }

        impl ObjectImpl for SettingsSplitBox {
            fn dispose(&self) {
                while let Some(child) = self.obj().first_child() {
                    child.unparent();
                }
            }
        }

        impl WidgetImpl for SettingsSplitBox {
            fn snapshot(&self, snapshot: &gtk4::Snapshot) {
                let obj = self.obj();
                let w = obj.width() as f32;
                let h = obj.height() as f32;
                if w <= 1.0 || h <= 1.0 {
                    return;
                }

                let sw = (super::super::SETTINGS_SIDEBAR_WIDTH as f32).min(w);
                let content_w = self.content.borrow().as_ref().and_then(|r| r.upgrade());
                let sidebar_w = self.sidebar.borrow().as_ref().and_then(|r| r.upgrade());
                let is_bleed = self
                    .stack
                    .borrow()
                    .as_ref()
                    .and_then(|r| r.upgrade())
                    .and_then(|st| st.visible_child())
                    .map(|c| c.has_css_class("wp-bleed-pane"))
                    .unwrap_or(false);

                if let Some(ref content) = content_w {
                    let content_snap = gtk4::Snapshot::new();
                    obj.snapshot_child(content, &content_snap);
                    if let Some(content_node) = content_snap.to_node() {
                        // 1. Mitad derecha [212..w]: nítida tal cual
                        if w > sw {
                            snapshot.push_clip(&gtk4::graphene::Rect::new(sw, 0.0, w - sw, h));
                            snapshot.append_node(&content_node);
                            snapshot.pop();
                        }

                        // 2. Mitad izquierda [0..212]: solo se dibuja desenfocada (nunca nítida)
                        //    cuando el panel activo sangra bajo la sidebar (.wp-bleed-pane).
                        let reduce = super::super::SETTINGS_REDUCE_TRANSPARENCY
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if is_bleed && !reduce && sw > 0.0 {
                            let left_rect = gtk4::graphene::Rect::new(0.0, 0.0, sw, h);
                            snapshot.push_clip(&left_rect);
                            snapshot.push_blur(32.0);
                            snapshot.push_clip(&left_rect);
                            snapshot.append_node(&content_node);
                            snapshot.pop();
                            snapshot.pop();
                            snapshot.pop();
                        }
                    }
                }

                if let Some(ref sidebar) = sidebar_w {
                    obj.snapshot_child(sidebar, snapshot);
                }
            }
        }
    }

    glib::wrapper! {
        pub struct SettingsSplitBox(ObjectSubclass<imp::SettingsSplitBox>)
            @extends gtk4::Widget;
    }

    impl SettingsSplitBox {
        pub fn new(
            content_area: &impl IsA<gtk4::Widget>,
            sidebar: &impl IsA<gtk4::Widget>,
            stack: &gtk4::Stack,
        ) -> Self {
            let obj: Self = glib::Object::new();
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            content_area.set_parent(&obj);
            sidebar.set_parent(&obj);
            *obj.imp().content.borrow_mut() = Some(content_area.as_ref().downgrade());
            *obj.imp().sidebar.borrow_mut() = Some(sidebar.as_ref().downgrade());
            *obj.imp().stack.borrow_mut() = Some(stack.downgrade());
            obj
        }
    }
}

/// Builds a wrapping horizontal `FlowBox` whose items fit on one row when wide
/// and wrap cleanly onto multiple lines when the window is narrowed.
fn make_wrap_flow(
    min_per_line: u32,
    max_per_line: u32,
    spacing: i32,
    homogeneous: bool,
) -> FlowBox {
    FlowBox::builder()
        .css_classes(["settings-wrap-flow"])
        .homogeneous(homogeneous)
        .min_children_per_line(min_per_line)
        .max_children_per_line(max_per_line)
        .row_spacing(spacing as u32)
        .column_spacing(spacing as u32)
        .selection_mode(gtk4::SelectionMode::None)
        .hexpand(true)
        .valign(Align::Start)
        .build()
}

// ============================================================================
// PANEL 1: PANTALLAS -> `displays_pane.rs`
// ============================================================================

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
        .wrap(true)
        .xalign(0.0)
        .build();
    layout_card.append(&active_status_lbl);

    let layouts_row = make_wrap_flow(1, 3, 10, true);

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
            .wrap(true)
            .justify(gtk4::Justification::Center)
            .build();
        let c = Label::builder()
            .label(code_sub)
            .css_classes(["res-dim-label"])
            .wrap(true)
            .justify(gtk4::Justification::Center)
            .build();
        let d = Label::builder()
            .label(desc)
            .css_classes(["res-pct-label"])
            .wrap(true)
            .justify(gtk4::Justification::Center)
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

    layouts_row.insert(&btn_latam, -1);
    layouts_row.insert(&btn_es, -1);
    layouts_row.insert(&btn_us, -1);
    layout_card.append(&layouts_row);

    // Campo de prueba en vivo
    let test_lbl = Label::builder()
        .label("Prueba tu teclado en vivo aquí (escribe ñ, á, é, í, ó, ú, ¿, ¡, @):")
        .css_classes(["card-section-sub"])
        .wrap(true)
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
        .wrap(true)
        .hexpand(true)
        .xalign(0.0)
        .build();
    let nat_switch = Switch::builder()
        .valign(Align::Center)
        .active(InputConfig::load().natural_scrolling)
        .build();
    nat_row.append(&nat_lbl);
    nat_row.append(&*nat_switch);
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
        st_lbl.set_label(&format!(
            "Distribución activa: {}",
            cfg.layout_display_name()
        ));
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
// PANEL 3: APARIENCIA Y TEMAS (HELPERS COMPARTIDOS)
// ============================================================================

pub(crate) fn get_installed_cursor_themes() -> Vec<(String, String, &'static str)> {
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
                                "macOS" => (
                                    "macOS Original",
                                    "Flecha negra clásica con contorno blanco Apple",
                                ),
                                "macOS-White" => {
                                    ("macOS Blanco", "Variante blanca oficial de Apple")
                                }
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
        themes.push((
            "macOS".to_string(),
            "macOS Original".to_string(),
            "Flecha negra clásica con contorno blanco Apple",
        ));
        themes.push((
            "macOS-White".to_string(),
            "macOS Blanco".to_string(),
            "Variante blanca oficial de Apple",
        ));
        themes.push((
            "Adwaita".to_string(),
            "Adwaita".to_string(),
            "Tema estándar de GNOME Linux",
        ));
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

pub(crate) fn get_current_cursor_config() -> (String, u32) {
    // Lectura directa vía gio::Settings (en proceso, sin lanzar subprocesos `gsettings get`)
    if let Some(source) = gtk4::gio::SettingsSchemaSource::default() {
        if source.lookup("org.gnome.desktop.interface", true).is_some() {
            let gs = gtk4::gio::Settings::new("org.gnome.desktop.interface");
            let theme = gs.string("cursor-theme").to_string();
            let size = gs.int("cursor-size");
            let theme = if theme.trim().is_empty() {
                "macOS".to_string()
            } else {
                theme
            };
            let size = if size > 0 { size as u32 } else { 24 };
            return (theme, size);
        }
    }
    malus_de_common::config::load_cursor_config()
}

pub(crate) fn is_dark_mode_active() -> bool {
    malus_de_common::config::is_dark_mode_active()
}

pub(crate) fn get_installed_gtk_themes() -> Vec<(String, String, &'static str)> {
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

pub(crate) fn get_current_gtk_theme() -> String {
    if is_dark_mode_active() {
        "Adwaita-Dark".to_string()
    } else {
        "Adwaita-Light".to_string()
    }
}

pub(crate) fn set_gtk_theme(theme_name: &str) {
    tracing::info!("🎨 Aplicando tema GTK y controles: {}", theme_name);
    let to_dark = theme_name.contains("Dark")
        || theme_name.contains("dark")
        || (!theme_name.contains("Light") && is_dark_mode_active());
    set_system_appearance_mode(to_dark);
}

pub(crate) fn set_system_appearance_mode(to_dark: bool) {
    let target = if to_dark {
        "prefer-dark"
    } else {
        "prefer-light"
    };
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

    libadwaita::StyleManager::default().set_color_scheme(if to_dark {
        libadwaita::ColorScheme::ForceDark
    } else {
        libadwaita::ColorScheme::ForceLight
    });

    let _ = IpcClient::send_command(IpcMessage::ThemeChanged {
        theme_name: if to_dark {
            "Adwaita-Dark".to_string()
        } else {
            "Adwaita-Light".to_string()
        },
        is_dark: to_dark,
    });
}

pub(crate) fn set_cursor_theme_and_size(theme: &str, size: u32) {
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
        .args([
            "set",
            "org.gnome.desktop.interface",
            "cursor-size",
            &size.to_string(),
        ])
        .status();
    let dark_active = is_dark_mode_active();
    let dark_num = if dark_active { "1" } else { "0" };
    let cur_gtk_theme = get_current_gtk_theme();
    let icon_theme_name = if dark_active {
        malus_de_common::config::DEFAULT_ICON_THEME_DARK
    } else {
        malus_de_common::config::DEFAULT_ICON_THEME_LIGHT
    };

    // Actualizar los settings.ini aislados de GTK3/GTK4 preservando el tema GTK, fuente y doble clic.
    let dbl_action = malus_de_common::config::titlebar_double_click_gtk_action(
        malus_de_common::config::DockConfig::load_or_create_default()
            .titlebar_double_click_action,
    );
    let decoration_layout = malus_de_common::config::active_window_decoration_layout();
    for ini_path in [
        format!("{}/.config/malus-de/gtk-3.0/settings.ini", home),
        format!("{}/.config/malus-de/gtk-4.0/settings.ini", home),
    ] {
        let p = PathBuf::from(ini_path);
        let size_str = size.to_string();
        let _ = malus_de_common::config::upsert_keyfile_entries(
            &p,
            &[(
                "Settings",
                &[
                    ("gtk-decoration-layout", decoration_layout),
                    ("gtk-application-prefer-dark-theme", dark_num),
                    ("gtk-cursor-theme-name", theme),
                    ("gtk-cursor-theme-size", size_str.as_str()),
                    ("gtk-theme-name", cur_gtk_theme.as_str()),
                    ("gtk-icon-theme-name", icon_theme_name),
                    ("gtk-titlebar-double-click", dbl_action),
                ],
            )],
        );
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

// ============================================================================
// PANEL 3.5: FONDO DE PANTALLA (WALLPAPER) -> `wallpaper_pane.rs`
// ============================================================================

// ============================================================================
// PANEL 4: SONIDO -> `sound_pane.rs`
// ============================================================================

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
            super::wallpaper_pane::wallpaper_chooser_initial_folder_in(&temp_root),
            Some(wallpapers.clone())
        );

        std::fs::remove_dir_all(&wallpapers).unwrap();
        assert_eq!(
            super::wallpaper_pane::wallpaper_chooser_initial_folder_in(&temp_root),
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
        let filter = super::wallpaper_pane::build_wallpaper_file_filter();
        assert_eq!(
            filter.name().as_deref(),
            Some("Archivos de imagen (*.jpg, *.png, *.webp, *.svg)")
        );
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        let dialog = gtk4::FileDialog::builder()
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
        let nav_btn =
            build_sidebar_nav_button("preferences-desktop-appearance-symbolic", "Aspecto");
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
        assert_eq!(
            pill_min_h <= 24 && pill_nat_h <= 24,
            true,
            "La píldora + ⌄ excede altura compacta: min={pill_min_h}, nat={pill_nat_h}"
        );

        // 5. Verificar configuración compacta (~2/3) del ancho mínimo del panel derecho
        assert_eq!(SETTINGS_RIGHT_PANE_MIN_WIDTH, 440);
        let test_stack = Stack::new();
        let test_sw = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .build();
        let wide_box = GtkBox::builder().width_request(700).build();
        test_sw.set_child(Some(&wide_box));
        test_stack.add_named(&test_sw, Some("wide"));
        configure_right_pane_stack(&test_stack);
        assert!(!test_stack.is_hhomogeneous());
        assert_eq!(test_sw.hscrollbar_policy(), PolicyType::Automatic);
        assert_eq!(test_sw.min_content_width(), SETTINGS_RIGHT_PANE_MIN_WIDTH);
        let (stack_min_w, _, _, _) = test_stack.measure(Orientation::Horizontal, -1);
        assert!(
            stack_min_w <= SETTINGS_RIGHT_PANE_MIN_WIDTH + 16,
            "El ancho mínimo del Stack derecho ({stack_min_w}) supera el objetivo compacto"
        );

        // 6. Verificar carga perezosa + caché por página en los cargadores con subvistas
        let app = Application::builder()
            .application_id("org.malus_de.SettingsTestLazyLoaders")
            .build();
        let win = ApplicationWindow::builder().application(&app).build();
        let lazy_stack = Stack::new();
        let noop_nav: Rc<dyn Fn(&str)> = Rc::new(|_| {});

        let gen_loader = general_pane::create_general_pane_loader(&win, &lazy_stack, noop_nav.clone());
        let a11y_loader =
            accessibility_pane::create_accessibility_pane_loader(&win, &lazy_stack, noop_nav.clone());
        let app_loader = appearance_pane::create_appearance_pane_loader(&lazy_stack, noop_nav);
        assert_eq!(lazy_stack.pages().n_items(), 0);

        gen_loader("about");
        assert_eq!(lazy_stack.pages().n_items(), 1);
        let first_about = lazy_stack.child_by_name("about").expect("about built");
        assert_eq!(first_about.margin_start(), SETTINGS_SIDEBAR_WIDTH);
        gen_loader("about");
        assert_eq!(lazy_stack.pages().n_items(), 1);
        assert_eq!(lazy_stack.child_by_name("about").as_ref(), Some(&first_about));

        a11y_loader("accessibility");
        assert_eq!(lazy_stack.pages().n_items(), 2);
        app_loader("appearance");
        assert_eq!(lazy_stack.pages().n_items(), 3);
        // Subvistas no visitadas siguen sin construirse hasta que se abran
        assert!(lazy_stack.child_by_name("a11y-voiceover").is_none());
        assert!(lazy_stack.child_by_name("general-storage").is_none());
        assert!(lazy_stack.child_by_name("appearance-themes").is_none());
    }

    #[test]
    fn chooses_slide_and_crossfade_transitions_cleanly() {
        assert_eq!(
            choose_panel_transition(None, "about", NavAction::Initial),
            (StackTransitionType::None, 0)
        );
        assert_eq!(
            choose_panel_transition(Some("about"), "general-info", NavAction::Push),
            (StackTransitionType::SlideLeft, 220)
        );
        assert_eq!(
            choose_panel_transition(Some("network"), "network-firewall", NavAction::Push),
            (StackTransitionType::SlideLeft, 220)
        );
        assert_eq!(
            choose_panel_transition(Some("general-info"), "about", NavAction::Back),
            (StackTransitionType::SlideRight, 220)
        );
        assert_eq!(
            choose_panel_transition(Some("about"), "general-info", NavAction::Forward),
            (StackTransitionType::SlideLeft, 220)
        );
        assert_eq!(
            choose_panel_transition(Some("wifi"), "battery", NavAction::Push),
            (StackTransitionType::Crossfade, 140)
        );
    }
}
