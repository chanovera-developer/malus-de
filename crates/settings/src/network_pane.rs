use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, Image, Label, MenuButton,
    Orientation, Overlay, PolicyType, Popover, ScrolledWindow, StringList,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use crate::wifi_pane::{
    build_squircle_chevron_menu_button, detail_kv_row, make_help_circle_button,
    make_popover_menu_row, make_squircle_badge, make_squircle_button, make_squircle_colored_badge,
    wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS,
};

const FIREWALL_BADGE_TOP: (f64, f64, f64) = (0.988, 0.584, 0.129);
const FIREWALL_BADGE_BOT: (f64, f64, f64) = (0.945, 0.459, 0.039);
const GRAY_BADGE_TOP: (f64, f64, f64) = (0.612, 0.620, 0.647);
const GRAY_BADGE_BOT: (f64, f64, f64) = (0.486, 0.494, 0.522);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceDotState {
    Connected,
    Pending,
    Disconnected,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkServiceEntry {
    pub id: String,
    pub display_name: String,
    pub device: String,
    pub dev_type: String,
    pub connection_name: String,
    pub state_raw: String,
    pub dot: ServiceDotState,
    pub status_label: String,
    pub icon_name: String,
    pub hwaddr: String,
    pub ipv4: Vec<String>,
    pub gateway: String,
    pub dns: Vec<String>,
    pub route_metric: Option<i32>,
    pub autoconnect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirewallAppAction {
    Allow,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallAppRule {
    pub app_name: String,
    pub service_or_port: String,
    pub action: FirewallAppAction,
    pub is_custom_binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallPreferences {
    /// Preferencia persistida; ver docs/decisiones/0008-red-pendiente.md (requiere eBPF/OpenSnitch en Linux).
    pub allow_builtin_signed: bool,
    /// Preferencia persistida; ver docs/decisiones/0008-red-pendiente.md (Linux no usa firma Mach-O por CA).
    pub allow_downloaded_signed: bool,
    /// Zona anterior antes de activar "Bloquear todas las conexiones entrantes" (ej. FedoraWorkstation / public).
    pub previous_zone: String,
    pub custom_rules: Vec<FirewallAppRule>,
}

impl Default for FirewallPreferences {
    fn default() -> Self {
        Self {
            allow_builtin_signed: true,
            allow_downloaded_signed: true,
            previous_zone: "public".to_string(),
            custom_rules: Vec::new(),
        }
    }
}

impl FirewallPreferences {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/firewall-settings.toml")
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkLocationsConfig {
    pub active_location: String,
    pub locations: Vec<String>,
}

impl Default for NetworkLocationsConfig {
    fn default() -> Self {
        Self {
            active_location: "Automática".to_string(),
            locations: vec![
                "Automática".to_string(),
                "Casa".to_string(),
                "Oficina".to_string(),
            ],
        }
    }
}

impl NetworkLocationsConfig {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/network-locations.toml")
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

#[derive(Debug, Clone, Default)]
pub struct FirewallSnapshot {
    pub firewalld_available: bool,
    pub active: bool,
    pub active_zone: String,
    pub block_all_incoming: bool,
    pub stealth_mode: bool,
    pub open_services: Vec<String>,
    pub open_ports: Vec<String>,
    pub merged_rules: Vec<FirewallAppRule>,
}

#[derive(Debug, Clone)]
pub struct NetworkOverviewSnapshot {
    pub nm_available: bool,
    pub wifi_dot: ServiceDotState,
    pub wifi_status: String,
    pub firewall: FirewallSnapshot,
    pub other_services: Vec<NetworkServiceEntry>,
}

impl Default for NetworkOverviewSnapshot {
    fn default() -> Self {
        Self {
            nm_available: false,
            wifi_dot: ServiceDotState::Unavailable,
            wifi_status: "Desactivado".to_string(),
            firewall: FirewallSnapshot::default(),
            other_services: Vec::new(),
        }
    }
}

/// Crea tanto el panel principal de "Red" (`Red00.png`) como el subpanel "Firewall" (`Red01.png`..`Red03.png`).
/// Devolver ambos permite navegar con `< | >` en la cabecera de Ajustes igual que en macOS.
pub fn create_network_panes(
    parent_window: &ApplicationWindow,
    open_wifi: Rc<dyn Fn()>,
    open_firewall: Rc<dyn Fn()>,
) -> (
    (ScrolledWindow, Rc<dyn Fn()>),
    (ScrolledWindow, Rc<dyn Fn()>),
) {
    let latest_snap = Rc::new(RefCell::new(NetworkOverviewSnapshot::default()));
    let reload_lock = Rc::new(Cell::new(false));

    // =========================================================
    // 1. Vista principal "Red" (assets/screenshots/Red00.png)
    // =========================================================
    let net_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let net_pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let net_banner = Label::builder()
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    net_pane.append(&net_banner);

    // Tarjeta 1: Wi-Fi -> navega a panel Wi-Fi
    let (wifi_row_btn, wifi_dot_lbl, wifi_sub_lbl) = build_nav_service_card_row(
        make_squircle_badge(
            32,
            "network-wireless-symbolic",
            17,
            WIFI_BADGE_SQUIRCLE_RADIUS,
        ),
        "Wi-Fi",
        "Conectado",
        ServiceDotState::Connected,
    );
    let open_wifi_c = open_wifi.clone();
    wifi_row_btn.connect_clicked(move |_| open_wifi_c());
    let wifi_card_box = GtkBox::builder().orientation(Orientation::Vertical).build();
    wifi_card_box.append(&wifi_row_btn);
    net_pane.append(&wrap_squircle_card(
        &wifi_card_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Tarjeta 2: Firewall -> navega a subvista Firewall
    let (fw_row_btn, fw_dot_lbl, fw_sub_lbl) = build_nav_service_card_row(
        make_squircle_colored_badge(
            32,
            "security-high-symbolic",
            17,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            FIREWALL_BADGE_TOP,
            FIREWALL_BADGE_BOT,
        ),
        "Firewall",
        "Activo",
        ServiceDotState::Connected,
    );
    let open_fw_c = open_firewall.clone();
    fw_row_btn.connect_clicked(move |_| open_fw_c());
    let fw_card_box = GtkBox::builder().orientation(Orientation::Vertical).build();
    fw_card_box.append(&fw_row_btn);
    net_pane.append(&wrap_squircle_card(&fw_card_box, WIFI_CARD_SQUIRCLE_RADIUS));

    // Sección "Otros servicios"
    let other_title = Label::builder()
        .label("Otros servicios")
        .css_classes(["wifi-section-title"])
        .xalign(0.0)
        .margin_top(6)
        .build();
    net_pane.append(&other_title);

    let other_list_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    net_pane.append(&wrap_squircle_card(
        &other_list_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Fila inferior derecha: [ ••• ⌄ ] [ ? ]
    let bottom_actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (more_pill_wrap, more_menu_btn) =
        build_squircle_chevron_menu_button("view-more-symbolic", "Acciones de servicios de red");
    bottom_actions.append(&more_pill_wrap);

    let (net_help_wrap, net_help_btn) = make_help_circle_button("Ayuda sobre Red");
    let net_help_pop = build_help_popover(
        "Red y Servicios",
        "Administra Wi-Fi, Ethernet, puentes de red, Firewalld y el orden de métricas de ruta.\n\nNota de adaptación: las ubicaciones de red y algunos tipos de servicio virtuales se guardan localmente mientras se amplía su integración automática (ver docs/decisiones/0008-red-pendiente.md).",
    );
    net_help_btn.set_popover(Some(&net_help_pop));
    bottom_actions.append(&net_help_wrap);
    net_pane.append(&bottom_actions);
    net_scroll.set_child(Some(&net_pane));

    // =========================================================
    // 2. Subvista "Firewall" (assets/screenshots/Red01.png)
    // =========================================================
    let fw_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let fw_pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let fw_status_banner = Label::builder()
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    fw_pane.append(&fw_status_banner);

    let fw_hero_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let fw_top_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    let fw_hero_badge = make_squircle_colored_badge(
        34,
        "security-high-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        FIREWALL_BADGE_TOP,
        FIREWALL_BADGE_BOT,
    );
    let fw_text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    let fw_hero_title = Label::builder()
        .label("Firewall")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let fw_hero_desc = Label::builder()
        .label("El firewall está activado y configurado para evitar que aplicaciones, programas y servicios no autorizados acepten conexiones entrantes.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();
    fw_text_col.append(&fw_hero_title);
    fw_text_col.append(&fw_hero_desc);

    let fw_switch = Switch::builder()
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();

    fw_top_row.append(&fw_hero_badge);
    fw_top_row.append(&fw_text_col);
    fw_top_row.append(&*fw_switch);
    fw_hero_card.append(&fw_top_row);

    let fw_sep = GtkBox::builder().css_classes(["wifi-hairline-sep"]).build();
    fw_hero_card.append(&fw_sep);

    let fw_warn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let fw_warn_icon = Image::builder()
        .icon_name("dialog-warning-symbolic")
        .pixel_size(15)
        .valign(Align::Start)
        .margin_top(2)
        .css_classes(["net-warn-icon"])
        .build();
    let fw_warn_lbl = Label::builder()
        .label("Comprobando el estado del firewall del sistema...")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .build();
    fw_warn_row.append(&fw_warn_icon);
    fw_warn_row.append(&fw_warn_lbl);
    fw_hero_card.append(&fw_warn_row);

    fw_pane.append(&wrap_squircle_card(
        &fw_hero_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Botones "Opciones..." y "?" bajo la tarjeta del Firewall (Red01.png)
    let fw_actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (fw_opt_wrap, fw_opt_btn) = make_squircle_button("Opciones...", false, None);
    fw_actions.append(&fw_opt_wrap);

    let (fw_help_wrap, fw_help_btn) = make_help_circle_button("Ayuda sobre el Firewall");
    let fw_help_pop = build_help_popover(
        "Firewall en Malus DE",
        "Controla firewalld y el bloqueo ICMP (modo encubierto).\n\nNota: en Linux firewalld opera por servicios, puertos y zonas; las preferencias de firma de software integrado/descargado se conservan como anotación de diseño para una futura integración con eBPF/OpenSnitch (docs/decisiones/0008-red-pendiente.md).",
    );
    fw_help_btn.set_popover(Some(&fw_help_pop));
    fw_actions.append(&fw_help_wrap);

    fw_pane.append(&fw_actions);
    fw_scroll.set_child(Some(&fw_pane));

    // =========================================================
    // 3. Sincronización de estado y callbacks compartidos
    // =========================================================
    let syncing_fw_switch = Rc::new(Cell::new(false));
    let trigger_reload: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let run_reload: Rc<dyn Fn()> = {
        let snap_ref = latest_snap.clone();
        let lock_c = reload_lock.clone();
        let win_c = parent_window.clone();
        let net_ban = net_banner.clone();
        let fw_ban = fw_status_banner.clone();
        let wifi_dot_c = wifi_dot_lbl.clone();
        let wifi_sub_c = wifi_sub_lbl.clone();
        let fw_dot_c = fw_dot_lbl.clone();
        let fw_sub_c = fw_sub_lbl.clone();
        let other_box_c = other_list_box.clone();
        let fw_desc_c = fw_hero_desc.clone();
        let fw_sw_c = fw_switch.clone();
        let fw_sync_c = syncing_fw_switch.clone();
        let fw_warn_ico_c = fw_warn_icon.clone();
        let fw_warn_lbl_c = fw_warn_lbl.clone();
        let fw_opt_btn_c = fw_opt_btn.clone();
        let trig_ref = trigger_reload.clone();

        Rc::new(move || {
            if lock_c.replace(true) {
                return;
            }
            let (tx, rx) = async_channel::bounded::<NetworkOverviewSnapshot>(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(collect_network_snapshot());
            });

            let snap_c = snap_ref.clone();
            let lock_done = lock_c.clone();
            let win_d = win_c.clone();
            let net_ban_d = net_ban.clone();
            let fw_ban_d = fw_ban.clone();
            let wifi_dot_d = wifi_dot_c.clone();
            let wifi_sub_d = wifi_sub_c.clone();
            let fw_dot_d = fw_dot_c.clone();
            let fw_sub_d = fw_sub_c.clone();
            let other_box_d = other_box_c.clone();
            let fw_desc_d = fw_desc_c.clone();
            let fw_sw_d = fw_sw_c.clone();
            let fw_sync_d = fw_sync_c.clone();
            let fw_warn_ico_d = fw_warn_ico_c.clone();
            let fw_warn_lbl_d = fw_warn_lbl_c.clone();
            let fw_opt_btn_d = fw_opt_btn_c.clone();
            let trig_d = trig_ref.clone();

            glib::spawn_future_local(async move {
                if let Ok(snapshot) = rx.recv().await {
                    *snap_c.borrow_mut() = snapshot.clone();

                    if !snapshot.nm_available {
                        net_ban_d.set_label(
                            "NetworkManager (nmcli) no disponible o inaccesible en este entorno.",
                        );
                        net_ban_d.set_visible(true);
                    } else {
                        net_ban_d.set_visible(false);
                    }

                    apply_dot_style(&wifi_dot_d, snapshot.wifi_dot);
                    wifi_sub_d.set_label(&snapshot.wifi_status);

                    let (fw_dot, fw_text) = if !snapshot.firewall.firewalld_available {
                        (ServiceDotState::Unavailable, "No disponible".to_string())
                    } else if snapshot.firewall.active {
                        (ServiceDotState::Connected, "Activo".to_string())
                    } else {
                        (ServiceDotState::Disconnected, "Inactivo".to_string())
                    };
                    apply_dot_style(&fw_dot_d, fw_dot);
                    fw_sub_d.set_label(&fw_text);

                    let reload_cb: Rc<dyn Fn()> = Rc::new({
                        let t = trig_d.clone();
                        move || {
                            if let Some(cb) = t.borrow().as_ref() {
                                cb();
                            }
                        }
                    });
                    populate_other_services_card(
                        &other_box_d,
                        &snapshot.other_services,
                        &win_d,
                        &net_ban_d,
                        &reload_cb,
                    );

                    fw_sync_d.set(true);
                    fw_sw_d.set_active(snapshot.firewall.active);
                    fw_sw_d.set_sensitive(snapshot.firewall.firewalld_available);
                    fw_sync_d.set(false);
                    fw_opt_btn_d.set_sensitive(snapshot.firewall.firewalld_available);

                    if !snapshot.firewall.firewalld_available {
                        fw_desc_d.set_label(
                            "firewalld no está disponible o accesible desde este entorno.",
                        );
                        fw_warn_ico_d.set_icon_name(Some("dialog-information-symbolic"));
                        fw_warn_ico_d.remove_css_class("net-warn-icon");
                        fw_warn_lbl_d.set_label(
                            "Instala o habilita firewalld para gestionar reglas de acceso entrante desde Ajustes.",
                        );
                        fw_ban_d.set_visible(false);
                    } else if snapshot.firewall.active {
                        fw_desc_d.set_label(
                            "El firewall está activado y configurado para evitar que aplicaciones, programas y servicios no autorizados acepten conexiones entrantes.",
                        );
                        if snapshot.firewall.block_all_incoming {
                            fw_warn_ico_d.set_icon_name(Some("dialog-warning-symbolic"));
                            fw_warn_ico_d.add_css_class("net-warn-icon");
                            fw_warn_lbl_d.set_label(
                                "El firewall bloquea todos los servicios compartidos, como compartir archivos, compartir pantalla y compartir contenido multimedia. Para permitir los servicios compartidos, haz clic en Opciones y desactiva Bloquear todas las conexiones entrantes.",
                            );
                        } else {
                            fw_warn_ico_d.set_icon_name(Some("dialog-information-symbolic"));
                            fw_warn_ico_d.remove_css_class("net-warn-icon");
                            let stealth_note = if snapshot.firewall.stealth_mode {
                                " · Modo encubierto activo (ICMP bloqueado)"
                            } else {
                                ""
                            };
                            fw_warn_lbl_d.set_label(&format!(
                                "Zona activa: «{}» · {} servicios y {} puertos permitidos{}.",
                                snapshot.firewall.active_zone,
                                snapshot.firewall.open_services.len(),
                                snapshot.firewall.open_ports.len(),
                                stealth_note
                            ));
                        }
                    } else {
                        fw_desc_d.set_label(
                            "El firewall está desactivado. Las aplicaciones y servicios pueden aceptar conexiones entrantes libremente.",
                        );
                        fw_warn_ico_d.set_icon_name(Some("dialog-warning-symbolic"));
                        fw_warn_ico_d.add_css_class("net-warn-icon");
                        fw_warn_lbl_d.set_label(
                            "Activa el interruptor superior para habilitar firewalld y proteger las conexiones entrantes.",
                        );
                    }
                }
                lock_done.set(false);
            });
        })
    };

    *trigger_reload.borrow_mut() = Some(run_reload.clone());

    // Conectar menú [••• ⌄] de Red00.png
    {
        let pop =
            build_network_actions_popover(parent_window, &latest_snap, &net_banner, &run_reload);
        more_menu_btn.set_popover(Some(&pop));
    }

    // Conectar switch principal de Firewall (Red01.png)
    {
        let sync_c = syncing_fw_switch.clone();
        let ban_c = fw_status_banner.clone();
        let rel_c = run_reload.clone();
        fw_switch.connect_state_set(move |sw, enabled| {
            if sync_c.get() {
                return glib::Propagation::Proceed;
            }
            sw.set_sensitive(false);
            ban_c.set_label(if enabled {
                "Activando firewall del sistema..."
            } else {
                "Desactivando firewall del sistema..."
            });
            ban_c.set_visible(true);

            let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
            std::thread::spawn(move || {
                let res = set_firewalld_enabled(enabled);
                let _ = tx.send_blocking(res);
            });

            let sw_done = sw.clone();
            let ban_done = ban_c.clone();
            let rel_done = rel_c.clone();
            let sync_done = sync_c.clone();
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(())) => ban_done.set_visible(false),
                    Ok(Err(err)) => {
                        ban_done.set_label(&format!("No se pudo cambiar el Firewall: {err}"));
                        ban_done.set_visible(true);
                        sync_done.set(true);
                        sw_done.set_active(!enabled);
                        sync_done.set(false);
                    }
                    Err(_) => ban_done.set_visible(false),
                }
                sw_done.set_sensitive(true);
                rel_done();
            });
            glib::Propagation::Proceed
        });
    }

    // Botón "Opciones..." -> modal Red02.png
    {
        let win_c = parent_window.clone();
        let snap_c = latest_snap.clone();
        let ban_c = fw_status_banner.clone();
        let rel_c = run_reload.clone();
        fw_opt_btn.connect_clicked(move |_| {
            let snap = snap_c.borrow().firewall.clone();
            show_firewall_options_dialog(&win_c, &snap, &ban_c, &rel_c);
        });
    }

    let net_refresh: Rc<dyn Fn()> = run_reload.clone();
    let fw_refresh: Rc<dyn Fn()> = run_reload;
    ((net_scroll, net_refresh), (fw_scroll, fw_refresh))
}

fn build_nav_service_card_row(
    badge: Overlay,
    title: &str,
    subtitle: &str,
    dot: ServiceDotState,
) -> (Button, Label, Label) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .build();

    let sub_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let dot_lbl = Label::builder().label("●").valign(Align::Center).build();
    apply_dot_style(&dot_lbl, dot);
    let sub_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    sub_row.append(&dot_lbl);
    sub_row.append(&sub_lbl);

    text_col.append(&title_lbl);
    text_col.append(&sub_row);

    let chev = Image::builder()
        .icon_name("go-next-symbolic")
        .pixel_size(14)
        .css_classes(["net-row-chevron"])
        .valign(Align::Center)
        .build();

    badge.set_valign(Align::Center);
    row.append(&badge);
    row.append(&text_col);
    row.append(&chev);

    let btn = Button::builder()
        .child(&row)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();

    (btn, dot_lbl, sub_lbl)
}

fn apply_dot_style(lbl: &Label, state: ServiceDotState) {
    for cls in [
        "net-dot-green",
        "net-dot-amber",
        "net-dot-red",
        "net-dot-gray",
    ] {
        lbl.remove_css_class(cls);
    }
    let cls = match state {
        ServiceDotState::Connected => "net-dot-green",
        ServiceDotState::Pending => "net-dot-amber",
        ServiceDotState::Disconnected => "net-dot-red",
        ServiceDotState::Unavailable => "net-dot-gray",
    };
    lbl.add_css_class(cls);
}

fn populate_other_services_card(
    container: &GtkBox,
    services: &[NetworkServiceEntry],
    parent_window: &ApplicationWindow,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    if services.is_empty() {
        let empty = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        let badge = make_squircle_colored_badge(
            32,
            "thunderbolt-symbolic",
            17,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            GRAY_BADGE_TOP,
            GRAY_BADGE_BOT,
        );
        badge.set_valign(Align::Center);
        let text_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        text_col.append(
            &Label::builder()
                .label("Sin otros servicios de red activos")
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        text_col.append(
            &Label::builder()
                .label("Las interfaces Ethernet, puente o USB aparecerán aquí o desde “••• ⌄ → Añadir servicio...”.")
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        empty.append(&badge);
        empty.append(&text_col);
        container.append(&empty);
        return;
    }

    for (idx, svc) in services.iter().enumerate() {
        if idx > 0 {
            container.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let (top_rgb, bot_rgb) = match svc.dot {
            ServiceDotState::Connected => ((0.259, 0.565, 0.976), (0.235, 0.518, 0.949)),
            _ => (GRAY_BADGE_TOP, GRAY_BADGE_BOT),
        };
        let badge = make_squircle_colored_badge(
            32,
            &svc.icon_name,
            17,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            top_rgb,
            bot_rgb,
        );
        let (row_btn, _, _) =
            build_nav_service_card_row(badge, &svc.display_name, &svc.status_label, svc.dot);
        let svc_c = svc.clone();
        let win_c = parent_window.clone();
        let ban_c = banner.clone();
        let rel_c = trigger_reload.clone();
        row_btn.connect_clicked(move |_| {
            show_service_details_dialog(&win_c, &svc_c, &ban_c, &rel_c);
        });
        container.append(&row_btn);
    }
}

fn build_network_actions_popover(
    parent_window: &ApplicationWindow,
    latest_snap: &Rc<RefCell<NetworkOverviewSnapshot>>,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) -> Popover {
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

    let (add_wrap, add_btn) = make_popover_menu_row(None, "Añadir servicio...");
    let (order_wrap, order_btn) = make_popover_menu_row(None, "Establecer orden de servicios...");
    let (loc_wrap, loc_btn) = make_popover_menu_row(None, "Ubicaciones...");

    let pop1 = popover.clone();
    let win1 = parent_window.clone();
    let ban1 = banner.clone();
    let rel1 = trigger_reload.clone();
    add_btn.connect_clicked(move |_| {
        pop1.popdown();
        show_add_service_dialog(&win1, &ban1, &rel1);
    });

    let pop2 = popover.clone();
    let win2 = parent_window.clone();
    let snap2 = latest_snap.clone();
    let ban2 = banner.clone();
    let rel2 = trigger_reload.clone();
    order_btn.connect_clicked(move |_| {
        pop2.popdown();
        let services = snap2.borrow().other_services.clone();
        show_service_order_dialog(&win2, &services, &ban2, &rel2);
    });

    let pop3 = popover.clone();
    let win3 = parent_window.clone();
    loc_btn.connect_clicked(move |_| {
        pop3.popdown();
        show_locations_dialog(&win3);
    });

    menu_box.append(&add_wrap);
    menu_box.append(&order_wrap);
    menu_box.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    menu_box.append(&loc_wrap);

    popover.set_child(Some(&menu_box));
    popover
}

fn build_help_popover(title: &str, body: &str) -> Popover {
    let pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    let box_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .width_request(280)
        .build();
    box_inner.append(
        &Label::builder()
            .label(title)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    box_inner.append(
        &Label::builder()
            .label(body)
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pop.set_child(Some(&box_inner));
    pop
}

fn attach_scrim_lifecycle(dialog: &gtk4::Window, parent: &ApplicationWindow) {
    crate::set_modal_scrim_visible(parent, true);
    let p1 = parent.clone();
    dialog.connect_close_request(move |_| {
        crate::set_modal_scrim_visible(&p1, false);
        glib::Propagation::Proceed
    });
    let p2 = parent.clone();
    dialog.connect_destroy(move |_| {
        crate::set_modal_scrim_visible(&p2, false);
    });
}

fn show_service_details_dialog(
    parent: &ApplicationWindow,
    svc: &NetworkServiceEntry,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&svc.display_name)
        .default_width(410)
        .build();
    attach_scrim_lifecycle(&dialog, parent);

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
    card.append(&detail_kv_row("Servicio", &svc.display_name));
    card.append(&detail_kv_row(
        "Interfaz",
        if svc.device.is_empty() {
            "Virtual / Sin asignar"
        } else {
            &svc.device
        },
    ));
    card.append(&detail_kv_row("Tipo", &svc.dev_type));
    card.append(&detail_kv_row("Estado", &svc.status_label));
    if !svc.hwaddr.is_empty() {
        card.append(&detail_kv_row("Dirección MAC", &svc.hwaddr));
    }
    if !svc.ipv4.is_empty() {
        card.append(&detail_kv_row("IPv4", &svc.ipv4.join(", ")));
    }
    if !svc.gateway.is_empty() {
        card.append(&detail_kv_row("Router", &svc.gateway));
    }
    if !svc.dns.is_empty() {
        card.append(&detail_kv_row("DNS", &svc.dns.join(", ")));
    }
    if let Some(metric) = svc.route_metric {
        card.append(&detail_kv_row("Métrica de ruta", &metric.to_string()));
    }
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    if !svc.connection_name.is_empty() {
        let is_connected = svc.dot == ServiceDotState::Connected;
        let (toggle_wrap, toggle_btn) = make_squircle_button(
            if is_connected {
                "Desactivar"
            } else {
                "Conectar"
            },
            false,
            None,
        );
        let d_c = dialog.clone();
        let conn = svc.connection_name.clone();
        let ban_c = banner.clone();
        let rel_c = trigger_reload.clone();
        toggle_btn.connect_clicked(move |_| {
            crate::close_attached_modal(&d_c);
            let c_name = conn.clone();
            let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
            std::thread::spawn(move || {
                let sub = if is_connected { "down" } else { "up" };
                let res = run_nmcli(&["connection", sub, "id", &c_name]).map(|_| ());
                let _ = tx.send_blocking(res);
            });
            let b = ban_c.clone();
            let r = rel_c.clone();
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(())) => b.set_visible(false),
                    Ok(Err(err)) => {
                        b.set_label(&format!("Error de servicio de red: {err}"));
                        b.set_visible(true);
                    }
                    Err(_) => {}
                }
                r();
            });
        });
        footer.append(&toggle_wrap);
    }

    footer.append(&GtkBox::builder().hexpand(true).build());
    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    let d_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&d_ok));
    footer.append(&ok_wrap);

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    crate::present_attached_modal(&dialog, parent);
}

fn show_add_service_dialog(
    parent: &ApplicationWindow,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Añadir servicio")
        .default_width(390)
        .build();
    attach_scrim_lifecycle(&dialog, parent);

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
        .spacing(10)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    let types = StringList::new(&["Puente (Bridge)", "Ethernet (802-3-ethernet)", "VLAN"]);
    let type_dd = DropDown::builder().model(&types).selected(0).build();
    let name_entry = Entry::builder()
        .placeholder_text("Nombre del servicio (ej. Puente Thunderbolt)")
        .build();
    let if_entry = Entry::builder()
        .placeholder_text("Interfaz (ej. br0 o eth0, opcional)")
        .build();

    card.append(
        &Label::builder()
            .label("Interfaz")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    card.append(&type_dd);
    card.append(&name_entry);
    card.append(&if_entry);
    card.append(
        &Label::builder()
            .label("Nota: Thunderbolt Networking se detecta vía kernel/USB4; aquí se crean perfiles NetworkManager equivalentes.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("Crear", true, None);
    footer.append(&cancel_wrap);
    footer.append(&ok_wrap);

    let d_c = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&d_c));

    let d_ok = dialog.clone();
    let ban_c = banner.clone();
    let rel_c = trigger_reload.clone();
    ok_btn.connect_clicked(move |_| {
        let name = name_entry.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let iface = if_entry.text().trim().to_string();
        let kind_idx = type_dd.selected();
        crate::close_attached_modal(&d_ok);

        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let con_type = match kind_idx {
                0 => "bridge",
                1 => "ethernet",
                _ => "vlan",
            };
            let mut args = vec!["connection", "add", "type", con_type, "con-name", &name];
            if !iface.is_empty() {
                args.push("ifname");
                args.push(&iface);
            }
            let res = run_nmcli(&args).map(|_| ());
            let _ = tx.send_blocking(res);
        });
        let b = ban_c.clone();
        let r = rel_c.clone();
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(())) => b.set_visible(false),
                Ok(Err(err)) => {
                    b.set_label(&format!("No se pudo crear el servicio: {err}"));
                    b.set_visible(true);
                }
                Err(_) => {}
            }
            r();
        });
    });

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    crate::present_attached_modal(&dialog, parent);
}

