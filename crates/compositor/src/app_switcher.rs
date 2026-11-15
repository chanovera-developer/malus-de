//! Selector visual de aplicaciones estilo tarjetas vivas en cascada (Deck).
//!
//! Inspirado en el diseño de tarjetas superpuestas de `assets/screenshots/app-switcher.png`:
//! - Fondo desenfocado y oscurecido cubriendo el monitor activo.
//! - Carrusel horizontal en abanico con vistas previas en vivo (Wayland + barra SSD),
//!   recorte squircle n=4.2 y sombras de elevación entre planos.
//! - Encabezados superiores fuera de cada tarjeta con icono de aplicación y título.

use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::time::{Duration, Instant};

use cairo::{Context, Format, ImageSurface};
use malus_de_common::ipc::AppSwitcherItem;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
                Element, Kind,
            },
            gles::{GlesFrame, GlesRenderer},
            utils::draw_render_elements,
            Color32F,
        },
    },
    desktop::Window,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus,
};

use crate::blur::GaussianBlur;
use crate::shadow::WindowShadowPipeline;
use crate::ssd::{SsdManager, SSD_TITLEBAR_HEIGHT};
use crate::window_corners::{SurfaceTexParams, WindowSquircleShader};

const OPEN_ANIM_DURATION: Duration = Duration::from_millis(210);
const CLOSE_ANIM_DURATION: Duration = Duration::from_millis(180);
const FOCUS_SLIDE_DURATION: Duration = Duration::from_millis(190);
const DESKTOP_CARD_RADIUS_LOGICAL: f64 = 12.0;
const BASE_CARD_RADIUS_LOGICAL: f64 = 26.0;
const CARD_SQUIRCLE_EXPONENT: f64 = 4.2;
const HEADER_HEIGHT_LOGICAL: i32 = 34;
const HEADER_ICON_LOGICAL: i32 = 24;
const HEADER_GAP_LOGICAL: i32 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HeaderCacheKey {
    app_id: String,
    icon_name: String,
    title: String,
    show_title: bool,
    is_minimized: bool,
    align_right: bool,
    max_w_phys: i32,
    scale_milli: i32,
}

/// Geometría y atributos visuales calculados para una tarjeta dentro de la pila.
#[derive(Debug, Clone)]
pub struct DeckCardLayout {
    pub index: usize,
    pub window_id: u64,
    pub rect: Rectangle<i32, Logical>,
    pub depth_scale: f64,
    pub delta: f64,
    pub depth_dim: f32,
    pub corner_radius_logical: f64,
    pub header_pos: Point<i32, Logical>,
    pub header_max_w: i32,
    pub header_align_right: bool,
    pub show_title: bool,
    pub header_alpha: f32,
}

/// Estado visual y de animación para el selector estilo Deck.
#[derive(Debug, Default)]
pub struct AppSwitcherDeckState {
    pub active: bool,
    pub target_open: bool,
    pub progress: f32,
    anim_start_progress: f32,
    anim_started_at: Option<Instant>,
    pub output_name: Option<String>,
    pub origin_rects: HashMap<u64, Rectangle<i32, Logical>>,
    pub visual_focus: f64,
    focus_anim_from: f64,
    focus_anim_to: usize,
    focus_anim_started: Option<Instant>,
    pub hovered_index: Option<usize>,
    pub closed_dirty: bool,
    header_cache: HashMap<HeaderCacheKey, MemoryRenderBuffer>,
    icon_cache: HashMap<(String, i32), Option<ImageSurface>>,
}

