use crate::fs_backend::{open_file, FileItem};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{Key, MemoryFormat, MemoryTexture, ModifierType, Texture};
use gtk4::gio;
use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, Box as GtkBox, Button, ContentFit, EventControllerKey, GestureClick, Grid,
    Label, Orientation, Overflow, Overlay, Picture, PolicyType, PropagationPhase, ScrolledWindow,
    Separator, Spinner, TextView, Window, WindowHandle,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type PageStepCallback = Rc<dyn Fn(i32) -> bool>;

#[derive(Clone)]
pub struct MediaPlaybackHandle {
    stop_flag: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<Child>>>,
}

impl MediaPlaybackHandle {
    fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn register_child(&self, mut child: Child) {
        if self.stop_flag.load(Ordering::SeqCst) {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return;
        }
        if let Ok(mut guard) = self.children.lock() {
            guard.push(child);
        }
    }

    fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let mut drained = Vec::new();
        if let Ok(mut guard) = self.children.lock() {
            for mut child in guard.drain(..) {
                let _ = child.kill();
                drained.push(child);
            }
        }
        if !drained.is_empty() {
            std::thread::spawn(move || {
                for mut child in drained {
                    let _ = child.wait();
                }
            });
        }
    }
}

impl Drop for MediaPlaybackHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

const QL_HEADER_HEIGHT: i32 = 42;
const QL_IMAGE_INSET: i32 = 8;
const QL_VIDEO_CHROME_W: i32 = 28;
const QL_VIDEO_CHROME_H: i32 = 116;
const QL_PDF_CHROME_W: i32 = 28;
const QL_PDF_CHROME_H_BASE: i32 = 66;
const QL_PDF_FOOTER_H: i32 = 46;
const QL_MIN_CARD_W: i32 = 360;
const QL_MIN_CARD_H: i32 = 260;
const QL_MAX_DECODE_W: i32 = 1280;
const QL_MAX_DECODE_H: i32 = 960;
const QL_SQUIRCLE_EXPONENT: f64 = 4.2;
const QL_SQUIRCLE_SEGMENTS: usize = 24;
const QL_OPEN_ANIM_US: f64 = 220_000.0;
const QL_CLOSE_ANIM_US: f64 = 165_000.0;

pub fn quicklook_open_anim_state(t: f64, standalone: bool) -> (f32, f64, f32) {
    let t = t.clamp(0.0, 1.0);
    let ease_s = 1.0 - (1.0 - t).powi(4);
    let a_t = (t / 0.70).min(1.0);
    let opacity = 1.0 - (1.0 - a_t).powi(2);
    let start_scale = if standalone { 0.94 } else { 0.86 };
    let start_y = if standalone { 8.0 } else { 10.0 };
    let scale = (start_scale + (1.0 - start_scale) * ease_s) as f32;
    let offset_y = (start_y * (1.0 - ease_s)) as f32;
    (scale, opacity, offset_y)
}

pub fn quicklook_close_anim_state(
    t: f64,
    standalone: bool,
    start_scale: f32,
    start_opacity: f64,
    start_offset_y: f32,
) -> (f32, f64, f32) {
    let t = t.clamp(0.0, 1.0);
    let ease = t * t;
    let target_scale = if standalone { 0.94_f32 } else { 0.88_f32 };
    let target_y = if standalone { 6.0_f32 } else { 8.0_f32 };
    let scale = start_scale + (target_scale - start_scale) * ease as f32;
    let opacity = (start_opacity * (1.0 - ease)).clamp(0.0, 1.0);
    let offset_y = start_offset_y + (target_y - start_offset_y) * ease as f32;
    (scale, opacity, offset_y)
}

pub fn quicklook_card_squircle_radius() -> f64 {
    malus_de_common::config::DisplaysConfig::load()
        .window_corner_radius
        .clamp(0.0, 48.0)
}

pub fn quicklook_concentric_inner_radius(outer_radius: f64, inset: f64) -> f64 {
    if outer_radius <= 0.5 {
        0.0
    } else {
        (outer_radius - inset).max(8.0)
    }
}

fn draw_squircle_path(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let radius = radius.min(width * 0.5).min(height * 0.5);
    if radius <= 0.5 {
        cr.rectangle(x, y, width, height);
        return;
    }
    let power = 2.0 / QL_SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;

    cr.move_to(x + radius, y);
    cr.line_to(x + width - radius, y);
    for step in 1..=QL_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / QL_SQUIRCLE_SEGMENTS as f64;
        cr.line_to(
            x + width - radius + radius * angle.sin().powf(power),
            y + radius - radius * angle.cos().powf(power),
        );
    }
    cr.line_to(x + width, y + height - radius);
    for step in 1..=QL_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / QL_SQUIRCLE_SEGMENTS as f64;
        cr.line_to(
            x + width - radius + radius * angle.cos().powf(power),
            y + height - radius + radius * angle.sin().powf(power),
        );
    }
    cr.line_to(x + radius, y + height);
    for step in 1..=QL_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / QL_SQUIRCLE_SEGMENTS as f64;
        cr.line_to(
            x + radius - radius * angle.sin().powf(power),
            y + height - radius + radius * angle.cos().powf(power),
        );
    }
    cr.line_to(x, y + radius);
    for step in 1..=QL_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / QL_SQUIRCLE_SEGMENTS as f64;
        cr.line_to(
            x + radius - radius * angle.cos().powf(power),
            y + radius - radius * angle.sin().powf(power),
        );
    }
    cr.close_path();
}

#[inline]
fn squircle_sdf_f32(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let exp = QL_SQUIRCLE_EXPONENT as f32;
    let dx = px.abs() - half_w + radius;
    let dy = py.abs() - half_h + radius;
    if dx <= 0.0 || dy <= 0.0 {
        dx.max(dy) - radius
    } else {
        (dx.powf(exp) + dy.powf(exp)).powf(1.0 / exp) - radius
    }
}

