use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Image, Label, Orientation,
    PolicyType, Popover, ScrolledWindow, Stack, StringList, Window,
};
use malus_de_common::config::PrivacyConfig;
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, make_squircle_colored_badge, wrap_squircle_card,
    WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const PRIVACY_PANEL_ID: &str = "privacy";
pub const PRIVACY_LOCATION_SUBPANEL_ID: &str = "privacy-location";
pub const PRIVACY_FULL_DISK_SUBPANEL_ID: &str = "privacy-full-disk";
pub const PRIVACY_FILES_SUBPANEL_ID: &str = "privacy-files";
pub const PRIVACY_BLUETOOTH_SUBPANEL_ID: &str = "privacy-bluetooth";
pub const PRIVACY_CAMERA_SUBPANEL_ID: &str = "privacy-camera";
pub const PRIVACY_MICROPHONE_SUBPANEL_ID: &str = "privacy-microphone";
pub const PRIVACY_LOCAL_NETWORK_SUBPANEL_ID: &str = "privacy-local-network";
pub const PRIVACY_ANALYTICS_SUBPANEL_ID: &str = "privacy-analytics";
pub const PRIVACY_FILEVAULT_SUBPANEL_ID: &str = "privacy-filevault";
pub const PRIVACY_PANEL_TITLE: &str = "Privacidad y seguridad";

const BLUE_TOP: (f64, f64, f64) = (0.14, 0.54, 1.00);
const BLUE_BOT: (f64, f64, f64) = (0.04, 0.42, 0.90);
const CYAN_TOP: (f64, f64, f64) = (0.20, 0.68, 0.98);
const CYAN_BOT: (f64, f64, f64) = (0.08, 0.54, 0.88);
const GREEN_TOP: (f64, f64, f64) = (0.22, 0.80, 0.36);
const GREEN_BOT: (f64, f64, f64) = (0.14, 0.65, 0.26);
const ORANGE_TOP: (f64, f64, f64) = (1.00, 0.62, 0.08);
const ORANGE_BOT: (f64, f64, f64) = (0.90, 0.45, 0.02);
const RED_TOP: (f64, f64, f64) = (1.00, 0.31, 0.27);
const RED_BOT: (f64, f64, f64) = (0.89, 0.16, 0.14);
const INDIGO_TOP: (f64, f64, f64) = (0.37, 0.36, 0.90);
const INDIGO_BOT: (f64, f64, f64) = (0.29, 0.27, 0.78);
const PINK_TOP: (f64, f64, f64) = (0.98, 0.22, 0.42);
const PINK_BOT: (f64, f64, f64) = (0.86, 0.12, 0.30);
const PURPLE_TOP: (f64, f64, f64) = (0.68, 0.32, 0.87);
const PURPLE_BOT: (f64, f64, f64) = (0.54, 0.20, 0.76);
const GRAY_TOP: (f64, f64, f64) = (0.56, 0.57, 0.60);
const GRAY_BOT: (f64, f64, f64) = (0.43, 0.44, 0.48);

pub fn is_privacy_subpanel(id: &str) -> bool {
    matches!(
        id,
        PRIVACY_PANEL_ID
            | PRIVACY_LOCATION_SUBPANEL_ID
            | PRIVACY_FULL_DISK_SUBPANEL_ID
            | PRIVACY_FILES_SUBPANEL_ID
            | PRIVACY_BLUETOOTH_SUBPANEL_ID
            | PRIVACY_CAMERA_SUBPANEL_ID
            | PRIVACY_MICROPHONE_SUBPANEL_ID
            | PRIVACY_LOCAL_NETWORK_SUBPANEL_ID
            | PRIVACY_ANALYTICS_SUBPANEL_ID
            | PRIVACY_FILEVAULT_SUBPANEL_ID
    )
}

