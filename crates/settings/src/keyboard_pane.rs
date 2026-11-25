use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, Image, Label,
    Orientation, PolicyType, Popover, Scale, ScrolledWindow, StringList, Window,
};
use malus_de_common::config::{InputConfig, KeyboardPrefsConfig, TextReplacementEntry};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

/// Velocidades de repetición (caracteres/segundo) mapeadas al deslizador 0..=7 (0 = Desactivada).
const REPEAT_RATES: [i32; 8] = [0, 10, 15, 20, 25, 33, 45, 60];
/// Espera hasta repetición (ms) mapeada al deslizador 0..=5 (0 = Larga 1000ms -> 5 = Corta 200ms).
const REPEAT_DELAYS_MS: [i32; 6] = [1000, 750, 550, 400, 280, 200];

const XKB_LAYOUTS: &[(&str, &str, &str)] = &[
    (
        "latam",
        "Latinoamericano",
        "Español (Latinoamérica) · ñ, acentos muertos, ¿¡ y @ en AltGr+Q",
    ),
    (
        "es",
        "Español (España)",
        "Distribución ISO española · ç, ñ y @ en AltGr+2",
    ),
    (
        "us",
        "Inglés (EE. UU.)",
        "Distribución ANSI estadounidense estándar",
    ),
    ("fr", "Francés", "Distribución AZERTY francesa"),
    ("de", "Alemán", "Distribución QWERTZ alemana"),
    ("br", "Portugués (Brasil)", "Distribución ABNT2 brasileña"),
    ("it", "Italiano", "Distribución italiana estándar"),
];

const SYSTEM_SHORTCUTS: &[(&str, &str)] = &[
    ("Super + Espacio", "Abrir Canopy (búsqueda universal)"),
    ("Super + Tab / Alt + Tab", "Cambiar entre aplicaciones abiertas"),
    ("Super + ↑ / Ctrl + ↑", "Abrir Mission Control"),
    ("Super + L / Ctrl + Cmd + Q", "Bloquear pantalla"),
    ("Super + Shift + 3", "Captura de pantalla completa"),
    ("Super + Shift + 4", "Captura de área o ventana"),
    ("Super + Shift + 5", "Panel de captura y grabación"),
    ("Super + W / Alt + F4", "Cerrar ventana activa"),
    ("Super + M", "Minimizar ventana activa"),
    ("Super + ,", "Abrir Ajustes del Sistema"),
];

fn repeat_rate_to_slider_idx(rate: i32) -> f64 {
    if rate <= 0 {
        return 0.0;
    }
    REPEAT_RATES
        .iter()
        .enumerate()
        .skip(1)
        .min_by_key(|(_, &r)| (r - rate).abs())
        .map(|(i, _)| i as f64)
        .unwrap_or(4.0)
}

fn repeat_delay_to_slider_idx(delay_ms: i32) -> f64 {
    REPEAT_DELAYS_MS
        .iter()
        .enumerate()
        .min_by_key(|(_, &d)| (d - delay_ms).abs())
        .map(|(i, _)| i as f64)
        .unwrap_or(3.0)
}

fn sync_gsettings_keyboard_repeat(rate: i32, delay_ms: i32) {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return;
    };
    let schema = "org.gnome.desktop.peripherals.keyboard";
    if source.lookup(schema, true).is_none() {
        return;
    }
    let gs = gtk4::gio::Settings::new(schema);
    let enabled = rate > 0;
    let _ = gs.set_boolean("repeat", enabled);
    if enabled {
        let interval_ms = (1000 / rate.max(1)).clamp(15, 250) as u32;
        let _ = gs.set_uint("repeat-interval", interval_ms);
    }
    let _ = gs.set_uint("delay", delay_ms.clamp(150, 1500) as u32);
}