impl AppSwitcherDeckState {
    pub fn reset_on_open(
        &mut self,
        output_name: Option<String>,
        initial_index: usize,
        origin_rects: HashMap<u64, Rectangle<i32, Logical>>,
        now: Instant,
    ) {
        let start_progress = if self.active {
            self.tick(now);
            self.progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.active = true;
        self.target_open = true;
        self.progress = start_progress;
        self.anim_start_progress = start_progress;
        self.anim_started_at = Some(now);
        self.output_name = output_name;
        self.origin_rects = origin_rects;
        self.visual_focus = initial_index as f64;
        self.focus_anim_from = initial_index as f64;
        self.focus_anim_to = initial_index;
        self.focus_anim_started = None;
        self.hovered_index = None;
        self.closed_dirty = false;
    }

    pub fn begin_close(&mut self, now: Instant) {
        if !self.active {
            return;
        }
        self.tick(now);
        self.target_open = false;
        self.anim_start_progress = self.progress;
        self.anim_started_at = Some(now);
        self.hovered_index = None;
    }

    pub fn take_closed_dirty(&mut self) -> bool {
        let dirty = self.closed_dirty;
        self.closed_dirty = false;
        dirty
    }

    pub fn should_render_on_output(&self, output_name: &str) -> bool {
        self.active
            && self
                .output_name
                .as_deref()
                .is_none_or(|name| name == output_name)
    }

    pub fn retarget_focus(&mut self, target_index: usize, now: Instant) {
        self.tick(now);
        self.focus_anim_from = self.visual_focus;
        self.focus_anim_to = target_index;
        if (self.focus_anim_from - target_index as f64).abs() > 0.001 {
            self.focus_anim_started = Some(now);
        } else {
            self.visual_focus = target_index as f64;
            self.focus_anim_started = None;
        }
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.target_open = false;
        self.progress = 0.0;
        self.anim_start_progress = 0.0;
        self.anim_started_at = None;
        self.output_name = None;
        self.origin_rects.clear();
        self.focus_anim_started = None;
        self.hovered_index = None;
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        if let Some(started) = self.focus_anim_started {
            let t = (now.saturating_duration_since(started).as_secs_f64()
                / FOCUS_SLIDE_DURATION.as_secs_f64())
            .clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            self.visual_focus =
                self.focus_anim_from + (self.focus_anim_to as f64 - self.focus_anim_from) * eased;
            if t >= 1.0 {
                self.visual_focus = self.focus_anim_to as f64;
                self.focus_anim_started = None;
            }
        }

        if self.active {
            if let Some(started) = self.anim_started_at {
                let dur = if self.target_open {
                    OPEN_ANIM_DURATION
                } else {
                    CLOSE_ANIM_DURATION
                };
                let target = if self.target_open { 1.0 } else { 0.0 };
                let raw_t = (now.saturating_duration_since(started).as_secs_f32()
                    / dur.as_secs_f32())
                .clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - raw_t).powi(3);
                self.progress =
                    self.anim_start_progress + (target - self.anim_start_progress) * eased;
                if raw_t >= 1.0 {
                    self.progress = target;
                    self.anim_started_at = None;
                    if !self.target_open {
                        self.clear();
                        self.closed_dirty = true;
                        return false;
                    }
                }
            }
        }

        self.is_animating(now)
    }

    pub fn is_animating(&self, _now: Instant) -> bool {
        self.active && (self.anim_started_at.is_some() || self.focus_anim_started.is_some())
    }

    pub fn open_eased(&self, _now: Instant) -> f32 {
        self.progress.clamp(0.0, 1.0)
    }

    fn load_cached_icon(&mut self, icon_name: &str, phys_size: i32) -> Option<ImageSurface> {
        let key = (icon_name.to_string(), phys_size);
        if let Some(cached) = self.icon_cache.get(&key) {
            return cached.clone();
        }
        if self.icon_cache.len() > 64 {
            self.icon_cache.clear();
        }
        let loaded = crate::spotlight_visual::load_icon_surface(icon_name, phys_size);
        self.icon_cache.insert(key, loaded.clone());
        loaded
    }

    #[allow(clippy::too_many_arguments)]
    fn render_header_element(
        &mut self,
        renderer: &mut GlesRenderer,
        item: &AppSwitcherItem,
        layout: &DeckCardLayout,
        output_geo: Rectangle<i32, Logical>,
        scale: f64,
        alpha: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let max_w_phys = ((layout.header_max_w as f64 * scale).round() as i32).max(24);
        let h_phys = ((HEADER_HEIGHT_LOGICAL as f64 * scale).round() as i32).max(20);
        let icon_name = item
            .icon_name
            .as_deref()
            .unwrap_or("application-x-executable");
        let display_name = deck_display_name(item);

        let key = HeaderCacheKey {
            app_id: item.app_id.clone(),
            icon_name: icon_name.to_string(),
            title: display_name.clone(),
            show_title: layout.show_title,
            is_minimized: item.is_minimized,
            align_right: layout.header_align_right,
            max_w_phys,
            scale_milli: (scale * 1000.0).round() as i32,
        };

        if !self.header_cache.contains_key(&key) {
            if self.header_cache.len() > 96 {
                self.header_cache.clear();
            }
            let icon_px = ((HEADER_ICON_LOGICAL as f64 * scale).round() as i32).max(16);
            let icon_surf = self.load_cached_icon(icon_name, icon_px);
            let bytes = rasterize_card_header(
                max_w_phys,
                h_phys,
                scale,
                icon_surf.as_ref(),
                &display_name,
                layout.show_title,
                item.is_minimized,
                layout.header_align_right,
            )?;
            let buf = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                Size::from((max_w_phys, h_phys)),
                1,
                Transform::Normal,
                None,
            );
            self.header_cache.insert(key.clone(), buf);
        }