fn show_service_order_dialog(
    parent: &ApplicationWindow,
    services: &[NetworkServiceEntry],
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Establecer orden de servicios")
        .default_width(420)
        .build();
    attach_scrim_lifecycle(&dialog, parent);

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    vbox.append(
        &Label::builder()
            .label("NetworkManager prioriza la puerta de enlace según la métrica de ruta IPv4 (menor valor = mayor prioridad).")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let ordered: Rc<RefCell<Vec<NetworkServiceEntry>>> = Rc::new(RefCell::new(services.to_vec()));
    let list_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let render_order: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let render_fn: Rc<dyn Fn()> = {
        let ord_c = ordered.clone();
        let box_c = list_card.clone();
        let self_ref = render_order.clone();
        Rc::new(move || {
            while let Some(c) = box_c.first_child() {
                box_c.remove(&c);
            }
            let items = ord_c.borrow();
            if items.is_empty() {
                box_c.append(
                    &Label::builder()
                        .label("No hay servicios adicionales con perfil para reordenar.")
                        .css_classes(["card-section-sub", "wifi-pref-row"])
                        .xalign(0.0)
                        .build(),
                );
                return;
            }
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    box_c.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(8)
                    .css_classes(["wifi-pref-row"])
                    .build();
                row.append(
                    &Label::builder()
                        .label(&format!("{}. {}", idx + 1, item.display_name))
                        .css_classes(["wifi-row-title"])
                        .hexpand(true)
                        .xalign(0.0)
                        .build(),
                );
                let up_btn = Button::builder()
                    .icon_name("go-up-symbolic")
                    .has_frame(false)
                    .sensitive(idx > 0)
                    .build();
                let down_btn = Button::builder()
                    .icon_name("go-down-symbolic")
                    .has_frame(false)
                    .sensitive(idx + 1 < items.len())
                    .build();
                let o1 = ord_c.clone();
                let s1 = self_ref.clone();
                up_btn.connect_clicked(move |_| {
                    o1.borrow_mut().swap(idx - 1, idx);
                    if let Some(f) = s1.borrow().as_ref() {
                        f();
                    }
                });
                let o2 = ord_c.clone();
                let s2 = self_ref.clone();
                down_btn.connect_clicked(move |_| {
                    o2.borrow_mut().swap(idx, idx + 1);
                    if let Some(f) = s2.borrow().as_ref() {
                        f();
                    }
                });
                row.append(&up_btn);
                row.append(&down_btn);
                box_c.append(&row);
            }
        })
    };
    *render_order.borrow_mut() = Some(render_fn.clone());
    render_fn();

    vbox.append(&wrap_squircle_card(&list_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&cancel_wrap);
    footer.append(&ok_wrap);

    let d_c = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&d_c));

    let d_ok = dialog.clone();
    let ord_ok = ordered.clone();
    let ban_ok = banner.clone();
    let rel_ok = trigger_reload.clone();
    ok_btn.connect_clicked(move |_| {
        let final_items: Vec<String> = ord_ok
            .borrow()
            .iter()
            .filter(|s| !s.connection_name.is_empty())
            .map(|s| s.connection_name.clone())
            .collect();
        crate::close_attached_modal(&d_ok);
        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            for (i, conn_name) in final_items.iter().enumerate() {
                let metric = ((i as i32 + 1) * 100).to_string();
                if let Err(e) = run_nmcli(&[
                    "connection",
                    "modify",
                    "id",
                    conn_name,
                    "ipv4.route-metric",
                    &metric,
                ]) {
                    let _ = tx.send_blocking(Err(e));
                    return;
                }
            }
            let _ = tx.send_blocking(Ok(()));
        });
        let b = ban_ok.clone();
        let r = rel_ok.clone();
        glib::spawn_future_local(async move {
            if let Ok(Err(err)) = rx.recv().await {
                b.set_label(&format!("No se pudo actualizar el orden: {err}"));
                b.set_visible(true);
            }
            r();
        });
    });

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    crate::present_attached_modal(&dialog, parent);
}

