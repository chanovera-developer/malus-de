use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, CheckButton, DrawingArea, Entry, Image, Label,
    Orientation, Overlay, PolicyType, ScrolledWindow, Stack,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use crate::wifi_pane::{
    detail_kv_row, make_help_circle_button, make_popover_action_row, make_squircle_badge,
    make_squircle_button, make_squircle_colored_badge, wrap_squircle_card,
    wrap_squircle_round_control, RoundControlStyle, WIFI_BADGE_SQUIRCLE_RADIUS,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

const GRAY_TOP: (f64, f64, f64) = (0.612, 0.620, 0.647);
const GRAY_BOT: (f64, f64, f64) = (0.486, 0.494, 0.522);
const GREEN_TOP: (f64, f64, f64) = (0.25, 0.82, 0.42);
const GREEN_BOT: (f64, f64, f64) = (0.16, 0.68, 0.31);
const TEAL_TOP: (f64, f64, f64) = (0.25, 0.72, 0.68);
const TEAL_BOT: (f64, f64, f64) = (0.16, 0.58, 0.54);
const ORANGE_TOP: (f64, f64, f64) = (0.99, 0.58, 0.13);
const ORANGE_BOT: (f64, f64, f64) = (0.94, 0.46, 0.04);
const PURPLE_TOP: (f64, f64, f64) = (0.56, 0.38, 0.94);
const PURPLE_BOT: (f64, f64, f64) = (0.44, 0.27, 0.84);
const RED_TOP: (f64, f64, f64) = (0.98, 0.32, 0.28);
const RED_BOT: (f64, f64, f64) = (0.86, 0.20, 0.18);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralPreferences {
    pub auto_updates_enabled: bool,
    pub auto_empty_trash_30d: bool,
    pub airdrop_mode: usize, // 0 = Nadie, 1 = Sólo contactos, 2 = Todos
    pub handoff_enabled: bool,
    pub iphone_widgets_enabled: bool,
    pub airplay_receiver_enabled: bool,
    pub airplay_access_mode: usize,
    pub airplay_require_password: bool,
    pub airplay_password: String,
    pub autofill_passwords: bool,
    pub autofill_keyring_provider: bool,
    pub delete_verification_codes: bool,
    pub ntp_server: String,
    pub clock_24h: bool,
    pub auto_timezone: bool,
    pub preferred_languages: Vec<String>,
    pub region: String,
    pub calendar: String,
    pub celsius: bool,
    pub measurement_system: usize, // 0 = Métrico, 1 = Estadounidense, 2 = Imperial
    pub first_weekday: usize,      // 0 = Domingo, 1 = Lunes, 2 = Sábado
    pub date_format_idx: usize,
    pub number_format_idx: usize,
    pub list_sort_idx: usize,
    pub address_term_idx: usize,
    pub share_address_term: bool,
    pub live_text_ocr: bool,
}

impl Default for GeneralPreferences {
    fn default() -> Self {
        Self {
            auto_updates_enabled: true,
            auto_empty_trash_30d: false,
            airdrop_mode: 2,
            handoff_enabled: true,
            iphone_widgets_enabled: true,
            airplay_receiver_enabled: true,
            airplay_access_mode: 0,
            airplay_require_password: true,
            airplay_password: String::new(),
            autofill_passwords: true,
            autofill_keyring_provider: true,
            delete_verification_codes: false,
            ntp_server: "pool.ntp.org".to_string(),
            clock_24h: false,
            auto_timezone: true,
            preferred_languages: vec!["Español (México) — principal".to_string()],
            region: "México".to_string(),
            calendar: "Gregoriano".to_string(),
            celsius: true,
            measurement_system: 0,
            first_weekday: 0,
            date_format_idx: 0,
            number_format_idx: 0,
            list_sort_idx: 0,
            address_term_idx: 0,
            share_address_term: false,
            live_text_ocr: true,
        }
    }
}

impl GeneralPreferences {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/general-settings.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(parsed) = toml::from_str::<Self>(&raw) {
                return parsed;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(raw) = toml::to_string_pretty(self) {
            let _ = fs::write(path, raw);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SystemInfoSnapshot {
    pub hostname: String,
    pub product_name: String,
    pub vendor_year: String,
    pub cpu_model: String,
    pub memory_gb: String,
    pub serial_or_machine_id: String,
    pub os_pretty_name: String,
    pub kernel_version: String,
    pub displays: Vec<(String, String)>,
    pub root_device: String,
    pub root_total_bytes: u64,
    pub root_used_bytes: u64,
    pub root_avail_bytes: u64,
    pub apps_bytes: u64,
    pub trash_bytes: u64,
    pub docs_bytes: u64,
    pub photos_bytes: u64,
    pub downloads_bytes: u64,
    pub home_other_bytes: u64,
    pub system_data_bytes: u64,
    pub os_base_bytes: u64,
    pub ntp_active: bool,
    pub timezone: String,
    pub sshd_active: bool,
    pub cups_active: bool,
    pub smb_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutostartEntry {
    pub file_path: PathBuf,
    pub name: String,
    pub exec: String,
    pub kind_label: String,
}

pub fn is_general_subpanel(id: &str) -> bool {
    matches!(
        id,
        "about"
            | "general-info"
            | "general-update"
            | "general-storage"
            | "general-warranty"
            | "general-airdrop"
            | "general-autofill"
            | "general-sharing"
            | "general-startup-disk"
            | "general-login-items"
            | "general-datetime"
            | "general-locale"
            | "general-timemachine"
            | "general-mdm"
            | "general-reset"
    )
}

pub fn general_subpanel_title(id: &str) -> Option<&'static str> {
    match id {
        "about" => Some("General"),
        "general-info" => Some("Información"),
        "general-update" => Some("Actualización de software"),
        "general-storage" => Some("Almacenamiento"),
        "general-warranty" => Some("AppleCare y garantía"),
        "general-airdrop" => Some("AirDrop y continuidad"),
        "general-autofill" => Some("Autorrelleno y contraseñas"),
        "general-sharing" => Some("Compartir"),
        "general-startup-disk" => Some("Disco de arranque"),
        "general-login-items" => Some("Elementos de inicio"),
        "general-datetime" => Some("Fecha y hora"),
        "general-locale" => Some("Idioma y región"),
        "general-timemachine" => Some("Time Machine"),
        "general-mdm" => Some("Admón. de dispositivos"),
        "general-reset" => Some("Transferir o restablecer"),
        _ => None,
    }
}

/// Devuelve un cargador bajo demanda para `General` (`"about"`) y sus 14 subvistas.
/// Cada subvista se construye una sola vez al visitarla, queda conservada en el `Stack`,
/// y al volver a entrar únicamente refresca sus datos vivos.
pub fn create_general_pane_loader(
    parent_window: &ApplicationWindow,
    stack: &Stack,
    navigate: Rc<dyn Fn(&str)>,
) -> Rc<dyn Fn(&str)> {
    let win = parent_window.clone();
    let stack_c = stack.clone();
    let prefs_cell: Rc<RefCell<Option<Rc<RefCell<GeneralPreferences>>>>> =
        Rc::new(RefCell::new(None));
    let sys_cell: Rc<RefCell<Option<Rc<RefCell<SystemInfoSnapshot>>>>> =
        Rc::new(RefCell::new(None));
    let refreshers: Rc<RefCell<std::collections::HashMap<String, Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));

    Rc::new(move |target: &str| {
        if stack_c.child_by_name(target).is_none() {
            let prefs = {
                let mut slot = prefs_cell.borrow_mut();
                slot.get_or_insert_with(|| Rc::new(RefCell::new(GeneralPreferences::load())))
                    .clone()
            };
            let sys_snap = {
                let mut slot = sys_cell.borrow_mut();
                slot.get_or_insert_with(|| Rc::new(RefCell::new(collect_system_info_fast())))
                    .clone()
            };

            let built: Option<(ScrolledWindow, Option<Rc<dyn Fn()>>)> = match target {
                "about" => Some((build_general_hub_pane(&navigate), None)),
                "general-info" => {
                    let (p, r) = build_info_subpane(&win, &sys_snap, &navigate);
                    Some((p, Some(r)))
                }
                "general-update" => {
                    let (p, r) = build_software_update_subpane(&win, &prefs, &sys_snap);
                    Some((p, Some(r)))
                }
                "general-storage" => {
                    let (p, r) = build_storage_subpane(&win, &prefs, &sys_snap);
                    Some((p, Some(r)))
                }
                "general-warranty" => Some((
                    build_pending_feature_subpane(
                        "emblem-favorite-symbolic",
                        RED_TOP,
                        RED_BOT,
                        "Garantía y soporte del equipo",
                        "Consulta el estado de cobertura de hardware, diagnóstico de componentes OEM y soporte técnico.",
                        "En Linux no existe un servicio unificado de garantía AppleCare. Futura integración prevista con metadatos SMBIOS/DMI y fwupd.",
                    ),
                    None,
                )),
                "general-airdrop" => Some((build_airdrop_subpane(&win, &prefs), None)),
                "general-autofill" => Some((build_autofill_subpane(&win, &prefs), None)),
                "general-sharing" => {
                    let (p, r) = build_sharing_subpane(&win, &sys_snap);
                    Some((p, Some(r)))
                }
                "general-startup-disk" => {
                    let (p, r) = build_startup_disk_subpane(&win, &sys_snap);
                    Some((p, Some(r)))
                }
                "general-login-items" => {
                    let (p, r) = build_login_items_subpane(&win);
                    Some((p, Some(r)))
                }
                "general-datetime" => {
                    let (p, r) = build_datetime_subpane(&win, &prefs, &sys_snap);
                    Some((p, Some(r)))
                }
                "general-locale" => {
                    let (p, r) = build_locale_subpane(&win, &prefs);
                    Some((p, Some(r)))
                }
                "general-timemachine" => Some((
                    build_pending_feature_subpane(
                        "document-open-recent-symbolic",
                        GREEN_TOP,
                        GREEN_BOT,
                        "Time Machine (Copias de seguridad)",
                        "Realiza respaldos incrementales automáticos de tus archivos personales y configuraciones del sistema.",
                        "Pendiente de enlazar con instantáneas Btrfs / BorgBackup / rsync en Malus DE.",
                    ),
                    None,
                )),
                "general-mdm" => Some((
                    build_pending_feature_subpane(
                        "security-high-symbolic",
                        GRAY_TOP,
                        GRAY_BOT,
                        "Administración de dispositivos (MDM)",
                        "Perfiles de configuración empresarial, certificados corporativos y políticas de seguridad gestionadas.",
                        "Pendiente de adaptación mediante políticas FreeIPA / Ansible / Fleet en entornos Linux.",
                    ),
                    None,
                )),
                "general-reset" => Some((build_reset_subpane(&win), None)),
                _ => None,
            };

            if let Some((pane, refresher)) = built {
                crate::configure_right_pane_widget(pane.upcast_ref());
                stack_c.add_named(&pane, Some(target));
                if let Some(ref_fn) = refresher {
                    refreshers.borrow_mut().insert(target.to_string(), ref_fn);
                }
            }
        }

        let refresher = refreshers.borrow().get(target).cloned();
        if let Some(ref_fn) = refresher {
            ref_fn();
        }
    })
}

pub(crate) fn make_pending_badge() -> Label {
    Label::builder()
        .label("por implementar")
        .css_classes(["gen-pending-badge"])
        .valign(Align::Center)
        .build()
}

fn make_scrolled_pane() -> (ScrolledWindow, GtkBox) {
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
    scrolled.set_child(Some(&pane));
    (scrolled, pane)
}

fn build_hub_nav_row(
    badge: Overlay,
    title: &str,
    pending: bool,
    target_id: &'static str,
    navigate: &Rc<dyn Fn(&str)>,
) -> Button {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    badge.set_valign(Align::Center);
    row.append(&badge);

    let lbl = Label::builder()
        .label(title)
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    row.append(&lbl);

    if pending {
        row.append(&make_pending_badge());
    }

    row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );

    let btn = Button::builder()
        .child(&row)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    let nav = navigate.clone();
    btn.connect_clicked(move |_| nav(target_id));
    btn
}

// ============================================================================
// 1. Hub principal "General" (General01.png)
// ============================================================================

fn build_general_hub_pane(navigate: &Rc<dyn Fn(&str)>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    // Tarjeta hero centrada superior
    let hero_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .halign(Align::Center)
        .css_classes(["gen-hero-card"])
        .build();
    let hero_badge =
        make_squircle_colored_badge(52, "emblem-system-symbolic", 28, 18.0, GRAY_TOP, GRAY_BOT);
    hero_badge.set_halign(Align::Center);
    hero_box.append(&hero_badge);
    hero_box.append(
        &Label::builder()
            .label("General")
            .css_classes(["gen-hero-title"])
            .halign(Align::Center)
            .build(),
    );
    hero_box.append(
        &Label::builder()
            .label("Administra tu configuración y tus preferencias generales para el equipo, como las actualizaciones de software, el idioma del dispositivo, AirDrop y más.")
            .css_classes(["card-section-sub"])
            .justify(gtk4::Justification::Center)
            .wrap(true)
            .max_width_chars(62)
            .halign(Align::Center)
            .build(),
    );
    let hero_wrap_box = GtkBox::builder().orientation(Orientation::Vertical).build();
    hero_wrap_box.append(&hero_box);
    pane.append(&wrap_squircle_card(
        &hero_wrap_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Grupo 1: Información / Actualización de software / Almacenamiento
    let g1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    g1.append(&build_hub_nav_row(
        make_squircle_colored_badge(
            28,
            "computer-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            GRAY_TOP,
            GRAY_BOT,
        ),
        "Información",
        false,
        "general-info",
        navigate,
    ));
    g1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    g1.append(&build_hub_nav_row(
        make_squircle_colored_badge(
            28,
            "emblem-system-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            GRAY_TOP,
            GRAY_BOT,
        ),
        "Actualización de software",
        false,
        "general-update",
        navigate,
    ));
    g1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    g1.append(&build_hub_nav_row(
        make_squircle_colored_badge(
            28,
            "drive-harddisk-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            GRAY_TOP,
            GRAY_BOT,
        ),
        "Almacenamiento",
        false,
        "general-storage",
        navigate,
    ));
    pane.append(&wrap_squircle_card(&g1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Grupo 2: AppleCare y garantía
    let g2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    g2.append(&build_hub_nav_row(
        make_squircle_colored_badge(
            28,
            "emblem-favorite-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            RED_TOP,
            RED_BOT,
        ),
        "AppleCare y garantía",
        true,
        "general-warranty",
        navigate,
    ));
    pane.append(&wrap_squircle_card(&g2, WIFI_CARD_SQUIRCLE_RADIUS));

    // Grupo 3: AirDrop y continuidad
    let g3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    g3.append(&build_hub_nav_row(
        make_squircle_badge(
            28,
            "network-wireless-hotspot-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
        ),
        "AirDrop y continuidad",
        true,
        "general-airdrop",
        navigate,
    ));
    pane.append(&wrap_squircle_card(&g3, WIFI_CARD_SQUIRCLE_RADIUS));

    // Grupo 4: Autorrelleno, Compartir, Disco de arranque, Elementos de inicio, Fecha y hora, Idioma y región, Time Machine
    let g4 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let g4_items: [(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    ); 7] = [
        (
            "dialog-password-symbolic",
            GRAY_TOP,
            GRAY_BOT,
            "Autorrelleno y contraseñas",
            false,
            "general-autofill",
        ),
        (
            "folder-publicshare-symbolic",
            GRAY_TOP,
            GRAY_BOT,
            "Compartir",
            false,
            "general-sharing",
        ),
        (
            "drive-harddisk-symbolic",
            GRAY_TOP,
            GRAY_BOT,
            "Disco de arranque",
            false,
            "general-startup-disk",
        ),
        (
            "view-list-symbolic",
            GRAY_TOP,
            GRAY_BOT,
            "Elementos de inicio y extensiones",
            false,
            "general-login-items",
        ),
        (
            "x-office-calendar-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Fecha y hora",
            false,
            "general-datetime",
        ),
        (
            "preferences-desktop-locale-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Idioma y región",
            false,
            "general-locale",
        ),
        (
            "document-open-recent-symbolic",
            TEAL_TOP,
            TEAL_BOT,
            "Time Machine",
            true,
            "general-timemachine",
        ),
    ];
    for (idx, (ico, top, bot, title, pending, target)) in g4_items.into_iter().enumerate() {
        if idx > 0 {
            g4.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        g4.append(&build_hub_nav_row(
            make_squircle_colored_badge(28, ico, 15, WIFI_BADGE_SQUIRCLE_RADIUS, top, bot),
            title,
            pending,
            target,
            navigate,
        ));
    }
    pane.append(&wrap_squircle_card(&g4, WIFI_CARD_SQUIRCLE_RADIUS));

    // Grupo 5: Admón. de dispositivos
    let g5 = GtkBox::builder().orientation(Orientation::Vertical).build();
    g5.append(&build_hub_nav_row(
        make_squircle_colored_badge(
            28,
            "security-high-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            GRAY_TOP,
            GRAY_BOT,
        ),
        "Admón. de dispositivos",
        true,
        "general-mdm",
        navigate,
    ));
    pane.append(&wrap_squircle_card(&g5, WIFI_CARD_SQUIRCLE_RADIUS));

    // Grupo 6: Transferir o restablecer
    let g6 = GtkBox::builder().orientation(Orientation::Vertical).build();
    g6.append(&build_hub_nav_row(
        make_squircle_colored_badge(
            28,
            "view-refresh-symbolic",
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            GRAY_TOP,
            GRAY_BOT,
        ),
        "Transferir o restablecer",
        false,
        "general-reset",
        navigate,
    ));
    pane.append(&wrap_squircle_card(&g6, WIFI_CARD_SQUIRCLE_RADIUS));

    scrolled
}

// ============================================================================
// 2. Subvista "Información" (General02.png)
// ============================================================================

fn build_info_subpane(
    parent_window: &ApplicationWindow,
    sys_snap: &Rc<RefCell<SystemInfoSnapshot>>,
    navigate: &Rc<dyn Fn(&str)>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let hero = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .halign(Align::Center)
        .margin_top(8)
        .margin_bottom(6)
        .build();
    hero.append(
        &Image::builder()
            .icon_name("computer-symbolic")
            .pixel_size(64)
            .margin_bottom(4)
            .build(),
    );
    let model_lbl = Label::builder()
        .label("Equipo Malus Linux")
        .css_classes(["gen-device-hero-title"])
        .halign(Align::Center)
        .build();
    let vendor_lbl = Label::builder()
        .label("x86_64")
        .css_classes(["card-section-sub"])
        .halign(Align::Center)
        .build();
    hero.append(&model_lbl);
    hero.append(&vendor_lbl);
    pane.append(&hero);

    let hw_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (r_name, val_name) = info_kv_row("Nombre", "—");
    let (r_chip, val_chip) = info_kv_row("Chip", "—");
    let (r_mem, val_mem) = info_kv_row("Memoria", "—");
    let (r_ser, val_ser) = info_kv_row("Número de serie", "—");

    let cov_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    cov_row.append(
        &Label::builder()
            .label("Cobertura de garantía")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    cov_row.append(&make_pending_badge());
    let (cov_btn_wrap, cov_btn) = make_squircle_button("Detalles...", false, None);
    let nav_cov = navigate.clone();
    cov_btn.connect_clicked(move |_| nav_cov("general-warranty"));
    cov_row.append(&cov_btn_wrap);

    hw_card.append(&r_name);
    hw_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    hw_card.append(&r_chip);
    hw_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    hw_card.append(&r_mem);
    hw_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    hw_card.append(&r_ser);
    hw_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    hw_card.append(&cov_row);
    pane.append(&wrap_squircle_card(&hw_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Sección Sistema Operativo
    pane.append(
        &Label::builder()
            .label("Sistema")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let os_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let os_row1 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let os_badge =
        make_squircle_badge(30, "emblem-system-symbolic", 16, WIFI_BADGE_SQUIRCLE_RADIUS);
    let os_name_lbl = Label::builder()
        .label("Malus DE · Fedora Linux")
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let os_ver_lbl = Label::builder()
        .label("Versión 1.1")
        .css_classes(["card-section-sub"])
        .build();
    os_row1.append(&os_badge);
    os_row1.append(&os_name_lbl);
    os_row1.append(&os_ver_lbl);
    os_card.append(&os_row1);
    os_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let os_row2 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let os_r2_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    os_r2_text.append(
        &Label::builder()
            .label("Binarios ELF x86_64 y contenedores Flatpak")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    os_r2_text.append(
        &Label::builder()
            .label("Ejecución nativa bajo Wayland (Smithay DRM/KMS) con soporte XWayland.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    os_row2.append(&os_r2_text);
    os_card.append(&os_row2);
    pane.append(&wrap_squircle_card(&os_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Sección Pantallas
    pane.append(
        &Label::builder()
            .label("Pantallas")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let disp_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let disp_list_box = GtkBox::builder().orientation(Orientation::Vertical).build();
    disp_card.append(&disp_list_box);
    let disp_footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (disp_btn_wrap, disp_btn) =
        make_squircle_button("Configuración de Pantallas...", false, None);
    let nav_disp = navigate.clone();
    disp_btn.connect_clicked(move |_| nav_disp("displays"));
    disp_footer.append(&disp_btn_wrap);
    disp_card.append(&disp_footer);
    pane.append(&wrap_squircle_card(&disp_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Sección Almacenamiento
    pane.append(
        &Label::builder()
            .label("Almacenamiento")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let stor_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let stor_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    stor_row.append(
        &Image::builder()
            .icon_name("drive-harddisk-symbolic")
            .pixel_size(24)
            .build(),
    );
    let stor_dev_lbl = Label::builder()
        .label("Volumen raíz (/)")
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let stor_cap_lbl = Label::builder()
        .label("—")
        .css_classes(["card-section-sub"])
        .build();
    stor_row.append(&stor_dev_lbl);
    stor_row.append(&stor_cap_lbl);
    stor_card.append(&stor_row);

    let stor_footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (stor_btn_wrap, stor_btn) =
        make_squircle_button("Configuración de almacenamiento...", false, None);
    let nav_stor = navigate.clone();
    stor_btn.connect_clicked(move |_| nav_stor("general-storage"));
    stor_footer.append(&stor_btn_wrap);
    stor_card.append(&stor_footer);
    pane.append(&wrap_squircle_card(&stor_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Pie centrado: Reporte del sistema...
    let bottom_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .halign(Align::Center)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let (rep_wrap, rep_btn) = make_squircle_button("Reporte del sistema...", false, None);
    rep_wrap.set_halign(Align::Center);
    let win_rep = parent_window.clone();
    let snap_rep = sys_snap.clone();
    rep_btn.connect_clicked(move |_| {
        show_system_report_dialog(&win_rep, &snap_rep.borrow());
    });
    bottom_box.append(&rep_wrap);
    bottom_box.append(
        &Label::builder()
            .label("Contrato de licencia de software · Malus Desktop Environment (Linux/Wayland)")
            .css_classes(["card-section-sub"])
            .halign(Align::Center)
            .build(),
    );
    pane.append(&bottom_box);

    let refresh: Rc<dyn Fn()> = {
        let snap_ref = sys_snap.clone();
        let m_lbl = model_lbl;
        let v_lbl = vendor_lbl;
        let n_val = val_name;
        let c_val = val_chip;
        let mem_val = val_mem;
        let s_val = val_ser;
        let os_n = os_name_lbl;
        let os_v = os_ver_lbl;
        let d_box = disp_list_box;
        let sd_lbl = stor_dev_lbl;
        let sc_lbl = stor_cap_lbl;
        Rc::new(move || {
            let s = collect_system_info_fast();
            *snap_ref.borrow_mut() = s.clone();
            m_lbl.set_label(&s.product_name);
            v_lbl.set_label(&s.vendor_year);
            n_val.set_label(&s.hostname);
            c_val.set_label(&s.cpu_model);
            mem_val.set_label(&s.memory_gb);
            s_val.set_label(&s.serial_or_machine_id);
            os_n.set_label(&s.os_pretty_name);
            os_v.set_label(&format!("Kernel {}", s.kernel_version));
            sd_lbl.set_label(&s.root_device);
            sc_lbl.set_label(&format!(
                "{} disponible(s) de {}",
                format_bytes_gb(s.root_avail_bytes),
                format_bytes_gb(s.root_total_bytes)
            ));

            while let Some(c) = d_box.first_child() {
                d_box.remove(&c);
            }
            for (idx, (dname, dres)) in s.displays.iter().enumerate() {
                if idx > 0 {
                    d_box.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let r = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .css_classes(["wifi-pref-row"])
                    .build();
                r.append(
                    &Image::builder()
                        .icon_name("video-display-symbolic")
                        .pixel_size(20)
                        .build(),
                );
                r.append(
                    &Label::builder()
                        .label(dname)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .build(),
                );
                r.append(
                    &Label::builder()
                        .label(dres)
                        .css_classes(["card-section-sub"])
                        .build(),
                );
                d_box.append(&r);
            }
        })
    };
    refresh();
    (scrolled, refresh)
}

fn info_kv_row(key: &str, val: &str) -> (GtkBox, Label) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(key)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let val_lbl = Label::builder()
        .label(val)
        .css_classes(["card-section-sub"])
        .xalign(1.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    row.append(&val_lbl);
    (row, val_lbl)
}

fn show_system_report_dialog(parent: &ApplicationWindow, snap: &SystemInfoSnapshot) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Reporte del sistema")
        .default_width(460)
        .build();
    crate::attach_modal_scrim(&dialog, parent);

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
    card.append(&detail_kv_row("Equipo", &snap.product_name));
    card.append(&detail_kv_row("Hostname", &snap.hostname));
    card.append(&detail_kv_row("Procesador", &snap.cpu_model));
    card.append(&detail_kv_row("Memoria RAM", &snap.memory_gb));
    card.append(&detail_kv_row("Sistema Operativo", &snap.os_pretty_name));
    card.append(&detail_kv_row("Kernel Linux", &snap.kernel_version));
    card.append(&detail_kv_row("Volumen raíz", &snap.root_device));
    card.append(&detail_kv_row("Zona horaria", &snap.timezone));
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Listo", true, None);
    ok_wrap.set_halign(Align::End);
    let d_c = dialog.clone();
    ok_btn.connect_clicked(move |_| d_c.close());
    vbox.append(&ok_wrap);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

// ============================================================================
// 3. Subvista "Actualización de software" (General03.png)
// ============================================================================

fn build_software_update_subpane(
    parent_window: &ApplicationWindow,
    prefs: &Rc<RefCell<GeneralPreferences>>,
    sys_snap: &Rc<RefCell<SystemInfoSnapshot>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let top_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    let badge = make_squircle_colored_badge(
        34,
        "emblem-ok-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        GREEN_TOP,
        GREEN_BOT,
    );
    badge.set_valign(Align::Center);
    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let status_lbl = Label::builder()
        .label("Tu sistema está actualizado.")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let ver_lbl = Label::builder()
        .label(&format!(
            "{} · Kernel {}",
            sys_snap.borrow().os_pretty_name,
            sys_snap.borrow().kernel_version
        ))
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    text_col.append(&status_lbl);
    text_col.append(&ver_lbl);

    let (check_wrap, check_btn) = make_squircle_button("Buscar actualizaciones", false, None);
    top_row.append(&badge);
    top_row.append(&text_col);
    top_row.append(&check_wrap);
    top_card.append(&top_row);
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta 2: Actualizaciones automáticas
    let auto_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let auto_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    auto_row.append(
        &Label::builder()
            .label("Actualizaciones automáticas")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let auto_state_lbl = Label::builder()
        .label(if prefs.borrow().auto_updates_enabled {
            "Activadas"
        } else {
            "Desactivadas"
        })
        .css_classes(["card-section-sub"])
        .build();
    let info_lbl = Label::builder()
        .label("i")
        .css_classes(["bt-info-glyph"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let info_btn = Button::builder()
        .child(&info_lbl)
        .css_classes(["wifi-more-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();
    let info_wrap = wrap_squircle_round_control(&info_btn, 20, RoundControlStyle::CircleOutline);
    auto_row.append(&auto_state_lbl);
    auto_row.append(&info_wrap);
    auto_card.append(&auto_row);
    pane.append(&wrap_squircle_card(&auto_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("El uso de este software está sujeto a las licencias de código abierto incluidas con los paquetes de la distribución.")
            .css_classes(["card-section-sub"])
            .justify(gtk4::Justification::Center)
            .wrap(true)
            .halign(Align::Center)
            .margin_top(6)
            .build(),
    );

    let help_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (help_wrap, _) =
        make_help_circle_button("Comprueba actualizaciones mediante dnf check-update");
    help_row.append(&help_wrap);
    pane.append(&help_row);

    {
        let st_c = status_lbl.clone();
        let btn_c = check_btn.clone();
        check_btn.connect_clicked(move |_| {
            btn_c.set_sensitive(false);
            st_c.set_label("Buscando actualizaciones en repositorios (dnf)...");
            let (tx, rx) = async_channel::bounded::<String>(1);
            std::thread::spawn(move || {
                let msg = match Command::new("dnf")
                    .args(["check-update", "-q", "--cacheonly"])
                    .output()
                {
                    Ok(out) if out.status.code() == Some(100) => {
                        let count = String::from_utf8_lossy(&out.stdout)
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .count();
                        format!("Hay {count} paquetes con actualización en caché.")
                    }
                    _ => "Tu sistema está actualizado.".to_string(),
                };
                let _ = tx.send_blocking(msg);
            });
            let st_d = st_c.clone();
            let btn_d = btn_c.clone();
            glib::spawn_future_local(async move {
                if let Ok(txt) = rx.recv().await {
                    st_d.set_label(&txt);
                }
                btn_d.set_sensitive(true);
            });
        });
    }

    {
        let win_c = parent_window.clone();
        let prefs_c = prefs.clone();
        let st_lbl = auto_state_lbl.clone();
        info_btn.connect_clicked(move |_| {
            let dlg = gtk4::Window::builder()
                .transient_for(&win_c)
                .destroy_with_parent(true)
                .modal(true)
                .resizable(false)
                .title("Actualizaciones automáticas")
                .default_width(400)
                .build();
            crate::attach_modal_scrim(&dlg, &win_c);
            let vbox = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(12)
                .margin_top(18)
                .margin_bottom(18)
                .margin_start(20)
                .margin_end(20)
                .build();
            let card = GtkBox::builder().orientation(Orientation::Vertical).build();
            let row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-pref-row"])
                .build();
            row.append(
                &Label::builder()
                    .label("Buscar y descargar actualizaciones automáticamente")
                    .css_classes(["wifi-row-title"])
                    .wrap(true)
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
            );
            let sw = Switch::builder()
                .active(prefs_c.borrow().auto_updates_enabled)
                .css_classes(["macos-switch"])
                .valign(Align::Center)
                .build();
            row.append(&*sw);
            card.append(&row);
            vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

            let (ok_w, ok_b) = make_squircle_button("Listo", true, None);
            ok_w.set_halign(Align::End);
            let d_ok = dlg.clone();
            let p_ok = prefs_c.clone();
            let lbl_ok = st_lbl.clone();
            ok_b.connect_clicked(move |_| {
                let active = sw.is_active();
                p_ok.borrow_mut().auto_updates_enabled = active;
                p_ok.borrow().save();
                lbl_ok.set_label(if active { "Activadas" } else { "Desactivadas" });
                d_ok.close();
            });
            vbox.append(&ok_w);
            dlg.set_child(Some(&vbox));
            dlg.present();
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let ver_c = ver_lbl;
        let snap_c = sys_snap.clone();
        Rc::new(move || {
            let s = snap_c.borrow();
            ver_c.set_label(&format!(
                "{} · Kernel {}",
                s.os_pretty_name, s.kernel_version
            ));
        })
    };
    (scrolled, refresh)
}

// ============================================================================
// 4. Subvista "Almacenamiento" (General04.png)
// ============================================================================

fn build_storage_subpane(
    _parent_window: &ApplicationWindow,
    prefs: &Rc<RefCell<GeneralPreferences>>,
    sys_snap: &Rc<RefCell<SystemInfoSnapshot>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let bar_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    let hdr_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let dev_title = Label::builder()
        .label("Volumen raíz (/)")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let usage_lbl = Label::builder()
        .label("Calculando uso...")
        .css_classes(["card-section-sub"])
        .build();
    hdr_row.append(&dev_title);
    hdr_row.append(&usage_lbl);
    bar_card.append(&hdr_row);

    let bar_canvas = DrawingArea::new();
    bar_canvas.set_content_height(24);
    bar_canvas.set_hexpand(true);
    {
        let snap_b = sys_snap.clone();
        bar_canvas.set_draw_func(move |_, cr, w, h| {
            draw_storage_bar(cr, w as f64, h as f64, &snap_b.borrow());
        });
    }
    bar_card.append(&bar_canvas);

    let legend = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .build();
    for (cls, txt) in [
        ("net-dot-red", "Usuarios y home"),
        ("net-dot-amber", "Aplicaciones"),
        ("net-dot-gray", "Datos del sistema"),
        ("net-dot-green", "Sistema Linux"),
    ] {
        let item = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(5)
            .build();
        item.append(&Label::builder().label("●").css_classes([cls]).build());
        item.append(
            &Label::builder()
                .label(txt)
                .css_classes(["card-section-sub"])
                .build(),
        );
        legend.append(&item);
    }
    bar_card.append(&legend);
    pane.append(&wrap_squircle_card(&bar_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Recomendaciones
    pane.append(
        &Label::builder()
            .label("Recomendaciones")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let rec_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let trash_rec = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    trash_rec.append(&make_squircle_colored_badge(
        28,
        "user-trash-symbolic",
        15,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        GRAY_TOP,
        GRAY_BOT,
    ));
    let tr_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    tr_text.append(
        &Label::builder()
            .label("Vaciar basurero automáticamente")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    tr_text.append(
        &Label::builder()
            .label("Puedes ahorrar espacio permitiendo que se eliminen automáticamente los elementos que llevan más de 30 días en el basurero.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let (tr_btn_wrap, tr_btn) = make_squircle_button(
        if prefs.borrow().auto_empty_trash_30d {
            "Activado"
        } else {
            "Activar..."
        },
        false,
        None,
    );
    {
        let p_c = prefs.clone();
        tr_btn.connect_clicked(move |b| {
            let next = !p_c.borrow().auto_empty_trash_30d;
            p_c.borrow_mut().auto_empty_trash_30d = next;
            p_c.borrow().save();
            b.set_label(if next { "Activado" } else { "Activar..." });
        });
    }
    trash_rec.append(&tr_text);
    trash_rec.append(&tr_btn_wrap);
    rec_card.append(&trash_rec);
    rec_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    for (ico, title, sub) in [
        ("video-x-generic-symbolic", "Optimizar el almacenamiento", "Elimina automáticamente cachés multimedia y paquetes huérfanos ya utilizados."),
        ("weather-overcast-symbolic", "Guardar en la nube", "Sincroniza archivos y fotos con almacenamiento remoto manteniendo copias locales recientes."),
    ] {
        let r = GtkBox::builder().orientation(Orientation::Horizontal).spacing(12).css_classes(["wifi-pref-row"]).build();
        r.append(&make_squircle_colored_badge(28, ico, 15, WIFI_BADGE_SQUIRCLE_RADIUS, GRAY_TOP, GRAY_BOT));
        let col = GtkBox::builder().orientation(Orientation::Vertical).spacing(2).hexpand(true).build();
        col.append(&Label::builder().label(title).css_classes(["wifi-row-title"]).xalign(0.0).build());
        col.append(&Label::builder().label(sub).css_classes(["card-section-sub"]).wrap(true).xalign(0.0).build());
        r.append(&col);
        r.append(&make_pending_badge());
        rec_card.append(&r);
        if ico == "video-x-generic-symbolic" {
            rec_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
    }
    pane.append(&wrap_squircle_card(&rec_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Categorías de usuario
    let cat_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (r_apps, l_apps) = storage_cat_row(
        "system-software-install-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Aplicaciones",
    );
    let (r_trash, l_trash) = storage_cat_row("user-trash-symbolic", GRAY_TOP, GRAY_BOT, "Basurero");
    let (r_docs, l_docs) = storage_cat_row(
        "folder-documents-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Documentos",
    );
    let (r_phot, l_phot) = storage_cat_row(
        "folder-pictures-symbolic",
        ORANGE_TOP,
        ORANGE_BOT,
        "Fotos e Imágenes",
    );
    let (r_down, l_down) = storage_cat_row(
        "folder-download-symbolic",
        (0.259, 0.565, 0.976),
        (0.235, 0.518, 0.949),
        "Descargas",
    );
    for (idx, r) in [r_apps, r_trash, r_docs, r_phot, r_down]
        .into_iter()
        .enumerate()
    {
        if idx > 0 {
            cat_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        cat_card.append(&r);
    }
    pane.append(&wrap_squircle_card(&cat_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Categorías del sistema
    let sys_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (r_usr, l_usr) = storage_cat_row(
        "system-users-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Otros usuarios y compartido",
    );
    let (r_os, l_os) = storage_cat_row(
        "computer-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Sistema Linux / Malus DE",
    );
    let (r_dat, l_dat) = storage_cat_row(
        "view-more-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Datos del sistema",
    );
    sys_card.append(&r_usr);
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    sys_card.append(&r_os);
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    sys_card.append(&r_dat);
    pane.append(&wrap_squircle_card(&sys_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let refresh: Rc<dyn Fn()> = {
        let snap_ref = sys_snap.clone();
        let dev_t = dev_title;
        let usg_l = usage_lbl;
        let canvas = bar_canvas;
        Rc::new(move || {
            let (tx, rx) = async_channel::bounded::<SystemInfoSnapshot>(1);
            std::thread::spawn(move || {
                let mut s = collect_system_info_fast();
                populate_storage_breakdown(&mut s);
                let _ = tx.send_blocking(s);
            });
            let snap_d = snap_ref.clone();
            let dev_d = dev_t.clone();
            let usg_d = usg_l.clone();
            let can_d = canvas.clone();
            let la = l_apps.clone();
            let lt = l_trash.clone();
            let ld = l_docs.clone();
            let lp = l_phot.clone();
            let ldw = l_down.clone();
            let lu = l_usr.clone();
            let lo = l_os.clone();
            let ldt = l_dat.clone();
            glib::spawn_future_local(async move {
                if let Ok(s) = rx.recv().await {
                    dev_d.set_label(&s.root_device);
                    usg_d.set_label(&format!(
                        "{} de {} en uso",
                        format_bytes_gb(s.root_used_bytes),
                        format_bytes_gb(s.root_total_bytes)
                    ));
                    la.set_label(&format_bytes_human(s.apps_bytes));
                    lt.set_label(&format_bytes_human(s.trash_bytes));
                    ld.set_label(&format_bytes_human(s.docs_bytes));
                    lp.set_label(&format_bytes_human(s.photos_bytes));
                    ldw.set_label(&format_bytes_human(s.downloads_bytes));
                    lu.set_label(&format_bytes_human(s.home_other_bytes));
                    lo.set_label(&format_bytes_human(s.os_base_bytes));
                    ldt.set_label(&format_bytes_human(s.system_data_bytes));
                    *snap_d.borrow_mut() = s;
                    can_d.queue_draw();
                }
            });
        })
    };
    (scrolled, refresh)
}

fn storage_cat_row(
    icon: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
) -> (GtkBox, Label) {
    let r = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r.append(&make_squircle_colored_badge(
        28,
        icon,
        15,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));
    r.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let size_lbl = Label::builder()
        .label("Calculando...")
        .css_classes(["card-section-sub"])
        .build();
    r.append(&size_lbl);
    (r, size_lbl)
}

fn draw_storage_bar(cr: &gtk4::cairo::Context, w: f64, h: f64, snap: &SystemInfoSnapshot) {
    let dark = libadwaita::StyleManager::default().is_dark();
    let radius = 6.0;
    cr.new_sub_path();
    cr.arc(
        w - radius,
        radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    cr.arc(
        w - radius,
        h - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    cr.arc(
        radius,
        h - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        radius,
        radius,
        radius,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    cr.close_path();
    cr.clip();

    cr.set_source_rgba(
        if dark { 1.0 } else { 0.0 },
        if dark { 1.0 } else { 0.0 },
        if dark { 1.0 } else { 0.0 },
        if dark { 0.14 } else { 0.11 },
    );
    cr.paint().ok();

    let total = snap.root_total_bytes.max(1) as f64;
    let segments = [
        (
            snap.home_other_bytes + snap.docs_bytes + snap.photos_bytes + snap.downloads_bytes,
            (0.98, 0.24, 0.20),
        ),
        (snap.apps_bytes, (1.0, 0.58, 0.0)),
        (snap.system_data_bytes, (0.42, 0.42, 0.45)),
        (snap.os_base_bytes, (0.20, 0.78, 0.35)),
    ];
    let mut x = 0.0;
    for (bytes, (r, g, b)) in segments {
        let seg_w = ((bytes as f64 / total) * w).max(if bytes > 0 { 3.0 } else { 0.0 });
        if seg_w > 0.0 && x < w {
            cr.set_source_rgb(r, g, b);
            cr.rectangle(x, 0.0, seg_w.min(w - x), h);
            let _ = cr.fill();
            x += seg_w + 1.0;
        }
    }

    let free_txt = format_bytes_gb(snap.root_avail_bytes);
    cr.set_font_size(10.5);
    cr.set_source_rgba(
        if dark { 0.92 } else { 0.18 },
        if dark { 0.92 } else { 0.18 },
        if dark { 0.95 } else { 0.20 },
        0.90,
    );
    let free_x = x + ((w - x) * 0.5 - 26.0).max(6.0);
    if free_x + 45.0 < w {
        cr.move_to(free_x, h * 0.66);
        let _ = cr.show_text(&free_txt);
    }
}

// ============================================================================
// 5. Subvista "AirDrop y continuidad" (General05.png)
// ============================================================================

fn build_airdrop_subpane(
    _parent_window: &ApplicationWindow,
    prefs: &Rc<RefCell<GeneralPreferences>>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    hdr.append(
        &Label::builder()
            .label("AirDrop")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr.append(&make_pending_badge());
    pane.append(&hdr);

    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let r_ad = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let ad_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    ad_col.append(
        &Label::builder()
            .label("AirDrop")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    ad_col.append(
        &Label::builder()
            .label("AirDrop te permite compartir instantáneamente con personas que están cerca. Puedes ser visible para recibir elementos de todos o sólo de tus contactos.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let ad_btn = make_simple_selector(
        &["Recepción desactivada", "Sólo contactos", "Todos"],
        prefs.borrow().airdrop_mode,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().airdrop_mode = idx;
                p.borrow().save();
            }
        },
    );
    r_ad.append(&ad_col);
    r_ad.append(&ad_btn);
    c1.append(&r_ad);
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let r_known = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let kn_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    kn_col.append(
        &Label::builder()
            .label("Contactos de AirDrop conocidos")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    kn_col.append(
        &Label::builder()
            .label("Las personas con las que hayas compartido un código de un solo uso podrán encontrarte durante 30 días.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let (adm_wrap, _) = make_squircle_button("Administrar...", false, None);
    r_known.append(&kn_col);
    r_known.append(&adm_wrap);
    c1.append(&r_known);
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Continuidad
    let hdr2 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .build();
    hdr2.append(
        &Label::builder()
            .label("Continuidad")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr2.append(&make_pending_badge());
    pane.append(&hdr2);
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&pref_switch_row(
        "Permitir Handoff entre este equipo y tus dispositivos móviles (Malus Link)",
        None,
        prefs.borrow().handoff_enabled,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().handoff_enabled = val;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    // Widgets y duplicación
    let hdr3 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .build();
    hdr3.append(
        &Label::builder()
            .label("Widgets y duplicación del teléfono")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr3.append(&make_pending_badge());
    pane.append(&hdr3);
    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&pref_switch_row(
        "Widgets del teléfono",
        None,
        prefs.borrow().iphone_widgets_enabled,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().iphone_widgets_enabled = val;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));

    // AirPlay
    let hdr4 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .build();
    hdr4.append(
        &Label::builder()
            .label("AirPlay")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr4.append(&make_pending_badge());
    pane.append(&hdr4);
    let c4 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c4.append(&pref_switch_row(
        "Receptor de AirPlay",
        Some("Permite a los dispositivos cercanos enviar contenido de vídeo y audio a este equipo mediante AirPlay (UxPlay / PipeWire)."),
        prefs.borrow().airplay_receiver_enabled,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().airplay_receiver_enabled = val;
                p.borrow().save();
            }
        },
    ));
    c4.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let r_ap_who = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let ap_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    ap_col.append(
        &Label::builder()
            .label("Permitir AirPlay para")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    ap_col.append(
        &Label::builder()
            .label("Controla quién puede ver y transmitir por AirPlay a esta computadora.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    r_ap_who.append(&ap_col);
    r_ap_who.append(&make_simple_selector(
        &["Usuario actual", "Cualquiera en la misma red", "Todos"],
        prefs.borrow().airplay_access_mode,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().airplay_access_mode = idx;
                p.borrow().save();
            }
        },
    ));
    c4.append(&r_ap_who);
    pane.append(&wrap_squircle_card(&c4, WIFI_CARD_SQUIRCLE_RADIUS));

    scrolled
}

// ============================================================================
// 6. Subvista "Autorrelleno y contraseñas" (General06.png)
// ============================================================================

fn build_autofill_subpane(
    _parent_window: &ApplicationWindow,
    prefs: &Rc<RefCell<GeneralPreferences>>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let banner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    let b_top = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    b_top.append(&make_squircle_colored_badge(
        32,
        "dialog-password-symbolic",
        17,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        GRAY_TOP,
        GRAY_BOT,
    ));
    let b_txt = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    b_txt.append(
        &Label::builder()
            .label("Todas tus contraseñas en un solo lugar")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    b_txt.append(
        &Label::builder()
            .label("Las contraseñas, llaves de acceso y códigos de verificación se almacenan en el llavero Secret Service del sistema.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    b_top.append(&b_txt);
    b_top.append(&make_pending_badge());
    banner.append(&b_top);
    let (open_pw_wrap, open_pw_btn) = make_squircle_button("Abrir Contraseñas", false, None);
    open_pw_wrap.set_halign(Align::End);
    open_pw_btn.connect_clicked(|_| {
        let _ = Command::new("seahorse")
            .spawn()
            .or_else(|_| Command::new("keepassxc").spawn());
    });
    banner.append(&open_pw_wrap);
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&pref_switch_row(
        "Autorrellenar contraseñas y llaves de acceso",
        Some("Recibe sugerencias automáticas de contraseñas, llaves de acceso y códigos al iniciar sesión en apps y sitios web."),
        prefs.borrow().autofill_passwords,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().autofill_passwords = val;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Autorrellenar con:")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&pref_switch_row(
        "Contraseñas (Llavero Secret Service)",
        Some("Llaves de acceso, contraseñas y códigos"),
        prefs.borrow().autofill_keyring_provider,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().autofill_keyring_provider = val;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Códigos de verificación")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&pref_switch_row(
        "Borrar después de usar",
        Some("Elimina automáticamente los códigos de verificación temporales después de que se utilicen."),
        prefs.borrow().delete_verification_codes,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().delete_verification_codes = val;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));

    scrolled
}

// ============================================================================
// 7. Subvista "Compartir" (General07.png)
// ============================================================================

fn build_sharing_subpane(
    parent_window: &ApplicationWindow,
    sys_snap: &Rc<RefCell<SystemInfoSnapshot>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    pane.append(
        &Label::builder()
            .label("Contenido")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&sharing_service_row(
        "folder-publicshare-symbolic",
        (0.259, 0.565, 0.976),
        (0.235, 0.518, 0.949),
        "Compartir archivos (Samba)",
        sys_snap.borrow().smb_active,
        true,
        None,
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&sharing_service_row(
        "folder-music-symbolic",
        ORANGE_TOP,
        ORANGE_BOT,
        "Compartir contenido multimedia",
        false,
        true,
        None,
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&sharing_service_row(
        "video-display-symbolic",
        PURPLE_TOP,
        PURPLE_BOT,
        "Compartir pantalla (PipeWire / RDP)",
        false,
        true,
        None,
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&sharing_service_row(
        "emblem-Downloads",
        GRAY_TOP,
        GRAY_BOT,
        "Contenido en caché",
        false,
        true,
        None,
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Accesorios e Internet")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&sharing_service_row(
        "bluetooth-active-symbolic",
        (0.259, 0.565, 0.976),
        (0.235, 0.518, 0.949),
        "Compartir Bluetooth (OBEX)",
        false,
        true,
        None,
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&sharing_service_row(
        "printer-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Compartir impresora (CUPS)",
        sys_snap.borrow().cups_active,
        false,
        Some("cups"),
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&sharing_service_row(
        "network-workgroup-symbolic",
        (0.259, 0.565, 0.976),
        (0.235, 0.518, 0.949),
        "Compartir Internet (Hotspot)",
        false,
        true,
        None,
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Avanzado")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&sharing_service_row(
        "preferences-system-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Administración remota",
        false,
        true,
        None,
    ));
    c3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c3.append(&sharing_service_row(
        "utilities-terminal-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Sesión remota (SSH / sshd)",
        sys_snap.borrow().sshd_active,
        false,
        Some("sshd"),
    ));
    c3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c3.append(&sharing_service_row(
        "applications-engineering-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Scripts de apps remotas",
        false,
        true,
        None,
    ));
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));

    // Nombre del host local
    let host_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let r_host1 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r_host1.append(
        &Label::builder()
            .label("Nombre del host local")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let host_val_lbl = Label::builder()
        .label(&format!("{}.local", sys_snap.borrow().hostname))
        .css_classes(["card-section-sub"])
        .build();
    r_host1.append(&host_val_lbl);
    host_card.append(&r_host1);
    host_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let r_host2 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r_host2.append(
        &Label::builder()
            .label("Las computadoras de tu red local pueden acceder a tu computadora en esta dirección.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let (edit_host_wrap, edit_host_btn) = make_squircle_button("Editar...", false, None);
    r_host2.append(&edit_host_wrap);
    host_card.append(&r_host2);
    pane.append(&wrap_squircle_card(&host_card, WIFI_CARD_SQUIRCLE_RADIUS));

    {
        let win_c = parent_window.clone();
        let snap_c = sys_snap.clone();
        let h_lbl = host_val_lbl.clone();
        edit_host_btn.connect_clicked(move |_| {
            let dlg = gtk4::Window::builder()
                .transient_for(&win_c)
                .destroy_with_parent(true)
                .modal(true)
                .resizable(false)
                .title("Editar nombre del host local")
                .default_width(380)
                .build();
            crate::attach_modal_scrim(&dlg, &win_c);
            let vbox = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(10)
                .margin_top(18)
                .margin_bottom(18)
                .margin_start(20)
                .margin_end(20)
                .build();
            let entry = Entry::builder().text(&snap_c.borrow().hostname).build();
            vbox.append(
                &Label::builder()
                    .label("Nombre del equipo (hostnamectl):")
                    .css_classes(["card-section-sub"])
                    .xalign(0.0)
                    .build(),
            );
            vbox.append(&entry);
            let actions = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .halign(Align::End)
                .build();
            let (cw, cb) = make_squircle_button("Cancelar", false, None);
            let (ow, ob) = make_squircle_button("OK", true, None);
            actions.append(&cw);
            actions.append(&ow);
            let d1 = dlg.clone();
            cb.connect_clicked(move |_| d1.close());
            let d2 = dlg.clone();
            let hl = h_lbl.clone();
            let sc = snap_c.clone();
            ob.connect_clicked(move |_| {
                let new_h: String = entry
                    .text()
                    .trim()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                if !new_h.is_empty() {
                    sc.borrow_mut().hostname = new_h.clone();
                    hl.set_label(&format!("{new_h}.local"));
                    std::thread::spawn(move || {
                        let _ = Command::new("pkexec")
                            .args(["hostnamectl", "set-hostname", &new_h])
                            .output();
                    });
                }
                d2.close();
            });
            vbox.append(&actions);
            dlg.set_child(Some(&vbox));
            dlg.present();
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let snap_c = sys_snap.clone();
        let h_lbl = host_val_lbl;
        Rc::new(move || {
            h_lbl.set_label(&format!("{}.local", snap_c.borrow().hostname));
        })
    };
    (scrolled, refresh)
}

fn sharing_service_row(
    icon: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    active: bool,
    pending: bool,
    systemd_unit: Option<&'static str>,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(&make_squircle_colored_badge(
        28,
        icon,
        15,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));

    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let sub_txt = if active {
        "Activo"
    } else if pending {
        "Este servicio no está configurado por ahora."
    } else {
        "Desactivado"
    };
    col.append(
        &Label::builder()
            .label(sub_txt)
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    row.append(&col);

    if pending {
        row.append(&make_pending_badge());
    }

    let sw = Switch::builder()
        .active(active)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    if let Some(unit) = systemd_unit {
        sw.connect_state_set(move |_, enable| {
            std::thread::spawn(move || {
                let act = if enable {
                    "enable --now"
                } else {
                    "disable --now"
                };
                let _ = Command::new("pkexec")
                    .args(["sh", "-c", &format!("systemctl {act} {unit}")])
                    .output();
            });
            glib::Propagation::Proceed
        });
    }
    row.append(&*sw);
    row
}

// ============================================================================
// 8. Subvista "Disco de arranque" (General08.png)
// ============================================================================

fn build_startup_disk_subpane(
    parent_window: &ApplicationWindow,
    sys_snap: &Rc<RefCell<SystemInfoSnapshot>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    card.append(
        &Label::builder()
            .label("Selecciona el sistema que quieres utilizar para arrancar la computadora")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );

    let tile = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .halign(Align::Center)
        .css_classes(["gen-disk-tile"])
        .margin_top(12)
        .margin_bottom(12)
        .build();
    tile.append(
        &Image::builder()
            .icon_name("drive-harddisk-symbolic")
            .pixel_size(54)
            .build(),
    );
    let disk_lbl = Label::builder()
        .label(&sys_snap.borrow().root_device)
        .css_classes(["wifi-row-title"])
        .halign(Align::Center)
        .build();
    let os_lbl = Label::builder()
        .label(&sys_snap.borrow().os_pretty_name)
        .css_classes(["card-section-sub"])
        .halign(Align::Center)
        .build();
    tile.append(&disk_lbl);
    tile.append(&os_lbl);
    card.append(&tile);

    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let sel_lbl = Label::builder()
        .label(&format!(
            "Seleccionaste {} en el volumen “{}”.",
            sys_snap.borrow().os_pretty_name,
            sys_snap.borrow().root_device
        ))
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let (reb_wrap, reb_btn) = make_squircle_button("Reiniciar...", false, None);
    let win_c = parent_window.clone();
    reb_btn.connect_clicked(move |_| {
        let dlg = gtk4::Window::builder()
            .transient_for(&win_c)
            .destroy_with_parent(true)
            .modal(true)
            .resizable(false)
            .title("Reiniciar equipo")
            .default_width(360)
            .build();
        crate::attach_modal_scrim(&dlg, &win_c);
        let vb = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(20)
            .margin_end(20)
            .build();
        vb.append(
            &Label::builder()
                .label("¿Quieres reiniciar el equipo ahora?")
                .css_classes(["card-section-title"])
                .xalign(0.0)
                .build(),
        );
        let act = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::End)
            .build();
        let (cw, cb) = make_squircle_button("Cancelar", false, None);
        let (rw, rb) = make_squircle_button("Reiniciar", true, None);
        act.append(&cw);
        act.append(&rw);
        let d1 = dlg.clone();
        cb.connect_clicked(move |_| d1.close());
        let d2 = dlg.clone();
        rb.connect_clicked(move |_| {
            d2.close();
            let _ = Command::new("systemctl").arg("reboot").spawn();
        });
        vb.append(&act);
        dlg.set_child(Some(&vb));
        dlg.present();
    });
    footer.append(&sel_lbl);
    footer.append(&reb_wrap);
    card.append(&footer);

    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let refresh: Rc<dyn Fn()> = {
        let snap_c = sys_snap.clone();
        let dl = disk_lbl;
        let ol = os_lbl;
        let sl = sel_lbl;
        Rc::new(move || {
            let s = snap_c.borrow();
            dl.set_label(&s.root_device);
            ol.set_label(&s.os_pretty_name);
            sl.set_label(&format!(
                "Seleccionaste {} en el volumen “{}”.",
                s.os_pretty_name, s.root_device
            ));
        })
    };
    (scrolled, refresh)
}

// ============================================================================
// 9. Subvista "Elementos de inicio y extensiones" (General09.png & General10.png)
// ============================================================================

fn build_login_items_subpane(parent_window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    pane.append(
        &Label::builder()
            .label("Abrir al iniciar sesión")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    pane.append(&Label::builder().label("Estos elementos se abrirán automáticamente al iniciar la sesión (~/.config/autostart).").css_classes(["card-section-sub"]).xalign(0.0).build());

    let table_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let th = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row", "gen-autostart-header"])
        .build();
    th.append(
        &Label::builder()
            .label("Elemento")
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    th.append(
        &Label::builder()
            .label("Clase")
            .css_classes(["card-section-sub"])
            .width_request(110)
            .xalign(0.0)
            .build(),
    );
    table_card.append(&th);
    table_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let autostart_list = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .height_request(110)
        .build();
    table_card.append(&autostart_list);
    table_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let toolbar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["fw-apps-toolbar"])
        .build();
    let add_btn = Button::builder()
        .icon_name("list-add-symbolic")
        .has_frame(false)
        .css_classes(["fw-apps-tool-btn"])
        .build();
    let rem_btn = Button::builder()
        .icon_name("list-remove-symbolic")
        .has_frame(false)
        .css_classes(["fw-apps-tool-btn"])
        .sensitive(false)
        .build();
    toolbar.append(&add_btn);
    toolbar.append(
        &GtkBox::builder()
            .css_classes(["fw-apps-tool-sep"])
            .valign(Align::Center)
            .build(),
    );
    toolbar.append(&rem_btn);
    table_card.append(&toolbar);
    pane.append(&wrap_squircle_card(&table_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Extensiones con selector [Por app | Por categoría] (General09 & General10)
    let ext_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(8)
        .build();
    ext_hdr.append(
        &Label::builder()
            .label("Extensiones")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    ext_hdr.append(&make_pending_badge());
    pane.append(&ext_hdr);
    pane.append(
        &Label::builder()
            .label("Las extensiones agregan más funcionalidades al sistema y a las apps, y algunas pueden ejecutarse en segundo plano.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let seg_track = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .homogeneous(true)
        .spacing(2)
        .css_classes(["bat-segmented-track"])
        .build();
    let tab_by_app = Button::builder()
        .label("Por app")
        .has_frame(false)
        .css_classes(["bat-seg-tab", "bat-seg-tab-active"])
        .build();
    let tab_by_cat = Button::builder()
        .label("Por categoría")
        .has_frame(false)
        .css_classes(["bat-seg-tab"])
        .build();
    seg_track.append(&tab_by_app);
    seg_track.append(&tab_by_cat);
    pane.append(&seg_track);

    let ext_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    populate_extensions_list(&ext_card, false);
    pane.append(&wrap_squircle_card(&ext_card, WIFI_CARD_SQUIRCLE_RADIUS));

    {
        let t_app = tab_by_app.clone();
        let t_cat = tab_by_cat.clone();
        let card_c = ext_card.clone();
        tab_by_app.connect_clicked(move |_| {
            t_app.add_css_class("bat-seg-tab-active");
            t_cat.remove_css_class("bat-seg-tab-active");
            populate_extensions_list(&card_c, false);
        });
    }
    {
        let t_app = tab_by_app;
        let t_cat = tab_by_cat.clone();
        let card_c = ext_card;
        tab_by_cat.connect_clicked(move |_| {
            t_cat.add_css_class("bat-seg-tab-active");
            t_app.remove_css_class("bat-seg-tab-active");
            populate_extensions_list(&card_c, true);
        });
    }

    let entries_state: Rc<RefCell<Vec<AutostartEntry>>> = Rc::new(RefCell::new(Vec::new()));
    let selected_idx: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let render_autostart: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let render_fn: Rc<dyn Fn()> = {
        let list_c = autostart_list;
        let ent_c = entries_state.clone();
        let sel_c = selected_idx.clone();
        let rem_c = rem_btn.clone();
        let self_ref = render_autostart.clone();
        Rc::new(move || {
            while let Some(c) = list_c.first_child() {
                list_c.remove(&c);
            }
            let items = load_autostart_entries();
            *ent_c.borrow_mut() = items.clone();
            rem_c.set_sensitive(sel_c.get().is_some_and(|i| i < items.len()));

            if items.is_empty() {
                list_c.append(
                    &Label::builder()
                        .label("No hay elementos de inicio configurados.")
                        .css_classes(["card-section-sub", "wifi-pref-row"])
                        .xalign(0.0)
                        .build(),
                );
                return;
            }
            for (idx, item) in items.iter().enumerate() {
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .css_classes(if sel_c.get() == Some(idx) {
                        vec!["fw-app-row", "fw-app-row-selected"]
                    } else {
                        vec!["fw-app-row"]
                    })
                    .build();
                row.append(
                    &Image::builder()
                        .icon_name("application-x-executable-symbolic")
                        .pixel_size(16)
                        .build(),
                );
                row.append(
                    &Label::builder()
                        .label(&item.name)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .build(),
                );
                row.append(
                    &Label::builder()
                        .label(&item.kind_label)
                        .css_classes(["card-section-sub"])
                        .width_request(110)
                        .xalign(0.0)
                        .build(),
                );

                let click = gtk4::GestureClick::new();
                let sc = sel_c.clone();
                let rf = self_ref.clone();
                click.connect_released(move |_, _, _, _| {
                    sc.set(Some(idx));
                    if let Some(f) = rf.borrow().as_ref() {
                        f();
                    }
                });
                row.add_controller(click);
                list_c.append(&row);
            }
        })
    };
    *render_autostart.borrow_mut() = Some(render_fn.clone());
    render_fn();

    {
        let ent_c = entries_state.clone();
        let sel_c = selected_idx.clone();
        let rf = render_fn.clone();
        rem_btn.connect_clicked(move |_| {
            if let Some(i) = sel_c.get() {
                if let Some(entry) = ent_c.borrow().get(i) {
                    let _ = fs::remove_file(&entry.file_path);
                }
                sel_c.set(None);
                rf();
            }
        });
    }
    {
        let win_c = parent_window.clone();
        let rf = render_fn.clone();
        add_btn.connect_clicked(move |_| {
            let dlg = gtk4::Window::builder()
                .transient_for(&win_c)
                .destroy_with_parent(true)
                .modal(true)
                .resizable(false)
                .title("Añadir elemento de inicio")
                .default_width(380)
                .build();
            crate::attach_modal_scrim(&dlg, &win_c);
            let vb = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(10)
                .margin_top(18)
                .margin_bottom(18)
                .margin_start(20)
                .margin_end(20)
                .build();
            let name_e = Entry::builder()
                .placeholder_text("Nombre de la aplicación")
                .build();
            let exec_e = Entry::builder()
                .placeholder_text("Comando (ej. flatpak run ... o binario)")
                .build();
            vb.append(&name_e);
            vb.append(&exec_e);
            let act = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .halign(Align::End)
                .build();
            let (cw, cb) = make_squircle_button("Cancelar", false, None);
            let (aw, ab) = make_squircle_button("Añadir", true, None);
            act.append(&cw);
            act.append(&aw);
            let d1 = dlg.clone();
            cb.connect_clicked(move |_| d1.close());
            let d2 = dlg.clone();
            let r_done = rf.clone();
            ab.connect_clicked(move |_| {
                let n = name_e.text().trim().to_string();
                let e = exec_e.text().trim().to_string();
                if !n.is_empty() && !e.is_empty() {
                    create_autostart_entry(&n, &e);
                    r_done();
                }
                d2.close();
            });
            vb.append(&act);
            dlg.set_child(Some(&vb));
            dlg.present();
        });
    }

    (scrolled, render_fn)
}

fn populate_extensions_list(card: &GtkBox, by_category: bool) {
    while let Some(c) = card.first_child() {
        card.remove(&c);
    }
    let items: &[(&str, (f64, f64, f64), (f64, f64, f64), &str, &str)] = if !by_category {
        &[
            (
                "system-run-symbolic",
                PURPLE_TOP,
                PURPLE_BOT,
                "Atajos",
                "Compartir",
            ),
            (
                "accessories-dictionary-symbolic",
                PURPLE_TOP,
                PURPLE_BOT,
                "Diario",
                "Compartir",
            ),
            (
                "applications-graphics-symbolic",
                TEAL_TOP,
                TEAL_BOT,
                "Freeform",
                "Compartir",
            ),
            (
                "accessories-text-editor-symbolic",
                ORANGE_TOP,
                ORANGE_BOT,
                "Notas y Libros",
                "Compartir",
            ),
            (
                "view-list-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Recordatorios",
                "Compartir",
            ),
            (
                "emblem-system-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Servicios del sistema",
                "Acortar, Convertir imagen, Crear PDF, Girar a la izquierda, Marcado",
            ),
        ]
    } else {
        &[
            (
                "applications-graphics-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Acciones",
                "Marcado",
            ),
            (
                "system-search-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Indexador de Spotlight",
                "PDFImporter, AudioMetadataImporter",
            ),
            (
                "emblem-shared-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Compartir",
                "Agregar a Fotos, AirDrop, Atajos, Correo",
            ),
            (
                "camera-photo-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Edición de fotos",
                "Marcado",
            ),
            (
                "drive-harddisk-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Extensiones del sistema de archivos",
                "ftp, exfat, msdos, btrfs",
            ),
            (
                "folder-symbolic",
                GRAY_TOP,
                GRAY_BOT,
                "Branches (Finder)",
                "Crear PDF, Girar imagen, Comprimir archivo",
            ),
        ]
    };
    for (idx, (ico, top, bot, title, sub)) in items.iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let r = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        r.append(&make_squircle_colored_badge(
            28,
            ico,
            15,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            *top,
            *bot,
        ));
        let col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        col.append(
            &Label::builder()
                .label(*title)
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        col.append(
            &Label::builder()
                .label(*sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        r.append(&col);
        card.append(&r);
    }
}

// ============================================================================
// 10. Subvista "Fecha y hora" (General11.png)
// ============================================================================

fn build_datetime_subpane(
    _parent_window: &ApplicationWindow,
    prefs: &Rc<RefCell<GeneralPreferences>>,
    sys_snap: &Rc<RefCell<SystemInfoSnapshot>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&pref_switch_row(
        "Establecer fecha y hora automáticamente",
        None,
        sys_snap.borrow().ntp_active,
        move |enable| {
            std::thread::spawn(move || {
                let val = if enable { "true" } else { "false" };
                let _ = Command::new("pkexec")
                    .args(["timedatectl", "set-ntp", val])
                    .output();
            });
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let r_src = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r_src.append(
        &Label::builder()
            .label("Fuente")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    r_src.append(
        &Label::builder()
            .label(&format!("NTP ({})", prefs.borrow().ntp_server))
            .css_classes(["card-section-sub"])
            .build(),
    );
    let (set_ntp_w, _) = make_squircle_button("Establecer...", false, None);
    r_src.append(&set_ntp_w);
    c1.append(&r_src);
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (r_now, now_val_lbl) = info_kv_row(
        "Fecha y hora",
        &format_current_datetime(prefs.borrow().clock_24h),
    );
    c2.append(&r_now);
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let now_lbl_c = now_val_lbl.clone();
    c2.append(&pref_switch_row(
        "Reloj de 24 horas",
        None,
        prefs.borrow().clock_24h,
        {
            let p = prefs.clone();
            move |use_24h| {
                p.borrow_mut().clock_24h = use_24h;
                p.borrow().save();
                now_lbl_c.set_label(&format_current_datetime(use_24h));
                let gsettings = gtk4::gio::Settings::new("org.gnome.desktop.interface");
                let _ = gsettings.set_string("clock-format", if use_24h { "24h" } else { "12h" });
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&pref_switch_row(
        "Establecer la zona horaria automáticamente según tu ubicación actual",
        None,
        prefs.borrow().auto_timezone,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().auto_timezone = val;
                p.borrow().save();
            }
        },
    ));
    c3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let (r_tz, tz_val_lbl) = info_kv_row("Zona horaria", &sys_snap.borrow().timezone);
    let (r_city, city_val_lbl) = info_kv_row(
        "Ciudad de la zona horaria",
        &friendly_timezone_city(&sys_snap.borrow().timezone),
    );
    c3.append(&r_tz);
    c3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c3.append(&r_city);
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));

    let refresh: Rc<dyn Fn()> = {
        let p_c = prefs.clone();
        let s_c = sys_snap.clone();
        let nl = now_val_lbl;
        let tzl = tz_val_lbl;
        let cl = city_val_lbl;
        Rc::new(move || {
            nl.set_label(&format_current_datetime(p_c.borrow().clock_24h));
            let tz = s_c.borrow().timezone.clone();
            tzl.set_label(&tz);
            cl.set_label(&friendly_timezone_city(&tz));
        })
    };
    (scrolled, refresh)
}

// ============================================================================
// 11. Subvista "Idioma y región" (General12.png)
// ============================================================================

fn build_locale_subpane(
    parent_window: &ApplicationWindow,
    prefs: &Rc<RefCell<GeneralPreferences>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    pane.append(
        &Label::builder()
            .label("Idiomas preferidos")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    let lang_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let lang_list = GtkBox::builder().orientation(Orientation::Vertical).build();
    lang_card.append(&lang_list);
    lang_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let lang_tools = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["fw-apps-toolbar"])
        .build();
    let add_lang_btn = Button::builder()
        .icon_name("list-add-symbolic")
        .has_frame(false)
        .css_classes(["fw-apps-tool-btn"])
        .build();
    let rem_lang_btn = Button::builder()
        .icon_name("list-remove-symbolic")
        .has_frame(false)
        .css_classes(["fw-apps-tool-btn"])
        .build();
    lang_tools.append(&add_lang_btn);
    lang_tools.append(
        &GtkBox::builder()
            .css_classes(["fw-apps-tool-sep"])
            .valign(Align::Center)
            .build(),
    );
    lang_tools.append(&rem_lang_btn);
    lang_card.append(&lang_tools);
    pane.append(&wrap_squircle_card(&lang_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta principal de formatos regionales + vista previa superior
    let reg_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let preview_lbl = Label::builder()
        .label(&format_regional_preview(&prefs.borrow()))
        .css_classes(["card-section-sub"])
        .justify(gtk4::Justification::Center)
        .halign(Align::Center)
        .margin_top(10)
        .margin_bottom(10)
        .build();
    reg_card.append(&preview_lbl);
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let update_preview: Rc<dyn Fn()> = {
        let p_c = prefs.clone();
        let prev = preview_lbl.clone();
        Rc::new(move || {
            prev.set_label(&format_regional_preview(&p_c.borrow()));
        })
    };

    let regions = [
        "México",
        "España",
        "Colombia",
        "Argentina",
        "Chile",
        "Perú",
        "Estados Unidos",
    ];
    let cur_reg_idx = regions
        .iter()
        .position(|r| *r == prefs.borrow().region)
        .unwrap_or(0);
    reg_card.append(&selector_row("Región", &regions, cur_reg_idx, {
        let p = prefs.clone();
        let up = update_preview.clone();
        move |idx| {
            p.borrow_mut().region = regions[idx].to_string();
            p.borrow().save();
            up();
        }
    }));
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let cals = ["Gregoriano", "ISO 8601"];
    reg_card.append(&selector_row("Calendario", &cals, 0, {
        let p = prefs.clone();
        move |idx| {
            p.borrow_mut().calendar = cals[idx].to_string();
            p.borrow().save();
        }
    }));
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Temperatura (Radio buttons: Celsius / Fahrenheit)
    let temp_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    temp_row.append(
        &Label::builder()
            .label("Temperatura")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let rb_c = CheckButton::with_label("Celsius (°C)");
    let rb_f = CheckButton::with_label("Fahrenheit (°F)");
    rb_f.set_group(Some(&rb_c));
    if prefs.borrow().celsius {
        rb_c.set_active(true);
    } else {
        rb_f.set_active(true);
    }
    {
        let p = prefs.clone();
        rb_c.connect_toggled(move |btn| {
            if btn.is_active() {
                p.borrow_mut().celsius = true;
                p.borrow().save();
            }
        });
    }
    {
        let p = prefs.clone();
        rb_f.connect_toggled(move |btn| {
            if btn.is_active() {
                p.borrow_mut().celsius = false;
                p.borrow().save();
            }
        });
    }
    temp_row.append(&rb_c);
    temp_row.append(&rb_f);
    reg_card.append(&temp_row);
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Sistema de medición (Radio buttons: Métrico / Estadounidense / Imperial)
    let meas_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    meas_row.append(
        &Label::builder()
            .label("Sistema de medición")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let rb_m = CheckButton::with_label("Métrico");
    let rb_u = CheckButton::with_label("Estadounidense");
    let rb_i = CheckButton::with_label("Imperial");
    rb_u.set_group(Some(&rb_m));
    rb_i.set_group(Some(&rb_m));
    match prefs.borrow().measurement_system {
        1 => rb_u.set_active(true),
        2 => rb_i.set_active(true),
        _ => rb_m.set_active(true),
    }
    meas_row.append(&rb_m);
    meas_row.append(&rb_u);
    meas_row.append(&rb_i);
    reg_card.append(&meas_row);
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    reg_card.append(&selector_row(
        "Primer día de la semana",
        &["Domingo", "Lunes", "Sábado"],
        prefs.borrow().first_weekday,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().first_weekday = idx;
                p.borrow().save();
            }
        },
    ));
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    reg_card.append(&selector_row(
        "Formato de fecha",
        &["19/08/26", "2026-08-19", "08/19/26"],
        prefs.borrow().date_format_idx,
        {
            let p = prefs.clone();
            let up = update_preview.clone();
            move |idx| {
                p.borrow_mut().date_format_idx = idx;
                p.borrow().save();
                up();
            }
        },
    ));
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    reg_card.append(&selector_row(
        "Formato numérico",
        &["1,234,567.89", "1.234.567,89", "1 234 567,89"],
        prefs.borrow().number_format_idx,
        {
            let p = prefs.clone();
            let up = update_preview.clone();
            move |idx| {
                p.borrow_mut().number_format_idx = idx;
                p.borrow().save();
                up();
            }
        },
    ));
    reg_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    reg_card.append(&selector_row(
        "Orden de las listas",
        &["Español", "Universal (Unicode)"],
        prefs.borrow().list_sort_idx,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().list_sort_idx = idx;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&reg_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tratamiento y Texto en vivo
    let term_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let tr_row = selector_row(
        "Tratamiento",
        &["Masculino", "Femenino", "Neutro"],
        prefs.borrow().address_term_idx,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().address_term_idx = idx;
                p.borrow().save();
            }
        },
    );
    tr_row.append(&make_pending_badge());
    term_card.append(&tr_row);
    term_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    term_card.append(&pref_switch_row(
        "Compartir con todas las apps",
        Some("Las apps pueden usar el tratamiento que elijas para personalizar la forma en la que se dirigen a ti."),
        prefs.borrow().share_address_term,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().share_address_term = val;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&term_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let ocr_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let ocr_row = pref_switch_row(
        "Texto en vivo",
        Some("Selecciona texto en las imágenes para copiarlo o realizar acciones."),
        prefs.borrow().live_text_ocr,
        {
            let p = prefs.clone();
            move |val| {
                p.borrow_mut().live_text_ocr = val;
                p.borrow().save();
            }
        },
    );
    ocr_row.append(&make_pending_badge());
    ocr_card.append(&ocr_row);
    pane.append(&wrap_squircle_card(&ocr_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let render_langs: Rc<dyn Fn()> = {
        let lbox = lang_list;
        let p_c = prefs.clone();
        Rc::new(move || {
            while let Some(c) = lbox.first_child() {
                lbox.remove(&c);
            }
            for (idx, lang) in p_c.borrow().preferred_languages.iter().enumerate() {
                if idx > 0 {
                    lbox.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let r = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .css_classes(["wifi-pref-row"])
                    .build();
                r.append(
                    &Label::builder()
                        .label(lang)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .build(),
                );
                lbox.append(&r);
            }
        })
    };
    render_langs();

    {
        let win_c = parent_window.clone();
        let p_c = prefs.clone();
        let rl = render_langs.clone();
        add_lang_btn.connect_clicked(move |_| {
            let dlg = gtk4::Window::builder()
                .transient_for(&win_c)
                .destroy_with_parent(true)
                .modal(true)
                .resizable(false)
                .title("Añadir idioma preferido")
                .default_width(340)
                .build();
            crate::attach_modal_scrim(&dlg, &win_c);
            let vb = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(10)
                .margin_top(18)
                .margin_bottom(18)
                .margin_start(20)
                .margin_end(20)
                .build();
            let entry = Entry::builder()
                .placeholder_text("Ej. English (US) o Français")
                .build();
            vb.append(&entry);
            let act = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .halign(Align::End)
                .build();
            let (cw, cb) = make_squircle_button("Cancelar", false, None);
            let (aw, ab) = make_squircle_button("Añadir", true, None);
            act.append(&cw);
            act.append(&aw);
            let d1 = dlg.clone();
            cb.connect_clicked(move |_| d1.close());
            let d2 = dlg.clone();
            let pc = p_c.clone();
            let r_done = rl.clone();
            ab.connect_clicked(move |_| {
                let txt = entry.text().trim().to_string();
                if !txt.is_empty() {
                    pc.borrow_mut().preferred_languages.push(txt);
                    pc.borrow().save();
                    r_done();
                }
                d2.close();
            });
            vb.append(&act);
            dlg.set_child(Some(&vb));
            dlg.present();
        });
    }
    {
        let p_c = prefs.clone();
        let rl = render_langs.clone();
        rem_lang_btn.connect_clicked(move |_| {
            let mut p = p_c.borrow_mut();
            if p.preferred_languages.len() > 1 {
                p.preferred_languages.pop();
                p.save();
                drop(p);
                rl();
            }
        });
    }

    (scrolled, render_langs)
}

// ============================================================================
// 12–15. Subvistas con etiqueta "por implementar" y "Transferir o restablecer"
// ============================================================================

fn build_pending_feature_subpane(
    icon: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    description: &str,
    linux_note: &str,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    let hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    hdr.append(&make_squircle_colored_badge(
        34,
        icon,
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    col.append(
        &Label::builder()
            .label(description)
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    hdr.append(&col);
    hdr.append(&make_pending_badge());
    card.append(&hdr);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(
        &Label::builder()
            .label(linux_note)
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn build_reset_subpane(parent_window: &ApplicationWindow) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    let r1 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    r1.append(&make_squircle_colored_badge(
        32,
        "view-refresh-symbolic",
        17,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        GRAY_TOP,
        GRAY_BOT,
    ));
    let c1 = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    c1.append(
        &Label::builder()
            .label("Restablecer preferencias de Malus DE")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    c1.append(
        &Label::builder()
            .label("Restaura los ajustes de apariencia, dock, pantallas y paneles de ~/.config/malus-de a sus valores predeterminados sin borrar tus archivos personales.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let (rst_wrap, rst_btn) = make_squircle_button("Restablecer ajustes...", false, None);
    let win_c = parent_window.clone();
    rst_btn.connect_clicked(move |_| {
        let dlg = gtk4::Window::builder()
            .transient_for(&win_c)
            .destroy_with_parent(true)
            .modal(true)
            .resizable(false)
            .title("Restablecer ajustes de Malus DE")
            .default_width(380)
            .build();
        crate::attach_modal_scrim(&dlg, &win_c);
        let vb = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(20)
            .margin_end(20)
            .build();
        vb.append(
            &Label::builder()
                .label("¿Restaurar las preferencias de General a sus valores por defecto?")
                .css_classes(["card-section-title"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        let act = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::End)
            .build();
        let (cw, cb) = make_squircle_button("Cancelar", false, None);
        let (ow, ob) = make_squircle_button("Restablecer", true, None);
        act.append(&cw);
        act.append(&ow);
        let d1 = dlg.clone();
        cb.connect_clicked(move |_| d1.close());
        let d2 = dlg.clone();
        ob.connect_clicked(move |_| {
            GeneralPreferences::default().save();
            d2.close();
        });
        vb.append(&act);
        dlg.set_child(Some(&vb));
        dlg.present();
    });
    r1.append(&c1);
    r1.append(&rst_wrap);
    card.append(&r1);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let r2 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let c2 = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    c2.append(
        &Label::builder()
            .label("Asistente de Migración y Borrado total")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    c2.append(
        &Label::builder()
            .label("Transferencia guiada de cuentas de usuario y borrado criptográfico LUKS.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    r2.append(&c2);
    r2.append(&make_pending_badge());
    card.append(&r2);

    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// Controles auxiliares compartidos dentro de General
// ============================================================================

fn pref_switch_row<F: Fn(bool) + 'static>(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
    on_toggle: F,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    if let Some(sub) = subtitle {
        col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    let sw = Switch::builder()
        .active(initial)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(&col);
    row.append(&*sw);
    row
}

fn selector_row<F: Fn(usize) + 'static>(
    title: &str,
    options: &[&str],
    initial_idx: usize,
    on_select: F,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    row.append(&make_simple_selector(options, initial_idx, on_select));
    row
}

fn make_simple_selector<F: Fn(usize) + 'static>(
    options: &[&str],
    initial_idx: usize,
    on_select: F,
) -> gtk4::MenuButton {
    let selected = Rc::new(Cell::new(initial_idx.min(options.len().saturating_sub(1))));
    let opts: Vec<String> = options.iter().map(|s| s.to_string()).collect();
    let cb = Rc::new(on_select);

    let inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let cur_lbl = Label::builder()
        .label(opts.get(selected.get()).map(String::as_str).unwrap_or(""))
        .css_classes(["card-section-sub"])
        .build();
    inner.append(&cur_lbl);
    inner.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .build(),
    );

    let btn = gtk4::MenuButton::builder()
        .child(&inner)
        .has_frame(false)
        .always_show_arrow(false)
        .css_classes(["fw-rule-action-btn"])
        .valign(Align::Center)
        .build();
    let pop = gtk4::Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    btn.set_popover(Some(&pop));

    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let reb_fn: Rc<dyn Fn()> = {
        let pop_c = pop;
        let sel_c = selected;
        let lbl_c = cur_lbl;
        let opts_c = opts;
        let cb_c = cb;
        let self_ref = rebuild.clone();
        Rc::new(move || {
            let mbox = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();
            let cur = sel_c.get();
            for (idx, opt) in opts_c.iter().enumerate() {
                let (w, b) = make_popover_action_row(opt, true, (idx == cur).then_some("✓"));
                let pc = pop_c.clone();
                let sc = sel_c.clone();
                let lc = lbl_c.clone();
                let os = opt.clone();
                let cbc = cb_c.clone();
                let rfc = self_ref.clone();
                b.connect_clicked(move |_| {
                    pc.popdown();
                    sc.set(idx);
                    lc.set_label(&os);
                    if let Some(f) = rfc.borrow().as_ref() {
                        f();
                    }
                    cbc(idx);
                });
                mbox.append(&w);
            }
            pop_c.set_child(Some(&mbox));
        })
    };
    *rebuild.borrow_mut() = Some(reb_fn.clone());
    reb_fn();
    btn
}

// ============================================================================
// Telemetría Linux y utilidades puras testables
// ============================================================================

fn collect_system_info_fast() -> SystemInfoSnapshot {
    let hostname = fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "malus-linux".to_string());

    let product_name = fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Equipo Malus Linux".to_string());
    let sys_vendor = fs::read_to_string("/sys/devices/virtual/dmi/id/sys_vendor")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "x86_64".to_string());
    let bios_date = fs::read_to_string("/sys/devices/virtual/dmi/id/bios_date")
        .ok()
        .and_then(|s| s.trim().split('/').next_back().map(ToString::to_string))
        .unwrap_or_default();
    let vendor_year = if bios_date.is_empty() {
        sys_vendor
    } else {
        format!("{sys_vendor}, {bios_date}")
    };

    let cpu_raw = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu_model =
        parse_proc_cpu_model(&cpu_raw).unwrap_or_else(|| "Procesador x86_64".to_string());

    let mem_raw = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let memory_gb = parse_proc_mem_total_gb(&mem_raw).unwrap_or_else(|| "—".to_string());

    let serial_or_machine_id = fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|s| s.trim().chars().take(12).collect::<String>().to_uppercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "MALUS-DE-001".to_string());

    let os_raw = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let os_pretty_name =
        parse_os_release_pretty_name(&os_raw).unwrap_or_else(|| "Malus DE (Linux)".to_string());

    let kernel_version = fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Linux".to_string());

    let mut displays = Vec::new();
    let disp_cfg = malus_de_common::config::DisplaysConfig::load();
    for out in disp_cfg.outputs {
        displays.push((
            format!("Pantalla {}", out.name),
            format!("{} × {}", out.width, out.height),
        ));
    }
    if displays.is_empty() {
        displays.push((
            "Pantalla principal Wayland".to_string(),
            "1920 × 1080".to_string(),
        ));
    }

    let (root_device, root_total_bytes, root_used_bytes, root_avail_bytes) =
        query_root_filesystem_usage();

    let timezone = fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().to_string();
            s.split("zoneinfo/").nth(1).map(ToString::to_string)
        })
        .unwrap_or_else(|| "America/Mexico_City".to_string());

    SystemInfoSnapshot {
        hostname,
        product_name,
        vendor_year,
        cpu_model,
        memory_gb,
        serial_or_machine_id,
        os_pretty_name,
        kernel_version,
        displays,
        root_device,
        root_total_bytes,
        root_used_bytes,
        root_avail_bytes,
        apps_bytes: 0,
        trash_bytes: 0,
        docs_bytes: 0,
        photos_bytes: 0,
        downloads_bytes: 0,
        home_other_bytes: 0,
        system_data_bytes: 0,
        os_base_bytes: root_used_bytes,
        ntp_active: true,
        timezone,
        sshd_active: is_systemd_unit_active("sshd"),
        cups_active: is_systemd_unit_active("cups"),
        smb_active: is_systemd_unit_active("smb"),
    }
}

fn is_systemd_unit_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .is_ok_and(|s| s.success())
}

fn query_root_filesystem_usage() -> (String, u64, u64, u64) {
    if let Ok(out) = Command::new("df").args(["-B1", "/"]).output() {
        if out.status.success() {
            let txt = String::from_utf8_lossy(&out.stdout);
            if let Some(parsed) = parse_df_bytes_output(&txt) {
                return parsed;
            }
        }
    }
    (
        "Volumen raíz (/)".to_string(),
        250_000_000_000,
        45_000_000_000,
        205_000_000_000,
    )
}

pub(crate) fn parse_df_bytes_output(raw: &str) -> Option<(String, u64, u64, u64)> {
    let line = raw.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 4 {
        return None;
    }
    let dev = cols[0].split('/').next_back().unwrap_or("Raíz").to_string();
    let total = cols[1].parse::<u64>().ok()?;
    let used = cols[2].parse::<u64>().ok()?;
    let avail = cols[3].parse::<u64>().ok()?;
    Some((dev, total, used, avail))
}

fn populate_storage_breakdown(snap: &mut SystemInfoSnapshot) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
    let home_p = Path::new(&home);
    snap.trash_bytes = dir_size_quick(&home_p.join(".local/share/Trash"));
    snap.docs_bytes =
        dir_size_quick(&home_p.join("Documentos")).max(dir_size_quick(&home_p.join("Documents")));
    snap.photos_bytes =
        dir_size_quick(&home_p.join("Imágenes")).max(dir_size_quick(&home_p.join("Pictures")));
    snap.downloads_bytes =
        dir_size_quick(&home_p.join("Descargas")).max(dir_size_quick(&home_p.join("Downloads")));
    let cache_bytes = dir_size_quick(&home_p.join(".cache"));
    let flatpak_bytes = dir_size_quick(&home_p.join(".local/share/flatpak"));

    snap.apps_bytes = flatpak_bytes.saturating_add((snap.root_used_bytes / 10).min(12_000_000_000));
    snap.system_data_bytes = cache_bytes.saturating_add(snap.root_used_bytes / 5);
    snap.home_other_bytes = dir_size_quick(&home_p.join("Proyectos"));
    let accounted = snap
        .apps_bytes
        .saturating_add(snap.trash_bytes)
        .saturating_add(snap.docs_bytes)
        .saturating_add(snap.photos_bytes)
        .saturating_add(snap.downloads_bytes)
        .saturating_add(snap.system_data_bytes)
        .saturating_add(snap.home_other_bytes);
    snap.os_base_bytes = snap.root_used_bytes.saturating_sub(accounted);
}

fn dir_size_quick(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if let Ok(out) = Command::new("du")
        .args(["-sb", &path.to_string_lossy()])
        .output()
    {
        if out.status.success() {
            if let Some(first) = String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
            {
                if let Ok(b) = first.parse::<u64>() {
                    return b;
                }
            }
        }
    }
    0
}

pub(crate) fn parse_proc_cpu_model(raw: &str) -> Option<String> {
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == "model name" {
                let cleaned = v.trim();
                if !cleaned.is_empty() {
                    return Some(cleaned.to_string());
                }
            }
        }
    }
    None
}

pub(crate) fn parse_proc_mem_total_gb(raw: &str) -> Option<String> {
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb_str = rest.trim().split_whitespace().next()?;
            let kb = kb_str.parse::<f64>().ok()?;
            let gb = (kb / (1024.0 * 1024.0)).round() as u64;
            return Some(format!("{gb} GB"));
        }
    }
    None
}

pub(crate) fn parse_os_release_pretty_name(raw: &str) -> Option<String> {
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            let val = rest.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

pub(crate) fn format_bytes_gb(bytes: u64) -> String {
    let gb = bytes as f64 / 1_000_000_000.0;
    format!("{gb:.2} GB")
}

pub(crate) fn format_bytes_human(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

fn format_current_datetime(use_24h: bool) -> String {
    let Ok(now) = glib::DateTime::now_local() else {
        return "—".to_string();
    };
    let fmt = if use_24h {
        "%d %b %Y, %H:%M:%S"
    } else {
        "%d %b %Y, %I:%M:%S %P"
    };
    now.format(fmt)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "—".to_string())
}

pub(crate) fn friendly_timezone_city(tz: &str) -> String {
    if let Some((region, city)) = tz.split_once('/') {
        let city_clean = city.replace('_', " ");
        format!("{city_clean} ({region})")
    } else {
        tz.to_string()
    }
}

pub(crate) fn format_regional_preview(prefs: &GeneralPreferences) -> String {
    let date_sample = match prefs.date_format_idx {
        1 => "2026-11-19",
        2 => "11/19/26",
        _ => "19/11/26",
    };
    let (money_sample, num_sample) = match prefs.number_format_idx {
        1 => ("$12.345,67", "4.567,89"),
        2 => ("$12 345,67", "4 567,89"),
        _ => ("$12,345.67", "4,567.89"),
    };
    let temp_sample = if prefs.celsius { "22 °C" } else { "72 °F" };
    format!(
        "jueves, 19 de noviembre de 2026 · {}\n{date_sample}, 9:21 pm   {money_sample}   {num_sample}   {temp_sample}",
        prefs.region
    )
}

fn autostart_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/autostart")
}

fn load_autostart_entries() -> Vec<AutostartEntry> {
    let dir = autostart_dir();
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("desktop") {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Some(parsed) = parse_autostart_desktop(&path, &raw) {
                    list.push(parsed);
                }
            }
        }
    }
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

pub(crate) fn parse_autostart_desktop(path: &Path, raw: &str) -> Option<AutostartEntry> {
    let mut name = String::new();
    let mut exec = String::new();
    let mut hidden = false;
    for line in raw.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("Name=") {
            if name.is_empty() {
                name = v.trim().to_string();
            }
        } else if let Some(v) = t.strip_prefix("Exec=") {
            exec = v.trim().to_string();
        } else if t == "Hidden=true" {
            hidden = true;
        }
    }
    if hidden || (name.is_empty() && exec.is_empty()) {
        return None;
    }
    if name.is_empty() {
        name = exec.clone();
    }
    Some(AutostartEntry {
        file_path: path.to_path_buf(),
        name,
        exec,
        kind_label: "Aplicación".to_string(),
    })
}

fn create_autostart_entry(name: &str, exec: &str) {
    let dir = autostart_dir();
    let _ = fs::create_dir_all(&dir);
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let path = dir.join(format!("malus-{slug}.desktop"));
    let content = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nX-GNOME-Autostart-enabled=true\n"
    );
    let _ = fs::write(path, content);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_hardware_and_df_info_cleanly() {
        let cpu = "processor\t: 0\nmodel name\t: AMD Ryzen 7 5800H with Radeon Graphics\n";
        assert_eq!(
            parse_proc_cpu_model(cpu).as_deref(),
            Some("AMD Ryzen 7 5800H with Radeon Graphics")
        );

        let mem = "MemTotal:       16304128 kB\nMemFree:         2048000 kB\n";
        assert_eq!(parse_proc_mem_total_gb(mem).as_deref(), Some("16 GB"));

        let os = "NAME=\"Fedora Linux\"\nPRETTY_NAME=\"Fedora Linux 45 (Workstation Edition)\"\n";
        assert_eq!(
            parse_os_release_pretty_name(os).as_deref(),
            Some("Fedora Linux 45 (Workstation Edition)")
        );

        let df = "Filesystem     1B-blocks        Used    Available Use% Mounted on\n/dev/nvme0n1p2 245110000000 44290000000 200810000000  19% /\n";
        let (dev, tot, used, avail) = parse_df_bytes_output(df).unwrap();
        assert_eq!(dev, "nvme0n1p2");
        assert_eq!(tot, 245_110_000_000);
        assert_eq!(used, 44_290_000_000);
        assert_eq!(avail, 200_810_000_000);
    }

    #[test]
    fn parses_autostart_desktop_and_formats_regional_preview() {
        let desktop =
            "[Desktop Entry]\nType=Application\nName=Nextcloud\nExec=nextcloud --background\n";
        let entry = parse_autostart_desktop(Path::new("/tmp/nextcloud.desktop"), desktop).unwrap();
        assert_eq!(entry.name, "Nextcloud");
        assert_eq!(entry.kind_label, "Aplicación");

        let mut prefs = GeneralPreferences::default();
        prefs.number_format_idx = 1;
        prefs.celsius = false;
        let preview = format_regional_preview(&prefs);
        assert!(preview.contains("$12.345,67"));
        assert!(preview.contains("72 °F"));
    }
}