        let buf = self.header_cache.get(&key)?;
        let phys_x = ((layout.header_pos.x - output_geo.loc.x) as f64 * scale).round() as i32;
        let phys_y = ((layout.header_pos.y - output_geo.loc.y) as f64 * scale).round() as i32;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::from((phys_x as f64, phys_y as f64)),
            buf,
            Some((alpha * layout.header_alpha).clamp(0.0, 1.0)),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }
}

fn deck_display_name(item: &AppSwitcherItem) -> String {
    let trimmed = item.title.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    item.app_id
        .rsplit('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Aplicación")
        .to_string()
}

/// Desplazamiento horizontal continuo de una tarjeta según su distancia `delta` al foco.
fn deck_horizontal_offset(delta: f64, front_w: f64) -> f64 {
    let sign = delta.signum();
    let abs = delta.abs();
    let step1 = front_w * 0.43;
    let step_tail = front_w * 0.23;
    if abs <= 1.0 {
        sign * abs * step1
    } else {
        sign * (step1 + (abs - 1.0) * step_tail)
    }
}

/// Calcula el tamaño base de la tarjeta focal conservando la relación de aspecto de la ventana.
fn fit_card_size(win_size: Size<i32, Logical>, max_w: f64, max_h: f64) -> Size<i32, Logical> {
    let w = win_size.w.max(240) as f64;
    let h = win_size.h.max(180) as f64;
    let s = (max_w / w).min(max_h / h).min(0.88);
    let out_w = (w * s).round().clamp(240.0, max_w) as i32;
    let out_h = (h * s).round().clamp(160.0, max_h) as i32;
    Size::from((out_w, out_h))
}

/// Calcula las geometrías ordenadas de atrás hacia adelante para todas las tarjetas del Deck.
pub fn compute_deck_layout(
    output_geo: Rectangle<i32, Logical>,
    visual_focus: f64,
    items: &[(u64, Size<i32, Logical>)],
) -> Vec<DeckCardLayout> {
    if items.is_empty() {
        return Vec::new();
    }

    let max_card_w = (output_geo.size.w as f64 * 0.50).clamp(320.0, 1100.0);
    let max_card_h = (output_geo.size.h as f64 * 0.58).clamp(220.0, 720.0);
    let center_x = output_geo.loc.x as f64 + output_geo.size.w as f64 * 0.5;
    let center_y = output_geo.loc.y as f64 + output_geo.size.h as f64 * 0.53;

    let focus_idx = (visual_focus.round() as isize).clamp(0, items.len() as isize - 1) as usize;
    let focus_base = fit_card_size(items[focus_idx].1, max_card_w, max_card_h);
    let front_step_ref = (focus_base.w as f64).max(460.0);

    let mut front_rect = Rectangle::new(
        Point::from((
            (center_x - focus_base.w as f64 * 0.5).round() as i32,
            (center_y - focus_base.h as f64 * 0.5).round() as i32,
        )),
        focus_base,
    );

    let mut raw = Vec::with_capacity(items.len());
    for (i, &(window_id, win_size)) in items.iter().enumerate() {
        let delta = i as f64 - visual_focus;
        let abs_d = delta.abs();
        let depth_scale = (1.0 - 0.085 * abs_d).clamp(0.66, 1.0);
        let base_size = fit_card_size(win_size, max_card_w, max_card_h);
        let card_w = (base_size.w as f64 * depth_scale).round().max(180.0) as i32;
        let card_h = (base_size.h as f64 * depth_scale).round().max(130.0) as i32;

        let card_cx = center_x + deck_horizontal_offset(delta, front_step_ref);
        let card_x = (card_cx - card_w as f64 * 0.5).round() as i32;
        let card_y = (center_y - card_h as f64 * 0.5).round() as i32;
        let rect = Rectangle::new(Point::from((card_x, card_y)), Size::from((card_w, card_h)));

        if abs_d < 0.5 {
            front_rect = rect;
        }

        let depth_dim = if abs_d < 0.05 {
            0.0
        } else {
            (0.09 + 0.10 * abs_d).clamp(0.0, 0.44) as f32
        };
        let corner_radius_logical = (BASE_CARD_RADIUS_LOGICAL * depth_scale).max(14.0);
        let header_alpha = if abs_d <= 2.6 {
            (1.0 - (abs_d - 0.5).max(0.0) * 0.28).clamp(0.25, 1.0) as f32
        } else {
            0.0
        };

        raw.push((abs_d, delta, i, window_id, rect, depth_scale, depth_dim, corner_radius_logical, header_alpha));
    }

    let mut result: Vec<DeckCardLayout> = raw
        .into_iter()
        .map(
            |(abs_d, delta, index, window_id, rect, depth_scale, depth_dim, corner_radius_logical, header_alpha)| {
                let is_front = abs_d < 0.35;
                let header_y = rect.loc.y - HEADER_HEIGHT_LOGICAL - HEADER_GAP_LOGICAL;
                let (header_pos, header_max_w, header_align_right, show_title) = if is_front {
                    let hx = rect.loc.x + 12;
                    let hw = (rect.size.w - 24).max(60);
                    (Point::from((hx, header_y)), hw, false, true)
                } else if delta < 0.0 {
                    let exposed_w = (front_rect.loc.x - rect.loc.x - 16).max(36);
                    let hx = rect.loc.x + 12;
                    (Point::from((hx, header_y)), exposed_w.min(rect.size.w - 16), false, false)
                } else {
                    let front_right = front_rect.loc.x + front_rect.size.w;
                    let card_right = rect.loc.x + rect.size.w;
                    let icon_box_w = HEADER_ICON_LOGICAL + 12;
                    let hx = (card_right - icon_box_w - 10).max(front_right + 6).min(card_right - icon_box_w);
                    (Point::from((hx, header_y)), icon_box_w, true, false)
                };

                DeckCardLayout {
                    index,
                    window_id,
                    rect,
                    depth_scale,
                    delta,
                    depth_dim,
                    corner_radius_logical,
                    header_pos,
                    header_max_w,
                    header_align_right,
                    show_title,
                    header_alpha,
                }
            },
        )
        .collect();

    // Pintar de mayor distancia al foco hacia 0.0 para que la tarjeta focal quede encima.
    result.sort_by(|a, b| {
        b.delta
            .abs()
            .partial_cmp(&a.delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.index.cmp(&b.index))
    });
    result
}

/// Hit-testing de adelante hacia atrás para que los clics respeten el solapamiento.
pub fn hit_test_deck_card(
    cards_back_to_front: &[DeckCardLayout],
    point: Point<f64, Logical>,
) -> Option<usize> {
    cards_back_to_front
        .iter()
        .rev()
        .find(|card| card.rect.to_f64().contains(point))
        .map(|card| card.index)
}

#[allow(clippy::too_many_arguments)]
fn rasterize_card_header(
    w_phys: i32,
    h_phys: i32,
    scale: f64,
    icon: Option<&ImageSurface>,
    title: &str,
    show_title: bool,
    is_minimized: bool,
    align_right: bool,
) -> Option<Vec<u8>> {
    if w_phys <= 4 || h_phys <= 4 {
        return None;
    }
    let mut surface = ImageSurface::create(Format::ARgb32, w_phys, h_phys).ok()?;
    let cr = Context::new(&surface).ok()?;
    cr.set_operator(cairo::Operator::Clear);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    let w = w_phys as f64;
    let h = h_phys as f64;
    let icon_sz = (HEADER_ICON_LOGICAL as f64 * scale).clamp(14.0, h - 4.0);
    let icon_y = (h - icon_sz) * 0.5;
    let icon_x = if align_right {
        (w - icon_sz - 2.0 * scale).max(0.0)
    } else {
        2.0 * scale
    };

    // Dibujar icono redondeado squircle
    let _ = cr.save();
    cairo_squircle_rect(
        &cr,
        icon_x,
        icon_y,
        icon_sz,
        icon_sz,
        icon_sz * 0.24,
        CARD_SQUIRCLE_EXPONENT,
    );
    cr.clip();
    if let Some(img) = icon {
        let iw = img.width().max(1) as f64;
        let ih = img.height().max(1) as f64;
        let sx = icon_sz / iw;
        let sy = icon_sz / ih;
        let _ = cr.save();
        cr.translate(icon_x, icon_y);
        cr.scale(sx, sy);
        let _ = cr.set_source_surface(img, 0.0, 0.0);
        let _ = cr.paint();
        let _ = cr.restore();
    } else {
        cr.set_source_rgba(0.24, 0.52, 0.96, 0.92);
        let _ = cr.paint();
    }
    let _ = cr.restore();

    if show_title && !align_right {
        let text_x = icon_x + icon_sz + 9.0 * scale;
        let avail_text_w = (w - text_x - 4.0 * scale).max(0.0);
        if avail_text_w > 12.0 {
            let label = if is_minimized {
                format!("{title} · Minimizada")
            } else {
                title.to_string()
            };
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(14.0 * scale);
            let shown = fit_text_to_width(&cr, &label, avail_text_w);
            if let Ok(ext) = cr.text_extents(&shown) {
                let text_y = (h - ext.height()) * 0.5 - ext.y_bearing();
                // Sombra suave para contraste sobre fondos claros
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
                cr.move_to(text_x, text_y + 1.2 * scale);
                let _ = cr.show_text(&shown);

                cr.set_source_rgba(1.0, 1.0, 1.0, 0.97);
                cr.move_to(text_x, text_y);
                let _ = cr.show_text(&shown);
            }
        }
    }

    drop(cr);
    surface.flush();
    surface.data().ok().map(|d| d.to_vec())
}

fn fit_text_to_width(cr: &Context, text: &str, max_w: f64) -> String {
    if let Ok(ext) = cr.text_extents(text) {
        if ext.width() <= max_w {
            return text.to_string();
        }
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if let Ok(ext) = cr.text_extents(&candidate) {
            if ext.width() <= max_w {
                return candidate;
            }
        }
    }
    "…".to_string()
}

fn cairo_squircle_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64, exp_val: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.5 {
        cr.rectangle(x, y, w, h);
        return;
    }
    let lx = x + r;
    let rx = x + w - r;
    let ty = y + r;
    let by = y + h - r;
    let p = 2.0 / exp_val.max(2.0);
    let steps = 10;

    cr.new_sub_path();
    cr.move_to(lx, y);
    for i in 0..=steps {
        let u = FRAC_PI_2 * (i as f64 / steps as f64);
        cr.line_to(rx + r * u.sin().powf(p), ty - r * u.cos().powf(p));
    }
    for i in 0..=steps {
        let u = FRAC_PI_2 * (i as f64 / steps as f64);
        cr.line_to(rx + r * u.cos().powf(p), by + r * u.sin().powf(p));
    }
    for i in 0..=steps {
        let u = FRAC_PI_2 * (i as f64 / steps as f64);
        cr.line_to(lx - r * u.sin().powf(p), by + r * u.cos().powf(p));
    }
    for i in 0..=steps {
        let u = FRAC_PI_2 * (i as f64 / steps as f64);
        cr.line_to(lx - r * u.cos().powf(p), ty - r * u.sin().powf(p));
    }
    cr.close_path();
}

