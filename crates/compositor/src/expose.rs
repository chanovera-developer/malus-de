//! Motor de Exposé (Mission Control y App Exposé) para Malus DE.
//!
//! Calcula un mosaico sin solapamientos para las ventanas abiertas por monitor,
//! anima la transición de apertura/cierre con curva cúbica y genera los overlays
//! de resaltado, etiquetas de título y barra de filtrado en tiempo real.

use cairo::{Context, Format, ImageSurface};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            gles::GlesRenderer,
        },
    },
    desktop::Window,
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
};
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::time::Instant;

const ANIMATION_DURATION_SECS: f32 = 0.22;
pub const SPACES_BAR_HEIGHT_LOGICAL: i32 = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceDropTarget {
    Workspace(usize),
    NewSpace,
}

#[derive(Debug, Clone)]
pub struct SpaceThumbnailInfo {
    pub index: usize,
    pub name: String,
    pub is_active: bool,
    pub mini_rects: Vec<(f64, f64, f64, f64)>,
}

#[derive(Debug, Clone)]
pub struct SpaceBarHitbox {
    pub output_name: String,
    pub rect: Rectangle<i32, Logical>,
    pub target: SpaceDropTarget,
}

#[derive(Debug, Clone)]
pub struct ExposeDragState {
    pub slot_index: usize,
    pub window: Window,
    pub window_id: u64,
    pub orig_rect: Rectangle<i32, Logical>,
    pub start_rect: Rectangle<i32, Logical>,
    pub start_pointer: Point<f64, Logical>,
    pub current_pointer: Point<f64, Logical>,
    pub has_moved: bool,
    pub hovered_target: Option<SpaceDropTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExposeMode {
    Global,
    AppOnly { app_id: String },
}

#[derive(Debug, Clone)]
pub struct ExposeSlot {
    pub window: Window,
    pub window_id: u64,
    pub output_name: String,
    pub app_id: String,
    pub title: String,
    pub original_rect: Rectangle<i32, Logical>,
    pub target_rect: Rectangle<i32, Logical>,
    pub matches_filter: bool,
}

impl ExposeSlot {
    /// Interpola el rectángulo lógico actual según el progreso suavizado `t` (`0.0..=1.0`).
    pub fn current_rect(&self, t: f32) -> Rectangle<i32, Logical> {
        lerp_rect(self.original_rect, self.target_rect, t)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HighlightKey {
    w_phys: i32,
    h_phys: i32,
    scale_milli: i32,
    is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LabelKey {
    text: String,
    max_w_phys: i32,
    scale_milli: i32,
    highlighted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FilterKey {
    query: String,
    scale_milli: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SpacesBarKey {
    revision: u64,
    out_w_phys: i32,
    scale_milli: i32,
    active_workspace: usize,
    space_count: usize,
    can_add: bool,
    hovered_target: Option<(u8, usize)>,
    drop_target: Option<(u8, usize)>,
}

pub struct ExposeState {
    pub active: bool,
    pub target_open: bool,
    pub progress: f32,
    anim_start_progress: f32,
    anim_started_at: Option<Instant>,
    pub gesture_scrubbing: bool,
    pub mode: ExposeMode,
    pub slots: Vec<ExposeSlot>,
    pub hovered_index: Option<usize>,
    pub selected_index: Option<usize>,
    pub quick_look_index: Option<usize>,
    pub filter_query: String,
    pub previous_focus: Option<Window>,
    pub spaces_bar_hitboxes: Vec<SpaceBarHitbox>,
    pub hovered_space_target: Option<SpaceDropTarget>,
    pub dragging: Option<ExposeDragState>,
    pub render_revision: u64,
    highlight_cache: HashMap<HighlightKey, MemoryRenderBuffer>,
    label_cache: HashMap<LabelKey, (MemoryRenderBuffer, i32)>,
    filter_cache: Option<(FilterKey, MemoryRenderBuffer, i32)>,
    spaces_bar_cache: HashMap<SpacesBarKey, (MemoryRenderBuffer, i32, i32, i32, i32)>,
}

impl Default for ExposeState {
    fn default() -> Self {
        Self {
            active: false,
            target_open: false,
            progress: 0.0,
            anim_start_progress: 0.0,
            anim_started_at: None,
            gesture_scrubbing: false,
            mode: ExposeMode::Global,
            slots: Vec::new(),
            hovered_index: None,
            selected_index: None,
            quick_look_index: None,
            filter_query: String::new(),
            previous_focus: None,
            spaces_bar_hitboxes: Vec::new(),
            hovered_space_target: None,
            dragging: None,
            render_revision: 0,
            highlight_cache: HashMap::new(),
            label_cache: HashMap::new(),
            filter_cache: None,
            spaces_bar_cache: HashMap::new(),
        }
    }
}

impl ExposeState {
    pub fn bump_revision(&mut self) {
        self.render_revision = self.render_revision.wrapping_add(1);
    }

    pub fn is_animating(&self) -> bool {
        self.active && self.anim_started_at.is_some()
    }

    pub fn begin_gesture_scrub(&mut self) {
        self.gesture_scrubbing = true;
        self.anim_started_at = None;
    }

    pub fn set_gesture_progress(&mut self, progress: f32) {
        let p = progress.clamp(0.0, 1.0);
        if (self.progress - p).abs() > 0.001 {
            self.progress = p;
            self.target_open = p >= 0.35;
            self.bump_revision();
        }
    }

    pub fn finish_gesture_scrub(&mut self, open: bool) {
        self.gesture_scrubbing = false;
        if !self.active {
            return;
        }
        if open {
            self.target_open = true;
            self.anim_start_progress = self.progress;
            self.anim_started_at = Some(Instant::now());
            self.bump_revision();
        } else {
            self.start_close();
        }
    }

    /// Abre Exposé con los slots ya calculados e inicia la animación hacia `1.0`.
    pub fn open(
        &mut self,
        mode: ExposeMode,
        slots: Vec<ExposeSlot>,
        previous_focus: Option<Window>,
    ) {
        self.active = true;
        self.target_open = true;
        self.gesture_scrubbing = false;
        self.mode = mode;
        self.slots = slots;
        self.hovered_index = None;
        self.selected_index = (!self.slots.is_empty()).then_some(0);
        self.quick_look_index = None;
        self.filter_query.clear();
        self.previous_focus = previous_focus;
        self.hovered_space_target = None;
        self.dragging = None;
        self.anim_start_progress = self.progress;
        self.anim_started_at = Some(Instant::now());
        self.bump_revision();
    }

    /// Inicia la animación de cierre hacia `0.0`.
    pub fn start_close(&mut self) {
        if !self.active {
            return;
        }
        self.target_open = false;
        self.gesture_scrubbing = false;
        self.quick_look_index = None;
        self.hovered_index = None;
        self.hovered_space_target = None;
        self.dragging = None;
        self.anim_start_progress = self.progress;
        self.anim_started_at = Some(Instant::now());
        self.bump_revision();
    }

    /// Avanza la animación de apertura/cierre. Devuelve `true` si sigue animando.
    pub fn tick(&mut self) -> bool {
        if !self.active {
            return false;
        }
        if self.gesture_scrubbing {
            return false;
        }
        let target = if self.target_open { 1.0 } else { 0.0 };
        let Some(started_at) = self.anim_started_at else {
            return (self.progress - target).abs() > 0.001;
        };

        let raw_t = (started_at.elapsed().as_secs_f32() / ANIMATION_DURATION_SECS).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - raw_t).powi(3);
        self.progress = self.anim_start_progress + (target - self.anim_start_progress) * eased;
        self.bump_revision();

        if raw_t >= 1.0 {
            self.progress = target;
            self.anim_started_at = None;
            if !self.target_open {
                self.reset_closed();
                return false;
            }
            return false;
        }

        true
    }

    fn reset_closed(&mut self) {
        self.active = false;
        self.target_open = false;
        self.gesture_scrubbing = false;
        self.progress = 0.0;
        self.slots.clear();
        self.hovered_index = None;
        self.selected_index = None;
        self.quick_look_index = None;
        self.filter_query.clear();
        self.previous_focus = None;
        self.spaces_bar_hitboxes.clear();
        self.hovered_space_target = None;
        self.dragging = None;
        self.bump_revision();
    }

    /// Devuelve el progreso actual en `[0.0, 1.0]`.
    pub fn eased_progress(&self) -> f32 {
        self.progress.clamp(0.0, 1.0)
    }

    /// Actualiza la consulta de filtrado en vivo y marca las tarjetas coincidentes.
    pub fn apply_filter(&mut self, query: String) {
        self.filter_query = query;
        let q = self.filter_query.trim().to_lowercase();
        let mut first_match = None;
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            let matches = q.is_empty()
                || slot.title.to_lowercase().contains(&q)
                || slot.app_id.to_lowercase().contains(&q);
            slot.matches_filter = matches;
            if matches && first_match.is_none() {
                first_match = Some(idx);
            }
        }
        if let Some(sel) = self.selected_index {
            if !self.slots.get(sel).is_some_and(|s| s.matches_filter) {
                self.selected_index = first_match;
            }
        } else {
            self.selected_index = first_match;
        }
        if let Some(ql) = self.quick_look_index {
            if !self.slots.get(ql).is_some_and(|s| s.matches_filter) {
                self.quick_look_index = None;
            }
        }
        self.bump_revision();
    }

    /// Mueve la selección de teclado entre las tarjetas visibles que coinciden con el filtro.
    pub fn move_selection(&mut self, delta: isize) {
        let matching: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(idx, s)| s.matches_filter.then_some(idx))
            .collect();
        if matching.is_empty() {
            self.selected_index = None;
            return;
        }
        let current_pos = self
            .selected_index
            .and_then(|sel| matching.iter().position(|&idx| idx == sel))
            .unwrap_or(0);
        let len = matching.len() as isize;
        let next_pos = ((current_pos as isize + delta).rem_euclid(len)) as usize;
        self.selected_index = Some(matching[next_pos]);
        if self.quick_look_index.is_some() {
            self.quick_look_index = self.selected_index;
        }
        self.bump_revision();
    }

    /// Devuelve el índice de la tarjeta bajo `point` (en coordenadas lógicas globales).
    pub fn hit_test(&self, point: Point<f64, Logical>) -> Option<usize> {
        if !self.active {
            return None;
        }
        let t = self.eased_progress();
        if let Some(ql_idx) = self.quick_look_index {
            if let Some(slot) = self.slots.get(ql_idx) {
                let r = slot.current_rect(t);
                if point.x >= r.loc.x as f64
                    && point.x <= (r.loc.x + r.size.w) as f64
                    && point.y >= r.loc.y as f64
                    && point.y <= (r.loc.y + r.size.h) as f64
                {
                    return Some(ql_idx);
                }
            }
        }
        for (idx, slot) in self.slots.iter().enumerate() {
            if !slot.matches_filter {
                continue;
            }
            let r = slot.current_rect(t);
            if point.x >= r.loc.x as f64
                && point.x <= (r.loc.x + r.size.w) as f64
                && point.y >= r.loc.y as f64
                && point.y <= (r.loc.y + r.size.h) as f64
            {
                return Some(idx);
            }
        }
        None
    }

    /// Genera el marco de resaltado para la tarjeta seleccionada o con *hover*.
    pub fn render_highlight(
        &mut self,
        renderer: &mut GlesRenderer,
        card_phys: Rectangle<i32, Physical>,
        scale: f64,
        is_selected: bool,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let pad = (5.0 * scale).round() as i32;
        let w = card_phys.size.w + pad * 2;
        let h = card_phys.size.h + pad * 2;
        if w <= 8 || h <= 8 {
            return None;
        }
        let key = HighlightKey {
            w_phys: w,
            h_phys: h,
            scale_milli: (scale * 1000.0).round() as i32,
            is_selected,
        };
        if !self.highlight_cache.contains_key(&key) {
            if self.highlight_cache.len() > 24 {
                self.highlight_cache.clear();
            }
            let bytes = rasterize_highlight(w, h, pad as f64, scale, is_selected)?;
            let buf = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                Size::from((w, h)),
                1,
                Transform::Normal,
                None,
            );
            self.highlight_cache.insert(key.clone(), buf);
        }
        let buf = self.highlight_cache.get(&key)?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::from(((card_phys.loc.x - pad) as f64, (card_phys.loc.y - pad) as f64)),
            buf,
            None,
            None,
            None,
            smithay::backend::renderer::element::Kind::Unspecified,
        )
        .ok()
    }