fn apply_squircle_clip_rgba_corners(rgba: &mut [u8], width: usize, height: usize, radius: f32) {
    if width == 0 || height == 0 || radius <= 1.0 {
        return;
    }
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let r = radius.min(half_w).min(half_h);
    if r <= 1.0 {
        return;
    }
    let corner_span = (r.ceil() as usize + 2).min(width).min(height);
    let aa = 1.0_f32;

    let mut mask_pixel = |x: usize, y: usize| {
        let idx = (y * width + x) * 4;
        if idx + 3 >= rgba.len() {
            return;
        }
        let lx = (x as f32 + 0.5) - half_w;
        let ly = (y as f32 + 0.5) - half_h;
        let dist = squircle_sdf_f32(lx, ly, half_w, half_h, r);
        if dist >= 0.0 {
            rgba[idx + 3] = 0;
        } else if dist > -aa {
            let t = ((-dist) / aa).clamp(0.0, 1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            rgba[idx + 3] = ((rgba[idx + 3] as f32) * smooth).round() as u8;
        }
    };

    let x_right_start = width.saturating_sub(corner_span);
    let y_bottom_start = height.saturating_sub(corner_span);

    for y in 0..corner_span {
        for x in 0..corner_span {
            mask_pixel(x, y);
        }
        for x in x_right_start.max(corner_span)..width {
            mask_pixel(x, y);
        }
    }
    for y in y_bottom_start.max(corner_span)..height {
        for x in 0..corner_span {
            mask_pixel(x, y);
        }
        for x in x_right_start.max(corner_span)..width {
            mask_pixel(x, y);
        }
    }
}

fn precompute_squircle_corner_alpha(
    width: usize,
    height: usize,
    radius: f32,
) -> Vec<(usize, u8)> {
    let mut edits = Vec::new();
    if width == 0 || height == 0 || radius <= 1.0 {
        return edits;
    }
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let r = radius.min(half_w).min(half_h);
    if r <= 1.0 {
        return edits;
    }
    let corner_span = (r.ceil() as usize + 2).min(width).min(height);
    let aa = 1.0_f32;
    let mut collect = |x: usize, y: usize| {
        let lx = (x as f32 + 0.5) - half_w;
        let ly = (y as f32 + 0.5) - half_h;
        let dist = squircle_sdf_f32(lx, ly, half_w, half_h, r);
        if dist >= 0.0 {
            edits.push(((y * width + x) * 4 + 3, 0));
        } else if dist > -aa {
            let t = ((-dist) / aa).clamp(0.0, 1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            edits.push(((y * width + x) * 4 + 3, (smooth * 255.0).round() as u8));
        }
    };
    let x_right_start = width.saturating_sub(corner_span);
    let y_bottom_start = height.saturating_sub(corner_span);
    for y in 0..corner_span {
        for x in 0..corner_span {
            collect(x, y);
        }
        for x in x_right_start.max(corner_span)..width {
            collect(x, y);
        }
    }
    for y in y_bottom_start.max(corner_span)..height {
        for x in 0..corner_span {
            collect(x, y);
        }
        for x in x_right_start.max(corner_span)..width {
            collect(x, y);
        }
    }
    edits
}

/// Tarjeta Quick Look acelerada por GPU:
/// - En overlay dentro de Branches: dibuja sombra proyectada squircle (`n = 4.2`), recorte alfa
///   squircle (`gsk::MaskNode`), desenfoque gaussiano real del fondo (`push_blur`), tinte Liquid
///   Glass y animación fluida de apertura/cierre sincronizada al frame clock.
/// - En ventana independiente de escritorio: deja el squircle/blur/sombra exteriores al compositor
///   y anima suavemente la opacidad y escala interna al abrir y cerrar.
mod squircle_card {
    use super::{
        draw_squircle_path, quicklook_card_squircle_radius, quicklook_close_anim_state,
        quicklook_open_anim_state, QL_CLOSE_ANIM_US, QL_OPEN_ANIM_US,
    };
    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    mod imp {
        use super::*;

        pub struct QuickLookSquircleCard {
            pub standalone: Cell<bool>,
            pub anim_scale: Cell<f32>,
            pub anim_opacity: Cell<f64>,
            pub anim_offset_y: Cell<f32>,
            pub anim_gen: Cell<u64>,
            pub bg_widget: RefCell<Option<glib::WeakRef<gtk4::Widget>>>,
            pub bg_paintable: RefCell<Option<gtk4::WidgetPaintable>>,
        }

        impl Default for QuickLookSquircleCard {
            fn default() -> Self {
                Self {
                    standalone: Cell::new(false),
                    anim_scale: Cell::new(1.0),
                    anim_opacity: Cell::new(1.0),
                    anim_offset_y: Cell::new(0.0),
                    anim_gen: Cell::new(0),
                    bg_widget: RefCell::new(None),
                    bg_paintable: RefCell::new(None),
                }
            }
        }

        #[glib::object_subclass]
        impl ObjectSubclass for QuickLookSquircleCard {
            const NAME: &'static str = "MalusQuickLookSquircleCard";
            type Type = super::QuickLookSquircleCard;
            type ParentType = gtk4::Widget;

            fn class_init(klass: &mut Self::Class) {
                klass.set_layout_manager_type::<gtk4::BinLayout>();
            }
        }

        impl ObjectImpl for QuickLookSquircleCard {
            fn dispose(&self) {
                if let Some(paintable) = self.bg_paintable.borrow_mut().take() {
                    paintable.set_widget(gtk4::Widget::NONE);
                }
                *self.bg_widget.borrow_mut() = None;
                while let Some(child) = self.obj().first_child() {
                    child.unparent();
                }
            }
        }

        impl WidgetImpl for QuickLookSquircleCard {
            fn snapshot(&self, snapshot: &gtk4::Snapshot) {
                let obj = self.obj();
                let w = obj.width() as f32;
                let h = obj.height() as f32;
                if w <= 2.0 || h <= 2.0 {
                    return;
                }

                let alpha = self.anim_opacity.get().clamp(0.0, 1.0);
                if alpha <= 0.001 {
                    return;
                }
                let scale = self.anim_scale.get().max(0.05);
                let offset_y = self.anim_offset_y.get();
                let has_transform = (scale - 1.0).abs() > 0.0005 || offset_y.abs() > 0.05;

                // Modo ventana independiente en escritorio: el compositor dibuja el squircle,
                // blur y sombra exteriores; aquí animamos suavemente opacidad y contenido interior.
                if self.standalone.get() {
                    if alpha < 0.999 {
                        snapshot.push_opacity(alpha);
                    }
                    if has_transform {
                        snapshot.save();
                        snapshot.translate(&gtk4::graphene::Point::new(
                            w * 0.5,
                            h * 0.5 + offset_y,
                        ));
                        snapshot.scale(scale, scale);
                        snapshot.translate(&gtk4::graphene::Point::new(-w * 0.5, -h * 0.5));
                    }
                    if let Some(child) = obj.first_child() {
                        obj.snapshot_child(&child, snapshot);
                    }
                    if has_transform {
                        snapshot.restore();
                    }
                    if alpha < 0.999 {
                        snapshot.pop();
                    }
                    return;
                }

                let radius = quicklook_card_squircle_radius();
                let is_dark = libadwaita::StyleManager::default().is_dark();
                let reduce_transparency = malus_de_common::config::load_reduce_transparency();
                let opacity = malus_de_common::config::load_liquid_glass_opacity();
                let glass_scale = malus_de_common::config::liquid_glass_scale_for(opacity);
                let bounds = gtk4::graphene::Rect::new(0.0, 0.0, w, h);

                if alpha < 0.999 {
                    snapshot.push_opacity(alpha);
                }
                if has_transform {
                    snapshot.save();
                    snapshot.translate(&gtk4::graphene::Point::new(
                        w * 0.5,
                        h * 0.5 + offset_y,
                    ));
                    snapshot.scale(scale, scale);
                    snapshot.translate(&gtk4::graphene::Point::new(-w * 0.5, -h * 0.5));
                }

                // 1. Sombra proyectada squircle en dos capas (elevación macOS + contacto)
                let shadow_pad = 56.0_f32;
                let shadow_bounds = gtk4::graphene::Rect::new(
                    -shadow_pad,
                    -shadow_pad,
                    w + shadow_pad * 2.0,
                    h + shadow_pad * 2.0,
                );
                snapshot.push_blur(32.0);
                {
                    let cr = snapshot.append_cairo(&shadow_bounds);
                    draw_squircle_path(&cr, 0.0, 14.0, w as f64, h as f64, radius);
                    cr.set_source_rgba(0.0, 0.0, 0.0, if is_dark { 0.44 } else { 0.24 });
                    let _ = cr.fill();
                }
                snapshot.pop();

                snapshot.push_blur(10.0);
                {
                    let cr = snapshot.append_cairo(&shadow_bounds);
                    draw_squircle_path(&cr, 0.0, 3.0, w as f64, h as f64, radius);
                    cr.set_source_rgba(0.0, 0.0, 0.0, if is_dark { 0.24 } else { 0.12 });
                    let _ = cr.fill();
                }
                snapshot.pop();

                // 2. Contenido (blur GPU + cristal + hijos + bisel) enmascarado con squircle continuo (n = 4.2)
                let source_snap = gtk4::Snapshot::new();

                // 2a. Desenfoque gaussiano real en GPU del contenido de Branches detrás de la tarjeta
                if !reduce_transparency {
                    let bg_opt = self.bg_widget.borrow().as_ref().and_then(|r| r.upgrade());
                    let paintable_opt = self.bg_paintable.borrow().clone();
                    if let (Some(bg_widget), Some(paintable)) = (bg_opt, paintable_opt) {
                        let bg_w = bg_widget.width() as f64;
                        let bg_h = bg_widget.height() as f64;
                        if bg_w > 2.0 && bg_h > 2.0 {
                            if let Some(origin) = obj
                                .compute_point(&bg_widget, &gtk4::graphene::Point::new(0.0, 0.0))
                            {
                                let blur_pad = 36.0_f32;
                                let expanded = gtk4::graphene::Rect::new(
                                    -blur_pad,
                                    -blur_pad,
                                    w + blur_pad * 2.0,
                                    h + blur_pad * 2.0,
                                );
                                let fallback_bg = if is_dark {
                                    gtk4::gdk::RGBA::new(30.0 / 255.0, 30.0 / 255.0, 34.0 / 255.0, 1.0)
                                } else {
                                    gtk4::gdk::RGBA::new(242.0 / 255.0, 242.0 / 255.0, 246.0 / 255.0, 1.0)
                                };
                                source_snap.push_clip(&bounds);
                                source_snap.push_blur(28.0);
                                source_snap.push_clip(&expanded);
                                source_snap.append_color(&fallback_bg, &expanded);
                                source_snap.save();
                                source_snap.translate(&gtk4::graphene::Point::new(
                                    -origin.x(),
                                    -origin.y(),
                                ));
                                paintable.snapshot(&source_snap, bg_w, bg_h);
                                source_snap.restore();
                                source_snap.pop();
                                source_snap.pop();
                                source_snap.pop();
                            }
                        }
                    }
                }

                // 2b. Tinte Liquid Glass semitransparente + reflejo superior
                let fill_alpha = if reduce_transparency {
                    0.97
                } else if is_dark {
                    malus_de_common::config::scaled_liquid_glass_alpha(0.72, glass_scale, 0.16, 0.94)
                } else {
                    malus_de_common::config::scaled_liquid_glass_alpha(0.68, glass_scale, 0.16, 0.94)
                };
                {
                    let cr = source_snap.append_cairo(&bounds);
                    if is_dark {
                        cr.set_source_rgba(36.0 / 255.0, 36.0 / 255.0, 40.0 / 255.0, fill_alpha);
                    } else {
                        cr.set_source_rgba(246.0 / 255.0, 246.0 / 255.0, 248.0 / 255.0, fill_alpha);
                    }
                    cr.rectangle(0.0, 0.0, w as f64, h as f64);
                    let _ = cr.fill();

                    let sheen = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, h as f64);
                    if is_dark {
                        sheen.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.08);
                        sheen.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.01);
                    } else {
                        sheen.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.28);
                        sheen.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.04);
                    }
                    let _ = cr.set_source(&sheen);
                    cr.rectangle(0.0, 0.0, w as f64, h as f64);
                    let _ = cr.fill();
                }

                // 2c. Contenido hijo (cabecera + vista previa) recortado al squircle
                if let Some(child) = obj.first_child() {
                    obj.snapshot_child(&child, &source_snap);
                }

                // 2d. Bisel interior squircle de 1 px estilo macOS
                {
                    let cr = source_snap.append_cairo(&bounds);
                    draw_squircle_path(
                        &cr,
                        0.5,
                        0.5,
                        (w as f64 - 1.0).max(1.0),
                        (h as f64 - 1.0).max(1.0),
                        (radius - 0.5).max(0.0),
                    );
                    if is_dark {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
                    } else {
                        cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
                    }
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }

                let mask_snap = gtk4::Snapshot::new();
                {
                    let cr = mask_snap.append_cairo(&bounds);
                    draw_squircle_path(&cr, 0.0, 0.0, w as f64, h as f64, radius);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    let _ = cr.fill();
                }

                if let (Some(source_node), Some(mask_node)) =
                    (source_snap.to_node(), mask_snap.to_node())
                {
                    let masked = gtk4::gsk::MaskNode::new(
                        source_node,
                        mask_node,
                        gtk4::gsk::MaskMode::Alpha,
                    );
                    snapshot.append_node(&masked);
                }

                if has_transform {
                    snapshot.restore();
                }
                if alpha < 0.999 {
                    snapshot.pop();
                }
            }
        }
    }

    glib::wrapper! {
        pub struct QuickLookSquircleCard(ObjectSubclass<imp::QuickLookSquircleCard>)
            @extends gtk4::Widget;
    }

    impl QuickLookSquircleCard {
        pub fn new(
            child: &impl IsA<gtk4::Widget>,
            backdrop_source: Option<&gtk4::Widget>,
            standalone: bool,
        ) -> Self {
            let obj: Self = glib::Object::new();
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            obj.set_overflow(if standalone {
                gtk4::Overflow::Hidden
            } else {
                gtk4::Overflow::Visible
            });
            obj.imp().standalone.set(standalone);
            let (s0, a0, y0) = quicklook_open_anim_state(0.0, standalone);
            obj.imp().anim_scale.set(s0);
            obj.imp().anim_opacity.set(a0);
            obj.imp().anim_offset_y.set(y0);
            child.set_parent(&obj);

            if !standalone {
                if let Some(src) = backdrop_source {
                    let paintable = gtk4::WidgetPaintable::new(Some(src));
                    let obj_weak = obj.downgrade();
                    paintable.connect_invalidate_size(move |_| {
                        if let Some(card) = obj_weak.upgrade() {
                            card.queue_draw();
                        }
                    });
                    *obj.imp().bg_widget.borrow_mut() = Some(src.downgrade());
                    *obj.imp().bg_paintable.borrow_mut() = Some(paintable);
                }

                let obj_theme = obj.downgrade();
                libadwaita::StyleManager::default().connect_dark_notify(move |_| {
                    if let Some(card) = obj_theme.upgrade() {
                        card.queue_draw();
                    }
                });
            }

            obj
        }

        pub fn animate_open(&self) {
            let imp = self.imp();
            let gen = imp.anim_gen.get().wrapping_add(1);
            imp.anim_gen.set(gen);
            let standalone = imp.standalone.get();
            let (s0, a0, y0) = quicklook_open_anim_state(0.0, standalone);
            imp.anim_scale.set(s0);
            imp.anim_opacity.set(a0);
            imp.anim_offset_y.set(y0);
            self.queue_draw();

            let start_time: Cell<Option<i64>> = Cell::new(None);
            self.add_tick_callback(move |widget, clock| {
                let Some(card) = widget.downcast_ref::<Self>() else {
                    return glib::ControlFlow::Break;
                };
                let imp = card.imp();
                if imp.anim_gen.get() != gen {
                    return glib::ControlFlow::Break;
                }
                let now = clock.frame_time();
                let t0 = match start_time.get() {
                    Some(v) => v,
                    None => {
                        start_time.set(Some(now));
                        return glib::ControlFlow::Continue;
                    }
                };
                let t = ((now - t0) as f64 / QL_OPEN_ANIM_US).clamp(0.0, 1.0);
                let (s, a, y) = quicklook_open_anim_state(t, standalone);
                imp.anim_scale.set(s);
                imp.anim_opacity.set(a);
                imp.anim_offset_y.set(y);
                card.queue_draw();
                if t >= 1.0 {
                    imp.anim_scale.set(1.0);
                    imp.anim_opacity.set(1.0);
                    imp.anim_offset_y.set(0.0);
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }

        pub fn animate_close(&self, on_finished: impl FnOnce() + 'static) {
            let imp = self.imp();
            let gen = imp.anim_gen.get().wrapping_add(1);
            imp.anim_gen.set(gen);
            let standalone = imp.standalone.get();
            let start_scale = imp.anim_scale.get();
            let start_opacity = imp.anim_opacity.get();
            let start_offset_y = imp.anim_offset_y.get();

            let on_finished = RefCell::new(Some(on_finished));
            let start_time: Cell<Option<i64>> = Cell::new(None);
            self.add_tick_callback(move |widget, clock| {
                let Some(card) = widget.downcast_ref::<Self>() else {
                    return glib::ControlFlow::Break;
                };
                let imp = card.imp();
                if imp.anim_gen.get() != gen {
                    return glib::ControlFlow::Break;
                }
                let now = clock.frame_time();
                let t0 = match start_time.get() {
                    Some(v) => v,
                    None => {
                        start_time.set(Some(now));
                        return glib::ControlFlow::Continue;
                    }
                };
                let t = ((now - t0) as f64 / QL_CLOSE_ANIM_US).clamp(0.0, 1.0);
                let (s, a, y) = quicklook_close_anim_state(
                    t,
                    standalone,
                    start_scale,
                    start_opacity,
                    start_offset_y,
                );
                imp.anim_scale.set(s);
                imp.anim_opacity.set(a);
                imp.anim_offset_y.set(y);
                card.queue_draw();
                if t >= 1.0 {
                    if let Some(cb) = on_finished.borrow_mut().take() {
                        cb();
                    }
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
    }
}

pub fn fit_quicklook_media_card_size(
    media_w: i32,
    media_h: i32,
    max_card_w: i32,
    max_card_h: i32,
    chrome_w: i32,
    chrome_h: i32,
) -> (i32, i32, i32, i32) {
    let avail_w = (max_card_w - chrome_w).max(1) as f64;
    let avail_h = (max_card_h - chrome_h).max(1) as f64;
    let mw = media_w.max(1) as f64;
    let mh = media_h.max(1) as f64;

    // Nunca estirar archivos pequeños: solo reducir proporcionalmente si exceden el espacio disponible.
    let scale_down = (avail_w / mw).min(avail_h / mh).min(1.0);
    let fit_w = (mw * scale_down).round().max(1.0) as i32;
    let fit_h = (mh * scale_down).round().max(1.0) as i32;

    let min_w = QL_MIN_CARD_W.min(max_card_w);
    let min_h = QL_MIN_CARD_H.min(max_card_h);
    let card_w = (fit_w + chrome_w).clamp(min_w, max_card_w);
    let card_h = (fit_h + chrome_h).clamp(min_h, max_card_h);
    (card_w, card_h, fit_w, fit_h)
}

pub fn fit_quicklook_card_size(
    img_w: i32,
    img_h: i32,
    max_card_w: i32,
    max_card_h: i32,
) -> (i32, i32, i32, i32) {
    fit_quicklook_media_card_size(
        img_w,
        img_h,
        max_card_w,
        max_card_h,
        QL_IMAGE_INSET * 2,
        QL_HEADER_HEIGHT + QL_IMAGE_INSET,
    )
}

pub fn quicklook_image_corner_radius(card_w: i32, card_h: i32, fit_w: i32, fit_h: i32) -> f64 {
    let stage_w = (card_w - QL_IMAGE_INSET * 2).max(1);
    let stage_h = (card_h - QL_HEADER_HEIGHT - QL_IMAGE_INSET).max(1);
    if fit_w >= stage_w - 2 && fit_h >= stage_h - 2 {
        quicklook_concentric_inner_radius(
            quicklook_card_squircle_radius(),
            QL_IMAGE_INSET as f64,
        )
    } else {
        0.0
    }
}

/// Extrae las dimensiones `(ancho, alto)` desde la cabecera RIFF/WEBP (`VP8 `, `VP8L`, `VP8X`)
/// sin decodificar píxeles, útil cuando `gdk-pixbuf` no tiene loader WebP instalado.
pub fn parse_webp_dimensions_from_bytes(buf: &[u8]) -> Option<(i32, i32)> {
    if buf.len() < 30 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WEBP" {
        return None;
    }
    let chunk = &buf[12..16];
    if chunk == b"VP8X" {
        let w = 1 + (buf[24] as u32 | ((buf[25] as u32) << 8) | ((buf[26] as u32) << 16));
        let h = 1 + (buf[27] as u32 | ((buf[28] as u32) << 8) | ((buf[29] as u32) << 16));
        if w > 0 && h > 0 {
            return Some((w as i32, h as i32));
        }
    } else if chunk == b"VP8L" && buf[20] == 0x2f {
        let bits = u32::from_le_bytes([buf[21], buf[22], buf[23], buf[24]]);
        let w = 1 + (bits & 0x3fff);
        let h = 1 + ((bits >> 14) & 0x3fff);
        if w > 0 && h > 0 {
            return Some((w as i32, h as i32));
        }
    } else if chunk == b"VP8 " && buf[23] == 0x9d && buf[24] == 0x01 && buf[25] == 0x2a {
        let w = u16::from_le_bytes([buf[26], buf[27]]) & 0x3fff;
        let h = u16::from_le_bytes([buf[28], buf[29]]) & 0x3fff;
        if w > 0 && h > 0 {
            return Some((w as i32, h as i32));
        }
    }
    None
}

fn probe_webp_dimensions(path: &Path) -> Option<(i32, i32)> {
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0u8; 32];
    let n = file.read(&mut header).ok()?;
    parse_webp_dimensions_from_bytes(&header[..n])
}

fn probe_image_dimensions(path: &Path) -> Option<(i32, i32)> {
    if let Some((_, w, h)) = Pixbuf::file_info(path) {
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    if path
        .extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("webp"))
    {
        return probe_webp_dimensions(path);
    }
    None
}

fn default_open_app_name(path: &Path) -> Option<String> {
    let gfile = gio::File::for_path(path);
    let info = gfile
        .query_default_handler(gio::Cancellable::NONE)
        .ok()
        .or_else(|| {
            let ctype = gfile
                .query_info(
                    "standard::content-type",
                    gio::FileQueryInfoFlags::NONE,
                    gio::Cancellable::NONE,
                )
                .ok()
                .and_then(|fi| fi.content_type())?;
            gio::AppInfo::default_for_type(ctype.as_str(), false)
        })?;
    let name = info.display_name().to_string();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_open_pill_label(path: &Path, compact: bool, es: bool) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let app_name = default_open_app_name(path).unwrap_or_else(|| {
        if is_image_extension(&ext) || ext == "pdf" {
            if es {
                "Vista Previa".to_string()
            } else {
                "Preview".to_string()
            }
        } else if es {
            "Abrir".to_string()
        } else {
            "Open".to_string()
        }
    });
    if compact {
        if app_name.chars().count() > 16 {
            if es {
                "Abrir".to_string()
            } else {
                "Open".to_string()
            }
        } else {
            app_name
        }
    } else if es {
        format!("Abrir con {app_name}")
    } else {
        format!("Open with {app_name}")
    }
}

fn rotate_rgba_frame_ccw(frame: &DecodedRgbaFrame) -> DecodedRgbaFrame {
    let w = frame.width.max(1) as usize;
    let h = frame.height.max(1) as usize;
    let src = frame.bytes.as_ref();
    let new_w = h;
    let new_h = w;
    let new_stride = new_w * 4;
    let mut dst = vec![0u8; new_stride * new_h];

    for y in 0..h {
        let src_row = y * frame.stride;
        for x in 0..w {
            let nx = y;
            let ny = w - 1 - x;
            let sp = src_row + x * 4;
            let dp = ny * new_stride + nx * 4;
            if sp + 4 <= src.len() && dp + 4 <= dst.len() {
                dst[dp..dp + 4].copy_from_slice(&src[sp..sp + 4]);
            }
        }
    }

    DecodedRgbaFrame {
        width: new_w as i32,
        height: new_h as i32,
        stride: new_stride,
        bytes: glib::Bytes::from_owned(dst),
    }
}

fn primary_monitor_work_bounds() -> (i32, i32) {
    if let Some(display) = gtk4::gdk::Display::default() {
        let monitors = display.monitors();
        if let Some(obj) = monitors.item(0) {
            if let Ok(mon) = obj.downcast::<gtk4::gdk::Monitor>() {
                let geo = mon.geometry();
                if geo.width() > 0 && geo.height() > 0 {
                    let mw = ((geo.width() as f64) * 0.80).round() as i32;
                    let mh = ((geo.height() as f64) * 0.84).round() as i32;
                    return (mw.clamp(460, 1280), mh.clamp(360, 960));
                }
            }
        }
    }
    (920, 720)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickLookContentSizing {
    Image { width: i32, height: i32 },
    Video { width: i32, height: i32 },
    Pdf { width: i32, height: i32, has_footer: bool },
    Compact,
    Document,
}

fn probe_video_dimensions_fast(path: &Path) -> Option<(u32, u32)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(safe_ffmpeg_input_arg(path))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut w = None;
    let mut h = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "width" => w = v.trim().parse::<u32>().ok().filter(|&n| n > 0),
            "height" => h = v.trim().parse::<u32>().ok().filter(|&n| n > 0),
            _ => {}
        }
    }
    Some((w?, h?))
}

fn initial_content_sizing_for_item(item: &FileItem, probe_video: bool) -> QuickLookContentSizing {
    if item.is_dir {
        return QuickLookContentSizing::Compact;
    }
    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if is_image_extension(&ext) {
        if let Some((w, h)) = probe_image_dimensions(&item.path) {
            return QuickLookContentSizing::Image {
                width: w,
                height: h,
            };
        }
        return QuickLookContentSizing::Compact;
    }
    if is_video_extension(&ext) {
        if probe_video {
            if let Some((vw, vh)) = probe_video_dimensions_fast(&item.path) {
                let (pw, ph) = fit_video_dimensions(vw, vh, 760, 440);
                return QuickLookContentSizing::Video {
                    width: pw as i32,
                    height: ph as i32,
                };
            }
        }
        return QuickLookContentSizing::Document;
    }
    if ext == "pdf" || is_archive_extension(&ext) || is_text_preview_candidate(&ext, &item.kind_str) {
        return QuickLookContentSizing::Document;
    }
    QuickLookContentSizing::Compact
}

pub fn compute_quicklook_card_size(
    sizing: QuickLookContentSizing,
    max_w: i32,
    max_h: i32,
    expanded: bool,
) -> (i32, i32, Option<(i32, i32)>) {
    match sizing {
        QuickLookContentSizing::Image { width, height } => {
            let (cw, ch, fit_w, fit_h) = fit_quicklook_card_size(width, height, max_w, max_h);
            (cw, ch, Some((fit_w, fit_h)))
        }
        QuickLookContentSizing::Video { width, height } => {
            let (cw, ch, fit_w, fit_h) = fit_quicklook_media_card_size(
                width,
                height,
                max_w,
                max_h,
                QL_VIDEO_CHROME_W,
                QL_VIDEO_CHROME_H,
            );
            (cw, ch, Some((fit_w, fit_h)))
        }
        QuickLookContentSizing::Pdf {
            width,
            height,
            has_footer,
        } => {
            let chrome_h = QL_PDF_CHROME_H_BASE + if has_footer { QL_PDF_FOOTER_H } else { 0 };
            let (cw, ch, fit_w, fit_h) = fit_quicklook_media_card_size(
                width,
                height,
                max_w,
                max_h,
                QL_PDF_CHROME_W,
                chrome_h,
            );
            (cw, ch, Some((fit_w, fit_h)))
        }
        QuickLookContentSizing::Compact => {
            let min_w = QL_MIN_CARD_W.min(max_w);
            let min_h = QL_MIN_CARD_H.min(max_h);
            let cw = if expanded {
                max_w
            } else {
                460.clamp(min_w, max_w)
            };
            let ch = if expanded {
                max_h
            } else {
                380.clamp(min_h, max_h)
            };
            (cw, ch, None)
        }
        QuickLookContentSizing::Document => {
            let min_w = QL_MIN_CARD_W.min(max_w);
            let min_h = QL_MIN_CARD_H.min(max_h);
            let cw = if expanded {
                max_w
            } else {
                max_w.clamp(460.min(max_w).max(min_w), 820.max(min_w))
            };
            let ch = if expanded {
                max_h
            } else {
                max_h.clamp(360.min(max_h).max(min_h), 600.max(min_h))
            };
            (cw, ch, None)
        }
    }
}

#[derive(Clone)]
struct ImageViewSlot {
    frame: Rc<RefCell<Option<DecodedRgbaFrame>>>,
    picture: Picture,
    img_frame: Overlay,
    size_spacer: GtkBox,
    live_text: Option<malus_de_common::live_text::LiveTextController>,
}

pub struct QuickLook {
    window: RefCell<Option<Window>>,
    host_overlay: RefCell<Option<Overlay>>,
    backdrop: RefCell<Option<GtkBox>>,
    container: RefCell<Option<GtkBox>>,
    anim_card: RefCell<Option<squircle_card::QuickLookSquircleCard>>,
    closing: Cell<bool>,
    expanded: Cell<bool>,
    anim_gen: Cell<u64>,
    current_path: RefCell<Option<PathBuf>>,
    content_sizing: Cell<QuickLookContentSizing>,
    items_list: RefCell<Vec<FileItem>>,
    current_index: Cell<usize>,
    content_holder: RefCell<Option<GtkBox>>,
    title_label: RefCell<Option<Label>>,
    open_btn: RefCell<Option<Button>>,
    expand_btn: RefCell<Option<Button>>,
    rotate_btn: RefCell<Option<Button>>,
    image_slot: Rc<RefCell<Option<ImageViewSlot>>>,
    open_btn_path: Rc<RefCell<PathBuf>>,
    media_handles: Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: Rc<RefCell<Option<PageStepCallback>>>,
    preview_gen: Rc<Cell<u64>>,
    cache: Arc<Mutex<PreviewCache>>,
}

impl QuickLook {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            window: RefCell::new(None),
            host_overlay: RefCell::new(None),
            backdrop: RefCell::new(None),
            container: RefCell::new(None),
            anim_card: RefCell::new(None),
            closing: Cell::new(false),
            expanded: Cell::new(false),
            anim_gen: Cell::new(0),
            current_path: RefCell::new(None),
            content_sizing: Cell::new(QuickLookContentSizing::Compact),
            items_list: RefCell::new(Vec::new()),
            current_index: Cell::new(0),
            content_holder: RefCell::new(None),
            title_label: RefCell::new(None),
            open_btn: RefCell::new(None),
            expand_btn: RefCell::new(None),
            rotate_btn: RefCell::new(None),
            image_slot: Rc::new(RefCell::new(None)),
            open_btn_path: Rc::new(RefCell::new(PathBuf::new())),
            media_handles: Rc::new(RefCell::new(Vec::new())),
            page_step_cb: Rc::new(RefCell::new(None)),
            preview_gen: Rc::new(Cell::new(0)),
            cache: Arc::new(Mutex::new(PreviewCache::new(24))),
        })
    }

    pub fn is_open(&self) -> bool {
        (self.backdrop.borrow().is_some() || self.window.borrow().is_some()) && !self.closing.get()
    }

    pub fn close(self: &Rc<Self>) {
        if self.closing.get() {
            return;
        }
        let has_overlay = self.backdrop.borrow().is_some();
        let has_window = self.window.borrow().is_some();
        if !has_overlay && !has_window {
            return;
        }

        self.closing.set(true);
        let next_gen = self.anim_gen.get().wrapping_add(1);
        self.anim_gen.set(next_gen);
        self.preview_gen.set(self.preview_gen.get().wrapping_add(1));
        self.stop_active_media();

        if let Some(bd) = self.backdrop.borrow().as_ref() {
            bd.remove_css_class("ql-open");
            bd.add_css_class("ql-closing");
        }
        if let Some(card) = self.container.borrow().as_ref() {
            card.remove_css_class("ql-open");
            card.add_css_class("ql-closing");
        }

        let anim_card_opt = self.anim_card.borrow().clone();
        if let Some(anim_card) = anim_card_opt {
            let self_tick = self.clone();
            anim_card.animate_close(move || {
                if self_tick.anim_gen.get() != next_gen {
                    return;
                }
                self_tick.finish_close_immediately();
            });
        }

        // Respaldo de seguridad por si el frame clock no dispara (ventana oculta/minimizada)
        let self_c = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(260), move || {
            if self_c.anim_gen.get() != next_gen {
                return;
            }
            self_c.finish_close_immediately();
        });
    }

    fn finish_close_immediately(&self) {
        self.stop_active_media();
        let host_overlay = self.host_overlay.borrow_mut().take();
        let backdrop = self.backdrop.borrow_mut().take();
        *self.container.borrow_mut() = None;
        let win_opt = self.window.borrow_mut().take();
        self.clear_refs();
        self.closing.set(false);
        if let (Some(overlay), Some(bd)) = (host_overlay, backdrop) {
            overlay.remove_overlay(&bd);
        }
        if let Some(win) = win_opt {
            win.close();
        }
    }

    #[allow(dead_code)]
    pub fn toggle(self: &Rc<Self>, overlay: &Overlay, item: Option<FileItem>) {
        let items = item.iter().cloned().collect::<Vec<_>>();
        self.toggle_in_overlay(overlay, item, items);
    }

    pub fn toggle_in_overlay(
        self: &Rc<Self>,
        overlay: &Overlay,
        item: Option<FileItem>,
        items: Vec<FileItem>,
    ) {
        if self.is_open() {
            self.close();
            return;
        }
        // Si seguía en medio de la animación de cierre, limpiar antes de reabrir
        if self.closing.get() {
            let next_gen = self.anim_gen.get().wrapping_add(1);
            self.anim_gen.set(next_gen);
            self.finish_close_immediately();
        }

        let Some(item) = item else { return };

        // Puente con aplicación dedicada `malus-de-preview` (ADR 0007) cuando esté disponible
        if try_external_preview_dbus(&item.path, &items) {
            return;
        }

        let idx = items
            .iter()
            .position(|it| it.path == item.path)
            .unwrap_or(0);
        *self.items_list.borrow_mut() = if items.is_empty() {
            vec![item.clone()]
        } else {
            items
        };
        self.current_index.set(idx);
        self.show_in_overlay(overlay, &item);
        self.prefetch_neighbors();
    }

    /// Precarga silenciosamente en segundo plano el ítem seleccionado antes de pulsar Espacio
    /// (imitando la caché predictiva de Quick Look en macOS Finder).
    pub fn prefetch_item(&self, item: &FileItem) {
        if item.is_dir {
            return;
        }
        let ext = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !needs_background_preview(&ext, &item.kind_str) {
            return;
        }
        spawn_prefetch_into_cache(&self.cache, item.path.clone(), ext, item.kind_str.clone());
    }

    fn prefetch_neighbors(&self) {
        let items = self.items_list.borrow();
        let len = items.len();
        if len <= 1 {
            return;
        }
        let idx = self.current_index.get().min(len - 1);
        let next_idx = (idx + 1) % len;
        let prev_idx = (idx + len - 1) % len;
        if next_idx != idx {
            self.prefetch_item(&items[next_idx]);
        }
        if prev_idx != idx && prev_idx != next_idx {
            self.prefetch_item(&items[prev_idx]);
        }
    }

    fn stop_active_media(&self) {
        let handles: Vec<_> = self.media_handles.borrow_mut().drain(..).collect();
        for handle in handles {
            handle.stop();
        }
    }

    fn clear_refs(&self) {
        self.stop_active_media();
        self.preview_gen.set(self.preview_gen.get().wrapping_add(1));
        self.expanded.set(false);
        self.content_sizing.set(QuickLookContentSizing::Compact);
        *self.image_slot.borrow_mut() = None;
        *self.anim_card.borrow_mut() = None;
        *self.page_step_cb.borrow_mut() = None;
        *self.current_path.borrow_mut() = None;
        *self.content_holder.borrow_mut() = None;
        *self.title_label.borrow_mut() = None;
        *self.open_btn.borrow_mut() = None;
        *self.expand_btn.borrow_mut() = None;
        *self.rotate_btn.borrow_mut() = None;
        self.items_list.borrow_mut().clear();
    }

    fn apply_card_geometry(&self) {
        let Some(card) = self.container.borrow().clone() else {
            return;
        };
        let expanded = self.expanded.get();
        let es = malus_de_common::is_spanish_locale();

        let (max_w, max_h, is_standalone) = if let Some(overlay) = self.host_overlay.borrow().as_ref() {
            let ow = overlay.width();
            let oh = overlay.height();
            if expanded {
                let mw = if ow > 0 { (ow - 24).max(QL_MIN_CARD_W) } else { 960 };
                let mh = if oh > 0 { (oh - 24).max(QL_MIN_CARD_H) } else { 700 };
                (mw, mh, false)
            } else {
                let mw = if ow > 0 {
                    ((ow as f64) * 0.88).round() as i32
                } else {
                    840
                }
                .clamp(QL_MIN_CARD_W, 1120);
                let mh = if oh > 0 {
                    ((oh as f64) * 0.90).round() as i32
                } else {
                    620
                }
                .clamp(QL_MIN_CARD_H, 820);
                (mw, mh, false)
            }
        } else {
            let (mw, mh) = primary_monitor_work_bounds();
            (mw, mh, true)
        };

        let sizing = self.content_sizing.get();
        let card_w = if expanded && is_standalone {
            card.set_size_request(-1, -1);
            if let Some(slot) = self.image_slot.borrow().as_ref() {
                slot.size_spacer.set_size_request(-1, -1);
                slot.img_frame.set_halign(Align::Fill);
                slot.img_frame.set_valign(Align::Fill);
                if let Some(frame) = slot.frame.borrow().as_ref() {
                    let tex = frame.to_texture();
                    slot.picture.set_paintable(Some(&tex));
                }
            }
            max_w
        } else {
            let (cw, ch, fit_dims) = compute_quicklook_card_size(sizing, max_w, max_h, expanded);
            card.set_size_request(cw, ch);
            if matches!(sizing, QuickLookContentSizing::Image { .. }) {
                if let (Some(slot), Some((fit_w, fit_h))) =
                    (self.image_slot.borrow().as_ref(), fit_dims)
                {
                    slot.img_frame.set_halign(Align::Center);
                    slot.img_frame.set_valign(Align::Center);
                    slot.size_spacer.set_size_request(fit_w, fit_h);
                    if let Some(frame) = slot.frame.borrow().as_ref() {
                        let inner_r = quicklook_image_corner_radius(cw, ch, fit_w, fit_h);
                        let tex = if inner_r > 0.5 {
                            frame.to_squircle_texture(fit_w, fit_h, inner_r)
                        } else {
                            frame.to_texture()
                        };
                        slot.picture.set_paintable(Some(&tex));
                    }
                }
            }
            if is_standalone {
                if let Some(win) = self.window.borrow().as_ref() {
                    win.set_default_size(cw, ch);
                    win.queue_resize();
                }
            }
            cw
        };

        if let (Some(btn), Some(path)) = (
            self.open_btn.borrow().as_ref(),
            self.current_path.borrow().as_ref(),
        ) {
            let label = format_open_pill_label(path, card_w < 460, es);
            btn.set_label(&label);
        }
    }

    fn toggle_expanded(&self) {
        let next = !self.expanded.get();
        self.expanded.set(next);
        if let Some(btn) = self.expand_btn.borrow().as_ref() {
            if let Some(img) = btn.child().and_downcast::<gtk4::Image>() {
                img.set_icon_name(Some(if next {
                    "view-restore-symbolic"
                } else {
                    "view-fullscreen-symbolic"
                }));
            }
        }
        let win_opt = self.window.borrow().clone();
        if let Some(win) = win_opt {
            if next {
                win.set_resizable(true);
                win.fullscreen();
            } else {
                win.unfullscreen();
                win.set_resizable(false);
            }
        }
        self.apply_card_geometry();
    }

    fn rotate_current_image_ccw(&self) {
        let new_dims = {
            let slot_opt = self.image_slot.borrow();
            let Some(slot) = slot_opt.as_ref() else {
                return;
            };
            let mut frame_guard = slot.frame.borrow_mut();
            let Some(cur_frame) = frame_guard.as_ref() else {
                return;
            };
            let rotated = rotate_rgba_frame_ccw(cur_frame);
            let dims = (rotated.width, rotated.height);
            let tex = rotated.to_texture();
            slot.picture.set_paintable(Some(&tex));
            *frame_guard = Some(rotated);
            if let Some(ref lt) = slot.live_text {
                lt.rotate_ccw();
            }
            dims
        };
        self.content_sizing.set(QuickLookContentSizing::Image {
            width: new_dims.0,
            height: new_dims.1,
        });
        self.apply_card_geometry();
    }

    pub fn copy_live_text(&self) -> bool {
        self.image_slot
            .borrow()
            .as_ref()
            .and_then(|s| s.live_text.as_ref())
            .is_some_and(|lt| lt.copy_selection_or_highlighted())
    }

    pub fn select_all_live_text(&self) -> bool {
        let slot_opt = self.image_slot.borrow();
        let Some(lt) = slot_opt.as_ref().and_then(|s| s.live_text.as_ref()) else {
            return false;
        };
        if lt.has_detected_text() {
            lt.select_all();
            true
        } else {
            false
        }
    }

    pub fn update_if_open(self: &Rc<Self>, item: &FileItem) {
        if self.is_open() {
            self.stop_active_media();
            *self.page_step_cb.borrow_mut() = None;
            *self.image_slot.borrow_mut() = None;

            let ext = item
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let is_img = is_image_extension(&ext);
            let is_standalone = self.host_overlay.borrow().is_none();
            self.content_sizing
                .set(initial_content_sizing_for_item(item, is_standalone));
            if let Some(rb) = self.rotate_btn.borrow().as_ref() {
                rb.set_visible(is_img);
            }

            if let Some(tl) = self.title_label.borrow().as_ref() {
                tl.set_label(&item.name);
            }
            *self.open_btn_path.borrow_mut() = item.path.clone();
            *self.current_path.borrow_mut() = Some(item.path.clone());
            if let Some(win) = self.window.borrow().as_ref() {
                win.set_title(Some(&item.name));
            }

            self.apply_card_geometry();

            let holder_opt = self.content_holder.borrow().clone();
            if let Some(holder) = holder_opt.as_ref() {
                let ql_weak = Rc::downgrade(self);
                let on_sizing_ready: Rc<dyn Fn(QuickLookContentSizing)> = Rc::new(move |sizing| {
                    if let Some(ql) = ql_weak.upgrade() {
                        ql.content_sizing.set(sizing);
                        ql.apply_card_geometry();
                    }
                });
                populate_preview_async(
                    item,
                    holder,
                    &self.media_handles,
                    &self.page_step_cb,
                    &self.preview_gen,
                    &self.cache,
                    &self.image_slot,
                    on_sizing_ready,
                    0,
                );
            }
        }
    }

    pub fn step_item(self: &Rc<Self>, delta: isize) {
        let len = self.items_list.borrow().len();
        if len <= 1 {
            return;
        }
        let curr = self.current_index.get() as isize;
        let next = (curr + delta).rem_euclid(len as isize) as usize;
        self.current_index.set(next);
        let item = self.items_list.borrow().get(next).cloned();
        if let Some(item) = item {
            self.update_if_open(&item);
            self.prefetch_neighbors();
        }
    }

    pub fn step_pdf_page(&self, delta: i32) -> bool {
        let cb = self.page_step_cb.borrow().clone();
        if let Some(cb) = cb {
            return cb(delta);
        }
        false
    }

    pub fn show_standalone(self: &Rc<Self>, app: &Application, item: &FileItem) {
        let initial_sizing = initial_content_sizing_for_item(item, true);
        let (max_w, max_h) = primary_monitor_work_bounds();
        let (init_w, init_h, _) = compute_quicklook_card_size(initial_sizing, max_w, max_h, false);

        let win = Window::builder()
            .application(app)
            .title(&item.name)
            .default_width(init_w)
            .default_height(init_h)
            .resizable(false)
            .decorated(false)
            .css_classes(["branches-quicklook-window"])
            .build();
        *self.items_list.borrow_mut() = vec![item.clone()];
        self.current_index.set(0);
        self.expanded.set(false);
        self.content_sizing.set(initial_sizing);

        *self.window.borrow_mut() = Some(win.clone());
        let root_box = self.build_preview_card(item, true, None);
        win.set_child(Some(&root_box));

        let key_controller = EventControllerKey::new();
        key_controller.set_propagation_phase(PropagationPhase::Capture);
        let self_key = self.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            let has_mod = state.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK);
            let is_copy_mod = state.intersects(
                ModifierType::CONTROL_MASK | ModifierType::SUPER_MASK | ModifierType::META_MASK,
            );
            if is_copy_mod && matches!(keyval, Key::c | Key::C) && self_key.copy_live_text() {
                return glib::Propagation::Stop;
            }
            if is_copy_mod && matches!(keyval, Key::a | Key::A) && self_key.select_all_live_text() {
                return glib::Propagation::Stop;
            }
            if !has_mod && matches!(keyval, Key::space | Key::KP_Space | Key::Escape) {
                self_key.close();
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Left {
                self_key.step_item(-1);
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Right {
                self_key.step_item(1);
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Page_Up && self_key.step_pdf_page(-1) {
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Page_Down && self_key.step_pdf_page(1) {
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(key_controller);

        let self_destroy = self.clone();
        win.connect_close_request(move |_| {
            self_destroy.stop_active_media();
            *self_destroy.container.borrow_mut() = None;
            *self_destroy.window.borrow_mut() = None;
            self_destroy.clear_refs();
            self_destroy.closing.set(false);
            glib::Propagation::Proceed
        });

        *self.container.borrow_mut() = Some(root_box.clone());
        self.apply_card_geometry();
        win.present();
        self.trigger_macos_enter_animation(&root_box, None);
    }

    fn show_in_overlay(self: &Rc<Self>, overlay: &Overlay, item: &FileItem) {
        let next_gen = self.anim_gen.get().wrapping_add(1);
        self.anim_gen.set(next_gen);
        self.closing.set(false);
        self.expanded.set(false);

        self.content_sizing
            .set(initial_content_sizing_for_item(item, false));

        let backdrop = GtkBox::new(Orientation::Vertical, 0);
        backdrop.add_css_class("branches-quicklook-backdrop");
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);

        *self.host_overlay.borrow_mut() = Some(overlay.clone());
        let backdrop_source = overlay.child();
        let card = self.build_preview_card(item, false, backdrop_source.as_ref());
        card.set_halign(Align::Center);
        card.set_valign(Align::Center);
        card.set_vexpand(true);
        card.set_margin_top(16);
        card.set_margin_bottom(16);
        card.set_margin_start(16);
        card.set_margin_end(16);

        // Clic fuera de la tarjeta cierra Quick Look (sin interceptar los botones internos de la tarjeta)
        let bd_click = GestureClick::new();
        bd_click.set_button(gtk4::gdk::BUTTON_PRIMARY);
        let ql_bd = self.clone();
        let card_for_bounds = card.downgrade();
        let bd_for_bounds = backdrop.downgrade();
        bd_click.connect_released(move |_, _, x, y| {
            if let (Some(c), Some(bd)) = (card_for_bounds.upgrade(), bd_for_bounds.upgrade()) {
                if let Some(bounds) = c.compute_bounds(&bd) {
                    let pt = gtk4::graphene::Point::new(x as f32, y as f32);
                    if bounds.contains_point(&pt) {
                        return;
                    }
                }
            }
            ql_bd.close();
        });
        backdrop.add_controller(bd_click);

        backdrop.append(&card);
        *self.backdrop.borrow_mut() = Some(backdrop.clone());
        *self.container.borrow_mut() = Some(card.clone());
        self.apply_card_geometry();

        overlay.add_overlay(&backdrop);
        self.trigger_macos_enter_animation(&card, Some(&backdrop));
    }

    fn trigger_macos_enter_animation(&self, card: &GtkBox, backdrop: Option<&GtkBox>) {
        if let Some(anim_card) = self.anim_card.borrow().clone() {
            anim_card.animate_open();
        }
        let bd_opt = backdrop.cloned();
        card.add_tick_callback(move |widget, _| {
            widget.add_css_class("ql-open");
            if let Some(ref bd) = bd_opt {
                bd.add_css_class("ql-open");
            }
            glib::ControlFlow::Break
        });
    }

    fn build_preview_card(
        self: &Rc<Self>,
        item: &FileItem,
        draggable: bool,
        backdrop_source: Option<&gtk4::Widget>,
    ) -> GtkBox {
        let es = malus_de_common::is_spanish_locale();
        let ext = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let is_img = is_image_extension(&ext);

        let root_box = GtkBox::new(Orientation::Vertical, 0);
        root_box.add_css_class("branches-quicklook-container");
        root_box.set_overflow(if draggable {
            Overflow::Hidden
        } else {
            Overflow::Visible
        });

        let content_vbox = GtkBox::new(Orientation::Vertical, 0);
        content_vbox.set_hexpand(true);
        content_vbox.set_vexpand(true);
        let squircle_card =
            squircle_card::QuickLookSquircleCard::new(&content_vbox, backdrop_source, draggable);
        *self.anim_card.borrow_mut() = Some(squircle_card.clone());
        root_box.append(&squircle_card);
        let inner_box = content_vbox;

        // Barra superior compacta estilo macOS Tahoe Quick Look
        let header_box = GtkBox::new(Orientation::Horizontal, 6);
        header_box.add_css_class("branches-quicklook-header");

        let close_icon = gtk4::Image::from_icon_name("window-close-symbolic");
        close_icon.set_pixel_size(10);
        let close_btn = Button::builder()
            .child(&close_icon)
            .has_frame(false)
            .can_focus(false)
            .valign(Align::Center)
            .css_classes(["branches-quicklook-close-btn", "branches-quicklook-hud-btn"])
            .tooltip_text(if es {
                "Cerrar Vista Rápida (Espacio / Esc)"
            } else {
                "Close Quick Look (Space / Esc)"
            })
            .build();
        let ql_close = self.clone();
        close_btn.connect_clicked(move |_| ql_close.close());
        header_box.append(&close_btn);

        let expand_icon = gtk4::Image::from_icon_name("view-fullscreen-symbolic");
        expand_icon.set_pixel_size(10);
        let expand_btn = Button::builder()
            .child(&expand_icon)
            .has_frame(false)
            .can_focus(false)
            .valign(Align::Center)
            .css_classes(["branches-quicklook-hud-btn"])
            .tooltip_text(if es {
                "Ampliar / restaurar"
            } else {
                "Expand / Restore"
            })
            .build();
        {
            let ql_exp = self.clone();
            expand_btn.connect_clicked(move |_| ql_exp.toggle_expanded());
        }
        header_box.append(&expand_btn);

        let title_label = Label::builder()
            .label(&item.name)
            .halign(Align::Start)
            .valign(Align::Center)
            .hexpand(true)
            .margin_start(4)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(["bold", "branches-quicklook-title"])
            .build();
        header_box.append(&title_label);

        // Botón Girar a la izquierda (solo imágenes)
        let rotate_icon = gtk4::Image::from_icon_name("object-rotate-left-symbolic");
        rotate_icon.set_pixel_size(15);
        let rotate_btn = Button::builder()
            .child(&rotate_icon)
            .has_frame(false)
            .can_focus(false)
            .valign(Align::Center)
            .visible(is_img)
            .css_classes(["branches-quicklook-icon-btn"])
            .tooltip_text(if es {
                "Girar a la izquierda"
            } else {
                "Rotate Left"
            })
            .build();
        {
            let ql_rot = self.clone();
            rotate_btn.connect_clicked(move |_| ql_rot.rotate_current_image_ccw());
        }
        header_box.append(&rotate_btn);

        // Botón Compartir
        let share_icon = gtk4::Image::from_icon_name("send-to-symbolic");
        share_icon.set_pixel_size(15);
        let share_btn = Button::builder()
            .child(&share_icon)
            .has_frame(false)
            .can_focus(false)
            .valign(Align::Center)
            .css_classes(["branches-quicklook-icon-btn"])
            .tooltip_text(if es { "Compartir" } else { "Share" })
            .build();
        {
            let p_share = self.open_btn_path.clone();
            let root_ref = root_box.downgrade();
            share_btn.connect_clicked(move |_| {
                let path = p_share.borrow().clone();
                let parent_win = root_ref
                    .upgrade()
                    .and_then(|w| w.root())
                    .and_downcast::<Window>();
                crate::app_window::share_paths_via_malus_link(
                    parent_win.as_ref(),
                    None,
                    vec![path],
                );
            });
        }
        header_box.append(&share_btn);

        *self.open_btn_path.borrow_mut() = item.path.clone();
        let p_for_open = self.open_btn_path.clone();
        let ql_open_close = self.clone();
        let open_btn = Button::builder()
            .label(format_open_pill_label(&item.path, false, es))
            .has_frame(false)
            .can_focus(false)
            .css_classes(["branches-quicklook-open-pill"])
            .valign(Align::Center)
            .build();
        open_btn.connect_clicked(move |_| {
            let path = p_for_open.borrow().clone();
            let _ = open_file(&path);
            ql_open_close.close();
        });
        header_box.append(&open_btn);

        if draggable {
            let handle = WindowHandle::builder().child(&header_box).build();
            inner_box.append(&handle);
        } else {
            inner_box.append(&header_box);
        }

        // Contenedor dinámico de previsualización (carga asíncrona sin bloquear la transición)
        let content_holder = GtkBox::new(Orientation::Vertical, 0);
        content_holder.set_vexpand(true);
        content_holder.set_hexpand(true);
        content_holder.add_css_class("branches-quicklook-content");

        *self.title_label.borrow_mut() = Some(title_label);
        *self.open_btn.borrow_mut() = Some(open_btn);
        *self.expand_btn.borrow_mut() = Some(expand_btn);
        *self.rotate_btn.borrow_mut() = Some(rotate_btn);
        *self.content_holder.borrow_mut() = Some(content_holder.clone());
        *self.current_path.borrow_mut() = Some(item.path.clone());

        let ql_weak = Rc::downgrade(self);
        let on_sizing_ready: Rc<dyn Fn(QuickLookContentSizing)> = Rc::new(move |sizing| {
            if let Some(ql) = ql_weak.upgrade() {
                ql.content_sizing.set(sizing);
                ql.apply_card_geometry();
            }
        });
        populate_preview_async(
            item,
            &content_holder,
            &self.media_handles,
            &self.page_step_cb,
            &self.preview_gen,
            &self.cache,
            &self.image_slot,
            on_sizing_ready,
            185,
        );
        inner_box.append(&content_holder);

        root_box
    }
}

/// Intenta delegar la vista previa al servicio dedicado `dev.malus.Preview` (ADR 0007).
fn try_external_preview_dbus(path: &Path, items: &[FileItem]) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let paths: Vec<String> = items
        .iter()
        .map(|it| it.path.to_string_lossy().into_owned())
        .collect();
    let target = path.to_string_lossy().into_owned();
    let args = (target, paths).to_variant();
    conn.call_sync(
        Some("dev.malus.Preview"),
        "/dev/malus/Preview",
        "dev.malus.Preview",
        "TogglePreview",
        Some(&args),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        150,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

pub fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "svg"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "avif"
            | "heic"
            | "jxl"
    )
}

pub fn is_video_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp4"
            | "mkv"
            | "webm"
            | "mov"
            | "avi"
            | "m4v"
            | "wmv"
            | "flv"
            | "ts"
            | "mpg"
            | "mpeg"
            | "3gp"
            | "ogv"
    )
}

