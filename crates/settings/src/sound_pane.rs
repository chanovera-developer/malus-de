use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, CheckButton, Image, Label, MenuButton, Orientation, PolicyType,
    Popover, PositionType, Scale, ScrolledWindow,
};
use malus_de_common::config::SoundConfig;
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, wrap_squircle_card,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const SOUND_PANEL_ID: &str = "sound";
pub const SOUND_PANEL_TITLE: &str = "Sonido";

const ALERT_SOUNDS: [(&str, &str); 9] = [
    ("Boop", "bell.oga"),
    ("Breeze", "message.oga"),
    ("Bubble", "message-new-instant.oga"),
    ("Crystal", "complete.oga"),
    ("Funk", "dialog-warning.oga"),
    ("Glass", "dialog-information.oga"),
    ("Hero", "device-added.oga"),
    ("Pop", "audio-volume-change.oga"),
    ("Sosumi", "alarm-clock-elapsed.oga"),
];

const DEFAULT_EFFECTS_DEVICE_LABEL: &str = "Dispositivo de salida de sonido elegido";

#[derive(Debug, Clone, PartialEq)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub description: String,
    pub device_type: String,
    pub is_default: bool,
    pub volume_pct: f64,
    pub muted: bool,
    /// Balance estéreo `[-1.0 ..= 1.0]` (`-1.0` izquierda, `0.0` centro, `+1.0` derecha).
    pub balance: f64,
}