    /// Genera la píldora inferior con el título de la ventana debajo de la tarjeta.
    pub fn render_card_label(
        &mut self,
        renderer: &mut GlesRenderer,
        card_phys: Rectangle<i32, Physical>,
        scale: f64,
        text: &str,
        highlighted: bool,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let max_w = card_phys.size.w.max(80);
        let key = LabelKey {
            text: text.to_string(),
            max_w_phys: max_w,
            scale_milli: (scale * 1000.0).round() as i32,
            highlighted,
        };
        if !self.label_cache.contains_key(&key) {
            if self.label_cache.len() > 64 {
                self.label_cache.clear();
            }
            let (bytes, w, h) = rasterize_label_pill(text, max_w, scale, highlighted)?;
            let buf = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                Size::from((w, h)),
                1,
                Transform::Normal,
                None,
            );
            self.label_cache.insert(key.clone(), (buf, w));
        }
        let (buf, pill_w) = self.label_cache.get(&key)?;
        let h_offset = (8.0 * scale).round() as i32;
        let x = card_phys.loc.x + (card_phys.size.w - *pill_w) / 2;
        let y = card_phys.loc.y + card_phys.size.h + h_offset;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::from((x as f64, y as f64)),
            buf,
            None,
            None,
            None,
            smithay::backend::renderer::element::Kind::Unspecified,
        )
        .ok()
    }

    /// Genera la píldora superior de búsqueda en vivo cuando `filter_query` no está vacío.
    pub fn render_filter_pill(
        &mut self,
        renderer: &mut GlesRenderer,
        output_phys_size: Size<i32, Physical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        if self.filter_query.is_empty() {
            return None;
        }
        let key = FilterKey {
            query: self.filter_query.clone(),
            scale_milli: (scale * 1000.0).round() as i32,
        };
        let needs_rebuild = self
            .filter_cache
            .as_ref()
            .is_none_or(|(k, _, _)| *k != key);
        if needs_rebuild {
            let max_w = ((output_phys_size.w as f64) * 0.45).round() as i32;
            let text = format!("Buscar: {}", self.filter_query);
            let (bytes, w, h) = rasterize_label_pill(&text, max_w.max(220), scale * 1.15, true)?;
            let buf = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                Size::from((w, h)),
                1,
                Transform::Normal,
                None,
            );
            self.filter_cache = Some((key, buf, w));
        }
        let (_, buf, pill_w) = self.filter_cache.as_ref()?;
        let x = (output_phys_size.w - *pill_w) / 2;
        let y = (46.0 * scale).round() as i32;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::from((x as f64, y as f64)),
            buf,
            None,
            None,
            None,
            smithay::backend::renderer::element::Kind::Unspecified,
        )
        .ok()
    }

    pub fn hit_test_space_bar(&self, point: Point<f64, Logical>) -> Option<SpaceDropTarget> {
        self.spaces_bar_hitboxes.iter().find_map(|hb| {
            let r = hb.rect;
            let inside = point.x >= r.loc.x as f64
                && point.x <= (r.loc.x + r.size.w) as f64
                && point.y >= r.loc.y as f64
                && point.y <= (r.loc.y + r.size.h) as f64;
            inside.then(|| hb.target.clone())
        })
    }

    /// Genera o reutiliza la Barra de Espacios superior (*Spaces Bar*) para un monitor.
    pub fn render_spaces_bar(
        &mut self,
        renderer: &mut GlesRenderer,
        output_geo: Rectangle<i32, Logical>,
        usable: Rectangle<i32, Logical>,
        scale: f64,
        spaces: &[SpaceThumbnailInfo],
        active_workspace: usize,
        can_add: bool,
        workspace_revision: u64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        if spaces.is_empty() {
            return None;
        }
        let hitboxes = compute_spaces_bar_hitboxes("", usable, spaces.len(), can_add);
        if hitboxes.is_empty() {
            return None;
        }

        let encode_target = |t: &Option<SpaceDropTarget>| match t {
            Some(SpaceDropTarget::Workspace(i)) => Some((1u8, *i)),
            Some(SpaceDropTarget::NewSpace) => Some((2u8, 0usize)),
            None => None,
        };
        let drop_target = self
            .dragging
            .as_ref()
            .filter(|d| d.has_moved)
            .and_then(|d| encode_target(&d.hovered_target));
        let hovered_target = encode_target(&self.hovered_space_target);

        let min_x = hitboxes.iter().map(|h| h.rect.loc.x).min().unwrap_or(0);
        let max_x = hitboxes
            .iter()
            .map(|h| h.rect.loc.x + h.rect.size.w)
            .max()
            .unwrap_or(0);
        let min_y = hitboxes.iter().map(|h| h.rect.loc.y).min().unwrap_or(0);
        let max_y = hitboxes
            .iter()
            .map(|h| h.rect.loc.y + h.rect.size.h)
            .max()
            .unwrap_or(0);
        let pad_x = 14;
        let pad_y = 8;
        let bar_rect_logical = Rectangle::new(
            (min_x - pad_x, min_y - pad_y).into(),
            (max_x - min_x + pad_x * 2, max_y - min_y + pad_y * 2).into(),
        );
        let bar_phys_w = (bar_rect_logical.size.w as f64 * scale).round().max(32.0) as i32;
        let bar_phys_h = (bar_rect_logical.size.h as f64 * scale).round().max(24.0) as i32;
        let bar_phys_x = ((bar_rect_logical.loc.x - output_geo.loc.x) as f64 * scale).round() as i32;
        let bar_phys_y = ((bar_rect_logical.loc.y - output_geo.loc.y) as f64 * scale).round() as i32;

        let key = SpacesBarKey {
            revision: workspace_revision,
            out_w_phys: bar_phys_w,
            scale_milli: (scale * 1000.0).round() as i32,
            active_workspace,
            space_count: spaces.len(),
            can_add,
            hovered_target,
            drop_target,
        };
        if !self.spaces_bar_cache.contains_key(&key) {
            if self.spaces_bar_cache.len() > 12 {
                self.spaces_bar_cache.clear();
            }
            let bytes = rasterize_spaces_bar(
                bar_phys_w,
                bar_phys_h,
                bar_rect_logical,
                scale,
                spaces,
                &hitboxes,
                self.hovered_space_target.as_ref(),
                self.dragging
                    .as_ref()
                    .filter(|d| d.has_moved)
                    .and_then(|d| d.hovered_target.as_ref()),
            )?;
            let buf = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                Size::from((bar_phys_w, bar_phys_h)),
                1,
                Transform::Normal,
                None,
            );
            self.spaces_bar_cache
                .insert(key.clone(), (buf, bar_phys_x, bar_phys_y, bar_phys_w, bar_phys_h));
        }
        let (buf, _, _, _, _) = self.spaces_bar_cache.get(&key)?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::from((bar_phys_x as f64, bar_phys_y as f64)),
            buf,
            None,
            None,
            None,
            smithay::backend::renderer::element::Kind::Unspecified,
        )
        .ok()
    }
}

