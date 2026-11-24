use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DrawingArea, Entry, GestureClick,
    GestureDrag, Image, Label, Orientation, Overlay, Popover, PolicyType, Scale, ScrolledWindow,
    Window,
};
use malus_de_common::config::{
    connector_has_hardware_backlight, default_bundled_wallpaper_path, is_dark_mode_active,
    DisplayModeConfig, DisplayOutputConfig, DisplaysConfig, WallpaperConfig, WallpaperMode,
};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::general_pane::make_pending_badge;
use crate::squircle_path;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_button, wrap_squircle_card,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

const COLOR_PROFILE_OPTIONS: [&str; 4] = [
    "LCD color",
    "Monitor integrado",
    "sRGB IEC61966-2.1",
    "Display P3",
];
const ROTATION_OPTIONS: [&str; 4] = ["Estándar", "90°", "180°", "270°"];
const TV_CONNECT_OPTIONS: [&str; 3] = [
    "Preguntar qué mostrar",
    "Duplicar pantalla",
    "Pantalla extendida",
];
const NIGHT_SCHEDULE_OPTIONS: [&str; 3] =
    ["Desactivado", "Anochecer al amanecer", "Personalizado"];

fn save_displays_and_reload(cfg: &DisplaysConfig) {
    let _ = cfg.save();
    let _ = IpcClient::send_command(IpcMessage::ReloadDisplaysConfig);
}

fn read_sysfs_backlight_pct() -> Option<f64> {
    let entries = std::fs::read_dir("/sys/class/backlight").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let cur_path = path.join("brightness");
        let max_path = path.join("max_brightness");
        if let (Ok(cur_str), Ok(max_str)) = (
            std::fs::read_to_string(&cur_path),
            std::fs::read_to_string(&max_path),
        ) {
            if let (Ok(cur), Ok(max)) = (
                cur_str.trim().parse::<f64>(),
                max_str.trim().parse::<f64>(),
            ) {
                if max > 0.0 {
                    return Some((cur / max * 100.0).clamp(5.0, 100.0));
                }
            }
        }
    }
    None
}

fn read_output_brightness(_cfg: &DisplaysConfig, output: &DisplayOutputConfig) -> f64 {
    if connector_has_hardware_backlight(&output.connector) {
        if let Some(hw) = read_sysfs_backlight_pct() {
            return hw;
        }
    }
    output.brightness.clamp(5.0, 100.0)
}

fn apply_output_brightness(output_idx: usize, pct: f64) {
    let clamped = pct.clamp(5.0, 100.0);
    let mut cfg = DisplaysConfig::load();
    let mut connector = String::new();
    if let Some(out) = cfg.outputs.get_mut(output_idx) {
        connector = out.connector.clone();
        out.brightness = clamped;
        if out.primary {
            cfg.brightness = clamped;
        }
    } else {
        cfg.brightness = clamped;
    }
    let _ = cfg.save();
    let _ = IpcClient::send_command(IpcMessage::ReloadDisplayBrightness);

    if connector_has_hardware_backlight(&connector) {
        malus_de_common::config::apply_hardware_backlight_pct(clamped);
    }
}

fn schedule_output_brightness(output_idx: usize, pct: f64) {
    static WORKER: OnceLock<Arc<(Mutex<Option<(usize, f64)>>, Condvar)>> = OnceLock::new();
    let shared = WORKER.get_or_init(|| {
        let pair = Arc::new((Mutex::new(None::<(usize, f64)>), Condvar::new()));
        let bg = Arc::clone(&pair);
        std::thread::spawn(move || loop {
            let task = {
                let (lock, cvar) = &*bg;
                let mut guard = match lock.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                while guard.is_none() {
                    guard = match cvar.wait(guard) {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                }
                guard.take()
            };
            if let Some((idx, val)) = task {
                apply_output_brightness(idx, val);
            }
        });
        pair
    });

    if let Ok(mut guard) = shared.0.lock() {
        *guard = Some((output_idx, pct));
        shared.1.notify_one();
    }
}

fn read_night_shift_enabled() -> bool {
    Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.color",
            "night-light-enabled",
        ])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

fn write_night_shift_enabled(enabled: bool) {
    let val = if enabled { "true" } else { "false" };
    let _ = Command::new("gsettings")
        .args([
            "set",
            "org.gnome.settings-daemon.plugins.color",
            "night-light-enabled",
            val,
        ])
        .spawn();
}

fn read_night_shift_temperature() -> u32 {
    Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.color",
            "night-light-temperature",
        ])
        .output()
        .ok()
        .and_then(|o| {
            let txt = String::from_utf8_lossy(&o.stdout);
            txt.split_whitespace()
                .last()
                .and_then(|s| s.parse::<u32>().ok())
        })
        .map(|v| v.clamp(2700, 5500))
        .unwrap_or(4000)
}

fn write_night_shift_temperature(kelvin: u32) {
    let val = kelvin.clamp(2700, 5500).to_string();
    std::thread::spawn(move || {
        let _ = Command::new("gsettings")
            .args([
                "set",
                "org.gnome.settings-daemon.plugins.color",
                "night-light-temperature",
                &val,
            ])
            .status();
    });
}

pub(crate) fn friendly_display_name(o: &DisplayOutputConfig) -> String {
    let trimmed = o.name.trim();
    if is_builtin_connector(&o.connector) && (trimmed.is_empty() || trimmed.eq_ignore_ascii_case("eDP-1")) {
        "Pantalla integrada".to_string()
    } else if trimmed.is_empty() {
        o.connector.clone()
    } else if is_builtin_connector(&o.connector) {
        format!("Pantalla integrada ({})", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn is_builtin_connector(connector: &str) -> bool {
    let low = connector.to_lowercase();
    low.starts_with("edp") || low.starts_with("lvds") || low.starts_with("dsi")
}

fn load_hero_wallpaper_pixbuf() -> Option<gdk_pixbuf::Pixbuf> {
    let wp = WallpaperConfig::load();
    let path = match wp.mode {
        WallpaperMode::DynamicPair => {
            let p = if is_dark_mode_active() {
                wp.dark_image_path.as_deref()
            } else {
                wp.light_image_path.as_deref()
            };
            p.map(PathBuf::from).filter(|p| p.is_file())
        }
        WallpaperMode::Image => wp
            .image_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(default_bundled_wallpaper_path),
        _ => None,
    };
    path.and_then(|p| gdk_pixbuf::Pixbuf::from_file_at_scale(p, 240, 150, true).ok())
        .map(|pb| pb.apply_embedded_orientation().unwrap_or(pb))
}

fn draw_wallpaper_in_screen(
    cr: &gtk4::cairo::Context,
    pb: Option<&gdk_pixbuf::Pixbuf>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    let _ = cr.save();
    cr.rectangle(x, y, w, h);
    cr.clip();
    if let Some(pb) = pb {
        let sw = pb.width().max(1) as f64;
        let sh = pb.height().max(1) as f64;
        let scale = (w / sw).max(h / sh);
        let ox = x + (w - sw * scale) * 0.5;
        let oy = y + (h - sh * scale) * 0.5;
        cr.translate(ox, oy);
        cr.scale(scale, scale);
        gtk4::cairo::Context::set_source_pixbuf(cr, pb, 0.0, 0.0);
        let _ = cr.paint();
    } else {
        let grad = gtk4::cairo::LinearGradient::new(x, y, x + w, y + h);
        grad.add_color_stop_rgb(0.0, 0.25, 0.45, 0.72);
        grad.add_color_stop_rgb(1.0, 0.65, 0.45, 0.35);
        let _ = cr.set_source(&grad);
        let _ = cr.paint();
    }
    let _ = cr.restore();
}

fn draw_apple_monitor_silhouette(
    cr: &gtk4::cairo::Context,
    pb: Option<&gdk_pixbuf::Pixbuf>,
    cx: f64,
    bottom_y: f64,
    builtin: bool,
    selected: bool,
    is_dark: bool,
    accent: (f64, f64, f64),
) -> (f64, f64, f64, f64) {
    if builtin {
        // Silueta MacBook Pro estilo Apple
        let scr_w = 88.0;
        let scr_h = 56.0;
        let base_w = 104.0;
        let base_h = 5.5;
        let scr_x = cx - scr_w * 0.5;
        let scr_y = bottom_y - base_h - scr_h - 1.0;

        // Bisel exterior
        let _ = cr.save();
        cr.translate(scr_x, scr_y);
        squircle_path(cr, scr_w, scr_h, 6.0);
        cr.set_source_rgb(0.11, 0.12, 0.14);
        let _ = cr.fill();
        let _ = cr.restore();

        // Pantalla interior con wallpaper
        draw_wallpaper_in_screen(cr, pb, scr_x + 3.5, scr_y + 3.5, scr_w - 7.0, scr_h - 7.0);

        // Anillo si está seleccionada
        if selected {
            let _ = cr.save();
            cr.translate(scr_x - 1.0, scr_y - 1.0);
            squircle_path(cr, scr_w + 2.0, scr_h + 2.0, 7.0);
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.95);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
            let _ = cr.restore();
        }

        // Base de aluminio inferior
        let base_x = cx - base_w * 0.5;
        let base_y = bottom_y - base_h;
        let _ = cr.save();
        cr.translate(base_x, base_y);
        squircle_path(cr, base_w, base_h, 3.0);
        if is_dark {
            cr.set_source_rgb(0.55, 0.57, 0.61);
        } else {
            cr.set_source_rgb(0.75, 0.77, 0.80);
        }
        let _ = cr.fill();
        let _ = cr.restore();

        (scr_x.min(base_x), scr_y, base_w, scr_h + base_h + 1.0)
    } else {
        // Silueta Apple Studio Display / monitor externo
        let scr_w = 114.0;
        let scr_h = 68.0;
        let stand_h = 12.0;
        let scr_x = cx - scr_w * 0.5;
        let scr_y = bottom_y - stand_h - scr_h;

        // Pie de aluminio
        let neck_w = 18.0;
        cr.rectangle(cx - neck_w * 0.5, scr_y + scr_h - 2.0, neck_w, stand_h);
        cr.set_source_rgb(0.68, 0.70, 0.74);
        let _ = cr.fill();

        let foot_w = 40.0;
        cr.rectangle(cx - foot_w * 0.5, bottom_y - 2.5, foot_w, 2.5);
        cr.set_source_rgb(0.60, 0.62, 0.66);
        let _ = cr.fill();

        // Bisel exterior
        let _ = cr.save();
        cr.translate(scr_x, scr_y);
        squircle_path(cr, scr_w, scr_h, 6.0);
        cr.set_source_rgb(0.11, 0.12, 0.14);
        let _ = cr.fill();
        let _ = cr.restore();

        // Pantalla interior con wallpaper
        draw_wallpaper_in_screen(cr, pb, scr_x + 4.0, scr_y + 4.0, scr_w - 8.0, scr_h - 8.0);

        if selected {
            let _ = cr.save();
            cr.translate(scr_x - 1.0, scr_y - 1.0);
            squircle_path(cr, scr_w + 2.0, scr_h + 2.0, 7.0);
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.95);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
            let _ = cr.restore();
        }

        (scr_x, scr_y, scr_w, scr_h + stand_h)
    }
}

fn draw_scale_thumbnail_window(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    scale_pct: i32,
    selected: bool,
    is_dark: bool,
    accent: (f64, f64, f64),
) {
    // Sombra sutil
    let _ = cr.save();
    cr.translate(2.0, 3.0);
    squircle_path(cr, w - 4.0, h - 4.0, 9.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.10);
    let _ = cr.fill();
    let _ = cr.restore();

    // Cuerpo de la miniatura
    let _ = cr.save();
    cr.translate(2.0, 2.0);
    squircle_path(cr, w - 4.0, h - 4.0, 9.0);
    if is_dark {
        cr.set_source_rgb(0.18, 0.19, 0.21);
    } else {
        cr.set_source_rgb(0.98, 0.98, 0.99);
    }
    let _ = cr.fill();
    let _ = cr.restore();

    // Porcentaje centrado únicamente (sin texto de ejemplo ni semáforos)
    let _ = cr.save();
    let pct_text = format!("{}%", scale_pct);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        if selected {
            gtk4::cairo::FontWeight::Bold
        } else {
            gtk4::cairo::FontWeight::Normal
        },
    );
    cr.set_font_size(14.5);
    if selected {
        cr.set_source_rgba(accent.0, accent.1, accent.2, 0.98);
    } else if is_dark {
        cr.set_source_rgba(0.92, 0.93, 0.96, 0.88);
    } else {
        cr.set_source_rgba(0.14, 0.15, 0.18, 0.88);
    }
    if let Ok(ext) = cr.text_extents(&pct_text) {
        let tx = (w - ext.width()) * 0.5 - ext.x_bearing();
        let ty = (h - ext.height()) * 0.5 - ext.y_bearing();
        cr.move_to(tx, ty);
        let _ = cr.show_text(&pct_text);
    }
    let _ = cr.restore();

    // Borde de selección
    let _ = cr.save();
    cr.translate(2.0, 2.0);
    squircle_path(cr, w - 4.0, h - 4.0, 9.0);
    if selected {
        cr.set_source_rgba(accent.0, accent.1, accent.2, 0.95);
        cr.set_line_width(2.0);
    } else if is_dark {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        cr.set_line_width(1.0);
    } else {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();
    let _ = cr.restore();
}

fn build_switch_row(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
    pending: bool,
    on_toggle: impl Fn(bool) + 'static,
) -> (GtkBox, Switch) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
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
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    if let Some(sub) = subtitle {
        text_col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
    }
    row.append(&text_col);

    if pending {
        row.append(&make_pending_badge());
    }

    let sw = Switch::new(initial);
    sw.set_valign(Align::Center);
    if pending {
        sw.set_sensitive(false);
    } else {
        sw.connect_state_set(move |_, active| {
            on_toggle(active);
            gtk4::glib::Propagation::Proceed
        });
    }
    row.append(sw.widget());
    (row, sw)
}