pub fn privacy_subpanel_title(id: &str) -> Option<&'static str> {
    match id {
        PRIVACY_PANEL_ID => Some(PRIVACY_PANEL_TITLE),
        PRIVACY_LOCATION_SUBPANEL_ID => Some("Localización"),
        PRIVACY_FULL_DISK_SUBPANEL_ID => Some("Acceso completo al disco"),
        PRIVACY_FILES_SUBPANEL_ID => Some("Archivos y carpetas"),
        PRIVACY_BLUETOOTH_SUBPANEL_ID => Some("Bluetooth"),
        PRIVACY_CAMERA_SUBPANEL_ID => Some("Cámara"),
        PRIVACY_MICROPHONE_SUBPANEL_ID => Some("Micrófono"),
        PRIVACY_LOCAL_NETWORK_SUBPANEL_ID => Some("Red local"),
        PRIVACY_ANALYTICS_SUBPANEL_ID => Some("Análisis y mejoras"),
        PRIVACY_FILEVAULT_SUBPANEL_ID => Some("FileVault"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatpakAppPerm {
    app_id: String,
    name: String,
    full_disk: bool,
    home_files: bool,
    bluetooth: bool,
    camera: bool,
    microphone: bool,
    network: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LuksVolumeInfo {
    name: String,
    size: String,
    mountpoint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlatpakPermKind {
    FullDisk,
    HomeFiles,
    Bluetooth,
    Camera,
    Microphone,
    Network,
}

impl FlatpakPermKind {
    fn has(&self, app: &FlatpakAppPerm) -> bool {
        match self {
            Self::FullDisk => app.full_disk,
            Self::HomeFiles => app.home_files,
            Self::Bluetooth => app.bluetooth,
            Self::Camera => app.camera,
            Self::Microphone => app.microphone,
            Self::Network => app.network,
        }
    }

    fn override_flag(&self, allow: bool) -> &'static str {
        match (self, allow) {
            (Self::FullDisk, true) => "--filesystem=host",
            (Self::FullDisk, false) => "--nofilesystem=host",
            (Self::HomeFiles, true) => "--filesystem=home",
            (Self::HomeFiles, false) => "--nofilesystem=home",
            (Self::Bluetooth, true) => "--allow=bluetooth",
            (Self::Bluetooth, false) => "--disallow=bluetooth",
            (Self::Camera, true) => "--device=all",
            (Self::Camera, false) => "--nodevice=all",
            (Self::Microphone, true) => "--socket=pulseaudio",
            (Self::Microphone, false) => "--nosocket=pulseaudio",
            (Self::Network, true) => "--share=network",
            (Self::Network, false) => "--unshare=network",
        }
    }
}

fn parse_flatpak_permissions(app_id: &str, name: &str, raw: &str) -> FlatpakAppPerm {
    let mut full_disk = false;
    let mut home_files = false;
    let mut bluetooth = false;
    let mut camera = false;
    let mut microphone = false;
    let mut network = false;

    for line in raw.lines().map(str::trim) {
        if let Some(val) = line.strip_prefix("filesystems=") {
            for item in val.split(';').filter(|s| !s.is_empty()) {
                if item == "host" || item == "host:ro" || item == "host:rw" {
                    full_disk = true;
                } else if item == "home"
                    || item.starts_with("home:")
                    || item.starts_with("xdg-")
                    || item.starts_with("~/")
                {
                    home_files = true;
                }
            }
        } else if let Some(val) = line.strip_prefix("shared=") {
            for item in val.split(';').filter(|s| !s.is_empty()) {
                if item == "network" {
                    network = true;
                }
            }
        } else if let Some(val) = line.strip_prefix("features=") {
            for item in val.split(';').filter(|s| !s.is_empty()) {
                if item == "bluetooth" {
                    bluetooth = true;
                }
            }
        } else if let Some(val) = line.strip_prefix("devices=") {
            for item in val.split(';').filter(|s| !s.is_empty()) {
                if item == "all" {
                    camera = true;
                }
            }
        } else if let Some(val) = line.strip_prefix("sockets=") {
            for item in val.split(';').filter(|s| !s.is_empty()) {
                if item == "pulseaudio" || item == "pipewire" {
                    microphone = true;
                }
            }
        }
    }

    FlatpakAppPerm {
        app_id: app_id.to_string(),
        name: name.to_string(),
        full_disk,
        home_files,
        bluetooth,
        camera,
        microphone,
        network,
    }
}

fn probe_flatpak_permissions() -> Vec<FlatpakAppPerm> {
    let Ok(out) = Command::new("flatpak")
        .args(["list", "--app", "--columns=application,name"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.splitn(2, '\t');
        let Some(app_id) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let name = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(app_id);
        let perm_raw = Command::new("flatpak")
            .args(["info", "--show-permissions", app_id])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let override_raw = Command::new("flatpak")
            .args(["override", "--user", "--show", app_id])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();

        let mut perm = parse_flatpak_permissions(app_id, name, &perm_raw);
        // Aplicar overrides locales del usuario si existen
        for oline in override_raw.lines().map(str::trim) {
            if let Some(val) = oline.strip_prefix("filesystems=") {
                for item in val.split(';').filter(|s| !s.is_empty()) {
                    match item {
                        "!host" => perm.full_disk = false,
                        "host" | "host:rw" | "host:ro" => perm.full_disk = true,
                        "!home" => perm.home_files = false,
                        "home" | "home:rw" | "home:ro" => perm.home_files = true,
                        _ => {}
                    }
                }
            } else if let Some(val) = oline.strip_prefix("shared=") {
                for item in val.split(';').filter(|s| !s.is_empty()) {
                    if item == "!network" {
                        perm.network = false;
                    } else if item == "network" {
                        perm.network = true;
                    }
                }
            } else if let Some(val) = oline.strip_prefix("features=") {
                for item in val.split(';').filter(|s| !s.is_empty()) {
                    if item == "!bluetooth" {
                        perm.bluetooth = false;
                    } else if item == "bluetooth" {
                        perm.bluetooth = true;
                    }
                }
            } else if let Some(val) = oline.strip_prefix("devices=") {
                for item in val.split(';').filter(|s| !s.is_empty()) {
                    if item == "!all" {
                        perm.camera = false;
                    } else if item == "all" {
                        perm.camera = true;
                    }
                }
            } else if let Some(val) = oline.strip_prefix("sockets=") {
                for item in val.split(';').filter(|s| !s.is_empty()) {
                    if item == "!pulseaudio" {
                        perm.microphone = false;
                    } else if item == "pulseaudio" || item == "pipewire" {
                        perm.microphone = true;
                    }
                }
            }
        }
        results.push(perm);
    }
    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results
}

fn parse_lsblk_luks(raw: &str) -> Vec<LuksVolumeInfo> {
    let mut vols = Vec::new();
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 3 && cols[1] == "crypt" {
            let name = cols[0].to_string();
            let size = cols[2].to_string();
            let mountpoint = cols.get(3).copied().unwrap_or("—").to_string();
            vols.push(LuksVolumeInfo {
                name,
                size,
                mountpoint,
            });
        }
    }
    vols
}

fn probe_luks_volumes() -> Vec<LuksVolumeInfo> {
    let Ok(out) = Command::new("lsblk")
        .args(["-o", "NAME,TYPE,SIZE,MOUNTPOINT", "-rn"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_lsblk_luks(&String::from_utf8_lossy(&out.stdout))
}

fn read_gsetting_bool(schema: &str, key: &str, fallback: bool) -> bool {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return fallback;
    };
    if source.lookup(schema, true).is_none() {
        return fallback;
    }
    gtk4::gio::Settings::new(schema).boolean(key)
}

fn write_gsetting_bool(schema: &str, key: &str, value: bool) {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return;
    };
    if source.lookup(schema, true).is_none() {
        return;
    }
    let _ = gtk4::gio::Settings::new(schema).set_boolean(key, value);
}

fn read_gsetting_string(schema: &str, key: &str, fallback: &str) -> String {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return fallback.to_string();
    };
    if source.lookup(schema, true).is_none() {
        return fallback.to_string();
    }
    gtk4::gio::Settings::new(schema).string(key).to_string()
}

fn write_gsetting_string(schema: &str, key: &str, value: &str) {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return;
    };
    if source.lookup(schema, true).is_none() {
        return;
    }
    let _ = gtk4::gio::Settings::new(schema).set_string(key, value);
}

fn read_usb_accessories_index() -> u32 {
    let enabled = read_gsetting_bool("org.gnome.desktop.privacy", "usb-protection", true);
    if !enabled {
        return 2;
    }
    match read_gsetting_string(
        "org.gnome.desktop.privacy",
        "usb-protection-level",
        "lockscreen",
    )
    .as_str()
    {
        "always" => 1,
        _ => 0,
    }
}

fn write_usb_accessories_index(idx: u32) {
    match idx {
        0 => {
            write_gsetting_bool("org.gnome.desktop.privacy", "usb-protection", true);
            write_gsetting_string(
                "org.gnome.desktop.privacy",
                "usb-protection-level",
                "lockscreen",
            );
        }
        1 => {
            write_gsetting_bool("org.gnome.desktop.privacy", "usb-protection", true);
            write_gsetting_string("org.gnome.desktop.privacy", "usb-protection-level", "always");
        }
        _ => {
            write_gsetting_bool("org.gnome.desktop.privacy", "usb-protection", false);
        }
    }
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

fn make_banner_card(
    icon_name: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    subtitle: &str,
) -> GtkBox {
    let card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    card.append(&make_squircle_colored_badge(
        34,
        icon_name,
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
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
            .label(subtitle)
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    card.append(&col);
    card
}

fn make_nav_row(
    icon_name: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    subtitle: Option<&str>,
    right_value: Option<&str>,
    pending: bool,
    on_click: Option<Rc<dyn Fn()>>,
) -> (Button, Option<Label>, Option<Label>) {
    let btn = Button::builder()
        .css_classes(["wifi-nav-row"])
        .has_frame(false)
        .hexpand(true)
        .sensitive(on_click.is_some())
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .valign(Align::Center)
        .build();

    row.append(&make_squircle_colored_badge(
        24,
        icon_name,
        13,
        6.5,
        top,
        bot,
    ));

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .hexpand(true)
        .valign(Align::Center)
        .build();

    let title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();
    title_row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    if pending {
        title_row.append(&make_pending_badge());
    }
    text_col.append(&title_row);

    let sub_lbl = subtitle.map(|s| {
        let lbl = Label::builder()
            .label(s)
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build();
        text_col.append(&lbl);
        lbl
    });
    row.append(&text_col);

    let val_lbl = right_value.map(|v| {
        let lbl = Label::builder()
            .label(v)
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build();
        row.append(&lbl);
        lbl
    });

    row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(13)
            .css_classes(["wifi-chevron"])
            .valign(Align::Center)
            .build(),
    );

    btn.set_child(Some(&row));
    if let Some(cb) = on_click {
        btn.connect_clicked(move |_| cb());
    }
    (btn, sub_lbl, val_lbl)
}

fn format_app_count_sub(count: usize) -> String {
    match count {
        0 => "Ninguna".to_string(),
        1 => "1 app".to_string(),
        n => format!("{n} apps"),
    }
}

fn show_advanced_modal(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<PrivacyConfig>>,
    navigate: Rc<dyn Fn(&str)>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(450)
        .css_classes(["macos-settings-window"])
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    root.append(
        &Label::builder()
            .label("Opciones avanzadas de privacidad y seguridad")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let make_sw_row = |label: &str, pending: bool, active: bool| -> (GtkBox, Switch) {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wifi-pref-row"])
            .build();
        let left = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        left.append(
            &Label::builder()
                .label(label)
                .css_classes(["wifi-row-title"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        if pending {
            left.append(&make_pending_badge());
        }
        let sw = Switch::builder()
            .valign(Align::Center)
            .active(active)
            .build();
        row.append(&left);
        row.append(&*sw);
        (row, sw)
    };

    let (hide_row, hide_sw) = make_sw_row(
        "Ocultar identidad del usuario en la barra superior",
        false,
        read_gsetting_bool("org.gnome.desktop.privacy", "hide-identity", false),
    );
    hide_sw.connect_state_set(|_, active| {
        write_gsetting_bool("org.gnome.desktop.privacy", "hide-identity", active);
        glib::Propagation::Proceed
    });

    let (clean_row, clean_sw) = make_sw_row(
        "Eliminar archivos temporales y papelera antiguos automáticamente",
        false,
        read_gsetting_bool("org.gnome.desktop.privacy", "remove-old-temp-files", false),
    );
    clean_sw.connect_state_set(|_, active| {
        write_gsetting_bool("org.gnome.desktop.privacy", "remove-old-temp-files", active);
        write_gsetting_bool("org.gnome.desktop.privacy", "remove-old-trash-files", active);
        glib::Propagation::Proceed
    });

    let (admin_row, admin_sw) = make_sw_row(
        "Requerir contraseña de administrador para ajustes del sistema",
        true,
        cfg.borrow().require_admin_for_system_prefs,
    );
    {
        let cfg_c = cfg.clone();
        admin_sw.connect_state_set(move |_, active| {
            let mut c = cfg_c.borrow_mut();
            c.require_admin_for_system_prefs = active;
            let _ = c.save();
            glib::Propagation::Proceed
        });
    }

    card.append(&hide_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&clean_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&admin_row);
    root.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .build();
    let (fw_wrap, fw_btn) = make_squircle_button("Abrir Firewall...", false, None);
    let spacer = GtkBox::builder().hexpand(true).build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&fw_wrap);
    actions.append(&spacer);
    actions.append(&ok_wrap);
    root.append(&actions);

    dialog.set_child(Some(&root));

    let dlg_fw = dialog.clone();
    fw_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_fw);
        navigate("network-firewall");
    });

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_ok);
    });

    crate::present_attached_modal(&dialog, parent);
}