fn lerp_rect(
    a: Rectangle<i32, Logical>,
    b: Rectangle<i32, Logical>,
    t: f32,
) -> Rectangle<i32, Logical> {
    let t = t as f64;
    let x = a.loc.x as f64 + (b.loc.x - a.loc.x) as f64 * t;
    let y = a.loc.y as f64 + (b.loc.y - a.loc.y) as f64 * t;
    let w = a.size.w as f64 + (b.size.w - a.size.w) as f64 * t;
    let h = a.size.h as f64 + (b.size.h - a.size.h) as f64 * t;
    Rectangle::new(
        (x.round() as i32, y.round() as i32).into(),
        (w.round().max(1.0) as i32, h.round().max(1.0) as i32).into(),
    )
}

/// Datos preconstruidos en CPU/elementos Wayland antes de iniciar el `GlesFrame`.
pub struct PreparedDeckCard {
    pub card_phys: Rectangle<i32, Physical>,
    pub card_radius_phys: i32,
    pub card_alpha: f32,
    pub blur_rect_phys: Option<Rectangle<i32, Physical>>,
    pub depth_dim: f32,
    pub is_front: bool,
    pub ssd_elem: Option<MemoryRenderBufferRenderElement<GlesRenderer>>,
    pub scaled_elems: Vec<(
        RelocateRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
        Option<SurfaceTexParams>,
    )>,
    pub header_elem: Option<MemoryRenderBufferRenderElement<GlesRenderer>>,
}