fn make_card_separator() -> GtkBox {
    GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-row-divider"])
        .hexpand(true)
        .build()
}

fn build_apple_dropdown_row(
    title: &str,
    subtitle: Option<&str>,
    options: &[&str],
    initial_idx: usize,
    pending: bool,
    on_select: impl Fn(usize) + 'static,
) -> (GtkBox, Rc<dyn Fn(usize)>, Label, Button) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
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
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    if let Some(sub) = subtitle {
        text_col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
    }
    row.append(&text_col);

    if pending {
        row.append(&make_pending_badge());
    }

    let val_lbl = Label::builder()
        .label(options.get(initial_idx).copied().unwrap_or("—"))
        .css_classes(["disp-inline-val"])
        .valign(Align::Center)
        .build();
    let arrows = Image::builder()
        .icon_name("pan-down-symbolic")
        .pixel_size(12)
        .css_classes(["net-row-chevron"])
        .valign(Align::Center)
        .build();
    let btn_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    btn_box.append(&val_lbl);
    btn_box.append(&arrows);

    let trigger = Button::builder()
        .child(&btn_box)
        .css_classes(["fw-rule-action-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .sensitive(!pending)
        .build();

    let popover = Popover::builder()
        .css_classes(["wifi-macos-popover"])
        .has_arrow(false)
        .build();
    popover.set_parent(&trigger);

    let pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let selected_idx = Rc::new(Cell::new(initial_idx));
    let checks: Rc<RefCell<Vec<Label>>> = Rc::new(RefCell::new(Vec::new()));
    let on_sel = Rc::new(on_select);
    let opts_owned: Vec<String> = options.iter().map(|s| (*s).to_string()).collect();

    for (idx, opt) in opts_owned.iter().enumerate() {
        let (row_overlay, item_btn) = make_popover_action_row(
            opt,
            true,
            if idx == initial_idx { Some("✓") } else { None },
        );
        if let Some(chk_lbl) = item_btn
            .child()
            .and_then(|c| c.downcast::<GtkBox>().ok())
            .and_then(|b| b.first_child())
            .and_then(|w| w.downcast::<Label>().ok())
        {
            checks.borrow_mut().push(chk_lbl);
        }
        let pop_c = popover.clone();
        let val_c = val_lbl.clone();
        let checks_c = checks.clone();
        let sel_c = selected_idx.clone();
        let opts_c = opts_owned.clone();
        let cb_c = on_sel.clone();
        item_btn.connect_clicked(move |_| {
            sel_c.set(idx);
            if let Some(label) = opts_c.get(idx) {
                val_c.set_label(label);
            }
            for (i, c) in checks_c.borrow().iter().enumerate() {
                c.set_label(if i == idx { "✓" } else { "" });
            }
            pop_c.popdown();
            cb_c(idx);
        });
        pop_box.append(&row_overlay);
    }

    popover.set_child(Some(&pop_box));
    let pop_show = popover.clone();
    trigger.connect_clicked(move |_| {
        pop_show.popup();
    });
    row.append(&trigger);

    let val_set = val_lbl.clone();
    let checks_set = checks.clone();
    let sel_set = selected_idx.clone();
    let opts_set = opts_owned;
    let set_idx: Rc<dyn Fn(usize)> = Rc::new(move |idx: usize| {
        sel_set.set(idx);
        if let Some(label) = opts_set.get(idx) {
            val_set.set_label(label);
        }
        for (i, c) in checks_set.borrow().iter().enumerate() {
            c.set_label(if i == idx { "✓" } else { "" });
        }
    });

    (row, set_idx, val_lbl, trigger)
}

#[derive(Clone, Debug)]
struct CanvasInteractionState {
    selected_idx: usize,
    dragging_idx: Option<usize>,
    drag_dx: f64,
    drag_dy: f64,
    rects: Vec<(usize, f64, f64, f64, f64)>,
}

fn show_arrange_modal(
    parent_window: &ApplicationWindow,
    selected_output_idx: usize,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent_window)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .default_width(640)
        .title("Ordenar pantallas")
        .css_classes(["macos-modal-dialog"])
        .build();
    crate::attach_modal_scrim(&dialog, parent_window);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(18)
        .margin_end(18)
        .build();

    root.append(
        &Label::builder()
            .label("Ordenar pantallas")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    root.append(
        &Label::builder()
            .label("Arrastra una pantalla para reorganizar su posición horizontal o alineación vertical. La barra blanca superior indica la pantalla principal.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let canvas = DrawingArea::builder()
        .content_height(276)
        .hexpand(true)
        .build();
    crate::register_accent_draw_area(&canvas);

    let canvas_overlay = Overlay::builder()
        .hexpand(true)
        .build();
    canvas_overlay.set_child(Some(&canvas));

    let coords_lbl = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .wrap(true)
        .build();

    let state = Rc::new(RefCell::new(CanvasInteractionState {
        selected_idx: selected_output_idx,
        dragging_idx: None,
        drag_dx: 0.0,
        drag_dy: 0.0,
        rects: Vec::new(),
    }));

    let sel_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .halign(Align::Start)
        .build();
    let y_readout_lbl = Label::builder()
        .label("y = 0 px")
        .css_classes(["card-section-sub"])
        .margin_start(4)
        .margin_end(4)
        .build();

    let sync_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let sync_coords: Rc<dyn Fn()> = {
        let lbl = coords_lbl.clone();
        let canvas_c = canvas.clone();
        let st_sync = state.clone();
        let sel_row_c = sel_row.clone();
        let y_lbl_c = y_readout_lbl.clone();
        let sync_slot_c = sync_slot.clone();
        Rc::new(move || {
            let cfg = DisplaysConfig::load();
            let mut sorted: Vec<_> = cfg.outputs.iter().collect();
            sorted.sort_by_key(|o| o.x);
            let txt: Vec<String> = sorted
                .iter()
                .map(|o| {
                    let (lw, lh) = o.logical_size();
                    format!(
                        "{} [{}×{} px · x={} px, y={} px]",
                        o.connector, lw, lh, o.x, o.y
                    )
                })
                .collect();
            lbl.set_label(&txt.join("   ·   "));

            let sel_idx = if cfg.outputs.is_empty() {
                0
            } else {
                let mut st = st_sync.borrow_mut();
                if st.selected_idx >= cfg.outputs.len() {
                    st.selected_idx = 0;
                }
                st.selected_idx
            };

            if let Some(sel_out) = cfg.outputs.get(sel_idx) {
                y_lbl_c.set_label(&format!("y = {} px", sel_out.y));
            } else {
                y_lbl_c.set_label("y = 0 px");
            }

            while let Some(child) = sel_row_c.first_child() {
                sel_row_c.remove(&child);
            }
            if !cfg.outputs.is_empty() {
                sel_row_c.append(
                    &Label::builder()
                        .label("Pantalla:")
                        .css_classes(["card-section-sub"])
                        .margin_end(2)
                        .build(),
                );
                for (idx, out) in cfg.outputs.iter().enumerate() {
                    let pill_lbl = format!("{} (y={} px)", friendly_display_name(out), out.y);
                    let (wrap, btn) = make_squircle_button(
                        &pill_lbl,
                        idx == sel_idx,
                        Some(&format!("Seleccionar {} para ajustar su posición vertical", out.connector)),
                    );
                    let st_btn = st_sync.clone();
                    let slot_btn = sync_slot_c.clone();
                    btn.connect_clicked(move |_| {
                        st_btn.borrow_mut().selected_idx = idx;
                        if let Some(cb) = slot_btn.borrow().as_ref().cloned() {
                            cb();
                        }
                    });
                    sel_row_c.append(&wrap);
                }
            }

            canvas_c.queue_draw();
        })
    };
    *sync_slot.borrow_mut() = Some(sync_coords.clone());
    sync_coords();

    {
        let st_draw = state.clone();
        canvas.set_draw_func(move |_, cr, width, height| {
            let cfg = DisplaysConfig::load();
            let mut st = st_draw.borrow_mut();
            st.rects.clear();

            let is_dark = is_dark_mode_active();
            let w = width as f64;
            let h = height as f64;

            let _ = cr.save();
            squircle_path(cr, w, h, 12.0);
            if is_dark {
                cr.set_source_rgb(0.09, 0.10, 0.12);
            } else {
                cr.set_source_rgb(0.92, 0.93, 0.95);
            }
            let _ = cr.fill();
            let _ = cr.restore();

            if cfg.outputs.is_empty() {
                return;
            }

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
            let total_h = (max_y - min_y).max(760) as f64;
            let pad_x = 28.0;
            let top_pad = 42.0;
            let bottom_pad = 46.0;
            let avail_h = (h - top_pad - bottom_pad).max(100.0);
            let zoom = ((w - pad_x * 2.0).max(180.0) / total_w).min(avail_h / total_h);
            let origin_x = (w - total_w * zoom) * 0.5;
            let origin_y = top_pad + (avail_h - total_h * zoom) * 0.5;
            let accent = crate::accent_color();

            for (idx, o) in cfg.outputs.iter().enumerate() {
                let (lw, lh) = o.logical_size();
                let mut rx = origin_x + (o.x - min_x) as f64 * zoom;
                let mut ry = origin_y + (o.y - min_y) as f64 * zoom;
                let rw = lw as f64 * zoom;
                let rh = lh as f64 * zoom;
                if st.dragging_idx == Some(idx) {
                    rx += st.drag_dx;
                    ry += st.drag_dy;
                }
                st.rects.push((idx, rx, ry, rw, rh));

                let _ = cr.save();
                cr.translate(rx, ry);
                squircle_path(cr, rw, rh, 8.0);
                if o.primary {
                    cr.set_source_rgb(0.14, 0.34, 0.62);
                } else {
                    cr.set_source_rgb(0.22, 0.27, 0.44);
                }
                let _ = cr.fill_preserve();
                if st.selected_idx == idx || st.dragging_idx == Some(idx) {
                    cr.set_source_rgb(accent.0, accent.1, accent.2);
                    cr.set_line_width(2.2);
                } else {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
                    cr.set_line_width(1.2);
                }
                let _ = cr.stroke();
                let _ = cr.restore();

                if o.primary {
                    cr.rectangle(rx + 3.0, ry + 3.0, (rw - 6.0).max(4.0), 6.0);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.88);
                    let _ = cr.fill();
                }

                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.set_font_size(11.5);
                cr.move_to(rx + 10.0, ry + 26.0);
                let _ = cr.show_text(&friendly_display_name(o));

                cr.set_source_rgba(0.90, 0.92, 0.96, 0.85);
                cr.set_font_size(10.0);
                cr.move_to(rx + 10.0, ry + 41.0);
                let _ = cr.show_text(&format!("{} × {} ({:.0}%)", lw, lh, o.scale * 100.0));

                if rh >= 72.0 {
                    cr.set_source_rgba(0.84, 0.88, 0.96, 0.78);
                    cr.set_font_size(9.5);
                    cr.move_to(rx + 10.0, ry + 56.0);
                    let _ = cr.show_text(&format!("x={} px · y={} px", o.x, o.y));
                }
            }
        });
    }

    // Arrastre interactivo en Ordenar
    let drag = GestureDrag::new();
    {
        let st_c = state.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut st = st_c.borrow_mut();
            if let Some(&(idx, ..)) = st
                .rects
                .iter()
                .find(|&&(_, rx, ry, rw, rh)| x >= rx && x <= rx + rw && y >= ry && y <= ry + rh)
            {
                st.selected_idx = idx;
                st.dragging_idx = Some(idx);
                st.drag_dx = 0.0;
                st.drag_dy = 0.0;
            }
        });
    }
    {
        let st_c = state.clone();
        let canvas_c = canvas.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let mut st = st_c.borrow_mut();
            if st.dragging_idx.is_some() {
                st.drag_dx = dx;
                st.drag_dy = dy;
                canvas_c.queue_draw();
            }
        });
    }
    {
        let st_c = state.clone();
        let sync_c = sync_coords.clone();
        let on_ch = on_changed.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            let mut st = st_c.borrow_mut();
            let Some(drag_idx) = st.dragging_idx.take() else {
                return;
            };
            st.drag_dx = 0.0;
            st.drag_dy = 0.0;

            if dx.abs() < 4.0 && dy.abs() < 4.0 {
                drop(st);
                sync_c();
                return;
            }

            let mut cfg = DisplaysConfig::load();
            if cfg.outputs.len() >= 2 {
                let other_idx = if drag_idx == 0 { 1 } else { 0 };
                let r_drag = st.rects.iter().find(|r| r.0 == drag_idx).copied();
                let r_other = st.rects.iter().find(|r| r.0 == other_idx).copied();
                drop(st);

                if let (Some((_, rx, _, rw, _)), Some((_, ox, _, ow, _))) = (r_drag, r_other) {
                    let new_cx = rx + dx + rw * 0.5;
                    let other_cx = ox + ow * 0.5;
                    let was_left = cfg.outputs[drag_idx].x <= cfg.outputs[other_idx].x;
                    let now_left = new_cx < other_cx;
                    if was_left != now_left {
                        cfg.swap_horizontal_order();
                    }
                }
                let logical_dy = (dy * 8.0).round() as i32;
                if logical_dy.abs() >= 15 {
                    cfg.adjust_vertical_offset(drag_idx, logical_dy);
                }
                save_displays_and_reload(&cfg);
                sync_c();
                on_ch();
            }
        });
    }
    canvas.add_controller(drag);

    // Selector de pantalla y botones de ajuste vertical en píxeles dentro del recuadro
    sel_row.set_halign(Align::Start);
    sel_row.set_valign(Align::Start);
    sel_row.set_margin_top(8);
    sel_row.set_margin_start(10);
    sel_row.set_margin_end(10);
    canvas_overlay.add_overlay(&sel_row);

    let vert_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(5)
        .halign(Align::Center)
        .valign(Align::End)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();

    let append_step_buttons = |steps: &[(&str, i32, &str)]| {
        for &(label, delta, tip) in steps {
            let (w_btn, b) = make_squircle_button(label, false, Some(tip));
            let st_c = state.clone();
            let sync_c = sync_coords.clone();
            let on_ch = on_changed.clone();
            b.connect_clicked(move |_| {
                let mut cfg = DisplaysConfig::load();
                if cfg.outputs.is_empty() {
                    return;
                }
                let sel = st_c.borrow().selected_idx.min(cfg.outputs.len() - 1);
                cfg.adjust_vertical_offset(sel, delta);
                save_displays_and_reload(&cfg);
                sync_c();
                on_ch();
            });
            vert_row.append(&w_btn);
        }
    };
    append_step_buttons(&[
        ("↑ 50 px", -50, "Subir pantalla seleccionada 50 píxeles"),
        ("↑ 10 px", -10, "Subir pantalla seleccionada 10 píxeles"),
        ("↑ 1 px", -1, "Subir pantalla seleccionada 1 píxel"),
    ]);
    vert_row.append(&y_readout_lbl);
    append_step_buttons(&[
        ("↓ 1 px", 1, "Bajar pantalla seleccionada 1 píxel"),
        ("↓ 10 px", 10, "Bajar pantalla seleccionada 10 píxeles"),
        ("↓ 50 px", 50, "Bajar pantalla seleccionada 50 píxeles"),
    ]);
    canvas_overlay.add_overlay(&vert_row);

    root.append(&canvas_overlay);
    root.append(&coords_lbl);

    // Botones rápidos de disposición (Swap + alineaciones verticales)
    let tools_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .halign(Align::Start)
        .build();

    let (swap_wrap, swap_btn) = make_squircle_button("Invertir", false, Some("Intercambiar izquierda / derecha"));
    {
        let sync_c = sync_coords.clone();
        let on_ch = on_changed.clone();
        swap_btn.connect_clicked(move |_| {
            let mut cfg = DisplaysConfig::load();
            cfg.swap_horizontal_order();
            save_displays_and_reload(&cfg);
            sync_c();
            on_ch();
        });
    }
    tools_row.append(&swap_wrap);

    for (label, mode) in [
        ("Arriba", "top"),
        ("Centrar", "center"),
        ("Abajo", "bottom"),
        ("Escritorio (y=519)", "desk"),
    ] {
        let (w_btn, b) = make_squircle_button(label, false, None);
        let sync_c = sync_coords.clone();
        let on_ch = on_changed.clone();
        let mode_s = mode.to_string();
        b.connect_clicked(move |_| {
            let mut cfg = DisplaysConfig::load();
            cfg.set_vertical_alignment(&mode_s);
            save_displays_and_reload(&cfg);
            sync_c();
            on_ch();
        });
        tools_row.append(&w_btn);
    }
    root.append(&tools_row);

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (done_wrap, done_btn) = make_squircle_button("Listo", true, None);
    let dlg_close = dialog.clone();
    done_btn.connect_clicked(move |_| dlg_close.close());
    footer.append(&done_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));
    dialog.present();
}