pub fn is_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "aac" | "opus" | "wma"
    )
}

fn is_archive_extension(ext: &str) -> bool {
    matches!(ext, "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z")
}

fn is_text_preview_candidate(ext: &str, kind_str: &str) -> bool {
    [
        "rs", "toml", "json", "md", "txt", "sh", "c", "cpp", "h", "py", "js", "ts", "html", "css",
        "yaml", "yml", "xml", "ini", "conf", "log",
    ]
    .iter()
    .any(|e| *e == ext)
        || kind_str.to_lowercase().contains("texto")
        || kind_str.to_lowercase().contains("código")
}

fn needs_background_preview(ext: &str, kind_str: &str) -> bool {
    is_image_extension(ext)
        || ext == "pdf"
        || is_video_extension(ext)
        || is_audio_extension(ext)
        || is_archive_extension(ext)
        || is_text_preview_candidate(ext, kind_str)
}

#[derive(Clone)]
struct DecodedRgbaFrame {
    width: i32,
    height: i32,
    stride: usize,
    bytes: glib::Bytes,
}

impl DecodedRgbaFrame {
    fn to_texture(&self) -> Texture {
        MemoryTexture::new(
            self.width,
            self.height,
            MemoryFormat::R8g8b8a8,
            &self.bytes,
            self.stride,
        )
        .upcast()
    }

