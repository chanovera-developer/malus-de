//! Menú flotante Snap Layouts estilo Windows 11 al pasar el cursor sobre el botón de maximizar.
//!
//! Mantiene intacto el ajuste por arrastre a bordes de pantalla y añade un popover translúcido
//! con las 6 plantillas de distribución de Windows 11 al hacer hover sobre el botón de maximizar
//! (tanto en barras SSD como en controles semáforo CSD compatibles).

use std::f64::consts::FRAC_PI_2;
use std::time::{Duration, Instant};

use cairo::{Context, Format, ImageSurface, LinearGradient, Operator};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    desktop::{Space, Window},
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Size},
    wayland::seat::WaylandFocus,
};

use crate::state::{is_gnome_window, window_uses_ssd, SnapMode};

/// Retardo de reposo sobre el botón de maximizar antes de abrir el menú (evita parpadeos accidentales).
pub const HOVER_OPEN_DELAY: Duration = Duration::from_millis(240);

/// Margen de gracia antes de cerrar al salir del botón y del menú flotante.
pub const HOVER_CLOSE_DELAY: Duration = Duration::from_millis(260);

/// Duración de la animación de entrada del menú flotante.
pub const OPEN_ANIM_DURATION: Duration = Duration::from_millis(140);

const SQUIRCLE_EXPONENT: f64 = 4.2;
pub const POPUP_CORNER_RADIUS: f64 = 16.0;

const POPUP_PADDING: f64 = 14.0;
const CARD_WIDTH: f64 = 82.0;
const CARD_HEIGHT: f64 = 54.0;
const CARD_GAP: f64 = 10.0;
const ZONE_GAP: f64 = 3.5;
const ZONE_RADIUS: f64 = 4.5;

const SSD_MAX_HALF_W: f64 = 9.5;
const SSD_MAX_HALF_H: f64 = 11.0;
const CSD_MAX_MIN_X: f64 = 53.0;
const CSD_MAX_MAX_X: f64 = 78.0;
const CSD_MAX_MIN_Y: f64 = 7.0;
const CSD_MAX_MAX_Y: f64 = 34.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutZoneSpec {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub mode: SnapMode,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutCardSpec {
    pub zones: &'static [LayoutZoneSpec],
}

/// Las 6 plantillas estándar de Snap Layouts de Windows 11 (3 columnas × 2 filas):
/// 1. 50/50 | 2. 67/33 | 3. 33/33/33
/// 4. 50 + 2 cuartos | 5. 4 cuartos (2×2) | 6. 25/50/25
pub const WINDOWS_LAYOUT_CARDS: [LayoutCardSpec; 6] = [
    // 0: Mitad izquierda / Mitad derecha (50 / 50)
    LayoutCardSpec {
        zones: &[
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.0,
                x1: 0.5,
                y1: 1.0,
                mode: SnapMode::LeftHalf,
            },
            LayoutZoneSpec {
                x0: 0.5,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                mode: SnapMode::RightHalf,
            },
        ],
    },
    // 1: Dos tercios izquierda / Un tercio derecha (67 / 33)
    LayoutCardSpec {
        zones: &[
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.0,
                x1: 2.0 / 3.0,
                y1: 1.0,
                mode: SnapMode::LeftTwoThirds,
            },
            LayoutZoneSpec {
                x0: 2.0 / 3.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                mode: SnapMode::RightOneThird,
            },
        ],
    },
    // 2: Tres columnas iguales (33 / 33 / 33)
    LayoutCardSpec {
        zones: &[
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0 / 3.0,
                y1: 1.0,
                mode: SnapMode::LeftThird,
            },
            LayoutZoneSpec {
                x0: 1.0 / 3.0,
                y0: 0.0,
                x1: 2.0 / 3.0,
                y1: 1.0,
                mode: SnapMode::CenterThird,
            },
            LayoutZoneSpec {
                x0: 2.0 / 3.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                mode: SnapMode::RightThird,
            },
        ],
    },
    // 3: Mitad izquierda + 2 cuartos apilados a la derecha
    LayoutCardSpec {
        zones: &[
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.0,
                x1: 0.5,
                y1: 1.0,
                mode: SnapMode::LeftHalf,
            },
            LayoutZoneSpec {
                x0: 0.5,
                y0: 0.0,
                x1: 1.0,
                y1: 0.5,
                mode: SnapMode::TopRightQuarter,
            },
            LayoutZoneSpec {
                x0: 0.5,
                y0: 0.5,
                x1: 1.0,
                y1: 1.0,
                mode: SnapMode::BottomRightQuarter,
            },
        ],
    },
    // 4: Cuadrícula 2×2 (4 cuartos iguales)
    LayoutCardSpec {
        zones: &[
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.0,
                x1: 0.5,
                y1: 0.5,
                mode: SnapMode::TopLeftQuarter,
            },
            LayoutZoneSpec {
                x0: 0.5,
                y0: 0.0,
                x1: 1.0,
                y1: 0.5,
                mode: SnapMode::TopRightQuarter,
            },
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.5,
                x1: 0.5,
                y1: 1.0,
                mode: SnapMode::BottomLeftQuarter,
            },
            LayoutZoneSpec {
                x0: 0.5,
                y0: 0.5,
                x1: 1.0,
                y1: 1.0,
                mode: SnapMode::BottomRightQuarter,
            },
        ],
    },
    // 5: Centro prioritario (25 / 50 / 25)
    LayoutCardSpec {
        zones: &[
            LayoutZoneSpec {
                x0: 0.0,
                y0: 0.0,
                x1: 0.25,
                y1: 1.0,
                mode: SnapMode::LeftQuarter,
            },
            LayoutZoneSpec {
                x0: 0.25,
                y0: 0.0,
                x1: 0.75,
                y1: 1.0,
                mode: SnapMode::CenterHalf,
            },
            LayoutZoneSpec {
                x0: 0.75,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                mode: SnapMode::RightQuarter,
            },
        ],
    },
];

