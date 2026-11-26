use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Align, Button, DragSource, DrawingArea, Image, Overlay, PropagationPhase};
use malus_de_common::config::{
    is_dark_mode_active, liquid_glass_scale_for, load_liquid_glass_opacity,
    scaled_liquid_glass_alpha, ControlCenterConfig, ControlWidgetKind, MenuBarConfig,
};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use std::rc::Rc;

pub const CC_DRAG_PREFIX: &str = "malus-cc:";
pub const TRAY_DRAG_PREFIX: &str = "malus-tray:";

const SQUIRCLE_EXPONENT: f64 = 4.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPayload {
    FromControlCenter(ControlWidgetKind),
    FromTray(ControlWidgetKind),
}

impl DragPayload {
    pub fn encode(self) -> String {
        match self {
            Self::FromControlCenter(kind) => format!("{CC_DRAG_PREFIX}{}", kind.id()),
            Self::FromTray(kind) => format!("{TRAY_DRAG_PREFIX}{}", kind.id()),
        }
    }

    pub fn decode(raw: &str) -> Option<Self> {
        if let Some(id) = raw.strip_prefix(CC_DRAG_PREFIX) {
            return ControlWidgetKind::from_id(id.trim()).map(Self::FromControlCenter);
        }
        if let Some(id) = raw.strip_prefix(TRAY_DRAG_PREFIX) {
            return ControlWidgetKind::from_id(id.trim()).map(Self::FromTray);
        }
        None
    }

    pub fn kind(self) -> ControlWidgetKind {
        match self {
            Self::FromControlCenter(k) | Self::FromTray(k) => k,
        }
    }
}

/// Traza un rectángulo squircle continuo (`n = 4.2`) en Cairo, alineado con el blur GPU del compositor.
pub fn squircle_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.5 {
        cr.rectangle(x, y, w, h);
        return;
    }

    let steps = 16usize;
    let inv_n = 2.0 / SQUIRCLE_EXPONENT;
    let corner_pt = |cx: f64, cy: f64, sx: f64, sy: f64, t: f64| -> (f64, f64) {
        let theta = t * std::f64::consts::FRAC_PI_2;
        let dx = theta.cos().abs().powf(inv_n) * r;
        let dy = theta.sin().abs().powf(inv_n) * r;
        (cx + sx * dx, cy + sy * dy)
    };

    cr.new_sub_path();
    // Esquina superior derecha (de arriba hacia la derecha)
    let (cx_tr, cy_tr) = (x + w - r, y + r);
    for i in 0..=steps {
        let t = 1.0 - (i as f64 / steps as f64);
        let (px, py) = corner_pt(cx_tr, cy_tr, 1.0, -1.0, t);
        if i == 0 {
            cr.move_to(px, py);
        } else {
            cr.line_to(px, py);
        }
    }
    // Esquina inferior derecha
    let (cx_br, cy_br) = (x + w - r, y + h - r);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let (px, py) = corner_pt(cx_br, cy_br, 1.0, 1.0, t);
        cr.line_to(px, py);
    }
    // Esquina inferior izquierda
    let (cx_bl, cy_bl) = (x + r, y + h - r);
    for i in 0..=steps {
        let t = 1.0 - (i as f64 / steps as f64);
        let (px, py) = corner_pt(cx_bl, cy_bl, -1.0, 1.0, t);
        cr.line_to(px, py);
    }
    // Esquina superior izquierda
    let (cx_tl, cy_tl) = (x + r, y + r);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let (px, py) = corner_pt(cx_tl, cy_tl, -1.0, -1.0, t);
        cr.line_to(px, py);
    }
    cr.close_path();
}

