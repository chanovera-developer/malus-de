//! Decoraciones del Lado del Servidor (Server-Side Decorations / SSD) estilo macOS para Malus DE.
//!
//! Renderiza una barra de título nativa estilo macOS (con los 3 botones semáforo a la izquierda:
//! cerrar, minimizar y maximizar, título centrado y soporte de arrastre y doble clic) para aquellas
//! aplicaciones Wayland que solicitan `zxdg_toplevel_decoration_v1::Mode::ServerSide` (como apps Winit,
//! SDL, Qt o herramientas sin barra propia).

use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface, LinearGradient};
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
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Physical, Point, Size},
};
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::time::Instant;

/// Altura lógica de la barra de título SSD estilo macOS (en píxeles lógicos)
pub const SSD_TITLEBAR_HEIGHT: i32 = 38;

/// Posiciones lógicas de los tres botones semáforo (relativas a la esquina superior izquierda de la barra)
pub const BTN_Y: f64 = 19.0;
pub const BTN_CLOSE_X: f64 = 20.0;
const BTN_MIN_X: f64 = 40.0;
pub const BTN_MAX_X: f64 = 60.0;
const BTN_RADIUS: f64 = 6.5;
const BTN_HIT_RADIUS: f64 = 8.0;

/// Devuelve las coordenadas X lógicas `(close_x, min_x, max_x)` de los tres botones semáforo:
/// - Izquierda (`buttons_right = false`): cerrar (20), minimizar (40), maximizar (60).
/// - Derecha (`buttons_right = true`): minimizar (`width - 60`), maximizar (`width - 40`), cerrar (`width - 20`).
pub fn button_positions(width: f64, buttons_right: bool) -> (f64, f64, f64) {
    if buttons_right {
        (
            (width - BTN_CLOSE_X).max(BTN_CLOSE_X),
            (width - BTN_MAX_X).max(BTN_CLOSE_X),
            (width - BTN_MIN_X).max(BTN_CLOSE_X),
        )
    } else {
        (BTN_CLOSE_X, BTN_MIN_X, BTN_MAX_X)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SsdButton {
    Close,
    Minimize,
    Maximize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsdHit {
    Button(SsdButton),
    Titlebar,
}

/// Realiza hit-testing en coordenadas lógicas locales a la barra de título `(0..width, 0..SSD_TITLEBAR_HEIGHT)`.
pub fn hit_test_titlebar_with_layout(
    local_x: f64,
    local_y: f64,
    width: f64,
    buttons_right: bool,
) -> Option<SsdHit> {
    if local_x < 0.0 || local_x > width || local_y < 0.0 || local_y > SSD_TITLEBAR_HEIGHT as f64 {
        return None;
    }

    let (close_x, min_x, max_x) = button_positions(width, buttons_right);
    let dy = local_y - BTN_Y;
    if (local_x - close_x).hypot(dy) <= BTN_HIT_RADIUS {
        return Some(SsdHit::Button(SsdButton::Close));
    }
    if (local_x - min_x).hypot(dy) <= BTN_HIT_RADIUS {
        return Some(SsdHit::Button(SsdButton::Minimize));
    }
    if (local_x - max_x).hypot(dy) <= BTN_HIT_RADIUS {
        return Some(SsdHit::Button(SsdButton::Maximize));
    }

    Some(SsdHit::Titlebar)
}

/// Realiza hit-testing en coordenadas lógicas locales asumiendo el diseño izquierdo predeterminado.
pub fn hit_test_titlebar(local_x: f64, local_y: f64, width: f64) -> Option<SsdHit> {
    hit_test_titlebar_with_layout(local_x, local_y, width, false)
}

/// Determina si el cursor está sobre el grupo de los 3 botones semáforo (para revelar todos los glifos).
pub fn is_in_traffic_lights_group_with_layout(
    local_x: f64,
    local_y: f64,
    width: f64,
    buttons_right: bool,
) -> bool {
    let (close_x, min_x, max_x) = button_positions(width, buttons_right);
    let left_x = close_x.min(min_x).min(max_x);
    let right_x = close_x.max(min_x).max(max_x);
    local_x >= (left_x - BTN_HIT_RADIUS - 2.0)
        && local_x <= (right_x + BTN_HIT_RADIUS + 2.0)
        && local_y >= (BTN_Y - BTN_HIT_RADIUS - 2.0)
        && local_y <= (BTN_Y + BTN_HIT_RADIUS + 2.0)
}

/// Determina si el cursor está sobre el grupo izquierdo de los 3 botones semáforo.
pub fn is_in_traffic_lights_group(local_x: f64, local_y: f64) -> bool {
    is_in_traffic_lights_group_with_layout(local_x, local_y, 800.0, false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    width_phys: i32,
    height_phys: i32,
    scale_milli: i32,
    title: String,
    is_active: bool,
    is_dark: bool,
    buttons_right: bool,
    group_hovered: bool,
    hovered_button: Option<SsdButton>,
}

struct CachedTitlebar {
    key: CacheKey,
    buffer: MemoryRenderBuffer,
}

#[derive(Default)]
pub struct SsdManager {
    cache: HashMap<WlSurface, CachedTitlebar>,
    pub hovered_surface: Option<WlSurface>,
    pub group_hovered: bool,
    pub hovered_button: Option<SsdButton>,
    pub last_titlebar_click: Option<(WlSurface, Instant, Point<f64, Logical>)>,
    /// True cuando los botones semáforo se muestran del lado derecho (minimizar, maximizar, cerrar).
    pub buttons_right: bool,
    /// True cuando el puntero está sobre un borde de redimensión o barra SSD del compositor
    pub cursor_override: bool,
    /// True mientras un grab interactivo del compositor (ResizeSurfaceGrab / MoveSurfaceGrab) está activo
    pub grab_active: bool,
}

impl SsdManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_layout(buttons_right: bool) -> Self {
        Self {
            buttons_right,
            ..Self::default()
        }
    }

    pub fn set_buttons_right(&mut self, buttons_right: bool) {
        if self.buttons_right != buttons_right {
            self.buttons_right = buttons_right;
            self.cache.clear();
            self.group_hovered = false;
            self.hovered_button = None;
        }
    }

    pub fn retain_alive(&mut self) {
        self.cache.retain(|surf, _| surf.alive());
        if self
            .hovered_surface
            .as_ref()
            .is_some_and(|surf| !surf.alive())
        {
            self.hovered_surface = None;
            self.group_hovered = false;
            self.hovered_button = None;
        }
    }

    /// Actualiza el estado de hover sobre las barras de título SSD.
    pub fn update_hover(
        &mut self,
        surface: Option<WlSurface>,
        local_x: f64,
        local_y: f64,
        width: f64,
    ) {
        if let Some(surf) = surface {
            let group =
                is_in_traffic_lights_group_with_layout(local_x, local_y, width, self.buttons_right);
            let btn = match hit_test_titlebar_with_layout(local_x, local_y, width, self.buttons_right)
            {
                Some(SsdHit::Button(b)) => Some(b),
                _ => None,
            };
            self.hovered_surface = Some(surf);
            self.group_hovered = group;
            self.hovered_button = btn;
        } else {
            self.hovered_surface = None;
            self.group_hovered = false;
            self.hovered_button = None;
        }
    }

    /// Obtiene o genera el elemento de renderizado GPU para la barra de título SSD de una ventana.
    #[allow(clippy::too_many_arguments)]
    pub fn render_element_for_window(
        &mut self,
        renderer: &mut GlesRenderer,
        surface: &WlSurface,
        loc_phys: Point<i32, Physical>,
        width_phys: i32,
        height_phys: i32,
        scale: f64,
        title: &str,
        is_active: bool,
        is_dark: bool,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        if width_phys <= 4 || height_phys <= 4 {
            return None;
        }

        let is_hovered_win = self.hovered_surface.as_ref() == Some(surface);
        let group_hovered = is_hovered_win && self.group_hovered;
        let hovered_button = if is_hovered_win {
            self.hovered_button
        } else {
            None
        };

        let key = CacheKey {
            width_phys,
            height_phys,
            scale_milli: (scale * 1000.0).round() as i32,
            title: title.to_string(),
            is_active,
            is_dark,
            buttons_right: self.buttons_right,
            group_hovered,
            hovered_button,
        };

        let needs_rebuild = self
            .cache
            .get(surface)
            .map(|entry| entry.key != key)
            .unwrap_or(true);

        if needs_rebuild {
            let bytes = rasterize_titlebar(&key, scale)?;
            let buffer = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                Size::from((width_phys, height_phys)),
                1,
                smithay::utils::Transform::Normal,
                None,
            );
            self.cache
                .insert(surface.clone(), CachedTitlebar { key, buffer });
        }

        let entry = self.cache.get(surface)?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc_phys.to_f64(),
            &entry.buffer,
            Some(1.0),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }
}

/// Configura Cairo para renderizar texto con suavizado en escala de grises,
/// hinting vertical ligero y métricas sin cuantización horizontal (estilo macOS).
pub fn apply_macos_font_options(cr: &Context) {
    if let Ok(mut opts) = cairo::FontOptions::new() {
        opts.set_antialias(cairo::Antialias::Gray);
        opts.set_hint_style(cairo::HintStyle::Slight);
        opts.set_hint_metrics(cairo::HintMetrics::Off);
        cr.set_font_options(&opts);
    }
}

fn rasterize_titlebar(key: &CacheKey, scale: f64) -> Option<Vec<u8>> {
    let w = key.width_phys;
    let h = key.height_phys;
    let mut surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    {
        let ctx = Context::new(&surface).ok()?;
        apply_macos_font_options(&ctx);
        let wf = w as f64;
        let hf = h as f64;

        // 1. Fondo de barra de título macOS con gradiente vertical sutil
        let grad = LinearGradient::new(0.0, 0.0, 0.0, hf);
        if key.is_dark {
            if key.is_active {
                grad.add_color_stop_rgba(0.0, 0.165, 0.165, 0.180, 1.0); // #2a2a2e
                grad.add_color_stop_rgba(1.0, 0.133, 0.133, 0.145, 1.0); // #222225
            } else {
                grad.add_color_stop_rgba(0.0, 0.125, 0.125, 0.137, 1.0); // #202023
                grad.add_color_stop_rgba(1.0, 0.114, 0.114, 0.125, 1.0); // #1d1d20
            }
        } else if key.is_active {
            grad.add_color_stop_rgba(0.0, 0.969, 0.969, 0.976, 1.0); // #f7f7f9
            grad.add_color_stop_rgba(1.0, 0.929, 0.929, 0.941, 1.0); // #ededf0
        } else {
            grad.add_color_stop_rgba(0.0, 0.925, 0.925, 0.933, 1.0); // #ececee
            grad.add_color_stop_rgba(1.0, 0.902, 0.902, 0.910, 1.0); // #e6e6e8
        }
        let _ = ctx.set_source(&grad);
        ctx.rectangle(0.0, 0.0, wf, hf);
        let _ = ctx.fill();

        // Línea divisoria inferior de 1px físico
        if key.is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.07);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.10);
        }
        ctx.rectangle(0.0, (hf - 1.0).max(0.0), wf, 1.0);
        let _ = ctx.fill();

        // 2. Botones semáforo macOS (Izquierda: Cerrar, Minimizar, Maximizar; Derecha: Minimizar, Maximizar, Cerrar)
        let cy = BTN_Y * scale;
        let r = BTN_RADIUS * scale;
        let show_colors = key.is_active || key.group_hovered;
        let logical_w = wf / scale.max(0.1);
        let (close_x, min_x, max_x) = button_positions(logical_w, key.buttons_right);

        let buttons = [
            (
                SsdButton::Close,
                close_x * scale,
                (1.0, 0.373, 0.337),   // #ff5f56
                (0.878, 0.267, 0.243), // #e0443e
            ),
            (
                SsdButton::Minimize,
                min_x * scale,
                (1.0, 0.741, 0.180),   // #ffbd2e
                (0.871, 0.631, 0.137), // #dea123
            ),
            (
                SsdButton::Maximize,
                max_x * scale,
                (0.153, 0.788, 0.247), // #27c93f
                (0.102, 0.671, 0.161), // #1aab29
            ),
        ];

        for (btn, cx, fill_rgb, border_rgb) in buttons {
            ctx.new_sub_path();
            ctx.arc(cx, cy, r, 0.0, TAU);
            if show_colors {
                let boost: f64 = if key.hovered_button == Some(btn) {
                    0.06
                } else {
                    0.0
                };
                ctx.set_source_rgba(
                    (fill_rgb.0 + boost).min(1.0),
                    (fill_rgb.1 + boost).min(1.0),
                    (fill_rgb.2 + boost).min(1.0),
                    1.0,
                );
                let _ = ctx.fill_preserve();
                ctx.set_line_width((0.75 * scale).max(1.0));
                ctx.set_source_rgba(border_rgb.0, border_rgb.1, border_rgb.2, 0.9);
                let _ = ctx.stroke();
            } else {
                if key.is_dark {
                    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.18);
                } else {
                    ctx.set_source_rgba(0.0, 0.0, 0.0, 0.16);
                }
                let _ = ctx.fill_preserve();
                ctx.set_line_width((0.75 * scale).max(1.0));
                if key.is_dark {
                    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.08);
                } else {
                    ctx.set_source_rgba(0.0, 0.0, 0.0, 0.10);
                }
                let _ = ctx.stroke();
            }

            // Glifos interiores cuando el grupo está en hover
            if key.group_hovered {
                ctx.set_line_cap(cairo::LineCap::Round);
                ctx.set_line_width_scaled(scale);
                match btn {
                    SsdButton::Close => {
                        ctx.set_source_rgba(0.30, 0.0, 0.0, 0.85);
                        let d = 2.6 * scale;
                        ctx.move_to(cx - d, cy - d);
                        ctx.line_to(cx + d, cy + d);
                        ctx.move_to(cx + d, cy - d);
                        ctx.line_to(cx - d, cy + d);
                        let _ = ctx.stroke();
                    }
                    SsdButton::Minimize => {
                        ctx.set_source_rgba(0.38, 0.22, 0.0, 0.88);
                        let d = 3.2 * scale;
                        ctx.move_to(cx - d, cy);
                        ctx.line_to(cx + d, cy);
                        let _ = ctx.stroke();
                    }
                    SsdButton::Maximize => {
                        ctx.set_source_rgba(0.0, 0.28, 0.04, 0.88);
                        let d = 3.0 * scale;
                        ctx.move_to(cx - d, cy);
                        ctx.line_to(cx + d, cy);
                        ctx.move_to(cx, cy - d);
                        ctx.line_to(cx, cy + d);
                        let _ = ctx.stroke();
                    }
                }
            }
        }

        // 3. Título centrado de la ventana
        if !key.title.is_empty() {
            ctx.select_font_face("Inter", FontSlant::Normal, FontWeight::Bold);
            ctx.set_font_size(13.0 * scale);

            let max_text_w = (wf - 160.0 * scale).max(40.0);
            let mut display_title = key.title.clone();
            if let Ok(ext) = ctx.text_extents(&display_title) {
                if ext.width() > max_text_w {
                    let chars: Vec<char> = display_title.chars().collect();
                    let mut len = chars.len();
                    while len > 3 {
                        len -= 1;
                        let candidate: String = chars[..len].iter().collect();
                        let candidate = format!("{candidate}…");
                        if let Ok(cext) = ctx.text_extents(&candidate) {
                            if cext.width() <= max_text_w {
                                display_title = candidate;
                                break;
                            }
                        }
                    }
                }
            }

            if let Ok(ext) = ctx.text_extents(&display_title) {
                let tx = ((wf - ext.width()) * 0.5 - ext.x_bearing()).max(80.0 * scale);
                let ty = (hf - ext.height()) * 0.5 - ext.y_bearing();
                if key.is_dark {
                    let alpha = if key.is_active { 0.90 } else { 0.45 };
                    ctx.set_source_rgba(0.957, 0.957, 0.965, alpha);
                } else {
                    let alpha = if key.is_active { 0.88 } else { 0.42 };
                    ctx.set_source_rgba(0.114, 0.114, 0.122, alpha);
                }
                ctx.move_to(tx, ty);
                let _ = ctx.show_text(&display_title);
            }
        }
    }

    surface.flush();
    let data = surface.data().ok()?;
    Some(data.to_vec())
}