    fn to_squircle_texture(&self, display_w: i32, display_h: i32, logical_radius: f64) -> Texture {
        if logical_radius <= 0.5 || self.width <= 0 || self.height <= 0 {
            return self.to_texture();
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let expected_stride = w * 4;
        if self.stride != expected_stride || self.bytes.len() < expected_stride * h {
            return self.to_texture();
        }
        let sx = self.width as f64 / display_w.max(1) as f64;
        let sy = self.height as f64 / display_h.max(1) as f64;
        let r_px = (logical_radius * sx.max(sy)) as f32;
        let mut rgba = self.bytes.as_ref()[..expected_stride * h].to_vec();
        apply_squircle_clip_rgba_corners(&mut rgba, w, h, r_px);
        let bytes = glib::Bytes::from_owned(rgba);
        MemoryTexture::new(
            self.width,
            self.height,
            MemoryFormat::R8g8b8a8,
            &bytes,
            expected_stride,
        )
        .upcast()
    }
}

#[derive(Clone)]
enum PreparedPreview {
    Image {
        frame: Option<DecodedRgbaFrame>,
    },
    Pdf {
        total_pages: u32,
        first_page: Option<DecodedRgbaFrame>,
    },
    Video {
        meta: MediaMetadata,
        poster: Option<DecodedRgbaFrame>,
    },
    Audio {
        meta: MediaMetadata,
    },
    Text(String),
    Generic,
}

type CacheKey = (PathBuf, u64);

struct PreviewCache {
    max_entries: usize,
    entries: VecDeque<(CacheKey, PreparedPreview)>,
    in_flight: HashSet<CacheKey>,
}

impl PreviewCache {
    fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            entries: VecDeque::new(),
            in_flight: HashSet::new(),
        }
    }

    fn get_if_fresh(&mut self, key: &CacheKey) -> Option<PreparedPreview> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        let (k, val) = self.entries.remove(pos)?;
        let out = val.clone();
        self.entries.push_back((k, val));
        Some(out)
    }

    fn insert(&mut self, key: CacheKey, payload: PreparedPreview) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos);
        }
        while self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back((key, payload));
    }

    fn try_begin_fetch(&mut self, key: CacheKey) -> bool {
        if self.entries.iter().any(|(k, _)| k == &key) || self.in_flight.contains(&key) {
            return false;
        }
        self.in_flight.insert(key);
        true
    }

    fn finish_fetch(&mut self, key: CacheKey, payload: PreparedPreview) {
        self.in_flight.remove(&key);
        self.insert(key, payload);
    }
}