const NARROW_CARD_INDICES: [usize; 4] = [0, 1, 3, 4];
const WIDE_CARD_INDICES: [usize; 6] = [0, 1, 2, 3, 4, 5];

pub fn active_card_indices(usable_width: i32) -> &'static [usize] {
    if usable_width < 1024 {
        &NARROW_CARD_INDICES
    } else {
        &WIDE_CARD_INDICES
    }
}

/// Resuelve el `SnapMode` de una sección dentro de la cuadrícula de tarjetas del menú.
pub fn mode_for_zone(usable_width: i32, slot_idx: usize, zone_idx: usize) -> Option<SnapMode> {
    let cards = active_card_indices(usable_width);
    let &card_id = cards.get(slot_idx)?;
    WINDOWS_LAYOUT_CARDS[card_id]
        .zones
        .get(zone_idx)
        .map(|z| z.mode)
}

/// Calcula el rectángulo de previsualización en escritorio para una sección del menú.
pub fn preview_rect_for_zone(
    usable_zone: Rectangle<i32, Logical>,
    slot_idx: usize,
    zone_idx: usize,
) -> Option<Rectangle<i32, Logical>> {
    let mode = mode_for_zone(usable_zone.size.w, slot_idx, zone_idx)?;
    crate::grabs::preview_for_snap(mode, usable_zone)
}

#[derive(Debug, Clone)]
pub struct MaximizeButtonTarget {
    pub window: Window,
    pub surface: WlSurface,
    pub anchor_rect: Rectangle<i32, Logical>,
}

#[derive(Debug, Clone)]
struct HoverCandidate {
    window: Window,
    surface: WlSurface,
    anchor_rect: Rectangle<i32, Logical>,
    started_at: Instant,
}

#[derive(Debug, Clone)]
struct ActiveSnapPopup {
    window: Window,
    surface: WlSurface,
    output_name: String,
    anchor_rect: Rectangle<i32, Logical>,
    card_rect: Rectangle<i32, Logical>,
    usable_zone: Rectangle<i32, Logical>,
    opened_at: Instant,
    hovered_zone: Option<(usize, usize)>,
    pointer_outside_since: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderCacheKey {
    phys_w: i32,
    phys_h: i32,
    scale_milli: i32,
    glass_milli: i32,
    is_dark: bool,
    hovered_zone: Option<(usize, usize)>,
    card_count: usize,
}

#[derive(Debug)]
pub enum SnapLayoutClickResult {
    Ignored,
    Consumed,
    Snap { window: Window, mode: SnapMode },
}

pub struct SnapLayoutsState {
    candidate: Option<HoverCandidate>,
    active: Option<ActiveSnapPopup>,
    closed_dirty: bool,
    last_pointer: Option<Point<f64, Logical>>,
    cache_key: Option<RenderCacheKey>,
    render_buffer: Option<MemoryRenderBuffer>,
    render_buffer_size: (i32, i32),
    mask_buffer: Vec<u8>,
    mask_size: (i32, i32),
}

impl Default for SnapLayoutsState {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapLayoutsState {
    pub fn new() -> Self {
        Self {
            candidate: None,
            active: None,
            closed_dirty: false,
            last_pointer: None,
            cache_key: None,
            render_buffer: None,
            render_buffer_size: (0, 0),
            mask_buffer: Vec::new(),
            mask_size: (0, 0),
        }
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn has_pending_hover(&self) -> bool {
        self.candidate.is_some()
    }

    pub fn close(&mut self) {
        self.candidate = None;
        if self.active.take().is_some() {
            self.closed_dirty = true;
        }
    }

    pub fn take_closed_dirty(&mut self) -> bool {
        std::mem::take(&mut self.closed_dirty)
    }

    pub fn should_render_on_output(&self, output_name: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|p| p.output_name == output_name)
    }

    /// Devuelve el rectángulo de previsualización (estilo Aero Snap) en coordenadas lógicas globales
    /// para la sección actualmente bajo el cursor dentro del menú en este monitor.
    pub fn hovered_preview_rect(&self, output_name: &str) -> Option<Rectangle<i32, Logical>> {
        let active = self.active.as_ref()?;
        if active.output_name != output_name {
            return None;
        }
        let (slot_idx, zone_idx) = active.hovered_zone?;
        preview_rect_for_zone(active.usable_zone, slot_idx, zone_idx)
    }

