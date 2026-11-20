use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DrawingArea, DropDown, Entry, Image, Label,
    MenuButton, Orientation, Overlay, PasswordEntry, PolicyType, Popover, ScrolledWindow,
    Spinner, StateFlags, StringList, Switch,
};

/// Con superelipse n=4.2, r=19.0 px iguala la profundidad visual de esquina (~2.9 px) de `border-radius: 10px` en el sidebar.
const WIFI_CARD_SQUIRCLE_RADIUS: f64 = 19.0;
const WIFI_BUTTON_SQUIRCLE_RADIUS: f64 = 11.5;
const WIFI_BADGE_SQUIRCLE_RADIUS: f64 = 15.0;
const WIFI_MENU_SQUIRCLE_RADIUS: f64 = 12.0;
const WIFI_MENU_ITEM_SQUIRCLE_RADIUS: f64 = 14.0;

/// Dibuja la insignia cuadrada-squircle azul de Wi-Fi estilo macOS con el glifo centrado.
fn make_squircle_wifi_badge(size: i32, icon_px: i32, radius: f64) -> Overlay {
    let overlay = Overlay::builder()
        .width_request(size)
        .height_request(size)
        .halign(Align::Center)
        .valign(Align::Start)
        .build();

    let bg = DrawingArea::new();
    bg.set_content_width(size);
    bg.set_content_height(size);
    bg.set_can_target(false);
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let grad = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
        // Colores muestreados exactamente de macOS: (66, 144, 249) -> (60, 132, 242)
        grad.add_color_stop_rgba(0.0, 0.259, 0.565, 0.976, 1.0);
        grad.add_color_stop_rgba(1.0, 0.235, 0.518, 0.949, 1.0);

        let _ = cr.save();
        cr.translate(0.5, 0.5);
        crate::squircle_path(cr, width - 1.0, height - 1.0, radius);
        let _ = cr.set_source(&grad);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let _ = cr.restore();
    });

    let icon = Image::builder()
        .icon_name("network-wireless-symbolic")
        .pixel_size(icon_px)
        .halign(Align::Center)
        .valign(Align::Center)
        .css_classes(["wifi-badge-glyph"])
        .build();

    overlay.set_child(Some(&bg));
    overlay.add_overlay(&icon);
    overlay
}

/// Envuelve un contenedor tipo tarjeta con fondo gris claro y borde Squircle en Cairo.
/// El radio por defecto es 10.0px (idéntico al radio de las secciones y filas de la barra lateral).
fn wrap_squircle_card(inner: &GtkBox, radius: f64) -> Overlay {
    let overlay = Overlay::builder()
        .hexpand(true)
        .vexpand(false)
        .valign(Align::Start)
        .build();
    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(false);
    bg.set_valign(Align::Fill);
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let dark = libadwaita::StyleManager::default().is_dark();
        let _ = cr.save();
        cr.translate(0.5, 0.5);
        crate::squircle_path(cr, width - 1.0, height - 1.0, radius);
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.055);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.036);
        }
        let _ = cr.fill_preserve();
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.095);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.075);
        }
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let _ = cr.restore();
    });

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    inner.set_vexpand(false);
    inner.set_valign(Align::Start);
    overlay.set_child(Some(&bg));
    overlay.add_overlay(inner);
    overlay.set_measure_overlay(inner, true);
    overlay
}

/// Crea un botón estilo macOS compacto y ajustado al texto con forma cápsula/squircle.
fn make_squircle_button(label: &str, primary: bool, tooltip: Option<&str>) -> (Overlay, Button) {
    let btn = Button::builder()
        .label(label)
        .has_frame(false)
        .hexpand(false)
        .vexpand(false)
        .halign(Align::Center)
        .valign(Align::Center)
        .css_classes([if primary {
            "wifi-squircle-btn-primary"
        } else {
            "wifi-squircle-btn"
        }])
        .build();
    if let Some(tip) = tooltip {
        btn.set_tooltip_text(Some(tip));
    }

    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let flags = btn_weak
            .upgrade()
            .map(|b| b.state_flags())
            .unwrap_or(StateFlags::empty());
        let hovered = flags.contains(StateFlags::PRELIGHT);
        let active = flags.contains(StateFlags::ACTIVE);
        let insensitive = flags.contains(StateFlags::INSENSITIVE);
        let dark = libadwaita::StyleManager::default().is_dark();

        let _ = cr.save();
        cr.translate(0.5, 0.5);
        let pill_r = ((height - 1.0) * 0.48).max(WIFI_BUTTON_SQUIRCLE_RADIUS);
        crate::squircle_path(cr, width - 1.0, height - 1.0, pill_r);
        if primary {
            let alpha = if insensitive { 0.45 } else { 1.0 };
            if active {
                cr.set_source_rgba(0.0, 0.38, 0.80, alpha);
            } else if hovered {
                cr.set_source_rgba(0.04, 0.52, 1.0, alpha);
            } else {
                cr.set_source_rgba(0.0, 0.48, 1.0, alpha);
            }
            let _ = cr.fill_preserve();
            cr.set_source_rgba(0.0, 0.25, 0.65, 0.25 * alpha);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        } else {
            if dark {
                if active {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                } else if hovered {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
                } else {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
                }
            } else if active {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.16);
            } else if hovered {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.11);
            } else {
                // macOS usa gris translúcido rgba(0,0,0,0.075):
                // En fondo blanco (255) genera exactamente (236, 236, 236) [#ECECEC]
                // En fondo de tarjeta (247) genera exactamente (228, 228, 228) [#E4E4E4]
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.075);
            }
            let _ = cr.fill_preserve();
            if dark {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
            } else if active || hovered {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.09);
            } else {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.065);
            }
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let bg_theme = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_theme.upgrade() {
            area.queue_draw();
        }
    });

    let overlay = Overlay::builder()
        .hexpand(false)
        .vexpand(false)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&btn);
    overlay.set_measure_overlay(&btn, true);
    (overlay, btn)
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum RoundControlStyle {
    /// Botón redondo sólido estilo macOS (ej. botón '?' con relleno gris #ECECEC / rgba(0,0,0,0.075))
    SolidButton,
    /// Botón circular con contorno (ej. botón '⋯' de fila en macOS: borde circular sutil y fondo al pasar el cursor)
    CircleOutline,
    /// Botón con fondo transparente en reposo que se resalta solo al pasar el cursor (ej. botón '🔄' de refrescar)
    SubtleHover,
}

/// Envuelve botones compactos (`⋯`, `?`, recargar) con un Squircle redondo con los estilos muestreados de macOS.
fn wrap_squircle_round_control<W: gtk4::prelude::IsA<gtk4::Widget>>(
    widget: &W,
    size: i32,
    style: RoundControlStyle,
) -> Overlay {
    let w_ref = widget.as_ref();
    let overlay = Overlay::builder()
        .width_request(size)
        .height_request(size)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    let bg = DrawingArea::new();
    bg.set_content_width(size);
    bg.set_content_height(size);
    bg.set_can_target(false);

    let w_weak = w_ref.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let flags = w_weak
            .upgrade()
            .map(|x| x.state_flags())
            .unwrap_or(StateFlags::empty());
        let hovered = flags.contains(StateFlags::PRELIGHT);
        let active = flags.contains(StateFlags::ACTIVE);
        let dark = libadwaita::StyleManager::default().is_dark();
        let _ = cr.save();
        if matches!(
            style,
            RoundControlStyle::CircleOutline | RoundControlStyle::SolidButton
        ) {
            let cx = width * 0.5;
            let cy = height * 0.5;
            let circle_r = (width.min(height) - 1.0) * 0.5;
            cr.new_sub_path();
            cr.arc(cx, cy, circle_r, 0.0, std::f64::consts::TAU);
            cr.close_path();
        } else {
            cr.translate(0.5, 0.5);
            let radius = (width.min(height) - 1.0) * 0.48;
            crate::squircle_path(cr, width - 1.0, height - 1.0, radius);
        }

        match style {
            RoundControlStyle::SolidButton => {
                // Estilo botón '?' de macOS: fondo gris (236, 236, 236) idéntico a Avanzado...
                if dark {
                    if active {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                    } else if hovered {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
                    } else {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
                    }
                } else if active {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.16);
                } else if hovered {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.11);
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.075);
                }
                let _ = cr.fill_preserve();
                if dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
                } else if active || hovered {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.09);
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.065);
                }
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
            RoundControlStyle::CircleOutline => {
                // Estilo botón '⋯' de macOS: contorno circular visible en reposo
                if hovered || active {
                    if dark {
                        cr.set_source_rgba(1.0, 1.0, 1.0, if active { 0.20 } else { 0.14 });
                    } else {
                        cr.set_source_rgba(0.0, 0.0, 0.0, if active { 0.14 } else { 0.08 });
                    }
                    let _ = cr.fill_preserve();
                }
                if dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, if active { 0.30 } else { 0.22 });
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, if active { 0.26 } else { 0.18 });
                }
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
            RoundControlStyle::SubtleHover => {
                // Estilo botón '🔄': transparente en reposo, resaltado al interactuar
                if hovered || active {
                    if dark {
                        cr.set_source_rgba(1.0, 1.0, 1.0, if active { 0.20 } else { 0.14 });
                    } else {
                        cr.set_source_rgba(0.0, 0.0, 0.0, if active { 0.14 } else { 0.075 });
                    }
                    let _ = cr.fill_preserve();
                    if dark {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
                    } else {
                        cr.set_source_rgba(0.0, 0.0, 0.0, 0.09);
                    }
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
            }
        }
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    w_ref.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let bg_theme = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_theme.upgrade() {
            area.queue_draw();
        }
    });

    overlay.set_child(Some(&bg));
    overlay.add_overlay(w_ref);
    overlay.set_measure_overlay(w_ref, true);
    overlay
}
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiPreferences {
    pub ask_to_join: u32,
    pub ask_before_hotspot: u32,
}