fn show_advanced_modal(
    parent_window: &ApplicationWindow,
    on_list_mode_changed: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent_window)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .default_width(500)
        .title("Avanzado")
        .css_classes(["macos-modal-dialog"])
        .build();
    crate::attach_modal_scrim(&dialog, parent_window);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(18)
        .margin_end(18)
        .build();

    let cfg_now = DisplaysConfig::load();
    let (list_row, _list_sw) = build_switch_row(
        "Mostrar las resoluciones en forma de lista",
        None,
        cfg_now.show_resolutions_as_list,
        false,
        move |active| {
            let mut cfg = DisplaysConfig::load();
            cfg.show_resolutions_as_list = active;
            save_displays_and_reload(&cfg);
            on_list_mode_changed();
        },
    );
    root.append(&wrap_squircle_card(&list_row, WIFI_CARD_SQUIRCLE_RADIUS));

    root.append(
        &Label::builder()
            .label("Enlazar a Mac o iPad")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    let link_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (r1, _) = build_switch_row(
        "Permitir compartir el puntero y el teclado con dispositivos Mac o iPad cercanos",
        Some("Tu puntero y teclado pueden usarse en cualquier Mac o iPad cercanos donde hayas iniciado sesión en tu cuenta de iCloud."),
        true,
        true,
        |_| {},
    );
    let (r2, _) = build_switch_row(
        "Desplazar hacia afuera del borde de una pantalla para conectar con una Mac o iPad cercano",
        Some("Permite que el puntero se conecte a una Mac o iPad cercanos al desplazarlo a través del borde de una pantalla."),
        true,
        true,
        |_| {},
    );
    let (r3, _) = build_switch_row(
        "Reconectar automáticamente a dispositivos Mac o iPad cercanos",
        Some("Permite que esta Mac se vuelva a conectar de forma automática a cualquier Mac o iPad con los que se ha establecido conexión previamente."),
        false,
        true,
        |_| {},
    );
    link_card.append(&r1);
    link_card.append(&make_card_separator());
    link_card.append(&r2);
    link_card.append(&make_card_separator());
    link_card.append(&r3);
    root.append(&wrap_squircle_card(&link_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (done_wrap, done_btn) = make_squircle_button("Listo", true, None);
    let dlg_close = dialog.clone();
    done_btn.connect_clicked(move |_| dlg_close.close());
    footer.append(&done_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));
    dialog.present();
}

fn show_night_shift_modal(parent_window: &ApplicationWindow) {
    let dialog = Window::builder()
        .transient_for(parent_window)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .default_width(480)
        .title("Night Shift")
        .css_classes(["macos-modal-dialog"])
        .build();
    crate::attach_modal_scrim(&dialog, parent_window);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(18)
        .margin_end(18)
        .build();

    root.append(
        &Label::builder()
            .label("Night Shift cambia automáticamente los colores de tu pantalla volviéndolos más cálidos después del atardecer, lo cual podría ayudarte a dormir mejor.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let (sched_row, ..) = build_apple_dropdown_row(
        "Horario",
        None,
        &NIGHT_SCHEDULE_OPTIONS,
        0,
        true,
        |_| {},
    );
    card.append(&sched_row);
    card.append(&make_card_separator());

    let initial_ns = read_night_shift_enabled();
    let (ns_row, _) = build_switch_row(
        "Activar hasta mañana",
        None,
        initial_ns,
        false,
        |active| write_night_shift_enabled(active),
    );
    card.append(&ns_row);
    card.append(&make_card_separator());

    // Fila de Temperatura de los colores
    let temp_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let temp_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    let temp_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    temp_title_row.append(
        &Label::builder()
            .label("Temperatura de los colores")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    temp_title_row.append(&make_pending_badge());
    temp_text.append(&temp_title_row);
    temp_text.append(
        &Label::builder()
            .label("La configuración cálida podría afectar la apariencia de ciertos movimientos en la pantalla.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    temp_row.append(&temp_text);

    let slider_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .width_request(180)
        .valign(Align::Center)
        .build();
    let initial_temp = read_night_shift_temperature();
    // Invertido visualmente como macOS: izquierda Menos cálidos (5500K), derecha Más cálidos (2700K)
    let temp_slider = Scale::with_range(Orientation::Horizontal, 2700.0, 5500.0, 100.0);
    temp_slider.set_inverted(true);
    temp_slider.set_value(initial_temp as f64);
    temp_slider.set_draw_value(false);
    temp_slider.set_css_classes(&["bat-limit-slider"]);
    temp_slider.connect_value_changed(|s| {
        write_night_shift_temperature(s.value().round() as u32);
    });
    let temp_labels = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .build();
    temp_labels.append(
        &Label::builder()
            .label("Menos cálidos")
            .css_classes(["res-pct-label"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    temp_labels.append(
        &Label::builder()
            .label("Más cálidos")
            .css_classes(["res-pct-label"])
            .xalign(1.0)
            .build(),
    );
    slider_col.append(&temp_slider);
    slider_col.append(&temp_labels);
    temp_row.append(&slider_col);
    card.append(&temp_row);

    root.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (done_wrap, done_btn) = make_squircle_button("Listo", true, None);
    let dlg_close = dialog.clone();
    done_btn.connect_clicked(move |_| dlg_close.close());
    footer.append(&done_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));
    dialog.present();
}

pub fn create_displays_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["disp-content-pane"])
        .spacing(0)
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let selected_idx = Rc::new(Cell::new(0usize));
    let syncing_ui = Rc::new(Cell::new(false));
    let show_all_modes = Rc::new(Cell::new(false));

    // ========================================================================
    // 1. Banner superior gris (Pantallas0/1) con escenario de pantallas y separador
    // ========================================================================
    let stage_overlay = gtk4::Overlay::builder()
        .css_classes(["disp-stage-banner"])
        .hexpand(true)
        .build();

    let cards_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_start(22)
        .margin_end(22)
        .margin_top(14)
        .build();

    let stage_area = DrawingArea::builder()
        .content_height(114)
        .hexpand(true)
        .build();
    crate::register_accent_draw_area(&stage_area);

    let stage_hitboxes: Rc<RefCell<Vec<(usize, f64, f64, f64, f64)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let wallpaper_thumb_cache: Rc<RefCell<Option<gdk_pixbuf::Pixbuf>>> =
        Rc::new(RefCell::new(load_hero_wallpaper_pixbuf()));

    fn compute_stage_slots(cfg: &DisplaysConfig, width: f64) -> Vec<(usize, f64, bool)> {
        if cfg.outputs.is_empty() {
            return Vec::new();
        }
        let mut order: Vec<usize> = (0..cfg.outputs.len()).collect();
        order.sort_by_key(|&i| cfg.outputs[i].x);
        let gap = 24.0;
        let widths: Vec<f64> = order
            .iter()
            .map(|&idx| {
                if is_builtin_connector(&cfg.outputs[idx].connector) {
                    104.0
                } else {
                    114.0
                }
            })
            .collect();
        let total_w: f64 =
            widths.iter().sum::<f64>() + gap * (order.len().saturating_sub(1) as f64);
        let mut cur_x = (width - total_w) * 0.5;
        let mut slots = Vec::with_capacity(order.len());
        for (pos, &idx) in order.iter().enumerate() {
            let slot_w = widths[pos];
            let cx = cur_x + slot_w * 0.5;
            slots.push((idx, cx, is_builtin_connector(&cfg.outputs[idx].connector)));
            cur_x += slot_w + gap;
        }
        slots
    }

    {
        let sel_draw = selected_idx.clone();
        let hits_draw = stage_hitboxes.clone();
        let wp_draw = wallpaper_thumb_cache.clone();
        stage_area.set_draw_func(move |_, cr, width, height| {
            let cfg = DisplaysConfig::load();
            let mut hits = hits_draw.borrow_mut();
            hits.clear();
            let w = width as f64;
            let h = height as f64;
            if w <= 10.0 || h <= 10.0 || cfg.outputs.is_empty() {
                return;
            }

            let is_dark = is_dark_mode_active();
            let accent = crate::accent_color();
            let multi = cfg.outputs.len() > 1;
            let cur_sel = sel_draw.get().min(cfg.outputs.len().saturating_sub(1));
            let pb_opt = wp_draw.borrow().clone();
            let base_bottom_y = 86.0;

            for (idx, cx, builtin) in compute_stage_slots(&cfg, w) {
                let o = &cfg.outputs[idx];
                let is_sel = idx == cur_sel;

                let (rx, ry, rw, rh) = draw_apple_monitor_silhouette(
                    cr,
                    pb_opt.as_ref(),
                    cx,
                    base_bottom_y,
                    builtin,
                    is_sel && multi,
                    is_dark,
                    accent,
                );
                hits.push((idx, rx - 4.0, ry - 4.0, rw + 8.0, rh + 28.0));

                // Nombre bajo la pantalla
                let name = friendly_display_name(o);
                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    if is_sel {
                        gtk4::cairo::FontWeight::Bold
                    } else {
                        gtk4::cairo::FontWeight::Normal
                    },
                );
                cr.set_font_size(12.0);
                if is_dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, if is_sel { 0.95 } else { 0.65 });
                } else {
                    cr.set_source_rgba(0.08, 0.09, 0.12, if is_sel { 0.92 } else { 0.65 });
                }
                let ext = cr.text_extents(&name).ok();
                let text_w = ext.map(|e| e.width()).unwrap_or(80.0);
                cr.move_to(cx - text_w * 0.5, base_bottom_y + 18.0);
                let _ = cr.show_text(&name);
            }
        });
    }

    let (arrange_wrap, arrange_btn) = make_squircle_button("Ordenar...", false, None);
    arrange_wrap.set_halign(Align::Start);
    arrange_wrap.set_valign(Align::End);
    arrange_wrap.set_margin_bottom(2);
    stage_overlay.set_child(Some(&stage_area));
    stage_overlay.add_overlay(&arrange_wrap);
    pane.append(&stage_overlay);

    // ========================================================================
    // 2. Tarjeta 1: Usar como + Escala (con triángulo integrado si hay >1 pantalla)
    // ========================================================================
    let main_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let use_as_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    use_as_row.append(
        &Label::builder()
            .label("Usar como")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let use_as_val_lbl = Label::builder()
        .label("Pantalla principal")
        .css_classes(["disp-inline-val"])
        .valign(Align::Center)
        .build();
    let use_as_btn_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    use_as_btn_box.append(&use_as_val_lbl);
    use_as_btn_box.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let use_as_btn = Button::builder()
        .child(&use_as_btn_box)
        .css_classes(["fw-rule-action-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();
    let use_as_popover = Popover::builder()
        .css_classes(["wifi-macos-popover"])
        .has_arrow(false)
        .build();
    use_as_popover.set_parent(&use_as_btn);
    use_as_row.append(&use_as_btn);
    let use_as_sep = make_card_separator();
    main_card.append(&use_as_row);
    main_card.append(&use_as_sep);

    // Contenedor de miniaturas ("Texto más grande" .. "Más espacio")
    let thumbs_mode_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wp-hero-box"])
        .build();
    let thumbs_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .halign(Align::Center)
        .build();
    let active_res_lbl = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .halign(Align::Center)
        .build();
    thumbs_mode_box.append(&thumbs_row);
    thumbs_mode_box.append(&active_res_lbl);

    // Ajuste continuo de escala (%) + input exacto integrado estilo macOS
    let continuous_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(2)
        .build();
    continuous_row.append(
        &Label::builder()
            .label("Escala exacta:")
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build(),
    );
    let scale_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 2.5);
    scale_slider.set_draw_value(false);
    scale_slider.set_hexpand(true);
    scale_slider.set_css_classes(&["bat-limit-slider"]);
    let step_down_btn = Button::builder()
        .label("−")
        .has_frame(false)
        .tooltip_text("Reducir escala (-5%)")
        .css_classes(["disp-stepper-inner-btn", "disp-stepper-left"])
        .width_request(22)
        .height_request(22)
        .valign(Align::Fill)
        .build();

    let scale_entry = Entry::builder()
        .placeholder_text("150.0%")
        .width_chars(6)
        .max_width_chars(8)
        .height_request(22)
        .xalign(0.5)
        .css_classes(["scale-input-entry"])
        .valign(Align::Center)
        .build();

    let step_up_btn = Button::builder()
        .label("+")
        .has_frame(false)
        .tooltip_text("Aumentar escala (+5%)")
        .css_classes(["disp-stepper-inner-btn", "disp-stepper-right"])
        .width_request(22)
        .height_request(22)
        .valign(Align::Fill)
        .build();

    let stepper_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .height_request(24)
        .overflow(gtk4::Overflow::Hidden)
        .css_classes(["scale-stepper-shell"])
        .valign(Align::Center)
        .build();
    stepper_box.append(&step_down_btn);
    stepper_box.append(&scale_entry);
    stepper_box.append(&step_up_btn);

    let (apply_scale_wrap, apply_scale_btn) = make_squircle_button("Aplicar", false, None);
    apply_scale_wrap.set_height_request(24);
    apply_scale_btn.set_height_request(24);
    continuous_row.append(&scale_slider);
    continuous_row.append(&stepper_box);
    continuous_row.append(&apply_scale_wrap);
    thumbs_mode_box.append(&continuous_row);
    thumbs_mode_box.append(
        &Label::builder()
            .label("El uso de una resolución ajustada podría afectar al rendimiento del sistema.")
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    main_card.append(&thumbs_mode_box);

    // Contenedor alternativo: Modo Lista de resoluciones (Pantallas3.png)
    let list_mode_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .visible(false)
        .build();
    let list_rows_box = GtkBox::builder().orientation(Orientation::Vertical).build();
    list_mode_box.append(&list_rows_box);
    list_mode_box.append(&make_card_separator());
    let (show_all_res_row, show_all_res_sw) = build_switch_row(
        "Mostrar todas las resoluciones",
        None,
        false,
        false,
        |_| {},
    );
    list_mode_box.append(&show_all_res_row);
    main_card.append(&list_mode_box);

    let main_card_overlay = Overlay::builder()
        .hexpand(true)
        .vexpand(false)
        .valign(Align::Start)
        .build();
    let main_card_bg = DrawingArea::new();
    main_card_bg.set_can_target(false);
    main_card_bg.set_hexpand(true);
    main_card_bg.set_vexpand(false);
    main_card_bg.set_valign(Align::Fill);
    {
        let sel_bg = selected_idx.clone();
        main_card_bg.set_draw_func(move |_, cr, w, h| {
            let width = w as f64;
            let height = h as f64;
            if width <= 2.0 || height <= 10.0 {
                return;
            }
            let cfg = DisplaysConfig::load();
            let has_notch = cfg.outputs.len() > 1;
            let notch_h = if has_notch { 8.0 } else { 0.0 };
            let y0 = 0.5 + notch_h;
            let card_w = (width - 1.0).max(2.0);
            let card_h = (height - 0.5 - y0).max(2.0);
            let x0 = 0.5;
            let radius = WIFI_CARD_SQUIRCLE_RADIUS
                .min(card_w * 0.5)
                .min(card_h * 0.5)
                .max(1.0);
            let power = 2.0 / 4.2_f64;
            let q = std::f64::consts::FRAC_PI_2;
            let segs = 10usize;
            let dark = libadwaita::StyleManager::default().is_dark();

            let _ = cr.save();
            cr.new_path();
            cr.move_to(x0 + radius, y0);
            if has_notch {
                let cur_sel = sel_bg.get().min(cfg.outputs.len().saturating_sub(1));
                let target_cx = compute_stage_slots(&cfg, width)
                    .into_iter()
                    .find(|(idx, ..)| *idx == cur_sel)
                    .map(|(_, cx, _)| cx)
                    .unwrap_or(width * 0.5);
                let half_base = 8.0;
                let nx = target_cx.clamp(
                    x0 + radius + half_base + 2.0,
                    x0 + card_w - radius - half_base - 2.0,
                );
                cr.line_to(nx - half_base, y0);
                cr.line_to(nx, 0.5);
                cr.line_to(nx + half_base, y0);
            }
            cr.line_to(x0 + card_w - radius, y0);
            for step in 1..=segs {
                let a = q * step as f64 / segs as f64;
                cr.line_to(
                    x0 + card_w - radius + radius * a.sin().powf(power),
                    y0 + radius - radius * a.cos().powf(power),
                );
            }
            cr.line_to(x0 + card_w, y0 + card_h - radius);
            for step in 1..=segs {
                let a = q * step as f64 / segs as f64;
                cr.line_to(
                    x0 + card_w - radius + radius * a.cos().powf(power),
                    y0 + card_h - radius + radius * a.sin().powf(power),
                );
            }
            cr.line_to(x0 + radius, y0 + card_h);
            for step in 1..=segs {
                let a = q * step as f64 / segs as f64;
                cr.line_to(
                    x0 + radius - radius * a.sin().powf(power),
                    y0 + card_h - radius + radius * a.cos().powf(power),
                );
            }
            cr.line_to(x0, y0 + radius);
            for step in 1..=segs {
                let a = q * step as f64 / segs as f64;
                cr.line_to(
                    x0 + radius - radius * a.cos().powf(power),
                    y0 + radius - radius * a.sin().powf(power),
                );
            }
            cr.close_path();

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
    }
    let main_card_bg_weak = main_card_bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = main_card_bg_weak.upgrade() {
            area.queue_draw();
        }
    });
    main_card.set_vexpand(false);
    main_card.set_valign(Align::Start);
    main_card_overlay.set_child(Some(&main_card_bg));
    main_card_overlay.add_overlay(&main_card);
    main_card_overlay.set_measure_overlay(&main_card, true);
    cards_col.append(&main_card_overlay);

    // ========================================================================
    // 3. Tarjeta 2: Brillo + Brillo automático + True Tone
    // ========================================================================
    let brightness_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let bright_row = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();

    let bright_top = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    bright_top.append(
        &Label::builder()
            .label("Brillo")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .valign(Align::Center)
            .build(),
    );

    let bright_presets_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let mut bright_preset_buttons: Vec<(f64, Button)> = Vec::new();
    for pct in [25.0_f64, 50.0, 75.0, 100.0] {
        let (pw, pb) = make_squircle_button(
            &format!("{}%", pct as i32),
            false,
            Some(&format!("Fijar brillo al {}%", pct as i32)),
        );
        pb.add_css_class("disp-bright-preset-btn");
        pw.set_height_request(24);
        pb.set_height_request(24);
        bright_presets_box.append(&pw);
        bright_preset_buttons.push((pct, pb));
    }
    bright_top.append(&bright_presets_box);

    let bright_down_btn = Button::builder()
        .label("−")
        .has_frame(false)
        .tooltip_text("Reducir brillo (-5%)")
        .css_classes(["disp-stepper-inner-btn", "disp-stepper-left"])
        .width_request(22)
        .height_request(22)
        .valign(Align::Fill)
        .build();
    let bright_entry = Entry::builder()
        .placeholder_text("100%")
        .width_chars(5)
        .max_width_chars(6)
        .height_request(22)
        .xalign(0.5)
        .css_classes(["scale-input-entry"])
        .valign(Align::Center)
        .build();
    let bright_up_btn = Button::builder()
        .label("+")
        .has_frame(false)
        .tooltip_text("Aumentar brillo (+5%)")
        .css_classes(["disp-stepper-inner-btn", "disp-stepper-right"])
        .width_request(22)
        .height_request(22)
        .valign(Align::Fill)
        .build();
    let bright_stepper_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .height_request(24)
        .overflow(gtk4::Overflow::Hidden)
        .css_classes(["scale-stepper-shell"])
        .valign(Align::Center)
        .build();
    bright_stepper_box.append(&bright_down_btn);
    bright_stepper_box.append(&bright_entry);
    bright_stepper_box.append(&bright_up_btn);
    bright_top.append(&bright_stepper_box);
    bright_row.append(&bright_top);

    let bright_controls = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    bright_controls.append(
        &Image::builder()
            .icon_name("display-brightness-low-symbolic")
            .pixel_size(14)
            .valign(Align::Start)
            .margin_top(10)
            .css_classes(["wp-add-photo-icon"])
            .build(),
    );
    let bright_slider = Scale::with_range(Orientation::Horizontal, 5.0, 100.0, 1.0);
    bright_slider.set_increments(1.0, 5.0);
    bright_slider.set_draw_value(false);
    bright_slider.set_hexpand(true);
    bright_slider.set_width_request(220);
    bright_slider.set_css_classes(&["disp-bright-slider"]);
    for (val, mark_lbl) in [
        (5.0, "5%"),
        (25.0, "25%"),
        (50.0, "50%"),
        (75.0, "75%"),
        (100.0, "100%"),
    ] {
        bright_slider.add_mark(val, gtk4::PositionType::Bottom, Some(mark_lbl));
    }
    bright_controls.append(&bright_slider);
    bright_controls.append(
        &Image::builder()
            .icon_name("display-brightness-symbolic")
            .pixel_size(16)
            .valign(Align::Start)
            .margin_top(9)
            .css_classes(["wp-add-photo-icon"])
            .build(),
    );
    bright_row.append(&bright_controls);
    brightness_card.append(&bright_row);
    brightness_card.append(&make_card_separator());

    let (auto_bright_row, _) = build_switch_row(
        "Ajustar brillo automáticamente",
        None,
        true,
        true,
        |_| {},
    );
    brightness_card.append(&auto_bright_row);
    brightness_card.append(&make_card_separator());

    let (true_tone_row, _) = build_switch_row(
        "True Tone",
        Some("Adapta automáticamente la pantalla para que los colores se vean consistentes en distintas condiciones de luz."),
        true,
        true,
        |_| {},
    );
    brightness_card.append(&true_tone_row);
    cards_col.append(&wrap_squircle_card(
        &brightness_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ========================================================================
    // 4. Tarjeta 3: Enlace Físico 1:1 Pixel a Pixel (TrueSize PPI)
    // ========================================================================
    let truesize_inner = GtkBox::builder().orientation(Orientation::Vertical).build();
    let truesize_top = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let ts_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    ts_text.append(
        &Label::builder()
            .label("Enlace físico 1:1 (TrueSize PPI)")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    ts_text.append(
        &Label::builder()
            .label("Sincroniza la escala según la densidad de píxeles para conservar el tamaño físico entre pantallas.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let ts_switch = Switch::new(true);
    ts_switch.set_valign(Align::Center);
    truesize_top.append(&ts_text);
    truesize_top.append(ts_switch.widget());
    truesize_inner.append(&truesize_top);
    truesize_inner.append(&make_card_separator());

    let ts_presets_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();
    let ts_btns_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let (ts_p1_wrap, ts_p1_btn) =
        make_squircle_button("Ajuste 1 · Equilibrado (150%)", false, None);
    let (ts_p2_wrap, ts_p2_btn) =
        make_squircle_button("Ajuste 2 · Más espacio (133%)", false, None);
    ts_btns_row.append(&ts_p1_wrap);
    ts_btns_row.append(&ts_p2_wrap);
    let ts_math_lbl = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .wrap(true)
        .build();
    ts_presets_box.append(&ts_btns_row);
    ts_presets_box.append(&ts_math_lbl);
    truesize_inner.append(&ts_presets_box);
    let truesize_card_wrap = wrap_squircle_card(&truesize_inner, WIFI_CARD_SQUIRCLE_RADIUS);
    cards_col.append(&truesize_card_wrap);

    // ========================================================================
    // 5. Tarjeta 4: Perfil de color (por implementar)
    // ========================================================================
    let (color_row, ..) = build_apple_dropdown_row(
        "Perfil de color",
        None,
        &COLOR_PROFILE_OPTIONS,
        0,
        true,
        |_| {},
    );
    cards_col.append(&wrap_squircle_card(&color_row, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // 6. Tarjeta 5: Frecuencia de actualización + Alto rango dinámico + Rotación
    // ========================================================================
    let display_tech_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let refresh_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    refresh_row.append(
        &Label::builder()
            .label("Frecuencia de actualización")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let refresh_val_lbl = Label::builder()
        .label("60 Hertz")
        .css_classes(["disp-inline-val"])
        .valign(Align::Center)
        .build();
    let refresh_btn_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    refresh_btn_box.append(&refresh_val_lbl);
    refresh_btn_box.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let refresh_menu_btn = Button::builder()
        .child(&refresh_btn_box)
        .css_classes(["fw-rule-action-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();
    let refresh_popover = Popover::builder()
        .css_classes(["wifi-macos-popover"])
        .has_arrow(false)
        .build();
    refresh_popover.set_parent(&refresh_menu_btn);
    refresh_row.append(&refresh_menu_btn);
    display_tech_card.append(&refresh_row);
    display_tech_card.append(&make_card_separator());

    let (hdr_row, _) = build_switch_row(
        "Alto rango dinámico",
        Some("Ajusta automáticamente la pantalla para mostrar contenido en alto rango dinámico."),
        false,
        true,
        |_| {},
    );
    display_tech_card.append(&hdr_row);
    display_tech_card.append(&make_card_separator());

    let (rot_row, ..) = build_apple_dropdown_row(
        "Rotación",
        None,
        &ROTATION_OPTIONS,
        0,
        true,
        |_| {},
    );
    display_tech_card.append(&rot_row);
    cards_col.append(&wrap_squircle_card(
        &display_tech_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ========================================================================
    // 7. Tarjeta 6: Al conectarse a la TV (por implementar)
    // ========================================================================
    let (tv_row, ..) = build_apple_dropdown_row(
        "Al conectarse a la TV",
        Some("Elige qué mostrar o usa la TV como una pantalla secundaria."),
        &TV_CONNECT_OPTIONS,
        0,
        true,
        |_| {},
    );
    cards_col.append(&wrap_squircle_card(&tv_row, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // 8. Barra inferior derecha: Avanzado... | Night Shift... | ?
    // ========================================================================
    let bottom_actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(4)
        .build();

    let (adv_wrap, adv_btn) = make_squircle_button("Avanzado...", false, None);
    let (ns_wrap, ns_btn) = make_squircle_button("Night Shift...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Información sobre Pantallas y TrueSize PPI");
    let help_pop = Popover::builder()
        .css_classes(["wifi-macos-popover"])
        .has_arrow(false)
        .build();
    help_pop.set_child(Some(
        &Label::builder()
            .label("Selecciona una pantalla en la parte superior para ajustar su resolución, frecuencia, brillo y escala.\nUsa «Ordenar...» para reorganizar monitores físicos y «Enlace 1:1» para igualar milímetros físicos entre pantallas.")
            .css_classes(["card-section-sub"])
            .max_width_chars(38)
            .wrap(true)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build(),
    ));
    help_btn.set_popover(Some(&help_pop));

    bottom_actions.append(&adv_wrap);
    bottom_actions.append(&ns_wrap);
    bottom_actions.append(&help_wrap);
    cards_col.append(&bottom_actions);
    pane.append(&cards_col);

    scrolled.set_child(Some(&pane));

    // ========================================================================
    // Refresco completo sincronizado con DisplaysConfig + DRM
    // ========================================================================
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let refresh_all: Rc<dyn Fn()> = {
        let sel_c = selected_idx.clone();
        let sync_c = syncing_ui.clone();
        let stage_c = stage_area.clone();
        let main_bg_c = main_card_bg.clone();
        let main_card_c = main_card.clone();
        let cards_col_c = cards_col.clone();
        let wp_cache_c = wallpaper_thumb_cache.clone();
        let arrange_wrap_c = arrange_wrap.clone();
        let arrange_c = arrange_btn.clone();
        let use_as_row_c = use_as_row.clone();
        let use_as_sep_c = use_as_sep.clone();
        let use_as_lbl_c = use_as_val_lbl.clone();
        let use_as_pop_c = use_as_popover.clone();
        let use_as_btn_c = use_as_btn.clone();
        let thumbs_box_c = thumbs_mode_box.clone();
        let list_box_c = list_mode_box.clone();
        let thumbs_row_c = thumbs_row.clone();
        let list_rows_c = list_rows_box.clone();
        let res_lbl_c = active_res_lbl.clone();
        let sld_c = scale_slider.clone();
        let ent_c = scale_entry.clone();
        let show_all_sw_c = show_all_res_sw.clone();
        let show_all_flag_c = show_all_res_flag(&show_all_modes);
        let bright_sld_c = bright_slider.clone();
        let bright_ent_c = bright_entry.clone();
        let bright_presets_c = bright_preset_buttons.clone();
        let ts_wrap_c = truesize_card_wrap.clone();
        let ts_sw_c = ts_switch.clone();
        let ts_p1_c = ts_p1_btn.clone();
        let ts_p2_c = ts_p2_btn.clone();
        let ts_math_c = ts_math_lbl.clone();
        let ref_val_c = refresh_val_lbl.clone();
        let ref_pop_c = refresh_popover.clone();
        let slot_c = refresh_slot.clone();
        Rc::new(move || {
            sync_c.set(true);
            *wp_cache_c.borrow_mut() = load_hero_wallpaper_pixbuf();
            let cfg = DisplaysConfig::load();
            if cfg.outputs.is_empty() {
                arrange_wrap_c.set_visible(false);
                arrange_c.set_sensitive(false);
                use_as_row_c.set_visible(false);
                use_as_sep_c.set_visible(false);
                main_card_c.set_margin_top(0);
                stage_c.queue_draw();
                main_bg_c.queue_draw();
                sync_c.set(false);
                return;
            }
            let multi = cfg.outputs.len() > 1;
            let idx = sel_c.get().min(cfg.outputs.len().saturating_sub(1));
            sel_c.set(idx);
            let output = &cfg.outputs[idx];

            arrange_wrap_c.set_visible(multi);
            arrange_c.set_sensitive(multi);
            use_as_row_c.set_visible(multi);
            use_as_sep_c.set_visible(multi);
            main_card_c.set_margin_top(if multi { 8 } else { 0 });
            cards_col_c.set_margin_top(if multi { 6 } else { 14 });
            stage_c.queue_draw();
            main_bg_c.queue_draw();

            // 1. "Usar como" (Pantalla principal / Pantalla extendida)
            use_as_lbl_c.set_label(if output.primary {
                "Pantalla principal"
            } else {
                "Pantalla extendida"
            });
            use_as_btn_c.set_sensitive(multi);

            let ua_pop_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();
            let (row_prim, b_prim) = make_popover_action_row(
                "Pantalla principal",
                true,
                if output.primary { Some("✓") } else { None },
            );
            {
                let pop_c = use_as_pop_c.clone();
                let sl_c = slot_c.clone();
                b_prim.connect_clicked(move |_| {
                    let mut next = DisplaysConfig::load();
                    if idx < next.outputs.len() {
                        next.set_primary_output(idx);
                        save_displays_and_reload(&next);
                    }
                    pop_c.popdown();
                    if let Some(ref rf) = *sl_c.borrow() {
                        rf();
                    }
                });
            }
            ua_pop_box.append(&row_prim);

            if cfg.outputs.len() > 1 {
                let (row_ext, b_ext) = make_popover_action_row(
                    "Pantalla extendida",
                    true,
                    if !output.primary { Some("✓") } else { None },
                );
                let pop_c = use_as_pop_c.clone();
                let sl_c = slot_c.clone();
                b_ext.connect_clicked(move |_| {
                    let mut next = DisplaysConfig::load();
                    if next.outputs.len() > 1 && next.outputs.get(idx).map(|o| o.primary).unwrap_or(false) {
                        let other = if idx == 0 { 1 } else { 0 };
                        next.set_primary_output(other);
                        save_displays_and_reload(&next);
                    }
                    pop_c.popdown();
                    if let Some(ref rf) = *sl_c.borrow() {
                        rf();
                    }
                });
                ua_pop_box.append(&row_ext);
            }
            use_as_pop_c.set_child(Some(&ua_pop_box));

            // 2. Modo Miniaturas vs Modo Lista (Avanzado -> show_resolutions_as_list)
            let as_list = cfg.show_resolutions_as_list;
            thumbs_box_c.set_visible(!as_list);
            list_box_c.set_visible(as_list);

            while let Some(ch) = thumbs_row_c.first_child() {
                thumbs_row_c.remove(&ch);
            }
            let scales = [2.0_f64, 1.75, 1.5, 1.25, 1.0];
            let rec_scale = DisplayOutputConfig::recommended_scale(
                output.width,
                output.height,
                output.diagonal_inches,
            );
            let total_steps = scales.len();
            let accent = crate::accent_color();
            let is_dark = is_dark_mode_active();

            for (step, &sc) in scales.iter().enumerate() {
                let is_sel = (output.scale - sc).abs() < 0.03;
                let scale_pct = (sc * 100.0).round() as i32;
                let (lw, lh) = (
                    (output.width as f64 / sc).round() as i32,
                    (output.height as f64 / sc).round() as i32,
                );
                let btn = Button::builder()
                    .css_classes(["wp-tile-btn"])
                    .has_frame(false)
                    .tooltip_text(&format!("{} × {} ({}%)", lw, lh, scale_pct))
                    .build();
                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(5)
                    .width_request(92)
                    .build();
                let thumb = DrawingArea::builder()
                    .content_width(88)
                    .content_height(60)
                    .build();
                thumb.set_draw_func(move |_, cr, ww, hh| {
                    draw_scale_thumbnail_window(
                        cr,
                        ww as f64,
                        hh as f64,
                        scale_pct,
                        is_sel,
                        is_dark,
                        accent,
                    );
                });
                let caption = if step == 0 {
                    "Texto más grande".to_string()
                } else if step + 1 == total_steps {
                    "Más espacio".to_string()
                } else if (sc - rec_scale).abs() < 0.03 {
                    "Predeterminada".to_string()
                } else {
                    String::new()
                };
                col.append(&thumb);
                col.append(
                    &Label::builder()
                        .label(&caption)
                        .css_classes([if is_sel {
                            "app-thumb-label-active"
                        } else {
                            "wp-tile-label"
                        }])
                        .xalign(0.5)
                        .wrap(true)
                        .justify(gtk4::Justification::Center)
                        .build(),
                );
                btn.set_child(Some(&col));
                let sl_c = slot_c.clone();
                btn.connect_clicked(move |_| {
                    let mut next = DisplaysConfig::load();
                    next.set_output_scale_independent(idx, sc);
                    save_displays_and_reload(&next);
                    if let Some(ref rf) = *sl_c.borrow() {
                        rf();
                    }
                });
                thumbs_row_c.append(&btn);
            }

            let (cur_lw, cur_lh) = output.logical_size();
            res_lbl_c.set_label(&format!(
                "{} × {} ({:.1}%)",
                cur_lw,
                cur_lh,
                output.scale * 100.0
            ));
            sld_c.set_value((output.scale * 100.0).clamp(100.0, 250.0));
            ent_c.set_text(&format!("{:.1}%", output.scale * 100.0));

            // Lista de resoluciones (cuando show_resolutions_as_list == true)
            while let Some(ch) = list_rows_c.first_child() {
                list_rows_c.remove(&ch);
            }
            let show_all = show_all_flag_c.get();
            show_all_sw_c.set_active(show_all);

            if show_all && !output.modes.is_empty() {
                let mut unique_modes: Vec<DisplayModeConfig> = Vec::new();
                for m in &output.modes {
                    if !unique_modes
                        .iter()
                        .any(|u| u.width == m.width && u.height == m.height)
                    {
                        unique_modes.push(*m);
                    }
                }
                for m in unique_modes {
                    let active = m.width == output.width && m.height == output.height;
                    let text = format!("{} × {}", m.width, m.height);
                    let r_btn = make_resolution_list_row(&text, active);
                    let sl_c = slot_c.clone();
                    r_btn.connect_clicked(move |_| {
                        let mut next = DisplaysConfig::load();
                        if let Some(out) = next.outputs.get_mut(idx) {
                            let old_lw = out.width as f64 / out.scale.max(0.5);
                            out.width = m.width;
                            out.height = m.height;
                            out.refresh_rate = m.refresh_rate;
                            out.scale = (m.width as f64 / old_lw).clamp(0.5, 3.0);
                            if out.primary {
                                next.master_scale = out.scale;
                            }
                            next.linked_physical_1to1 = false;
                            next.recalculate_layout();
                            save_displays_and_reload(&next);
                        }
                        if let Some(ref rf) = *sl_c.borrow() {
                            rf();
                        }
                    });
                    list_rows_c.append(&r_btn);
                }
            } else {
                for &sc in &scales {
                    let (lw, lh) = (
                        (output.width as f64 / sc).round() as i32,
                        (output.height as f64 / sc).round() as i32,
                    );
                    let is_def = (sc - rec_scale).abs() < 0.03;
                    let active = (output.scale - sc).abs() < 0.03;
                    let text = if is_def {
                        format!("{} × {} (predeterminada)", lw, lh)
                    } else {
                        format!("{} × {}", lw, lh)
                    };
                    let r_btn = make_resolution_list_row(&text, active);
                    let sl_c = slot_c.clone();
                    r_btn.connect_clicked(move |_| {
                        let mut next = DisplaysConfig::load();
                        next.set_output_scale_independent(idx, sc);
                        save_displays_and_reload(&next);
                        if let Some(ref rf) = *sl_c.borrow() {
                            rf();
                        }
                    });
                    list_rows_c.append(&r_btn);
                }
            }

            // 3. Brillo (hardware sysfs o por pantalla en el compositor)
            let cur_bright = read_output_brightness(&cfg, output);
            bright_sld_c.set_value(cur_bright);
            bright_ent_c.set_text(&format!("{}%", cur_bright.round() as i32));
            for (preset_val, pb) in &bright_presets_c {
                if (cur_bright - *preset_val).abs() < 0.6 {
                    pb.add_css_class("truesize-preset-btn-active");
                } else {
                    pb.remove_css_class("truesize-preset-btn-active");
                }
            }

            // 4. Enlace físico 1:1 TrueSize PPI
            ts_wrap_c.set_visible(cfg.outputs.len() >= 2);
            ts_sw_c.set_active(cfg.linked_physical_1to1);
            let preset = cfg.current_linked_preset();
            if preset == Some(1) {
                ts_p1_c.add_css_class("truesize-preset-btn-active");
                ts_p2_c.remove_css_class("truesize-preset-btn-active");
            } else if preset == Some(2) {
                ts_p2_c.add_css_class("truesize-preset-btn-active");
                ts_p1_c.remove_css_class("truesize-preset-btn-active");
            } else {
                ts_p1_c.remove_css_class("truesize-preset-btn-active");
                ts_p2_c.remove_css_class("truesize-preset-btn-active");
            }
            if cfg.outputs.len() >= 2 {
                let o0 = &cfg.outputs[0];
                let o1 = &cfg.outputs[1];
                if let (Some(ppi0), Some(ppi1)) = (o0.ppi(), o1.ppi()) {
                    let mm0 = (100.0 * o0.scale / ppi0) * 25.4;
                    let mm1 = (100.0 * o1.scale / ppi1) * 25.4;
                    ts_math_c.set_label(&format!(
                        "Regla Física: 100 pt lógicos = {:.1} mm en {} ({:.0}%) · {:.1} mm en {} ({:.0}%)",
                        mm0, o0.connector, o0.scale * 100.0, mm1, o1.connector, o1.scale * 100.0
                    ));
                } else {
                    ts_math_c.set_label(
                        "Falta el tamaño físico real (EDID) en al menos una pantalla para calcular 1:1.",
                    );
                }
            }

            // 5. Frecuencia de actualización (desde output.modes del DRM)
            let mut rates: Vec<f64> = output
                .modes
                .iter()
                .filter(|m| m.width == output.width && m.height == output.height)
                .map(|m| m.refresh_rate)
                .collect();
            if rates.is_empty() {
                rates.push(output.refresh_rate);
            }
            rates.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            rates.dedup_by(|a, b| (*a - *b).abs() < 0.5);

            ref_val_c.set_label(&format!("{:.0} Hertz", output.refresh_rate));
            let r_pop_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();
            for hz in rates {
                let active = (hz - output.refresh_rate).abs() < 0.5;
                let (r_overlay, rb) = make_popover_action_row(
                    &format!("{:.0} Hertz", hz),
                    true,
                    if active { Some("✓") } else { None },
                );
                let pop_c = ref_pop_c.clone();
                let sl_c = slot_c.clone();
                rb.connect_clicked(move |_| {
                    let mut next = DisplaysConfig::load();
                    if let Some(out) = next.outputs.get_mut(idx) {
                        out.refresh_rate = hz;
                        save_displays_and_reload(&next);
                    }
                    pop_c.popdown();
                    if let Some(ref rf) = *sl_c.borrow() {
                        rf();
                    }
                });
                r_pop_box.append(&r_overlay);
            }
            ref_pop_c.set_child(Some(&r_pop_box));

            sync_c.set(false);
        })
    };

    *refresh_slot.borrow_mut() = Some(refresh_all.clone());

    // Clic en el escenario superior para cambiar de monitor seleccionado
    {
        let click = GestureClick::new();
        let hits_c = stage_hitboxes.clone();
        let sel_c = selected_idx.clone();
        let ref_c = refresh_all.clone();
        click.connect_pressed(move |_, _, x, y| {
            let hit = hits_c
                .borrow()
                .iter()
                .find(|&&(_, rx, ry, rw, rh)| x >= rx && x <= rx + rw && y >= ry && y <= ry + rh)
                .map(|&(idx, ..)| idx);
            if let Some(idx) = hit {
                if sel_c.get() != idx {
                    sel_c.set(idx);
                    ref_c();
                }
            }
        });
        stage_area.add_controller(click);
    }

    // Popovers "Usar como" y "Frecuencia de actualización"
    {
        let pop = use_as_popover.clone();
        use_as_btn.connect_clicked(move |_| pop.popup());
    }
    {
        let pop = refresh_popover.clone();
        refresh_menu_btn.connect_clicked(move |_| pop.popup());
    }

    // Switch "Mostrar todas las resoluciones"
    {
        let sync_c = syncing_ui.clone();
        let flag_c = show_all_modes.clone();
        let ref_c = refresh_all.clone();
        show_all_res_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                flag_c.set(active);
                ref_c();
            }
            gtk4::glib::Propagation::Proceed
        });
    }

    // Escala continua + Entry exacto
    {
        let sync_c = syncing_ui.clone();
        let sel_c = selected_idx.clone();
        let ref_c = refresh_all.clone();
        scale_slider.connect_value_changed(move |sld| {
            if sync_c.get() {
                return;
            }
            let pct = (sld.value() / 2.5).round() * 2.5;
            let next_scale = (pct / 100.0).clamp(0.5, 3.0);
            let mut cfg = DisplaysConfig::load();
            cfg.set_output_scale_independent(sel_c.get(), next_scale);
            save_displays_and_reload(&cfg);
            ref_c();
        });
    }
    let apply_entry_scale: Rc<dyn Fn()> = {
        let sync_c = syncing_ui.clone();
        let sel_c = selected_idx.clone();
        let ent_c = scale_entry.clone();
        let ref_c = refresh_all.clone();
        Rc::new(move || {
            if sync_c.get() {
                return;
            }
            let text = ent_c
                .text()
                .trim()
                .trim_end_matches('%')
                .trim()
                .to_string();
            if let Ok(mut val) = text.parse::<f64>() {
                if val <= 0.0 {
                    return;
                }
                if val > 4.0 {
                    val /= 100.0;
                }
                let next_scale = (val.clamp(0.5, 3.0) * 10000.0).round() / 10000.0;
                let mut cfg = DisplaysConfig::load();
                cfg.set_output_scale_independent(sel_c.get(), next_scale);
                save_displays_and_reload(&cfg);
                ref_c();
            }
        })
    };
    {
        let f = apply_entry_scale.clone();
        scale_entry.connect_activate(move |_| f());
    }
    {
        let f = apply_entry_scale;
        apply_scale_btn.connect_clicked(move |_| f());
    }
    let step_scale_delta = {
        let sync_c = syncing_ui.clone();
        let sel_c = selected_idx.clone();
        let ref_c = refresh_all.clone();
        Rc::new(move |delta_pct: f64| {
            if sync_c.get() {
                return;
            }
            let mut cfg = DisplaysConfig::load();
            let idx = sel_c.get();
            if let Some(out) = cfg.outputs.get(idx) {
                let next_pct = (out.scale * 100.0 + delta_pct).round().clamp(50.0, 300.0);
                let next_scale = ((next_pct / 100.0) * 10000.0).round() / 10000.0;
                cfg.set_output_scale_independent(idx, next_scale);
                save_displays_and_reload(&cfg);
                ref_c();
            }
        })
    };
    {
        let step = step_scale_delta.clone();
        step_down_btn.connect_clicked(move |_| step(-5.0));
    }
    {
        let step = step_scale_delta;
        step_up_btn.connect_clicked(move |_| step(5.0));
    }

    // Brillo fluido (worker en segundo plano) + escala, botones paso ±5%, entrada y presets
    let apply_brightness_from_ui: Rc<dyn Fn(f64, bool)> = {
        let sync_c = syncing_ui.clone();
        let sel_c = selected_idx.clone();
        let sld_c = bright_slider.clone();
        let ent_c = bright_entry.clone();
        let presets_c = bright_preset_buttons.clone();
        Rc::new(move |pct: f64, update_slider: bool| {
            let clamped = pct.clamp(5.0, 100.0);
            if update_slider {
                let prev_sync = sync_c.get();
                sync_c.set(true);
                sld_c.set_value(clamped);
                sync_c.set(prev_sync);
            }
            ent_c.set_text(&format!("{}%", clamped.round() as i32));
            for (preset_val, pb) in &presets_c {
                if (clamped - *preset_val).abs() < 0.6 {
                    pb.add_css_class("truesize-preset-btn-active");
                } else {
                    pb.remove_css_class("truesize-preset-btn-active");
                }
            }
            schedule_output_brightness(sel_c.get(), clamped);
        })
    };
    {
        let sync_c = syncing_ui.clone();
        let apply_b = apply_brightness_from_ui.clone();
        bright_slider.connect_value_changed(move |sld| {
            if sync_c.get() {
                return;
            }
            apply_b(sld.value(), false);
        });
    }
    for (preset_val, pb) in &bright_preset_buttons {
        let apply_b = apply_brightness_from_ui.clone();
        let target = *preset_val;
        pb.connect_clicked(move |_| {
            apply_b(target, true);
        });
    }
    {
        let apply_b = apply_brightness_from_ui.clone();
        let sld_c = bright_slider.clone();
        bright_down_btn.connect_clicked(move |_| {
            apply_b((sld_c.value() - 5.0).round(), true);
        });
    }
    {
        let apply_b = apply_brightness_from_ui.clone();
        let sld_c = bright_slider.clone();
        bright_up_btn.connect_clicked(move |_| {
            apply_b((sld_c.value() + 5.0).round(), true);
        });
    }
    {
        let apply_b = apply_brightness_from_ui;
        bright_entry.connect_activate(move |ent| {
            let txt = ent.text().trim().trim_end_matches('%').trim().to_string();
            if let Ok(val) = txt.parse::<f64>() {
                apply_b(val, true);
            }
        });
    }

    // Enlace 1:1 TrueSize PPI + Presets
    {
        let sync_c = syncing_ui.clone();
        let ref_c = refresh_all.clone();
        ts_switch.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut cfg = DisplaysConfig::load();
                cfg.linked_physical_1to1 = active;
                if active {
                    if cfg.current_linked_preset().is_none() {
                        cfg.apply_linked_preset(1);
                    } else {
                        cfg.apply_1to1_link();
                    }
                } else {
                    cfg.recalculate_layout();
                }
                save_displays_and_reload(&cfg);
                ref_c();
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    {
        let ref_c = refresh_all.clone();
        ts_p1_btn.connect_clicked(move |_| {
            let mut cfg = DisplaysConfig::load();
            cfg.apply_linked_preset(1);
            save_displays_and_reload(&cfg);
            ref_c();
        });
    }
    {
        let ref_c = refresh_all.clone();
        ts_p2_btn.connect_clicked(move |_| {
            let mut cfg = DisplaysConfig::load();
            cfg.apply_linked_preset(2);
            save_displays_and_reload(&cfg);
            ref_c();
        });
    }

    // Modales: Ordenar..., Avanzado..., Night Shift...
    {
        let win_c = window.clone();
        let sel_c = selected_idx.clone();
        let ref_c = refresh_all.clone();
        arrange_btn.connect_clicked(move |_| {
            show_arrange_modal(&win_c, sel_c.get(), ref_c.clone());
        });
    }
    {
        let win_c = window.clone();
        let ref_c = refresh_all.clone();
        adv_btn.connect_clicked(move |_| {
            show_advanced_modal(&win_c, ref_c.clone());
        });
    }
    {
        let win_c = window.clone();
        ns_btn.connect_clicked(move |_| {
            show_night_shift_modal(&win_c);
        });
    }

    // Monitorización reactiva de monitores GDK y monitors.toml
    if let Some(display) = gtk4::gdk::Display::default() {
        let ref_mon = refresh_all.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            let r = ref_mon.clone();
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                r();
            });
        });
    }
    let cfg_monitor_file = gio::File::for_path(
        malus_de_common::config::malus_wallpaper_config_dir().join("monitors.toml"),
    );
    if let Ok(file_mon) =
        cfg_monitor_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
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

    refresh_all();
    (scrolled, refresh_all)
}

fn show_all_res_flag(flag: &Rc<Cell<bool>>) -> Rc<Cell<bool>> {
    flag.clone()
}

fn make_resolution_list_row(label: &str, active: bool) -> Button {
    let box_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();
    box_row.append(
        &Label::builder()
            .label(label)
            .css_classes([if active {
                "app-thumb-label-active"
            } else {
                "wifi-row-title"
            }])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    if active {
        box_row.append(
            &Image::builder()
                .icon_name("object-select-symbolic")
                .pixel_size(14)
                .css_classes(["app-theme-active-tag"])
                .build(),
        );
    }
    Button::builder()
        .child(&box_row)
        .css_classes([if active {
            "disp-res-list-row-active"
        } else {
            "net-service-row"
        }])
        .has_frame(false)
        .hexpand(true)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_builtin_connectors_and_formats_names() {
        let edp = DisplayOutputConfig {
            connector: "eDP-1".to_string(),
            name: "eDP-1".to_string(),
            width: 2880,
            height: 1800,
            refresh_rate: 120.0,
            modes: vec![],
            x: 0,
            y: 0,
            scale: 2.0,
            primary: true,
            brightness: 100.0,
            diagonal_inches: Some(14.0),
        };
        assert_eq!(friendly_display_name(&edp), "Pantalla integrada");

        let dp = DisplayOutputConfig {
            connector: "DP-1".to_string(),
            name: "Mi Monitor".to_string(),
            ..edp
        };
        assert_eq!(friendly_display_name(&dp), "Mi Monitor");
    }

    #[test]
    fn displays_pane_builds_under_gtk_init() {
        if gtk4::init().is_err() {
            return;
        }
        let _ = libadwaita::init();
        let app = gtk4::Application::builder()
            .application_id("org.malus_de.SettingsTestDisplays")
            .build();
        let win = ApplicationWindow::builder().application(&app).build();
        let (scrolled, refresh) = create_displays_pane(&win);
        refresh();
        assert!(scrolled.child().is_some());
    }

    #[test]
    fn adjust_vertical_offset_moves_display_by_exact_pixels() {
        let out0 = DisplayOutputConfig {
            connector: "eDP-1".to_string(),
            name: "eDP-1".to_string(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            modes: vec![],
            x: 0,
            y: 0,
            scale: 1.0,
            primary: true,
            brightness: 100.0,
            diagonal_inches: Some(14.0),
        };
        let out1 = DisplayOutputConfig {
            connector: "HDMI-A-1".to_string(),
            name: "HDMI-A-1".to_string(),
            x: 1920,
            primary: false,
            ..out0.clone()
        };
        let mut cfg = DisplaysConfig {
            outputs: vec![out0, out1],
            master_scale: 1.0,
            linked_physical_1to1: false,
            ..Default::default()
        };

        // Bajar pantalla secundaria 50 px
        cfg.adjust_vertical_offset(1, 50);
        assert_eq!(cfg.outputs[0].y, 0);
        assert_eq!(cfg.outputs[1].y, 50);

        // Subir pantalla secundaria 10 px
        cfg.adjust_vertical_offset(1, -10);
        assert_eq!(cfg.outputs[0].y, 0);
        assert_eq!(cfg.outputs[1].y, 40);

        // Subir pantalla primaria cuando ya estaba en y=0 desplaza hacia abajo a la otra
        cfg.adjust_vertical_offset(0, -10);
        assert_eq!(cfg.outputs[0].y, 0);
        assert_eq!(cfg.outputs[1].y, 50);
    }
}