/// Reserva la franja superior para la Barra de Espacios dentro del área utilizable del monitor.
pub fn expose_usable_zone_with_spaces_bar(usable: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let reserve = SPACES_BAR_HEIGHT_LOGICAL.min((usable.size.h - 140).max(0));
    Rectangle::new(
        (usable.loc.x, usable.loc.y + reserve).into(),
        (usable.size.w, (usable.size.h - reserve).max(120)).into(),
    )
}

/// Calcula los rectángulos lógicos de las miniaturas de espacios y el botón `+` en la barra superior.
pub fn compute_spaces_bar_hitboxes(
    output_name: &str,
    usable: Rectangle<i32, Logical>,
    space_count: usize,
    can_add: bool,
) -> Vec<SpaceBarHitbox> {
    if space_count == 0 || usable.size.w <= 80 {
        return Vec::new();
    }
    let gap = 12;
    let add_w = if can_add { 52 } else { 0 };
    let max_bar_w = (usable.size.w - 48).max(160);
    let fixed_gaps = (space_count.saturating_sub(1) as i32) * gap + if can_add { gap + add_w } else { 0 };
    let thumb_w = (((max_bar_w - fixed_gaps) / space_count as i32).clamp(72, 124)) as i32;
    let thumb_h = 68;
    let total_w = space_count as i32 * thumb_w + fixed_gaps;
    let start_x = usable.loc.x + (usable.size.w - total_w) / 2;
    let y = usable.loc.y + 10;

    let mut out = Vec::with_capacity(space_count + usize::from(can_add));
    for idx in 0..space_count {
        let x = start_x + idx as i32 * (thumb_w + gap);
        out.push(SpaceBarHitbox {
            output_name: output_name.to_string(),
            rect: Rectangle::new((x, y).into(), (thumb_w, thumb_h).into()),
            target: SpaceDropTarget::Workspace(idx),
        });
    }
    if can_add {
        let add_x = start_x + space_count as i32 * (thumb_w + gap);
        out.push(SpaceBarHitbox {
            output_name: output_name.to_string(),
            rect: Rectangle::new((add_x, y).into(), (add_w, thumb_h).into()),
            target: SpaceDropTarget::NewSpace,
        });
    }
    out
}