fn show_locations_dialog(parent: &ApplicationWindow) {
    let cfg = Rc::new(RefCell::new(NetworkLocationsConfig::load()));
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Ubicaciones de red")
        .default_width(380)
        .build();
    attach_scrim_lifecycle(&dialog, parent);

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
    let (model, selected_idx) = {
        let cfg_b = cfg.borrow();
        let cur_locs: Vec<&str> = cfg_b.locations.iter().map(|s| s.as_str()).collect();
        let model = StringList::new(&cur_locs);
        let selected_idx = cfg_b
            .locations
            .iter()
            .position(|l| l == &cfg_b.active_location)
            .unwrap_or(0) as u32;
        (model, selected_idx)
    };
    let dd = DropDown::builder()
        .model(&model)
        .selected(selected_idx)
        .build();

    card.append(
        &Label::builder()
            .label("Ubicación activa")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    card.append(&dd);
    // TODO(malus-red): Enlazar ubicaciones con snapshots reales de perfiles/DNS/proxy en NetworkManager.
    card.append(
        &Label::builder()
            .label("Anotación de diseño: el perfil de ubicación se guarda localmente; la conmutación automática de DNS/proxy por ubicación queda documentada en docs/decisiones/0008-red-pendiente.md.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&cancel_wrap);
    footer.append(&ok_wrap);

    let d_c = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&d_c));

    let d_ok = dialog.clone();
    let cfg_ok = cfg.clone();
    ok_btn.connect_clicked(move |_| {
        let idx = dd.selected() as usize;
        let mut c = cfg_ok.borrow_mut();
        if let Some(name) = c.locations.get(idx).cloned() {
            c.active_location = name;
            c.save();
        }
        crate::close_attached_modal(&d_ok);
    });

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    crate::present_attached_modal(&dialog, parent);
}

/// Modal "Opciones de Firewall" inspirado fielmente en `assets/screenshots/Red02.png`.
fn show_firewall_options_dialog(
    parent: &ApplicationWindow,
    snap: &FirewallSnapshot,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let prefs = Rc::new(RefCell::new(FirewallPreferences::load()));
    let draft_rules = Rc::new(RefCell::new(snap.merged_rules.clone()));
    let selected_idx: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));

    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Opciones de Firewall")
        .default_width(540)
        .build();
    attach_scrim_lifecycle(&dialog, parent);

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let outer_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    // 1. "Bloquear todas las conexiones entrantes"
    let (block_row, block_sw) = build_option_switch_row(
        "Bloquear todas las conexiones entrantes",
        Some("Bloquear todas las conexiones entrantes excepto aquellas necesarias para los servicios de Internet básicos, como DHCP e IPsec."),
        snap.block_all_incoming,
    );
    outer_card.append(&block_row);

    // 2. Caja embebida de reglas por aplicación/servicio + barra inferior [+ | -]
    let rules_frame = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["fw-apps-frame"])
        .margin_start(14)
        .margin_end(14)
        .margin_top(4)
        .margin_bottom(10)
        .build();

    let rules_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(190)
        .max_content_height(215)
        .build();
    let rules_list = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    rules_scroll.set_child(Some(&rules_list));
    rules_frame.append(&rules_scroll);

    rules_frame.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let toolbar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .css_classes(["fw-apps-toolbar"])
        .build();
    let add_rule_btn = Button::builder()
        .icon_name("list-add-symbolic")
        .has_frame(false)
        .css_classes(["fw-apps-tool-btn"])
        .tooltip_text("Añadir servicio o binario")
        .build();
    let remove_rule_btn = Button::builder()
        .icon_name("list-remove-symbolic")
        .has_frame(false)
        .css_classes(["fw-apps-tool-btn"])
        .tooltip_text("Eliminar regla seleccionada")
        .sensitive(false)
        .build();
    toolbar.append(&add_rule_btn);
    toolbar.append(
        &GtkBox::builder()
            .css_classes(["fw-apps-tool-sep"])
            .valign(Align::Center)
            .build(),
    );
    toolbar.append(&remove_rule_btn);
    rules_frame.append(&toolbar);
    outer_card.append(&rules_frame);

    let render_rules: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let render_rules_fn: Rc<dyn Fn()> = {
        let list_c = rules_list.clone();
        let rules_c = draft_rules.clone();
        let sel_c = selected_idx.clone();
        let rem_c = remove_rule_btn.clone();
        let self_ref = render_rules.clone();
        Rc::new(move || {
            while let Some(child) = list_c.first_child() {
                list_c.remove(&child);
            }
            let rules = rules_c.borrow();
            rem_c.set_sensitive(sel_c.get().is_some_and(|i| i < rules.len()));

            if rules.is_empty() {
                list_c.append(
                    &Label::builder()
                        .label("No hay servicios o aplicaciones configuradas.")
                        .css_classes(["card-section-sub", "wifi-pref-row"])
                        .xalign(0.0)
                        .build(),
                );
                return;
            }

            for (idx, rule) in rules.iter().enumerate() {
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .css_classes(if sel_c.get() == Some(idx) {
                        vec!["fw-app-row", "fw-app-row-selected"]
                    } else {
                        vec!["fw-app-row"]
                    })
                    .build();

                let icon_name = if rule.is_custom_binary {
                    "application-x-executable-symbolic"
                } else {
                    "network-server-symbolic"
                };
                row.append(
                    &Image::builder()
                        .icon_name(icon_name)
                        .pixel_size(18)
                        .valign(Align::Center)
                        .build(),
                );

                let name_lbl = Label::builder()
                    .label(&rule.app_name)
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .build();
                if rule.is_custom_binary {
                    name_lbl.set_tooltip_text(Some(
                        "Regla por binario: mapeada a servicio firewalld cuando existe; filtrado directo por binario requiere eBPF/OpenSnitch.",
                    ));
                }

                let action_btn = build_rule_action_dropdown_button(rule.action, {
                    let rules_m = rules_c.clone();
                    let ref_m = self_ref.clone();
                    move |new_action| {
                        if let Some(entry) = rules_m.borrow_mut().get_mut(idx) {
                            entry.action = new_action;
                        }
                        if let Some(f) = ref_m.borrow().as_ref() {
                            f();
                        }
                    }
                });

                let click = gtk4::GestureClick::new();
                let sel_click = sel_c.clone();
                let ref_click = self_ref.clone();
                click.connect_released(move |_, _, _, _| {
                    sel_click.set(Some(idx));
                    if let Some(f) = ref_click.borrow().as_ref() {
                        f();
                    }
                });
                row.add_controller(click);

                row.append(&name_lbl);
                row.append(&action_btn);
                list_c.append(&row);
            }
        })
    };
    *render_rules.borrow_mut() = Some(render_rules_fn.clone());
    render_rules_fn();

    {
        let rules_c = draft_rules.clone();
        let sel_c = selected_idx.clone();
        let ref_c = render_rules.clone();
        remove_rule_btn.connect_clicked(move |_| {
            if let Some(idx) = sel_c.get() {
                let mut rules = rules_c.borrow_mut();
                if idx < rules.len() {
                    rules.remove(idx);
                    sel_c.set(None);
                }
                drop(rules);
                if let Some(f) = ref_c.borrow().as_ref() {
                    f();
                }
            }
        });
    }

    {
        let dlg_parent = dialog.clone();
        let rules_c = draft_rules.clone();
        let ref_c = render_rules.clone();
        add_rule_btn.connect_clicked(move |_| {
            show_add_firewall_rule_dialog(&dlg_parent, &rules_c, &ref_c);
        });
    }

    outer_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 3. Interruptores inferiores fieles a Red02.png
    // TODO(malus-red): Linux/firewalld no implementa firma criptográfica de binarios Mach-O; estas dos opciones
    // persisten como configuración de diseño en firewall-settings.toml para futura capa eBPF/OpenSnitch.
    let (builtin_row, builtin_sw) = build_option_switch_row(
        "Permitir automáticamente que el software integrado reciba conexiones entrantes",
        None,
        prefs.borrow().allow_builtin_signed,
    );
    builtin_row.set_tooltip_text(Some(
        "Anotación Linux: firewalld filtra por puerto/servicio. Esta preferencia queda lista para futuro motor eBPF por ejecutable (ver docs/decisiones/0008-red-pendiente.md).",
    ));
    outer_card.append(&builtin_row);
    outer_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (signed_row, signed_sw) = build_option_switch_row(
        "Permitir automáticamente que el software firmado descargado reciba conexiones entrantes",
        Some("Permite que el software firmado por una entidad de certificación válida proporcione servicios ofrecidos a través de la red."),
        prefs.borrow().allow_downloaded_signed,
    );
    signed_row.set_tooltip_text(Some(
        "Anotación Linux: sin verificación de firma de app en sockets entrantes; preferencia guardada para futura capa eBPF/OpenSnitch.",
    ));
    outer_card.append(&signed_row);
    outer_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (stealth_row, stealth_sw) = build_option_switch_row(
        "Activar modo encubierto",
        Some("Permite no responder o no aceptar los intentos de acceder a esta computadora desde la red por parte de aplicaciones de prueba que usan ICMP (por ejemplo, Ping)."),
        snap.stealth_mode,
    );
    outer_card.append(&stealth_row);

    vbox.append(&wrap_squircle_card(&outer_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Botones Cancelar / OK
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

    let d_c = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&d_c));

    let d_ok = dialog.clone();
    let snap_init = snap.clone();
    let rules_ok = draft_rules.clone();
    let prefs_ok = prefs.clone();
    let ban_ok = banner.clone();
    let rel_ok = trigger_reload.clone();
    ok_btn.connect_clicked(move |_| {
        let block_all = block_sw.is_active();
        let stealth = stealth_sw.is_active();
        let new_rules = rules_ok.borrow().clone();

        {
            let mut p = prefs_ok.borrow_mut();
            p.allow_builtin_signed = builtin_sw.is_active();
            p.allow_downloaded_signed = signed_sw.is_active();
            if !snap_init.block_all_incoming
                && !snap_init.active_zone.is_empty()
                && snap_init.active_zone != "block"
                && snap_init.active_zone != "drop"
            {
                p.previous_zone = snap_init.active_zone.clone();
            }
            p.custom_rules = new_rules
                .iter()
                .filter(|r| r.is_custom_binary || r.action == FirewallAppAction::Block)
                .cloned()
                .collect();
            p.save();
        }

        let prev_zone = prefs_ok.borrow().previous_zone.clone();
        let old_snap = snap_init.clone();
        crate::close_attached_modal(&d_ok);

        ban_ok.set_label("Aplicando opciones de Firewall...");
        ban_ok.set_visible(true);

        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let res =
                apply_firewall_options_batch(&old_snap, &prev_zone, block_all, stealth, &new_rules);
            let _ = tx.send_blocking(res);
        });

        let b = ban_ok.clone();
        let r = rel_ok.clone();
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(())) => b.set_visible(false),
                Ok(Err(err)) => {
                    b.set_label(&format!("No se pudieron aplicar las opciones: {err}"));
                    b.set_visible(true);
                }
                Err(_) => b.set_visible(false),
            }
            r();
        });
    });

    dialog.set_child(Some(&vbox));
    crate::present_attached_modal(&dialog, parent);
}