    /// Devuelve `true` si el punto está dentro de la tarjeta activa del menú flotante.
    pub fn contains_point(&self, point: Point<f64, Logical>) -> bool {
        self.active
            .as_ref()
            .is_some_and(|p| p.card_rect.to_f64().contains(point))
    }

    /// Actualiza la detección de hover sobre botones de maximizar y sobre el popup abierto.
    pub fn update_pointer_motion(
        &mut self,
        space: &Space<Window>,
        point: Point<f64, Logical>,
        occluded: bool,
        buttons_right: bool,
        now: Instant,
    ) {
        self.last_pointer = Some(point);
        if occluded {
            self.candidate = None;
            if let Some(ref mut active) = self.active {
                if active.pointer_outside_since.is_none() {
                    active.pointer_outside_since = Some(now);
                }
            }
            return;
        }

        let hovered_btn = detect_maximize_button_under(space, point, buttons_right);

        if let Some(ref mut active) = self.active {
            // Si el usuario pasó el cursor al botón de maximizar de OTRA ventana (fuera del popup actual),
            // cerramos el actual y comenzamos la espera en la nueva ventana.
            if !active.card_rect.to_f64().contains(point) {
                if let Some(ref target) = hovered_btn {
                    if target.surface != active.surface {
                        self.active = None;
                        self.closed_dirty = true;
                        self.candidate = Some(HoverCandidate {
                            window: target.window.clone(),
                            surface: target.surface.clone(),
                            anchor_rect: target.anchor_rect,
                            started_at: now,
                        });
                        return;
                    }
                }
            }

            let in_keepalive = is_pointer_in_keepalive(active.anchor_rect, active.card_rect, point);
            if in_keepalive {
                active.pointer_outside_since = None;
                active.hovered_zone =
                    hit_test_popup_zone(active.card_rect, active.usable_zone.size.w, point);
            } else {
                active.hovered_zone = None;
                if active.pointer_outside_since.is_none() {
                    active.pointer_outside_since = Some(now);
                }
            }
            self.candidate = None;
            return;
        }

        match hovered_btn {
            Some(target) => {
                let keep_existing = self
                    .candidate
                    .as_ref()
                    .is_some_and(|c| c.surface == target.surface);
                if keep_existing {
                    if let Some(ref mut c) = self.candidate {
                        c.anchor_rect = target.anchor_rect;
                    }
                } else {
                    self.candidate = Some(HoverCandidate {
                        window: target.window,
                        surface: target.surface,
                        anchor_rect: target.anchor_rect,
                        started_at: now,
                    });
                }
            }
            None => {
                let keep_jitter = self.candidate.as_ref().is_some_and(|cand| {
                    let hysteresis = Rectangle::new(
                        Point::from((cand.anchor_rect.loc.x - 4, cand.anchor_rect.loc.y - 4)),
                        Size::from((cand.anchor_rect.size.w + 8, cand.anchor_rect.size.h + 8)),
                    );
                    hysteresis.to_f64().contains(point)
                });
                if !keep_jitter {
                    self.candidate = None;
                }
            }
        }
    }

    /// Avanza temporizadores de apertura/cierre por hover y valida que la ventana siga viva y mapeada.
    pub fn tick<F>(&mut self, space: &Space<Window>, now: Instant, resolve_output: F)
    where
        F: Fn(&Window, Point<i32, Logical>) -> Option<(String, Rectangle<i32, Logical>)>,
    {
        if let Some(ref active) = self.active {
            let still_valid = active.surface.alive()
                && space.element_location(&active.window).is_some()
                && !is_window_fullscreen(&active.window);
            let expired = active
                .pointer_outside_since
                .is_some_and(|t| now.saturating_duration_since(t) >= HOVER_CLOSE_DELAY);
            if !still_valid || expired {
                self.active = None;
                self.closed_dirty = true;
            }
        }

        if self.active.is_some() {
            return;
        }

        if let Some(cand) = self.candidate.clone() {
            if !cand.surface.alive() || space.element_location(&cand.window).is_none() {
                self.candidate = None;
                return;
            }
            if now.saturating_duration_since(cand.started_at) >= HOVER_OPEN_DELAY {
                let anchor_center = Point::from((
                    cand.anchor_rect.loc.x + cand.anchor_rect.size.w / 2,
                    cand.anchor_rect.loc.y + cand.anchor_rect.size.h / 2,
                ));
                if let Some((output_name, usable_zone)) =
                    resolve_output(&cand.window, anchor_center)
                {
                    let card_rect = compute_popup_rect(cand.anchor_rect, usable_zone);
                    let hovered_zone = self
                        .last_pointer
                        .and_then(|pt| hit_test_popup_zone(card_rect, usable_zone.size.w, pt));
                    self.active = Some(ActiveSnapPopup {
                        window: cand.window,
                        surface: cand.surface,
                        output_name,
                        anchor_rect: cand.anchor_rect,
                        card_rect,
                        usable_zone,
                        opened_at: now,
                        hovered_zone,
                        pointer_outside_since: None,
                    });
                    self.candidate = None;
                }
            }
        }
    }