/// Dibuja el mismo fondo semitransparente con borde sutil usado en los menús/popovers de Malus DE
/// pero con geometría squircle (`n = 4.2`).
pub fn draw_squircle_glass_surface(
    cr: &gtk4::cairo::Context,
    w: i32,
    h: i32,
    radius: f64,
    hovered: bool,
    pressed: bool,
) {
    let wf = w as f64;
    let hf = h as f64;
    if wf <= 2.0 || hf <= 2.0 {
        return;
    }

    let dark = is_dark_mode_active();
    let scale = liquid_glass_scale_for(load_liquid_glass_opacity());
    let base_a = scaled_liquid_glass_alpha(0.72, scale, 0.14, 0.96);
    let fill_a = if pressed {
        (base_a + 0.10).min(0.98)
    } else if hovered {
        (base_a + 0.05).min(0.96)
    } else {
        base_a
    };

    let _ = cr.save();
    squircle_path(cr, 0.5, 0.5, wf - 1.0, hf - 1.0, radius);
    if dark {
        cr.set_source_rgba(30.0 / 255.0, 30.0 / 255.0, 36.0 / 255.0, fill_a);
    } else {
        cr.set_source_rgba(244.0 / 255.0, 246.0 / 255.0, 250.0 / 255.0, fill_a);
    }
    let _ = cr.fill_preserve();

    // Borde sutil acorde al tema (igual que los popovers translúcidos)
    if dark {
        let ba = if hovered { 0.22 } else { 0.15 };
        cr.set_source_rgba(1.0, 1.0, 1.0, ba);
    } else {
        let ba = if hovered { 0.14 } else { 0.09 };
        cr.set_source_rgba(0.0, 0.0, 0.0, ba);
    }
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let _ = cr.restore();
}

/// Envuelve un contenedor del Centro de Control (elementos de >2 columnas, tarjeta multimedia o subvistas)
/// con fondo semitransparente squircle (`n = 4.2`) alineado pixel a pixel con el blur GPU.
pub fn wrap_squircle_glass_container<W: IsA<gtk4::Widget>>(inner: &W, radius: f64) -> Overlay {
    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);
    bg.set_draw_func(move |_, cr, w, h| {
        draw_squircle_glass_surface(cr, w, h, radius, false, false);
    });

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    let overlay = Overlay::builder()
        .halign(Align::Fill)
        .valign(Align::Fill)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(inner);
    overlay.set_measure_overlay(inner, true);
    overlay
}

/// Aplica fondo semitransparente squircle (`n = 4.2`) a un botón compacto ajustado al tamaño de su texto.
pub fn apply_squircle_glass_button(btn: &Button, radius: f64, pad_v: i32, pad_h: i32) {
    apply_squircle_glass_button_inner(btn, radius, pad_v, pad_h, false);
}

/// Aplica fondo semitransparente squircle (`n = 4.2`) a un botón de tarjeta/tile del Centro de Control
/// que llena completamente su celda de la cuadrícula (`1×1` o `2×1`).
pub fn apply_squircle_glass_fill_button(btn: &Button, radius: f64, pad_v: i32, pad_h: i32) {
    apply_squircle_glass_button_inner(btn, radius, pad_v, pad_h, true);
}

fn apply_squircle_glass_button_inner(
    btn: &Button,
    radius: f64,
    pad_v: i32,
    pad_h: i32,
    fill_cell: bool,
) {
    let Some(inner) = btn.child() else {
        return;
    };
    btn.set_child(gtk4::Widget::NONE);
    inner.set_margin_top(pad_v);
    inner.set_margin_bottom(pad_v);
    inner.set_margin_start(pad_h);
    inner.set_margin_end(pad_h);
    if !fill_cell {
        inner.set_halign(Align::Center);
        inner.set_valign(Align::Center);
    }

    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let (hovered, pressed, drop_hover, active_border) = btn_weak
            .upgrade()
            .map(|b| {
                let f = b.state_flags();
                (
                    f.contains(gtk4::StateFlags::PRELIGHT),
                    f.contains(gtk4::StateFlags::ACTIVE),
                    b.has_css_class("cc-drop-hover"),
                    b.has_css_class("cc-glass-circle-active"),
                )
            })
            .unwrap_or((false, false, false, false));
        draw_squircle_glass_surface(cr, w, h, radius, hovered, pressed);
        if drop_hover || active_border {
            let wf = w as f64;
            let hf = h as f64;
            if wf > 2.0 && hf > 2.0 {
                let (r, g, b_col) = malus_de_common::config::load_accent_color();
                let _ = cr.save();
                squircle_path(cr, 0.75, 0.75, wf - 1.5, hf - 1.5, radius);
                if drop_hover {
                    cr.set_source_rgba(r, g, b_col, 0.22);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(r, g, b_col, 0.85);
                    cr.set_line_width(1.5);
                } else {
                    cr.set_source_rgba(r, g, b_col, 0.55);
                    cr.set_line_width(1.2);
                }
                let _ = cr.stroke();
                let _ = cr.restore();
            }
        }
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    let align = if fill_cell { Align::Fill } else { Align::Center };
    let overlay = Overlay::builder()
        .halign(align)
        .valign(align)
        .hexpand(fill_cell)
        .vexpand(fill_cell)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner);
    overlay.set_measure_overlay(&inner, true);
    btn.set_child(Some(&overlay));
}

