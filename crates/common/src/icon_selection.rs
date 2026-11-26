//! Resaltado de selección de iconos estilo macOS Tahoe Finder (Branches y Escritorio).
//!
//! Dibuja en Cairo:
//! 1. Una caja squircle cuadrada (`n = 4.2`) exclusivamente alrededor de la miniatura/icono
//!    (gris suave en ventana de Branches; oscura translúcida con borde hairline claro en Escritorio).
//! 2. Una píldora azul squircle ajustada línea por línea alrededor del texto de la etiqueta,
//!    con uniones continuas y filetes cóncavos suaves cuando la etiqueta ocupa varias líneas.

use gtk4::cairo;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, DrawingArea, Label, Orientation, Overlay};

const SQUIRCLE_EXPONENT: f64 = 4.2;
const SQUIRCLE_SEGMENTS: usize = 16;
const MAX_LABEL_LINES: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSelectionTheme {
    /// Vista de iconos dentro de Branches (fondo sólido de ventana).
    WindowCanvas,
    /// Iconos sobre el fondo de pantalla del Escritorio.
    DesktopWallpaper,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinePillRect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Traza un rectángulo squircle (`n = 4.2`) con control independiente de las 4 esquinas.
#[allow(clippy::too_many_arguments)]
pub fn squircle_path_4corners(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    r_tl: bool,
    r_tr: bool,
    r_br: bool,
    r_bl: bool,
) {
    let max_r = (width * 0.5).min(height * 0.5).max(0.0);
    let tl = if r_tl { radius.min(max_r).max(0.5) } else { 0.0 };
    let tr = if r_tr { radius.min(max_r).max(0.5) } else { 0.0 };
    let br = if r_br { radius.min(max_r).max(0.5) } else { 0.0 };
    let bl = if r_bl { radius.min(max_r).max(0.5) } else { 0.0 };
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter = std::f64::consts::FRAC_PI_2;

    cr.new_path();
    cr.move_to(x + tl, y);
    cr.line_to(x + width - tr, y);

    if tr > 0.0 {
        for step in 1..=SQUIRCLE_SEGMENTS {
            let a = quarter * step as f64 / SQUIRCLE_SEGMENTS as f64;
            cr.line_to(
                x + width - tr + tr * a.sin().powf(power),
                y + tr - tr * a.cos().powf(power),
            );
        }
    }
    cr.line_to(x + width, y + height - br);

    if br > 0.0 {
        for step in 1..=SQUIRCLE_SEGMENTS {
            let a = quarter * step as f64 / SQUIRCLE_SEGMENTS as f64;
            cr.line_to(
                x + width - br + br * a.cos().powf(power),
                y + height - br + br * a.sin().powf(power),
            );
        }
    }
    cr.line_to(x + bl, y + height);

    if bl > 0.0 {
        for step in 1..=SQUIRCLE_SEGMENTS {
            let a = quarter * step as f64 / SQUIRCLE_SEGMENTS as f64;
            cr.line_to(
                x + bl - bl * a.sin().powf(power),
                y + height - bl + bl * a.cos().powf(power),
            );
        }
    }
    cr.line_to(x, y + tl);

    if tl > 0.0 {
        for step in 1..=SQUIRCLE_SEGMENTS {
            let a = quarter * step as f64 / SQUIRCLE_SEGMENTS as f64;
            cr.line_to(
                x + tl - tl * a.cos().powf(power),
                y + tl - tl * a.sin().powf(power),
            );
        }
    }
    cr.close_path();
}

/// Normaliza rectángulos de líneas consecutivas para eliminar huecos verticales entre líneas
/// y unificar líneas con anchos prácticamente idénticos.
pub fn normalize_line_pill_rects(rects: &mut [LinePillRect]) {
    for i in 0..rects.len().saturating_sub(1) {
        let mid_y = (rects[i].y1 + rects[i + 1].y0) * 0.5;
        rects[i].y1 = mid_y;
        rects[i + 1].y0 = mid_y;

        if (rects[i].x0 - rects[i + 1].x0).abs() < 3.5 {
            let min_x = rects[i].x0.min(rects[i + 1].x0);
            rects[i].x0 = min_x;
            rects[i + 1].x0 = min_x;
        }
        if (rects[i].x1 - rects[i + 1].x1).abs() < 3.5 {
            let max_x = rects[i].x1.max(rects[i + 1].x1);
            rects[i].x1 = max_x;
            rects[i + 1].x1 = max_x;
        }
    }
}

/// Dibuja la píldora azul multilínea estilo macOS Finder con squircles externos y filetes cóncavos internos.
pub fn draw_multiline_squircle_highlight(
    cr: &cairo::Context,
    raw_rects: &[LinePillRect],
    accent_rgb: (f64, f64, f64),
) {
    if raw_rects.is_empty() {
        return;
    }
    let mut rects = raw_rects.to_vec();
    normalize_line_pill_rects(&mut rects);

    let (r_col, g_col, b_col) = accent_rgb;
    cr.set_source_rgba(r_col, g_col, b_col, 0.96);

    let n = rects.len();
    for i in 0..n {
        let cur = rects[i];
        let w = (cur.x1 - cur.x0).max(2.0);
        let h = (cur.y1 - cur.y0).max(2.0);
        let radius = (h * 0.36).clamp(4.5, 7.5);

        let prev = if i > 0 { Some(rects[i - 1]) } else { None };
        let next = if i + 1 < n { Some(rects[i + 1]) } else { None };

        let r_tl = prev.map_or(true, |p| p.x0 > cur.x0 + 1.0);
        let r_tr = prev.map_or(true, |p| p.x1 < cur.x1 - 1.0);
        let r_bl = next.map_or(true, |nx| nx.x0 > cur.x0 + 1.0);
        let r_br = next.map_or(true, |nx| nx.x1 < cur.x1 - 1.0);

        squircle_path_4corners(cr, cur.x0, cur.y0, w, h, radius, r_tl, r_tr, r_br, r_bl);
        let _ = cr.fill();
    }

    // Filetes cóncavos suaves en las transiciones de ancho entre líneas contiguas
    for i in 0..n.saturating_sub(1) {
        let a = rects[i];
        let b = rects[i + 1];
        let seam_y = a.y1;
        let max_h = (a.y1 - a.y0).min(b.y1 - b.y0) * 0.42;

        // Izquierda: línea superior más estrecha que la inferior
        if a.x0 > b.x0 + 2.5 {
            let fr = ((a.x0 - b.x0) * 0.45).min(max_h).min(4.0);
            cr.new_path();
            cr.move_to(a.x0 + 0.5, seam_y + 0.5);
            cr.line_to(a.x0 - fr, seam_y + 0.5);
            cr.curve_to(a.x0, seam_y, a.x0, seam_y, a.x0 + 0.5, seam_y - fr);
            cr.close_path();
            let _ = cr.fill();
        }
        // Derecha: línea superior más estrecha que la inferior
        if a.x1 < b.x1 - 2.5 {
            let fr = ((b.x1 - a.x1) * 0.45).min(max_h).min(4.0);
            cr.new_path();
            cr.move_to(a.x1 - 0.5, seam_y + 0.5);
            cr.line_to(a.x1 + fr, seam_y + 0.5);
            cr.curve_to(a.x1, seam_y, a.x1, seam_y, a.x1 - 0.5, seam_y - fr);
            cr.close_path();
            let _ = cr.fill();
        }
        // Izquierda: línea inferior más estrecha que la superior
        if b.x0 > a.x0 + 2.5 {
            let fr = ((b.x0 - a.x0) * 0.45).min(max_h).min(4.0);
            cr.new_path();
            cr.move_to(b.x0 + 0.5, seam_y - 0.5);
            cr.line_to(b.x0 - fr, seam_y - 0.5);
            cr.curve_to(b.x0, seam_y, b.x0, seam_y, b.x0 + 0.5, seam_y + fr);
            cr.close_path();
            let _ = cr.fill();
        }
        // Derecha: línea inferior más estrecha que la superior
        if b.x1 < a.x1 - 2.5 {
            let fr = ((a.x1 - b.x1) * 0.45).min(max_h).min(4.0);
            cr.new_path();
            cr.move_to(b.x1 - 0.5, seam_y - 0.5);
            cr.line_to(b.x1 + fr, seam_y - 0.5);
            cr.curve_to(b.x1, seam_y, b.x1, seam_y, b.x1 - 0.5, seam_y + fr);
            cr.close_path();
            let _ = cr.fill();
        }
    }
}

/// Construye el interior de un icono (Branches o Escritorio) dentro de un `Overlay` con fondo Cairo
/// que pinta la caja squircle cuadrada alrededor del icono y la píldora azul por línea bajo `label`.
///
/// Mantiene `label` como hijo directo de un `GtkBox` vertical para preservar compatibilidad total
/// con la edición inline de nombres.
pub fn populate_icon_with_macos_selection<W: IsA<gtk4::Widget>>(
    outer_box: &GtkBox,
    icon_widget: &W,
    icon_size: i32,
    label: &Label,
    extra_bottom: Option<&gtk4::Widget>,
    theme: IconSelectionTheme,
) {
    let icon_slot = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .halign(Align::Center)
        .valign(Align::Center)
        .width_request(icon_size)
        .height_request(icon_size)
        .css_classes(["malus-icon-slot"])
        .build();
    icon_widget.as_ref().set_halign(Align::Center);
    icon_widget.as_ref().set_valign(Align::Center);
    icon_widget.as_ref().set_vexpand(true);
    icon_slot.append(icon_widget);

    let inner_vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .halign(Align::Fill)
        .valign(Align::Start)
        .hexpand(true)
        .margin_top(6)
        .margin_bottom(4)
        .margin_start(6)
        .margin_end(6)
        .build();
    inner_vbox.append(&icon_slot);
    inner_vbox.append(label);
    if let Some(extra) = extra_bottom {
        inner_vbox.append(extra);
    }

    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let outer_weak = outer_box.downgrade();
    let slot_weak = icon_slot.downgrade();
    let label_weak = label.downgrade();

    bg.set_draw_func(move |area, cr, w, h| {
        let area_w = w as f64;
        let area_h = h as f64;
        if area_w <= 2.0 || area_h <= 2.0 {
            return;
        }
        let Some(outer) = outer_weak.upgrade() else {
            return;
        };
        let is_selected = outer.has_css_class("selected")
            || outer.state_flags().contains(gtk4::StateFlags::SELECTED);
        if !is_selected {
            return;
        }

        let label_bounds = label_weak
            .upgrade()
            .filter(|l| l.is_visible())
            .and_then(|l| l.compute_bounds(area).map(|b| (l, b)));

        // 1. Caja squircle cuadrada alrededor del icono
        if let Some(slot) = slot_weak.upgrade() {
            if let Some(ib) = slot.compute_bounds(area) {
                let ix = ib.x() as f64;
                let iy = ib.y() as f64;
                let iw = ib.width() as f64;
                let ih = ib.height() as f64;
                if iw > 2.0 && ih > 2.0 {
                    let pad = (iw.max(ih) * 0.09).clamp(4.5, 6.5);
                    let side = (iw.max(ih) + pad * 2.0).min(area_w - 2.0);
                    let cx = ix + iw * 0.5;
                    let cy = iy + ih * 0.5;
                    let bx = (cx - side * 0.5).clamp(1.0, (area_w - side - 1.0).max(1.0));
                    let by = (cy - side * 0.5).max(1.0);
                    let max_bottom = label_bounds
                        .as_ref()
                        .map(|(_, lb)| (lb.y() as f64 - 4.5).max(iy + ih + 2.0))
                        .unwrap_or(area_h - 1.0);
                    let bh = side.min(max_bottom - by).max(ih + 4.0);
                    let radius = (side.min(bh) * 0.17).clamp(8.0, 14.0);

                    let _ = cr.save();
                    match theme {
                        IconSelectionTheme::WindowCanvas => {
                            let fg = outer.color();
                            squircle_path_4corners(
                                cr, bx, by, side, bh, radius, true, true, true, true,
                            );
                            cr.set_source_rgba(
                                fg.red() as f64,
                                fg.green() as f64,
                                fg.blue() as f64,
                                0.07,
                            );
                            let _ = cr.fill();
                        }
                        IconSelectionTheme::DesktopWallpaper => {
                            squircle_path_4corners(
                                cr, bx, by, side, bh, radius, true, true, true, true,
                            );
                            cr.set_source_rgba(0.0, 0.0, 0.0, 0.30);
                            let _ = cr.fill();

                            squircle_path_4corners(
                                cr,
                                bx + 0.75,
                                by + 0.75,
                                (side - 1.5).max(2.0),
                                (bh - 1.5).max(2.0),
                                radius,
                                true,
                                true,
                                true,
                                true,
                            );
                            cr.set_source_rgba(1.0, 1.0, 1.0, 0.36);
                            cr.set_line_width(1.5);
                            let _ = cr.stroke();
                        }
                    }
                    let _ = cr.restore();
                }
            }
        }

        // 2. Píldora azul squircle ajustada línea por línea (máximo 2 líneas) con coordenadas exactas de PangoLayoutIter
        if let Some((lbl, lb)) = label_bounds {
            let lx = lb.x() as f64;
            let ly = lb.y() as f64;
            let layout = lbl.layout();
            let (off_x, off_y) = lbl.layout_offsets();
            let scale = gtk4::pango::SCALE as f64;
            let pad_x = 5.5;
            let pad_y = 1.5;

            let mut line_rects = Vec::with_capacity(MAX_LABEL_LINES);
            let mut iter = layout.iter();
            loop {
                let (ink, logical) = iter.line_extents();
                let log_w = logical.width() as f64 / scale;
                let log_h = logical.height() as f64 / scale;
                if log_w > 0.5 && log_h > 0.5 {
                    let ink_w = ink.width() as f64 / scale;
                    // Usar `ink` horizontal (excluye espacios finales de salto de línea) y `logical` vertical (altura uniforme)
                    let (hx, hw) = if ink_w > 0.5 {
                        (ink.x() as f64 / scale, ink_w)
                    } else {
                        (logical.x() as f64 / scale, log_w)
                    };
                    let ly_line = logical.y() as f64 / scale;
                    let x0 = (lx + off_x as f64 + hx - pad_x).max(0.5);
                    let x1 = (lx + off_x as f64 + hx + hw + pad_x).min(area_w - 0.5);
                    let y0 = (ly + off_y as f64 + ly_line - pad_y).max(0.0);
                    let y1 = (ly + off_y as f64 + ly_line + log_h + pad_y).min(area_h);
                    if x1 > x0 + 2.0 && y1 > y0 + 2.0 {
                        line_rects.push(LinePillRect { x0, y0, x1, y1 });
                    }
                }
                if line_rects.len() >= MAX_LABEL_LINES || !iter.next_line() {
                    break;
                }
            }

            if !line_rects.is_empty() {
                let accent = crate::config::load_accent_color();
                let _ = cr.save();
                draw_multiline_squircle_highlight(cr, &line_rects, accent);
                let _ = cr.restore();
            }
        }
    });

    // Redibujar instantáneamente cuando cambie selección, texto o visibilidad (ej. renombrado inline)
    {
        let bg_c = bg.clone();
        outer_box.connect_notify_local(Some("css-classes"), move |_, _| {
            bg_c.queue_draw();
        });
    }
    {
        let bg_c = bg.clone();
        outer_box.connect_state_flags_changed(move |_, _| {
            bg_c.queue_draw();
        });
    }
    {
        let bg_c = bg.clone();
        label.connect_notify_local(Some("label"), move |_, _| {
            bg_c.queue_draw();
        });
    }
    {
        let bg_c = bg.clone();
        label.connect_notify_local(Some("visible"), move |_, _| {
            bg_c.queue_draw();
        });
    }

    let overlay = Overlay::builder()
        .hexpand(true)
        .vexpand(true)
        .halign(Align::Fill)
        .valign(Align::Fill)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner_vbox);
    overlay.set_measure_overlay(&inner_vbox, true);
    outer_box.append(&overlay);
}