    pub fn handle_pointer_button(
        &mut self,
        point: Point<f64, Logical>,
        button_code: u32,
        is_pressed: bool,
    ) -> SnapLayoutClickResult {
        let Some(ref mut active) = self.active else {
            return SnapLayoutClickResult::Ignored;
        };

        if !active.card_rect.to_f64().contains(point) {
            if is_pressed {
                self.close();
            }
            return SnapLayoutClickResult::Ignored;
        }

        if !is_pressed || button_code != 0x110 {
            return SnapLayoutClickResult::Consumed;
        }

        if let Some((slot_idx, zone_idx)) =
            hit_test_popup_zone(active.card_rect, active.usable_zone.size.w, point)
        {
            if let Some(mode) = mode_for_zone(active.usable_zone.size.w, slot_idx, zone_idx) {
                let target_window = active.window.clone();
                self.close();
                return SnapLayoutClickResult::Snap {
                    window: target_window,
                    mode,
                };
            }
        }

        SnapLayoutClickResult::Consumed
    }

    pub fn handle_escape(&mut self) -> bool {
        if self.active.is_some() {
            self.close();
            true
        } else {
            false
        }
    }

    /// Prepara el buffer visual Cairo + máscara de desenfoque acrílico en coordenadas locales al monitor.
    pub fn prepare_frame(
        &mut self,
        output_name: &str,
        output_geo: Rectangle<i32, Logical>,
        scale: f64,
        is_dark: bool,
    ) -> Option<(Rectangle<i32, Physical>, Option<&[u8]>, (i32, i32), f32)> {
        let active = self.active.as_ref()?;
        if active.output_name != output_name {
            return None;
        }

        let card_rect = active.card_rect;
        let local_card = localize_popup_rect(card_rect, output_geo);
        let card_phys_rect: Rectangle<i32, Physical> = local_card.to_physical_precise_round(scale);
        let phys_w = card_phys_rect.size.w.max(2);
        let phys_h = card_phys_rect.size.h.max(2);
        let cards = active_card_indices(active.usable_zone.size.w);

        let elapsed = active.opened_at.elapsed().as_secs_f32();
        let t = (elapsed / OPEN_ANIM_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let alpha = 1.0 - (1.0 - t).powi(3);

        let glass_scale = malus_de_common::config::current_liquid_glass_scale();
        let key = RenderCacheKey {
            phys_w,
            phys_h,
            scale_milli: (scale * 1000.0).round() as i32,
            glass_milli: (glass_scale * 1000.0).round() as i32,
            is_dark,
            hovered_zone: active.hovered_zone,
            card_count: cards.len(),
        };

        let needs_rebuild = self.cache_key.as_ref() != Some(&key)
            || self.render_buffer.is_none()
            || self.mask_size != (phys_w, phys_h);

        if needs_rebuild {
            self.rebuild_buffers(
                phys_w,
                phys_h,
                card_rect.size,
                scale,
                is_dark,
                cards,
                active.hovered_zone,
            )?;
            self.cache_key = Some(key);
        }

        Some((
            card_phys_rect,
            Some(self.mask_buffer.as_slice()),
            self.mask_size,
            alpha,
        ))
    }

    pub fn create_render_element(
        &self,
        renderer: &mut GlesRenderer,
        card_loc_phys: Point<i32, Physical>,
        alpha: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let buf = self.render_buffer.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            card_loc_phys.to_f64(),
            buf,
            Some(alpha),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }

    #[allow(clippy::too_many_arguments)]
    fn rebuild_buffers(
        &mut self,
        phys_w: i32,
        phys_h: i32,
        card_size_log: Size<i32, Logical>,
        scale: f64,
        is_dark: bool,
        cards: &[usize],
        hovered_zone: Option<(usize, usize)>,
    ) -> Option<()> {
        let mut surface = ImageSurface::create(Format::ARgb32, phys_w, phys_h).ok()?;
        let mut mask_surface = ImageSurface::create(Format::ARgb32, phys_w, phys_h).ok()?;

        {
            let ctx = Context::new(&surface).ok()?;
            ctx.scale(scale, scale);
            ctx.set_operator(Operator::Source);
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = ctx.paint();
            ctx.set_operator(Operator::Over);

            let cw = card_size_log.w as f64;
            let ch = card_size_log.h as f64;

            // Superficie Liquid Glass del popup (la sombra difusa se proyecta por GPU con WindowShadowPipeline)
            let glass_scale = malus_de_common::config::current_liquid_glass_scale();
            let a_top =
                malus_de_common::config::scaled_liquid_glass_alpha(0.78, glass_scale, 0.16, 0.95);
            let a_bot =
                malus_de_common::config::scaled_liquid_glass_alpha(0.84, glass_scale, 0.20, 0.96);
            append_squircle_path(
                &ctx,
                0.0,
                0.0,
                cw,
                ch,
                POPUP_CORNER_RADIUS,
                SQUIRCLE_EXPONENT,
            );
            let bg_grad = LinearGradient::new(0.0, 0.0, 0.0, ch);
            if is_dark {
                bg_grad.add_color_stop_rgba(0.0, 0.17, 0.17, 0.19, a_top);
                bg_grad.add_color_stop_rgba(1.0, 0.12, 0.12, 0.14, a_bot);
            } else {
                bg_grad.add_color_stop_rgba(0.0, 0.98, 0.98, 0.99, a_top);
                bg_grad.add_color_stop_rgba(1.0, 0.93, 0.94, 0.96, a_bot);
            }
            let _ = ctx.set_source(&bg_grad);
            let _ = ctx.fill();

            // Borde sutil especular contenido dentro de la silueta exacta
            append_squircle_path(
                &ctx,
                0.5,
                0.5,
                (cw - 1.0).max(2.0),
                (ch - 1.0).max(2.0),
                (POPUP_CORNER_RADIUS - 0.5).max(1.0),
                SQUIRCLE_EXPONENT,
            );
            ctx.set_line_width(1.0);
            if is_dark {
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.18);
            } else {
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.75);
            }
            let _ = ctx.stroke();

            // Dibujar cada tarjeta de diseño (3×2 o 2×2)
            let cols = if cards.len() <= 4 { 2 } else { 3 };
            for (slot_idx, &card_id) in cards.iter().enumerate() {
                let col = (slot_idx % cols) as f64;
                let row = (slot_idx / cols) as f64;
                let card_x = POPUP_PADDING + col * (CARD_WIDTH + CARD_GAP);
                let card_y = POPUP_PADDING + row * (CARD_HEIGHT + CARD_GAP);

                let card = &WINDOWS_LAYOUT_CARDS[card_id];
                for (zone_idx, zone) in card.zones.iter().enumerate() {
                    let zx = card_x + zone.x0 * CARD_WIDTH + ZONE_GAP * 0.5;
                    let zy = card_y + zone.y0 * CARD_HEIGHT + ZONE_GAP * 0.5;
                    let zw = ((zone.x1 - zone.x0) * CARD_WIDTH - ZONE_GAP).max(4.0);
                    let zh = ((zone.y1 - zone.y0) * CARD_HEIGHT - ZONE_GAP).max(4.0);
                    let is_hovered = hovered_zone == Some((slot_idx, zone_idx));

                    append_rounded_rect_path(&ctx, zx, zy, zw, zh, ZONE_RADIUS);
                    if is_hovered {
                        let sel_grad = LinearGradient::new(zx, zy, zx, zy + zh);
                        sel_grad.add_color_stop_rgba(0.0, 0.14, 0.56, 1.0, 0.96);
                        sel_grad.add_color_stop_rgba(1.0, 0.00, 0.45, 0.94, 0.96);
                        let _ = ctx.set_source(&sel_grad);
                        let _ = ctx.fill_preserve();
                        ctx.set_line_width(1.1);
                        ctx.set_source_rgba(1.0, 1.0, 1.0, 0.55);
                        let _ = ctx.stroke();
                    } else {
                        if is_dark {
                            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.13);
                        } else {
                            ctx.set_source_rgba(0.08, 0.10, 0.14, 0.10);
                        }
                        let _ = ctx.fill_preserve();
                        ctx.set_line_width(1.0);
                        if is_dark {
                            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                        } else {
                            ctx.set_source_rgba(0.08, 0.10, 0.14, 0.22);
                        }
                        let _ = ctx.stroke();
                    }
                }
            }
        }