/// Calcula el rectángulo de una tarjeta de Exposé mientras está siendo arrastrada por el puntero.
pub fn drag_card_rect(drag: &ExposeDragState) -> Rectangle<i32, Logical> {
    let scale_factor = if drag.hovered_target.is_some() {
        0.34
    } else {
        0.64
    };
    let w = ((drag.start_rect.size.w as f64) * scale_factor).round().max(64.0) as i32;
    let h = ((drag.start_rect.size.h as f64) * scale_factor).round().max(44.0) as i32;
    let x = (drag.current_pointer.x - w as f64 * 0.5).round() as i32;
    let y = (drag.current_pointer.y - h as f64 * 0.5).round() as i32;
    Rectangle::new((x, y).into(), (w, h).into())
}

/// Calcula un mosaico sin solapamientos dentro de `usable_rect` conservando
/// la relación de aspecto de cada ventana.
pub fn compute_expose_layout(
    usable_rect: Rectangle<i32, Logical>,
    windows: &[(usize, Rectangle<i32, Logical>)],
) -> Vec<(usize, Rectangle<i32, Logical>)> {
    let n = windows.len();
    if n == 0 || usable_rect.size.w <= 0 || usable_rect.size.h <= 0 {
        return Vec::new();
    }

    let pad_x = 32;
    let pad_top = 28;
    let pad_bottom = 44;
    let gap_x = 24;
    let gap_y = 40;

    let avail_w = (usable_rect.size.w - pad_x * 2).max(120);
    let avail_h = (usable_rect.size.h - pad_top - pad_bottom).max(120);

    let mut best_cols = 1usize;
    let mut best_score = -1.0f64;

    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cell_w = ((avail_w - gap_x * (cols as i32 - 1)).max(40)) as f64 / cols as f64;
        let cell_h = ((avail_h - gap_y * (rows as i32 - 1)).max(40)) as f64 / rows as f64;

        let mut total_area = 0.0f64;
        let mut min_scale = f64::INFINITY;
        for &(_, rect) in windows {
            let w = rect.size.w.max(1) as f64;
            let h = rect.size.h.max(1) as f64;
            let s = (cell_w / w).min(cell_h / h).min(0.90);
            min_scale = min_scale.min(s);
            total_area += (w * s) * (h * s);
        }

        // Favorecer área total máxima y evitar filas vacías innecesarias.
        let empty_slots = (cols * rows - n) as f64;
        let score = total_area * (1.0 - empty_slots * 0.02) + min_scale * 1000.0;
        if score > best_score {
            best_score = score;
            best_cols = cols;
        }
    }

    let rows = n.div_ceil(best_cols);
    let cell_w = ((avail_w - gap_x * (best_cols as i32 - 1)).max(40)) as f64 / best_cols as f64;
    let cell_h = ((avail_h - gap_y * (rows as i32 - 1)).max(40)) as f64 / rows as f64;

    let grid_w = (best_cols as f64 * cell_w + (best_cols as i32 - 1) as f64 * gap_x as f64).round() as i32;
    let grid_h = (rows as f64 * cell_h + (rows as i32 - 1) as f64 * gap_y as f64).round() as i32;
    let origin_x = usable_rect.loc.x + pad_x + ((avail_w - grid_w) / 2).max(0);
    let origin_y = usable_rect.loc.y + pad_top + ((avail_h - grid_h) / 2).max(0);

    let mut result = Vec::with_capacity(n);
    for (idx, &(id, orig)) in windows.iter().enumerate() {
        let row = idx / best_cols;
        let col = idx % best_cols;

        // Centrar la última fila si tiene menos elementos que `best_cols`.
        let items_in_row = if row == rows - 1 {
            n - row * best_cols
        } else {
            best_cols
        };
        let row_offset_x = if items_in_row < best_cols {
            let row_w = items_in_row as f64 * cell_w + (items_in_row.saturating_sub(1)) as f64 * gap_x as f64;
            ((grid_w as f64 - row_w) / 2.0).round() as i32
        } else {
            0
        };

        let w = orig.size.w.max(1) as f64;
        let h = orig.size.h.max(1) as f64;
        let s = (cell_w / w).min(cell_h / h).min(0.90);
        let scaled_w = (w * s).round().max(32.0) as i32;
        let scaled_h = (h * s).round().max(24.0) as i32;

        let cell_x = origin_x + row_offset_x + (col as f64 * (cell_w + gap_x as f64)).round() as i32;
        let cell_y = origin_y + (row as f64 * (cell_h + gap_y as f64)).round() as i32;

        let target_x = cell_x + ((cell_w.round() as i32 - scaled_w) / 2).max(0);
        let target_y = cell_y + ((cell_h.round() as i32 - scaled_h) / 2).max(0);

        result.push((
            id,
            Rectangle::new((target_x, target_y).into(), (scaled_w, scaled_h).into()),
        ));
    }

    result
}

