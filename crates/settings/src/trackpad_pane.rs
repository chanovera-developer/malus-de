use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, ApplicationWindow, Box as GtkBox, Button, DrawingArea, DropDown, Label,
    Orientation, Popover, Scale, Stack, StackTransitionType, StringList, Window,
};
use malus_de_common::config::{InputConfig, TrackpadPrefsConfig};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const TRACKPAD_PANEL_ID: &str = "trackpad";
pub const TRACKPAD_PANEL_TITLE: &str = "Trackpad";

/// Mapeo de los 10 pasos del deslizador «Velocidad del cursor» (0..=9) a velocidad libinput (-1.0..=1.0).
const CURSOR_SPEEDS: [f64; 10] = [-0.75, -0.55, -0.35, -0.18, 0.0, 0.16, 0.34, 0.52, 0.70, 0.88];

const LOOKUP_OPTIONS: &[&str] = &[
    "Desactivado",
    "Hacer clic fuerte con 1 dedo",
    "Presionar con 3 dedos",
];

const SECONDARY_CLICK_OPTIONS: &[&str] = &[
    "Desactivado",
    "Hacer clic con 2 dedos",
    "Hacer clic en esquina inferior derecha",
    "Hacer clic en esquina inferior izquierda",
];

const SWIPE_PAGES_OPTIONS: &[&str] = &[
    "Desactivado",
    "Ir a la izquierda o la derecha con 2 dedos",
    "Deslizar con 3 dedos",
    "Deslizar con 2 o 3 dedos",
];

const SWIPE_SPACES_OPTIONS: &[&str] = &[
    "Desactivado",
    "Ir a la izquierda o la derecha con 3 dedos",
    "Ir a la izquierda o la derecha con 4 dedos",
];

const MISSION_CONTROL_OPTIONS: &[&str] = &[
    "Desactivado",
    "Ir hacia arriba con 3 dedos",
    "Ir hacia arriba con 4 dedos",
];

const APP_EXPOSE_OPTIONS: &[&str] = &[
    "Desactivado",
    "Ir hacia abajo con 3 dedos",
    "Ir hacia abajo con 4 dedos",
];

fn speed_to_slider_idx(speed: f64) -> f64 {
    CURSOR_SPEEDS
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
    CURSOR_SPEEDS[idx.min(CURSOR_SPEEDS.len() - 1)]
}

fn secondary_click_to_idx(mode: &str) -> u32 {
    match mode {
        "off" => 0,
        "bottom_right" => 2,
        "bottom_left" => 3,
        _ => 1,
    }
}

fn idx_to_secondary_click(idx: u32) -> &'static str {
    match idx {
        0 => "off",
        2 => "bottom_right",
        3 => "bottom_left",
        _ => "two_fingers",
    }
}

fn fingers_to_dropdown_idx(fingers: u8) -> u32 {
    match fingers {
        3 => 1,
        4 => 2,
        _ => 0,
    }
}

fn dropdown_idx_to_fingers(idx: u32) -> u8 {
    match idx {
        1 => 3,
        2 => 4,
        _ => 0,
    }
}

fn notify_compositor_input_reload() {
    std::thread::spawn(|| {
        let _ = IpcClient::send_command(IpcMessage::ReloadInputConfig);
    });
}

fn sync_gsettings_touchpad(cfg: &InputConfig) {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return;
    };
    let schema = "org.gnome.desktop.peripherals.touchpad";
    if source.lookup(schema, true).is_none() {
        return;
    }
    let gs = gtk4::gio::Settings::new(schema);
    let _ = gs.set_double("speed", cfg.trackpad_speed.clamp(-1.0, 1.0));
    let _ = gs.set_boolean("tap-to-click", cfg.trackpad_tap_to_click);
    let _ = gs.set_boolean("disable-while-typing", cfg.trackpad_disable_while_typing);
    let _ = gs.set_boolean("natural-scroll", cfg.natural_scrolling);
    let click_method = match cfg.trackpad_secondary_click.as_str() {
        "bottom_right" | "bottom_left" => "areas",
        "off" => "none",
        _ => "fingers",
    };
    let _ = gs.set_string("click-method", click_method);
}