/// Actualiza el tamaño del contenedor cuadrado del icono cuando el usuario cambia el tamaño
/// de iconos en caliente (por ejemplo mediante el deslizador de vista de iconos de Branches).
pub fn update_icon_slot_size<W: IsA<gtk4::Widget>>(outer: &W, icon_size: i32) {
    fn visit(widget: &gtk4::Widget, size: i32) {
        if widget.has_css_class("malus-icon-slot") {
            widget.set_size_request(size, size);
            return;
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            visit(&c, size);
            child = c.next_sibling();
        }
    }
    visit(outer.as_ref(), icon_size);
    outer.as_ref().queue_draw();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_line_pill_rects_closes_vertical_gap_and_snaps_similar_widths() {
        let mut rects = vec![
            LinePillRect {
                x0: 20.0,
                y0: 80.0,
                x1: 80.0,
                y1: 96.0,
            },
            LinePillRect {
                x0: 8.0,
                y0: 98.0,
                x1: 92.0,
                y1: 114.0,
            },
        ];
        normalize_line_pill_rects(&mut rects);
        // Sin hueco vertical entre la línea 1 y la línea 2
        assert!((rects[0].y1 - rects[1].y0).abs() < 1e-6);
        assert!((rects[0].y1 - 97.0).abs() < 1e-6);
        // La línea 1 conserva su ancho más estrecho al diferir más de 3.5 px
        assert!((rects[0].x0 - 20.0).abs() < 1e-6);
        assert!((rects[1].x0 - 8.0).abs() < 1e-6);

        let mut similar = vec![
            LinePillRect {
                x0: 10.0,
                y0: 80.0,
                x1: 90.0,
                y1: 96.0,
            },
            LinePillRect {
                x0: 12.0,
                y0: 96.0,
                x1: 88.0,
                y1: 112.0,
            },
        ];
        normalize_line_pill_rects(&mut similar);
        assert!((similar[0].x0 - similar[1].x0).abs() < 1e-6);
        assert!((similar[0].x1 - similar[1].x1).abs() < 1e-6);
    }
}