/// Prepara las tarjetas ordenadas para renderizarse sobre `output_geo`.
#[allow(clippy::too_many_arguments)]
pub fn prepare_deck_cards(
    renderer: &mut GlesRenderer,
    deck: &mut AppSwitcherDeckState,
    ssd: &mut SsdManager,
    items: &[AppSwitcherItem],
    windows_by_id: &[(u64, Window, Size<i32, Logical>, bool)],
    output_geo: Rectangle<i32, Logical>,
    scale: f64,
    is_dark: bool,
    now: Instant,
) -> (f32, Vec<PreparedDeckCard>) {
    deck.tick(now);
    let progress = deck.open_eased(now);
    let header_alpha = ((progress - 0.25) / 0.75).clamp(0.0, 1.0);

    let mut size_inputs = Vec::with_capacity(items.len());
    for item in items {
        let win_sz = windows_by_id
            .iter()
            .find(|(id, ..)| *id == item.window_id)
            .map(|(_, _, sz, _)| *sz)
            .unwrap_or_else(|| Size::from((1080, 700)));
        size_inputs.push((item.window_id, win_sz));
    }

    let layouts = compute_deck_layout(output_geo, deck.visual_focus, &size_inputs);
    let mut prepared = Vec::with_capacity(layouts.len());

    for layout in layouts {
        let item = &items[layout.index];
        let win_entry = windows_by_id.iter().find(|(id, ..)| *id == item.window_id);

        let (draw_rect, card_alpha, radius_logical) =
            if let Some(&orig_rect) = deck.origin_rects.get(&item.window_id) {
                let rect = lerp_rect(orig_rect, layout.rect, progress);
                let radius = DESKTOP_CARD_RADIUS_LOGICAL
                    + (layout.corner_radius_logical - DESKTOP_CARD_RADIUS_LOGICAL)
                        * progress as f64;
                (rect, 1.0_f32, radius)
            } else {
                let start_w = ((layout.rect.size.w as f32) * 0.80).round().max(32.0) as i32;
                let start_h = ((layout.rect.size.h as f32) * 0.80).round().max(24.0) as i32;
                let start_rect = Rectangle::new(
                    (
                        layout.rect.loc.x + (layout.rect.size.w - start_w) / 2,
                        layout.rect.loc.y + (layout.rect.size.h - start_h) / 2 + 28,
                    )
                        .into(),
                    (start_w, start_h).into(),
                );
                (
                    lerp_rect(start_rect, layout.rect, progress),
                    progress,
                    layout.corner_radius_logical,
                )
            };

        let card_phys_loc = (draw_rect.loc - output_geo.loc)
            .to_f64()
            .to_physical(scale)
            .to_i32_round();
        let card_phys_size = draw_rect
            .size
            .to_f64()
            .to_physical(scale)
            .to_i32_round();
        let card_phys = Rectangle::new(card_phys_loc, card_phys_size);
        let card_radius_phys = (radius_logical * scale).round().max(8.0) as i32;
        let is_front = layout.delta.abs() < 0.35;

        let mut ssd_elem = None;
        let mut scaled_elems = Vec::new();
        let mut blur_rect_phys = None;

        if card_alpha > 0.02 {
            if let Some((_, win, orig_total_sz, uses_ssd)) = win_entry {
                let win_geo = win.geometry();
                let k = (draw_rect.size.w as f64 / orig_total_sz.w.max(1) as f64)
                    .min(draw_rect.size.h as f64 / orig_total_sz.h.max(1) as f64)
                    .clamp(0.02, 4.0);
                let tb_logical = if *uses_ssd { SSD_TITLEBAR_HEIGHT } else { 0 };
                let scaled_tb_phys = ((tb_logical as f64 * k * scale).round() as i32).max(0);

                if crate::state::is_translucent_window(win) {
                    blur_rect_phys = Some(if crate::state::is_branches_window(win) {
                        let sb_w = ((212.0 * k * scale).round() as i32 + card_radius_phys + 4)
                            .min(card_phys.size.w);
                        Rectangle::new(card_phys.loc, (sb_w, card_phys.size.h).into())
                    } else {
                        card_phys
                    });
                }

                if let Some(surf) = win.wl_surface() {
                    if *uses_ssd && scaled_tb_phys > 4 && card_alpha > 0.15 {
                        ssd_elem = ssd.render_element_for_window(
                            renderer,
                            &surf,
                            card_phys.loc,
                            card_phys.size.w,
                            scaled_tb_phys,
                            scale * k,
                            &item.title,
                            is_front,
                            is_dark,
                        );
                    }

                    let orig_phys_loc = Point::from((0, 0));
                    let surf_phys_loc =
                        Point::<i32, Logical>::from((-win_geo.loc.x, -win_geo.loc.y))
                            .to_f64()
                            .to_physical(scale)
                            .to_i32_round();
                    let target_client_loc =
                        Point::from((card_phys.loc.x, card_phys.loc.y + scaled_tb_phys));

                    let raw_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                        render_elements_from_surface_tree(
                            renderer,
                            &surf,
                            surf_phys_loc,
                            Scale::from(scale),
                            card_alpha,
                            Kind::Unspecified,
                        );
                    scaled_elems = raw_elems
                        .into_iter()
                        .map(|elem| {
                            let tex_params = SurfaceTexParams::from_wayland_element(&elem);
                            let rescaled = RescaleRenderElement::from_element(
                                elem,
                                orig_phys_loc,
                                Scale::from(k),
                            );
                            let relocated = RelocateRenderElement::from_element(
                                rescaled,
                                target_client_loc - orig_phys_loc,
                                Relocate::Relative,
                            );
                            (relocated, tex_params)
                        })
                        .collect();
                }
            }
        }

        let header_elem = if header_alpha > 0.02 {
            deck.render_header_element(renderer, item, &layout, output_geo, scale, header_alpha)
        } else {
            None
        };

        prepared.push(PreparedDeckCard {
            card_phys,
            card_radius_phys,
            card_alpha,
            blur_rect_phys,
            depth_dim: layout.depth_dim * header_alpha,
            is_front,
            ssd_elem,
            scaled_elems,
            header_elem,
        });
    }

    (progress, prepared)
}