/// Cargador bajo demanda para «Privacidad y seguridad» y todas sus subvistas reales.
pub fn create_privacy_pane_loader(
    window: &ApplicationWindow,
    stack: &Stack,
    navigate: Rc<dyn Fn(&str)>,
) -> Rc<dyn Fn(&str)> {
    let win_c = window.clone();
    let stack_c = stack.clone();
    let cfg = Rc::new(RefCell::new(PrivacyConfig::load()));
    let flatpaks: Rc<RefCell<Vec<FlatpakAppPerm>>> = Rc::new(RefCell::new(Vec::new()));
    let luks_vols: Rc<RefCell<Vec<LuksVolumeInfo>>> = Rc::new(RefCell::new(Vec::new()));

    let refresh_main: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_location: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_full_disk: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_files: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_bt: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_cam: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_mic: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_net: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_analytics: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_filevault: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    Rc::new(move |target: &str| {
        *cfg.borrow_mut() = PrivacyConfig::load();
        *flatpaks.borrow_mut() = probe_flatpak_permissions();
        *luks_vols.borrow_mut() = probe_luks_volumes();

        match target {
            PRIVACY_PANEL_ID => {
                if stack_c.child_by_name(PRIVACY_PANEL_ID).is_none() {
                    let (pane, rf) = build_privacy_main_pane(
                        &win_c,
                        &cfg,
                        &flatpaks,
                        &luks_vols,
                        navigate.clone(),
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_PANEL_ID));
                    *refresh_main.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_main.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_LOCATION_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_LOCATION_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_location_pane();
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_LOCATION_SUBPANEL_ID));
                    *refresh_location.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_location.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_FULL_DISK_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_FULL_DISK_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_flatpak_perm_pane(
                        "drive-harddisk-symbolic",
                        GRAY_TOP,
                        GRAY_BOT,
                        "Acceso completo al disco",
                        "Controla qué aplicaciones Flatpak tienen acceso completo al sistema de archivos del equipo (filesystem=host).",
                        FlatpakPermKind::FullDisk,
                        None,
                        &flatpaks,
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_FULL_DISK_SUBPANEL_ID));
                    *refresh_full_disk.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_full_disk.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_FILES_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_FILES_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_flatpak_perm_pane(
                        "folder-symbolic",
                        CYAN_TOP,
                        CYAN_BOT,
                        "Archivos y carpetas",
                        "Administra qué aplicaciones Flatpak pueden acceder a tu carpeta personal (filesystem=home).",
                        FlatpakPermKind::HomeFiles,
                        None,
                        &flatpaks,
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_FILES_SUBPANEL_ID));
                    *refresh_files.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_files.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_BLUETOOTH_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_BLUETOOTH_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_flatpak_perm_pane(
                        "bluetooth-active-symbolic",
                        BLUE_TOP,
                        BLUE_BOT,
                        "Bluetooth",
                        "Controla qué aplicaciones Flatpak tienen permiso para comunicarse con dispositivos Bluetooth.",
                        FlatpakPermKind::Bluetooth,
                        None,
                        &flatpaks,
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_BLUETOOTH_SUBPANEL_ID));
                    *refresh_bt.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_bt.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_CAMERA_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_CAMERA_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_flatpak_perm_pane(
                        "camera-web-symbolic",
                        GRAY_TOP,
                        GRAY_BOT,
                        "Cámara",
                        "Controla el acceso global a la cámara del sistema y los permisos por aplicación Flatpak.",
                        FlatpakPermKind::Camera,
                        Some((
                            "Permitir acceso a la cámara",
                            "org.gnome.desktop.privacy",
                            "disable-camera",
                        )),
                        &flatpaks,
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_CAMERA_SUBPANEL_ID));
                    *refresh_cam.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_cam.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_MICROPHONE_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_MICROPHONE_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_flatpak_perm_pane(
                        "audio-input-microphone-symbolic",
                        GRAY_TOP,
                        GRAY_BOT,
                        "Micrófono",
                        "Controla el acceso global al micrófono y el permiso de audio (PulseAudio/PipeWire) para aplicaciones Flatpak.",
                        FlatpakPermKind::Microphone,
                        Some((
                            "Permitir acceso al micrófono",
                            "org.gnome.desktop.privacy",
                            "disable-microphone",
                        )),
                        &flatpaks,
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_MICROPHONE_SUBPANEL_ID));
                    *refresh_mic.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_mic.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_LOCAL_NETWORK_SUBPANEL_ID => {
                if stack_c
                    .child_by_name(PRIVACY_LOCAL_NETWORK_SUBPANEL_ID)
                    .is_none()
                {
                    let (pane, rf) = build_flatpak_perm_pane(
                        "network-workgroup-symbolic",
                        BLUE_TOP,
                        BLUE_BOT,
                        "Red local",
                        "Controla qué aplicaciones Flatpak tienen acceso a la red del equipo (share=network).",
                        FlatpakPermKind::Network,
                        None,
                        &flatpaks,
                    );
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_LOCAL_NETWORK_SUBPANEL_ID));
                    *refresh_net.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_net.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_ANALYTICS_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_ANALYTICS_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_analytics_pane();
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_ANALYTICS_SUBPANEL_ID));
                    *refresh_analytics.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_analytics.borrow().as_ref() {
                    rf();
                }
            }
            PRIVACY_FILEVAULT_SUBPANEL_ID => {
                if stack_c.child_by_name(PRIVACY_FILEVAULT_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_filevault_pane(&luks_vols);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(PRIVACY_FILEVAULT_SUBPANEL_ID));
                    *refresh_filevault.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_filevault.borrow().as_ref() {
                    rf();
                }
            }
            _ => {}
        }
    })
}

