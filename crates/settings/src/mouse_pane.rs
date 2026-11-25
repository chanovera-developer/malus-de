use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, ApplicationWindow, Box as GtkBox, Button, DrawingArea, DropDown, Image,
    Label, Orientation, Popover, Scale, Stack, StackTransitionType, StringList, Window,
};
use malus_de_common::config::{InputConfig, MousePrefsConfig};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const MOUSE_PANEL_ID: &str = "mouse";
pub const MOUSE_PANEL_TITLE: &str = "Mouse";

/// Mapeo de los 10 pasos del deslizador «Velocidad del cursor» (0..=9) a velocidad libinput (-1.0..=1.0).
const MOUSE_CURSOR_SPEEDS: [f64; 10] = [-0.75, -0.55, -0.35, -0.18, 0.0, 0.16, 0.34, 0.52, 0.70, 0.88];

const SECONDARY_CLICK_OPTIONS: &[&str] = &[
    "Desactivado",
    "Hacer clic en el lado derecho",
    "Hacer clic en el lado izquierdo",
];

const SWIPE_PAGES_OPTIONS: &[&str] = &[
    "Desactivado",
    "Ir a la izquierda o la derecha con 1 dedo",
    "Ir a la izquierda o la derecha con 2 dedos",
    "Deslizar con 1 o 2 dedos",
];

fn speed_to_slider_idx(speed: f64) -> f64 {
    MOUSE_CURSOR_SPEEDS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            ((*a - speed).abs())
                .partial_cmp(&((*b - speed).abs()))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i as f64)
        .unwrap_or(4.0)
}

fn slider_idx_to_speed(idx: usize) -> f64 {
    MOUSE_CURSOR_SPEEDS[idx.min(MOUSE_CURSOR_SPEEDS.len() - 1)]
}

fn secondary_click_to_idx(mode: &str) -> u32 {
    match mode {
        "off" => 0,
        "left" => 2,
        _ => 1,
    }
}

fn idx_to_secondary_click(idx: u32) -> &'static str {
    match idx {
        0 => "off",
        2 => "left",
        _ => "right",
    }
}

fn notify_compositor_input_reload() {
    std::thread::spawn(|| {
        let _ = IpcClient::send_command(IpcMessage::ReloadInputConfig);
    });
}

fn sync_gsettings_mouse(cfg: &InputConfig) {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return;
    };
    let schema = "org.gnome.desktop.peripherals.mouse";
    if source.lookup(schema, true).is_none() {
        return;
    }
    let gs = gtk4::gio::Settings::new(schema);
    let _ = gs.set_double("speed", cfg.mouse_speed.clamp(-1.0, 1.0));
    let _ = gs.set_boolean("natural-scroll", cfg.mouse_natural_scrolling);
    let _ = gs.set_boolean("left-handed", cfg.mouse_secondary_click == "left");
    let profile = if cfg.mouse_pointer_acceleration {
        "adaptive"
    } else {
        "flat"
    };
    let _ = gs.set_string("accel-profile", profile);
}

/// Detecta si hay un mouse inalámbrico reportando porcentaje de batería en `upower` o `/sys/class/power_supply`.
fn probe_wireless_mouse_battery() -> Option<(String, u8)> {
    if let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let is_hid = name.contains("hid") || name.contains("mouse");
            if !is_hid {
                continue;
            }
            let scope = std::fs::read_to_string(path.join("scope"))
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if scope != "device" && !name.contains("mouse") {
                continue;
            }
            if let Ok(cap_raw) = std::fs::read_to_string(path.join("capacity")) {
                if let Ok(pct) = cap_raw.trim().parse::<u8>() {
                    let model = std::fs::read_to_string(path.join("model_name"))
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Mouse inalámbrico".to_string());
                    return Some((model, pct.min(100)));
                }
            }
        }
    }

    let out = Command::new("upower").arg("-e").output().ok()?;
    if !out.status.success() {
        return None;
    }
    for dev in String::from_utf8_lossy(&out.stdout).lines() {
        let dev = dev.trim();
        if !dev.to_lowercase().contains("mouse") {
            continue;
        }
        let info = Command::new("upower").args(["-i", dev]).output().ok()?;
        if !info.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&info.stdout);
        let mut model = String::new();
        let mut pct: Option<u8> = None;
        for line in text.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("model:") {
                model = rest.trim().to_string();
            } else if let Some(rest) = l.strip_prefix("percentage:") {
                if let Ok(v) = rest.trim().trim_end_matches('%').trim().parse::<f64>() {
                    pct = Some(v.round().clamp(0.0, 100.0) as u8);
                }
            }
        }
        if let Some(p) = pct {
            if model.is_empty() {
                model = "Mouse Bluetooth".to_string();
            }
            return Some((model, p));
        }
    }
    None
}