fn file_mtime_nanos(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn spawn_prefetch_into_cache(
    cache: &Arc<Mutex<PreviewCache>>,
    path: PathBuf,
    ext: String,
    kind_str: String,
) {
    let key = (path.clone(), file_mtime_nanos(&path));
    {
        let Ok(mut guard) = cache.lock() else {
            return;
        };
        if !guard.try_begin_fetch(key.clone()) {
            return;
        }
    }
    let cache_clone = cache.clone();
    std::thread::spawn(move || {
        let payload = prepare_preview_payload(&path, &ext, &kind_str);
        if let Ok(mut guard) = cache_clone.lock() {
            guard.finish_fetch(key, payload);
        }
    });
}

fn create_loading_placeholder(item: &FileItem) -> gtk4::Widget {
    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_vexpand(true);
    vbox.set_hexpand(true);

    let icon = crate::views::create_folder_aware_icon(item, 72);
    vbox.append(&icon);

    let spinner = Spinner::new();
    spinner.set_size_request(24, 24);
    spinner.start();
    vbox.append(&spinner);

    vbox.upcast()
}

fn populate_preview_async(
    item: &FileItem,
    holder: &GtkBox,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
    preview_gen: &Rc<Cell<u64>>,
    cache: &Arc<Mutex<PreviewCache>>,
    image_slot: &Rc<RefCell<Option<ImageViewSlot>>>,
    on_sizing_ready: Rc<dyn Fn(QuickLookContentSizing)>,
    delay_playback_ms: u64,
) {
    let next_gen = preview_gen.get().wrapping_add(1);
    preview_gen.set(next_gen);

    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }

    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if !needs_background_preview(&ext, &item.kind_str) {
        holder.append(&create_generic_preview(item));
        on_sizing_ready(QuickLookContentSizing::Compact);
        return;
    }

    let key = (item.path.clone(), file_mtime_nanos(&item.path));

    // Ruta rápida macOS (0 ms): si ya estaba precargado/cacheado en LRU, montarlo antes del Frame 0
    if let Ok(mut guard) = cache.lock() {
        if let Some(cached) = guard.get_if_fresh(&key) {
            let widget = build_widget_from_payload(
                item,
                cached,
                media_handles,
                page_step_cb,
                preview_gen,
                next_gen,
                image_slot,
                &on_sizing_ready,
                delay_playback_ms,
            );
            holder.append(&widget);
            return;
        }
    }

    holder.append(&create_loading_placeholder(item));
    spawn_prefetch_into_cache(cache, item.path.clone(), ext, item.kind_str.clone());

    let opened_at = std::time::Instant::now();
    let holder_weak = holder.downgrade();
    let item_clone = item.clone();
    let mh = media_handles.clone();
    let psc = page_step_cb.clone();
    let gen_ref = preview_gen.clone();
    let cache_poll = cache.clone();
    let slot_poll = image_slot.clone();

    glib::timeout_add_local(Duration::from_millis(8), move || {
        if gen_ref.get() != next_gen {
            return glib::ControlFlow::Break;
        }
        let cached_opt = cache_poll
            .lock()
            .ok()
            .and_then(|mut guard| guard.get_if_fresh(&key));
        if let Some(payload) = cached_opt {
            if let Some(h) = holder_weak.upgrade() {
                if gen_ref.get() == next_gen {
                    let elapsed_ms = opened_at.elapsed().as_millis() as u64;
                    let remaining_delay = delay_playback_ms.saturating_sub(elapsed_ms);
                    let widget = build_widget_from_payload(
                        &item_clone,
                        payload,
                        &mh,
                        &psc,
                        &gen_ref,
                        next_gen,
                        &slot_poll,
                        &on_sizing_ready,
                        remaining_delay,
                    );
                    while let Some(child) = h.first_child() {
                        h.remove(&child);
                    }
                    h.append(&widget);
                }
            }
            return glib::ControlFlow::Break;
        }
        let still_fetching = cache_poll
            .lock()
            .ok()
            .is_some_and(|guard| guard.in_flight.contains(&key));
        if still_fetching {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

fn prepare_preview_payload(path: &Path, ext: &str, kind_str: &str) -> PreparedPreview {
    if is_image_extension(ext) {
        return PreparedPreview::Image {
            frame: decode_image_rgba_offthread(path, ext),
        };
    }
    if ext == "pdf" {
        let total_pages = query_pdf_page_count(path);
        let first_page = render_pdf_page_rgba_offthread(path, 1);
        return PreparedPreview::Pdf {
            total_pages,
            first_page,
        };
    }
    if is_video_extension(ext) {
        let meta = probe_media_metadata(path);
        let poster = extract_video_poster_rgba_offthread(path, meta.duration_secs);
        return PreparedPreview::Video { meta, poster };
    }
    if is_audio_extension(ext) {
        let meta = probe_media_metadata(path);
        return PreparedPreview::Audio { meta };
    }
    if is_archive_extension(ext) {
        if let Some(text) = read_archive_listing_text(path, ext) {
            return PreparedPreview::Text(text);
        }
    }
    if is_text_preview_candidate(ext, kind_str) {
        return PreparedPreview::Text(read_text_preview_string(path));
    }
    PreparedPreview::Generic
}

fn build_widget_from_payload(
    item: &FileItem,
    payload: PreparedPreview,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
    preview_gen: &Rc<Cell<u64>>,
    expected_gen: u64,
    image_slot: &Rc<RefCell<Option<ImageViewSlot>>>,
    on_sizing_ready: &Rc<dyn Fn(QuickLookContentSizing)>,
    remaining_delay_ms: u64,
) -> gtk4::Widget {
    match payload {
        PreparedPreview::Image { frame } => {
            create_image_preview_from_frame(&item.path, item, frame, image_slot, on_sizing_ready)
        }
        PreparedPreview::Pdf {
            total_pages,
            first_page,
        } => create_pdf_preview_from_frame(
            &item.path,
            total_pages,
            first_page,
            page_step_cb,
            on_sizing_ready,
        )
        .unwrap_or_else(|| {
            on_sizing_ready(QuickLookContentSizing::Compact);
            create_generic_preview(item)
        }),
        PreparedPreview::Video { meta, poster } => create_video_preview_from_payload(
            item,
            meta,
            poster,
            media_handles,
            preview_gen,
            expected_gen,
            on_sizing_ready,
            remaining_delay_ms,
        ),
        PreparedPreview::Audio { meta } => {
            on_sizing_ready(QuickLookContentSizing::Compact);
            create_audio_preview_from_payload(
                item,
                meta,
                media_handles,
                preview_gen,
                expected_gen,
                remaining_delay_ms,
            )
        }
        PreparedPreview::Text(content) => {
            on_sizing_ready(QuickLookContentSizing::Document);
            create_text_widget_from_string(&content)
        }
        PreparedPreview::Generic => {
            on_sizing_ready(QuickLookContentSizing::Compact);
            create_generic_preview(item)
        }
    }
}

fn unique_temp_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "malus_ql_{}_{}_{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext
    ))
}

fn decode_pixbuf_to_rgba_frame(pixbuf: &Pixbuf) -> Option<DecodedRgbaFrame> {
    let oriented = pixbuf.apply_embedded_orientation();
    let pb = oriented.as_ref().unwrap_or(pixbuf);
    let width = pb.width();
    let height = pb.height();
    let rowstride = pb.rowstride();
    if width <= 0 || height <= 0 || rowstride <= 0 {
        return None;
    }
    let has_alpha = pb.has_alpha();
    let src_bytes = pb.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let w = width as usize;
    let h = height as usize;
    let stride = rowstride as usize;
    let out_stride = w * 4;
    let mut rgba = vec![0u8; out_stride * h];

    if has_alpha {
        for y in 0..h {
            let s0 = y * stride;
            let d0 = y * out_stride;
            if s0 + out_stride > src.len() {
                return None;
            }
            rgba[d0..d0 + out_stride].copy_from_slice(&src[s0..s0 + out_stride]);
        }
    } else {
        for y in 0..h {
            let s0 = y * stride;
            let d0 = y * out_stride;
            if s0 + w * 3 > src.len() {
                return None;
            }
            for x in 0..w {
                let sp = s0 + x * 3;
                let dp = d0 + x * 4;
                rgba[dp] = src[sp];
                rgba[dp + 1] = src[sp + 1];
                rgba[dp + 2] = src[sp + 2];
                rgba[dp + 3] = 255;
            }
        }
    }

    Some(DecodedRgbaFrame {
        width,
        height,
        stride: out_stride,
        bytes: glib::Bytes::from_owned(rgba),
    })
}

/// Carga un `Pixbuf` reduciendo imágenes mayores a `(max_w, max_h)` pero SIN estirar nunca
/// imágenes pequeñas (a diferencia de `Pixbuf::from_file_at_scale`, que siempre escala al tamaño destino).
pub fn load_pixbuf_no_upscale(path: &Path, max_w: i32, max_h: i32) -> Option<Pixbuf> {
    if let Some((_, w, h)) = Pixbuf::file_info(path) {
        if w > 0 && h > 0 {
            if w <= max_w && h <= max_h {
                return Pixbuf::from_file(path).ok();
            }
            return Pixbuf::from_file_at_scale(path, max_w, max_h, true).ok();
        }
    }
    let pb = Pixbuf::from_file(path).ok()?;
    let w = pb.width();
    let h = pb.height();
    if w > max_w || h > max_h {
        let scale = (max_w as f64 / w.max(1) as f64).min(max_h as f64 / h.max(1) as f64);
        let tw = ((w as f64 * scale).round() as i32).max(1);
        let th = ((h as f64 * scale).round() as i32).max(1);
        pb.scale_simple(tw, th, gdk_pixbuf::InterpType::Bilinear)
            .or(Some(pb))
    } else {
        Some(pb)
    }
}

fn decode_image_rgba_offthread(path: &Path, ext: &str) -> Option<DecodedRgbaFrame> {
    if !matches!(ext, "heic" | "avif" | "jxl") {
        if let Some(pb) = load_pixbuf_no_upscale(path, QL_MAX_DECODE_W, QL_MAX_DECODE_H) {
            if let Some(frame) = decode_pixbuf_to_rgba_frame(&pb) {
                return Some(frame);
            }
        }
    }

    let tmp_png = unique_temp_path("img", "png");
    let scale_vf = format!(
        "scale='min({QL_MAX_DECODE_W},iw)':'min({QL_MAX_DECODE_H},ih)':force_original_aspect_ratio=decrease"
    );
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(safe_ffmpeg_input_arg(path))
        .args(["-frames:v", "1", "-vf", &scale_vf])
        .arg(&tmp_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let frame = if ok && tmp_png.exists() {
        load_pixbuf_no_upscale(&tmp_png, QL_MAX_DECODE_W, QL_MAX_DECODE_H)
            .and_then(|pb| decode_pixbuf_to_rgba_frame(&pb))
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    frame
}

fn create_image_preview_from_frame(
    path: &Path,
    item: &FileItem,
    frame_opt: Option<DecodedRgbaFrame>,
    image_slot: &Rc<RefCell<Option<ImageViewSlot>>>,
    on_sizing_ready: &Rc<dyn Fn(QuickLookContentSizing)>,
) -> gtk4::Widget {
    let (texture, dims, stored_frame) = if let Some(frame) = frame_opt {
        let dims = (frame.width, frame.height);
        let tex = frame.to_texture();
        (Some(tex), Some(dims), Some(frame))
    } else if let Ok(tex) = Texture::from_file(&gio::File::for_path(path)) {
        let dims = (tex.width(), tex.height());
        (Some(tex), Some(dims), None)
    } else {
        (None, None, None)
    };

    let Some(texture) = texture else {
        on_sizing_ready(QuickLookContentSizing::Compact);
        return create_generic_preview(item);
    };

    let stage = GtkBox::new(Orientation::Vertical, 0);
    stage.set_vexpand(true);
    stage.set_hexpand(true);
    stage.add_css_class("branches-quicklook-image-stage");

    let size_spacer = GtkBox::new(Orientation::Vertical, 0);
    size_spacer.set_halign(Align::Fill);
    size_spacer.set_valign(Align::Fill);

    let img_frame = Overlay::new();
    img_frame.set_halign(Align::Center);
    img_frame.set_valign(Align::Center);
    img_frame.set_vexpand(true);
    img_frame.set_hexpand(true);
    img_frame.set_overflow(Overflow::Hidden);
    img_frame.add_css_class("branches-quicklook-image-frame");
    img_frame.set_child(Some(&size_spacer));

    let picture = Picture::for_paintable(&texture);
    picture.set_content_fit(ContentFit::ScaleDown);
    picture.set_can_shrink(true);
    picture.set_vexpand(true);
    picture.set_hexpand(true);
    picture.add_css_class("branches-quicklook-image");
    img_frame.add_overlay(&picture);
    img_frame.set_measure_overlay(&picture, false);
    let live_text = dims.and_then(|(w, h)| {
        if w > 0 && h > 0 {
            Some(malus_de_common::live_text::LiveTextController::attach(
                &img_frame, path, w, h,
            ))
        } else {
            None
        }
    });
    stage.append(&img_frame);

    *image_slot.borrow_mut() = Some(ImageViewSlot {
        frame: Rc::new(RefCell::new(stored_frame)),
        picture,
        img_frame,
        size_spacer,
        live_text,
    });

    if let Some((w, h)) = dims {
        if w > 0 && h > 0 {
            on_sizing_ready(QuickLookContentSizing::Image {
                width: w,
                height: h,
            });
        }
    }

    stage.upcast()
}

pub fn parse_pdfinfo_page_count(stdout: &str) -> u32 {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return n.max(1);
            }
        }
    }
    1
}