/// Calcula el rectángulo ampliado estilo *Quick Look* centrado dentro de `usable_rect`.
pub fn quick_look_rect(
    usable_rect: Rectangle<i32, Logical>,
    orig: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let max_w = (usable_rect.size.w as f64 * 0.84).max(160.0);
    let max_h = (usable_rect.size.h as f64 * 0.84).max(120.0);
    let w = orig.size.w.max(1) as f64;
    let h = orig.size.h.max(1) as f64;
    let s = (max_w / w).min(max_h / h).min(1.0);
    let out_w = (w * s).round() as i32;
    let out_h = (h * s).round() as i32;
    let out_x = usable_rect.loc.x + (usable_rect.size.w - out_w) / 2;
    let out_y = usable_rect.loc.y + (usable_rect.size.h - out_h) / 2;
    Rectangle::new((out_x, out_y).into(), (out_w, out_h).into())
}

fn lerp_rect(
    a: Rectangle<i32, Logical>,
    b: Rectangle<i32, Logical>,
    t: f32,
) -> Rectangle<i32, Logical> {
    let t = t as f64;
    let x = (a.loc.x as f64 + (b.loc.x - a.loc.x) as f64 * t).round() as i32;
    let y = (a.loc.y as f64 + (b.loc.y - a.loc.y) as f64 * t).round() as i32;
    let w = (a.size.w as f64 + (b.size.w - a.size.w) as f64 * t).round().max(1.0) as i32;
    let h = (a.size.h as f64 + (b.size.h - a.size.h) as f64 * t).round().max(1.0) as i32;
    Rectangle::new((x, y).into(), (w, h).into())
}