/// Dibuja las tarjetas animadas del App Switcher Deck sobre el escritorio limpio (sin blur global).
#[allow(clippy::too_many_arguments)]
pub fn render_deck_frame(
    frame: &mut GlesFrame<'_, '_>,
    projection: &[f32; 9],
    screen_size: Size<i32, Physical>,
    scale: f64,
    damage: &[Rectangle<i32, Physical>],
    blur: Option<&mut GaussianBlur>,
    shadow: Option<&WindowShadowPipeline>,
    squircle_shader: Option<&WindowSquircleShader>,
    eased: f32,
    cards: &[PreparedDeckCard],
) {
    let mut blur = blur;
    let _ = frame.draw_solid(
        Rectangle::from_size(screen_size),
        damage,
        Color32F::new(0.02, 0.03, 0.05, 0.22 * eased),
    );

    for card in cards {
        if card.card_alpha <= 0.02 {
            continue;
        }
        if let Some(sh) = shadow {
            let shadow_alpha = if card.is_front { 0.96 } else { 0.68 } * card.card_alpha;
            let _ = frame.with_context(|gl| {
                sh.render_menu_shadow_alpha(
                    gl,
                    projection,
                    card.card_phys,
                    card.card_radius_phys as f32,
                    scale,
                    shadow_alpha,
                );
            });
        }

        let has_content = !card.scaled_elems.is_empty() || card.ssd_elem.is_some();
        if !has_content {
            if let Some(ref mut b) = blur {
                let _ = frame.with_context(|gl| {
                    b.render_squircle_tint(
                        gl,
                        projection,
                        screen_size,
                        card.card_phys,
                        card.card_radius_phys as f32,
                        [0.14, 0.16, 0.22, 0.88 * card.card_alpha],
                        (1.2 * scale as f32).max(1.0),
                        [1.0, 1.0, 1.0, 0.22 * card.card_alpha],
                    );
                });
            }
        } else {
            if let Some(ref mut b) = blur {
                if let Some(blur_rect) = card.blur_rect_phys {
                    let _ = frame.with_context(|gl| {
                        b.render_blur_alpha(
                            gl,
                            projection,
                            screen_size,
                            blur_rect,
                            card.card_radius_phys as f32,
                            card.card_alpha,
                        );
                    });
                }
            }

            if let Some(shader) = squircle_shader {
                for (elem, tex_params) in card.scaled_elems.iter().rev() {
                    let elem_geo = elem.geometry(Scale::from(scale));
                    let sub_offset = Point::from((
                        card.card_phys.loc.x - elem_geo.loc.x,
                        card.card_phys.loc.y - elem_geo.loc.y,
                    ));
                    shader.apply_surface_to_frame(
                        frame,
                        *tex_params,
                        elem_geo.size,
                        sub_offset,
                        card.card_phys.size,
                        card.card_radius_phys,
                        if card.is_front { 1.0 } else { 0.4 },
                        false,
                    );
                    let _ = draw_render_elements(frame, scale, std::slice::from_ref(elem), damage);
                    frame.clear_tex_program_override();
                }
                if let Some(ref tb_elem) = card.ssd_elem {
                    let tb_geo = tb_elem.geometry(Scale::from(1.0));
                    let sub_offset = Point::from((
                        card.card_phys.loc.x - tb_geo.loc.x,
                        card.card_phys.loc.y - tb_geo.loc.y,
                    ));
                    shader.apply_to_frame(
                        frame,
                        tb_geo.size,
                        sub_offset,
                        card.card_phys.size,
                        card.card_radius_phys,
                    );
                    let _ = draw_render_elements(frame, 1.0, std::slice::from_ref(tb_elem), damage);
                    frame.clear_tex_program_override();
                }
            } else {
                for (elem, _) in card.scaled_elems.iter().rev() {
                    let _ = draw_render_elements(frame, scale, std::slice::from_ref(elem), damage);
                }
                if let Some(ref tb_elem) = card.ssd_elem {
                    let _ = draw_render_elements(frame, 1.0, std::slice::from_ref(tb_elem), damage);
                }
            }

            if let Some(ref mut b) = blur {
                if card.depth_dim > 0.01 {
                    let _ = frame.with_context(|gl| {
                        b.render_squircle_tint(
                            gl,
                            projection,
                            screen_size,
                            card.card_phys,
                            card.card_radius_phys as f32,
                            [0.02, 0.03, 0.06, card.depth_dim],
                            0.0,
                            [0.0; 4],
                        );
                    });
                } else if card.is_front && eased > 0.15 {
                    let _ = frame.with_context(|gl| {
                        b.render_squircle_tint(
                            gl,
                            projection,
                            screen_size,
                            card.card_phys,
                            card.card_radius_phys as f32,
                            [0.0, 0.0, 0.0, 0.0],
                            (1.2 * scale as f32).max(1.0),
                            [1.0, 1.0, 1.0, 0.24 * ((eased - 0.15) / 0.85).clamp(0.0, 1.0)],
                        );
                    });
                }
            }
        }

        if let Some(ref header) = card.header_elem {
            let _ = draw_render_elements(frame, 1.0, std::slice::from_ref(header), damage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_layout_centers_focal_card_and_sorts_back_to_front() {
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let items = vec![
            (1, Size::from((1200, 800))),
            (2, Size::from((1200, 800))),
            (3, Size::from((1200, 800))),
            (4, Size::from((1200, 800))),
        ];

        let layout = compute_deck_layout(output_geo, 1.0, &items);
        assert_eq!(layout.len(), 4);

        // El orden de pintado termina siempre en la tarjeta focal (index 1)
        let front = layout.last().unwrap();
        assert_eq!(front.index, 1);
        assert!((front.depth_scale - 1.0).abs() < 1e-6);
        assert_eq!(front.depth_dim, 0.0);
        assert!(front.show_title);

        let center_x = front.rect.loc.x + front.rect.size.w / 2;
        assert!((center_x - 960).abs() <= 2);

        // Las tarjetas vecinas tienen escala decreciente y quedan parcialmente solapadas
        let left = layout.iter().find(|c| c.index == 0).unwrap();
        let right = layout.iter().find(|c| c.index == 2).unwrap();
        assert!(left.depth_scale < front.depth_scale);
        assert!(right.depth_scale < front.depth_scale);
        assert!(left.rect.loc.x < front.rect.loc.x);
        assert!(left.rect.loc.x + left.rect.size.w > front.rect.loc.x);
        assert!(right.rect.loc.x > front.rect.loc.x);
        assert!(right.rect.loc.x < front.rect.loc.x + front.rect.size.w);
    }

    #[test]
    fn hit_test_prefers_front_card_on_overlap() {
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let items = vec![
            (1, Size::from((1000, 700))),
            (2, Size::from((1000, 700))),
            (3, Size::from((1000, 700))),
        ];

        let layout = compute_deck_layout(output_geo, 1.0, &items);
        let front = layout.last().unwrap();
        let left = layout.iter().find(|c| c.index == 0).unwrap();

        // Clic en el centro de la tarjeta frontal debe seleccionar la frontal (1)
        let front_pt = Point::from((
            (front.rect.loc.x + front.rect.size.w / 2) as f64,
            (front.rect.loc.y + front.rect.size.h / 2) as f64,
        ));
        assert_eq!(hit_test_deck_card(&layout, front_pt), Some(1));

        // Clic en la franja izquierda descubierta debe seleccionar la izquierda (0)
        let left_exposed_pt = Point::from((
            (left.rect.loc.x + 12) as f64,
            (left.rect.loc.y + left.rect.size.h / 2) as f64,
        ));
        assert_eq!(hit_test_deck_card(&layout, left_exposed_pt), Some(0));

        // Clic fuera de todas las tarjetas devuelve None
        assert_eq!(hit_test_deck_card(&layout, Point::from((10.0, 10.0))), None);
    }

    #[test]
    fn deck_state_retargets_and_interpolates_focus_smoothly() {
        let t0 = Instant::now();
        let mut deck = AppSwitcherDeckState::default();
        deck.reset_on_open(Some("eDP-1".to_string()), 0, HashMap::new(), t0);
        assert_eq!(deck.visual_focus, 0.0);

        deck.retarget_focus(2, t0 + Duration::from_millis(20));
        deck.tick(t0 + Duration::from_millis(100));
        assert!(deck.visual_focus > 0.0 && deck.visual_focus < 2.0);

        deck.tick(t0 + Duration::from_millis(300));
        assert!((deck.visual_focus - 2.0).abs() < 1e-6);
        assert!(!deck.is_animating(t0 + Duration::from_millis(300)));

        // Prueba de cierre animado: vuelve de 1.0 -> 0.0 y marca closed_dirty al terminar
        deck.begin_close(t0 + Duration::from_millis(320));
        assert!(deck.active);
        assert!(!deck.target_open);
        deck.tick(t0 + Duration::from_millis(400));
        assert!(deck.active && deck.progress < 1.0 && deck.progress > 0.0);
        deck.tick(t0 + Duration::from_millis(600));
        assert!(!deck.active);
        assert!(deck.take_closed_dirty());
        assert!(!deck.take_closed_dirty());
    }
}