fn probe_bluetooth_devices() -> Vec<String> {
    let Ok(out) = Command::new("bluetoothctl").arg("devices").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let mut parts = rest.splitn(2, ' ');
            let mac = parts.next()?.trim();
            let name = parts.next().unwrap_or(mac).trim();
            if name.is_empty() {
                None
            } else {
                Some(format!("{name}  ({mac})"))
            }
        })
        .collect()
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

/// Dibuja la vista previa dual superior estilo macOS Tahoe (superficie del trackpad + escritorio miniatura).
fn build_trackpad_preview_hero(active_tab: Rc<Cell<u8>>) -> DrawingArea {
    let area = DrawingArea::builder()
        .content_height(126)
        .hexpand(true)
        .vexpand(false)
        .build();

    let tab_draw = active_tab.clone();
    area.set_draw_func(move |widget, cr, width, height| {
        let w = width as f64;
        let h = height as f64;
        let tab = tab_draw.get();
        let (ar, ag, ab) = crate::accent_color();
        let fg = widget.color();
        let is_dark = (0.299 * fg.red() + 0.587 * fg.green() + 0.114 * fg.blue()) > 0.5;

        let card_gap = 12.0;
        let card_w = ((w - card_gap).min(410.0) / 2.0).max(140.0);
        let card_h = (h - 8.0).min(118.0).max(96.0);
        let total_w = card_w * 2.0 + card_gap;
        let start_x = ((w - total_w) * 0.5).max(0.0);
        let top_y = ((h - card_h) * 0.5).max(0.0);

        // --- Tarjeta izquierda: superficie de Trackpad minimalista ---
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

        // Silueta interior del trackpad físico
        let pad_w = card_w * 0.48;
        let pad_h = card_h * 0.54;
        let pad_x = start_x + (card_w - pad_w) * 0.5;
        let pad_y = top_y + (card_h - pad_h) * 0.5;
        rounded_rect_path(cr, pad_x, pad_y, pad_w, pad_h, 9.0);
        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.04);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.72);
        }
        let _ = cr.fill_preserve();
        if is_dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.30);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
        }
        cr.set_line_width(1.4);
        let _ = cr.stroke();

        // Puntos de dedos según la pestaña activa
        let draw_finger = |cx: f64, cy: f64, radius: f64, alpha: f64| {
            cr.set_source_rgba(ar, ag, ab, alpha * 0.22);
            cr.arc(cx, cy, radius * 1.65, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.set_source_rgba(ar, ag, ab, alpha);
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        };

        match tab {
            0 => {
                // Señalamiento y clic: 1 dedo en zona superior central
                draw_finger(pad_x + pad_w * 0.50, pad_y + pad_h * 0.34, 4.4, 0.92);
            }
            1 => {
                // Desplazamiento y zoom: 2 dedos en gesto de desplazamiento / pellizco
                draw_finger(pad_x + pad_w * 0.38, pad_y + pad_h * 0.56, 4.1, 0.90);
                draw_finger(pad_x + pad_w * 0.62, pad_y + pad_h * 0.36, 4.1, 0.90);
            }
            _ => {
                // Más gestos: 2-3 dedos desplazando hacia la derecha
                draw_finger(pad_x + pad_w * 0.58, pad_y + pad_h * 0.34, 4.0, 0.88);
                draw_finger(pad_x + pad_w * 0.74, pad_y + pad_h * 0.34, 4.0, 0.88);
            }
        }

        // --- Tarjeta derecha: escritorio miniatura estilo macOS Tahoe ---
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

        // Ventana miniatura en el centro
        let win_w = card_w * 0.72;
        let win_h = card_h * 0.68;
        let win_x = rx + (card_w - win_w) * 0.5;
        let win_y = top_y + 13.0;
        rounded_rect_path(cr, win_x, win_y, win_w, win_h, 6.0);
        cr.set_source_rgba(0.95, 0.97, 1.0, 0.92);
        let _ = cr.fill();

        // Barra de título y botones de ventana
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

        // Líneas de contenido simulado
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

        // Tres tarjetas con figuras geométricas (círculo morado, cuadrado azul, triángulo naranja)
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

        // Indicador sutil por pestaña en la ventana miniatura
        if tab == 0 {
            // Cursor pequeño apuntando al texto
            let cx = win_x + win_w * 0.62;
            let cy = content_y + 6.0;
            cr.move_to(cx, cy);
            cr.line_to(cx, cy + 9.0);
            cr.line_to(cx + 2.8, cy + 6.6);
            cr.line_to(cx + 5.8, cy + 6.8);
            cr.close_path();
            cr.set_source_rgb(0.10, 0.10, 0.12);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(0.8);
            let _ = cr.stroke();
        } else if tab == 1 {
            // Marco suave de zoom sobre las figuras
            rounded_rect_path(
                cr,
                bx0 - 3.0,
                box_y - 3.0,
                boxes_w + 6.0,
                box_sz + 6.0,
                5.0,
            );
            cr.set_source_rgba(ar, ag, ab, 0.55);
            cr.set_line_width(1.2);
            let _ = cr.stroke();
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
        let row_w = (dock_colors.len() as f64) * icon_sz + ((dock_colors.len() - 1) as f64) * icon_gap;
        let ix0 = dock_x + (dock_w - row_w) * 0.5;
        let iy = dock_y + (dock_h - icon_sz) * 0.5;
        for (i, (r, g, b)) in dock_colors.iter().enumerate() {
            let ix = ix0 + (i as f64) * (icon_sz + icon_gap);
            rounded_rect_path(cr, ix, iy, icon_sz, icon_sz, 1.8);
            cr.set_source_rgb(*r, *g, *b);
            let _ = cr.fill();
        }

        cr.restore().ok();

        // Borde sutil de la tarjeta derecha
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

fn open_bluetooth_trackpad_modal(parent: &ApplicationWindow) {
    let dialog = Window::builder()
        .title("Configurar trackpad Bluetooth")
        .transient_for(parent)
        .modal(true)
        .default_width(430)
        .resizable(false)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .margin_top(20)
        .margin_bottom(18)
        .margin_start(22)
        .margin_end(22)
        .build();

    let title = Label::builder()
        .label("Configurar trackpad Bluetooth")
        .halign(Align::Start)
        .css_classes(["title-3"])
        .build();
    root.append(&title);

    let desc = Label::builder()
        .label(
            "Enciende tu trackpad inalámbrico en modo de emparejamiento y selecciónalo en los ajustes de Bluetooth de Malus DE.",
        )
        .halign(Align::Start)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["card-section-sub"])
        .build();
    root.append(&desc);

    let list_card_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();

    let status_lbl = Label::builder()
        .label("Buscando dispositivos Bluetooth conocidos…")
        .halign(Align::Start)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["card-section-sub"])
        .build();
    list_card_inner.append(&status_lbl);

    let list_card = wrap_squircle_card(&list_card_inner, WIFI_CARD_SQUIRCLE_RADIUS);
    root.append(&list_card);

    let btn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::End)
        .margin_top(4)
        .build();

    let (close_wrap, close_btn) = make_squircle_button("Cancelar", false, None);
    let open_bt_btn = Button::builder()
        .label("Abrir Bluetooth…")
        .css_classes(["suggested-action"])
        .build();
    btn_row.append(&close_wrap);
    btn_row.append(&open_bt_btn);
    root.append(&btn_row);

    dialog.set_child(Some(&root));

    let dlg_c = dialog.clone();
    close_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_c);
    });

    let dlg_bt = dialog.clone();
    open_bt_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_bt);
        crate::navigate_to_panel("bluetooth");
    });

    let inner_c = list_card_inner.clone();
    let status_c = status_lbl.clone();
    glib::MainContext::default().spawn_local(async move {
        let Ok(devices) = gtk4::gio::spawn_blocking(probe_bluetooth_devices).await else {
            return;
        };
        if devices.is_empty() {
            status_c.set_label(
                "No se encontraron dispositivos Bluetooth emparejados aún. Abre Bluetooth para emparejar un trackpad.",
            );
            return;
        }
        status_c.set_label("Dispositivos Bluetooth conocidos:");
        for dev in devices.into_iter().take(5) {
            let item = Label::builder()
                .label(&format!("• {dev}"))
                .halign(Align::Start)
                .wrap(true)
                .xalign(0.0)
                .css_classes(["wifi-row-title"])
                .build();
            inner_c.append(&item);
        }
    });

    crate::present_attached_modal(&dialog, parent);
}