        {
            let mctx = Context::new(&mask_surface).ok()?;
            mctx.scale(scale, scale);
            mctx.set_operator(Operator::Source);
            mctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = mctx.paint();
            mctx.set_operator(Operator::Over);

            append_squircle_path(
                &mctx,
                0.0,
                0.0,
                card_size_log.w as f64,
                card_size_log.h as f64,
                POPUP_CORNER_RADIUS,
                SQUIRCLE_EXPONENT,
            );
            mctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            let _ = mctx.fill();
        }

        surface.flush();
        mask_surface.flush();

        let mask_data = mask_surface.data().ok()?;
        self.mask_buffer = mask_data.to_vec();
        self.mask_size = (phys_w, phys_h);

        let data = surface.data().ok()?;
        if self.render_buffer.is_none() || self.render_buffer_size != (phys_w, phys_h) {
            self.render_buffer = Some(MemoryRenderBuffer::new(
                Fourcc::Argb8888,
                Size::from((phys_w, phys_h)),
                1,
                smithay::utils::Transform::Normal,
                None,
            ));
            self.render_buffer_size = (phys_w, phys_h);
        }
        if let Some(ref mut buf) = self.render_buffer {
            let mut rctx = buf.render();
            let _ = rctx.draw(|dst| {
                if dst.len() == data.len() {
                    dst.copy_from_slice(&data);
                }
                Result::<_, ()>::Ok(vec![Rectangle::from_size((phys_w, phys_h).into())])
            });
        }

        Some(())
    }
}

pub fn compute_popup_size(usable_width: i32) -> Size<i32, Logical> {
    let cards = active_card_indices(usable_width);
    let cols = if cards.len() <= 4 { 2.0 } else { 3.0 };
    let w = (POPUP_PADDING * 2.0 + cols * CARD_WIDTH + (cols - 1.0) * CARD_GAP).round() as i32;
    let h = (POPUP_PADDING * 2.0 + 2.0 * CARD_HEIGHT + CARD_GAP).round() as i32;
    Size::from((w, h))
}