fn rounded_rect_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

/// Vista previa dual superior estilo macOS Tahoe (silueta de mouse + escritorio / Mission Control miniatura).
fn build_mouse_preview_hero(active_tab: Rc<Cell<u8>>, secondary_mode: Rc<Cell<u8>>) -> DrawingArea {
    let area = DrawingArea::builder()
        .content_height(126)
        .hexpand(true)
        .vexpand(false)
        .build();

    let tab_draw = active_tab.clone();
    let sec_draw = secondary_mode.clone();
    area.set_draw_func(move |widget, cr, width, height| {
        let w = width as f64;
        let h = height as f64;
        let tab = tab_draw.get();
        let sec = sec_draw.get();
        let (ar, ag, ab) = crate::accent_color();
        let fg = widget.color();
        let is_dark = (0.299 * fg.red() + 0.587 * fg.green() + 0.114 * fg.blue()) > 0.5;

        let card_gap = 12.0;
        let card_w = ((w - card_gap).min(410.0) / 2.0).max(140.0);
        let card_h = (h - 8.0).min(118.0).max(96.0);
        let total_w = card_w * 2.0 + card_gap;
        let start_x = ((w - total_w) * 0.5).max(0.0);
        let top_y = ((h - card_h) * 0.5).max(0.0);

        // --- Tarjeta izquierda: silueta minimalista de Mouse ---
        rounded_rect_path(cr, start_x, top_y, card_w, card_h, 12.0);
        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.038);
        }
        let _ = cr.fill_preserve();
        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.09);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.06);
        }
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        // Cuerpo ovalado del mouse
        let mouse_w = 42.0;
        let mouse_h = 76.0;
        let mx = start_x + (card_w - mouse_w) * 0.5;
        let my = top_y + (card_h - mouse_h) * 0.5;
        rounded_rect_path(cr, mx, my, mouse_w, mouse_h, 18.0);
        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.05);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.76);
        }
        let _ = cr.fill_preserve();
        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.30);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
        }
        cr.set_line_width(1.4);
        let _ = cr.stroke();

        // Pequeño emblema sutil en la parte inferior del mouse
        cr.set_source_rgba(
            if is_dark { 1.0 } else { 0.0 },
            if is_dark { 1.0 } else { 0.0 },
            if is_dark { 1.0 } else { 0.0 },
            0.18,
        );
        cr.arc(
            mx + mouse_w * 0.5,
            my + mouse_h * 0.78,
            4.2,
            0.0,
            std::f64::consts::TAU,
        );
        let _ = cr.fill();

        let draw_finger = |cx: f64, cy: f64, radius: f64| {
            cr.set_source_rgba(ar, ag, ab, 0.22);
            cr.arc(cx, cy, radius * 1.65, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.set_source_rgba(ar, ag, ab, 0.92);
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        };

        if tab == 0 {
            let fx = match sec {
                2 => mx + mouse_w * 0.33,
                1 => mx + mouse_w * 0.67,
                _ => mx + mouse_w * 0.50,
            };
            draw_finger(fx, my + mouse_h * 0.32, 4.3);
        } else {
            // Más gestos: 2 dedos arriba sobre el mouse
            draw_finger(mx + mouse_w * 0.34, my + mouse_h * 0.28, 3.8);
            draw_finger(mx + mouse_w * 0.66, my + mouse_h * 0.28, 3.8);
        }

        // --- Tarjeta derecha: escritorio miniatura o Mission Control ---
        let rx = start_x + card_w + card_gap;
        cr.save().ok();
        rounded_rect_path(cr, rx, top_y, card_w, card_h, 12.0);
        cr.clip();

        let grad = gtk4::cairo::LinearGradient::new(rx, top_y, rx, top_y + card_h);
        grad.add_color_stop_rgb(0.0, 0.44, 0.72, 0.94);
        grad.add_color_stop_rgb(1.0, 0.26, 0.58, 0.89);
        let _ = cr.set_source(&grad);
        cr.rectangle(rx, top_y, card_w, card_h);
        let _ = cr.fill();

        // Barra de menús superior
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.28);
        cr.rectangle(rx, top_y, card_w, 9.0);
        let _ = cr.fill();

        if tab == 0 {
            // Ventana miniatura con las 3 figuras geométricas
            let win_w = card_w * 0.72;
            let win_h = card_h * 0.68;
            let win_x = rx + (card_w - win_w) * 0.5;
            let win_y = top_y + 13.0;
            rounded_rect_path(cr, win_x, win_y, win_w, win_h, 6.0);
            cr.set_source_rgba(0.95, 0.97, 1.0, 0.92);
            let _ = cr.fill();

            cr.set_source_rgba(0.75, 0.85, 0.96, 0.90);
            rounded_rect_path(cr, win_x, win_y, win_w, 12.0, 5.0);
            let _ = cr.fill();
            for i in 0..3 {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.82);
                cr.arc(
                    win_x + 7.0 + (i as f64) * 6.0,
                    win_y + 6.0,
                    1.9,
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
            }

            let content_y = win_y + 17.0;
            cr.set_source_rgba(0.32, 0.56, 0.80, 0.70);
            cr.rectangle(win_x + 18.0, content_y, win_w * 0.34, 4.0);
            cr.rectangle(win_x + 18.0, content_y + 7.0, win_w * 0.30, 3.0);
            cr.rectangle(win_x + 18.0, content_y + 12.5, win_w * 0.28, 3.0);
            cr.set_source_rgba(0.45, 0.68, 0.88, 0.52);
            cr.rectangle(win_x + 18.0 + win_w * 0.38, content_y, win_w * 0.28, 4.0);
            cr.rectangle(win_x + 18.0 + win_w * 0.38, content_y + 7.0, win_w * 0.26, 3.0);
            cr.rectangle(win_x + 18.0 + win_w * 0.38, content_y + 12.5, win_w * 0.24, 3.0);
            let _ = cr.fill();

            let box_y = win_y + win_h * 0.54;
            let box_sz = (win_w * 0.22).min(24.0);
            let box_gap = 7.0;
            let boxes_w = box_sz * 3.0 + box_gap * 2.0;
            let bx0 = win_x + (win_w - boxes_w) * 0.5;
            for i in 0..3 {
                let bx = bx0 + (i as f64) * (box_sz + box_gap);
                rounded_rect_path(cr, bx, box_y, box_sz, box_sz, 4.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.96);
                let _ = cr.fill();

                match i {
                    0 => {
                        cr.set_source_rgb(0.48, 0.42, 0.92);
                        cr.arc(
                            bx + box_sz * 0.5,
                            box_y + box_sz * 0.5,
                            box_sz * 0.30,
                            0.0,
                            std::f64::consts::TAU,
                        );
                        let _ = cr.fill();
                    }
                    1 => {
                        cr.set_source_rgb(0.32, 0.74, 0.97);
                        let sq = box_sz * 0.52;
                        rounded_rect_path(
                            cr,
                            bx + (box_sz - sq) * 0.5,
                            box_y + (box_sz - sq) * 0.5,
                            sq,
                            sq,
                            2.0,
                        );
                        let _ = cr.fill();
                    }
                    _ => {
                        cr.set_source_rgb(0.95, 0.40, 0.26);
                        cr.move_to(bx + box_sz * 0.5, box_y + box_sz * 0.22);
                        cr.line_to(bx + box_sz * 0.78, box_y + box_sz * 0.76);
                        cr.line_to(bx + box_sz * 0.22, box_y + box_sz * 0.76);
                        cr.close_path();
                        let _ = cr.fill();
                    }
                }
            }
        } else {
            // Vista miniatura de Mission Control (5 ventanas flotantes organizadas)
            let tiles = [
                (0.08, 0.14, 0.32, 0.30),
                (0.45, 0.14, 0.22, 0.22),
                (0.72, 0.26, 0.22, 0.34),
                (0.14, 0.50, 0.22, 0.26),
                (0.42, 0.46, 0.26, 0.30),
            ];
            for (tx, ty, tw, th) in tiles {
                let x = rx + card_w * tx;
                let y = top_y + card_h * ty;
                let ww = card_w * tw;
                let wh = card_h * th;
                rounded_rect_path(cr, x, y, ww, wh, 3.0);
                cr.set_source_rgba(0.86, 0.93, 1.0, 0.82);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
                cr.set_line_width(0.8);
                let _ = cr.stroke();
            }
        }

        // Dock inferior miniatura
        let dock_w = card_w * 0.62;
        let dock_h = 11.0;
        let dock_x = rx + (card_w - dock_w) * 0.5;
        let dock_y = top_y + card_h - dock_h - 4.0;
        rounded_rect_path(cr, dock_x, dock_y, dock_w, dock_h, 4.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.42);
        let _ = cr.fill();

        let dock_colors = [
            (0.20, 0.62, 0.96),
            (0.95, 0.36, 0.32),
            (0.98, 0.72, 0.20),
            (0.32, 0.80, 0.42),
            (0.58, 0.42, 0.95),
            (0.24, 0.78, 0.88),
            (0.96, 0.46, 0.64),
            (0.42, 0.48, 0.58),
        ];
        let icon_sz = 6.5;
        let icon_gap = 4.0;
        let row_w =
            (dock_colors.len() as f64) * icon_sz + ((dock_colors.len() - 1) as f64) * icon_gap;
        let ix0 = dock_x + (dock_w - row_w) * 0.5;
        let iy = dock_y + (dock_h - icon_sz) * 0.5;
        for (i, (r, g, b)) in dock_colors.iter().enumerate() {
            let ix = ix0 + (i as f64) * (icon_sz + icon_gap);
            rounded_rect_path(cr, ix, iy, icon_sz, icon_sz, 1.8);
            cr.set_source_rgb(*r, *g, *b);
            let _ = cr.fill();
        }

        cr.restore().ok();

        rounded_rect_path(cr, rx, top_y, card_w, card_h, 12.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, if is_dark { 0.25 } else { 0.10 });
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    });

    crate::register_accent_draw_area(&area);
    area
}