fn build_option_switch_row(title: &str, subtitle: Option<&str>, active: bool) -> (GtkBox, Switch) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .css_classes(["wifi-pref-row"])
        .build();

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    text_col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    if let Some(sub) = subtitle {
        text_col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }

    let sw = Switch::builder()
        .active(active)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();

    row.append(&text_col);
    row.append(&*sw);
    (row, sw)
}

fn build_rule_action_dropdown_button<F: Fn(FirewallAppAction) + 'static>(
    action: FirewallAppAction,
    on_select: F,
) -> MenuButton {
    let inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let dot = Label::builder().label("●").build();
    dot.add_css_class(match action {
        FirewallAppAction::Allow => "net-dot-green",
        FirewallAppAction::Block => "net-dot-red",
    });
    let text = Label::builder()
        .label(match action {
            FirewallAppAction::Allow => "Permitir las conexiones entrantes",
            FirewallAppAction::Block => "Bloquear las conexiones entrantes",
        })
        .css_classes(["card-section-sub"])
        .build();
    let chev = Image::builder()
        .icon_name("pan-down-symbolic")
        .pixel_size(12)
        .build();
    inner.append(&dot);
    inner.append(&text);
    inner.append(&chev);

    let btn = MenuButton::builder()
        .child(&inner)
        .has_frame(false)
        .always_show_arrow(false)
        .css_classes(["fw-rule-action-btn"])
        .build();

    let pop = Popover::builder()
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

    let on_select = Rc::new(on_select);
    let (allow_wrap, allow_btn) = make_popover_menu_row(
        (action == FirewallAppAction::Allow).then_some("✓"),
        "Permitir las conexiones entrantes",
    );
    let (block_wrap, block_btn) = make_popover_menu_row(
        (action == FirewallAppAction::Block).then_some("✓"),
        "Bloquear las conexiones entrantes",
    );

    let pop1 = pop.clone();
    let cb1 = on_select.clone();
    allow_btn.connect_clicked(move |_| {
        pop1.popdown();
        cb1(FirewallAppAction::Allow);
    });
    let pop2 = pop.clone();
    let cb2 = on_select;
    block_btn.connect_clicked(move |_| {
        pop2.popdown();
        cb2(FirewallAppAction::Block);
    });

    menu_box.append(&allow_wrap);
    menu_box.append(&block_wrap);
    pop.set_child(Some(&menu_box));
    btn.set_popover(Some(&pop));
    btn
}