pub fn localize_popup_rect(
    card_rect: Rectangle<i32, Logical>,
    output_geo: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((
            card_rect.loc.x - output_geo.loc.x,
            card_rect.loc.y - output_geo.loc.y,
        )),
        card_rect.size,
    )
}

pub fn compute_popup_rect(
    anchor_rect: Rectangle<i32, Logical>,
    usable_zone: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let size = compute_popup_size(usable_zone.size.w);
    let margin = 8;
    let min_x = usable_zone.loc.x + margin;
    let max_x = (usable_zone.loc.x + usable_zone.size.w - size.w - margin).max(min_x);
    let anchor_center_x = anchor_rect.loc.x + anchor_rect.size.w / 2;
    let x = (anchor_center_x - size.w / 2).clamp(min_x, max_x);

    let below_y = anchor_rect.loc.y + anchor_rect.size.h + 8;
    let max_y = usable_zone.loc.y + usable_zone.size.h - size.h - margin;
    let y = if below_y <= max_y {
        below_y
    } else {
        (anchor_rect.loc.y - size.h - 8).max(usable_zone.loc.y + margin)
    };

    Rectangle::new(Point::from((x, y)), size)
}

pub fn hit_test_popup_zone(
    card_rect: Rectangle<i32, Logical>,
    usable_width: i32,
    point: Point<f64, Logical>,
) -> Option<(usize, usize)> {
    let lx = point.x - card_rect.loc.x as f64;
    let ly = point.y - card_rect.loc.y as f64;
    if lx < 0.0 || ly < 0.0 || lx > card_rect.size.w as f64 || ly > card_rect.size.h as f64 {
        return None;
    }

    let cards = active_card_indices(usable_width);
    let cols = if cards.len() <= 4 { 2 } else { 3 };
    for (slot_idx, &card_id) in cards.iter().enumerate() {
        let col = (slot_idx % cols) as f64;
        let row = (slot_idx / cols) as f64;
        let card_x = POPUP_PADDING + col * (CARD_WIDTH + CARD_GAP);
        let card_y = POPUP_PADDING + row * (CARD_HEIGHT + CARD_GAP);

        if lx < card_x || lx > card_x + CARD_WIDTH || ly < card_y || ly > card_y + CARD_HEIGHT {
            continue;
        }

        let card = &WINDOWS_LAYOUT_CARDS[card_id];
        for (zone_idx, zone) in card.zones.iter().enumerate() {
            let zx = card_x + zone.x0 * CARD_WIDTH;
            let zy = card_y + zone.y0 * CARD_HEIGHT;
            let zw = (zone.x1 - zone.x0) * CARD_WIDTH;
            let zh = (zone.y1 - zone.y0) * CARD_HEIGHT;
            if lx >= zx && lx <= zx + zw && ly >= zy && ly <= zy + zh {
                return Some((slot_idx, zone_idx));
            }
        }
    }

    None
}

fn is_pointer_in_keepalive(
    anchor_rect: Rectangle<i32, Logical>,
    card_rect: Rectangle<i32, Logical>,
    point: Point<f64, Logical>,
) -> bool {
    let pad = 6.0;
    let card_expanded = Rectangle::new(
        Point::from((card_rect.loc.x as f64 - pad, card_rect.loc.y as f64 - pad)),
        Size::from((
            card_rect.size.w as f64 + pad * 2.0,
            card_rect.size.h as f64 + pad * 2.0,
        )),
    );
    if card_expanded.contains(point) {
        return true;
    }

    let anchor_expanded = Rectangle::new(
        Point::from((
            anchor_rect.loc.x as f64 - pad,
            anchor_rect.loc.y as f64 - pad,
        )),
        Size::from((
            anchor_rect.size.w as f64 + pad * 2.0,
            anchor_rect.size.h as f64 + pad * 2.0,
        )),
    );
    if anchor_expanded.contains(point) {
        return true;
    }

    // Puente continuo entre el botón de maximizar y el menú para evitar cierres
    // al deslizar el cursor en diagonal hacia alguna de las tarjetas.
    let bridge_left = (card_rect.loc.x as f64).min(anchor_rect.loc.x as f64 - 16.0);
    let bridge_right = (card_rect.loc.x as f64 + card_rect.size.w as f64)
        .max(anchor_rect.loc.x as f64 + anchor_rect.size.w as f64 + 16.0);
    let bridge_top = anchor_rect.loc.y.min(card_rect.loc.y + card_rect.size.h) as f64;
    let bridge_bottom = (anchor_rect.loc.y + anchor_rect.size.h).max(card_rect.loc.y) as f64;

    point.x >= bridge_left
        && point.x <= bridge_right
        && point.y >= bridge_top
        && point.y <= bridge_bottom
}

fn is_window_fullscreen(window: &Window) -> bool {
    window
        .toplevel()
        .is_some_and(|t| t.current_state().states.contains(XdgState::Fullscreen))
}