fn make_row_divider() -> GtkBox {
    GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-row-separator"])
        .hexpand(true)
        .build()
}

fn make_card_row() -> GtkBox {
    GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .valign(Align::Center)
        .build()
}

fn make_title_col(title: &str, subtitle: Option<&str>, pending: bool) -> GtkBox {
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();

    let title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();
    let lbl = Label::builder()
        .label(title)
        .halign(Align::Start)
        .css_classes(["wifi-row-title"])
        .build();
    title_row.append(&lbl);
    if pending {
        title_row.append(&make_pending_badge());
    }
    col.append(&title_row);

    if let Some(sub) = subtitle {
        let sub_lbl = Label::builder()
            .label(sub)
            .halign(Align::Start)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["card-section-sub"])
            .build();
        col.append(&sub_lbl);
    }
    col
}

fn make_switch_row(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
    pending: bool,
) -> (GtkBox, Switch) {
    let row = make_card_row();
    row.append(&make_title_col(title, subtitle, pending));
    let sw = Switch::builder()
        .active(initial)
        .valign(Align::Center)
        .build();
    row.append(&*sw);
    (row, sw)
}

fn make_dropdown_row(
    title: &str,
    options: &[&str],
    initial_idx: u32,
    pending: bool,
) -> (GtkBox, DropDown) {
    let row = make_card_row();
    row.append(&make_title_col(title, None, pending));
    let dd = DropDown::builder()
        .model(&StringList::new(options))
        .selected(initial_idx.min(options.len().saturating_sub(1) as u32))
        .css_classes(["wifi-compact-dropdown"])
        .valign(Align::Center)
        .build();
    row.append(&dd);
    (row, dd)
}