fn show_add_firewall_rule_dialog(
    parent: &gtk4::Window,
    draft_rules: &Rc<RefCell<Vec<FirewallAppRule>>>,
    render_rules: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
) {
    let dlg = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Añadir regla de Firewall")
        .default_width(360)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(18)
        .margin_end(18)
        .build();

    vbox.append(
        &Label::builder()
            .label("Nombre del servicio (ej. ssh, samba, ipp, 8080/tcp) o binario:")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let entry = Entry::builder()
        .placeholder_text("ssh / samba / 8080/tcp / python3")
        .build();
    vbox.append(&entry);

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (c_wrap, c_btn) = make_squircle_button("Cancelar", false, None);
    let (a_wrap, a_btn) = make_squircle_button("Añadir", true, None);
    footer.append(&c_wrap);
    footer.append(&a_wrap);

    let d1 = dlg.clone();
    c_btn.connect_clicked(move |_| crate::close_attached_modal(&d1));

    let d2 = dlg.clone();
    let r_c = draft_rules.clone();
    let f_c = render_rules.clone();
    a_btn.connect_clicked(move |_| {
        let raw = entry.text().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let mapped = map_binary_to_firewalld_service(&raw);
        let is_bin = mapped.as_ref().is_some_and(|m| m != &raw);
        let svc_or_port = mapped.unwrap_or_else(|| raw.clone());
        r_c.borrow_mut().push(FirewallAppRule {
            app_name: raw,
            service_or_port: svc_or_port,
            action: FirewallAppAction::Allow,
            is_custom_binary: is_bin,
        });
        crate::close_attached_modal(&d2);
        if let Some(f) = f_c.borrow().as_ref() {
            f();
        }
    });

    vbox.append(&footer);
    dlg.set_child(Some(&vbox));
    crate::present_attached_modal(&dlg, parent);
}

// ============================================================================
// Backend NetworkManager (nmcli) + firewalld (firewall-cmd) + parseos testables
// ============================================================================

fn run_nmcli(args: &[&str]) -> Result<String, String> {
    let out = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| format!("nmcli: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("nmcli salió con código {}", out.status)
        } else {
            err
        })
    }
}