pub const EXPOSE_CARD_RADIUS_LOGICAL: f64 = 24.0;
pub const EXPOSE_SQUIRCLE_EXPONENT: f64 = 4.2;

/// Radio squircle en píxeles físicos para las tarjetas y miniaturas de Exposé.
pub fn card_corner_radius_phys(scale: f64) -> i32 {
    (EXPOSE_CARD_RADIUS_LOGICAL * scale).round().max(8.0) as i32
}

fn cairo_rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * FRAC_PI_2);
    cr.close_path();
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
    let steps = 12;

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

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn squircle_sdf(px: f64, py: f64, bx: f64, by: f64, r: f64, exp_val: f64) -> f64 {
    let dx = px.abs() - bx + r;
    let dy = py.abs() - by + r;
    if dx <= 0.0 || dy <= 0.0 {
        return dx.max(dy) - r;
    }
    (dx.powf(exp_val) + dy.powf(exp_val)).powf(1.0 / exp_val) - r
}

fn rasterize_highlight(
    w: i32,
    h: i32,
    pad: f64,
    scale: f64,
    is_selected: bool,
) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let line_w = (3.0 * scale).max(2.5);
    let card_radius = card_corner_radius_phys(scale) as f64;
    let half_w = w as f64 * 0.5;
    let half_h = h as f64 * 0.5;
    let outer_r = (card_radius + pad).min(half_w).min(half_h);

    let (cr, cg, cb, ca) = if is_selected {
        (0.04, 0.52, 1.0, 0.96)
    } else {
        (1.0, 1.0, 1.0, 0.72)
    };

    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let py = y as f64 + 0.5 - half_h;
        for x in 0..w {
            let px = x as f64 + 0.5 - half_w;
            let dist = squircle_sdf(px, py, half_w, half_h, outer_r, EXPOSE_SQUIRCLE_EXPONENT);
            if dist > 0.5 || dist < -(line_w + 1.5) {
                continue;
            }
            let outer_a = 1.0 - smoothstep(-1.0, 0.0, dist);
            let inner_a = smoothstep(-line_w - 1.0, -line_w, dist);
            let a = (outer_a * inner_a * ca).clamp(0.0, 1.0);
            if a <= 0.001 {
                continue;
            }
            let idx = ((y * w + x) * 4) as usize;
            data[idx] = (cb * a * 255.0).round() as u8;
            data[idx + 1] = (cg * a * 255.0).round() as u8;
            data[idx + 2] = (cr * a * 255.0).round() as u8;
            data[idx + 3] = (a * 255.0).round() as u8;
        }
    }
    Some(data)
}

fn rasterize_label_pill(
    text: &str,
    max_w: i32,
    scale: f64,
    highlighted: bool,
) -> Option<(Vec<u8>, i32, i32)> {
    let h = (28.0 * scale).round().max(20.0) as i32;
    let max_pill_w = max_w.max(60);
    let pad_x = 12.0 * scale;
    let font_size = 12.5 * scale;
    let max_text_w = (max_pill_w as f64 - pad_x * 2.0).max(16.0);

    let (display, text_w) = {
        let scratch = ImageSurface::create(Format::ARgb32, 1, 1).ok()?;
        let cr = Context::new(&scratch).ok()?;
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(font_size);
        let truncated = truncate_text(&cr, text, max_text_w);
        let ext_w = cr
            .text_extents(&truncated)
            .map(|e| e.width().max(e.x_advance()))
            .unwrap_or(0.0);
        (truncated, ext_w)
    };

    let w = ((text_w + pad_x * 2.0).ceil() as i32).clamp(48, max_pill_w);
    let mut surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    {
        let cr = Context::new(&surface).ok()?;
        let radius = h as f64 * 0.5;
        cairo_rounded_rect(&cr, 0.0, 0.0, w as f64, h as f64, radius);
        if highlighted {
            cr.set_source_rgba(0.04, 0.52, 1.0, 0.90);
        } else {
            cr.set_source_rgba(0.10, 0.11, 0.14, 0.78);
        }
        cr.fill_preserve().ok()?;
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
        cr.set_line_width((1.0 * scale).max(1.0));
        cr.stroke().ok()?;

        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(font_size);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.96);

        if let Ok(ext) = cr.text_extents(&display) {
            let tx = ((w as f64 - ext.width()) * 0.5 - ext.x_bearing()).max(pad_x);
            let ty = (h as f64 - ext.height()) * 0.5 - ext.y_bearing();
            cr.move_to(tx, ty);
            let _ = cr.show_text(&display);
        }
    }
    surface.flush();
    let data = surface.data().ok()?.to_vec();
    Some((data, w, h))
}