/// Adjunta un `DragSource` GTK4 a cualquier widget del Centro de Control, galería
/// «Editar controles» o bandeja del sistema para arrastrar controles de forma bidireccional.
pub fn attach_drag_source<W: IsA<gtk4::Widget>>(
    widget: &W,
    payload: DragPayload,
    on_begin: Option<Rc<dyn Fn()>>,
    on_end: Option<Rc<dyn Fn()>>,
) {
    let source = DragSource::new();
    source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    source.set_propagation_phase(PropagationPhase::Bubble);

    let encoded = payload.encode();
    source.connect_prepare(move |_, _, _| {
        Some(gdk::ContentProvider::for_value(&encoded.to_value()))
    });

    let icon_name = payload.kind().icon_name();
    let w_ref = widget.clone().upcast::<gtk4::Widget>();
    source.connect_drag_begin(move |src, _| {
        let paintable = gtk4::WidgetPaintable::new(Some(&w_ref));
        let w = w_ref.width().max(1);
        let h = w_ref.height().max(1);
        if w >= 16 && h >= 16 {
            src.set_icon(Some(&paintable), w / 2, h / 2);
        } else {
            let img = Image::from_icon_name(icon_name);
            img.set_pixel_size(24);
            let p = gtk4::WidgetPaintable::new(Some(&img));
            src.set_icon(Some(&p), 12, 12);
        }
        if let Some(ref cb) = on_begin {
            cb();
        }
    });

    if let Some(cb) = on_end {
        source.connect_drag_end(move |_, _, _| {
            cb();
        });
    }

    widget.add_controller(source);
}

/// Mueve un control desde el Centro de Control hacia la bandeja del sistema (evitando duplicados).
pub fn pin_widget_to_tray(kind: ControlWidgetKind) {
    let mut mb_cfg = MenuBarConfig::load();
    if !mb_cfg.is_widget_in_tray(kind) {
        mb_cfg.set_widget_in_tray(kind, true);
        let _ = mb_cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadMenuBarConfig);
    }

    let mut cc_cfg = ControlCenterConfig::load();
    if cc_cfg.remove_widget(kind) {
        let _ = cc_cfg.save();
    }

    crate::ui::status_tray::refresh_tray_visibility();
    crate::ui::control_center::refresh_control_center_layout();
}

/// Retira un control de la bandeja del sistema de la barra superior.
pub fn unpin_widget_from_tray(kind: ControlWidgetKind) {
    let mut cfg = MenuBarConfig::load();
    if cfg.is_widget_in_tray(kind) {
        cfg.set_widget_in_tray(kind, false);
        let _ = cfg.save();
        let _ = IpcClient::send_command(IpcMessage::ReloadMenuBarConfig);
    }
    crate::ui::status_tray::refresh_tray_visibility();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_payload_roundtrip_cc_and_tray() {
        for &kind in ControlWidgetKind::ALL {
            let from_cc = DragPayload::FromControlCenter(kind);
            assert_eq!(DragPayload::decode(&from_cc.encode()), Some(from_cc));

            let from_tray = DragPayload::FromTray(kind);
            assert_eq!(DragPayload::decode(&from_tray.encode()), Some(from_tray));
        }
        assert_eq!(DragPayload::decode("unrelated:foo"), None);
        assert_eq!(DragPayload::decode("malus-cc:unknown"), None);
    }
}