/// Normaliza una ruta local para herramientas CLI (evitando que un nombre que empiece por `-` sea tratado como opción).
fn safe_cli_path_arg(path: &Path) -> std::ffi::OsString {
    if path.is_absolute() {
        path.as_os_str().to_os_string()
    } else {
        let mut s = std::ffi::OsString::from("./");
        s.push(path.as_os_str());
        s
    }
}

/// Prefija `file:` para FFmpeg/FFprobe/FFplay evitando que nombres con `:` sean interpretados como protocolos de red.
fn safe_ffmpeg_input_arg(path: &Path) -> std::ffi::OsString {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut s = std::ffi::OsString::from("file:");
    s.push(abs.as_os_str());
    s
}

fn query_pdf_page_count(path: &Path) -> u32 {
    let Ok(out) = Command::new("pdfinfo")
        .arg("--")
        .arg(safe_cli_path_arg(path))
        .output()
    else {
        return 1;
    };
    if !out.status.success() {
        return 1;
    }
    parse_pdfinfo_page_count(&String::from_utf8_lossy(&out.stdout))
}

fn render_pdf_page_rgba_offthread(path: &Path, page: u32) -> Option<DecodedRgbaFrame> {
    let prefix = unique_temp_path("pdf", "tmp").with_extension("");
    let page_str = page.to_string();
    let status = Command::new("pdftoppm")
        .args([
            "-png",
            "-f",
            &page_str,
            "-l",
            &page_str,
            "-singlefile",
            "-scale-to",
            "1100",
        ])
        .arg(safe_cli_path_arg(path))
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let png_path = PathBuf::from(format!("{}.png", prefix.display()));
    let frame = if png_path.exists() {
        Pixbuf::from_file(&png_path)
            .ok()
            .and_then(|pb| decode_pixbuf_to_rgba_frame(&pb))
    } else {
        None
    };
    let _ = fs::remove_file(&png_path);
    frame
}

fn create_pdf_preview_from_frame(
    path: &Path,
    total_pages: u32,
    first_page: Option<DecodedRgbaFrame>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
    on_sizing_ready: &Rc<dyn Fn(QuickLookContentSizing)>,
) -> Option<gtk4::Widget> {
    let page_frame = first_page?;
    let page_w = page_frame.width;
    let page_h = page_frame.height;
    let initial_tex = page_frame.to_texture();
    if page_w > 0 && page_h > 0 {
        on_sizing_ready(QuickLookContentSizing::Pdf {
            width: page_w,
            height: page_h,
            has_footer: total_pages > 1,
        });
    }

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.set_hexpand(true);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();

    let pic_box = GtkBox::new(Orientation::Vertical, 0);
    pic_box.set_margin_top(12);
    pic_box.set_margin_bottom(12);
    pic_box.set_margin_start(14);
    pic_box.set_margin_end(14);
    pic_box.set_vexpand(true);
    pic_box.set_hexpand(true);

    let picture = Picture::for_paintable(&initial_tex);
    picture.set_content_fit(ContentFit::ScaleDown);
    picture.set_can_shrink(true);
    picture.set_vexpand(true);
    picture.set_hexpand(true);
    picture.add_css_class("branches-quicklook-image");
    pic_box.append(&picture);

    scrolled.set_child(Some(&pic_box));
    root.append(&scrolled);

    if total_pages > 1 {
        let footer = GtkBox::new(Orientation::Horizontal, 10);
        footer.add_css_class("branches-quicklook-footer");
        footer.set_halign(Align::Center);
        footer.set_margin_top(8);
        footer.set_margin_bottom(10);

        let current_page = Rc::new(Cell::new(1u32));
        let page_gen = Rc::new(Cell::new(0u64));
        let prev_btn = Button::builder()
            .icon_name("go-previous-symbolic")
            .has_frame(false)
            .can_focus(false)
            .sensitive(false)
            .tooltip_text("Página anterior (Re Pág)")
            .build();
        let page_lbl = Label::builder()
            .label(&format!("Página 1 de {}", total_pages))
            .css_classes(["caption", "bold"])
            .build();
        let next_btn = Button::builder()
            .icon_name("go-next-symbolic")
            .has_frame(false)
            .can_focus(false)
            .tooltip_text("Página siguiente (Av Pág)")
            .build();

        let step_page: PageStepCallback = Rc::new({
            let path = path.to_path_buf();
            let current_page = current_page.clone();
            let page_gen = page_gen.clone();
            let picture = picture.clone();
            let page_lbl = page_lbl.clone();
            let prev_btn = prev_btn.clone();
            let next_btn = next_btn.clone();
            move |delta: i32| -> bool {
                let curr = current_page.get() as i32;
                let target = (curr + delta).clamp(1, total_pages as i32) as u32;
                if target == current_page.get() {
                    return false;
                }
                current_page.set(target);
                page_lbl.set_label(&format!("Página {} de {}", target, total_pages));
                prev_btn.set_sensitive(target > 1);
                next_btn.set_sensitive(target < total_pages);

                let gen = page_gen.get().wrapping_add(1);
                page_gen.set(gen);
                let (tx, rx) = mpsc::channel::<Option<DecodedRgbaFrame>>();
                let p = path.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(render_pdf_page_rgba_offthread(&p, target));
                });
                let pic_weak = picture.downgrade();
                let pg_ref = page_gen.clone();
                glib::timeout_add_local(Duration::from_millis(8), move || {
                    if pg_ref.get() != gen {
                        return glib::ControlFlow::Break;
                    }
                    match rx.try_recv() {
                        Ok(Some(frame)) => {
                            if let Some(pic) = pic_weak.upgrade() {
                                let tex = frame.to_texture();
                                pic.set_paintable(Some(&tex));
                            }
                            glib::ControlFlow::Break
                        }
                        Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    }
                });
                true
            }
        });

        let sp1 = step_page.clone();
        prev_btn.connect_clicked(move |_| {
            sp1(-1);
        });
        let sp2 = step_page.clone();
        next_btn.connect_clicked(move |_| {
            sp2(1);
        });
        *page_step_cb.borrow_mut() = Some(step_page);

        footer.append(&prev_btn);
        footer.append(&page_lbl);
        footer.append(&next_btn);
        root.append(&footer);
    }

    Some(root.upcast())
}

#[derive(Clone, Default)]
struct MediaMetadata {
    duration_secs: Option<f64>,
    duration_str: Option<String>,
    codec: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    has_audio: bool,
}

fn probe_media_metadata(path: &Path) -> MediaMetadata {
    let mut meta = MediaMetadata::default();
    let Ok(output) = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_name,codec_type,width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(safe_ffmpeg_input_arg(path))
        .output()
    else {
        return meta;
    };
    if !output.status.success() {
        return meta;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let val = v.trim();
        match k.trim() {
            "duration" => {
                if let Ok(secs) = val.parse::<f64>() {
                    meta.duration_secs = Some(secs);
                    let mins = (secs / 60.0).floor() as u64;
                    let rem = (secs % 60.0).round() as u64;
                    meta.duration_str = Some(format!("{:02}:{:02}", mins, rem));
                }
            }
            "codec_type" if val == "audio" => {
                meta.has_audio = true;
            }
            "codec_name" if meta.codec.is_none() => {
                meta.codec = Some(val.to_string());
            }
            "width" if meta.width.is_none() => {
                if let Ok(w) = val.parse::<u32>() {
                    if w > 0 {
                        meta.width = Some(w);
                    }
                }
            }
            "height" if meta.height.is_none() => {
                if let Ok(h) = val.parse::<u32>() {
                    if h > 0 {
                        meta.height = Some(h);
                    }
                }
            }
            _ => {}
        }
    }
    meta
}