pub fn create_trackpad_pane(parent: &ApplicationWindow) -> (GtkBox, Rc<dyn Fn()>) {
    let input_cfg = Rc::new(RefCell::new(InputConfig::load()));
    let tp_prefs = Rc::new(RefCell::new(TrackpadPrefsConfig::load()));
    let syncing = Rc::new(Cell::new(false));
    let active_tab = Rc::new(Cell::new(0u8));

    let container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    // 1. Vista previa interactiva superior (trackpad + miniatura del escritorio)
    let preview_hero = build_trackpad_preview_hero(active_tab.clone());
    container.append(&preview_hero);

    // 2. Barra segmentada de 3 pestañas estilo macOS Tahoe
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
    let tab_scroll_btn = Button::builder()
        .label("Desplazamiento y zoom")
        .css_classes(["bat-seg-tab"])
        .hexpand(true)
        .build();
    let tab_gestures_btn = Button::builder()
        .label("Más gestos")
        .css_classes(["bat-seg-tab"])
        .hexpand(true)
        .build();

    seg_track.append(&tab_point_btn);
    seg_track.append(&tab_scroll_btn);
    seg_track.append(&tab_gestures_btn);
    container.append(&seg_track);

    // 3. Stack de las 3 pestañas
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
        speed_to_slider_idx(input_cfg.borrow().trackpad_speed),
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

    // Clic (Ligero / Medio / Firme) — Force Touch hardware -> por implementar
    let click_row = make_card_row();
    click_row.append(&make_title_col("Clic", None, true));
    let click_ctrl = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .valign(Align::Center)
        .width_request(210)
        .build();
    let click_adj = Adjustment::new(
        tp_prefs.borrow().click_firmness_idx.min(2) as f64,
        0.0,
        2.0,
        1.0,
        1.0,
        0.0,
    );
    let click_scale = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&click_adj)
        .draw_value(false)
        .round_digits(0)
        .css_classes(["bat-limit-slider"])
        .hexpand(true)
        .build();
    for i in 0..=2 {
        click_scale.add_mark(i as f64, gtk4::PositionType::Bottom, None);
    }
    let click_marks = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();
    click_marks.append(
        &Label::builder()
            .label("Ligero")
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["card-section-sub"])
            .build(),
    );
    click_marks.append(
        &Label::builder()
            .label("Medio")
            .halign(Align::Center)
            .hexpand(true)
            .css_classes(["card-section-sub"])
            .build(),
    );
    click_marks.append(
        &Label::builder()
            .label("Firme")
            .halign(Align::End)
            .hexpand(true)
            .css_classes(["card-section-sub"])
            .build(),
    );
    click_ctrl.append(&click_scale);
    click_ctrl.append(&click_marks);
    click_row.append(&click_ctrl);
    point_card_box.append(&click_row);
    point_card_box.append(&make_row_divider());

    // Clic silencioso — por implementar
    let (silent_row, silent_sw) = make_switch_row(
        "Clic silencioso",
        None,
        tp_prefs.borrow().silent_click,
        true,
    );
    point_card_box.append(&silent_row);
    point_card_box.append(&make_row_divider());

    // Clic fuerte y vibración — por implementar
    let (force_row, force_sw) = make_switch_row(
        "Clic fuerte y vibración",
        Some(
            "Haz clic y luego presiona con fuerza para usar Vista Rápida, Consultar y los controles multimedia de velocidad variable.",
        ),
        tp_prefs.borrow().force_click_haptic,
        true,
    );
    point_card_box.append(&force_row);
    point_card_box.append(&make_row_divider());

    // Consultar y detectores de datos — por implementar
    let (lookup_row, lookup_dd) = make_dropdown_row(
        "Consultar y detectores de datos",
        LOOKUP_OPTIONS,
        tp_prefs.borrow().lookup_data_detectors_idx as u32,
        true,
    );
    point_card_box.append(&lookup_row);
    point_card_box.append(&make_row_divider());

    // Clic secundario (EN VIVO -> libinput ClickMethod + GSettings)
    let (sec_click_row, sec_click_dd) = make_dropdown_row(
        "Clic secundario",
        SECONDARY_CLICK_OPTIONS,
        secondary_click_to_idx(&input_cfg.borrow().trackpad_secondary_click),
        false,
    );
    point_card_box.append(&sec_click_row);
    point_card_box.append(&make_row_divider());

    // Presionar para hacer clic (EN VIVO -> libinput tap-to-click + GSettings)
    let (tap_row, tap_sw) = make_switch_row(
        "Presionar para hacer clic",
        Some("Presionar con 1 dedo"),
        input_cfg.borrow().trackpad_tap_to_click,
        false,
    );
    point_card_box.append(&tap_row);
    point_card_box.append(&make_row_divider());

    // Desactivar al escribir (EN VIVO -> libinput DWT + GSettings)
    let (dwt_row, dwt_sw) = make_switch_row(
        "Desactivar al escribir",
        Some("Ignora toques accidentales de la palma mientras escribes"),
        input_cfg.borrow().trackpad_disable_while_typing,
        false,
    );
    point_card_box.append(&dwt_row);

    let point_card = wrap_squircle_card(&point_card_box, WIFI_CARD_SQUIRCLE_RADIUS);
    tab_stack.add_named(&point_card, Some("point"));

    // =========================================================================
    // Pestaña 2: Desplazamiento y zoom
    // =========================================================================
    let scroll_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    // Desplazamiento natural (EN VIVO -> libinput natural scroll + GSettings)
    let (nat_scroll_row, nat_scroll_sw) = make_switch_row(
        "Desplazamiento natural",
        Some("El contenido sigue el movimiento del dedo"),
        input_cfg.borrow().natural_scrolling,
        false,
    );
    scroll_card_box.append(&nat_scroll_row);
    scroll_card_box.append(&make_row_divider());

    // Aumentar o reducir el zoom (EN VIVO -> compositor GesturePinch 2 dedos)
    let (pinch_zoom_row, pinch_zoom_sw) = make_switch_row(
        "Aumentar o reducir el zoom",
        Some("Separar o juntar 2 dedos"),
        input_cfg.borrow().trackpad_pinch_zoom,
        false,
    );
    scroll_card_box.append(&pinch_zoom_row);
    scroll_card_box.append(&make_row_divider());

    // Zoom inteligente — por implementar
    let (smart_zoom_row, smart_zoom_sw) = make_switch_row(
        "Zoom inteligente",
        Some("Presionar dos veces con 2 dedos"),
        tp_prefs.borrow().smart_zoom,
        true,
    );
    scroll_card_box.append(&smart_zoom_row);
    scroll_card_box.append(&make_row_divider());

    // Girar (EN VIVO -> compositor GesturePinch rotación con 2 dedos)
    let (rotate_row, rotate_sw) = make_switch_row(
        "Girar",
        Some("Girar con 2 dedos"),
        input_cfg.borrow().trackpad_rotate_gesture,
        false,
    );
    scroll_card_box.append(&rotate_row);

    let scroll_card = wrap_squircle_card(&scroll_card_box, WIFI_CARD_SQUIRCLE_RADIUS);
    tab_stack.add_named(&scroll_card, Some("scroll"));

    // =========================================================================
    // Pestaña 3: Más gestos
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
        tp_prefs.borrow().swipe_pages_idx as u32,
        true,
    );
    gestures_card_box.append(&pages_row);
    gestures_card_box.append(&make_row_divider());

    // Cambiar de espacio (EN VIVO -> compositor swipe horizontal 3/4 dedos)
    let (spaces_row, spaces_dd) = make_dropdown_row(
        "Cambiar de espacio",
        SWIPE_SPACES_OPTIONS,
        fingers_to_dropdown_idx(input_cfg.borrow().trackpad_swipe_spaces_fingers),
        false,
    );
    gestures_card_box.append(&spaces_row);
    gestures_card_box.append(&make_row_divider());

    // Mission Control (EN VIVO -> compositor swipe arriba 3/4 dedos)
    let (mc_row, mc_dd) = make_dropdown_row(
        "Mission Control",
        MISSION_CONTROL_OPTIONS,
        fingers_to_dropdown_idx(input_cfg.borrow().trackpad_mission_control_fingers),
        false,
    );
    gestures_card_box.append(&mc_row);
    gestures_card_box.append(&make_row_divider());

    // Exposé (EN VIVO -> compositor swipe abajo 3/4 dedos)
    let (expose_row, expose_dd) = make_dropdown_row(
        "Exposé",
        APP_EXPOSE_OPTIONS,
        fingers_to_dropdown_idx(input_cfg.borrow().trackpad_app_expose_fingers),
        false,
    );
    gestures_card_box.append(&expose_row);
    gestures_card_box.append(&make_row_divider());

    // Centro de notificaciones — por implementar
    let (notif_row, notif_sw) = make_switch_row(
        "Centro de notificaciones",
        Some("Ir a la izquierda desde el borde derecho con 2 dedos"),
        tp_prefs.borrow().notification_center_gesture,
        true,
    );
    gestures_card_box.append(&notif_row);
    gestures_card_box.append(&make_row_divider());

    // Apps (EN VIVO -> compositor 4 dedos pellizco hacia adentro -> Canopy/Apps)
    let (apps_row, apps_sw) = make_switch_row(
        "Apps",
        Some("Juntar el pulgar con 3 dedos"),
        input_cfg.borrow().trackpad_apps_gesture,
        false,
    );
    gestures_card_box.append(&apps_row);
    gestures_card_box.append(&make_row_divider());

    // Mostrar escritorio (EN VIVO -> compositor 4 dedos pellizco hacia afuera -> toggle_show_desktop)
    let (desktop_row, desktop_sw) = make_switch_row(
        "Mostrar escritorio",
        Some("Separar el pulgar y 3 dedos"),
        input_cfg.borrow().show_desktop_gesture,
        false,
    );
    gestures_card_box.append(&desktop_row);

    let gestures_card = wrap_squircle_card(&gestures_card_box, WIFI_CARD_SQUIRCLE_RADIUS);
    tab_stack.add_named(&gestures_card, Some("gestures"));

    tab_stack.set_visible_child_name("point");
    container.append(&tab_stack);

    // 4. Pie de página: «Configurar trackpad Bluetooth...» + botón «?»
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (bt_wrap, bt_btn) =
        make_squircle_button("Configurar trackpad Bluetooth...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Trackpad");
    let help_pop = Popover::builder().has_arrow(true).build();
    let help_lbl = Label::builder()
        .label(
            "Ajusta la velocidad del cursor, el clic secundario, el desplazamiento natural y los gestos multitáctiles del trackpad.\n\nLos cambios compatibles con libinput y el compositor de Malus DE se aplican en tiempo real.",
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

    footer.append(&bt_wrap);
    footer.append(&help_wrap);
    container.append(&footer);

    // =========================================================================
    // Navegación entre las 3 pestañas
    // =========================================================================
    let select_tab = {
        let stack_c = tab_stack.clone();
        let b0 = tab_point_btn.clone();
        let b1 = tab_scroll_btn.clone();
        let b2 = tab_gestures_btn.clone();
        let tab_c = active_tab.clone();
        let hero_c = preview_hero.clone();
        Rc::new(move |idx: u8| {
            tab_c.set(idx);
            let name = match idx {
                1 => "scroll",
                2 => "gestures",
                _ => "point",
            };
            stack_c.set_visible_child_name(name);
            for (i, b) in [(&b0, 0u8), (&b1, 1u8), (&b2, 2u8)] {
                if b == idx {
                    i.add_css_class("bat-seg-tab-active");
                } else {
                    i.remove_css_class("bat-seg-tab-active");
                }
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
        tab_scroll_btn.connect_clicked(move |_| st(1));
    }
    {
        let st = select_tab.clone();
        tab_gestures_btn.connect_clicked(move |_| st(2));
    }

    // =========================================================================
    // Señales en vivo (InputConfig + GSettings + IPC ReloadInputConfig)
    // =========================================================================
    let save_input_and_apply = {
        let icfg_c = input_cfg.clone();
        Rc::new(move || {
            let cfg = icfg_c.borrow().clone();
            let _ = cfg.save();
            sync_gsettings_touchpad(&cfg);
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
            icfg_c.borrow_mut().trackpad_speed = slider_idx_to_speed(idx);
            apply_c();
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        sec_click_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            icfg_c.borrow_mut().trackpad_secondary_click =
                idx_to_secondary_click(dd.selected()).to_string();
            apply_c();
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        tap_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().trackpad_tap_to_click = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        dwt_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().trackpad_disable_while_typing = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        nat_scroll_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().natural_scrolling = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        pinch_zoom_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().trackpad_pinch_zoom = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        rotate_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().trackpad_rotate_gesture = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        spaces_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            icfg_c.borrow_mut().trackpad_swipe_spaces_fingers =
                dropdown_idx_to_fingers(dd.selected());
            apply_c();
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        mc_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            icfg_c.borrow_mut().trackpad_mission_control_fingers =
                dropdown_idx_to_fingers(dd.selected());
            apply_c();
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        expose_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            icfg_c.borrow_mut().trackpad_app_expose_fingers =
                dropdown_idx_to_fingers(dd.selected());
            apply_c();
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        apps_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().trackpad_apps_gesture = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    {
        let icfg_c = input_cfg.clone();
        let sync_c = syncing.clone();
        let apply_c = save_input_and_apply.clone();
        desktop_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                icfg_c.borrow_mut().show_desktop_gesture = active;
                apply_c();
            }
            glib::Propagation::Proceed
        });
    }

    // =========================================================================
    // Señales persistentes (TrackpadPrefsConfig — controles «por implementar»)
    // =========================================================================
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        click_scale.connect_value_changed(move |sc| {
            if sync_c.get() {
                return;
            }
            tp_c.borrow_mut().click_firmness_idx = sc.value().round().clamp(0.0, 2.0) as u8;
            let _ = tp_c.borrow().save();
        });
    }
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        silent_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                tp_c.borrow_mut().silent_click = active;
                let _ = tp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        force_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                tp_c.borrow_mut().force_click_haptic = active;
                let _ = tp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        lookup_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            tp_c.borrow_mut().lookup_data_detectors_idx = dd.selected().min(2) as u8;
            let _ = tp_c.borrow().save();
        });
    }
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        smart_zoom_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                tp_c.borrow_mut().smart_zoom = active;
                let _ = tp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        pages_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            tp_c.borrow_mut().swipe_pages_idx = dd.selected().min(3) as u8;
            let _ = tp_c.borrow().save();
        });
    }
    {
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        notif_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                tp_c.borrow_mut().notification_center_gesture = active;
                let _ = tp_c.borrow().save();
            }
            glib::Propagation::Proceed
        });
    }

    // Botón «Configurar trackpad Bluetooth...»
    {
        let parent_c = parent.clone();
        bt_btn.connect_clicked(move |_| {
            open_bluetooth_trackpad_modal(&parent_c);
        });
    }

    // Refresher para recargar el estado desde disco al volver al panel
    let refresh: Rc<dyn Fn()> = {
        let icfg_c = input_cfg.clone();
        let tp_c = tp_prefs.clone();
        let sync_c = syncing.clone();
        let speed_sc = speed_scale.clone();
        let click_sc = click_scale.clone();
        let silent_s = silent_sw.clone();
        let force_s = force_sw.clone();
        let lookup_d = lookup_dd.clone();
        let sec_d = sec_click_dd.clone();
        let tap_s = tap_sw.clone();
        let dwt_s = dwt_sw.clone();
        let nat_s = nat_scroll_sw.clone();
        let zoom_s = pinch_zoom_sw.clone();
        let smart_s = smart_zoom_sw.clone();
        let rot_s = rotate_sw.clone();
        let pages_d = pages_dd.clone();
        let spaces_d = spaces_dd.clone();
        let mc_d = mc_dd.clone();
        let exp_d = expose_dd.clone();
        let notif_s = notif_sw.clone();
        let apps_s = apps_sw.clone();
        let desk_s = desktop_sw.clone();
        let hero_c = preview_hero.clone();
        Rc::new(move || {
            let ic = InputConfig::load();
            let tp = TrackpadPrefsConfig::load();
            sync_c.set(true);
            speed_sc.set_value(speed_to_slider_idx(ic.trackpad_speed));
            click_sc.set_value(tp.click_firmness_idx.min(2) as f64);
            silent_s.set_active(tp.silent_click);
            force_s.set_active(tp.force_click_haptic);
            lookup_d.set_selected((tp.lookup_data_detectors_idx as u32).min(2));
            sec_d.set_selected(secondary_click_to_idx(&ic.trackpad_secondary_click));
            tap_s.set_active(ic.trackpad_tap_to_click);
            dwt_s.set_active(ic.trackpad_disable_while_typing);
            nat_s.set_active(ic.natural_scrolling);
            zoom_s.set_active(ic.trackpad_pinch_zoom);
            smart_s.set_active(tp.smart_zoom);
            rot_s.set_active(ic.trackpad_rotate_gesture);
            pages_d.set_selected((tp.swipe_pages_idx as u32).min(3));
            spaces_d.set_selected(fingers_to_dropdown_idx(ic.trackpad_swipe_spaces_fingers));
            mc_d.set_selected(fingers_to_dropdown_idx(ic.trackpad_mission_control_fingers));
            exp_d.set_selected(fingers_to_dropdown_idx(ic.trackpad_app_expose_fingers));
            notif_s.set_active(tp.notification_center_gesture);
            apps_s.set_active(ic.trackpad_apps_gesture);
            desk_s.set_active(ic.show_desktop_gesture);
            *icfg_c.borrow_mut() = ic;
            *tp_c.borrow_mut() = tp;
            sync_c.set(false);
            hero_c.queue_draw();
        })
    };

    (container, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trackpad_speed_slider_mapping_is_monotonic_and_roundtrips() {
        for idx in 0..CURSOR_SPEEDS.len() {
            let speed = slider_idx_to_speed(idx);
            assert_eq!(speed_to_slider_idx(speed) as usize, idx);
            if idx > 0 {
                assert!(speed > slider_idx_to_speed(idx - 1));
            }
        }
    }

    #[test]
    fn gesture_fingers_and_secondary_click_roundtrip() {
        for (fingers, expected_idx) in [(0u8, 0u32), (3, 1), (4, 2)] {
            assert_eq!(fingers_to_dropdown_idx(fingers), expected_idx);
            assert_eq!(dropdown_idx_to_fingers(expected_idx), fingers);
        }
        for (mode, expected_idx) in [
            ("off", 0u32),
            ("two_fingers", 1),
            ("bottom_right", 2),
            ("bottom_left", 3),
        ] {
            assert_eq!(secondary_click_to_idx(mode), expected_idx);
            assert_eq!(idx_to_secondary_click(expected_idx), mode);
        }
    }
}