fn run_firewall_cmd(args: &[&str]) -> Result<String, String> {
    let out = Command::new("firewall-cmd")
        .args(args)
        .output()
        .map_err(|e| format!("firewall-cmd: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("firewall-cmd salió con código {}", out.status)
        } else {
            err
        })
    }
}

fn set_firewalld_enabled(enable: bool) -> Result<(), String> {
    let subcmd = if enable { "enable" } else { "disable" };
    let out = Command::new("pkexec")
        .args(["systemctl", subcmd, "--now", "--", "firewalld"])
        .output()
        .map_err(|e| format!("pkexec: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            "Operación cancelada o sin permisos.".to_string()
        } else {
            err
        })
    }
}

fn is_valid_firewall_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn is_valid_firewall_port_spec(s: &str) -> bool {
    let Some((ports, proto)) = s.split_once('/') else {
        return false;
    };
    if !matches!(proto, "tcp" | "udp" | "sctp" | "dccp") {
        return false;
    }
    let parts: Vec<&str> = ports.split('-').collect();
    if parts.is_empty() || parts.len() > 2 {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.parse::<u16>().is_ok_and(|n| n > 0))
}

fn apply_firewall_options_batch(
    old_snap: &FirewallSnapshot,
    previous_zone: &str,
    block_all: bool,
    stealth: bool,
    new_rules: &[FirewallAppRule],
) -> Result<(), String> {
    let mut ops: Vec<String> = Vec::new();
    let target_zone = if block_all {
        "block".to_string()
    } else if previous_zone.is_empty() || previous_zone == "block" || previous_zone == "drop" {
        "public".to_string()
    } else {
        previous_zone.to_string()
    };

    if !is_valid_firewall_name(&target_zone) {
        return Err(format!("Zona de firewall inválida: {target_zone}"));
    }

    if block_all != old_snap.block_all_incoming {
        ops.push(format!("set-default-zone:{target_zone}"));
    }

    if stealth != old_snap.stealth_mode {
        if stealth {
            ops.push(format!("stealth-on:{target_zone}"));
        } else {
            ops.push(format!("stealth-off:{target_zone}"));
        }
    }

    let old_allowed: BTreeSet<String> = old_snap
        .open_services
        .iter()
        .chain(old_snap.open_ports.iter())
        .cloned()
        .collect();
    let mut desired_allowed: BTreeSet<String> = BTreeSet::new();
    for rule in new_rules {
        if rule.action == FirewallAppAction::Allow && !rule.service_or_port.is_empty() {
            desired_allowed.insert(rule.service_or_port.clone());
        }
    }

    for to_add in desired_allowed.difference(&old_allowed) {
        if is_port_spec(to_add) {
            if !is_valid_firewall_port_spec(to_add) {
                return Err(format!("Especificación de puerto inválida: {to_add}"));
            }
            ops.push(format!("add-port:{target_zone}:{to_add}"));
        } else {
            if !is_valid_firewall_name(to_add) {
                return Err(format!("Nombre de servicio inválido: {to_add}"));
            }
            ops.push(format!("add-service:{target_zone}:{to_add}"));
        }
    }
    for to_rem in old_allowed.difference(&desired_allowed) {
        if is_port_spec(to_rem) {
            if !is_valid_firewall_port_spec(to_rem) {
                return Err(format!("Especificación de puerto inválida: {to_rem}"));
            }
            ops.push(format!("remove-port:{target_zone}:{to_rem}"));
        } else {
            if !is_valid_firewall_name(to_rem) {
                return Err(format!("Nombre de servicio inválido: {to_rem}"));
            }
            ops.push(format!("remove-service:{target_zone}:{to_rem}"));
        }
    }

    if ops.is_empty() {
        return Ok(());
    }

    // Script completamente estático sin interpolación de datos de usuario; todos los valores
    // pasan como argumentos posicionales validados.
    const FIREWALL_BATCH_RUNNER: &str = r#"set -e
for op in "$@"; do
  case "$op" in
    set-default-zone:*)
      firewall-cmd "--set-default-zone=${op#set-default-zone:}"
      ;;
    stealth-on:*)
      firewall-cmd --permanent "--zone=${op#stealth-on:}" --add-icmp-block=echo-request
      sysctl -w net.ipv4.icmp_echo_ignore_all=1
      ;;
    stealth-off:*)
      firewall-cmd --permanent "--zone=${op#stealth-off:}" --remove-icmp-block=echo-request
      sysctl -w net.ipv4.icmp_echo_ignore_all=0
      ;;
    add-port:*)
      rest="${op#add-port:}"
      firewall-cmd --permanent "--zone=${rest%%:*}" "--add-port=${rest#*:}"
      ;;
    remove-port:*)
      rest="${op#remove-port:}"
      firewall-cmd --permanent "--zone=${rest%%:*}" "--remove-port=${rest#*:}"
      ;;
    add-service:*)
      rest="${op#add-service:}"
      firewall-cmd --permanent "--zone=${rest%%:*}" "--add-service=${rest#*:}"
      ;;
    remove-service:*)
      rest="${op#remove-service:}"
      firewall-cmd --permanent "--zone=${rest%%:*}" "--remove-service=${rest#*:}"
      ;;
    *)
      exit 2
      ;;
  esac