/// Modal anclado «Avanzado...» idéntico al de macOS Tahoe: Aceleración del puntero + botón «Listo».
fn open_advanced_mouse_modal(
    parent: &ApplicationWindow,
    input_cfg: Rc<RefCell<InputConfig>>,
    on_apply: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .title("Avanzado")
        .transient_for(parent)
        .modal(true)
        .default_width(480)
        .resizable(false)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .margin_top(18)
        .margin_bottom(16)
        .margin_start(18)
        .margin_end(18)
        .build();

    let card_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    let (accel_row, accel_sw) = make_switch_row(
        "Aceleración del puntero",
        Some(
            "Si se activa, el puntero se desplazará más rápido al realizar movimientos rápidos con el mouse, y con más precisión al realizar movimientos lentos.",
        ),
        input_cfg.borrow().mouse_pointer_acceleration,
        false,
    );
    card_inner.append(&accel_row);

    let card = wrap_squircle_card(&card_inner, WIFI_CARD_SQUIRCLE_RADIUS);
    root.append(&card);

    let btn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let done_btn = Button::builder()
        .label("Listo")
        .css_classes(["suggested-action"])
        .build();
    btn_row.append(&done_btn);
    root.append(&btn_row);

    dialog.set_child(Some(&root));

    {
        let icfg_c = input_cfg.clone();
        let apply_c = on_apply.clone();
        accel_sw.connect_state_set(move |_, active| {
            icfg_c.borrow_mut().mouse_pointer_acceleration = active;
            apply_c();
            glib::Propagation::Proceed
        });
    }

    let dlg_c = dialog.clone();
    done_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_c);
    });

    crate::present_attached_modal(&dialog, parent);
}