impl Default for WifiPreferences {
    fn default() -> Self {
        Self {
            ask_to_join: 0,        // Notificar
            ask_before_hotspot: 0, // Preguntar al conectar
        }
    }
}

impl WifiPreferences {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/wifi-settings.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(parsed) = toml::from_str::<Self>(&raw) {
                return parsed;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(raw) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, raw);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWifiProfile {
    pub name: String,
    pub autoconnect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedWifiAp {
    pub in_use: bool,
    pub ssid: String,
    pub bssid: String,
    pub mode: String,
    pub channel: String,
    pub freq: String,
    pub rate: String,
    pub signal: u8,
    pub security: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiDisplayNetwork {
    pub ssid: String,
    pub known: bool,
    pub connected: bool,
    pub autoconnect: bool,
    pub in_range: bool,
    pub signal: u8,
    pub security: String,
    pub bssid: String,
    pub channel: String,
    pub freq: String,
    pub rate: String,
}

impl WifiDisplayNetwork {
    pub fn is_secured(&self) -> bool {
        let s = self.security.trim();
        !s.is_empty() && s != "--"
    }
}

#[derive(Debug, Clone, Default)]
pub struct WifiInterfaceDetails {
    pub device: String,
    pub hwaddr: String,
    pub state: String,
    pub connection: String,
    pub ip4_addresses: Vec<String>,
    pub ip4_gateway: String,
    pub ip4_dns: Vec<String>,
    pub ip6_addresses: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WifiSnapshot {
    pub radio_enabled: bool,
    pub interface_name: Option<String>,
    pub interface_state: String,
    pub active_connection: Option<String>,
    pub known_networks: Vec<WifiDisplayNetwork>,
    pub other_networks: Vec<WifiDisplayNetwork>,
    pub details: WifiInterfaceDetails,
    pub nm_available: bool,
}

pub fn create_wifi_pane(parent_window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let status_banner = Label::builder()
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .valign(Align::Start)
        .vexpand(false)
        .build();

    // =========================================================================
    // 1. Tarjeta Superior: Cabecera Wi-Fi + Red conectada actual
    // =========================================================================
    let top_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let header_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-header-section"])
        .build();

    let icon_badge = make_squircle_wifi_badge(34, 18, WIFI_BADGE_SQUIRCLE_RADIUS);

    let header_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let wifi_title = Label::builder()
        .label("Wi-Fi")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let desc_lbl = Label::builder()
        .label("Configura la red Wi-Fi para conectar tu equipo de manera inalámbrica a Internet. Activa Wi-Fi y luego selecciona una red para conectarte a ella.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    let get_details_link = Button::builder()
        .label("Obtén detalles...")
        .css_classes(["wifi-link-btn"])
        .has_frame(false)
        .halign(Align::Start)
        .build();

    header_text.append(&wifi_title);
    header_text.append(&desc_lbl);
    header_text.append(&get_details_link);

    let wifi_switch = Switch::builder()
        .valign(Align::Start)
        .css_classes(["macos-switch"])
        .build();

    header_row.append(&icon_badge);
    header_row.append(&header_text);
    header_row.append(&wifi_switch);
    top_card.append(&header_row);

    let top_sep = GtkBox::builder()
        .css_classes(["wifi-row-separator"])
        .build();
    top_card.append(&top_sep);

    // Fila de red conectada actual
    let active_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-active-section"])
        .build();

    let active_info_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();

    let active_ssid_lbl = Label::builder()
        .label("Buscando red Wi-Fi...")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let status_line = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let status_dot = Label::builder()
        .label("●")
        .css_classes(["wifi-status-dot-disconnected"])
        .build();
    let status_text = Label::builder()
        .label("Consultando estado...")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    status_line.append(&status_dot);
    status_line.append(&status_text);

    let security_warn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .visible(false)
        .build();
    let warn_icon = Label::builder()
        .label("⚠")
        .css_classes(["wifi-warn-label"])
        .build();
    let warn_lbl = Label::builder()
        .label("Advertencia de seguridad")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    security_warn_row.append(&warn_icon);
    security_warn_row.append(&warn_lbl);

    active_info_col.append(&active_ssid_lbl);
    active_info_col.append(&status_line);
    active_info_col.append(&security_warn_row);

    let active_right_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .valign(Align::Center)
        .build();

    let active_lock_icon = Image::builder()
        .icon_name("channel-secure-symbolic")
        .pixel_size(14)
        .visible(false)
        .build();
    let active_signal_icon = Image::builder()
        .icon_name("network-wireless-signal-excellent-symbolic")
        .pixel_size(15)
        .visible(false)
        .build();
    let (active_details_wrap, active_details_btn) = make_squircle_button("Detalles...", false, None);

    active_right_box.append(&active_lock_icon);
    active_right_box.append(&active_signal_icon);
    active_right_box.append(&active_details_wrap);

    active_row.append(&active_info_col);
    active_row.append(&active_right_box);
    top_card.append(&active_row);

    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&status_banner);

    // =========================================================================
    // 2. Sección: Redes conocidas
    // =========================================================================
    let known_section_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let known_title = Label::builder()
        .label("Redes conocidas")
        .css_classes(["wifi-section-title"])
        .xalign(0.0)
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let known_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();
    known_section_box.append(&known_title);
    known_section_box.append(&wrap_squircle_card(&known_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&known_section_box);

    // =========================================================================
    // 3. Sección: Otras redes + Botón "Otra..."
    // =========================================================================
    let others_section_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let others_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();
    let others_title = Label::builder()
        .label("Otras redes")
        .css_classes(["wifi-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .valign(Align::Center)
        .build();
    let scan_spinner = Spinner::builder()
        .spinning(false)
        .visible(false)
        .valign(Align::Center)
        .build();
    let rescan_btn = Button::builder()
        .icon_name("view-refresh-symbolic")
        .css_classes(["wifi-more-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Buscar redes cercanas nuevamente")
        .build();
    let rescan_wrap = wrap_squircle_round_control(&rescan_btn, 24, RoundControlStyle::SubtleHover);
    others_hdr.append(&others_title);
    others_hdr.append(&scan_spinner);
    others_hdr.append(&rescan_wrap);

    let others_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let other_join_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .valign(Align::Start)
        .vexpand(false)
        .margin_top(2)
        .build();
    let (other_network_wrap, other_network_btn) = make_squircle_button(
        "Otra...",
        false,
        Some("Conectarse manualmente a una red oculta"),
    );
    other_join_row.append(&other_network_wrap);

    others_section_box.append(&others_hdr);
    others_section_box.append(&wrap_squircle_card(&others_card, WIFI_CARD_SQUIRCLE_RADIUS));
    others_section_box.append(&other_join_row);
    pane.append(&others_section_box);

    // =========================================================================
    // 4. Tarjeta Inferior: Preferencias de conexión y Hotspot + Avanzado / ?
    // =========================================================================
    let prefs = Rc::new(RefCell::new(WifiPreferences::load()));
    let prefs_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let ask_join_row = build_preference_dropdown_row(
        "Preguntar al conectar",
        "Se accederá automáticamente a las redes conocidas. Si no hay ninguna red conocida disponible, se te avisará de las que lo estén.",
        &["Notificar", "Preguntar al conectar", "Desactivado"],
        prefs.borrow().ask_to_join,
    );
    let ask_join_dd = ask_join_row.1;
    let prefs_c1 = prefs.clone();
    ask_join_dd.connect_selected_notify(move |dd| {
        let mut p = prefs_c1.borrow_mut();
        p.ask_to_join = dd.selected();
        p.save();
    });

    let prefs_sep = GtkBox::builder()
        .css_classes(["wifi-row-separator"])
        .build();

    let ask_hotspot_row = build_preference_dropdown_row(
        "Preguntar antes de conectarse",
        "Permite a este equipo detectar automáticamente puntos de conexión que comparten Internet cuando no haya redes Wi-Fi disponibles.",
        &["Preguntar al conectar", "Automático", "Nunca"],
        prefs.borrow().ask_before_hotspot,
    );
    let ask_hotspot_dd = ask_hotspot_row.1;
    let prefs_c2 = prefs.clone();
    ask_hotspot_dd.connect_selected_notify(move |dd| {
        let mut p = prefs_c2.borrow_mut();
        p.ask_before_hotspot = dd.selected();
        p.save();
    });

    prefs_card.append(&ask_join_row.0);
    prefs_card.append(&prefs_sep);
    prefs_card.append(&ask_hotspot_row.0);
    pane.append(&wrap_squircle_card(&prefs_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer_actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let (advanced_wrap, advanced_btn) = make_squircle_button("Avanzado...", false, None);
    let help_lbl = Label::builder()
        .label("?")
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let help_btn = MenuButton::builder()
        .child(&help_lbl)
        .css_classes(["wifi-help-btn"])
        .has_frame(false)
        .always_show_arrow(false)
        .tooltip_text("Ayuda de Wi-Fi")
        .build();
    let help_wrap = wrap_squircle_round_control(&help_btn, 22, RoundControlStyle::SolidButton);

    footer_actions.append(&advanced_wrap);
    footer_actions.append(&help_wrap);
    pane.append(&footer_actions);

    scrolled.set_child(Some(&pane));

    // =========================================================================
    // Estado compartido y refresco asíncrono vía nmcli
    // =========================================================================
    let latest_snapshot: Rc<RefCell<Option<WifiSnapshot>>> = Rc::new(RefCell::new(None));
    let is_syncing_switch = Rc::new(RefCell::new(false));
    let refresh_gen = Rc::new(Cell::new(0u64));
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>> = Rc::new(RefCell::new(None));

    let win_ref = parent_window.clone();
    let snap_ref = latest_snapshot.clone();
    let sync_sw_ref = is_syncing_switch.clone();
    let gen_ref = refresh_gen.clone();
    let sw_ref = wifi_switch.clone();
    let active_ssid_ref = active_ssid_lbl.clone();
    let status_dot_ref = status_dot.clone();
    let status_txt_ref = status_text.clone();
    let warn_row_ref = security_warn_row.clone();
    let warn_lbl_ref = warn_lbl.clone();
    let lock_ico_ref = active_lock_icon.clone();
    let sig_ico_ref = active_signal_icon.clone();
    let det_btn_ref = active_details_btn.clone();
    let known_sec_ref = known_section_box.clone();
    let known_card_ref = known_card.clone();
    let others_sec_ref = others_section_box.clone();
    let others_card_ref = others_card.clone();
    let spinner_ref = scan_spinner.clone();
    let banner_ref = status_banner.clone();
    let refresh_slot_inner = refresh_slot.clone();

    let run_refresh: Rc<dyn Fn(bool)> = Rc::new(move |rescan: bool| {
        let my_gen = gen_ref.get().wrapping_add(1);
        gen_ref.set(my_gen);

        if rescan {
            spinner_ref.set_visible(true);
            spinner_ref.set_spinning(true);
        }

        let (tx, rx) = async_channel::bounded::<(WifiSnapshot, bool)>(2);
        std::thread::spawn(move || {
            // Fase 1 inmediata (<50ms): muestra redes en caché sin esperar bloqueo de hardware
            let fast_snap = collect_wifi_snapshot(false);
            let want_rescan = rescan && fast_snap.nm_available && fast_snap.radio_enabled;
            let _ = tx.send_blocking((fast_snap, !want_rescan));
            // Fase 2 opcional: dispara rescan en vivo y devuelve resultados actualizados
            if want_rescan {
                let _ = run_nmcli(&["device", "wifi", "rescan"]);
                std::thread::sleep(std::time::Duration::from_millis(1800));
                let fresh_snap = collect_wifi_snapshot(false);
                let _ = tx.send_blocking((fresh_snap, true));
            }
        });

        let snap_dest = snap_ref.clone();
        let sync_sw = sync_sw_ref.clone();
        let gen_check = gen_ref.clone();
        let sw = sw_ref.clone();
        let ssid_lbl = active_ssid_ref.clone();
        let dot = status_dot_ref.clone();
        let st_txt = status_txt_ref.clone();
        let warn_row = warn_row_ref.clone();
        let warn_text = warn_lbl_ref.clone();
        let lock_ico = lock_ico_ref.clone();
        let sig_ico = sig_ico_ref.clone();
        let det_btn = det_btn_ref.clone();
        let known_sec = known_sec_ref.clone();
        let known_c = known_card_ref.clone();
        let others_sec = others_sec_ref.clone();
        let others_c = others_card_ref.clone();
        let sp = spinner_ref.clone();
        let banner = banner_ref.clone();
        let win = win_ref.clone();
        let slot = refresh_slot_inner.clone();

        glib::spawn_future_local(async move {
            while let Ok((snap, is_final)) = rx.recv().await {
                if gen_check.get() != my_gen {
                    if is_final {
                        break;
                    }
                    continue;
                }
                if is_final {
                    sp.set_spinning(false);
                    sp.set_visible(false);
                }

                *snap_dest.borrow_mut() = Some(snap.clone());

                *sync_sw.borrow_mut() = true;
                sw.set_active(snap.radio_enabled);
                sw.set_state(snap.radio_enabled);
                *sync_sw.borrow_mut() = false;

                if !snap.nm_available {
                    ssid_lbl.set_label("NetworkManager no disponible");
                    set_status_dot(&dot, "disconnected");
                    st_txt.set_label("No se pudo consultar nmcli");
                    warn_row.set_visible(false);
                    lock_ico.set_visible(false);
                    sig_ico.set_visible(false);
                    det_btn.set_sensitive(false);
                    known_sec.set_visible(false);
                    others_sec.set_visible(false);
                    banner.set_visible(false);
                    sp.set_spinning(false);
                    sp.set_visible(false);
                    break;
                }

                if !snap.radio_enabled {
                    ssid_lbl.set_label("Wi-Fi desactivado");
                    set_status_dot(&dot, "disconnected");
                    st_txt.set_label("Activa el interruptor Wi-Fi para ver redes cercanas");
                    warn_row.set_visible(false);
                    lock_ico.set_visible(false);
                    sig_ico.set_visible(false);
                    det_btn.set_sensitive(snap.interface_name.is_some());
                    known_sec.set_visible(!snap.known_networks.is_empty());
                    others_sec.set_visible(false);
                } else if let Some(active_net) = snap.known_networks.iter().find(|n| n.connected) {
                    ssid_lbl.set_label(&active_net.ssid);
                    set_status_dot(&dot, "connected");
                    let sub_info = if !active_net.freq.is_empty() && !active_net.rate.is_empty() {
                        format!(
                            "Con conexión · {} · {}",
                            active_net.freq, active_net.rate
                        )
                    } else {
                        "Con conexión".to_string()
                    };
                    st_txt.set_label(&sub_info);

                    if !active_net.is_secured() && active_net.in_range {
                        warn_text.set_label("Red abierta (sin cifrado)");
                        warn_row.set_visible(true);
                    } else if active_net.security.to_uppercase().contains("WEP") {
                        warn_text.set_label("Seguridad débil (WEP)");
                        warn_row.set_visible(true);
                    } else {
                        warn_row.set_visible(false);
                    }

                    lock_ico.set_visible(active_net.is_secured());
                    sig_ico.set_icon_name(Some(wifi_signal_icon_name(active_net.signal)));
                    sig_ico.set_visible(true);
                    det_btn.set_sensitive(true);
                    known_sec.set_visible(true);
                    others_sec.set_visible(true);
                } else {
                    ssid_lbl.set_label(
                        snap.active_connection
                            .as_deref()
                            .unwrap_or("Sin conexión"),
                    );
                    let connecting = snap.interface_state.contains("connecting");
                    set_status_dot(&dot, if connecting { "connecting" } else { "disconnected" });
                    st_txt.set_label(if connecting {
                        "Conectando..."
                    } else {
                        "Selecciona una red de la lista inferior para conectarte"
                    });
                    warn_row.set_visible(false);
                    lock_ico.set_visible(false);
                    sig_ico.set_visible(false);
                    det_btn.set_sensitive(snap.interface_name.is_some());
                    known_sec.set_visible(!snap.known_networks.is_empty());
                    others_sec.set_visible(true);
                }

                let trigger_reload: Rc<dyn Fn(bool)> = {
                    let slot_c = slot.clone();
                    Rc::new(move |r| {
                        if let Some(ref f) = *slot_c.borrow() {
                            f(r);
                        }
                    })
                };

                populate_wifi_list_card(
                    &known_c,
                    &snap.known_networks,
                    true,
                    &win,
                    &banner,
                    &snap,
                    &trigger_reload,
                );
                populate_wifi_list_card(
                    &others_c,
                    &snap.other_networks,
                    false,
                    &win,
                    &banner,
                    &snap,
                    &trigger_reload,
                );

                if is_final {
                    break;
                }
            }
        });
    });

    *refresh_slot.borrow_mut() = Some(run_refresh.clone());

    let ref_rescan = run_refresh.clone();
    rescan_btn.connect_clicked(move |_| {
        ref_rescan(true);
    });

    // Conectar Switch de Wi-Fi
    let sync_sw_ev = is_syncing_switch.clone();
    let ref_sw = run_refresh.clone();
    let banner_sw = status_banner.clone();
    wifi_switch.connect_state_set(move |_, state| {
        if *sync_sw_ev.borrow() {
            return glib::Propagation::Proceed;
        }
        banner_sw.set_label(if state {
            "Activando adaptador Wi-Fi..."
        } else {
            "Desactivando adaptador Wi-Fi..."
        });
        banner_sw.set_visible(true);
        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let res = run_nmcli(&["radio", "wifi", if state { "on" } else { "off" }]).map(|_| ());
            let _ = tx.send_blocking(res);
        });
        let ref_after = ref_sw.clone();
        let b_after = banner_sw.clone();
        glib::spawn_future_local(async move {
            if let Ok(Err(err)) = rx.recv().await {
                b_after.set_label(&format!("No se pudo cambiar Wi-Fi: {}", err));
                b_after.set_visible(true);
            } else {
                b_after.set_visible(false);
            }
            ref_after(state);
        });
        glib::Propagation::Proceed
    });

    // Enlace "Obtén detalles..." y botón "Detalles..."
    let win_gd = parent_window.clone();
    let snap_gd = latest_snapshot.clone();
    get_details_link.connect_clicked(move |_| {
        if let Some(ref snap) = *snap_gd.borrow() {
            show_adapter_info_dialog(&win_gd, snap);
        }
    });

    let win_det = parent_window.clone();
    let snap_det = latest_snapshot.clone();
    let ref_det = run_refresh.clone();
    let ban_det = status_banner.clone();
    active_details_btn.connect_clicked(move |_| {
        if let Some(ref snap) = *snap_det.borrow() {
            let active_net = snap.known_networks.iter().find(|n| n.connected).cloned();
            show_network_details_dialog(&win_det, snap, active_net.as_ref(), &ban_det, &ref_det);
        }
    });

    // Botón "Otra..." (red oculta)
    let win_other = parent_window.clone();
    let ban_other = status_banner.clone();
    let ref_other = run_refresh.clone();
    other_network_btn.connect_clicked(move |_| {
        show_hidden_network_dialog(&win_other, &ban_other, &ref_other);
    });

    // Botón "Avanzado..."
    let win_adv = parent_window.clone();
    let snap_adv = latest_snapshot.clone();
    let ban_adv = status_banner.clone();
    let ref_adv = run_refresh.clone();
    advanced_btn.connect_clicked(move |_| {
        if let Some(ref snap) = *snap_adv.borrow() {
            show_advanced_dialog(&win_adv, snap, &ban_adv, &ref_adv);
        }
    });

    // Botón "?"
    let help_popover = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    let help_pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .build();
    let help_lbl = Label::builder()
        .label("• Haz clic en cualquier red conocida para alternar conexión.\n• Pulsa ⋯ en cada red para ver detalles u olvidar su perfil.\n• En tarjetas Intel AX210, activar AirDrop pausa temporalmente el Wi-Fi mientras dura la transferencia.")
        .wrap(true)
        .max_width_chars(38)
        .xalign(0.0)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    help_pop_box.append(&help_lbl);
    help_popover.set_child(Some(&wrap_squircle_popover_card(&help_pop_box, WIFI_MENU_SQUIRCLE_RADIUS)));
    help_btn.set_popover(Some(&help_popover));

    // Refresco periódico mientras el panel esté visible
    let pane_timer = pane.clone();
    let ref_timer = run_refresh.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(10), move || {
        if pane_timer.is_mapped() {
            ref_timer(false);
        }
        glib::ControlFlow::Continue
    });

    let public_refresh: Rc<dyn Fn()> = Rc::new(move || {
        run_refresh(true);
    });

    (scrolled, public_refresh)
}

fn build_preference_dropdown_row(
    title: &str,
    subtitle: &str,
    options: &[&str],
    selected: u32,
) -> (GtkBox, DropDown) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let t_lbl = Label::builder()
        .label(title)
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let s_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();
    text_col.append(&t_lbl);
    text_col.append(&s_lbl);

    let model = StringList::new(options);
    let dd = DropDown::builder()
        .model(&model)
        .selected(selected.min(options.len().saturating_sub(1) as u32))
        .valign(Align::Start)
        .css_classes(["wifi-compact-dropdown"])
        .build();

    row.append(&text_col);
    row.append(&dd);
    (row, dd)
}

fn set_status_dot(dot: &Label, state: &str) {
    dot.remove_css_class("wifi-status-dot-connected");
    dot.remove_css_class("wifi-status-dot-connecting");
    dot.remove_css_class("wifi-status-dot-disconnected");
    match state {
        "connected" => dot.add_css_class("wifi-status-dot-connected"),
        "connecting" => dot.add_css_class("wifi-status-dot-connecting"),
        _ => dot.add_css_class("wifi-status-dot-disconnected"),
    }
}

fn populate_wifi_list_card(
    card: &GtkBox,
    networks: &[WifiDisplayNetwork],
    is_known_section: bool,
    parent_window: &ApplicationWindow,
    banner: &Label,
    snapshot: &WifiSnapshot,
    trigger_reload: &Rc<dyn Fn(bool)>,
) {
    while let Some(child) = card.first_child() {
        card.remove(&child);
    }

    if networks.is_empty() {
        let empty = Label::builder()
            .label(if is_known_section {
                "No hay redes Wi-Fi guardadas."
            } else if snapshot.radio_enabled {
                "No se encontraron otras redes Wi-Fi cercanas."
            } else {
                "Wi-Fi desactivado."
            })
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(14)
            .margin_end(14)
            .build();
        card.append(&empty);
        return;
    }

    for (idx, net) in networks.iter().enumerate() {
        if idx > 0 {
            let sep = GtkBox::builder()
                .css_classes(["wifi-row-separator"])
                .build();
            card.append(&sep);
        }

        let row_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .css_classes(["wifi-network-row"])
            .build();

        let connect_btn = Button::builder()
            .has_frame(false)
            .hexpand(true)
            .css_classes(["wifi-row-btn"])
            .build();
        connect_btn.set_margin_top(0);
        connect_btn.set_margin_bottom(0);

        let left_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .hexpand(true)
            .valign(Align::Center)
            .build();

        let check_lbl = Label::builder()
            .label(if net.connected { "✓" } else { "" })
            .css_classes(["wifi-row-check"])
            .xalign(0.5)
            .build();

        let name_lbl = Label::builder()
            .label(&net.ssid)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        left_inner.append(&check_lbl);
        left_inner.append(&name_lbl);
        connect_btn.set_child(Some(&left_inner));

        let right_icons = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_end(2)
            .valign(Align::Center)
            .build();

        // Botón "Conectar" que aparece al pasar el cursor sobre redes desconectadas (estilo macOS)
        let hover_conn_btn = if !net.connected {
            let (wrap, btn) = make_squircle_button("Conectar", false, None);
            wrap.set_visible(false);
            right_icons.append(&wrap);

            let motion = gtk4::EventControllerMotion::new();
            let wrap_enter = wrap.clone();
            motion.connect_enter(move |_, _, _| {
                wrap_enter.set_visible(true);
            });
            let wrap_leave = wrap.clone();
            motion.connect_leave(move |_| {
                wrap_leave.set_visible(false);
            });
            row_box.add_controller(motion);
            Some(btn)
        } else {
            None
        };

        if net.is_secured() || (net.known && !net.in_range) {
            let lock_ico = Image::builder()
                .icon_name("channel-secure-symbolic")
                .pixel_size(14)
                .build();
            right_icons.append(&lock_ico);
        }

        if net.in_range || net.connected {
            let sig_ico = Image::builder()
                .icon_name(wifi_signal_icon_name(net.signal))
                .pixel_size(15)
                .build();
            right_icons.append(&sig_ico);
        }

        // Botón "⋯": menú popover en Redes conocidas; diálogo Dirección Wi-Fi privada en Otras redes
        if is_known_section {
            let more_btn = MenuButton::builder()
                .icon_name("view-more-symbolic")
                .css_classes(["wifi-more-btn"])
                .has_frame(false)
                .always_show_arrow(false)
                .valign(Align::Center)
                .tooltip_text("Opciones de red")
                .build();
            let more_wrap = wrap_squircle_round_control(&more_btn, 20, RoundControlStyle::CircleOutline);
            right_icons.append(&more_wrap);

            let popover = Popover::builder()
                .has_arrow(false)
                .css_classes(["wifi-macos-popover"])
                .build();
            let menu_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();

            let (auto_wrap, auto_btn) = make_popover_menu_row(
                if net.autoconnect { Some("✓") } else { None },
                "Conexión automática",
            );
            let pop_c = popover.clone();
            let ssid_c = net.ssid.clone();
            let next_val = if net.autoconnect { "no" } else { "yes" };
            let rel_c = trigger_reload.clone();
            auto_btn.connect_clicked(move |_| {
                pop_c.popdown();
                let s = ssid_c.clone();
                let r = rel_c.clone();
                let (tx, rx) = async_channel::bounded::<()>(1);
                std::thread::spawn(move || {
                    let _ = run_nmcli(&["connection", "modify", "id", &s, "connection.autoconnect", next_val]);
                    let _ = tx.send_blocking(());
                });
                glib::spawn_future_local(async move {
                    let _ = rx.recv().await;
                    r(false);
                });
            });
            menu_box.append(&auto_wrap);

            let (copy_pw_wrap, copy_pw_btn) = make_popover_menu_row(None, "Copiar contraseña");
            copy_pw_btn.set_sensitive(net.is_secured());
            let pop_c = popover.clone();
            let ssid_c = net.ssid.clone();
            let ban_c = banner.clone();
            copy_pw_btn.connect_clicked(move |_| {
                pop_c.popdown();
                let s = ssid_c.clone();
                let b = ban_c.clone();
                let (tx, rx) = async_channel::bounded::<Result<String, String>>(1);
                std::thread::spawn(move || {
                    let res = run_nmcli(&[
                        "-s",
                        "-g",
                        "802-11-wireless-security.psk",
                        "connection",
                        "show",
                        "id",
                        &s,
                    ]);
                    let _ = tx.send_blocking(res);
                });
                glib::spawn_future_local(async move {
                    match rx.recv().await {
                        Ok(Ok(pw)) if !pw.trim().is_empty() => {
                            if let Some(display) = gtk4::gdk::Display::default() {
                                display.clipboard().set_text(pw.trim());
                                b.set_label("Contraseña copiada al portapapeles.");
                            } else {
                                b.set_label("No se pudo acceder al portapapeles.");
                            }
                            b.set_visible(true);
                        }
                        _ => {
                            b.set_label("Esta red no tiene contraseña guardada accesible.");
                            b.set_visible(true);
                        }
                    }
                });
            });
            menu_box.append(&copy_pw_wrap);

            let (det_wrap, det_item) = make_popover_menu_row(None, "Configuración de esta red...");
            let pop_c = popover.clone();
            let win_c = parent_window.clone();
            let snap_c = snapshot.clone();
            let net_c = net.clone();
            let ban_c = banner.clone();
            let rel_c = trigger_reload.clone();
            det_item.connect_clicked(move |_| {
                pop_c.popdown();
                show_network_details_dialog(&win_c, &snap_c, Some(&net_c), &ban_c, &rel_c);
            });
            menu_box.append(&det_wrap);

            let (forget_wrap, forget_item) = make_popover_menu_row(None, "Olvidar esta red...");
            let pop_c = popover.clone();
            let ssid_c = net.ssid.clone();
            let ban_c = banner.clone();
            let rel_c = trigger_reload.clone();
            forget_item.connect_clicked(move |_| {
                pop_c.popdown();
                forget_wifi_async(ssid_c.clone(), &ban_c, &rel_c);
            });
            menu_box.append(&forget_wrap);

            popover.set_child(Some(&menu_box));
            more_btn.set_popover(Some(&popover));
        } else {
            let more_btn = Button::builder()
                .icon_name("view-more-symbolic")
                .css_classes(["wifi-more-btn"])
                .has_frame(false)
                .valign(Align::Center)
                .tooltip_text("Dirección Wi-Fi privada")
                .build();
            let more_wrap = wrap_squircle_round_control(&more_btn, 20, RoundControlStyle::CircleOutline);
            right_icons.append(&more_wrap);

            let win_c = parent_window.clone();
            let snap_c = snapshot.clone();
            let net_c = net.clone();
            more_btn.connect_clicked(move |_| {
                show_private_wifi_address_dialog(&win_c, &snap_c, &net_c);
            });
        }

        // Acción al pulsar la fila o el botón flotante "Conectar"
        let trigger_connect: Rc<dyn Fn()> = {
            let net_click = net.clone();
            let win_click = parent_window.clone();
            let ban_click = banner.clone();
            let rel_click = trigger_reload.clone();
            Rc::new(move || {
                if net_click.connected {
                    return;
                }
                if net_click.known || !net_click.is_secured() {
                    connect_wifi_async(net_click.ssid.clone(), None, net_click.known, &ban_click, &rel_click);
                } else {
                    show_password_prompt_dialog(&win_click, &net_click.ssid, &ban_click, &rel_click);
                }
            })
        };
        let tc1 = trigger_connect.clone();
        connect_btn.connect_clicked(move |_| tc1());
        if let Some(hbtn) = hover_conn_btn {
            let tc2 = trigger_connect.clone();
            hbtn.connect_clicked(move |_| tc2());
        }

        row_box.set_margin_start(0);
        row_box.set_margin_end(0);
        row_box.set_spacing(4);
        row_box.append(&connect_btn);
        row_box.append(&right_icons);
        card.append(&row_box);
    }
}

/// Envuelve el contenido de un menú contextual con un fondo y borde Squircle propio de Malus DE.
fn wrap_squircle_popover_card(inner: &GtkBox, radius: f64) -> Overlay {
    let overlay = Overlay::new();
    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let dark = libadwaita::StyleManager::default().is_dark();
        let _ = cr.save();
        cr.translate(0.5, 0.5);
        crate::squircle_path(cr, width - 1.0, height - 1.0, radius);
        if dark {
            cr.set_source_rgba(0.17, 0.17, 0.18, 0.97);
        } else {
            cr.set_source_rgba(0.985, 0.985, 0.99, 0.98);
        }
        let _ = cr.fill_preserve();
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.10);
        }
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let _ = cr.restore();
    });

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    overlay.set_child(Some(&bg));
    overlay.add_overlay(inner);
    overlay.set_measure_overlay(inner, true);
    overlay
}

fn make_popover_menu_row(check: Option<&str>, text: &str) -> (Overlay, Button) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let check_lbl = Label::builder()
        .label(check.unwrap_or(""))
        .width_request(14)
        .xalign(0.5)
        .build();
    let text_lbl = Label::builder()
        .label(text)
        .xalign(0.0)
        .hexpand(true)
        .build();
    row.append(&check_lbl);
    row.append(&text_lbl);

    let btn = Button::builder()
        .child(&row)
        .css_classes(["wifi-menu-action-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();

    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let Some(b) = btn_weak.upgrade() else {
            return;
        };
        let flags = b.state_flags();
        if flags.contains(StateFlags::INSENSITIVE) {
            return;
        }
        let hovered = flags.contains(StateFlags::PRELIGHT);
        let active = flags.contains(StateFlags::ACTIVE);
        if !hovered && !active {
            return;
        }
        let _ = cr.save();
        crate::squircle_path(cr, width, height, WIFI_MENU_ITEM_SQUIRCLE_RADIUS);
        if active {
            cr.set_source_rgba(0.0, 0.39, 0.85, 0.86);
        } else {
            cr.set_source_rgba(0.0, 0.48, 1.0, 0.74);
        }
        let _ = cr.fill();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = Overlay::builder().hexpand(true).build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&btn);
    overlay.set_measure_overlay(&btn, true);
    (overlay, btn)
}

fn deterministic_private_mac(ssid: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in ssid.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let bytes = hash.to_le_bytes();
    // Byte 0: localmente administrada (bit 1 = 1) y unicast (bit 0 = 0)
    let b0 = (bytes[0] & 0xfc) | 0x02;
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        b0, bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

fn attach_modal_scrim(parent: &ApplicationWindow, dialog: &gtk4::Window) {
    crate::set_modal_scrim_visible(parent, true);
    let p_c1 = parent.clone();
    dialog.connect_close_request(move |_| {
        crate::set_modal_scrim_visible(&p_c1, false);
        glib::Propagation::Proceed
    });
    let p_c2 = parent.clone();
    dialog.connect_hide(move |_| {
        crate::set_modal_scrim_visible(&p_c2, false);
    });
    let p_c3 = parent.clone();
    dialog.connect_destroy(move |_| {
        crate::set_modal_scrim_visible(&p_c3, false);
    });
}

fn show_private_wifi_address_dialog(
    parent: &ApplicationWindow,
    snap: &WifiSnapshot,
    net: &WifiDisplayNetwork,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&format!("{} — Dirección Wi-Fi privada", net.ssid))
        .default_width(470)
        .build();
    attach_modal_scrim(parent, &dialog);

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    let mode_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .css_classes(["wifi-pref-row"])
        .build();

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    let title_lbl = Label::builder()
        .label("Dirección Wi-Fi privada")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let sub_lbl = Label::builder()
        .label("Usar una dirección privada ayuda a reducir el rastreo de tu equipo por parte de los operadores de redes Wi-Fi. Si la dirección privada es fija, se permite que esta red rastree el equipo, pero dificulta rastrearlo en otras redes.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();
    text_col.append(&title_lbl);
    text_col.append(&sub_lbl);

    let mode_list = StringList::new(&["Fija", "Desactivada", "Rotatoria"]);
    let mode_dd = DropDown::builder()
        .model(&mode_list)
        .selected(0)
        .valign(Align::Start)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    mode_row.append(&text_col);
    mode_row.append(&mode_dd);

    let sep = GtkBox::builder()
        .css_classes(["wifi-row-separator"])
        .build();

    let addr_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let addr_key = Label::builder()
        .label("Dirección Wi-Fi")
        .css_classes(["card-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let hw_mac = if snap.details.hwaddr.is_empty() {
        "—".to_string()
    } else {
        snap.details.hwaddr.to_lowercase()
    };
    let fixed_mac = deterministic_private_mac(&net.ssid);
    let addr_val = Label::builder()
        .label(&fixed_mac)
        .css_classes(["card-section-sub"])
        .xalign(1.0)
        .selectable(true)
        .build();
    addr_row.append(&addr_key);
    addr_row.append(&addr_val);

    let addr_val_c = addr_val.clone();
    let fixed_c = fixed_mac.clone();
    let hw_c = hw_mac.clone();
    mode_dd.connect_selected_notify(move |dd| match dd.selected() {
        0 => addr_val_c.set_label(&fixed_c),
        1 => addr_val_c.set_label(&hw_c),
        _ => addr_val_c.set_label("Rotatoria (aleatoria)"),
    });

    card.append(&mode_row);
    card.append(&sep);
    card.append(&addr_row);
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&cancel_wrap);
    actions.append(&ok_wrap);
    vbox.append(&actions);

    let d_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| d_cancel.close());

    let d_ok = dialog.clone();
    let ssid_ok = net.ssid.clone();
    let mode_dd_ok = mode_dd.clone();
    ok_btn.connect_clicked(move |_| {
        let nm_mode = match mode_dd_ok.selected() {
            0 => "stable",
            1 => "permanent",
            _ => "random",
        };
        let s = ssid_ok.clone();
        std::thread::spawn(move || {
            let _ = run_nmcli(&["connection", "modify", "id", &s, "wifi.cloned-mac-address", nm_mode]);
        });
        d_ok.close();
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn wifi_signal_icon_name(signal: u8) -> &'static str {
    if signal >= 75 {
        "network-wireless-signal-excellent-symbolic"
    } else if signal >= 50 {
        "network-wireless-signal-good-symbolic"
    } else if signal >= 25 {
        "network-wireless-signal-ok-symbolic"
    } else {
        "network-wireless-signal-weak-symbolic"
    }
}

fn connect_wifi_async(
    ssid: String,
    password: Option<String>,
    known: bool,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
) {
    banner.set_label(&format!("Conectando a «{}»...", ssid));
    banner.set_visible(true);

    let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
    std::thread::spawn(move || {
        let res = if let Some(pw) = password {
            run_nmcli(&["device", "wifi", "connect", &ssid, "password", &pw]).map(|_| ())
        } else if known {
            run_nmcli(&["connection", "up", "id", &ssid])
                .or_else(|_| run_nmcli(&["device", "wifi", "connect", &ssid]))
                .map(|_| ())
        } else {
            run_nmcli(&["device", "wifi", "connect", &ssid]).map(|_| ())
        };
        let _ = tx.send_blocking(res);
    });

    let b = banner.clone();
    let rel = trigger_reload.clone();
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(())) => b.set_visible(false),
            Ok(Err(err)) => {
                b.set_label(&format!("Error al conectar: {}", err));
                b.set_visible(true);
            }
            Err(_) => b.set_visible(false),
        }
        rel(true);
    });
}