fn find_kbd_backlight_dir() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/leds").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.contains("kbd_backlight") {
            let p = entry.path();
            if p.join("brightness").is_file() && p.join("max_brightness").is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn read_kbd_backlight_pct(dir: &std::path::Path) -> Option<u8> {
    let cur: f64 = std::fs::read_to_string(dir.join("brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let max: f64 = std::fs::read_to_string(dir.join("max_brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if max <= 0.0 {
        return None;
    }
    Some(((cur / max) * 100.0).round().clamp(0.0, 100.0) as u8)
}

fn write_kbd_backlight_pct(dir: &std::path::Path, pct: u8) {
    let Ok(max_str) = std::fs::read_to_string(dir.join("max_brightness")) else {
        return;
    };
    let Ok(max) = max_str.trim().parse::<f64>() else {
        return;
    };
    if max <= 0.0 {
        return;
    }
    let val = ((pct as f64 / 100.0) * max).round().clamp(0.0, max) as u32;
    let _ = std::fs::write(dir.join("brightness"), format!("{val}\n"));
}

fn probe_microphone_sources() -> Vec<String> {
    let mut sources = vec!["Automático".to_string()];
    if let Ok(out) = Command::new("pactl")
        .args(["-f", "text", "list", "sources"])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                if let Some(desc) = trimmed.strip_prefix("Description:") {
                    let name = desc.trim();
                    if !name.is_empty()
                        && !name.to_lowercase().contains("monitor of")
                        && !name.to_lowercase().contains("monitor de")
                        && !sources.iter().any(|s| s == name)
                    {
                        sources.push(name.to_string());
                    }
                }
            }
        }
    }
    sources
}