/// Ajusta las dimensiones del frame de video dentro de un máximo manteniendo relación de aspecto
/// y asegurando ancho/alto pares para alineación RGBA limpia.
pub fn fit_video_dimensions(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let sw = src_w.max(2) as f64;
    let sh = src_h.max(2) as f64;
    let scale = (max_w as f64 / sw).min(max_h as f64 / sh).min(1.0);
    let mut w = ((sw * scale).round() as u32).max(2);
    let mut h = ((sh * scale).round() as u32).max(2);
    if w % 2 != 0 {
        w += 1;
    }
    if h % 2 != 0 {
        h += 1;
    }
    (w, h)
}

fn extract_video_poster_rgba_offthread(
    path: &Path,
    duration_secs: Option<f64>,
) -> Option<DecodedRgbaFrame> {
    let seek_arg = match duration_secs {
        Some(d) if d > 2.0 => "0.5",
        _ => "0",
    };
    let tmp_png = unique_temp_path("vposter", "png");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-ss", seek_arg, "-i"])
        .arg(safe_ffmpeg_input_arg(path))
        .args(["-frames:v", "1", "-vf", "scale='min(760,iw)':-1"])
        .arg(&tmp_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let frame = if ok && tmp_png.exists() {
        Pixbuf::from_file(&tmp_png)
            .ok()
            .and_then(|pb| decode_pixbuf_to_rgba_frame(&pb))
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    frame
}

fn spawn_video_playback(
    path: &Path,
    width: u32,
    height: u32,
    play_audio: bool,
    picture: &Picture,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    on_finished: Rc<dyn Fn()>,
) -> Rc<MediaPlaybackHandle> {
    let handle = Rc::new(MediaPlaybackHandle::new());
    media_handles.borrow_mut().push(handle.clone());

    let scale_filter = format!("scale={}:{}:flags=fast_bilinear,fps=24", width, height);
    let child_video = Command::new("ffmpeg")
        .args(["-re", "-loglevel", "error", "-i"])
        .arg(safe_ffmpeg_input_arg(path))
        .args([
            "-vf",
            &scale_filter,
            "-an",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    if play_audio {
        if let Ok(audio_child) = Command::new("ffplay")
            .args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-i"])
            .arg(safe_ffmpeg_input_arg(path))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            handle.register_child(audio_child);
        }
    }

    let Ok(mut child) = child_video else {
        on_finished();
        return handle;
    };

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        on_finished();
        return handle;
    };
    handle.register_child(child);

    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(2);
    let stop_flag = handle.stop_flag.clone();
    let frame_bytes = (width as usize) * (height as usize) * 4;

    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::SeqCst) {
            let mut buf = vec![0u8; frame_bytes];
            if stdout.read_exact(&mut buf).is_err() {
                break;
            }
            match tx.try_send(buf) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        }
    });

    let stop_ui = handle.stop_flag.clone();
    let pic_weak = picture.downgrade();
    let stride = (width as usize) * 4;
    let w_i32 = width as i32;
    let h_i32 = height as i32;
    let video_r = if width >= 160 && height >= 120 {
        quicklook_concentric_inner_radius(quicklook_card_squircle_radius(), 14.0) as f32
    } else {
        0.0
    };
    let corner_mask = precompute_squircle_corner_alpha(width as usize, height as usize, video_r);

    glib::timeout_add_local(Duration::from_millis(20), move || {
        if stop_ui.load(Ordering::SeqCst) {
            return glib::ControlFlow::Break;
        }
        let Some(pic) = pic_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let mut latest_frame = None;
        loop {
            match rx.try_recv() {
                Ok(frame) => latest_frame = Some(frame),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(mut frame) = latest_frame {
                        for &(alpha_idx, alpha_val) in &corner_mask {
                            if alpha_idx < frame.len() {
                                frame[alpha_idx] = alpha_val;
                            }
                        }
                        let bytes = glib::Bytes::from_owned(frame);
                        let tex = MemoryTexture::new(
                            w_i32,
                            h_i32,
                            MemoryFormat::R8g8b8a8,
                            &bytes,
                            stride,
                        );
                        pic.set_paintable(Some(&tex));
                    }
                    on_finished();
                    return glib::ControlFlow::Break;
                }
            }
        }

        if let Some(mut frame) = latest_frame {
            for &(alpha_idx, alpha_val) in &corner_mask {
                if alpha_idx < frame.len() {
                    frame[alpha_idx] = alpha_val;
                }
            }
            let bytes = glib::Bytes::from_owned(frame);
            let tex = MemoryTexture::new(w_i32, h_i32, MemoryFormat::R8g8b8a8, &bytes, stride);
            pic.set_paintable(Some(&tex));
        }

        glib::ControlFlow::Continue
    });

    handle
}

fn create_video_preview_from_payload(
    item: &FileItem,
    meta: MediaMetadata,
    poster_frame: Option<DecodedRgbaFrame>,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    preview_gen: &Rc<Cell<u64>>,
    expected_gen: u64,
    on_sizing_ready: &Rc<dyn Fn(QuickLookContentSizing)>,
    remaining_delay_ms: u64,
) -> gtk4::Widget {
    let (play_w, play_h) = fit_video_dimensions(
        meta.width.unwrap_or(720),
        meta.height.unwrap_or(420),
        760,
        440,
    );
    on_sizing_ready(QuickLookContentSizing::Video {
        width: play_w as i32,
        height: play_h as i32,
    });

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.set_hexpand(true);

    let stage = GtkBox::new(Orientation::Vertical, 0);
    stage.set_vexpand(true);
    stage.set_hexpand(true);
    stage.set_margin_top(12);
    stage.set_margin_bottom(8);
    stage.set_margin_start(14);
    stage.set_margin_end(14);
    stage.add_css_class("branches-quicklook-video-stage");

    let picture = Picture::new();
    picture.set_content_fit(ContentFit::ScaleDown);
    picture.set_can_shrink(true);
    picture.set_vexpand(true);
    picture.set_hexpand(true);
    if let Some(frame) = poster_frame {
        let video_r = if play_w >= 160 && play_h >= 120 {
            quicklook_concentric_inner_radius(quicklook_card_squircle_radius(), 14.0)
        } else {
            0.0
        };
        let poster = if video_r > 0.5 {
            frame.to_squircle_texture(play_w as i32, play_h as i32, video_r)
        } else {
            frame.to_texture()
        };
        picture.set_paintable(Some(&poster));
    }
    stage.append(&picture);
    root.append(&stage);

    let footer = GtkBox::new(Orientation::Horizontal, 10);
    footer.add_css_class("branches-quicklook-footer");
    footer.set_margin_top(4);
    footer.set_margin_bottom(12);
    footer.set_margin_start(16);
    footer.set_margin_end(16);

    let is_playing = Rc::new(Cell::new(true));
    let is_muted = Rc::new(Cell::new(false));
    let current_handle: Rc<RefCell<Option<Rc<MediaPlaybackHandle>>>> = Rc::new(RefCell::new(None));

    let play_btn = Button::builder()
        .icon_name("media-playback-stop-symbolic")
        .label("Detener")
        .has_frame(true)
        .can_focus(false)
        .css_classes(["branches-quicklook-btn"])
        .build();

    let mute_btn = Button::builder()
        .icon_name("audio-volume-high-symbolic")
        .has_frame(false)
        .can_focus(false)
        .tooltip_text("Silenciar / Activar audio")
        .visible(meta.has_audio)
        .build();

    let mut info_parts = Vec::new();
    if let (Some(w), Some(h)) = (meta.width, meta.height) {
        info_parts.push(format!("{}×{}", w, h));
    }
    if let Some(ref d) = meta.duration_str {
        info_parts.push(d.clone());
    }
    if let Some(ref c) = meta.codec {
        info_parts.push(c.clone());
    }
    let meta_lbl = Label::builder()
        .label(&info_parts.join(" • "))
        .halign(Align::Start)
        .hexpand(true)
        .css_classes(["dim-label", "caption"])
        .build();

    footer.append(&play_btn);
    footer.append(&mute_btn);
    footer.append(&meta_lbl);
    root.append(&footer);

    let start_playback: Rc<dyn Fn()> = Rc::new({
        let path = item.path.clone();
        let picture = picture.clone();
        let media_handles = media_handles.clone();
        let is_playing = is_playing.clone();
        let is_muted = is_muted.clone();
        let current_handle = current_handle.clone();
        let play_btn = play_btn.clone();
        let has_audio = meta.has_audio;
        move || {
            if let Some(h) = current_handle.borrow_mut().take() {
                h.stop();
            }
            is_playing.set(true);
            play_btn.set_icon_name("media-playback-stop-symbolic");
            play_btn.set_label("Detener");

            let is_playing_end = is_playing.clone();
            let play_btn_end = play_btn.clone();
            let on_end: Rc<dyn Fn()> = Rc::new(move || {
                is_playing_end.set(false);
                play_btn_end.set_icon_name("media-playback-start-symbolic");
                play_btn_end.set_label("Reproducir");
            });

            let h = spawn_video_playback(
                &path,
                play_w,
                play_h,
                has_audio && !is_muted.get(),
                &picture,
                &media_handles,
                on_end,
            );
            *current_handle.borrow_mut() = Some(h);
        }
    });

    // Diferir arranque de ffmpeg/ffplay hasta completar la transición de apertura
    if remaining_delay_ms == 0 {
        start_playback();
    } else {
        let sp_delayed = start_playback.clone();
        let gen_ref = preview_gen.clone();
        glib::timeout_add_local_once(Duration::from_millis(remaining_delay_ms), move || {
            if gen_ref.get() == expected_gen {
                sp_delayed();
            }
        });
    }

    let sp_toggle = start_playback.clone();
    let ch_toggle = current_handle.clone();
    let ip_toggle = is_playing.clone();
    let pb_toggle = play_btn.clone();
    play_btn.connect_clicked(move |_| {
        if ip_toggle.get() {
            if let Some(h) = ch_toggle.borrow_mut().take() {
                h.stop();
            }
            ip_toggle.set(false);
            pb_toggle.set_icon_name("media-playback-start-symbolic");
            pb_toggle.set_label("Reproducir");
        } else {
            sp_toggle();
        }
    });

    let im_mute = is_muted.clone();
    let mb_mute = mute_btn.clone();
    let sp_mute = start_playback;
    let ip_mute = is_playing;
    mute_btn.connect_clicked(move |_| {
        let next_muted = !im_mute.get();
        im_mute.set(next_muted);
        mb_mute.set_icon_name(if next_muted {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
        if ip_mute.get() {
            sp_mute();
        }
    });

    root.upcast()
}

fn create_audio_preview_from_payload(
    item: &FileItem,
    meta: MediaMetadata,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    preview_gen: &Rc<Cell<u64>>,
    expected_gen: u64,
    remaining_delay_ms: u64,
) -> gtk4::Widget {
    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);

    let icon = crate::views::create_folder_aware_icon(item, 96);
    vbox.append(&icon);

    let name = Label::builder()
        .label(&item.name)
        .css_classes(["title-2", "bold"])
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .build();
    vbox.append(&name);

    let mut info = Vec::new();
    if let Some(ref d) = meta.duration_str {
        info.push(format!("Duración: {}", d));
    }
    if let Some(ref c) = meta.codec {
        info.push(format!("Códec: {}", c));
    }
    if !info.is_empty() {
        let lbl = Label::builder()
            .label(&info.join(" • "))
            .css_classes(["dim-label"])
            .build();
        vbox.append(&lbl);
    }

    let is_playing = Rc::new(Cell::new(true));
    let current_handle: Rc<RefCell<Option<Rc<MediaPlaybackHandle>>>> = Rc::new(RefCell::new(None));
    let toggle_btn = Button::builder()
        .label("Detener audio")
        .icon_name("media-playback-stop-symbolic")
        .can_focus(false)
        .css_classes(["suggested-action", "branches-quicklook-btn"])
        .margin_top(10)
        .halign(Align::Center)
        .build();
    vbox.append(&toggle_btn);

    let start_audio: Rc<dyn Fn()> = Rc::new({
        let path = item.path.clone();
        let media_handles = media_handles.clone();
        let is_playing = is_playing.clone();
        let current_handle = current_handle.clone();
        let toggle_btn = toggle_btn.clone();
        move || {
            if let Some(h) = current_handle.borrow_mut().take() {
                h.stop();
            }
            let h = Rc::new(MediaPlaybackHandle::new());
            if let Ok(child) = Command::new("ffplay")
                .args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-i"])
                .arg(safe_ffmpeg_input_arg(&path))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                h.register_child(child);
                media_handles.borrow_mut().push(h.clone());
                *current_handle.borrow_mut() = Some(h);
                is_playing.set(true);
                toggle_btn.set_label("Detener audio");
                toggle_btn.set_icon_name("media-playback-stop-symbolic");
            }
        }
    });

    if remaining_delay_ms == 0 {
        start_audio();
    } else {
        let sa_delayed = start_audio.clone();
        let gen_ref = preview_gen.clone();
        glib::timeout_add_local_once(Duration::from_millis(remaining_delay_ms), move || {
            if gen_ref.get() == expected_gen {
                sa_delayed();
            }
        });
    }

    let ip_btn = is_playing;
    let ch_btn = current_handle;
    let sa_btn = start_audio;
    let tb_btn = toggle_btn.clone();
    toggle_btn.connect_clicked(move |_| {
        if ip_btn.get() {
            if let Some(h) = ch_btn.borrow_mut().take() {
                h.stop();
            }
            ip_btn.set(false);
            tb_btn.set_label("Reproducir audio");
            tb_btn.set_icon_name("media-playback-start-symbolic");
        } else {
            sa_btn();
        }
    });

    vbox.upcast()
}