#[derive(Debug, Clone, Default)]
struct SoundSystemSnapshot {
    outputs: Vec<AudioDeviceInfo>,
    inputs: Vec<AudioDeviceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IoTab {
    Output,
    Input,
}

/// Calcula los porcentajes de canal izquierdo y derecho a partir de un volumen maestro y balance `[-1, 1]`.
pub fn channel_volumes_for_balance(master_pct: f64, balance: f64) -> (u32, u32) {
    let m = master_pct.clamp(0.0, 100.0);
    let b = balance.clamp(-1.0, 1.0);
    let (left, right) = if b < 0.0 {
        (m, m * (1.0 + b))
    } else if b > 0.0 {
        (m * (1.0 - b), m)
    } else {
        (m, m)
    };
    (left.round() as u32, right.round() as u32)
}

/// Deduce el volumen maestro y el balance estéreo `[-1.0 ..= 1.0]` a partir de los canales L/R.
pub fn master_and_balance_from_channels(left_pct: f64, right_pct: f64) -> (f64, f64) {
    let l = left_pct.clamp(0.0, 150.0);
    let r = right_pct.clamp(0.0, 150.0);
    let max_v = l.max(r);
    if max_v <= 0.5 {
        return (0.0, 0.0);
    }
    let balance = if (l - r).abs() < 0.5 {
        0.0
    } else if l > r {
        -((l - r) / l).clamp(0.0, 1.0)
    } else {
        ((r - l) / r).clamp(0.0, 1.0)
    };
    (max_v.clamp(0.0, 100.0), balance)
}

pub fn infer_audio_device_type(
    name: &str,
    description: &str,
    bus: &str,
    active_port: &str,
    form_factor: &str,
) -> String {
    let combined = format!(
        "{} {} {} {} {}",
        name.to_ascii_lowercase(),
        description.to_ascii_lowercase(),
        bus.to_ascii_lowercase(),
        active_port.to_ascii_lowercase(),
        form_factor.to_ascii_lowercase()
    );
    if combined.contains("displayport") || combined.contains("-dp") || combined.contains("dp-") {
        "DisplayPort".to_string()
    } else if combined.contains("hdmi") {
        "HDMI".to_string()
    } else if combined.contains("bluetooth") || combined.contains("bluez") {
        "Bluetooth".to_string()
    } else if combined.contains("usb") {
        "USB".to_string()
    } else if combined.contains("raop") || combined.contains("airplay") {
        "AirPlay".to_string()
    } else if combined.contains("null")
        || combined.contains("dummy")
        || combined.contains("virtual")
    {
        "Virtual".to_string()
    } else if combined.contains("pci")
        || combined.contains("internal")
        || combined.contains("analog")
        || combined.contains("speaker")
        || combined.contains("headphone")
        || combined.contains("mic")
        || combined.contains("hda")
    {
        "Integrado".to_string()
    } else {
        "Audio".to_string()
    }
}

fn parse_percent_str(raw: &str) -> Option<f64> {
    raw.trim().trim_end_matches('%').trim().parse::<f64>().ok()
}

fn is_connected_real_device(
    item: &serde_json::Value,
    name: &str,
    description: &str,
    is_default: bool,
) -> bool {
    let name_low = name.to_ascii_lowercase();
    let desc_low = description.to_ascii_lowercase();
    let props = item.get("properties");
    let str_prop = |k: &str| {
        props
            .and_then(|p| p.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };

    if name_low.contains("auto_null")
        || name_low.contains("null-sink")
        || name_low.contains("null_sink")
        || name_low.contains("dummy")
        || name_low.starts_with("easyeffects_")
        || name_low.starts_with("effect_input.")
        || name_low.starts_with("effect_output.")
        || name_low.starts_with("loopback")
        || name_low.starts_with("raop_sink")
        || name_low.contains(".raop.")
        || desc_low.contains("dummy output")
        || desc_low.contains("salida ficticia")
    {
        return false;
    }
    if str_prop("node.virtual") == "true" || str_prop("device.api") == "raop" {
        return false;
    }
    let factory = str_prop("factory.name");
    if factory.contains("null") || factory.contains("raop") {
        return false;
    }
    let dev_class = str_prop("device.class");
    if dev_class == "abstract" || dev_class == "virtual" {
        return false;
    }

    let active_port = item
        .get("active_port")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let mut active_availability: Option<String> = None;
    if let Some(ports) = item.get("ports").and_then(|v| v.as_array()) {
        if !ports.is_empty() {
            if !active_port.is_empty() {
                if let Some(port) = ports
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(active_port))
                {
                    let avail = port
                        .get("availability")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if avail == "not available" {
                        return false;
                    }
                    active_availability = Some(avail);
                }
            } else {
                let all_unavailable = ports.iter().all(|p| {
                    p.get("availability")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .eq_ignore_ascii_case("not available")
                });
                if all_unavailable {
                    return false;
                }
            }
        }
    }

    if !is_default
        && (name_low.contains("iec958")
            || name_low.contains("spdif")
            || active_port.to_ascii_lowercase().contains("iec958"))
        && active_availability.as_deref() != Some("available")
    {
        return false;
    }
    true
}

fn parse_pactl_devices_json(
    raw_json: &str,
    default_name: &str,
    skip_monitors: bool,
) -> Vec<AudioDeviceInfo> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw_json)
    else {
        return Vec::new();
    };

    let mut devices = Vec::new();
    for item in items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        if skip_monitors && name.ends_with(".monitor") {
            continue;
        }
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&name)
            .to_string();
        let is_default = !default_name.is_empty() && name == default_name;
        if !is_connected_real_device(&item, &name, &description, is_default) {
            continue;
        }
        let props = item.get("properties");
        let bus = props
            .and_then(|p| p.get("device.bus"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let form_factor = props
            .and_then(|p| p.get("device.form_factor"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let active_port = item
            .get("active_port")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let muted = item.get("mute").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut left_pct: Option<f64> = None;
        let mut right_pct: Option<f64> = None;
        let mut any_pct: Option<f64> = None;
        if let Some(vol_map) = item.get("volume").and_then(|v| v.as_object()) {
            for (chan, entry) in vol_map {
                let pct = entry
                    .get("value_percent")
                    .and_then(|v| v.as_str())
                    .and_then(parse_percent_str);
                if let Some(p) = pct {
                    if any_pct.is_none() {
                        any_pct = Some(p);
                    }
                    let c_low = chan.to_ascii_lowercase();
                    if c_low.contains("left") && left_pct.is_none() {
                        left_pct = Some(p);
                    } else if c_low.contains("right") && right_pct.is_none() {
                        right_pct = Some(p);
                    }
                }
            }
        }
        let (volume_pct, balance) = match (left_pct, right_pct) {
            (Some(l), Some(r)) => master_and_balance_from_channels(l, r),
            _ => (any_pct.unwrap_or(75.0).clamp(0.0, 100.0), 0.0),
        };

        devices.push(AudioDeviceInfo {
            is_default: !default_name.is_empty() && name == default_name,
            device_type: infer_audio_device_type(&name, &description, bus, active_port, form_factor),
            name,
            description,
            volume_pct,
            muted,
            balance,
        });
    }

    if !devices.is_empty() && !devices.iter().any(|d| d.is_default) {
        devices[0].is_default = true;
    }
    devices
}

fn query_wpctl_fallback(target: &str, fallback_desc: &str) -> Option<AudioDeviceInfo> {
    let out = Command::new("wpctl")
        .args(["get-volume", target])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let muted = s.contains("[MUTED]");
    let mut vol = 75.0;
    for tok in s.split_whitespace() {
        if let Ok(v) = tok.parse::<f64>() {
            vol = (v * 100.0).clamp(0.0, 100.0);
            break;
        }
    }
    let mut description = fallback_desc.to_string();
    if let Ok(insp) = Command::new("wpctl").args(["inspect", target]).output() {
        for line in String::from_utf8_lossy(&insp.stdout).lines() {
            let t = line.trim();
            if t.contains("node.description =") {
                if let Some((_, val)) = t.split_once('=') {
                    let clean = val.trim().trim_matches('"');
                    if !clean.is_empty() {
                        description = clean.to_string();
                        break;
                    }
                }
            }
        }
    }
    Some(AudioDeviceInfo {
        name: target.to_string(),
        device_type: infer_audio_device_type(target, &description, "", "", ""),
        description,
        is_default: true,
        volume_pct: vol,
        muted,
        balance: 0.0,
    })
}

fn query_sound_system_snapshot() -> SoundSystemSnapshot {
    let default_sink = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let default_source = Command::new("pactl")
        .arg("get-default-source")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let mut outputs = Command::new("pactl")
        .args(["--format=json", "list", "sinks"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let raw = String::from_utf8_lossy(&o.stdout);
            parse_pactl_devices_json(&raw, &default_sink, false)
        })
        .unwrap_or_default();

    if outputs.is_empty() {
        if let Some(fb) = query_wpctl_fallback("@DEFAULT_AUDIO_SINK@", "Altavoces integrados") {
            outputs.push(fb);
        }
    }

    let mut inputs = Command::new("pactl")
        .args(["--format=json", "list", "sources"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let raw = String::from_utf8_lossy(&o.stdout);
            parse_pactl_devices_json(&raw, &default_source, true)
        })
        .unwrap_or_default();

    if inputs.is_empty() {
        if let Some(fb) = query_wpctl_fallback("@DEFAULT_AUDIO_SOURCE@", "Micrófono integrado") {
            inputs.push(fb);
        }
    }

    SoundSystemSnapshot { outputs, inputs }
}

fn apply_sink_volume_and_balance(sink_name: String, master_pct: f64, balance: f64) {
    let (left, right) = channel_volumes_for_balance(master_pct, balance);
    std::thread::spawn(move || {
        let target = if sink_name.is_empty() || sink_name.starts_with('@') {
            "@DEFAULT_SINK@".to_string()
        } else {
            sink_name
        };
        let status = Command::new("pactl")
            .args([
                "set-sink-volume",
                &target,
                &format!("{}%", left),
                &format!("{}%", right),
            ])
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            let _ = Command::new("wpctl")
                .args([
                    "set-volume",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{}%", master_pct.round() as u32),
                ])
                .status();
        }
    });
}

fn apply_sink_mute(sink_name: String, muted: bool) {
    std::thread::spawn(move || {
        let target = if sink_name.is_empty() || sink_name.starts_with('@') {
            "@DEFAULT_SINK@".to_string()
        } else {
            sink_name
        };
        let flag = if muted { "1" } else { "0" };
        let status = Command::new("pactl")
            .args(["set-sink-mute", &target, flag])
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", flag])
                .status();
        }
    });
}