fn detect_maximize_button_under(
    space: &Space<Window>,
    point: Point<f64, Logical>,
    buttons_right: bool,
) -> Option<MaximizeButtonTarget> {
    for window in space.elements().rev() {
        let Some(loc) = space.element_location(window) else {
            continue;
        };
        let geo = window.geometry();
        if geo.size.w <= 0 || geo.size.h <= 0 {
            continue;
        }
        let Some(surf) = window.wl_surface() else {
            continue;
        };
        let is_top_level_app = window.toplevel().is_some_and(|t| t.parent().is_none())
            && geo.size.w >= 260
            && geo.size.h >= 140
            && !is_window_fullscreen(window);

        if window_uses_ssd(window) {
            let tb_x = loc.x as f64;
            let tb_y = (loc.y - crate::ssd::SSD_TITLEBAR_HEIGHT) as f64;
            let tb_w = geo.size.w as f64;
            let tb_h = crate::ssd::SSD_TITLEBAR_HEIGHT as f64;
            if point.x >= tb_x
                && point.x <= tb_x + tb_w
                && point.y >= tb_y
                && point.y <= tb_y + tb_h
            {
                if is_top_level_app {
                    let (_, _, max_x) = crate::ssd::button_positions(tb_w, buttons_right);
                    let cx = tb_x + max_x;
                    let cy = tb_y + crate::ssd::BTN_Y;
                    if (point.x - cx).abs() <= SSD_MAX_HALF_W
                        && (point.y - cy).abs() <= SSD_MAX_HALF_H
                    {
                        return Some(MaximizeButtonTarget {
                            window: window.clone(),
                            surface: surf.into_owned(),
                            anchor_rect: Rectangle::new(
                                Point::from((
                                    loc.x + (max_x - 7.0).round() as i32,
                                    loc.y - crate::ssd::SSD_TITLEBAR_HEIGHT + 12,
                                )),
                                Size::from((14, 14)),
                            ),
                        });
                    }
                }
                return None;
            }
        }

        let body_rect = Rectangle::new(loc, geo.size).to_f64();
        if body_rect.contains(point) {
            if is_top_level_app && !window_uses_ssd(window) {
                let lx = point.x - loc.x as f64;
                let ly = point.y - loc.y as f64;
                if buttons_right {
                    let rx = geo.size.w as f64 - lx;
                    if (26.0..=58.0).contains(&rx)
                        && (CSD_MAX_MIN_Y..=CSD_MAX_MAX_Y).contains(&ly)
                    {
                        let (ax, ay) = if is_gnome_window(window) {
                            (loc.x + geo.size.w - 48, loc.y + 15)
                        } else {
                            (loc.x + geo.size.w - 46, loc.y + 13)
                        };
                        return Some(MaximizeButtonTarget {
                            window: window.clone(),
                            surface: surf.into_owned(),
                            anchor_rect: Rectangle::new(
                                Point::from((ax, ay)),
                                Size::from((14, 14)),
                            ),
                        });
                    }
                } else if (CSD_MAX_MIN_X..=CSD_MAX_MAX_X).contains(&lx)
                    && (CSD_MAX_MIN_Y..=CSD_MAX_MAX_Y).contains(&ly)
                {
                    let (ax, ay) = if is_gnome_window(window) {
                        (loc.x + 62, loc.y + 15)
                    } else {
                        (loc.x + 56, loc.y + 13)
                    };
                    return Some(MaximizeButtonTarget {
                        window: window.clone(),
                        surface: surf.into_owned(),
                        anchor_rect: Rectangle::new(Point::from((ax, ay)), Size::from((14, 14))),
                    });
                }
            }
            return None;
        }
    }
    None
}

fn append_rounded_rect_path(ctx: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    ctx.arc(x + r, y + h - r, r, FRAC_PI_2, std::f64::consts::PI);
    ctx.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * FRAC_PI_2);
    ctx.close_path();
}