fn read_archive_listing_text(path: &Path, ext: &str) -> Option<String> {
    let safe_path = safe_cli_path_arg(path);
    let out = match ext {
        "zip" => Command::new("unzip")
            .args(["-Z1", "--"])
            .arg(&safe_path)
            .output()
            .ok()?,
        "tar" | "gz" | "tgz" | "bz2" | "xz" => Command::new("tar")
            .args(["--force-local", "-tf"])
            .arg(&safe_path)
            .output()
            .ok()?,
        "7z" => Command::new("7z")
            .args(["l", "-ba", "--"])
            .arg(&safe_path)
            .output()
            .ok()?,
        _ => return None,
    };
    if !out.status.success() {
        return None;
    }

    let lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .take(500)
        .map(ToString::to_string)
        .collect();
    if lines.is_empty() {
        Some("(Archivo comprimido vacío)".to_string())
    } else {
        Some(lines.join("\n"))
    }
}

fn read_text_preview_string(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => {
            let truncated = if bytes.len() > 150_000 {
                &bytes[..150_000]
            } else {
                &bytes[..]
            };
            String::from_utf8_lossy(truncated).to_string()
        }
        Err(_) => "(No se pudo leer el archivo de texto)".to_string(),
    }
}

fn create_text_widget_from_string(content: &str) -> gtk4::Widget {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();

    let text_view = TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .can_focus(false)
        .monospace(true)
        .left_margin(16)
        .right_margin(16)
        .top_margin(14)
        .bottom_margin(14)
        .css_classes(["branches-quicklook-text"])
        .build();

    text_view.buffer().set_text(content);
    scrolled.set_child(Some(&text_view));
    scrolled.upcast()
}

fn create_generic_preview(item: &FileItem) -> gtk4::Widget {
    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(32);
    vbox.set_margin_end(32);

    let icon = crate::views::create_folder_aware_icon(item, 128);
    vbox.append(&icon);

    let name = Label::builder()
        .label(&item.name)
        .css_classes(["title-2", "bold"])
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .justify(gtk4::Justification::Center)
        .build();
    vbox.append(&name);

    let subtitle_text = if item.is_dir {
        let count = fs::read_dir(&item.path)
            .map(|rd| rd.flatten().count())
            .unwrap_or(0);
        format!("{} • {} elementos", item.kind_str, count)
    } else {
        format!("{} • {}", item.kind_str, item.size_str)
    };
    let kind = Label::builder()
        .label(&subtitle_text)
        .css_classes(["dim-label"])
        .build();
    vbox.append(&kind);

    let sep = Separator::new(Orientation::Horizontal);
    sep.set_margin_top(8);
    sep.set_margin_bottom(8);
    vbox.append(&sep);

    let grid = Grid::builder()
        .row_spacing(6)
        .column_spacing(12)
        .halign(Align::Center)
        .build();

    let mut row = 0;
    let add_row = |g: &Grid, r: &mut i32, k: &str, v: &str| {
        let kl = Label::builder()
            .label(k)
            .halign(Align::End)
            .css_classes(["dim-label", "bold"])
            .build();
        let vl = Label::builder().label(v).halign(Align::Start).build();
        g.attach(&kl, 0, *r, 1, 1);
        g.attach(&vl, 1, *r, 1, 1);
        *r += 1;
    };

    add_row(&grid, &mut row, "Modificado:", &item.modified_str);
    add_row(&grid, &mut row, "Ubicación:", &item.path.to_string_lossy());

    vbox.append(&grid);

    let p_clone = item.path.clone();
    let btn = Button::builder()
        .label("Abrir archivo")
        .can_focus(false)
        .css_classes(["suggested-action", "branches-quicklook-btn"])
        .margin_top(16)
        .halign(Align::Center)
        .build();
    btn.connect_clicked(move |_| {
        let _ = open_file(&p_clone);
    });
    vbox.append(&btn);

    vbox.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_webp_dimensions_supports_vp8_vp8l_and_vp8x() {
        // VP8 lossy 500x333
        let mut vp8 = vec![0u8; 30];
        vp8[0..4].copy_from_slice(b"RIFF");
        vp8[8..12].copy_from_slice(b"WEBP");
        vp8[12..16].copy_from_slice(b"VP8 ");
        vp8[23] = 0x9d;
        vp8[24] = 0x01;
        vp8[25] = 0x2a;
        vp8[26..28].copy_from_slice(&500u16.to_le_bytes());
        vp8[28..30].copy_from_slice(&333u16.to_le_bytes());
        assert_eq!(parse_webp_dimensions_from_bytes(&vp8), Some((500, 333)));

        // VP8L lossless 640x480
        let mut vp8l = vec![0u8; 30];
        vp8l[0..4].copy_from_slice(b"RIFF");
        vp8l[8..12].copy_from_slice(b"WEBP");
        vp8l[12..16].copy_from_slice(b"VP8L");
        vp8l[20] = 0x2f;
        let bits = (640u32 - 1) | ((480u32 - 1) << 14);
        vp8l[21..25].copy_from_slice(&bits.to_le_bytes());
        assert_eq!(parse_webp_dimensions_from_bytes(&vp8l), Some((640, 480)));

        // VP8X extended 1200x800
        let mut vp8x = vec![0u8; 30];
        vp8x[0..4].copy_from_slice(b"RIFF");
        vp8x[8..12].copy_from_slice(b"WEBP");
        vp8x[12..16].copy_from_slice(b"VP8X");
        let w = 1200u32 - 1;
        let h = 800u32 - 1;
        vp8x[24..27].copy_from_slice(&w.to_le_bytes()[0..3]);
        vp8x[27..30].copy_from_slice(&h.to_le_bytes()[0..3]);
        assert_eq!(parse_webp_dimensions_from_bytes(&vp8x), Some((1200, 800)));
    }

    #[test]
    fn quick_look_extension_classifiers_cover_images_videos_and_audio() {
        assert!(is_image_extension("png"));
        assert!(is_image_extension("webp"));
        assert!(is_image_extension("avif"));
        assert!(is_video_extension("mp4"));
        assert!(is_video_extension("mkv"));
        assert!(is_video_extension("webm"));
        assert!(is_audio_extension("mp3"));
        assert!(is_audio_extension("flac"));
        assert!(!is_image_extension("pdf"));
        assert!(!is_video_extension("pdf"));
    }

    #[test]
    fn parse_pdfinfo_page_count_extracts_total_pages_with_safe_default() {
        let sample = "Title: Demo\nAuthor: Test\nPages:          14\nEncrypted: no\n";
        assert_eq!(parse_pdfinfo_page_count(sample), 14);
        assert_eq!(parse_pdfinfo_page_count("no pages header"), 1);
    }

    #[test]
    fn fit_video_dimensions_keeps_aspect_ratio_and_even_stride() {
        let (w, h) = fit_video_dimensions(1920, 1080, 760, 440);
        assert!(w <= 760 && h <= 440);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert_eq!((w, h), (760, 428));
    }

    #[test]
    fn preview_cache_respects_lru_eviction_and_mtime_invalidation() {
        let mut cache = PreviewCache::new(2);
        let k1 = (PathBuf::from("/tmp/a.png"), 100);
        let k2 = (PathBuf::from("/tmp/b.png"), 200);
        let k3 = (PathBuf::from("/tmp/c.png"), 300);

        cache.insert(k1.clone(), PreparedPreview::Text("a".into()));
        cache.insert(k2.clone(), PreparedPreview::Text("b".into()));
        // Re-acceder k1 lo promueve a MRU; k2 pasa a ser LRU
        assert!(cache.get_if_fresh(&k1).is_some());
        cache.insert(k3.clone(), PreparedPreview::Text("c".into()));

        assert!(cache.get_if_fresh(&k1).is_some());
        assert!(cache.get_if_fresh(&k2).is_none());
        assert!(cache.get_if_fresh(&k3).is_some());
        // Cambio de mtime invalida la clave
        assert!(cache
            .get_if_fresh(&(PathBuf::from("/tmp/a.png"), 101))
            .is_none());
    }

    #[test]
    fn decode_pixbuf_to_rgba_frame_produces_aligned_rgba_bytes() {
        let pb = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 5, 3).unwrap();
        let frame = decode_pixbuf_to_rgba_frame(&pb).unwrap();
        assert_eq!(frame.width, 5);
        assert_eq!(frame.height, 3);
        assert_eq!(frame.stride, 20);
        assert_eq!(frame.bytes.len(), 60);

        let rot = rotate_rgba_frame_ccw(&frame);
        assert_eq!(rot.width, 3);
        assert_eq!(rot.height, 5);
        assert_eq!(rot.stride, 12);
        assert_eq!(rot.bytes.len(), 60);
    }

    #[test]
    fn fit_quicklook_card_size_adapts_to_landscape_square_and_portrait() {
        // Landscape
        let (cw, ch, fw, fh) = fit_quicklook_card_size(1200, 750, 900, 700);
        assert!(cw > ch);
        assert_eq!(cw, fw + QL_IMAGE_INSET * 2);
        assert_eq!(ch, fh + QL_HEADER_HEIGHT + QL_IMAGE_INSET);

        // Square compact
        let (sq_w, sq_h, _, _) = fit_quicklook_card_size(420, 420, 900, 700);
        assert_eq!(sq_w, 420 + QL_IMAGE_INSET * 2);
        assert_eq!(sq_h, 420 + QL_HEADER_HEIGHT + QL_IMAGE_INSET);

        // Tall portrait
        let (pw, ph, _, _) = fit_quicklook_card_size(520, 900, 900, 720);
        assert!(ph > pw);
        assert!(ph <= 720);

        // Small image: NEVER stretched and card enforces minimum size
        let (sw, sh, sfw, sfh) = fit_quicklook_card_size(64, 64, 900, 700);
        assert_eq!((sfw, sfh), (64, 64));
        assert_eq!((sw, sh), (QL_MIN_CARD_W, QL_MIN_CARD_H));
        assert_eq!(quicklook_image_corner_radius(sw, sh, sfw, sfh), 0.0);

        // Medium image (e.g., 500x333 desktop thumbnail): card fits tightly to image without upscaling
        let (mw, mh, mfw, mfh) = fit_quicklook_card_size(500, 333, 1280, 960);
        assert_eq!((mfw, mfh), (500, 333));
        assert_eq!(mw, 500 + QL_IMAGE_INSET * 2);
        assert_eq!(mh, 333 + QL_HEADER_HEIGHT + QL_IMAGE_INSET);
    }

    #[test]
    fn load_pixbuf_no_upscale_preserves_small_image_dimensions() {
        let tmp = unique_temp_path("small_img", "png");
        let small = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 96, 72).unwrap();
        small.savev(&tmp, "png", &[]).unwrap();

        let loaded = load_pixbuf_no_upscale(&tmp, QL_MAX_DECODE_W, QL_MAX_DECODE_H).unwrap();
        assert_eq!((loaded.width(), loaded.height()), (96, 72));
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn compute_quicklook_card_size_adapts_video_pdf_and_enforces_minimums() {
        // Small video must not upscale and respects minimum card size
        let (vw, vh, vfit) = compute_quicklook_card_size(
            QuickLookContentSizing::Video {
                width: 200,
                height: 120,
            },
            1280,
            960,
            false,
        );
        assert_eq!(vfit, Some((200, 120)));
        assert_eq!(vw, QL_MIN_CARD_W);
        assert_eq!(vh, QL_MIN_CARD_H);

        // Portrait video adjusts card width/height to video instead of wide 820x600
        let (pvw, pvh, pvfit) = compute_quicklook_card_size(
            QuickLookContentSizing::Video {
                width: 248,
                height: 440,
            },
            1280,
            960,
            false,
        );
        assert_eq!(pvfit, Some((248, 440)));
        assert_eq!(pvw, QL_MIN_CARD_W);
        assert_eq!(pvh, 440 + QL_VIDEO_CHROME_H);

        // Portrait PDF adjusts to portrait proportions
        let (pdf_w, pdf_h, _) = compute_quicklook_card_size(
            QuickLookContentSizing::Pdf {
                width: 550,
                height: 780,
                has_footer: true,
            },
            1280,
            960,
            false,
        );
        assert!(pdf_h > pdf_w);
        assert!(pdf_w >= QL_MIN_CARD_W && pdf_h <= 960);
    }

    #[test]
    fn finish_close_immediately_without_overlay_does_not_reborrow_refcell() {
        let ql = QuickLook::new();
        ql.closing.set(true);
        ql.finish_close_immediately();
        assert!(!ql.closing.get());
        assert!(!ql.is_open());
        assert!(ql.host_overlay.borrow().is_none());
        assert!(ql.backdrop.borrow().is_none());
    }

    #[test]
    fn quicklook_squircle_clip_clears_extreme_corners_and_preserves_center() {
        assert_eq!(quicklook_concentric_inner_radius(32.0, 8.0), 24.0);
        assert_eq!(quicklook_concentric_inner_radius(12.0, 8.0), 8.0);
        assert_eq!(quicklook_concentric_inner_radius(0.0, 8.0), 0.0);

        let w = 64usize;
        let h = 64usize;
        let mut rgba = vec![255u8; w * h * 4];
        apply_squircle_clip_rgba_corners(&mut rgba, w, h, 24.0);
        // Las 4 esquinas extremas deben quedar transparentes
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[(w - 1) * 4 + 3], 0);
        assert_eq!(rgba[((h - 1) * w) * 4 + 3], 0);
        assert_eq!(rgba[((h - 1) * w + (w - 1)) * 4 + 3], 0);
        // El centro permanece 100% opaco
        assert_eq!(rgba[((h / 2) * w + (w / 2)) * 4 + 3], 255);
    }

    #[test]
    fn quicklook_open_and_close_animations_interpolate_cleanly() {
        let (s0, a0, y0) = quicklook_open_anim_state(0.0, false);
        assert!((s0 - 0.86).abs() < 1e-4);
        assert!(a0 <= 1e-4);
        assert!((y0 - 10.0).abs() < 1e-4);

        let (sm, am, ym) = quicklook_open_anim_state(0.5, false);
        assert!(sm > s0 && sm < 1.0);
        assert!(am > 0.5 && am <= 1.0);
        assert!(ym > 0.0 && ym < y0);

        let (s1, a1, y1) = quicklook_open_anim_state(1.0, false);
        assert!((s1 - 1.0).abs() < 1e-4);
        assert!((a1 - 1.0).abs() < 1e-4);
        assert!(y1.abs() < 1e-4);

        let (cs1, ca1, cy1) = quicklook_close_anim_state(1.0, false, 1.0, 1.0, 0.0);
        assert!((cs1 - 0.88).abs() < 1e-4);
        assert!(ca1 <= 1e-4);
        assert!((cy1 - 8.0).abs() < 1e-4);
    }
}