fn apply_source_volume(source_name: String, pct: f64) {
    let pct_u = pct.clamp(0.0, 100.0).round() as u32;
    std::thread::spawn(move || {
        let target = if source_name.is_empty() || source_name.starts_with('@') {
            "@DEFAULT_SOURCE@".to_string()
        } else {
            source_name
        };
        let status = Command::new("pactl")
            .args(["set-source-volume", &target, &format!("{}%", pct_u)])
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            let _ = Command::new("wpctl")
                .args(["set-volume", "@DEFAULT_AUDIO_SOURCE@", &format!("{}%", pct_u)])
                .status();
        }
    });
}

fn apply_source_mute(source_name: String, muted: bool) {
    std::thread::spawn(move || {
        let target = if source_name.is_empty() || source_name.starts_with('@') {
            "@DEFAULT_SOURCE@".to_string()
        } else {
            source_name
        };
        let flag = if muted { "1" } else { "0" };
        let status = Command::new("pactl")
            .args(["set-source-mute", &target, flag])
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", flag])
                .status();
        }
    });
}

fn resolve_alert_sound_file(alert_name: &str) -> Option<PathBuf> {
    let file_name = ALERT_SOUNDS
        .iter()
        .find(|(label, _)| label.eq_ignore_ascii_case(alert_name))
        .map(|(_, f)| *f)
        .unwrap_or("bell.oga");

    let base_dirs = [
        "/usr/share/sounds/freedesktop/stereo",
        "/usr/share/sounds/gnome/default/alerts",
    ];
    for dir in base_dirs {
        let candidate = PathBuf::from(dir).join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let fallback = PathBuf::from("/usr/share/sounds/freedesktop/stereo/bell.oga");
    fallback.is_file().then_some(fallback)
}

fn play_alert_preview(alert_name: String, volume_pct: f64, effects_device: String) {
    let Some(path) = resolve_alert_sound_file(&alert_name) else {
        if let Some(display) = gtk4::gdk::Display::default() {
            display.beep();
        }
        return;
    };
    let clamped_pct = volume_pct.clamp(0.0, 100.0);
    std::thread::spawn(move || {
        let pa_vol = ((clamped_pct / 100.0) * 65536.0).round() as u32;
        let mut cmd = Command::new("paplay");
        cmd.arg(format!("--volume={}", pa_vol));
        if !effects_device.trim().is_empty() {
            cmd.arg(format!("--device={}", effects_device));
        }
        cmd.arg(&path);
        if matches!(cmd.status(), Ok(s) if s.success()) {
            return;
        }
        let mut pw = Command::new("pw-play");
        pw.arg(format!("--volume={:.2}", clamped_pct / 100.0));
        if !effects_device.trim().is_empty() {
            pw.arg("--target").arg(&effects_device);
        }
        pw.arg(&path);
        let _ = pw.status();
    });
}

/// Construye el panel «Sonido» estilo macOS Tahoe y devuelve `(widget, refresher)`.
pub fn create_sound_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
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

    let cfg = Rc::new(RefCell::new(SoundConfig::load()));
    let snapshot = Rc::new(RefCell::new(SoundSystemSnapshot::default()));
    let active_tab = Rc::new(Cell::new(IoTab::Output));
    let syncing_ui = Rc::new(Cell::new(false));

    // =========================================================================
    // SECCIÓN 1: Efectos de sonido
    // =========================================================================
    pane.append(
        &Label::builder()
            .label("Efectos de sonido")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_start(2)
            .build(),
    );

    let effects_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    // 1.1 Sonido de alerta (dropdown + botón circular ▶)
    let alert_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();
    alert_row.append(
        &Label::builder()
            .label("Sonido de alerta")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let alert_lbl = Label::builder()
        .label(&cfg.borrow().alert_sound)
        .css_classes(["wifi-row-title"])
        .valign(Align::Center)
        .build();
    let alert_menu = build_alert_sound_popover(&alert_lbl, &cfg);
    alert_row.append(&alert_menu);

    let play_btn = Button::builder()
        .css_classes(["sound-play-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Escuchar sonido de alerta")
        .build();
    play_btn.set_child(Some(
        &Image::builder()
            .icon_name("media-playback-start-symbolic")
            .pixel_size(11)
            .build(),
    ));
    {
        let c = cfg.clone();
        play_btn.connect_clicked(move |_| {
            let guard = c.borrow();
            play_alert_preview(
                guard.alert_sound.clone(),
                guard.alert_volume,
                guard.effects_device.clone(),
            );
        });
    }
    alert_row.append(&play_btn);
    effects_card.append(&alert_row);
    effects_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 1.2 Reproducir efectos a través de
    let effects_dev_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    effects_dev_row.append(
        &Label::builder()
            .label("Reproducir efectos a través de")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let effects_dev_lbl = Label::builder()
        .label(DEFAULT_EFFECTS_DEVICE_LABEL)
        .css_classes(["wifi-row-title"])
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(34)
        .valign(Align::Center)
        .build();
    let effects_dev_pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();
    let effects_dev_popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .css_classes(["wifi-menu-popover"])
        .child(&effects_dev_pop_box)
        .build();
    let effects_dev_menu_btn = build_inline_menu_button(&effects_dev_lbl, &effects_dev_popover);
    effects_dev_row.append(&effects_dev_menu_btn);
    effects_card.append(&effects_dev_row);
    effects_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 1.3 Volumen de alerta
    let alert_vol_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    alert_vol_row.append(
        &Label::builder()
            .label("Volumen de alerta")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let alert_slider_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    alert_slider_box.append(
        &Image::builder()
            .icon_name("audio-volume-low-symbolic")
            .pixel_size(14)
            .css_classes(["sound-vol-icon"])
            .build(),
    );
    let alert_vol_slider = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    alert_vol_slider.set_draw_value(false);
    alert_vol_slider.set_width_request(236);
    alert_vol_slider.set_value(cfg.borrow().alert_volume);
    alert_vol_slider.set_css_classes(&["bat-limit-slider"]);
    {
        let c = cfg.clone();
        alert_vol_slider.connect_value_changed(move |s| {
            let val = s.value().clamp(0.0, 100.0);
            c.borrow_mut().alert_volume = val;
            let _ = c.borrow().save();
        });
    }
    alert_slider_box.append(&alert_vol_slider);
    alert_slider_box.append(
        &Image::builder()
            .icon_name("audio-volume-high-symbolic")
            .pixel_size(14)
            .css_classes(["sound-vol-icon"])
            .build(),
    );
    alert_vol_row.append(&alert_slider_box);
    effects_card.append(&alert_vol_row);
    effects_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 1.4 Reproducir sonido durante el arranque (Por implementar)
    effects_card.append(&make_sound_pending_switch_row(
        "Reproducir sonido durante el arranque",
        cfg.borrow().play_startup_sound,
        {
            let c = cfg.clone();
            move |active| {
                c.borrow_mut().play_startup_sound = active;
                let _ = c.borrow().save();
            }
        },
    ));
    effects_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 1.5 Reproducir los sonidos de la interfaz de usuario (Por implementar)
    effects_card.append(&make_sound_pending_switch_row(
        "Reproducir los sonidos de la interfaz de usuario",
        cfg.borrow().play_ui_sound_effects,
        {
            let c = cfg.clone();
            move |active| {
                c.borrow_mut().play_ui_sound_effects = active;
                let _ = c.borrow().save();
            }
        },
    ));
    effects_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 1.6 Emitir sonido al cambiar el volumen (Por implementar)
    effects_card.append(&make_sound_pending_switch_row(
        "Emitir sonido al cambiar el volumen",
        cfg.borrow().volume_change_feedback,
        {
            let c = cfg.clone();
            move |active| {
                c.borrow_mut().volume_change_feedback = active;
                let _ = c.borrow().save();
            }
        },
    ));

    pane.append(&wrap_squircle_card(
        &effects_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // =========================================================================
    // SECCIÓN 2: Entrada y salida
    // =========================================================================
    pane.append(
        &Label::builder()
            .label("Entrada y salida")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .margin_start(2)
            .build(),
    );

    // Control segmentado Salida | Entrada
    let seg_track = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .homogeneous(true)
        .spacing(2)
        .hexpand(true)
        .css_classes(["bat-segmented-track"])
        .build();
    let btn_output_tab = Button::builder()
        .label("Salida")
        .css_classes(["bat-seg-tab", "bat-seg-tab-active"])
        .has_frame(false)
        .hexpand(true)
        .build();
    let btn_input_tab = Button::builder()
        .label("Entrada")
        .css_classes(["bat-seg-tab"])
        .has_frame(false)
        .hexpand(true)
        .build();
    seg_track.append(&btn_output_tab);
    seg_track.append(&btn_input_tab);
    pane.append(&seg_track);

    // Tabla de dispositivos (Nombre | Tipo)
    let devices_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let table_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .css_classes(["sound-table-header"])
        .build();
    table_hdr.append(
        &Label::builder()
            .label("Nombre")
            .css_classes(["sound-table-header-lbl"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    table_hdr.append(
        &Label::builder()
            .label("Tipo")
            .css_classes(["sound-table-header-lbl"])
            .xalign(0.0)
            .width_request(140)
            .build(),
    );
    devices_card.append(&table_hdr);
    devices_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let devices_list_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["sound-devices-list"])
        .build();
    devices_card.append(&devices_list_box);
    pane.append(&wrap_squircle_card(
        &devices_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Tarjeta de controles de Salida (Volumen de salida + Silencio + Balance)
    let output_controls_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let out_vol_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    out_vol_row.append(
        &Label::builder()
            .label("Volumen de salida")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Start)
            .margin_top(4)
            .build(),
    );
    let out_vol_right = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let out_slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    out_slider_row.append(
        &Image::builder()
            .icon_name("audio-volume-low-symbolic")
            .pixel_size(14)
            .css_classes(["sound-vol-icon"])
            .build(),
    );
    let out_vol_slider = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    out_vol_slider.set_draw_value(false);
    out_vol_slider.set_width_request(236);
    out_vol_slider.set_value(75.0);
    out_vol_slider.set_css_classes(&["bat-limit-slider"]);
    out_slider_row.append(&out_vol_slider);
    out_slider_row.append(
        &Image::builder()
            .icon_name("audio-volume-high-symbolic")
            .pixel_size(14)
            .css_classes(["sound-vol-icon"])
            .build(),
    );
    out_vol_right.append(&out_slider_row);

    let out_mute_check = CheckButton::builder()
        .label("Silencio")
        .css_classes(["sound-mute-check"])
        .halign(Align::End)
        .build();
    out_vol_right.append(&out_mute_check);
    out_vol_row.append(&out_vol_right);
    output_controls_card.append(&out_vol_row);
    output_controls_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Fila Balance
    let balance_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    balance_row.append(
        &Label::builder()
            .label("Balance")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Start)
            .margin_top(4)
            .build(),
    );
    let balance_right = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .width_request(276)
        .valign(Align::Center)
        .build();
    let balance_slider = Scale::with_range(Orientation::Horizontal, -1.0, 1.0, 0.02);
    balance_slider.set_draw_value(false);
    balance_slider.set_value(0.0);
    balance_slider.add_mark(0.0, PositionType::Bottom, None);
    balance_slider.set_css_classes(&["sound-balance-slider"]);
    balance_right.append(&balance_slider);

    let balance_labels = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .build();
    balance_labels.append(
        &Label::builder()
            .label("Izquierda")
            .css_classes(["sound-balance-caption"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    balance_labels.append(
        &Label::builder()
            .label("Derecha")
            .css_classes(["sound-balance-caption"])
            .xalign(1.0)
            .build(),
    );
    balance_right.append(&balance_labels);
    balance_row.append(&balance_right);
    output_controls_card.append(&balance_row);

    let output_controls_wrap =
        wrap_squircle_card(&output_controls_card, WIFI_CARD_SQUIRCLE_RADIUS);
    pane.append(&output_controls_wrap);

    // Tarjeta de controles de Entrada (Volumen de entrada + Silencio + Nivel de entrada)
    let input_controls_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let in_vol_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    in_vol_row.append(
        &Label::builder()
            .label("Volumen de entrada")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Start)
            .margin_top(4)
            .build(),
    );
    let in_vol_right = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let in_slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    in_slider_row.append(
        &Image::builder()
            .icon_name("microphone-sensitivity-low-symbolic")
            .pixel_size(14)
            .css_classes(["sound-vol-icon"])
            .build(),
    );
    let in_vol_slider = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    in_vol_slider.set_draw_value(false);
    in_vol_slider.set_width_request(236);
    in_vol_slider.set_value(75.0);
    in_vol_slider.set_css_classes(&["bat-limit-slider"]);
    in_slider_row.append(&in_vol_slider);
    in_slider_row.append(
        &Image::builder()
            .icon_name("microphone-sensitivity-high-symbolic")
            .pixel_size(14)
            .css_classes(["sound-vol-icon"])
            .build(),
    );
    in_vol_right.append(&in_slider_row);

    let in_mute_check = CheckButton::builder()
        .label("Silencio")
        .css_classes(["sound-mute-check"])
        .halign(Align::End)
        .build();
    in_vol_right.append(&in_mute_check);
    in_vol_row.append(&in_vol_right);
    input_controls_card.append(&in_vol_row);
    input_controls_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Fila Nivel de entrada (Por implementar)
    let level_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let level_title_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    level_title_box.append(
        &Label::builder()
            .label("Nivel de entrada")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    level_title_box.append(&make_pending_badge());
    level_row.append(&level_title_box);

    let level_meter = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(5)
        .valign(Align::Center)
        .build();
    for _ in 0..15 {
        level_meter.append(&GtkBox::builder().css_classes(["sound-level-seg"]).build());
    }
    level_row.append(&level_meter);
    input_controls_card.append(&level_row);

    let input_controls_wrap = wrap_squircle_card(&input_controls_card, WIFI_CARD_SQUIRCLE_RADIUS);
    input_controls_wrap.set_visible(false);
    pane.append(&input_controls_wrap);

    // Botón de ayuda `?` inferior derecho
    let help_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Sonido");
    let help_pop = Popover::builder()
        .has_arrow(true)
        .autohide(true)
        .css_classes(["wifi-menu-popover"])
        .child(
            &Label::builder()
                .label("Administra los efectos de alerta y los dispositivos de entrada y salida de audio gestionados por PipeWire y WirePlumber.")
                .wrap(true)
                .max_width_chars(38)
                .xalign(0.0)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(10)
                .margin_end(10)
                .build(),
        )
        .build();
    help_btn.set_popover(Some(&help_pop));
    help_row.append(&help_wrap);
    pane.append(&help_row);

    // =========================================================================
    // CONEXIÓN DE CONTROLES INTERACTIVOS (PipeWire / PulseAudio)
    // =========================================================================
    {
        let snap_c = snapshot.clone();
        let sync_c = syncing_ui.clone();
        let bal_c = balance_slider.clone();
        out_vol_slider.connect_value_changed(move |s| {
            if sync_c.get() {
                return;
            }
            let vol = s.value().clamp(0.0, 100.0);
            let bal = bal_c.value().clamp(-1.0, 1.0);
            let sink_name = {
                let mut guard = snap_c.borrow_mut();
                if let Some(dev) = guard.outputs.iter_mut().find(|d| d.is_default) {
                    dev.volume_pct = vol;
                    dev.balance = bal;
                    dev.name.clone()
                } else {
                    String::new()
                }
            };
            apply_sink_volume_and_balance(sink_name, vol, bal);
        });
    }

    {
        let snap_c = snapshot.clone();
        let sync_c = syncing_ui.clone();
        let vol_c = out_vol_slider.clone();
        balance_slider.connect_value_changed(move |s| {
            if sync_c.get() {
                return;
            }
            let bal = if s.value().abs() < 0.04 {
                0.0
            } else {
                s.value().clamp(-1.0, 1.0)
            };
            let vol = vol_c.value().clamp(0.0, 100.0);
            let sink_name = {
                let mut guard = snap_c.borrow_mut();
                if let Some(dev) = guard.outputs.iter_mut().find(|d| d.is_default) {
                    dev.balance = bal;
                    dev.volume_pct = vol;
                    dev.name.clone()
                } else {
                    String::new()
                }
            };
            apply_sink_volume_and_balance(sink_name, vol, bal);
        });
    }

    {
        let snap_c = snapshot.clone();
        let sync_c = syncing_ui.clone();
        out_mute_check.connect_toggled(move |chk| {
            if sync_c.get() {
                return;
            }
            let muted = chk.is_active();
            let sink_name = {
                let mut guard = snap_c.borrow_mut();
                if let Some(dev) = guard.outputs.iter_mut().find(|d| d.is_default) {
                    dev.muted = muted;
                    dev.name.clone()
                } else {
                    String::new()
                }
            };
            apply_sink_mute(sink_name, muted);
        });
    }

    {
        let snap_c = snapshot.clone();
        let sync_c = syncing_ui.clone();
        in_vol_slider.connect_value_changed(move |s| {
            if sync_c.get() {
                return;
            }
            let vol = s.value().clamp(0.0, 100.0);
            let source_name = {
                let mut guard = snap_c.borrow_mut();
                if let Some(dev) = guard.inputs.iter_mut().find(|d| d.is_default) {
                    dev.volume_pct = vol;
                    dev.name.clone()
                } else {
                    String::new()
                }
            };
            apply_source_volume(source_name, vol);
        });
    }

    {
        let snap_c = snapshot.clone();
        let sync_c = syncing_ui.clone();
        in_mute_check.connect_toggled(move |chk| {
            if sync_c.get() {
                return;
            }
            let muted = chk.is_active();
            let source_name = {
                let mut guard = snap_c.borrow_mut();
                if let Some(dev) = guard.inputs.iter_mut().find(|d| d.is_default) {
                    dev.muted = muted;
                    dev.name.clone()
                } else {
                    String::new()
                }
            };
            apply_source_mute(source_name, muted);
        });
    }

    // Reconstrucción reactiva de tabla de dispositivos + controles + menú de efectos
    let refresh_trigger: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let render_ui: Rc<dyn Fn()> = {
        let snap_c = snapshot.clone();
        let tab_c = active_tab.clone();
        let sync_c = syncing_ui.clone();
        let cfg_c = cfg.clone();
        let list_box_c = devices_list_box.clone();
        let out_wrap_c = output_controls_wrap.clone();
        let in_wrap_c = input_controls_wrap.clone();
        let out_vol_c = out_vol_slider.clone();
        let out_mute_c = out_mute_check.clone();
        let bal_c = balance_slider.clone();
        let in_vol_c = in_vol_slider.clone();
        let in_mute_c = in_mute_check.clone();
        let eff_lbl_c = effects_dev_lbl.clone();
        let eff_pop_box_c = effects_dev_pop_box.clone();
        let eff_pop_c = effects_dev_popover.clone();
        let refresh_ref = refresh_trigger.clone();

        Rc::new(move || {
            let tab = tab_c.get();
            out_wrap_c.set_visible(tab == IoTab::Output);
            in_wrap_c.set_visible(tab == IoTab::Input);

            let snap = snap_c.borrow().clone();

            // 1. Actualizar menú "Reproducir efectos a través de"
            while let Some(child) = eff_pop_box_c.first_child() {
                eff_pop_box_c.remove(&child);
            }
            let selected_eff = cfg_c.borrow().effects_device.clone();
            let selected_eff_label = if selected_eff.is_empty() {
                DEFAULT_EFFECTS_DEVICE_LABEL.to_string()
            } else {
                snap.outputs
                    .iter()
                    .find(|d| d.name == selected_eff)
                    .map(|d| d.description.clone())
                    .unwrap_or_else(|| DEFAULT_EFFECTS_DEVICE_LABEL.to_string())
            };
            eff_lbl_c.set_label(&selected_eff_label);

            let mut eff_choices: Vec<(String, String)> =
                vec![(DEFAULT_EFFECTS_DEVICE_LABEL.to_string(), String::new())];
            for dev in &snap.outputs {
                eff_choices.push((dev.description.clone(), dev.name.clone()));
            }
            for (label, dev_name) in eff_choices {
                let is_sel = dev_name == selected_eff;
                let (item_wrap, item_btn) = make_popover_action_row(&label, is_sel, None);
                let cfg_i = cfg_c.clone();
                let lbl_i = eff_lbl_c.clone();
                let pop_i = eff_pop_c.clone();
                let label_c = label.clone();
                item_btn.connect_clicked(move |_| {
                    cfg_i.borrow_mut().effects_device = dev_name.clone();
                    let _ = cfg_i.borrow().save();
                    lbl_i.set_label(&label_c);
                    pop_i.popdown();
                });
                eff_pop_box_c.append(&item_wrap);
            }

            // 2. Reconstruir filas de dispositivos de la pestaña activa
            while let Some(child) = list_box_c.first_child() {
                list_box_c.remove(&child);
            }
            let devs = match tab {
                IoTab::Output => &snap.outputs,
                IoTab::Input => &snap.inputs,
            };
            if devs.is_empty() {
                let empty_msg = match tab {
                    IoTab::Output => "No se encontraron dispositivos de salida de audio.",
                    IoTab::Input => "No se encontraron dispositivos de entrada de audio.",
                };
                list_box_c.append(
                    &Label::builder()
                        .label(empty_msg)
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .margin_start(8)
                        .margin_top(6)
                        .margin_bottom(6)
                        .build(),
                );
            } else {
                for dev in devs {
                    let row_btn = Button::builder()
                        .css_classes(if dev.is_default {
                            ["sound-device-row", "sound-device-row-active"].as_slice()
                        } else {
                            ["sound-device-row"].as_slice()
                        })
                        .has_frame(false)
                        .hexpand(true)
                        .build();

                    let row_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(8)
                        .hexpand(true)
                        .build();
                    row_box.append(
                        &Label::builder()
                            .label(&dev.description)
                            .css_classes(["sound-device-name"])
                            .xalign(0.0)
                            .hexpand(true)
                            .ellipsize(gtk4::pango::EllipsizeMode::End)
                            .build(),
                    );
                    row_box.append(
                        &Label::builder()
                            .label(&dev.device_type)
                            .css_classes(["sound-device-type"])
                            .xalign(0.0)
                            .width_request(140)
                            .ellipsize(gtk4::pango::EllipsizeMode::End)
                            .build(),
                    );
                    row_btn.set_child(Some(&row_box));

                    let target_name = dev.name.clone();
                    let rf_holder = refresh_ref.clone();
                    row_btn.connect_clicked(move |_| {
                        let name_bg = target_name.clone();
                        let (tx, rx) = async_channel::bounded::<()>(1);
                        std::thread::spawn(move || {
                            let cmd_arg = match tab {
                                IoTab::Output => "set-default-sink",
                                IoTab::Input => "set-default-source",
                            };
                            let _ = Command::new("pactl").args([cmd_arg, &name_bg]).status();
                            let _ = tx.send_blocking(());
                        });
                        let rf_c = rf_holder.clone();
                        glib::MainContext::default().spawn_local(async move {
                            let _ = rx.recv().await;
                            if let Some(rf) = rf_c.borrow().as_ref() {
                                rf();
                            }
                        });
                    });
                    list_box_c.append(&row_btn);
                }
            }

            // 3. Sincronizar sliders y casillas con el dispositivo activo sin disparar señales
            sync_c.set(true);
            if let Some(active_out) = snap.outputs.iter().find(|d| d.is_default) {
                out_vol_c.set_value(active_out.volume_pct);
                out_mute_c.set_active(active_out.muted);
                bal_c.set_value(active_out.balance);
            }
            if let Some(active_in) = snap.inputs.iter().find(|d| d.is_default) {
                in_vol_c.set_value(active_in.volume_pct);
                in_mute_c.set_active(active_in.muted);
            }
            sync_c.set(false);
        })
    };

    // Cambio de pestaña Salida / Entrada
    {
        let tab_c = active_tab.clone();
        let btn_out = btn_output_tab.clone();
        let btn_in = btn_input_tab.clone();
        let render_c = render_ui.clone();
        btn_output_tab.connect_clicked(move |_| {
            if tab_c.get() == IoTab::Output {
                return;
            }
            tab_c.set(IoTab::Output);
            btn_out.add_css_class("bat-seg-tab-active");
            btn_in.remove_css_class("bat-seg-tab-active");
            render_c();
        });
    }
    {
        let tab_c = active_tab.clone();
        let btn_out = btn_output_tab.clone();
        let btn_in = btn_input_tab.clone();
        let render_c = render_ui.clone();
        btn_input_tab.connect_clicked(move |_| {
            if tab_c.get() == IoTab::Input {
                return;
            }
            tab_c.set(IoTab::Input);
            btn_in.add_css_class("bat-seg-tab-active");
            btn_out.remove_css_class("bat-seg-tab-active");
            render_c();
        });
    }

    // Refresco asíncrono sin bloquear el hilo GTK
    let refresher: Rc<dyn Fn()> = {
        let snap_c = snapshot.clone();
        let render_c = render_ui.clone();
        Rc::new(move || {
            let (tx, rx) = async_channel::bounded::<SoundSystemSnapshot>(1);
            std::thread::spawn(move || {
                let data = query_sound_system_snapshot();
                let _ = tx.send_blocking(data);
            });
            let snap_ui = snap_c.clone();
            let render_ui = render_c.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Ok(data) = rx.recv().await {
                    *snap_ui.borrow_mut() = data;
                    render_ui();
                }
            });
        })
    };
    *refresh_trigger.borrow_mut() = Some(refresher.clone());

    // Render inicial + sondeo asíncrono de dispositivos PipeWire
    render_ui();
    refresher();

    (scrolled, refresher)
}

fn build_inline_menu_button(value_label: &Label, popover: &Popover) -> MenuButton {
    let content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    content.append(value_label);

    let arrows_pill = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-chevron-pill"])
        .valign(Align::Center)
        .halign(Align::Center)
        .build();
    arrows_pill.append(
        &Image::builder()
            .icon_name("pan-up-symbolic")
            .pixel_size(11)
            .css_classes(["wifi-chevron-icon"])
            .build(),
    );
    content.append(&arrows_pill);

    MenuButton::builder()
        .child(&content)
        .popover(popover)
        .css_classes(["wifi-inline-dropdown"])
        .has_frame(false)
        .valign(Align::Center)
        .build()
}

fn build_alert_sound_popover(value_label: &Label, cfg: &Rc<RefCell<SoundConfig>>) -> MenuButton {
    let pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();
    let popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .css_classes(["wifi-menu-popover"])
        .child(&pop_box)
        .build();

    let current = cfg.borrow().alert_sound.clone();
    for (name, _) in ALERT_SOUNDS {
        let is_active = name.eq_ignore_ascii_case(&current);
        let (item_wrap, item_btn) = make_popover_action_row(name, is_active, None);
        let c = cfg.clone();
        let lbl = value_label.clone();
        let pop = popover.clone();
        item_btn.connect_clicked(move |_| {
            let (vol, dev) = {
                let mut guard = c.borrow_mut();
                guard.alert_sound = name.to_string();
                let _ = guard.save();
                (guard.alert_volume, guard.effects_device.clone())
            };
            lbl.set_label(name);
            pop.popdown();
            play_alert_preview(name.to_string(), vol, dev);
        });
        pop_box.append(&item_wrap);
    }

    build_inline_menu_button(value_label, &popover)
}

fn make_sound_pending_switch_row(
    title: &str,
    initial: bool,
    on_toggle: impl Fn(bool) + 'static,
) -> GtkBox {
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
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    left.append(&make_pending_badge());
    row.append(&left);

    let sw = Switch::new(initial);
    sw.widget().set_valign(Align::Center);
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(sw.widget());
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_balance_channel_math_roundtrips_cleanly() {
        assert_eq!(channel_volumes_for_balance(80.0, 0.0), (80, 80));
        assert_eq!(channel_volumes_for_balance(80.0, -1.0), (80, 0));
        assert_eq!(channel_volumes_for_balance(80.0, 1.0), (0, 80));
        assert_eq!(channel_volumes_for_balance(80.0, -0.5), (80, 40));
        assert_eq!(channel_volumes_for_balance(80.0, 0.5), (40, 80));

        let (m, b) = master_and_balance_from_channels(80.0, 40.0);
        assert!((m - 80.0).abs() < 1e-6);
        assert!((b - (-0.5)).abs() < 1e-6);

        let (m2, b2) = master_and_balance_from_channels(40.0, 80.0);
        assert!((m2 - 80.0).abs() < 1e-6);
        assert!((b2 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn parse_pactl_json_extracts_devices_types_and_filters_monitors() {
        let sample = r#"[
            {
                "name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
                "description": "Bocinas de MacBook Air",
                "mute": false,
                "active_port": "analog-output-speaker",
                "volume": {
                    "front-left": { "value_percent": "72%" },
                    "front-right": { "value_percent": "72%" }
                },
                "properties": { "device.bus": "pci" }
            },
            {
                "name": "alsa_output.pci-0000_01_00.1.hdmi-stereo",
                "description": "Mi Monitor",
                "mute": true,
                "active_port": "hdmi-output-0",
                "volume": {
                    "front-left": { "value_percent": "50%" },
                    "front-right": { "value_percent": "25%" }
                },
                "properties": { "device.bus": "pci", "device.form_factor": "displayport" }
            },
            {
                "name": "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor",
                "description": "Monitor of Bocinas",
                "mute": false,
                "volume": {}
            }
        ]"#;

        let sinks = parse_pactl_devices_json(
            sample,
            "alsa_output.pci-0000_00_1f.3.analog-stereo",
            false,
        );
        assert_eq!(sinks.len(), 3);
        assert!(sinks[0].is_default);
        assert_eq!(sinks[0].device_type, "Integrado");
        assert!((sinks[0].volume_pct - 72.0).abs() < 1e-6);
        assert!((sinks[0].balance).abs() < 1e-6);
        assert_eq!(sinks[1].device_type, "DisplayPort");
        assert!(sinks[1].muted);
        assert!((sinks[1].balance - (-0.5)).abs() < 1e-6);

        let sources = parse_pactl_devices_json(sample, "", true);
        assert_eq!(sources.len(), 2);
        assert!(!sources.iter().any(|s| s.name.ends_with(".monitor")));
    }
}