trait ContextLineScaleExt {
    fn set_line_width_scaled(&self, scale: f64);
}

impl ContextLineScaleExt for Context {
    fn set_line_width_scaled(&self, scale: f64) {
        self.set_line_width((1.4 * scale).max(1.2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssd_hit_test_buttons_and_titlebar() {
        assert_eq!(
            hit_test_titlebar(20.0, 19.0, 800.0),
            Some(SsdHit::Button(SsdButton::Close))
        );
        assert_eq!(
            hit_test_titlebar(40.0, 19.0, 800.0),
            Some(SsdHit::Button(SsdButton::Minimize))
        );
        assert_eq!(
            hit_test_titlebar(60.0, 19.0, 800.0),
            Some(SsdHit::Button(SsdButton::Maximize))
        );
        assert_eq!(
            hit_test_titlebar(200.0, 19.0, 800.0),
            Some(SsdHit::Titlebar)
        );
        assert_eq!(hit_test_titlebar(200.0, 45.0, 800.0), None);
    }

    #[test]
    fn test_ssd_hit_test_right_side_order_min_max_close() {
        // Ancho 800: minimizar en 740, maximizar en 760, cerrar en 780
        assert_eq!(
            hit_test_titlebar_with_layout(740.0, 19.0, 800.0, true),
            Some(SsdHit::Button(SsdButton::Minimize))
        );
        assert_eq!(
            hit_test_titlebar_with_layout(760.0, 19.0, 800.0, true),
            Some(SsdHit::Button(SsdButton::Maximize))
        );
        assert_eq!(
            hit_test_titlebar_with_layout(780.0, 19.0, 800.0, true),
            Some(SsdHit::Button(SsdButton::Close))
        );
        assert_eq!(
            hit_test_titlebar_with_layout(20.0, 19.0, 800.0, true),
            Some(SsdHit::Titlebar)
        );
        assert!(is_in_traffic_lights_group_with_layout(760.0, 19.0, 800.0, true));
        assert!(!is_in_traffic_lights_group_with_layout(20.0, 19.0, 800.0, true));
    }

    #[test]
    fn test_rasterize_titlebar_produces_valid_argb_buffer() {
        for buttons_right in [false, true] {
            let key = CacheKey {
                width_phys: 600,
                height_phys: 38,
                scale_milli: 1000,
                title: "ZapFast".to_string(),
                is_active: true,
                is_dark: true,
                buttons_right,
                group_hovered: true,
                hovered_button: Some(SsdButton::Close),
            };
            let buf = rasterize_titlebar(&key, 1.0).expect("rasterize should succeed");
            assert_eq!(buf.len(), 600 * 38 * 4);
        }
    }
}