fn truncate_text(cr: &Context, text: &str, max_w: f64) -> String {
    if cr.text_extents(text).is_ok_and(|e| e.width() <= max_w) {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if cr.text_extents(&candidate).is_ok_and(|e| e.width() <= max_w) {
            return candidate;
        }
    }
    "…".to_string()
}

fn rasterize_spaces_bar(
    bar_w: i32,
    bar_h: i32,
    bar_logical: Rectangle<i32, Logical>,
    scale: f64,
    spaces: &[SpaceThumbnailInfo],
    hitboxes: &[SpaceBarHitbox],
    hovered: Option<&SpaceDropTarget>,
    drop_target: Option<&SpaceDropTarget>,
) -> Option<Vec<u8>> {
    let mut surface = ImageSurface::create(Format::ARgb32, bar_w, bar_h).ok()?;
    {
        let cr = Context::new(&surface).ok()?;
        let bar_r = 20.0 * scale;
        cairo_squircle_rect(
            &cr,
            0.5,
            0.5,
            bar_w as f64 - 1.0,
            bar_h as f64 - 1.0,
            bar_r,
            EXPOSE_SQUIRCLE_EXPONENT,
        );
        cr.set_source_rgba(0.08, 0.09, 0.12, 0.72);
        cr.fill_preserve().ok()?;
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        cr.set_line_width((1.0 * scale).max(1.0));
        cr.stroke().ok()?;

        for hb in hitboxes {
            let x = (hb.rect.loc.x - bar_logical.loc.x) as f64 * scale;
            let y = (hb.rect.loc.y - bar_logical.loc.y) as f64 * scale;
            let w = hb.rect.size.w as f64 * scale;
            let h = hb.rect.size.h as f64 * scale;
            let is_hovered = hovered == Some(&hb.target);
            let is_drop = drop_target == Some(&hb.target);

            match hb.target {
                SpaceDropTarget::Workspace(idx) => {
                    let Some(space) = spaces.get(idx) else {
                        continue;
                    };
                    let card_h = (h - 18.0 * scale).max(24.0);
                    let r = 13.0 * scale;
                    cairo_squircle_rect(&cr, x, y, w, card_h, r, EXPOSE_SQUIRCLE_EXPONENT);
                    if is_drop {
                        cr.set_source_rgba(0.04, 0.52, 1.0, 0.34);
                    } else if space.is_active {
                        cr.set_source_rgba(0.18, 0.24, 0.36, 0.88);
                    } else if is_hovered {
                        cr.set_source_rgba(0.18, 0.20, 0.25, 0.86);
                    } else {
                        cr.set_source_rgba(0.13, 0.14, 0.18, 0.80);
                    }
                    cr.fill_preserve().ok()?;

                    if is_drop || space.is_active {
                        cr.set_source_rgba(0.04, 0.52, 1.0, 0.96);
                        cr.set_line_width((2.2 * scale).max(1.5));
                    } else if is_hovered {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.60);
                        cr.set_line_width((1.5 * scale).max(1.0));
                    } else {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                        cr.set_line_width((1.0 * scale).max(1.0));
                    }
                    cr.stroke().ok()?;

                    // Miniaturas de ventanas dentro del espacio
                    let inner_pad = 4.0 * scale;
                    let inner_w = (w - inner_pad * 2.0).max(8.0);
                    let inner_h = (card_h - inner_pad * 2.0).max(8.0);
                    for &(nx, ny, nw, nh) in &space.mini_rects {
                        let mx = x + inner_pad + nx.clamp(0.0, 0.92) * inner_w;
                        let my = y + inner_pad + ny.clamp(0.0, 0.92) * inner_h;
                        let mw = (nw * inner_w).clamp(4.0 * scale, inner_w);
                        let mh = (nh * inner_h).clamp(3.0 * scale, inner_h);
                        cairo_squircle_rect(&cr, mx, my, mw, mh, 3.5 * scale, EXPOSE_SQUIRCLE_EXPONENT);
                        cr.set_source_rgba(0.85, 0.90, 0.98, 0.58);
                        let _ = cr.fill();
                    }

                    // Nombre del escritorio debajo de la miniatura
                    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
                    cr.set_font_size(10.5 * scale);
                    if space.is_active || is_drop {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
                    } else {
                        cr.set_source_rgba(0.85, 0.87, 0.92, 0.82);
                    }
                    if let Ok(ext) = cr.text_extents(&space.name) {
                        let tx = x + (w - ext.width()) * 0.5 - ext.x_bearing();
                        let ty = y + card_h + 13.0 * scale;
                        cr.move_to(tx, ty);
                        let _ = cr.show_text(&space.name);
                    }
                }
                SpaceDropTarget::NewSpace => {
                    let card_h = (h - 18.0 * scale).max(24.0);
                    let r = 13.0 * scale;
                    cairo_squircle_rect(&cr, x, y, w, card_h, r, EXPOSE_SQUIRCLE_EXPONENT);
                    if is_drop {
                        cr.set_source_rgba(0.04, 0.52, 1.0, 0.34);
                    } else if is_hovered {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                    } else {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
                    }
                    cr.fill_preserve().ok()?;
                    if is_drop {
                        cr.set_source_rgba(0.04, 0.52, 1.0, 0.96);
                        cr.set_line_width((2.0 * scale).max(1.5));
                    } else {
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.28);
                        cr.set_line_width((1.0 * scale).max(1.0));
                    }
                    cr.stroke().ok()?;

                    let cx = x + w * 0.5;
                    let cy = y + card_h * 0.5;
                    let arm = 7.0 * scale;
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
                    cr.set_line_width((2.0 * scale).max(1.5));
                    cr.move_to(cx - arm, cy);
                    cr.line_to(cx + arm, cy);
                    cr.move_to(cx, cy - arm);
                    cr.line_to(cx, cy + arm);
                    let _ = cr.stroke();
                }
            }
        }
    }
    surface.flush();
    let data = surface.data().ok()?.to_vec();
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects_overlap(a: Rectangle<i32, Logical>, b: Rectangle<i32, Logical>) -> bool {
        a.loc.x < b.loc.x + b.size.w
            && a.loc.x + a.size.w > b.loc.x
            && a.loc.y < b.loc.y + b.size.h
            && a.loc.y + a.size.h > b.loc.y
    }

    #[test]
    fn expose_layout_produces_non_overlapping_slots_inside_usable_area() {
        let usable = Rectangle::new((0, 32).into(), (1920, 960).into());
        let windows = vec![
            (1, Rectangle::new((100, 80).into(), (1200, 800).into())),
            (2, Rectangle::new((200, 120).into(), (900, 700).into())),
            (3, Rectangle::new((50, 50).into(), (1600, 900).into())),
            (4, Rectangle::new((300, 150).into(), (640, 480).into())),
            (5, Rectangle::new((400, 200).into(), (1000, 600).into())),
        ];

        let layout = compute_expose_layout(usable, &windows);
        assert_eq!(layout.len(), windows.len());

        for (i, (_, rect_i)) in layout.iter().enumerate() {
            assert!(rect_i.loc.x >= usable.loc.x);
            assert!(rect_i.loc.y >= usable.loc.y);
            assert!(rect_i.loc.x + rect_i.size.w <= usable.loc.x + usable.size.w);
            assert!(rect_i.loc.y + rect_i.size.h <= usable.loc.y + usable.size.h);
            assert!(rect_i.size.w > 0 && rect_i.size.h > 0);

            for (_, rect_j) in layout.iter().skip(i + 1) {
                assert!(
                    !rects_overlap(*rect_i, *rect_j),
                    "Overlap between {:?} and {:?}",
                    rect_i,
                    rect_j
                );
            }
        }
    }

    #[test]
    fn expose_layout_preserves_aspect_ratio() {
        let usable = Rectangle::new((0, 0).into(), (2560, 1440).into());
        let windows = vec![
            (1, Rectangle::new((0, 0).into(), (1600, 800).into())),
            (2, Rectangle::new((0, 0).into(), (800, 1200).into())),
        ];

        let layout = compute_expose_layout(usable, &windows);
        let (_, r1) = layout[0];
        let (_, r2) = layout[1];

        let ratio1 = r1.size.w as f64 / r1.size.h as f64;
        let ratio2 = r2.size.w as f64 / r2.size.h as f64;
        assert!((ratio1 - 2.0).abs() < 0.05);
        assert!((ratio2 - (800.0 / 1200.0)).abs() < 0.05);
    }

    #[test]
    fn quick_look_rect_stays_centered_inside_usable_area() {
        let usable = Rectangle::new((0, 32).into(), (1920, 960).into());
        let orig = Rectangle::new((40, 80).into(), (1200, 800).into());
        let ql = quick_look_rect(usable, orig);
        assert!(ql.loc.x >= usable.loc.x);
        assert!(ql.loc.y >= usable.loc.y);
        assert!(ql.loc.x + ql.size.w <= usable.loc.x + usable.size.w);
        assert!(ql.loc.y + ql.size.h <= usable.loc.y + usable.size.h);
        assert!(ql.size.w <= 1200 && ql.size.h <= 800);
    }

    #[test]
    fn spaces_bar_hitboxes_stay_centered_and_reserve_top_zone() {
        let usable = Rectangle::new((0, 36).into(), (1920, 1000).into());
        let reduced = expose_usable_zone_with_spaces_bar(usable);
        assert_eq!(reduced.loc.y, usable.loc.y + SPACES_BAR_HEIGHT_LOGICAL);
        assert_eq!(reduced.size.h, usable.size.h - SPACES_BAR_HEIGHT_LOGICAL);

        let hitboxes = compute_spaces_bar_hitboxes("DP-1", usable, 3, true);
        assert_eq!(hitboxes.len(), 4);
        assert_eq!(hitboxes[0].target, SpaceDropTarget::Workspace(0));
        assert_eq!(hitboxes[2].target, SpaceDropTarget::Workspace(2));
        assert_eq!(hitboxes[3].target, SpaceDropTarget::NewSpace);
        for hb in &hitboxes {
            assert!(hb.rect.loc.x >= usable.loc.x);
            assert!(hb.rect.loc.y >= usable.loc.y);
            assert!(hb.rect.loc.y + hb.rect.size.h <= reduced.loc.y);
        }
    }

    #[test]
    fn gesture_scrub_updates_progress_and_commits_cleanly() {
        let mut st = ExposeState::default();
        st.open(ExposeMode::Global, Vec::new(), None);
        st.begin_gesture_scrub();
        assert!(!st.is_animating());

        st.set_gesture_progress(0.45);
        assert!((st.progress - 0.45).abs() < 1e-5);
        assert!(st.target_open);
        assert!(!st.tick());

        st.finish_gesture_scrub(true);
        assert!(st.is_animating());
        assert!(st.target_open);
    }
}