pub fn create_mouse_pane(parent: &ApplicationWindow) -> (GtkBox, Rc<dyn Fn()>) {
    let input_cfg = Rc::new(RefCell::new(InputConfig::load()));
    let mouse_prefs = Rc::new(RefCell::new(MousePrefsConfig::load()));
    let syncing = Rc::new(Cell::new(false));
    let active_tab = Rc::new(Cell::new(0u8));
    let secondary_mode_cell = Rc::new(Cell::new(
        secondary_click_to_idx(&input_cfg.borrow().mouse_secondary_click) as u8,
    ));

    let container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    // Indicador compacto de batería (solo visible si se detecta un mouse inalámbrico con batería)
    let battery_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .halign(Align::Center)
        .visible(false)
        .build();
    let battery_icon = Image::builder()
        .icon_name("battery-good-symbolic")
        .pixel_size(14)
        .build();
    let battery_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .build();
    battery_row.append(&battery_icon);
    battery_row.append(&battery_lbl);
    container.append(&battery_row);

    // 1. Vista previa dual superior (Mouse + Escritorio / Mission Control)
    let preview_hero = build_mouse_preview_hero(active_tab.clone(), secondary_mode_cell.clone());
    container.append(&preview_hero);

    // 2. Barra segmentada de 2 pestañas estilo macOS Tahoe
    let seg_track = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .homogeneous(true)
        .css_classes(["bat-segmented-track"])
        .hexpand(true)
        .build();

    let tab_point_btn = Button::builder()
        .label("Señalamiento y clic")
        .css_classes(["bat-seg-tab", "bat-seg-tab-active"])
        .hexpand(true)
        .build();
    let tab_gestures_btn = Button::builder()
        .label("Más gestos")
        .css_classes(["bat-seg-tab"])
        .hexpand(true)
        .build();

    seg_track.append(&tab_point_btn);
    seg_track.append(&tab_gestures_btn);
    container.append(&seg_track);

    // 3. Stack de las 2 pestañas
    let tab_stack = Stack::builder()
        .transition_type(StackTransitionType::Crossfade)
        .transition_duration(120)
        .vhomogeneous(false)
        .hexpand(true)
        .build();

    // =========================================================================
    // Pestaña 1: Señalamiento y clic
    // =========================================================================
    let point_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    // Velocidad del cursor (EN VIVO -> libinput accel speed + GSettings)
    let speed_row = make_card_row();
    speed_row.append(&make_title_col("Velocidad del cursor", None, false));
    let speed_ctrl = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .valign(Align::Center)
        .width_request(210)
        .build();
    let speed_adj = Adjustment::new(
        speed_to_slider_idx(input_cfg.borrow().mouse_speed),
        0.0,
        9.0,
        1.0,
        1.0,
        0.0,
    );
    let speed_scale = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&speed_adj)
        .draw_value(false)
        .round_digits(0)
        .css_classes(["bat-limit-slider"])
        .hexpand(true)
        .build();
    for i in 0..=9 {
        speed_scale.add_mark(i as f64, gtk4::PositionType::Bottom, None);
    }
    let speed_marks = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();
    speed_marks.append(
        &Label::builder()
            .label("Lenta")
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["card-section-sub"])
            .build(),
    );
    speed_marks.append(
        &Label::builder()
            .label("Rápida")
            .halign(Align::End)
            .css_classes(["card-section-sub"])
            .build(),
    );
    speed_ctrl.append(&speed_scale);
    speed_ctrl.append(&speed_marks);
    speed_row.append(&speed_ctrl);
    point_card_box.append(&speed_row);
    point_card_box.append(&make_row_divider());

    // Desplazamiento natural (EN VIVO -> libinput natural scroll + GSettings)
    let (nat_scroll_row, nat_scroll_sw) = make_switch_row(
        "Desplazamiento natural",
        Some("El contenido sigue el movimiento del dedo"),
        input_cfg.borrow().mouse_natural_scrolling,
        false,
    );
    point_card_box.append(&nat_scroll_row);
    point_card_box.append(&make_row_divider());

    // Clic secundario (EN VIVO -> libinput left-handed + GSettings)
    let (sec_click_row, sec_click_dd) = make_dropdown_row(
        "Clic secundario",
        SECONDARY_CLICK_OPTIONS,
        secondary_click_to_idx(&input_cfg.borrow().mouse_secondary_click),
        false,
    );
    point_card_box.append(&sec_click_row);
    point_card_box.append(&make_row_divider());

    // Zoom inteligente — Magic Mouse -> por implementar
    let (smart_zoom_row, smart_zoom_sw) = make_switch_row(
        "Zoom inteligente",
        Some("Presionar dos veces con 1 dedo"),
        mouse_prefs.borrow().smart_zoom,
        true,
    );
    point_card_box.append(&smart_zoom_row);

    let point_card = wrap_squircle_card(&point_card_box, WIFI_CARD_SQUIRCLE_RADIUS);
    tab_stack.add_named(&point_card, Some("point"));

    // =========================================================================
    // Pestaña 2: Más gestos
    // =========================================================================
    let gestures_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    // Cambiar de página — por implementar
    let (pages_row, pages_dd) = make_dropdown_row(
        "Cambiar de página",
        SWIPE_PAGES_OPTIONS,
        mouse_prefs.borrow().swipe_pages_idx as u32,
        true,
    );
    gestures_card_box.append(&pages_row);
    gestures_card_box.append(&make_row_divider());

    // Cambiar entre apps en pantalla completa — por implementar
    let (fs_apps_row, fs_apps_sw) = make_switch_row(
        "Cambiar entre apps en pantalla completa",
        Some("Ir a la izquierda o la derecha con 2 dedos"),
        mouse_prefs.borrow().swipe_fullscreen_apps,
        true,
    );
    gestures_card_box.append(&fs_apps_row);
    gestures_card_box.append(&make_row_divider());

    // Mission Control (presionar dos veces con 2 dedos en Magic Mouse) — por implementar
    let (mc_row, mc_sw) = make_switch_row(
        "Mission Control",
        Some("Presionar dos veces con 2 dedos"),
        mouse_prefs.borrow().mission_control_double_tap,
        true,
    );
    gestures_card_box.append(&mc_row);

    let gestures_card = wrap_squircle_card(&gestures_card_box, WIFI_CARD_SQUIRCLE_RADIUS);
    tab_stack.add_named(&gestures_card, Some("gestures"));

    tab_stack.set_visible_child_name("point");
    container.append(&tab_stack);

    // 4. Pie de página: «Avanzado...» + «Configurar mouse Bluetooth...» + botón «?»
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (adv_wrap, adv_btn) = make_squircle_button("Avanzado...", false, None);
    let (bt_wrap, bt_btn) = make_squircle_button("Configurar mouse Bluetooth...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Mouse");
    let help_pop = Popover::builder().has_arrow(true).build();
    let help_lbl = Label::builder()
        .label(
            "Ajusta la velocidad del cursor, el desplazamiento natural, el clic secundario (diestro/zurdo) y la aceleración del puntero.\n\nLos controles compatibles con libinput se aplican en tiempo real en Malus DE.",
        )
        .wrap(true)
        .max_width_chars(38)
        .xalign(0.0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    help_pop.set_child(Some(&help_lbl));
    help_btn.set_popover(Some(&help_pop));

    footer.append(&adv_wrap);
    footer.append(&bt_wrap);
    footer.append(&help_wrap);
    container.append(&footer);

    // =========================================================================
    // Navegación entre las 2 pestañas
    // =========================================================================
    let select_tab = {
        let stack_c = tab_stack.clone();
        let b0 = tab_point_btn.clone();
        let b1 = tab_gestures_btn.clone();
        let tab_c = active_tab.clone();
        let hero_c = preview_hero.clone();
        Rc::new(move |idx: u8| {
            tab_c.set(idx);
            stack_c.set_visible_child_name(if idx == 1 { "gestures" } else { "point" });
            if idx == 0 {
                b0.add_css_class("bat-seg-tab-active");
                b1.remove_css_class("bat-seg-tab-active");
            } else {
                b1.add_css_class("bat-seg-tab-active");
                b0.remove_css_class("bat-seg-tab-active");
            }
            hero_c.queue_draw();
        })
    };

    {
        let st = select_tab.clone();
        tab_point_btn.connect_clicked(move |_| st(0));
    }
    {
        let st = select_tab.clone();
        tab_gestures_btn.connect_clicked(move |_| st(1));
    }

    // =========================================================================
    // Señales en vivo (InputConfig + GSettings + IPC ReloadInputConfig)
    // =========================================================================
    let save_input_and_apply = {
        let icfg_c = input_cfg.clone();
        Rc::new(move || {
            let cfg = icfg_c.borrow().clone();
            let _ = cfg.save();
            sync_gsettings_mouse(&cfg);
            notify_compositor_input_reload();
        })
    };

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        speed_scale.connect_value_changed(move |sc| {
            if sync_c.get() {
                return;
            }
            let idx = sc.value().round().clamp(0.0, 9.0) as usize;
            icfg_c.borrow_mut().mouse_speed = slider_idx_to_speed(idx);
            apply_c();
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        nat_scroll_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().mouse_natural_scrolling = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        let sec_cell = secondary_mode_cell.clone();
        let hero_c = preview_hero.clone();
        sec_click_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let idx = dd.selected();
            sec_cell.set(idx as u8);
            icfg_c.borrow_mut().mouse_secondary_click = idx_to_secondary_click(idx).to_string();
            apply_c();
            hero_c.queue_draw();
        });
    }

    // =========================================================================
    // Señales persistentes (MousePrefsConfig — controles «por implementar»)
    // =========================================================================
    {
        let mp_c = mouse_prefs.clone();
        let sync_c = syncing.clone();
        smart_zoom_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                mp_c.borrow_mut().smart_zoom = active;
                let _ = mp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let mp_c = mouse_prefs.clone();
        let sync_c = syncing.clone();
        pages_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            mp_c.borrow_mut().swipe_pages_idx = dd.selected().min(3) as u8;
            let _ = mp_c.borrow().save();
        });
    }
    {
        let mp_c = mouse_prefs.clone();
        let sync_c = syncing.clone();
        fs_apps_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                mp_c.borrow_mut().swipe_fullscreen_apps = active;
                let _ = mp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let mp_c = mouse_prefs.clone();
        let sync_c = syncing.clone();
        mc_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                mp_c.borrow_mut().mission_control_double_tap = active;
                let _ = mp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }

    // Botones del pie: Avanzado... y Configurar mouse Bluetooth...
    {
        let parent_c = parent.clone();
        let icfg_c = input_cfg.clone();
        let apply_c = save_input_and_apply.clone();
        adv_btn.connect_clicked(move |_| {
            open_advanced_mouse_modal(&parent_c, icfg_c.clone(), apply_c.clone());
        });
    }
    bt_btn.connect_clicked(|_| {
        crate::navigate_to_panel("bluetooth");
    });

    // Refresher para recargar estado + detectar batería de mouse inalámbrico
    let refresh: Rc<dyn Fn()> = {
        let icfg_c = input_cfg.clone();
        let mp_c = mouse_prefs.clone();
        let sync_c = syncing.clone();
        let speed_sc = speed_scale.clone();
        let nat_s = nat_scroll_sw.clone();
        let sec_d = sec_click_dd.clone();
        let sec_cell = secondary_mode_cell.clone();
        let smart_s = smart_zoom_sw.clone();
        let pages_d = pages_dd.clone();
        let fs_s = fs_apps_sw.clone();
        let mc_s = mc_sw.clone();
        let hero_c = preview_hero.clone();
        let bat_row_c = battery_row.clone();
        let bat_lbl_c = battery_lbl.clone();
        Rc::new(move || {
            let ic = InputConfig::load();
            let mp = MousePrefsConfig::load();
            let sec_idx = secondary_click_to_idx(&ic.mouse_secondary_click);
            sync_c.set(true);
            speed_sc.set_value(speed_to_slider_idx(ic.mouse_speed));
            nat_s.set_active(ic.mouse_natural_scrolling);
            sec_d.set_selected(sec_idx);
            sec_cell.set(sec_idx as u8);
            smart_s.set_active(mp.smart_zoom);
            pages_d.set_selected((mp.swipe_pages_idx as u32).min(3));
            fs_s.set_active(mp.swipe_fullscreen_apps);
            mc_s.set_active(mp.mission_control_double_tap);
            *icfg_c.borrow_mut() = ic;
            *mp_c.borrow_mut() = mp;
            sync_c.set(false);
            hero_c.queue_draw();

            let row_b = bat_row_c.clone();
            let lbl_b = bat_lbl_c.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(res) = gtk4::gio::spawn_blocking(probe_wireless_mouse_battery).await else {
                    return;
                };
                if let Some((model, pct)) = res {
                    lbl_b.set_label(&format!("{model} · {pct}%"));
                    row_b.set_visible(true);
                } else {
                    row_b.set_visible(false);
                }
            });
        })
    };

    refresh();

    (container, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_speed_slider_mapping_is_monotonic_and_roundtrips() {
        for idx in 0..MOUSE_CURSOR_SPEEDS.len() {
            let speed = slider_idx_to_speed(idx);
            assert_eq!(speed_to_slider_idx(speed) as usize, idx);
            if idx > 0 {
                assert!(speed > slider_idx_to_speed(idx - 1));
            }
        }
    }

    #[test]
    fn mouse_secondary_click_roundtrips() {
        for (mode, expected_idx) in [("off", 0u32), ("right", 1), ("left", 2)] {
            assert_eq!(secondary_click_to_idx(mode), expected_idx);
            assert_eq!(idx_to_secondary_click(expected_idx), mode);
        }
    }
}