fn forget_wifi_async(ssid: String, banner: &Label, trigger_reload: &Rc<dyn Fn(bool)>) {
    banner.set_label(&format!("Olvidando red «{}»...", ssid));
    banner.set_visible(true);
    let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
    std::thread::spawn(move || {
        let res = run_nmcli(&["connection", "delete", "id", &ssid]).map(|_| ());
        let _ = tx.send_blocking(res);
    });
    let b = banner.clone();
    let rel = trigger_reload.clone();
    glib::spawn_future_local(async move {
        if let Ok(Err(err)) = rx.recv().await {
            b.set_label(&format!("No se pudo olvidar la red: {}", err));
            b.set_visible(true);
        } else {
            b.set_visible(false);
        }
        rel(false);
    });
}

fn show_password_prompt_dialog(
    parent: &ApplicationWindow,
    ssid: &str,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&format!("Conectar a «{}»", ssid))
        .default_width(360)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let title = Label::builder()
        .label(&format!("La red «{}» requiere contraseña WPA2/WPA3", ssid))
        .css_classes(["card-section-title"])
        .wrap(true)
        .xalign(0.0)
        .build();

    let pw_entry = PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Contraseña")
        .hexpand(true)
        .build();

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("Conectar", true, None);
    actions.append(&cancel_wrap);
    actions.append(&ok_wrap);

    let d_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| d_cancel.close());

    let submit_action = {
        let d_ok = dialog.clone();
        let entry_c = pw_entry.clone();
        let ssid_owned = ssid.to_string();
        let ban_c = banner.clone();
        let rel_c = trigger_reload.clone();
        Rc::new(move || {
            let pw = entry_c.text().to_string();
            d_ok.close();
            connect_wifi_async(ssid_owned.clone(), Some(pw), false, &ban_c, &rel_c);
        })
    };

    let sub_btn = submit_action.clone();
    ok_btn.connect_clicked(move |_| sub_btn());
    let sub_act = submit_action.clone();
    pw_entry.connect_activate(move |_| sub_act());

    vbox.append(&title);
    vbox.append(&pw_entry);
    vbox.append(&actions);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_hidden_network_dialog(
    parent: &ApplicationWindow,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Acceder a otra red")
        .default_width(380)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let hdr = Label::builder()
        .label("Introduce el nombre y la seguridad de la red oculta:")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    let ssid_entry = Entry::builder()
        .placeholder_text("Nombre de la red (SSID)")
        .hexpand(true)
        .build();

    let sec_list = StringList::new(&["WPA2/WPA3 Personal", "Ninguna (Abierta)"]);
    let sec_dd = DropDown::builder().model(&sec_list).selected(0).build();

    let pw_entry = PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Contraseña")
        .hexpand(true)
        .build();

    let pw_c = pw_entry.clone();
    sec_dd.connect_selected_notify(move |dd| {
        pw_c.set_sensitive(dd.selected() == 0);
    });

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(6)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (join_wrap, join_btn) = make_squircle_button("Acceder", true, None);
    actions.append(&cancel_wrap);
    actions.append(&join_wrap);

    let d_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| d_cancel.close());

    let d_join = dialog.clone();
    let ssid_e_c = ssid_entry.clone();
    let sec_dd_c = sec_dd.clone();
    let pw_e_c = pw_entry.clone();
    let ban_c = banner.clone();
    let rel_c = trigger_reload.clone();
    join_btn.connect_clicked(move |_| {
        let ssid = ssid_e_c.text().trim().to_string();
        if ssid.is_empty() {
            return;
        }
        let open = sec_dd_c.selected() == 1;
        let pw = pw_e_c.text().to_string();
        d_join.close();

        ban_c.set_label(&format!("Conectando a red oculta «{}»...", ssid));
        ban_c.set_visible(true);
        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let res = if open {
                run_nmcli(&["device", "wifi", "connect", &ssid, "hidden", "yes"]).map(|_| ())
            } else {
                run_nmcli(&[
                    "device",
                    "wifi",
                    "connect",
                    &ssid,
                    "password",
                    &pw,
                    "hidden",
                    "yes",
                ])
                .map(|_| ())
            };
            let _ = tx.send_blocking(res);
        });
        let b = ban_c.clone();
        let r = rel_c.clone();
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(())) => b.set_visible(false),
                Ok(Err(e)) => {
                    b.set_label(&format!("Error al conectar: {}", e));
                    b.set_visible(true);
                }
                Err(_) => b.set_visible(false),
            }
            r(true);
        });
    });

    vbox.append(&hdr);
    vbox.append(&ssid_entry);
    vbox.append(&sec_dd);
    vbox.append(&pw_entry);
    vbox.append(&actions);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_adapter_info_dialog(parent: &ApplicationWindow, snap: &WifiSnapshot) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Detalles del adaptador Wi-Fi")
        .default_width(380)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let rows = [
        (
            "Interfaz",
            snap.interface_name
                .as_deref()
                .unwrap_or("No detectada")
                .to_string(),
        ),
        (
            "Dirección MAC",
            if snap.details.hwaddr.is_empty() {
                "—".to_string()
            } else {
                snap.details.hwaddr.clone()
            },
        ),
        (
            "Estado del radio",
            if snap.radio_enabled {
                "Activado".to_string()
            } else {
                "Desactivado".to_string()
            },
        ),
        (
            "Estado",
            if snap.details.state.is_empty() {
                snap.interface_state.clone()
            } else {
                snap.details.state.clone()
            },
        ),
        (
            "IPv4",
            if snap.details.ip4_addresses.is_empty() {
                "—".to_string()
            } else {
                snap.details.ip4_addresses.join(", ")
            },
        ),
    ];

    let rows_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    for (k, v) in rows {
        rows_card.append(&detail_kv_row(k, &v));
    }
    vbox.append(&wrap_squircle_card(&rows_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    ok_wrap.set_halign(Align::End);
    ok_wrap.set_margin_top(6);
    let d_c = dialog.clone();
    ok_btn.connect_clicked(move |_| d_c.close());
    vbox.append(&ok_wrap);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_network_details_dialog(
    parent: &ApplicationWindow,
    snap: &WifiSnapshot,
    net: Option<&WifiDisplayNetwork>,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
) {
    let title_ssid = net
        .map(|n| n.ssid.as_str())
        .or(snap.active_connection.as_deref())
        .unwrap_or("Wi-Fi");

    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&format!("Detalles — {}", title_ssid))
        .default_width(430)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    let is_connected = net.is_some_and(|n| n.connected);
    card.append(&detail_kv_row("Red (SSID)", title_ssid));
    card.append(&detail_kv_row(
        "Seguridad",
        net.map(|n| {
            if n.is_secured() {
                n.security.as_str()
            } else {
                "Ninguna"
            }
        })
        .unwrap_or("—"),
    ));
    if let Some(n) = net {
        if !n.freq.is_empty() || !n.channel.is_empty() {
            card.append(&detail_kv_row(
                "Canal / Frecuencia",
                &format!("Canal {} ({})", n.channel, n.freq),
            ));
        }
        if !n.rate.is_empty() {
            card.append(&detail_kv_row("Velocidad Tx", &n.rate));
        }
        if !n.bssid.is_empty() {
            card.append(&detail_kv_row("BSSID", &n.bssid));
        }
    }
    if is_connected {
        if !snap.details.ip4_addresses.is_empty() {
            card.append(&detail_kv_row(
                "Dirección IPv4",
                &snap.details.ip4_addresses.join(", "),
            ));
        }
        if !snap.details.ip4_gateway.is_empty() {
            card.append(&detail_kv_row("Router", &snap.details.ip4_gateway));
        }
        if !snap.details.ip4_dns.is_empty() {
            card.append(&detail_kv_row("DNS", &snap.details.ip4_dns.join(", ")));
        }
        if !snap.details.ip6_addresses.is_empty() {
            card.append(&detail_kv_row(
                "Dirección IPv6",
                &snap.details.ip6_addresses[0],
            ));
        }
    }

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    if let Some(n) = net.filter(|n| n.known) {
        let auto_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        let auto_lbl = Label::builder()
            .label("Conectarse automáticamente a esta red")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build();
        let auto_sw = Switch::builder()
            .active(n.autoconnect)
            .css_classes(["macos-switch"])
            .build();
        let ssid_auto = n.ssid.clone();
        let rel_auto = trigger_reload.clone();
        auto_sw.connect_state_set(move |_, enabled| {
            let s = ssid_auto.clone();
            let val = if enabled { "yes" } else { "no" };
            let r = rel_auto.clone();
            let (tx, rx) = async_channel::bounded::<()>(1);
            std::thread::spawn(move || {
                let _ = run_nmcli(&["connection", "modify", "id", &s, "connection.autoconnect", val]);
                let _ = tx.send_blocking(());
            });
            glib::spawn_future_local(async move {
                let _ = rx.recv().await;
                r(false);
            });
            glib::Propagation::Proceed
        });
        auto_row.append(&auto_lbl);
        auto_row.append(&auto_sw);
        vbox.append(&auto_row);
    }

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    if let Some(n) = net.filter(|n| n.known) {
        let (forget_wrap, forget_btn) = make_squircle_button("Olvidar esta red...", false, None);
        let d_f = dialog.clone();
        let ssid_f = n.ssid.clone();
        let ban_f = banner.clone();
        let rel_f = trigger_reload.clone();
        forget_btn.connect_clicked(move |_| {
            d_f.close();
            forget_wifi_async(ssid_f.clone(), &ban_f, &rel_f);
        });
        footer.append(&forget_wrap);
    }

    let spacer = GtkBox::builder().hexpand(true).build();
    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    let d_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| d_ok.close());

    footer.append(&spacer);
    footer.append(&ok_wrap);
    vbox.append(&footer);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_advanced_dialog(
    parent: &ApplicationWindow,
    snap: &WifiSnapshot,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Opciones avanzadas de Wi-Fi")
        .default_width(450)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let hw_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    hw_card.append(&detail_kv_row(
        "Interfaz física",
        snap.interface_name.as_deref().unwrap_or("—"),
    ));
    hw_card.append(&detail_kv_row(
        "Dirección MAC Wi-Fi",
        if snap.details.hwaddr.is_empty() {
            "—"
        } else {
            &snap.details.hwaddr
        },
    ));
    hw_card.append(&detail_kv_row(
        "DNS activos",
        &if snap.details.ip4_dns.is_empty() {
            "—".to_string()
        } else {
            snap.details.ip4_dns.join(", ")
        },
    ));
    vbox.append(&wrap_squircle_card(&hw_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let known_lbl = Label::builder()
        .label("Perfiles Wi-Fi conocidos:")
        .css_classes(["wifi-section-title"])
        .xalign(0.0)
        .build();
    vbox.append(&known_lbl);

    let profiles_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    if snap.known_networks.is_empty() {
        profiles_card.append(
            &Label::builder()
                .label("Sin perfiles guardados.")
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .build(),
        );
    } else {
        for net in &snap.known_networks {
            let r = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .build();
            let lbl = Label::builder()
                .label(&format!(
                    "{}{}",
                    net.ssid,
                    if net.autoconnect {
                        " · Automática"
                    } else {
                        ""
                    }
                ))
                .css_classes(["wifi-row-title"])
                .hexpand(true)
                .xalign(0.0)
                .build();
            let (forget_wrap, forget) = make_squircle_button("Olvidar", false, None);
            let d_c = dialog.clone();
            let ssid_c = net.ssid.clone();
            let ban_c = banner.clone();
            let rel_c = trigger_reload.clone();
            forget.connect_clicked(move |_| {
                d_c.close();
                forget_wifi_async(ssid_c.clone(), &ban_c, &rel_c);
            });
            r.append(&lbl);
            r.append(&forget_wrap);
            profiles_card.append(&r);
        }
    }
    vbox.append(&wrap_squircle_card(&profiles_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    ok_wrap.set_halign(Align::End);
    let d_c = dialog.clone();
    ok_btn.connect_clicked(move |_| d_c.close());
    vbox.append(&ok_wrap);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn detail_kv_row(key: &str, val: &str) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let k = Label::builder()
        .label(key)
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let v = Label::builder()
        .label(val)
        .css_classes(["card-section-title"])
        .xalign(1.0)
        .selectable(true)
        .build();
    row.append(&k);
    row.append(&v);
    row
}

// =========================================================================
// Backend NetworkManager (nmcli) + Parsers testables
// =========================================================================

fn run_nmcli(args: &[&str]) -> Result<String, String> {
    let output = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| format!("No se pudo ejecutar nmcli: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("nmcli devolvió estado {}", output.status)
        })
    }
}

pub fn collect_wifi_snapshot(rescan: bool) -> WifiSnapshot {
    let Ok(radio_raw) = run_nmcli(&["radio", "wifi"]) else {
        return WifiSnapshot {
            radio_enabled: false,
            interface_name: None,
            interface_state: String::new(),
            active_connection: None,
            known_networks: Vec::new(),
            other_networks: Vec::new(),
            details: WifiInterfaceDetails::default(),
            nm_available: false,
        };
    };
    let radio_enabled = radio_raw.trim().eq_ignore_ascii_case("enabled");

    let dev_raw = run_nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"])
        .unwrap_or_default();
    let (interface_name, interface_state, active_connection) = parse_wifi_device_status(&dev_raw);

    let saved_raw = run_nmcli(&["-t", "-f", "NAME,TYPE,AUTOCONNECT", "connection", "show"])
        .unwrap_or_default();
    let saved_profiles = parse_saved_wifi_profiles(&saved_raw);

    let scanned = if radio_enabled {
        let rescan_arg = if rescan { "yes" } else { "no" };
        let wifi_raw = run_nmcli(&[
            "-t",
            "-f",
            "IN-USE,SSID,BSSID,MODE,CHAN,FREQ,RATE,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "--rescan",
            rescan_arg,
        ])
        .or_else(|_| {
            run_nmcli(&[
                "-t",
                "-f",
                "IN-USE,SSID,BSSID,MODE,CHAN,FREQ,RATE,SIGNAL,SECURITY",
                "device",
                "wifi",
                "list",
            ])
        })
        .unwrap_or_default();
        parse_scanned_wifi_aps(&wifi_raw)
    } else {
        Vec::new()
    };

    let details = if let Some(ref dev) = interface_name {
        let dev_detail_raw = run_nmcli(&[
            "-t",
            "-f",
            "GENERAL.DEVICE,GENERAL.HWADDR,GENERAL.STATE,GENERAL.CONNECTION,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP6.ADDRESS",
            "device",
            "show",
            dev,
        ])
        .unwrap_or_default();
        parse_wifi_device_details(&dev_detail_raw)
    } else {
        WifiInterfaceDetails::default()
    };

    let (known_networks, other_networks) =
        classify_wifi_networks(&saved_profiles, &scanned, active_connection.as_deref());

    WifiSnapshot {
        radio_enabled,
        interface_name,
        interface_state,
        active_connection,
        known_networks,
        other_networks,
        details,
        nm_available: true,
    }
}

pub fn split_nmcli_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                cur.push(next);
            } else {
                cur.push('\\');
            }
        } else if c == ':' {
            fields.push(cur);
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

pub fn parse_wifi_device_status(raw: &str) -> (Option<String>, String, Option<String>) {
    for line in raw.lines() {
        let parts = split_nmcli_terse(line.trim());
        if parts.len() >= 4 && parts[1] == "wifi" {
            let conn = if parts[3].trim().is_empty() {
                None
            } else {
                Some(parts[3].trim().to_string())
            };
            return (Some(parts[0].trim().to_string()), parts[2].trim().to_string(), conn);
        }
    }
    (None, String::new(), None)
}

pub fn parse_saved_wifi_profiles(raw: &str) -> Vec<SavedWifiProfile> {
    let mut list = Vec::new();
    let mut seen = HashSet::new();
    for line in raw.lines() {
        let parts = split_nmcli_terse(line.trim());
        if parts.len() >= 3 && parts[1].contains("wireless") {
            let name = parts[0].trim().to_string();
            if !name.is_empty() && seen.insert(name.clone()) {
                let ac = parts[2].trim().eq_ignore_ascii_case("yes")
                    || parts[2].trim().eq_ignore_ascii_case("sí");
                list.push(SavedWifiProfile {
                    name,
                    autoconnect: ac,
                });
            }
        }
    }
    list
}

pub fn parse_scanned_wifi_aps(raw: &str) -> Vec<ScannedWifiAp> {
    let mut by_ssid: BTreeMap<String, ScannedWifiAp> = BTreeMap::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parts = split_nmcli_terse(line);
        if parts.len() < 9 {
            continue;
        }
        let in_use_tok = parts[0].trim().to_lowercase();
        let in_use = in_use_tok == "*"
            || in_use_tok == "yes"
            || in_use_tok == "sí"
            || in_use_tok == "si";
        let ssid = parts[1].trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let ap = ScannedWifiAp {
            in_use,
            ssid: ssid.clone(),
            bssid: parts[2].trim().to_string(),
            mode: parts[3].trim().to_string(),
            channel: parts[4].trim().to_string(),
            freq: parts[5].trim().to_string(),
            rate: parts[6].trim().to_string(),
            signal: parts[7].trim().parse::<u8>().unwrap_or(0),
            security: parts[8].trim().to_string(),
        };

        match by_ssid.get(&ssid) {
            Some(existing) if existing.in_use && !ap.in_use => {}
            Some(existing) if existing.in_use == ap.in_use && existing.signal >= ap.signal => {}
            _ => {
                by_ssid.insert(ssid, ap);
            }
        }
    }
    by_ssid.into_values().collect()
}

pub fn parse_wifi_device_details(raw: &str) -> WifiInterfaceDetails {
    let mut details = WifiInterfaceDetails::default();
    for line in raw.lines() {
        let parts = split_nmcli_terse(line.trim());
        if parts.len() < 2 {
            continue;
        }
        let key = parts[0].trim();
        let val = parts[1..].join(":");
        let val = val.trim().to_string();
        if val.is_empty() || val == "--" {
            continue;
        }
        if key == "GENERAL.DEVICE" {
            details.device = val;
        } else if key == "GENERAL.HWADDR" {
            details.hwaddr = val;
        } else if key == "GENERAL.STATE" {
            details.state = val;
        } else if key == "GENERAL.CONNECTION" {
            details.connection = val;
        } else if key.starts_with("IP4.ADDRESS") {
            details.ip4_addresses.push(val);
        } else if key == "IP4.GATEWAY" {
            details.ip4_gateway = val;
        } else if key.starts_with("IP4.DNS") {
            details.ip4_dns.push(val);
        } else if key.starts_with("IP6.ADDRESS") {
            details.ip6_addresses.push(val);
        }
    }
    details
}

pub fn classify_wifi_networks(
    saved: &[SavedWifiProfile],
    scanned: &[ScannedWifiAp],
    active_connection: Option<&str>,
) -> (Vec<WifiDisplayNetwork>, Vec<WifiDisplayNetwork>) {
    let mut scan_map: BTreeMap<String, &ScannedWifiAp> = BTreeMap::new();
    for ap in scanned {
        scan_map.insert(ap.ssid.clone(), ap);
    }

    let mut known_set = HashSet::new();
    let mut known = Vec::new();
    for profile in saved {
        known_set.insert(profile.name.clone());
        let ap = scan_map.get(&profile.name).copied();
        let connected = active_connection == Some(profile.name.as_str())
            || ap.is_some_and(|a| a.in_use);
        known.push(WifiDisplayNetwork {
            ssid: profile.name.clone(),
            known: true,
            connected,
            autoconnect: profile.autoconnect,
            in_range: ap.is_some(),
            signal: ap.map(|a| a.signal).unwrap_or(0),
            security: ap.map(|a| a.security.clone()).unwrap_or_default(),
            bssid: ap.map(|a| a.bssid.clone()).unwrap_or_default(),
            channel: ap.map(|a| a.channel.clone()).unwrap_or_default(),
            freq: ap.map(|a| a.freq.clone()).unwrap_or_default(),
            rate: ap.map(|a| a.rate.clone()).unwrap_or_default(),
        });
    }

    // Ordenar conocidas: alfabético (como IZZI-6E4C, Mega_5G_B982)
    known.sort_by(|a, b| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()));

    let mut others: Vec<WifiDisplayNetwork> = scanned
        .iter()
        .filter(|ap| !known_set.contains(&ap.ssid))
        .map(|ap| WifiDisplayNetwork {
            ssid: ap.ssid.clone(),
            known: false,
            connected: ap.in_use || active_connection == Some(ap.ssid.as_str()),
            autoconnect: false,
            in_range: true,
            signal: ap.signal,
            security: ap.security.clone(),
            bssid: ap.bssid.clone(),
            channel: ap.channel.clone(),
            freq: ap.freq.clone(),
            rate: ap.rate.clone(),
        })
        .collect();
    others.sort_by(|a, b| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()));

    (known, others)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nmcli_terse_with_escaped_colons_and_classify() {
        let saved_raw = "IZZI-6E4C:802-11-wireless:yes\nMega_5G_B982:802-11-wireless:yes\nWired connection 1:802-3-ethernet:yes\n";
        let saved = parse_saved_wifi_profiles(saved_raw);
        assert_eq!(saved.len(), 2);

        let scan_raw = "\
sí:Mega_5G_B982:18\\:B0\\:A4\\:5F\\:B9\\:83:Infra:52:5260 MHz:270 Mbit/s:82:WPA2
no:INFINITUM63B5_5:AA\\:BB\\:CC\\:DD\\:EE\\:FF:Infra:36:5180 MHz:400 Mbit/s:68:WPA2
no:Red\\:ConDosPuntos:11\\:22\\:33\\:44\\:55\\:66:Infra:6:2437 MHz:130 Mbit/s:45:--
";
        let scanned = parse_scanned_wifi_aps(scan_raw);
        assert_eq!(scanned.len(), 3);
        let custom = scanned.iter().find(|s| s.ssid == "Red:ConDosPuntos").unwrap();
        assert_eq!(custom.bssid, "11:22:33:44:55:66");

        let (known, others) = classify_wifi_networks(&saved, &scanned, Some("Mega_5G_B982"));
        assert_eq!(known.len(), 2);
        assert_eq!(known[0].ssid, "IZZI-6E4C");
        assert!(!known[0].connected);
        assert_eq!(known[1].ssid, "Mega_5G_B982");
        assert!(known[1].connected);
        assert_eq!(others.len(), 2);
        assert_eq!(others[0].ssid, "INFINITUM63B5_5");
        assert!(others[0].is_secured());
        assert!(!others[1].is_secured());
    }

    #[test]
    fn parses_wifi_device_status_and_details() {
        let dev_raw = "wlp4s0:wifi:connected:Mega_5G_B982\nenp6s0:ethernet:unavailable:\n";
        let (iface, state, conn) = parse_wifi_device_status(dev_raw);
        assert_eq!(iface.as_deref(), Some("wlp4s0"));
        assert_eq!(state, "connected");
        assert_eq!(conn.as_deref(), Some("Mega_5G_B982"));

        let det_raw = "GENERAL.DEVICE:wlp4s0\nGENERAL.HWADDR:9A\\:61\\:39\\:D9\\:A6\\:9A\nIP4.ADDRESS[1]:192.168.1.8/24\nIP4.GATEWAY:192.168.1.1\nIP4.DNS[1]:192.168.1.1\n";
        let det = parse_wifi_device_details(det_raw);
        assert_eq!(det.hwaddr, "9A:61:39:D9:A6:9A");
        assert_eq!(det.ip4_addresses, vec!["192.168.1.8/24"]);
        assert_eq!(det.ip4_gateway, "192.168.1.1");
    }

    #[test]
    fn squircle_wifi_widgets_smoke_when_display_available() {
        if gtk4::init().is_err() {
            return;
        }
        let badge = make_squircle_wifi_badge(34, 18, WIFI_BADGE_SQUIRCLE_RADIUS);
        assert_eq!(badge.width_request(), 34);
        assert_eq!(badge.height_request(), 34);

        let card_inner = GtkBox::new(Orientation::Vertical, 0);
        let card_wrap = wrap_squircle_card(&card_inner, WIFI_CARD_SQUIRCLE_RADIUS);
        assert!(card_wrap.child().is_some());

        let (btn_wrap, btn) = make_squircle_button("Detalles...", false, None);
        assert!(btn_wrap.child().is_some());
        assert_eq!(btn.label().as_deref(), Some("Detalles..."));
    }
}