fn build_privacy_main_pane(
    window: &ApplicationWindow,
    cfg: &Rc<RefCell<PrivacyConfig>>,
    flatpaks: &Rc<RefCell<Vec<FlatpakAppPerm>>>,
    luks_vols: &Rc<RefCell<Vec<LuksVolumeInfo>>>,
    navigate: Rc<dyn Fn(&str)>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();
    let syncing = Rc::new(Cell::new(false));

    // Banner principal
    let banner = make_banner_card(
        "security-high-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Privacidad",
        "Controla qué apps pueden acceder a tus datos, ubicación, cámara y micrófono, y administra la protección de seguridad.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 1: Localización (real via GSettings)
    // ========================================================
    let loc_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let nav_loc = navigate.clone();
    let (loc_row, _, loc_val_lbl) = make_nav_row(
        "find-location-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Localización",
        None,
        Some("Desactivado"),
        false,
        Some(Rc::new(move || nav_loc(PRIVACY_LOCATION_SUBPANEL_ID))),
    );
    loc_card.append(&loc_row);
    pane.append(&wrap_squircle_card(&loc_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 2: Permisos de datos personales (9 filas)
    // ========================================================
    let data_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let nav_fd = navigate.clone();
    let (full_disk_row, full_disk_sub_lbl, _) = make_nav_row(
        "drive-harddisk-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Acceso completo al disco",
        Some("Ninguna"),
        None,
        false,
        Some(Rc::new(move || nav_fd(PRIVACY_FULL_DISK_SUBPANEL_ID))),
    );
    let (passkeys_row, _, _) = make_nav_row(
        "dialog-password-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Acceso de navegadores a llaves de acceso",
        Some("Ninguna"),
        None,
        true,
        None,
    );
    let nav_files = navigate.clone();
    let (files_row, files_sub_lbl, _) = make_nav_row(
        "folder-symbolic",
        CYAN_TOP,
        CYAN_BOT,
        "Archivos y carpetas",
        Some("Ninguna"),
        None,
        false,
        Some(Rc::new(move || nav_files(PRIVACY_FILES_SUBPANEL_ID))),
    );
    let (cal_row, _, _) = make_nav_row(
        "x-office-calendar-symbolic",
        RED_TOP,
        RED_BOT,
        "Calendarios",
        Some("Ninguna"),
        None,
        true,
        None,
    );
    let (home_row, _, _) = make_nav_row(
        "go-home-symbolic",
        ORANGE_TOP,
        ORANGE_BOT,
        "Casa",
        Some("Ninguna"),
        None,
        true,
        None,
    );
    let (contacts_row, _, _) = make_nav_row(
        "avatar-default-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Contactos",
        Some("Ninguna"),
        None,
        true,
        None,
    );
    let (media_row, _, _) = make_nav_row(
        "audio-x-generic-symbolic",
        PINK_TOP,
        PINK_BOT,
        "Contenidos y Apple Music",
        Some("Ninguna"),
        None,
        true,
        None,
    );
    let (photos_row, _, _) = make_nav_row(
        "image-x-generic-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Fotos",
        Some("Ninguna"),
        None,
        true,
        None,
    );
    let (reminders_row, _, _) = make_nav_row(
        "view-list-symbolic",
        ORANGE_TOP,
        ORANGE_BOT,
        "Recordatorios",
        Some("Ninguna"),
        None,
        true,
        None,
    );

    let data_rows = [
        full_disk_row,
        passkeys_row,
        files_row,
        cal_row,
        home_row,
        contacts_row,
        media_row,
        photos_row,
        reminders_row,
    ];
    for (idx, row) in data_rows.into_iter().enumerate() {
        if idx > 0 {
            data_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        data_card.append(&row);
    }
    pane.append(&wrap_squircle_card(&data_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 3: Permisos de hardware y sistema (14 filas)
    // ========================================================
    let hw_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let (app_mgmt_row, _, _) = make_nav_row(
        "application-x-executable-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Administración de apps",
        None,
        Some("0"),
        true,
        None,
    );
    let (auto_row, _, _) = make_nav_row(
        "emblem-system-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Automatización",
        None,
        Some("0"),
        true,
        None,
    );
    let nav_bt = navigate.clone();
    let (bt_row, _, bt_val_lbl) = make_nav_row(
        "bluetooth-active-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Bluetooth",
        None,
        Some("0"),
        false,
        Some(Rc::new(move || nav_bt(PRIVACY_BLUETOOTH_SUBPANEL_ID))),
    );
    let nav_cam = navigate.clone();
    let (cam_row, _, cam_val_lbl) = make_nav_row(
        "camera-web-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Cámara",
        None,
        Some("0"),
        false,
        Some(Rc::new(move || nav_cam(PRIVACY_CAMERA_SUBPANEL_ID))),
    );
    let (fitness_row, _, _) = make_nav_row(
        "emblem-favorite-symbolic",
        GREEN_TOP,
        GREEN_BOT,
        "Condición y actividad física",
        None,
        Some("0"),
        true,
        None,
    );
    let (dev_ctrl_row, _, _) = make_nav_row(
        "computer-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Control de dispositivos y acceso a los datos",
        None,
        Some("0"),
        true,
        None,
    );
    let (focus_row, _, _) = make_nav_row(
        "weather-clear-night-symbolic",
        INDIGO_TOP,
        INDIGO_BOT,
        "Enfoque",
        None,
        Some("0"),
        true,
        None,
    );
    let (rd_row, _, _) = make_nav_row(
        "preferences-desktop-remote-desktop-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Escritorio remoto",
        None,
        Some("0"),
        true,
        None,
    );
    let (screen_rec_row, _, _) = make_nav_row(
        "media-record-symbolic",
        RED_TOP,
        RED_BOT,
        "Grabación de audio del sistema y pantalla",
        None,
        Some("0"),
        true,
        None,
    );
    let (dev_tools_row, _, _) = make_nav_row(
        "utilities-terminal-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Herramientas para desarrolladores",
        None,
        Some("0"),
        true,
        None,
    );
    let nav_mic = navigate.clone();
    let (mic_row, _, mic_val_lbl) = make_nav_row(
        "audio-input-microphone-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Micrófono",
        None,
        Some("0"),
        false,
        Some(Rc::new(move || nav_mic(PRIVACY_MICROPHONE_SUBPANEL_ID))),
    );
    let (input_mon_row, _, _) = make_nav_row(
        "input-keyboard-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Monitorización de entrada",
        None,
        Some("0"),
        true,
        None,
    );
    let (speech_row, _, _) = make_nav_row(
        "microphone-sensitivity-high-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "Reconocimiento de voz",
        None,
        Some("0"),
        true,
        None,
    );
    let nav_net = navigate.clone();
    let (local_net_row, _, net_val_lbl) = make_nav_row(
        "network-workgroup-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Red local",
        None,
        Some("0"),
        false,
        Some(Rc::new(move || nav_net(PRIVACY_LOCAL_NETWORK_SUBPANEL_ID))),
    );

    let hw_rows = [
        app_mgmt_row,
        auto_row,
        bt_row,
        cam_row,
        fitness_row,
        dev_ctrl_row,
        focus_row,
        rd_row,
        screen_rec_row,
        dev_tools_row,
        mic_row,
        input_mon_row,
        speech_row,
        local_net_row,
    ];
    for (idx, row) in hw_rows.into_iter().enumerate() {
        if idx > 0 {
            hw_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        hw_card.append(&row);
    }
    pane.append(&wrap_squircle_card(&hw_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 4: Advertencia de contenido íntimo (por implementar)
    // ========================================================
    let sensitive_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let (sens_row, _, _) = make_nav_row(
        "dialog-warning-symbolic",
        CYAN_TOP,
        CYAN_BOT,
        "Advertencia de contenido íntimo",
        None,
        Some(if cfg.borrow().sensitive_content_warning {
            "Sí"
        } else {
            "No"
        }),
        true,
        None,
    );
    sensitive_card.append(&sens_row);
    pane.append(&wrap_squircle_card(
        &sensitive_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ========================================================
    // Tarjeta 5: Contactos bloqueados (por implementar)
    // ========================================================
    let blocked_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let (blocked_row, _, _) = make_nav_row(
        "action-unavailable-symbolic",
        RED_TOP,
        RED_BOT,
        "Contactos bloqueados",
        None,
        None,
        true,
        None,
    );
    blocked_card.append(&blocked_row);
    pane.append(&wrap_squircle_card(
        &blocked_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ========================================================
    // Tarjeta 6: Análisis y mejoras (REAL) + Publicidad de Apple (por implementar)
    // ========================================================
    let analytics_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let nav_ana = navigate.clone();
    let (ana_row, _, _) = make_nav_row(
        "utilities-system-monitor-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Análisis y mejoras",
        None,
        None,
        false,
        Some(Rc::new(move || nav_ana(PRIVACY_ANALYTICS_SUBPANEL_ID))),
    );
    let (ads_row, _, _) = make_nav_row(
        "emblem-shared-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Publicidad de Apple",
        None,
        None,
        true,
        None,
    );
    analytics_card.append(&ana_row);
    analytics_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    analytics_card.append(&ads_row);
    pane.append(&wrap_squircle_card(
        &analytics_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ========================================================
    // Tarjeta 7: Reporte de Apple Intelligence (por implementar)
    // ========================================================
    let ai_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let (ai_row, _, _) = make_nav_row(
        "starred-symbolic",
        PURPLE_TOP,
        PURPLE_BOT,
        "Reporte de Apple Intelligence",
        None,
        Some(if cfg.borrow().intelligence_report {
            "Activado"
        } else {
            "Desactivado"
        }),
        true,
        None,
    );
    ai_card.append(&ai_row);
    pane.append(&wrap_squircle_card(&ai_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Sección: Seguridad
    // ========================================================
    pane.append(
        &Label::builder()
            .label("Seguridad")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    // Tarjeta 8: Permitir apps de (por implementar, persistido)
    let allow_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let allow_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let allow_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    allow_left.append(
        &Label::builder()
            .label("Permitir apps de")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    allow_left.append(&make_pending_badge());
    let allow_model = StringList::new(&[
        "App Store",
        "App Store y desarrolladores conocidos",
        "Cualquier fuente",
    ]);
    let allow_dd = DropDown::builder()
        .model(&allow_model)
        .selected((cfg.borrow().allow_apps_from as u32).min(2))
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    allow_row.append(&allow_left);
    allow_row.append(&allow_dd);
    allow_card.append(&allow_row);
    pane.append(&wrap_squircle_card(&allow_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta 9: FileVault (REAL via lsblk / LUKS)
    let fv_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let nav_fv = navigate.clone();
    let (fv_row, _, fv_val_lbl) = make_nav_row(
        "drive-harddisk-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "FileVault",
        None,
        Some("Desactivado"),
        false,
        Some(Rc::new(move || nav_fv(PRIVACY_FILEVAULT_SUBPANEL_ID))),
    );
    fv_card.append(&fv_row);
    pane.append(&wrap_squircle_card(&fv_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta 10: Accesorios (REAL via GSettings usb-protection)
    let acc_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let acc_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    acc_row.append(&make_squircle_colored_badge(
        24,
        "drive-removable-media-symbolic",
        13,
        6.5,
        GRAY_TOP,
        GRAY_BOT,
    ));
    acc_row.append(
        &Label::builder()
            .label("Accesorios")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let acc_model = StringList::new(&["Preguntar", "Siempre bloquear nuevos USB", "Permitir siempre"]);
    let acc_dd = DropDown::builder()
        .model(&acc_model)
        .selected(read_usb_accessories_index())
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    acc_row.append(&acc_dd);
    acc_card.append(&acc_row);
    pane.append(&wrap_squircle_card(&acc_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta 11: Mejoras de seguridad en segundo plano (por implementar)
    let bg_sec_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let (bg_sec_row, _, _) = make_nav_row(
        "software-update-available-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Mejoras de seguridad en segundo plano",
        None,
        None,
        true,
        None,
    );
    bg_sec_card.append(&bg_sec_row);
    pane.append(&wrap_squircle_card(
        &bg_sec_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Tarjeta 12: Modo hermético (por implementar) + nota al pie
    let lockdown_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let (lockdown_row, _, _) = make_nav_row(
        "security-high-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Modo hermético",
        None,
        Some(if cfg.borrow().lockdown_mode {
            "Activado"
        } else {
            "Desactivado"
        }),
        true,
        None,
    );
    lockdown_card.append(&lockdown_row);
    pane.append(&wrap_squircle_card(
        &lockdown_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));
    pane.append(
        &Label::builder()
            .label(
                "Tu equipo puede reducir su exposición a amenazas digitales al aplicar límites \
                 de seguridad a las apps y las funciones. Esta opción está pensada para personas \
                 en situaciones de alto riesgo, como periodistas, activistas y figuras públicas.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(6)
            .margin_end(6)
            .build(),
    );

    // ========================================================
    // Pie inferior: Avanzado... y ?
    // ========================================================
    let footer_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (adv_wrap, adv_btn) = make_squircle_button("Avanzado...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Privacidad y seguridad");

    let help_popover = Popover::builder().has_arrow(true).autohide(true).build();
    let help_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .width_request(300)
        .build();
    help_box.append(
        &Label::builder()
            .label("Privacidad y seguridad")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "Localización, Cámara, Micrófono, Análisis y mejoras, Accesorios USB y Avanzado \
                 se sincronizan con GSettings y permisos de Flatpak; FileVault detecta en vivo \
                 los volúmenes cifrados con LUKS. Las funciones exclusivas de macOS sin equivalente \
                 directo en Linux aparecen marcadas como «por implementar».",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    help_popover.set_child(Some(&help_box));
    help_btn.set_popover(Some(&help_popover));

    footer_row.append(&adv_wrap);
    footer_row.append(&help_wrap);
    pane.append(&footer_row);

    // Señales interactivas
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        allow_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let mut c = cfg_c.borrow_mut();
            c.allow_apps_from = dd.selected().min(2) as u8;
            let _ = c.save();
        });
    }
    {
        let sync_c = syncing.clone();
        acc_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            write_usb_accessories_index(dd.selected());
        });
    }
    {
        let win_c = window.clone();
        let cfg_c = cfg.clone();
        let nav_c = navigate;
        adv_btn.connect_clicked(move |_| {
            show_advanced_modal(&win_c, &cfg_c, nav_c.clone());
        });
    }

    // Refresco vivo de contadores y estados
    let refresh: Rc<dyn Fn()> = {
        let cfg_c = cfg.clone();
        let fp_c = flatpaks.clone();
        let luks_c = luks_vols.clone();
        let sync_c = syncing;
        Rc::new(move || {
            let apps = fp_c.borrow();
            let full_disk_n = apps.iter().filter(|a| a.full_disk).count();
            let files_n = apps.iter().filter(|a| a.home_files).count();
            let bt_n = apps.iter().filter(|a| a.bluetooth).count();
            let cam_n = apps.iter().filter(|a| a.camera).count();
            let mic_n = apps.iter().filter(|a| a.microphone).count();
            let net_n = apps.iter().filter(|a| a.network).count();

            if let Some(ref lbl) = loc_val_lbl {
                let loc_on = read_gsetting_bool("org.gnome.system.location", "enabled", false);
                lbl.set_label(if loc_on { "Activado" } else { "Desactivado" });
            }
            if let Some(ref lbl) = full_disk_sub_lbl {
                lbl.set_label(&format_app_count_sub(full_disk_n));
            }
            if let Some(ref lbl) = files_sub_lbl {
                lbl.set_label(&format_app_count_sub(files_n));
            }
            if let Some(ref lbl) = bt_val_lbl {
                lbl.set_label(&bt_n.to_string());
            }
            if let Some(ref lbl) = cam_val_lbl {
                if read_gsetting_bool("org.gnome.desktop.privacy", "disable-camera", false) {
                    lbl.set_label("Desactivada");
                } else {
                    lbl.set_label(&cam_n.to_string());
                }
            }
            if let Some(ref lbl) = mic_val_lbl {
                if read_gsetting_bool("org.gnome.desktop.privacy", "disable-microphone", false) {
                    lbl.set_label("Desactivado");
                } else {
                    lbl.set_label(&mic_n.to_string());
                }
            }
            if let Some(ref lbl) = net_val_lbl {
                lbl.set_label(&net_n.to_string());
            }
            if let Some(ref lbl) = fv_val_lbl {
                let active = !luks_c.borrow().is_empty();
                lbl.set_label(if active { "Activado" } else { "Desactivado" });
            }

            sync_c.set(true);
            allow_dd.set_selected((cfg_c.borrow().allow_apps_from as u32).min(2));
            acc_dd.set_selected(read_usb_accessories_index());
            sync_c.set(false);
        })
    };

    (scrolled, refresh)
}

fn build_location_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();
    let syncing = Rc::new(Cell::new(false));

    let banner = make_banner_card(
        "find-location-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Localización",
        "Permite que el sistema y las aplicaciones determinen tu ubicación aproximada o exacta mediante GeoClue.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let enable_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    enable_row.append(
        &Label::builder()
            .label("Servicios de localización")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let loc_sw = Switch::builder()
        .valign(Align::Center)
        .active(read_gsetting_bool("org.gnome.system.location", "enabled", false))
        .build();
    enable_row.append(&*loc_sw);

    let acc_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    acc_row.append(
        &Label::builder()
            .label("Precisión máxima")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let levels = ["Exacta", "Calle", "Vecindario", "Ciudad", "País"];
    let level_keys = ["exact", "street", "neighborhood", "city", "country"];
    let acc_model = StringList::new(&levels);
    let acc_dd = DropDown::builder()
        .model(&acc_model)
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    acc_row.append(&acc_dd);

    card.append(&enable_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&acc_row);
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    {
        let sync_c = syncing.clone();
        loc_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                write_gsetting_bool("org.gnome.system.location", "enabled", active);
            }
            glib::Propagation::Proceed
        });
    }
    {
        let sync_c = syncing.clone();
        acc_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            if let Some(key) = level_keys.get(dd.selected() as usize) {
                write_gsetting_string("org.gnome.system.location", "max-accuracy-level", key);
            }
        });
    }

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        syncing.set(true);
        loc_sw.set_active(read_gsetting_bool(
            "org.gnome.system.location",
            "enabled",
            false,
        ));
        let cur = read_gsetting_string("org.gnome.system.location", "max-accuracy-level", "exact");
        let idx = level_keys
            .iter()
            .position(|k| *k == cur.as_str())
            .unwrap_or(0) as u32;
        acc_dd.set_selected(idx);
        syncing.set(false);
    });

    (scrolled, refresh)
}

#[allow(clippy::too_many_arguments)]
fn build_flatpak_perm_pane(
    icon_name: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    subtitle: &str,
    kind: FlatpakPermKind,
    global_toggle: Option<(&'static str, &'static str, &'static str)>,
    flatpaks: &Rc<RefCell<Vec<FlatpakAppPerm>>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();
    let syncing = Rc::new(Cell::new(false));

    let banner = make_banner_card(icon_name, top, bot, title, subtitle);
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    let global_sw_slot: Option<(Switch, &'static str, &'static str)> =
        global_toggle.map(|(label, schema, key)| {
            let gcard = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(0)
                .build();
            let row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-pref-row"])
                .build();
            row.append(
                &Label::builder()
                    .label(label)
                    .css_classes(["wifi-row-title"])
                    .hexpand(true)
                    .xalign(0.0)
                    .build(),
            );
            let allowed = !read_gsetting_bool(schema, key, false);
            let sw = Switch::builder()
                .valign(Align::Center)
                .active(allowed)
                .build();
            let sync_c = syncing.clone();
            sw.connect_state_set(move |_, active| {
                if !sync_c.get() {
                    write_gsetting_bool(schema, key, !active);
                }
                glib::Propagation::Proceed
            });
            row.append(&*sw);
            gcard.append(&row);
            pane.append(&wrap_squircle_card(&gcard, WIFI_CARD_SQUIRCLE_RADIUS));
            (sw, schema, key)
        });

    pane.append(
        &Label::builder()
            .label("Aplicaciones Flatpak")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .margin_top(2)
            .build(),
    );

    let list_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    pane.append(&wrap_squircle_card(&list_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let fp_c = flatpaks.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        if let Some((ref sw, schema, key)) = global_sw_slot {
            syncing.set(true);
            sw.set_active(!read_gsetting_bool(schema, key, false));
            syncing.set(false);
        }

        while let Some(child) = list_card.first_child() {
            list_card.remove(&child);
        }

        let apps = fp_c.borrow();
        if apps.is_empty() {
            let empty = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .css_classes(["wifi-pref-row"])
                .build();
            empty.append(
                &Label::builder()
                    .label("No se detectaron aplicaciones Flatpak instaladas.")
                    .css_classes(["card-section-sub"])
                    .xalign(0.0)
                    .build(),
            );
            list_card.append(&empty);
            return;
        }

        for (idx, app) in apps.iter().enumerate() {
            if idx > 0 {
                list_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
            }
            let row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(10)
                .css_classes(["wifi-pref-row"])
                .build();
            let col = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .hexpand(true)
                .valign(Align::Center)
                .build();
            col.append(
                &Label::builder()
                    .label(&app.name)
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .build(),
            );
            col.append(
                &Label::builder()
                    .label(&app.app_id)
                    .css_classes(["card-section-sub"])
                    .xalign(0.0)
                    .build(),
            );
            let sw = Switch::builder()
                .valign(Align::Center)
                .active(kind.has(app))
                .build();
            let app_id = app.app_id.clone();
            sw.connect_state_set(move |_, active| {
                let flag = kind.override_flag(active);
                let _ = Command::new("flatpak")
                    .args(["override", "--user", flag, &app_id])
                    .spawn();
                glib::Propagation::Proceed
            });
            row.append(&col);
            row.append(&*sw);
            list_card.append(&row);
        }
    });

    (scrolled, refresh)
}

fn build_analytics_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();
    let syncing = Rc::new(Cell::new(false));

    let banner = make_banner_card(
        "utilities-system-monitor-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Análisis y mejoras",
        "Controla el envío de informes de diagnóstico, estadísticas de software e historial local de actividad.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let items: [(&str, &str, &str); 6] = [
        (
            "Enviar informes de problemas técnicos",
            "Permite enviar diagnósticos automáticos cuando ocurre un fallo.",
            "report-technical-problems",
        ),
        (
            "Enviar estadísticas de uso de software",
            "Comparte métricas anónimas de uso de aplicaciones instaladas.",
            "send-software-usage-stats",
        ),
        (
            "Recordar historial de uso de apps",
            "Registra qué aplicaciones usas con más frecuencia para ordenar resultados.",
            "remember-app-usage",
        ),
        (
            "Recordar archivos recientes",
            "Muestra documentos y archivos abiertos recientemente en las apps y diálogos.",
            "remember-recent-files",
        ),
        (
            "Eliminar archivos temporales antiguos automáticamente",
            "Limpia archivos temporales expirados en segundo plano.",
            "remove-old-temp-files",
        ),
        (
            "Vaciar papelera antigua automáticamente",
            "Elimina elementos que llevan demasiado tiempo en la papelera.",
            "remove-old-trash-files",
        ),
    ];

    let mut switches: Vec<(Switch, &'static str)> = Vec::new();
    for (idx, (title, sub, key)) in items.into_iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
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
        col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        let sw = Switch::builder()
            .valign(Align::Center)
            .active(read_gsetting_bool("org.gnome.desktop.privacy", key, false))
            .build();
        let sync_c = syncing.clone();
        sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                write_gsetting_bool("org.gnome.desktop.privacy", key, active);
            }
            glib::Propagation::Proceed
        });
        row.append(&col);
        row.append(&*sw);
        card.append(&row);
        switches.push((sw, key));
    }

    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        syncing.set(true);
        for (sw, key) in &switches {
            sw.set_active(read_gsetting_bool("org.gnome.desktop.privacy", key, false));
        }
        syncing.set(false);
    });

    (scrolled, refresh)
}

fn build_filevault_pane(
    luks_vols: &Rc<RefCell<Vec<LuksVolumeInfo>>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let banner = make_banner_card(
        "drive-harddisk-symbolic",
        GRAY_TOP,
        GRAY_BOT,
        "FileVault (Cifrado de disco LUKS)",
        "Protege los datos almacenados en tu disco mediante cifrado completo de volumen LUKS / dm-crypt.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    let status_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    pane.append(&wrap_squircle_card(&status_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let luks_c = luks_vols.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        while let Some(child) = status_card.first_child() {
            status_card.remove(&child);
        }
        let vols = luks_c.borrow();

        let summary_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        summary_row.append(
            &Label::builder()
                .label("Estado del cifrado de disco")
                .css_classes(["wifi-row-title"])
                .hexpand(true)
                .xalign(0.0)
                .build(),
        );
        summary_row.append(
            &Label::builder()
                .label(if vols.is_empty() {
                    "Desactivado"
                } else {
                    "Activado (LUKS)"
                })
                .css_classes(["card-section-sub"])
                .build(),
        );
        status_card.append(&summary_row);

        if vols.is_empty() {
            status_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
            let note = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .css_classes(["wifi-pref-row"])
                .build();
            note.append(
                &Label::builder()
                    .label("No se detectaron volúmenes activos cifrados con LUKS (dm-crypt) en este equipo.")
                    .css_classes(["card-section-sub"])
                    .wrap(true)
                    .xalign(0.0)
                    .build(),
            );
            status_card.append(&note);
        } else {
            for v in vols.iter() {
                status_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .css_classes(["wifi-pref-row"])
                    .build();
                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .build();
                col.append(
                    &Label::builder()
                        .label(&v.name)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .build(),
                );
                col.append(
                    &Label::builder()
                        .label(&format!("Punto de montaje: {} · Tamaño: {}", v.mountpoint, v.size))
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .build(),
                );
                row.append(&col);
                row.append(
                    &Label::builder()
                        .label("Cifrado")
                        .css_classes(["card-section-sub"])
                        .valign(Align::Center)
                        .build(),
                );
                status_card.append(&row);
            }
        }
    });

    (scrolled, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_subpanels_and_parsers_work() {
        assert!(is_privacy_subpanel(PRIVACY_PANEL_ID));
        assert!(is_privacy_subpanel(PRIVACY_FILEVAULT_SUBPANEL_ID));
        assert!(!is_privacy_subpanel("wifi"));
        assert_eq!(
            privacy_subpanel_title(PRIVACY_PANEL_ID),
            Some("Privacidad y seguridad")
        );
        assert_eq!(privacy_subpanel_title(PRIVACY_CAMERA_SUBPANEL_ID), Some("Cámara"));
        assert_eq!(privacy_subpanel_title("other"), None);

        let raw_fp = "\
[Context]
shared=network;ipc;
sockets=wayland;pulseaudio;
devices=all;
features=bluetooth;
filesystems=host;home;xdg-download;
";
        let perm = parse_flatpak_permissions("org.demo.App", "Demo", raw_fp);
        assert!(perm.full_disk);
        assert!(perm.home_files);
        assert!(perm.bluetooth);
        assert!(perm.camera);
        assert!(perm.microphone);
        assert!(perm.network);

        let raw_lsblk = "\
nvme0n1 disk 476.9G
nvme0n1p3 part 475G
luks-abc123 crypt 475G /var
";
        let vols = parse_lsblk_luks(raw_lsblk);
        assert_eq!(vols.len(), 1);
        assert_eq!(vols[0].name, "luks-abc123");
        assert_eq!(vols[0].mountpoint, "/var");
    }
}