fn show_input_sources_modal(parent: &ApplicationWindow, on_updated: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(470)
        .build();

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
            .label("Fuentes de entrada (XKB)")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    root.append(
        &Label::builder()
            .label(
                "Selecciona la distribución de teclado XKB activa en toda la sesión Wayland de \
                 Malus DE.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let list_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let current_layout = Rc::new(RefCell::new(InputConfig::load().keyboard_layout));
    let mut check_icons: Vec<(String, Image)> = Vec::new();

    for (idx, (code, label, desc)) in XKB_LAYOUTS.iter().enumerate() {
        if idx > 0 {
            list_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let btn = Button::builder()
            .css_classes(["wifi-nav-row"])
            .has_frame(false)
            .hexpand(true)
            .build();
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .valign(Align::Center)
            .build();
        let col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(1)
            .hexpand(true)
            .build();
        col.append(
            &Label::builder()
                .label(&format!("{label} ({code})"))
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        col.append(
            &Label::builder()
                .label(*desc)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        let check = Image::builder()
            .icon_name("object-select-symbolic")
            .pixel_size(14)
            .valign(Align::Center)
            .visible(*code == current_layout.borrow().as_str())
            .build();
        row.append(&col);
        row.append(&check);
        btn.set_child(Some(&row));

        check_icons.push(((*code).to_string(), check));
        list_card.append(&btn);
    }

    let checks_rc = Rc::new(check_icons);
    let mut child = list_card.first_child();
    let mut btn_idx = 0usize;
    while let Some(w) = child {
        if let Some(btn) = w.downcast_ref::<Button>() {
            if let Some((code, _, _)) = XKB_LAYOUTS.get(btn_idx) {
                let code_s = (*code).to_string();
                let checks_c = checks_rc.clone();
                let cur_c = current_layout.clone();
                let cb_c = on_updated.clone();
                btn.connect_clicked(move |_| {
                    let mut cfg = InputConfig::load();
                    cfg.keyboard_layout = code_s.clone();
                    cfg.keyboard_variant = String::new();
                    let _ = cfg.save();
                    let _ = IpcClient::send_command(IpcMessage::SetKeyboardLayout {
                        layout: code_s.clone(),
                        variant: String::new(),
                    });
                    *cur_c.borrow_mut() = code_s.clone();
                    for (c, img) in checks_c.iter() {
                        img.set_visible(c == &code_s);
                    }
                    cb_c();
                });
            }
            btn_idx += 1;
        }
        child = w.next_sibling();
    }

    root.append(&wrap_squircle_card(&list_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Campo de prueba en vivo (conservado del panel original)
    let test_entry = Entry::builder()
        .placeholder_text("Prueba en vivo: niño, canción, ¿qué tal?, @...")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    root.append(&test_entry);

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&ok_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));
    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

fn show_shortcuts_modal(parent: &ApplicationWindow) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(470)
        .build();

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
            .label("Atajos de teclado")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    root.append(
        &Label::builder()
            .label("Atajos activos en el compositor y entorno de Malus DE.")
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    for (idx, (keys, desc)) in SYSTEM_SHORTCUTS.iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        row.append(
            &Label::builder()
                .label(*desc)
                .css_classes(["wifi-row-title"])
                .hexpand(true)
                .xalign(0.0)
                .build(),
        );
        row.append(
            &Label::builder()
                .label(*keys)
                .css_classes(["card-section-sub"])
                .halign(Align::End)
                .build(),
        );
        card.append(&row);
    }
    root.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&ok_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));
    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

fn show_text_replacements_modal(
    parent: &ApplicationWindow,
    prefs: &Rc<RefCell<KeyboardPrefsConfig>>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(460)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    let hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    hdr.append(
        &Label::builder()
            .label("Sustituciones de texto")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr.append(&make_pending_badge());
    root.append(&hdr);

    root.append(
        &Label::builder()
            .label(
                "Define abreviaturas rápidas guardadas en keyboard.toml. La expansión automática \
                 global mediante IME del sistema queda pendiente de implementar.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let list_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    root.append(&wrap_squircle_card(&list_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let add_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let trig_ent = Entry::builder()
        .placeholder_text("Reemplazar")
        .css_classes(["sidebar-search"])
        .width_request(115)
        .build();
    let repl_ent = Entry::builder()
        .placeholder_text("Por texto...")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    let (add_wrap, add_btn) = make_squircle_button("Agregar", false, None);
    add_row.append(&trig_ent);
    add_row.append(&repl_ent);
    add_row.append(&add_wrap);
    root.append(&add_row);

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&ok_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));

    let rebuild_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuild: Rc<dyn Fn()> = {
        let prefs_c = prefs.clone();
        let list_c = list_card.clone();
        let slot_c = rebuild_slot.clone();
        Rc::new(move || {
            while let Some(child) = list_c.first_child() {
                list_c.remove(&child);
            }
            let items = prefs_c.borrow().text_replacements.clone();
            if items.is_empty() {
                let row = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .css_classes(["wifi-pref-row"])
                    .build();
                row.append(
                    &Label::builder()
                        .label("No hay sustituciones de texto configuradas.")
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .build(),
                );
                list_c.append(&row);
                return;
            }
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    list_c.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .css_classes(["wifi-pref-row"])
                    .build();
                row.append(
                    &Label::builder()
                        .label(&item.trigger)
                        .css_classes(["wifi-row-title"])
                        .width_request(95)
                        .xalign(0.0)
                        .build(),
                );
                row.append(
                    &Label::builder()
                        .label(&item.replacement)
                        .css_classes(["card-section-sub"])
                        .hexpand(true)
                        .xalign(0.0)
                        .ellipsize(gtk4::pango::EllipsizeMode::End)
                        .build(),
                );
                let (del_wrap, del_btn) = make_squircle_button("Quitar", false, None);
                let p_d = prefs_c.clone();
                let sl_d = slot_c.clone();
                del_btn.connect_clicked(move |_| {
                    {
                        let mut c = p_d.borrow_mut();
                        if idx < c.text_replacements.len() {
                            c.text_replacements.remove(idx);
                            let _ = c.save();
                        }
                    }
                    if let Some(rf) = sl_d.borrow().as_ref() {
                        rf();
                    }
                });
                row.append(&del_wrap);
                list_c.append(&row);
            }
        })
    };
    *rebuild_slot.borrow_mut() = Some(rebuild.clone());
    rebuild();

    {
        let prefs_c = prefs.clone();
        let rb_c = rebuild;
        add_btn.connect_clicked(move |_| {
            let t = trig_ent.text().trim().to_string();
            let r = repl_ent.text().trim().to_string();
            if t.is_empty() || r.is_empty() {
                return;
            }
            {
                let mut c = prefs_c.borrow_mut();
                c.text_replacements.push(TextReplacementEntry {
                    trigger: t,
                    replacement: r,
                });
                let _ = c.save();
            }
            trig_ent.set_text("");
            repl_ent.set_text("");
            rb_c();
        });
    }

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

fn show_dictation_privacy_modal(parent: &ApplicationWindow) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(420)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build();

    root.append(
        &Image::builder()
            .icon_name("audio-input-microphone-symbolic")
            .pixel_size(36)
            .halign(Align::Center)
            .build(),
    );
    root.append(
        &Label::builder()
            .label("Dictado y la privacidad")
            .css_classes(["card-section-title"])
            .halign(Align::Center)
            .build(),
    );
    root.append(
        &Label::builder()
            .label(
                "El dictado por voz local para Malus DE está previsto mediante modelos locales \
                 (Whisper / Vosk) sin envío obligatorio a la nube. Esta función está marcada como \
                 «por implementar».",
            )
            .css_classes(["card-section-sub"])
            .justify(gtk4::Justification::Center)
            .wrap(true)
            .build(),
    );
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&ok_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));
    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

/// Construye el panel «Teclado» estilo macOS Tahoe.
pub fn create_keyboard_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
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

    let input_cfg = Rc::new(RefCell::new(InputConfig::load()));
    let prefs = Rc::new(RefCell::new(KeyboardPrefsConfig::load()));
    let syncing = Rc::new(Cell::new(false));
    let kbd_backlight_dir = Rc::new(find_kbd_backlight_dir());

    // ========================================================
    // Tarjeta 1: Velocidad de repetición y Espera hasta la repetición (2 columnas, REAL)
    // ========================================================
    let repeat_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .homogeneous(true)
        .spacing(20)
        .css_classes(["wifi-pref-row"])
        .build();

    // Columna izquierda: Velocidad de repetición
    let rate_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .build();
    rate_col.append(
        &Label::builder()
            .label("Velocidad de repetición de tecla")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let rate_adj = Adjustment::new(
        repeat_rate_to_slider_idx(input_cfg.borrow().keyboard_repeat_rate),
        0.0,
        7.0,
        1.0,
        1.0,
        0.0,
    );
    let rate_scale = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&rate_adj)
        .draw_value(false)
        .round_digits(0)
        .hexpand(true)
        .build();
    for tick in 0..=7 {
        rate_scale.add_mark(tick as f64, gtk4::PositionType::Bottom, None);
    }
    let rate_labels = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    rate_labels.append(
        &Label::builder()
            .label("Desactivada  Lenta")
            .css_classes(["card-section-sub"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    rate_labels.append(
        &Label::builder()
            .label("Rápida")
            .css_classes(["card-section-sub"])
            .xalign(1.0)
            .build(),
    );
    rate_col.append(&rate_scale);
    rate_col.append(&rate_labels);

    // Columna derecha: Espera hasta la repetición
    let delay_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .build();
    delay_col.append(
        &Label::builder()
            .label("Espera hasta la repetición")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let delay_adj = Adjustment::new(
        repeat_delay_to_slider_idx(input_cfg.borrow().keyboard_repeat_delay_ms),
        0.0,
        5.0,
        1.0,
        1.0,
        0.0,
    );
    let delay_scale = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&delay_adj)
        .draw_value(false)
        .round_digits(0)
        .hexpand(true)
        .build();
    for tick in 0..=5 {
        delay_scale.add_mark(tick as f64, gtk4::PositionType::Bottom, None);
    }
    let delay_labels = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    delay_labels.append(
        &Label::builder()
            .label("Larga")
            .css_classes(["card-section-sub"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    delay_labels.append(
        &Label::builder()
            .label("Corta")
            .css_classes(["card-section-sub"])
            .xalign(1.0)
            .build(),
    );
    delay_col.append(&delay_scale);
    delay_col.append(&delay_labels);

    repeat_card.append(&rate_col);
    repeat_card.append(&delay_col);
    pane.append(&wrap_squircle_card(&repeat_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 2: Iluminación, Tecla Globo, Navegación y Atajos
    // ========================================================
    let card2 = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    // 2.1 Ajustar brillo en escasa iluminación (por implementar)
    let low_light_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let low_light_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    low_light_left.append(
        &Label::builder()
            .label("Ajustar brillo del teclado en condiciones de escasa iluminación")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    low_light_left.append(&make_pending_badge());
    let low_light_sw = Switch::builder()
        .valign(Align::Center)
        .active(prefs.borrow().adjust_backlight_low_light)
        .build();
    low_light_row.append(&low_light_left);
    low_light_row.append(&*low_light_sw);

    // 2.2 Brillo del teclado (real si hay /sys/class/leds/*kbd_backlight*, si no por implementar)
    let has_hw_backlight = kbd_backlight_dir.is_some();
    let bright_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let bright_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    bright_left.append(
        &Label::builder()
            .label("Brillo del teclado")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    if !has_hw_backlight {
        bright_left.append(&make_pending_badge());
    }
    let init_bright = kbd_backlight_dir
        .as_deref()
        .and_then(read_kbd_backlight_pct)
        .unwrap_or(prefs.borrow().backlight_level_pct) as f64;
    let bright_adj = Adjustment::new(init_bright, 0.0, 100.0, 5.0, 10.0, 0.0);
    let bright_scale = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&bright_adj)
        .draw_value(false)
        .width_request(160)
        .valign(Align::Center)
        .build();
    bright_row.append(&bright_left);
    bright_row.append(
        &Image::builder()
            .icon_name("keyboard-brightness-symbolic")
            .pixel_size(14)
            .valign(Align::Center)
            .build(),
    );
    bright_row.append(&bright_scale);

    // 2.3 Apagar iluminación después de inactividad (por implementar)
    let idle_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let idle_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    idle_left.append(
        &Label::builder()
            .label("Apagar la iluminación del teclado después de inactividad")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    idle_left.append(&make_pending_badge());
    let idle_dd = DropDown::builder()
        .model(&StringList::new(&[
            "Después de 5 segundos",
            "Después de 10 segundos",
            "Después de 30 segundos",
            "Después de 1 minuto",
            "Después de 5 minutos",
            "Nunca",
        ]))
        .selected((prefs.borrow().backlight_idle_idx as u32).min(5))
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    idle_row.append(&idle_left);
    idle_row.append(&idle_dd);

    // 2.4 Presionar la tecla Globo para (por implementar)
    let globe_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let globe_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    globe_left.append(
        &Label::builder()
            .label("Presionar la tecla 🌐 para")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    globe_left.append(&make_pending_badge());
    let globe_dd = DropDown::builder()
        .model(&StringList::new(&[
            "Cambiar fuente de entrada",
            "Mostrar emojis y símbolos",
            "Iniciar dictado",
            "No hacer nada",
        ]))
        .selected((prefs.borrow().globe_key_action_idx as u32).min(3))
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    globe_row.append(&globe_left);
    globe_row.append(&globe_dd);

    // 2.5 Navegación con teclado (por implementar)
    let nav_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let nav_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    let nav_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    nav_hdr.append(
        &Label::builder()
            .label("Navegación con teclado")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    nav_hdr.append(&make_pending_badge());
    nav_col.append(&nav_hdr);
    nav_col.append(
        &Label::builder()
            .label(
                "Usa la navegación con teclado para desplazar el enfoque entre los controles: \
                 presiona la tecla Tabulador para avanzar y las teclas Mayúsculas + Tabulador \
                 para retroceder.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let nav_sw = Switch::builder()
        .valign(Align::Center)
        .active(prefs.borrow().keyboard_navigation)
        .build();
    nav_row.append(&nav_col);
    nav_row.append(&*nav_sw);

    // 2.6 Botón "Atajos de teclado..." (REAL)
    let shortcuts_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (shortcuts_wrap, shortcuts_btn) =
        make_squircle_button("Atajos de teclado...", false, None);
    shortcuts_row.append(&shortcuts_wrap);

    let c2_rows = [
        low_light_row,
        bright_row,
        idle_row,
        globe_row,
        nav_row,
        shortcuts_row,
    ];
    for (idx, r) in c2_rows.into_iter().enumerate() {
        if idx > 0 {
            card2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        card2.append(&r);
    }
    pane.append(&wrap_squircle_card(&card2, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Sección 3: Entrada de texto (Fuentes de entrada XKB REAL + Sustituciones)
    // ========================================================
    pane.append(
        &Label::builder()
            .label("Entrada de texto")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    let text_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let sources_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    sources_row.append(
        &Label::builder()
            .label("Fuentes de entrada")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let active_layout_lbl = Label::builder()
        .label(input_cfg.borrow().layout_display_name())
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let (edit_sources_wrap, edit_sources_btn) = make_squircle_button("Editar...", false, None);
    sources_row.append(&active_layout_lbl);
    sources_row.append(&edit_sources_wrap);

    let repl_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (repl_wrap, repl_btn) = make_squircle_button("Sustituciones de texto...", false, None);
    repl_row.append(&repl_wrap);

    text_card.append(&sources_row);
    text_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    text_card.append(&repl_row);
    pane.append(&wrap_squircle_card(&text_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Sección 4: Dictado (por implementar + fuente real de micrófono)
    // ========================================================
    pane.append(
        &Label::builder()
            .label("Dictado")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    let dict_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let dict_hdr_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    dict_hdr_row.append(
        &Image::builder()
            .icon_name("audio-input-microphone-symbolic")
            .pixel_size(22)
            .valign(Align::Center)
            .build(),
    );
    let dict_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    let dict_badge_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::Start)
        .build();
    dict_badge_row.append(&make_pending_badge());
    dict_col.append(
        &Label::builder()
            .label(
                "Usa Dictado donde sea que puedas escribir texto. Para empezar a dictar, usa el \
                 atajo de teclado o selecciona Iniciar dictado en el menú Edición.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    dict_col.append(&dict_badge_row);
    let dict_sw = Switch::builder()
        .valign(Align::Center)
        .active(prefs.borrow().dictation_enabled)
        .build();
    dict_hdr_row.append(&dict_col);
    dict_hdr_row.append(&*dict_sw);

    let lang_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let lang_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    lang_left.append(
        &Label::builder()
            .label("Idiomas")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    lang_left.append(&make_pending_badge());
    let lang_val = Label::builder()
        .label(&prefs.borrow().dictation_language)
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let (edit_lang_wrap, edit_lang_btn) = make_squircle_button("Editar...", false, None);
    edit_lang_btn.set_sensitive(false);
    lang_row.append(&lang_left);
    lang_row.append(&lang_val);
    lang_row.append(&edit_lang_wrap);

    // Fuente del micrófono (detecta micrófonos reales de PipeWire/PulseAudio)
    let mic_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    mic_row.append(
        &Label::builder()
            .label("Fuente del micrófono")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let mic_sources: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(vec!["Automático".to_string()]));
    let mic_dd = DropDown::builder()
        .model(&StringList::new(&["Automático"]))
        .selected(0)
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    mic_row.append(&mic_dd);

    let dshort_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let dshort_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    dshort_left.append(
        &Label::builder()
            .label("Atajo")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    dshort_left.append(&make_pending_badge());
    let dshort_dd = DropDown::builder()
        .model(&StringList::new(&[
            "Presionar 🎙",
            "Presionar Ctrl dos veces",
            "Presionar Fn dos veces",
            "Desactivado",
        ]))
        .selected((prefs.borrow().dictation_shortcut_idx as u32).min(3))
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    dshort_row.append(&dshort_left);
    dshort_row.append(&dshort_dd);

    let dict_priv_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (dict_priv_wrap, dict_priv_btn) =
        make_squircle_button("Acerca de Dictado y la privacidad...", false, None);
    dict_priv_row.append(&dict_priv_wrap);

    let d_rows = [dict_hdr_row, lang_row, mic_row, dshort_row, dict_priv_row];
    for (idx, r) in d_rows.into_iter().enumerate() {
        if idx > 0 {
            dict_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        dict_card.append(&r);
    }
    pane.append(&wrap_squircle_card(&dict_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Pie inferior: Configurar teclado... y ?
    // ========================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (setup_wrap, setup_btn) = make_squircle_button("Configurar teclado...", false, None);
    setup_btn.set_sensitive(false);
    setup_btn.set_tooltip_text(Some("Asistente de teclado Bluetooth (por implementar)"));
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Teclado");

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
            .label("Teclado")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "Velocidad/espera de repetición, Fuentes de entrada (XKB en vivo), Atajos de \
                 teclado y selección de micrófono están activos en Malus DE. El brillo del teclado \
                 se conecta automáticamente si el equipo expone /sys/class/leds/*kbd_backlight*.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    help_popover.set_child(Some(&help_box));
    help_btn.set_popover(Some(&help_popover));

    footer.append(&make_pending_badge());
    footer.append(&setup_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    // ========================================================
    // Refresco desde disco y PipeWire
    // ========================================================
    let refresh: Rc<dyn Fn()> = {
        let icfg_c = input_cfg.clone();
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        let layout_lbl_c = active_layout_lbl;
        let rate_sc = rate_scale.clone();
        let delay_sc = delay_scale.clone();
        let low_sw = low_light_sw.clone();
        let br_sc = bright_scale.clone();
        let bl_dir = kbd_backlight_dir.clone();
        let idl_dd = idle_dd.clone();
        let glb_dd = globe_dd.clone();
        let nv_sw = nav_sw.clone();
        let dc_sw = dict_sw.clone();
        let ds_dd = dshort_dd.clone();
        let mc_dd = mic_dd.clone();
        let mc_list = mic_sources.clone();

        Rc::new(move || {
            let ic = InputConfig::load();
            let kp = KeyboardPrefsConfig::load();
            *icfg_c.borrow_mut() = ic.clone();
            *prefs_c.borrow_mut() = kp.clone();

            layout_lbl_c.set_label(ic.layout_display_name());

            sync_c.set(true);
            rate_sc.set_value(repeat_rate_to_slider_idx(ic.keyboard_repeat_rate));
            delay_sc.set_value(repeat_delay_to_slider_idx(ic.keyboard_repeat_delay_ms));
            low_sw.set_active(kp.adjust_backlight_low_light);
            let b_val = bl_dir
                .as_deref()
                .and_then(read_kbd_backlight_pct)
                .unwrap_or(kp.backlight_level_pct) as f64;
            br_sc.set_value(b_val);
            idl_dd.set_selected((kp.backlight_idle_idx as u32).min(5));
            glb_dd.set_selected((kp.globe_key_action_idx as u32).min(3));
            nv_sw.set_active(kp.keyboard_navigation);
            dc_sw.set_active(kp.dictation_enabled);
            ds_dd.set_selected((kp.dictation_shortcut_idx as u32).min(3));
            sync_c.set(false);

            let mc_dd_r = mc_dd.clone();
            let mc_list_r = mc_list.clone();
            let sync_r = sync_c.clone();
            let saved_mic = kp.dictation_mic_source.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(sources) = gtk4::gio::spawn_blocking(probe_microphone_sources).await else {
                    return;
                };
                let sel = sources
                    .iter()
                    .position(|s| s == &saved_mic)
                    .unwrap_or(0) as u32;
                let model = {
                    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
                    StringList::new(&refs)
                };
                *mc_list_r.borrow_mut() = sources;
                sync_r.set(true);
                mc_dd_r.set_model(Some(&model));
                mc_dd_r.set_selected(sel);
                sync_r.set(false);
            });
        })
    };

    // ========================================================
    // Señales interactivas
    // ========================================================
    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        rate_scale.connect_value_changed(move |sc| {
            if sync_c.get() {
                return;
            }
            let idx = (sc.value().round() as usize).min(REPEAT_RATES.len() - 1);
            let rate = REPEAT_RATES[idx];
            let delay = {
                let mut c = icfg_c.borrow_mut();
                if c.keyboard_repeat_rate == rate {
                    return;
                }
                c.keyboard_repeat_rate = rate;
                let _ = c.save();
                c.keyboard_repeat_delay_ms
            };
            sync_gsettings_keyboard_repeat(rate, delay);
            let _ = IpcClient::send_command(IpcMessage::ReloadInputConfig);
        });
    }
    {
        let icfg_c = input_cfg;
        let sync_c = syncing.clone();
        delay_scale.connect_value_changed(move |sc| {
            if sync_c.get() {
                return;
            }
            let idx = (sc.value().round() as usize).min(REPEAT_DELAYS_MS.len() - 1);
            let delay = REPEAT_DELAYS_MS[idx];
            let rate = {
                let mut c = icfg_c.borrow_mut();
                if c.keyboard_repeat_delay_ms == delay {
                    return;
                }
                c.keyboard_repeat_delay_ms = delay;
                let _ = c.save();
                c.keyboard_repeat_rate
            };
            sync_gsettings_keyboard_repeat(rate, delay);
            let _ = IpcClient::send_command(IpcMessage::ReloadInputConfig);
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        low_light_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = prefs_c.borrow_mut();
                c.adjust_backlight_low_light = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        let bl_dir = kbd_backlight_dir;
        bright_scale.connect_value_changed(move |sc| {
            if sync_c.get() {
                return;
            }
            let pct = sc.value().round().clamp(0.0, 100.0) as u8;
            if let Some(ref dir) = *bl_dir {
                write_kbd_backlight_pct(dir, pct);
            }
            let mut c = prefs_c.borrow_mut();
            if c.backlight_level_pct != pct {
                c.backlight_level_pct = pct;
                let _ = c.save();
            }
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        idle_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let mut c = prefs_c.borrow_mut();
            c.backlight_idle_idx = dd.selected().min(5) as u8;
            let _ = c.save();
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        globe_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let mut c = prefs_c.borrow_mut();
            c.globe_key_action_idx = dd.selected().min(3) as u8;
            let _ = c.save();
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        nav_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = prefs_c.borrow_mut();
                c.keyboard_navigation = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let win_c = window.clone();
        shortcuts_btn.connect_clicked(move |_| show_shortcuts_modal(&win_c));
    }
    {
        let win_c = window.clone();
        let rf_c = refresh.clone();
        edit_sources_btn.connect_clicked(move |_| {
            show_input_sources_modal(&win_c, rf_c.clone());
        });
    }
    {
        let win_c = window.clone();
        let prefs_c = prefs.clone();
        repl_btn.connect_clicked(move |_| {
            show_text_replacements_modal(&win_c, &prefs_c);
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        dict_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = prefs_c.borrow_mut();
                c.dictation_enabled = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let prefs_c = prefs.clone();
        let sync_c = syncing.clone();
        let mc_list = mic_sources;
        mic_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            if let Some(name) = mc_list.borrow().get(dd.selected() as usize) {
                let mut c = prefs_c.borrow_mut();
                c.dictation_mic_source = name.clone();
                let _ = c.save();
            }
        });
    }
    {
        let prefs_c = prefs;
        let sync_c = syncing;
        dshort_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let mut c = prefs_c.borrow_mut();
            c.dictation_shortcut_idx = dd.selected().min(3) as u8;
            let _ = c.save();
        });
    }
    {
        let win_c = window.clone();
        dict_priv_btn.connect_clicked(move |_| show_dictation_privacy_modal(&win_c));
    }

    refresh();
    scrolled.set_child(Some(&pane));
    (scrolled, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_repeat_sliders_map_cleanly_to_rates_and_delays() {
        assert_eq!(repeat_rate_to_slider_idx(0), 0.0);
        assert_eq!(repeat_rate_to_slider_idx(25), 4.0);
        assert_eq!(repeat_rate_to_slider_idx(60), 7.0);
        assert_eq!(repeat_delay_to_slider_idx(1000), 0.0);
        assert_eq!(repeat_delay_to_slider_idx(400), 3.0);
        assert_eq!(repeat_delay_to_slider_idx(200), 5.0);
    }
}