done
firewall-cmd --reload
"#;

    let mut cmd = Command::new("pkexec");
    cmd.args(["sh", "-c", FIREWALL_BATCH_RUNNER, "malus-firewall-batch"]);
    for op in &ops {
        cmd.arg(op);
    }
    let out = cmd.output().map_err(|e| format!("pkexec: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            "Operación cancelada o sin permisos.".to_string()
        } else {
            err
        })
    }
}

pub(crate) fn is_port_spec(s: &str) -> bool {
    s.contains('/')
        && s.split('/')
            .next()
            .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit() || c == '-'))
}

pub(crate) fn map_binary_to_firewalld_service(name: &str) -> Option<String> {
    let lower = name.trim().to_lowercase();
    match lower.as_str() {
        "cupsd" => Some("ipp".to_string()),
        "smbd" | "nmbd" => Some("samba".to_string()),
        "sshd" | "sshd-keygen-wrapper" => Some("ssh".to_string()),
        "avahi-daemon" => Some("mdns".to_string()),
        "cockpit-ws" => Some("cockpit".to_string()),
        _ if lower.contains("ssh") => Some("ssh".to_string()),
        _ if is_port_spec(&lower) => Some(lower),
        _ => None,
    }
}

fn collect_network_snapshot() -> NetworkOverviewSnapshot {
    let mut snap = NetworkOverviewSnapshot::default();

    // 1. Estado de Wi-Fi y "Otros servicios" desde nmcli
    if let Ok(radio) = run_nmcli(&["radio", "wifi"]) {
        snap.nm_available = true;
        let radio_on = radio.trim().eq_ignore_ascii_case("enabled");
        if !radio_on {
            snap.wifi_dot = ServiceDotState::Disconnected;
            snap.wifi_status = "Desactivado".to_string();
        } else if let Ok(dev_raw) =
            run_nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"])
        {
            let mut found_wifi = false;
            for line in dev_raw.lines() {
                let parts = split_nmcli_terse_row(line);
                if parts.len() >= 4 && parts[1] == "wifi" {
                    found_wifi = true;
                    if parts[2].starts_with("connected") {
                        snap.wifi_dot = ServiceDotState::Connected;
                        snap.wifi_status = "Conectado".to_string();
                    } else if parts[2].contains("connecting") {
                        snap.wifi_dot = ServiceDotState::Pending;
                        snap.wifi_status = "Conectando...".to_string();
                    } else {
                        snap.wifi_dot = ServiceDotState::Disconnected;
                        snap.wifi_status = "Sin conexión".to_string();
                    }
                    break;
                }
            }
            if !found_wifi {
                snap.wifi_dot = ServiceDotState::Disconnected;
                snap.wifi_status = "Sin conexión".to_string();
            }
        }
    }

    if snap.nm_available {
        let dev_raw =
            run_nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]).unwrap_or_default();
        let conn_raw = run_nmcli(&[
            "-t",
            "-f",
            "NAME,TYPE,DEVICE,AUTOCONNECT",
            "connection",
            "show",
        ])
        .unwrap_or_default();
        let mut services = parse_other_network_services(&dev_raw, &conn_raw);

        for svc in &mut services {
            if !svc.device.is_empty() {
                if let Ok(det) =
                    run_nmcli(&["-t", "-f", "GENERAL,IP4", "device", "show", &svc.device])
                {
                    enrich_service_device_details(svc, &det);
                }
            }
        }
        snap.other_services = services;
    }

    // 2. Estado de firewalld
    snap.firewall = collect_firewall_snapshot();
    snap
}

fn collect_firewall_snapshot() -> FirewallSnapshot {
    let mut fw = FirewallSnapshot::default();
    let prefs = FirewallPreferences::load();

    // Comprobar si firewalld responde o si el binario existe en el sistema
    match run_firewall_cmd(&["--state"]) {
        Ok(state) if state.trim() == "running" => {
            fw.firewalld_available = true;
            fw.active = true;
        }
        Ok(_) => {
            fw.firewalld_available = true;
            fw.active = false;
        }
        Err(err) => {
            // Si firewall-cmd reporta "not running", sigue disponible pero inactivo.
            if err.contains("not running") {
                fw.firewalld_available = true;
                fw.active = false;
            }
        }
    }

    if fw.active {
        let zone = run_firewall_cmd(&["--get-default-zone"])
            .unwrap_or_else(|_| "public".to_string())
            .trim()
            .to_string();
        fw.active_zone = zone.clone();
        fw.block_all_incoming = zone == "block" || zone == "drop";

        let services_raw = run_firewall_cmd(&["--list-services"]).unwrap_or_default();
        let ports_raw = run_firewall_cmd(&["--list-ports"]).unwrap_or_default();
        fw.open_services = services_raw
            .split_whitespace()
            .map(ToString::to_string)
            .collect();
        fw.open_ports = ports_raw
            .split_whitespace()
            .map(ToString::to_string)
            .collect();

        let icmp_blocked = run_firewall_cmd(&["--query-icmp-block=echo-request"]).is_ok();
        let sysctl_ignore = std::fs::read_to_string("/proc/sys/net/ipv4/icmp_echo_ignore_all")
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        fw.stealth_mode = icmp_blocked || sysctl_ignore;
    }

    fw.merged_rules = merge_firewall_rules(&fw.open_services, &fw.open_ports, &prefs.custom_rules);
    fw
}

pub(crate) fn merge_firewall_rules(
    open_services: &[String],
    open_ports: &[String],
    custom_rules: &[FirewallAppRule],
) -> Vec<FirewallAppRule> {
    let mut merged: Vec<FirewallAppRule> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for rule in custom_rules {
        seen.insert(rule.service_or_port.clone());
        merged.push(rule.clone());
    }
    for svc in open_services {
        if seen.insert(svc.clone()) {
            merged.push(FirewallAppRule {
                app_name: svc.clone(),
                service_or_port: svc.clone(),
                action: FirewallAppAction::Allow,
                is_custom_binary: false,
            });
        }
    }
    for port in open_ports {
        if seen.insert(port.clone()) {
            merged.push(FirewallAppRule {
                app_name: port.clone(),
                service_or_port: port.clone(),
                action: FirewallAppAction::Allow,
                is_custom_binary: false,
            });
        }
    }
    merged.sort_by(|a, b| a.app_name.to_lowercase().cmp(&b.app_name.to_lowercase()));
    merged
}