fn append_squircle_path(ctx: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64, exponent: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.5 {
        ctx.rectangle(x, y, w, h);
        return;
    }

    let steps = 14usize;
    let inv_n = 1.0 / exponent;
    ctx.new_sub_path();
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);

    let corner_point = |cx: f64, cy: f64, sx: f64, sy: f64, t: f64| -> (f64, f64) {
        let angle = t * FRAC_PI_2;
        let dx = angle.cos().abs().powf(2.0 * inv_n) * r * sx;
        let dy = angle.sin().abs().powf(2.0 * inv_n) * r * sy;
        (cx + dx, cy + dy)
    };

    // Superior derecha
    for i in 0..=steps {
        let (px, py) = corner_point(x + w - r, y + r, 1.0, -1.0, 1.0 - (i as f64 / steps as f64));
        ctx.line_to(px, py);
    }
    ctx.line_to(x + w, y + h - r);

    // Inferior derecha
    for i in 0..=steps {
        let (px, py) = corner_point(x + w - r, y + h - r, 1.0, 1.0, i as f64 / steps as f64);
        ctx.line_to(px, py);
    }
    ctx.line_to(x + r, y + h);

    // Inferior izquierda
    for i in 0..=steps {
        let (px, py) = corner_point(x + r, y + h - r, -1.0, 1.0, 1.0 - (i as f64 / steps as f64));
        ctx.line_to(px, py);
    }
    ctx.line_to(x, y + r);

    // Superior izquierda
    for i in 0..=steps {
        let (px, py) = corner_point(x + r, y + r, -1.0, -1.0, i as f64 / steps as f64);
        ctx.line_to(px, py);
    }
    ctx.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_layout_cards_contain_expected_sections() {
        assert_eq!(WINDOWS_LAYOUT_CARDS.len(), 6);
        assert_eq!(WINDOWS_LAYOUT_CARDS[0].zones.len(), 2); // 50/50
        assert_eq!(WINDOWS_LAYOUT_CARDS[1].zones.len(), 2); // 67/33
        assert_eq!(WINDOWS_LAYOUT_CARDS[2].zones.len(), 3); // 33/33/33
        assert_eq!(WINDOWS_LAYOUT_CARDS[3].zones.len(), 3); // 50 + 2 quarters
        assert_eq!(WINDOWS_LAYOUT_CARDS[4].zones.len(), 4); // 4 quarters
        assert_eq!(WINDOWS_LAYOUT_CARDS[5].zones.len(), 3); // 25/50/25

        assert_eq!(active_card_indices(1920).len(), 6);
        assert_eq!(active_card_indices(800).len(), 4);
    }

    #[test]
    fn hit_test_popup_zone_maps_all_six_cards() {
        let usable = Rectangle::new((0, 32).into(), (1920, 1000).into());
        let anchor = Rectangle::new((200, 100).into(), (18, 18).into());
        let rect = compute_popup_rect(anchor, usable);

        // Tarjeta 0 (Mitad izq / der)
        let card0_left = Point::from((
            rect.loc.x as f64 + POPUP_PADDING + 10.0,
            rect.loc.y as f64 + POPUP_PADDING + 20.0,
        ));
        assert_eq!(
            hit_test_popup_zone(rect, usable.size.w, card0_left),
            Some((0, 0))
        );

        // Tarjeta 4 (Cuadrícula 2×2 -> cuarto inferior derecho)
        let col1_x =
            rect.loc.x as f64 + POPUP_PADDING + (CARD_WIDTH + CARD_GAP) + CARD_WIDTH * 0.75;
        let row1_y =
            rect.loc.y as f64 + POPUP_PADDING + (CARD_HEIGHT + CARD_GAP) + CARD_HEIGHT * 0.75;
        assert_eq!(
            hit_test_popup_zone(rect, usable.size.w, Point::from((col1_x, row1_y))),
            Some((4, 3))
        );

        // Tarjeta 5 (25 / 50 / 25 -> centro ancho)
        let col2_x =
            rect.loc.x as f64 + POPUP_PADDING + 2.0 * (CARD_WIDTH + CARD_GAP) + CARD_WIDTH * 0.5;
        assert_eq!(
            hit_test_popup_zone(rect, usable.size.w, Point::from((col2_x, row1_y))),
            Some((5, 1))
        );
    }

    #[test]
    fn compute_popup_rect_clamps_to_screen_edges_and_flips_when_near_bottom() {
        let usable = Rectangle::new((0, 32).into(), (1280, 720).into());
        let near_bottom = Rectangle::new((1250, 720).into(), (14, 14).into());
        let rect = compute_popup_rect(near_bottom, usable);

        assert!(rect.loc.x + rect.size.w <= usable.loc.x + usable.size.w - 8);
        assert!(rect.loc.y < near_bottom.loc.y);
    }

    #[test]
    fn compute_popup_rect_centers_directly_below_maximize_button_and_localizes() {
        let output_geo = Rectangle::new((0, 420).into(), (2304, 1296).into());
        let usable = Rectangle::new((0, 456).into(), (2304, 1260).into());
        let anchor = Rectangle::new((340, 740).into(), (14, 14).into());
        let card_rect = compute_popup_rect(anchor, usable);

        assert_eq!(
            card_rect.loc.x + card_rect.size.w / 2,
            anchor.loc.x + anchor.size.w / 2
        );
        assert_eq!(card_rect.loc.y, anchor.loc.y + anchor.size.h + 8);

        let local_card = localize_popup_rect(card_rect, output_geo);
        assert_eq!(local_card.loc.y, card_rect.loc.y - 420);
    }

    #[test]
    fn preview_rect_for_zone_matches_manual_aero_snap_preview() {
        let usable = Rectangle::new((0, 32).into(), (1920, 1000).into());

        let left_half = preview_rect_for_zone(usable, 0, 0);
        assert_eq!(
            left_half,
            crate::grabs::preview_for_snap(SnapMode::LeftHalf, usable)
        );

        let bottom_right = preview_rect_for_zone(usable, 4, 3);
        assert_eq!(
            bottom_right,
            crate::grabs::preview_for_snap(SnapMode::BottomRightQuarter, usable)
        );

        let center_half = preview_rect_for_zone(usable, 5, 1);
        assert_eq!(
            center_half,
            crate::grabs::preview_for_snap(SnapMode::CenterHalf, usable)
        );
        assert_eq!(preview_rect_for_zone(usable, 99, 0), None);
    }
}