pub(crate) fn parse_other_network_services(
    dev_raw: &str,
    conn_raw: &str,
) -> Vec<NetworkServiceEntry> {
    let mut services: Vec<NetworkServiceEntry> = Vec::new();
    let mut by_conn: HashMap<String, (String, bool)> = HashMap::new();

    for line in conn_raw.lines() {
        let parts = split_nmcli_terse_row(line);
        if parts.len() >= 4 {
            by_conn.insert(
                parts[0].clone(),
                (parts[1].clone(), parts[3].eq_ignore_ascii_case("yes")),
            );
        }
    }

    let mut seen_conns: BTreeSet<String> = BTreeSet::new();
    for line in dev_raw.lines() {
        let parts = split_nmcli_terse_row(line);
        if parts.len() < 4 {
            continue;
        }
        let dev = &parts[0];
        let dev_type = &parts[1];
        let state = &parts[2];
        let conn = &parts[3];

        if matches!(
            dev_type.as_str(),
            "wifi" | "loopback" | "wifi-p2p" | "dummy"
        ) {
            continue;
        }
        if !conn.is_empty() {
            seen_conns.insert(conn.clone());
        }

        let (dot, status_label) = if state.starts_with("connected") {
            (ServiceDotState::Connected, "Conectado".to_string())
        } else if state.contains("connecting") {
            (ServiceDotState::Pending, "Conectando...".to_string())
        } else {
            (ServiceDotState::Disconnected, "Sin conexión".to_string())
        };

        let (display_name, icon_name) = friendly_service_label_and_icon(dev, dev_type, conn);
        let autoconnect = by_conn.get(conn).map(|(_, a)| *a).unwrap_or(false);

        services.push(NetworkServiceEntry {
            id: format!("dev:{dev}"),
            display_name,
            device: dev.clone(),
            dev_type: dev_type.clone(),
            connection_name: conn.clone(),
            state_raw: state.clone(),
            dot,
            status_label,
            icon_name,
            hwaddr: String::new(),
            ipv4: Vec::new(),
            gateway: String::new(),
            dns: Vec::new(),
            route_metric: None,
            autoconnect,
        });
    }

    // Perfiles guardados no Wi-Fi sin dispositivo asignado (por ej. Puente Thunderbolt sin cable conectado)
    for line in conn_raw.lines() {
        let parts = split_nmcli_terse_row(line);
        if parts.len() < 4 {
            continue;
        }
        let name = &parts[0];
        let ctype = &parts[1];
        let device = &parts[2];
        let auto = parts[3].eq_ignore_ascii_case("yes");
        if ctype.contains("wireless")
            || ctype == "loopback"
            || ctype == "vpn"
            || ctype == "wireguard"
        {
            continue;
        }
        if seen_conns.contains(name) || !device.is_empty() && device != "--" {
            continue;
        }
        let (display_name, icon_name) = friendly_service_label_and_icon("", ctype, name);
        services.push(NetworkServiceEntry {
            id: format!("conn:{name}"),
            display_name,
            device: String::new(),
            dev_type: ctype.clone(),
            connection_name: name.clone(),
            state_raw: "disconnected".to_string(),
            dot: ServiceDotState::Disconnected,
            status_label: "Sin conexión".to_string(),
            icon_name,
            hwaddr: String::new(),
            ipv4: Vec::new(),
            gateway: String::new(),
            dns: Vec::new(),
            route_metric: None,
            autoconnect: auto,
        });
    }

    services.sort_by(|a, b| {
        let rank = |d: ServiceDotState| match d {
            ServiceDotState::Connected => 0,
            ServiceDotState::Pending => 1,
            ServiceDotState::Disconnected => 2,
            ServiceDotState::Unavailable => 3,
        };
        rank(a.dot).cmp(&rank(b.dot)).then_with(|| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        })
    });
    services
}

fn friendly_service_label_and_icon(dev: &str, dev_type: &str, conn: &str) -> (String, String) {
    let lower_all = format!("{} {} {}", dev, dev_type, conn).to_lowercase();
    if lower_all.contains("thunderbolt") || lower_all.contains("tbt") {
        let title = if conn.is_empty() {
            "Puente Thunderbolt".to_string()
        } else {
            conn.to_string()
        };
        (title, "thunderbolt-symbolic".to_string())
    } else if dev_type == "bridge" || lower_all.contains("bridge") || lower_all.contains("puente") {
        let title = if !conn.is_empty() {
            conn.to_string()
        } else {
            format!("Puente ({dev})")
        };
        (title, "network-workgroup-symbolic".to_string())
    } else if dev_type == "ethernet" || dev_type == "802-3-ethernet" {
        let title = if !conn.is_empty() {
            conn.to_string()
        } else if !dev.is_empty() {
            format!("Ethernet ({dev})")
        } else {
            "Ethernet".to_string()
        };
        (title, "network-wired-symbolic".to_string())
    } else if dev_type == "gsm" || dev_type == "cdma" {
        (
            "Red móvil".to_string(),
            "network-cellular-symbolic".to_string(),
        )
    } else {
        let title = if !conn.is_empty() {
            conn.to_string()
        } else {
            dev.to_string()
        };
        (title, "network-wired-symbolic".to_string())
    }
}

fn enrich_service_device_details(svc: &mut NetworkServiceEntry, raw: &str) {
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let val = v.replace("\\:", ":");
            if k == "GENERAL.HWADDR" {
                svc.hwaddr = val;
            } else if k.starts_with("IP4.ADDRESS") {
                svc.ipv4.push(val);
            } else if k == "IP4.GATEWAY" && val != "--" {
                svc.gateway = val;
            } else if k.starts_with("IP4.DNS") {
                svc.dns.push(val);
            }
        }
    }
}

fn split_nmcli_terse_row(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == ':' || next == '\\' {
                    cur.push(next);
                    chars.next();
                    continue;
                }
            }
            cur.push(c);
        } else if c == ':' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_other_network_services_excludes_wifi_and_detects_thunderbolt_and_ethernet() {
        let devs = "wlp4s0:wifi:connected:Mega_5G\nlo:loopback:connected (externally):lo\nenp6s0:ethernet:unavailable:\n";
        let conns = "Mega_5G:802-11-wireless:wlp4s0:yes\nPuente Thunderbolt:bridge::no\nOficina Ethernet:802-3-ethernet:enp6s0:yes\n";
        let parsed = parse_other_network_services(devs, conns);
        assert_eq!(parsed.len(), 2);
        assert!(parsed
            .iter()
            .all(|s| s.dev_type != "wifi" && s.dev_type != "loopback"));
        let tb = parsed
            .iter()
            .find(|s| s.display_name == "Puente Thunderbolt")
            .unwrap();
        assert_eq!(tb.icon_name, "thunderbolt-symbolic");
        assert_eq!(tb.dot, ServiceDotState::Disconnected);
        assert_eq!(tb.status_label, "Sin conexión");
    }

    #[test]
    fn merges_firewalld_rules_and_maps_common_daemons() {
        assert_eq!(
            map_binary_to_firewalld_service("cupsd").as_deref(),
            Some("ipp")
        );
        assert_eq!(
            map_binary_to_firewalld_service("smbd").as_deref(),
            Some("samba")
        );
        assert_eq!(
            map_binary_to_firewalld_service("8080/tcp").as_deref(),
            Some("8080/tcp")
        );

        let open_svcs = vec!["ssh".to_string(), "mdns".to_string()];
        let open_ports = vec!["8080/tcp".to_string()];
        let custom = vec![FirewallAppRule {
            app_name: "python3".to_string(),
            service_or_port: "python3".to_string(),
            action: FirewallAppAction::Allow,
            is_custom_binary: true,
        }];
        let merged = merge_firewall_rules(&open_svcs, &open_ports, &custom);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].app_name, "8080/tcp");
        assert_eq!(merged[2].app_name, "python3");
    }

    #[test]
    fn validates_firewall_names_and_port_specs() {
        assert!(is_valid_firewall_name("public"));
        assert!(is_valid_firewall_name("dhcpv6-client"));
        assert!(!is_valid_firewall_name("--bad"));
        assert!(!is_valid_firewall_name("ssh;id"));
        assert!(!is_valid_firewall_name(""));

        assert!(is_valid_firewall_port_spec("8080/tcp"));
        assert!(is_valid_firewall_port_spec("5000-5010/udp"));
        assert!(!is_valid_firewall_port_spec("0/tcp"));
        assert!(!is_valid_firewall_port_spec("8080/http"));
        assert!(!is_valid_firewall_port_spec("8080/tcp;rm"));
    }
}
