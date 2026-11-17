//! Pipeline GPU para la animación de cierre de ventanas "Agujero Negro" (Black Hole + Accretion Disk).
//!
//! Flujo de ejecución en 3 fases (680 ms por defecto):
//! 1. **Ignición (`0.00 -> 0.16`)**: el botón de cierre se abre como un horizonte de sucesos
//!    con anillo de fotones y disco de acreción inclinado.
//! 2. **Succión gravitacional (`0.10 -> 0.76`)**: la captura GPU offscreen de la ventana se deforma
//!    mediante una malla teselada 36x36 (`gl_VertexID`), estirándose e ingresando en espiral
//!    con calentamiento cromático hacia el centro del agujero negro.
//! 3. **Colapso y desaparición (`0.76 -> 1.00`)**: el horizonte de sucesos y el disco de acreción
//!    se contraen rápidamente con un destello final de implosión hasta desaparecer por completo.

use std::collections::HashMap;
use std::ffi::CString;
use std::time::{Duration, Instant};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement, WaylandSurfaceTexture,
};
use smithay::backend::renderer::element::{Element, Id, Kind};
use smithay::backend::renderer::gles::{ffi, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{
    draw_render_elements, CommitCounter, RendererSurfaceStateUserData,
};
use smithay::backend::renderer::{Bind, Color32F, ContextId, Frame, Offscreen, Renderer, Texture};
use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::compositor::{self, TraversalAction};
use smithay::wayland::seat::WaylandFocus;
use tracing::{error, info, warn};

/// Duración total del efecto de cierre por agujero negro.
pub const BLACK_HOLE_DURATION: Duration = Duration::from_millis(680);

/// Resolución de la malla de succión (GRID_COLS x GRID_ROWS quads -> 7,776 vértices).
const GRID_COLS: i32 = 36;
const GRID_ROWS: i32 = 36;

/// Contenido gráfico preservado de una capa de superficie Wayland.
#[derive(Clone)]
pub enum CachedLayerContent {
    Texture(GlesTexture),
    SolidColor(Color32F),
}

/// Instantánea ligera de una superficie Wayland preservada antes de que el cliente desvincule o destruya su buffer.
#[derive(Clone)]
pub struct CachedSurfaceLayer {
    pub content: CachedLayerContent,
    pub commit: CommitCounter,
    pub offset_phys: Point<i32, Physical>,
    pub dst_size_phys: Size<i32, Physical>,
    pub captured_scale: f64,
    pub src: Rectangle<f64, Buffer>,
    pub transform: Transform,
    pub is_root: bool,
}

/// Ventana en cierre pendiente de capturar su textura offscreen antes de liberar el toplevel.
pub struct PendingBlackHoleClose {
    pub window: Window,
    pub geometry: Rectangle<i32, Logical>,
    pub location: Point<i32, Logical>,
    pub origin_logical: Point<f64, Logical>,
    pub uses_ssd: bool,
    pub is_fullscreen: bool,
    pub is_active: bool,
    pub is_firefox: bool,
    pub is_translucent: bool,
    pub title: String,
    pub cached_layers: Vec<CachedSurfaceLayer>,
}

/// Actualiza en O(1) amortizado la última instantánea de texturas renderizadas de una ventana viva.
/// Si sus `CommitCounter` y escala no cambiaron desde el frame anterior, retorna sin asignar memoria.
pub fn update_cached_window_snapshot(
    snapshots: &mut HashMap<WlSurface, (Vec<CachedSurfaceLayer>, Instant)>,
    root_surface: &WlSurface,
    elements: &[WaylandSurfaceRenderElement<GlesRenderer>],
    root_id: &Id,
    phys_loc: Point<i32, Physical>,
    scale: f64,
) {
    if elements.is_empty() {
        return;
    }

    if let Some((existing, _)) = snapshots.get(root_surface) {
        if existing.len() == elements.len()
            && existing
                .first()
                .is_some_and(|e| (e.captured_scale - scale).abs() < 1e-4)
            && existing
                .iter()
                .zip(elements.iter())
                .all(|(cached, elem)| cached.commit == elem.current_commit())
        {
            return;
        }
    }

    let scale_f = smithay::utils::Scale::from(scale);
    let layers = elements
        .iter()
        .map(|elem| {
            let elem_geo = elem.geometry(scale_f);
            let content = match elem.texture() {
                WaylandSurfaceTexture::Texture(tex) => CachedLayerContent::Texture(tex.clone()),
                WaylandSurfaceTexture::SolidColor(color) => CachedLayerContent::SolidColor(*color),
            };
            CachedSurfaceLayer {
                content,
                commit: elem.current_commit(),
                offset_phys: elem_geo.loc - phys_loc,
                dst_size_phys: elem_geo.size,
                captured_scale: scale,
                src: elem.src(),
                transform: elem.transform(),
                is_root: elem.id() == root_id,
            }
        })
        .collect();

    snapshots.insert(root_surface.clone(), (layers, Instant::now()));
}

/// Extrae una instantánea de las texturas GPU ya importadas en el árbol de superficies
/// antes de que `attach(null)` o `wl_surface.destroy()` limpie `RendererSurfaceState`.
pub fn capture_surface_tree_snapshot(
    root_surface: &WlSurface,
    ctx_id: ContextId<GlesTexture>,
) -> Vec<CachedSurfaceLayer> {
    let mut layers = Vec::new();
    let origin = Point::<i32, Logical>::from((0, 0));

    compositor::with_surface_tree_downward(
        root_surface,
        origin,
        |_, states, loc| {
            let mut next_loc = *loc;
            if let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() {
                if let Some(view) = data.lock().unwrap().view() {
                    next_loc += view.offset;
                    return TraversalAction::DoChildren(next_loc);
                }
            }
            TraversalAction::SkipChildren
        },
        |surf, states, loc| {
            if let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() {
                let rstate = data.lock().unwrap();
                if let (Some(tex), Some(view), Some(buf_size)) = (
                    rstate.texture::<GlesTexture>(ctx_id.clone()),
                    rstate.view(),
                    rstate.buffer_size(),
                ) {
                    let src = view.src.to_buffer(
                        rstate.buffer_scale() as f64,
                        rstate.buffer_transform(),
                        &buf_size.to_f64(),
                    );
                    let off = *loc + view.offset;
                    layers.push(CachedSurfaceLayer {
                        content: CachedLayerContent::Texture(tex.clone()),
                        commit: rstate.current_commit(),
                        offset_phys: Point::from((off.x, off.y)),
                        dst_size_phys: Size::from((view.dst.w, view.dst.h)),
                        captured_scale: 1.0,
                        src,
                        transform: rstate.buffer_transform(),
                        is_root: surf == root_surface,
                    });
                }
            }
        },
        |_, _, _| true,
    );

    layers
}

/// Estado visual de una animación de cierre en curso sobre un monitor específico.
pub struct ActiveBlackHoleClose {
    pub output_name: String,
    pub texture: GlesTexture,
    pub win_rect: Rectangle<i32, Physical>,
    pub origin_phys: (f32, f32),
    pub scale: f32,
    pub started_at: Instant,
    pub duration: Duration,
}

impl ActiveBlackHoleClose {
    pub fn is_finished(&self, now: Instant) -> bool {
        now.duration_since(self.started_at) >= self.duration
    }

    pub fn raw_progress(&self, now: Instant) -> f32 {
        let dur = self.duration.as_secs_f32().max(0.001);
        (now.duration_since(self.started_at).as_secs_f32() / dur).clamp(0.0, 1.0)
    }
}

/// Parámetros instantáneos evaluados en la línea de tiempo `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackHoleTimelineSample {
    /// Progreso normalizado de entrada `[0.0, 1.0]`.
    pub t: f32,
    /// Factor de apertura inicial del horizonte `[0.0, 1.0]`.
    pub ignite: f32,
    /// Factor de succión gravitacional de la malla `[0.0, 1.0]`.
    pub suction: f32,
    /// Factor de colapso/encogimiento final `[0.0, 1.0]`.
    pub collapse: f32,
    /// Radio actual del horizonte de sucesos en píxeles físicos.
    pub horizon_radius_px: f32,
    /// Intensidad lumínica del disco de acreción (`0.0` al inicio/fin, pico ~`1.28`).
    pub disk_intensity: f32,
    /// Destello breve justo antes de desaparecer por completo `[0.0, 1.0]`.
    pub pinch_flash: f32,
    /// Opacidad maestra `[0.0, 1.0]` garantizada `0.0` al finalizar.
    pub fade: f32,
    /// Fase temporal acumulada para la rotación del plasma kepleriano.
    pub spin_phase: f32,
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let u = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

impl BlackHoleTimelineSample {
    /// Evalúa todos los canales de animación en `raw_t ∈ [0.0, 1.0]`.
    pub fn sample(raw_t: f32, peak_radius_px: f32, scale: f32) -> Self {
        let t = raw_t.clamp(0.0, 1.0);
        let ignite = smoothstep(0.0, 0.16, t);

        // Succión: arranca suave cuando el botón se abre (0.10) y acelera hasta tragar todo en 0.76.
        let suction_u = ((t - 0.10) / 0.66).clamp(0.0, 1.0);
        let suction = suction_u * suction_u * (2.5 - 1.5 * suction_u);

        // Colapso final: el agujero negro y su disco se encogen y desaparecen (0.76 -> 1.00).
        let collapse_u = ((t - 0.76) / 0.24).clamp(0.0, 1.0);
        let collapse = smoothstep(0.0, 1.0, collapse_u);

        let button_r = (CLOSE_BUTTON_CORE_RADIUS_LOGICAL * scale).max(4.0);
        let grown_r = button_r + (peak_radius_px - button_r) * ignite;
        // Ligera pulsación al absorber la ventana y luego implosión a 0 px.
        let swallow_swell = 1.0 + 0.08 * (suction * std::f32::consts::PI).sin();
        let shrink_factor = (1.0 - collapse).powf(1.35);
        let horizon_radius_px = (grown_r * swallow_swell * shrink_factor).max(0.0);

        let disk_rise = smoothstep(0.0, 0.13, t);
        let mass_flare = 1.0 + 0.25 * smoothstep(0.25, 0.68, t) * (1.0 - collapse * 0.4);
        let disk_intensity = disk_rise * mass_flare * (1.0 - smoothstep(0.50, 1.0, collapse));

        // Destello breve de pinch al colapsar sobre la singularidad (collapse_u ~ 0.15..0.60).
        let flash_up = smoothstep(0.05, 0.30, collapse_u);
        let flash_down = 1.0 - smoothstep(0.30, 0.78, collapse_u);
        let pinch_flash = flash_up * flash_down;

        let fade = (smoothstep(0.0, 0.04, t) * (1.0 - smoothstep(0.55, 1.0, collapse_u))).clamp(0.0, 1.0);

        // Giro acelerado al comprimirse (conservación de momento angular).
        let spin_phase = t * 5.2 + t * t * 3.8 + collapse * collapse * 3.2;

        Self {
            t,
            ignite,
            suction,
            collapse,
            horizon_radius_px,
            disk_intensity,
            pinch_flash,
            fade,
            spin_phase,
        }
    }
}

/// Radio lógico del núcleo oscuro del horizonte igual al radio del botón rojo (`BTN_RADIUS = 6.5 px`).
pub const CLOSE_BUTTON_CORE_RADIUS_LOGICAL: f32 = 6.5;

/// Radio del horizonte de sucesos dimensionado al tamaño del botón de cierre (`6.5 px` lógicos).
pub fn compute_peak_horizon_radius(_win_size: Size<i32, Physical>, scale: f32) -> f32 {
    (CLOSE_BUTTON_CORE_RADIUS_LOGICAL * scale).max(4.0)
}

/// Captura una instantánea offscreen RGBA premultiplicada de la ventana que acaba de cerrarse,
/// incluyendo barra SSD y recorte squircle idéntico al cuadro vivo en pantalla.
#[allow(clippy::too_many_arguments)]
pub fn capture_closing_window_texture(
    renderer: &mut GlesRenderer,
    ssd: &mut crate::ssd::SsdManager,
    squircle_shader: Option<&crate::window_corners::WindowSquircleShader>,
    blur_pipeline: Option<&mut crate::blur::GaussianBlur>,
    pending: &PendingBlackHoleClose,
    output_name: &str,
    output_geo: Rectangle<i32, Logical>,
    screen_size: Size<i32, Physical>,
    scale: f64,
    win_phys_r: Option<i32>,
    is_dark_theme: bool,
) -> Option<ActiveBlackHoleClose> {
    let geo = pending.geometry;
    if geo.size.w <= 4 || geo.size.h <= 4 {
        return None;
    }

    let win_global_rect = Rectangle::new(pending.location, geo.size);
    let expanded_global_rect = if pending.uses_ssd && !pending.is_fullscreen {
        Rectangle::new(
            Point::from((
                win_global_rect.loc.x,
                win_global_rect.loc.y - crate::ssd::SSD_TITLEBAR_HEIGHT,
            )),
            Size::from((
                win_global_rect.size.w,
                win_global_rect.size.h + crate::ssd::SSD_TITLEBAR_HEIGHT,
            )),
        )
    } else {
        win_global_rect
    };

    if !output_geo.overlaps(expanded_global_rect) {
        return None;
    }

    let main_surface = pending.window.wl_surface()?;
    let root_id = Id::from_wayland_resource(main_surface.as_ref());

    let phys_loc = (pending.location - geo.loc - output_geo.loc).to_physical_precise_round(scale);
    let base_win_rect = Rectangle::new(
        (pending.location - output_geo.loc).to_physical_precise_round(scale),
        geo.size.to_physical_precise_round(scale),
    );

    let probe_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            main_surface.as_ref(),
            phys_loc,
            smithay::utils::Scale::from(scale),
            1.0,
            Kind::Unspecified,
        );
    let use_cached = probe_elems.is_empty();
    if use_cached && pending.cached_layers.is_empty() {
        warn!(
            "⚠️ [BlackHole] Sin elementos vivos ni snapshot cacheado para '{}' ({}x{})",
            pending.title, geo.size.w, geo.size.h
        );
        return None;
    }

    let screen_content_rect = if !use_cached && !pending.uses_ssd {
        crate::window_corners::effective_shadow_rect_for_window(
            &probe_elems,
            &root_id,
            base_win_rect,
            scale,
            pending.is_translucent,
            pending.uses_ssd,
        )
    } else {
        base_win_rect
    };

    let (final_screen_rect, tb_screen_loc, tb_phys_h) = if pending.uses_ssd && !pending.is_fullscreen
    {
        let tb_h = (crate::ssd::SSD_TITLEBAR_HEIGHT as f64 * scale).round() as i32;
        let tb_loc = Point::from((screen_content_rect.loc.x, screen_content_rect.loc.y - tb_h));
        let full_rect = Rectangle::new(
            tb_loc,
            Size::from((screen_content_rect.size.w, screen_content_rect.size.h + tb_h)),
        );
        (full_rect, Some(tb_loc), tb_h)
    } else {
        (screen_content_rect, None, 0)
    };

    if final_screen_rect.size.w <= 4 || final_screen_rect.size.h <= 4 {
        return None;
    }

    // Desplazamiento al espacio local [0..w, 0..h] de la textura offscreen.
    let shift = Point::from((-final_screen_rect.loc.x, -final_screen_rect.loc.y));
    let offscreen_win_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = if !use_cached {
        render_elements_from_surface_tree(
            renderer,
            main_surface.as_ref(),
            phys_loc + shift,
            smithay::utils::Scale::from(scale),
            1.0,
            Kind::Unspecified,
        )
    } else {
        Vec::new()
    };

    let ssd_elem = tb_screen_loc.and_then(|tb_loc| {
        ssd.render_element_for_window(
            renderer,
            main_surface.as_ref(),
            tb_loc + shift,
            final_screen_rect.size.w,
            tb_phys_h,
            scale,
            &pending.title,
            pending.is_active,
            is_dark_theme,
        )
    });

    let buf_size = Size::<i32, Buffer>::from((final_screen_rect.size.w, final_screen_rect.size.h));
    let mut texture = match renderer.create_buffer(Fourcc::Abgr8888, buf_size) {
        Ok(tex) => tex,
        Err(err) => {
            warn!("⚠️ [BlackHole] No se pudo crear textura offscreen: {:?}", err);
            return None;
        }
    };

    let offscreen_rect = Rectangle::from_size(final_screen_rect.size);
    let should_round = win_phys_r.is_some() && !pending.is_fullscreen;

    {
        let mut target = match renderer.bind(&mut texture) {
            Ok(t) => t,
            Err(err) => {
                warn!("⚠️ [BlackHole] Fallo al enlazar FBO offscreen: {:?}", err);
                return None;
            }
        };
        let mut frame = match renderer.render(&mut target, final_screen_rect.size, Transform::Normal)
        {
            Ok(f) => f,
            Err(err) => {
                warn!("⚠️ [BlackHole] Fallo al iniciar frame offscreen: {:?}", err);
                return None;
            }
        };

        let _ = frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[offscreen_rect]);
        if pending.is_translucent {
            if let Some(blur) = blur_pipeline {
                blur.render_translucent_offscreen_blur(
                    &mut frame,
                    screen_size,
                    final_screen_rect,
                    offscreen_rect,
                    scale,
                    win_phys_r,
                    should_round,
                );
            }
        }
        if !offscreen_win_elems.is_empty() {
            crate::window_corners::render_window_elements(
                &mut frame,
                scale,
                &[offscreen_rect],
                squircle_shader,
                &root_id,
                &offscreen_win_elems,
                ssd_elem.as_ref(),
                offscreen_rect,
                win_phys_r,
                should_round,
                pending.is_firefox,
                None,
            );
        } else {
            let clip_radius = if should_round && squircle_shader.is_some() {
                win_phys_r
            } else {
                None
            };
            let unified_clip = pending.is_firefox || ssd_elem.is_some();

            for layer in pending.cached_layers.iter().rev() {
                let ratio = if (scale - layer.captured_scale).abs() < 1e-4 {
                    1.0
                } else {
                    scale / layer.captured_scale.max(0.1)
                };
                let dst_loc = phys_loc
                    + shift
                    + Point::from((
                        (layer.offset_phys.x as f64 * ratio).round() as i32,
                        (layer.offset_phys.y as f64 * ratio).round() as i32,
                    ));
                let dst_size = Size::from((
                    (layer.dst_size_phys.w as f64 * ratio).round() as i32,
                    (layer.dst_size_phys.h as f64 * ratio).round() as i32,
                ));
                if dst_size.w <= 0 || dst_size.h <= 0 {
                    continue;
                }
                let dst_rect = Rectangle::new(dst_loc, dst_size);
                match layer.content {
                    CachedLayerContent::Texture(ref texture) => {
                        let applied = if let (Some(r), Some(sh)) = (clip_radius, squircle_shader) {
                            if unified_clip
                                || layer.is_root
                                || crate::window_corners::should_clip_subsurface_to_window(
                                    dst_rect,
                                    offscreen_rect,
                                )
                            {
                                let tex_params = crate::window_corners::SurfaceTexParams {
                                    src: layer.src,
                                    tex_size: texture.size(),
                                    transform: layer.transform,
                                    y_inverted: texture.is_y_inverted(),
                                };
                                let sub_offset = Point::from((-dst_rect.loc.x, -dst_rect.loc.y));
                                let border_strength =
                                    if unified_clip || layer.is_root || dst_rect == offscreen_rect {
                                        1.0
                                    } else {
                                        0.0
                                    };
                                sh.apply_surface_to_frame(
                                    &mut frame,
                                    Some(tex_params),
                                    dst_size,
                                    sub_offset,
                                    offscreen_rect.size,
                                    r,
                                    border_strength,
                                    false,
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        let _ = frame.render_texture_from_to(
                            texture,
                            layer.src,
                            dst_rect,
                            &[Rectangle::from_size(dst_size)],
                            &[],
                            layer.transform,
                            1.0,
                            None,
                            &[],
                        );
                        if applied {
                            frame.clear_tex_program_override();
                        }
                    }
                    CachedLayerContent::SolidColor(color) => {
                        let _ =
                            frame.draw_solid(dst_rect, &[Rectangle::from_size(dst_size)], color);
                    }
                }
            }

            if let Some(ref tb_elem) = ssd_elem {
                if let (Some(r), Some(sh)) = (clip_radius, squircle_shader) {
                    let tb_geo = tb_elem.geometry(smithay::utils::Scale::from(1.0));
                    let sub_offset = Point::from((-tb_geo.loc.x, -tb_geo.loc.y));
                    sh.apply_to_frame(&mut frame, tb_geo.size, sub_offset, offscreen_rect.size, r);
                    let _ = draw_render_elements(
                        &mut frame,
                        1.0,
                        std::slice::from_ref(tb_elem),
                        &[offscreen_rect],
                    );
                    frame.clear_tex_program_override();
                } else {
                    let _ = draw_render_elements(
                        &mut frame,
                        1.0,
                        std::slice::from_ref(tb_elem),
                        &[offscreen_rect],
                    );
                }
            }
        }
        if let Err(err) = frame.finish() {
            warn!("⚠️ [BlackHole] Fallo al finalizar frame offscreen: {:?}", err);
            return None;
        }
    }

    let origin_phys = (
        ((pending.origin_logical.x - output_geo.loc.x as f64) * scale) as f32,
        ((pending.origin_logical.y - output_geo.loc.y as f64) * scale) as f32,
    );

    info!(
        "🕳️ [BlackHole] Captura lista para '{}' en {} ({}x{}, cached={})",
        pending.title, output_name, final_screen_rect.size.w, final_screen_rect.size.h, use_cached
    );

    Some(ActiveBlackHoleClose {
        output_name: output_name.to_string(),
        texture,
        win_rect: final_screen_rect,
        origin_phys,
        scale: scale as f32,
        started_at: Instant::now(),
        duration: BLACK_HOLE_DURATION,
    })
}

// ============================================================================
// SHADERS GLES 3.0 PROCEDURALES (sin VBO, basados en gl_VertexID)
// ============================================================================

const WARP_VERT_SRC: &str = r#"#version 300 es
precision highp float;

uniform mat3 u_matrix;
uniform vec4 u_win_rect;     // (x, y, w, h) en píxeles físicos del monitor
uniform vec2 u_bh_center;    // centro de succión en píxeles físicos
uniform float u_max_dist;    // distancia máxima desde u_bh_center a las 4 esquinas
uniform float u_suction;     // [0.0 .. 1.0]
uniform float u_spin_dir;    // +1.0 ó -1.0 según botón izq/der
uniform int u_grid_cols;
uniform int u_grid_rows;

out vec2 v_uv;
out vec2 v_world_pos;
out float v_pull;

void main() {
    int quad_idx = gl_VertexID / 6;
    int tri_v = gl_VertexID % 6;
    int col = quad_idx % u_grid_cols;
    int row = quad_idx / u_grid_cols;

    float vx = (tri_v == 1 || tri_v == 2 || tri_v == 4) ? 1.0 : 0.0;
    float vy = (tri_v == 2 || tri_v == 4 || tri_v == 5) ? 1.0 : 0.0;

    vec2 uv = vec2(
        (float(col) + vx) / float(u_grid_cols),
        (float(row) + vy) / float(u_grid_rows)
    );
    v_uv = uv;

    vec2 orig_pos = u_win_rect.xy + uv * u_win_rect.zw;
    vec2 d = orig_pos - u_bh_center;
    float dist = length(d);
    float norm_r = clamp(dist / max(u_max_dist, 1.0), 0.0, 1.0);

    // Absorción cohesiva directa hacia el botón de cierre (sin estiramiento en túnel)
    float lag = norm_r * 0.14;
    float local_t = clamp((u_suction - lag) / max(1.0 - lag, 0.001), 0.0, 1.0);
    float eased = local_t * local_t * (3.0 - 2.0 * local_t);
    float pull = pow(eased, 1.25);
    v_pull = pull;

    // Giro sutil únicamente justo al entrar en el disco del botón (sin rotar todo el cuerpo)
    float near_spin = smoothstep(0.68, 1.0, pull);
    float twist = u_spin_dir * near_spin * near_spin * 0.90;
    float c = cos(twist);
    float s = sin(twist);
    vec2 rotated = vec2(d.x * c - d.y * s, d.x * s + d.y * c);

    float shrink = max(1.0 - pull, 0.0);
    vec2 warped_pos = u_bh_center + rotated * shrink;
    v_world_pos = warped_pos;

    vec3 ndc = u_matrix * vec3(warped_pos, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
}
"#;

const WARP_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
in vec2 v_world_pos;
in float v_pull;

out vec4 frag_color;

uniform sampler2D u_tex;     // textura premultiplicada de la ventana
uniform vec2 u_bh_center;
uniform float u_horizon_r;

void main() {
    vec4 tex = texture(u_tex, v_uv);
    if (tex.a <= 0.002) {
        discard;
    }

    float r = length(v_world_pos - u_bh_center);

    // Calentamiento sutil justo al tocar el disco de acreción del botón
    float near_ring = smoothstep(u_horizon_r * 2.8, u_horizon_r * 0.85, r);
    float heat = smoothstep(0.55, 0.96, v_pull) * near_ring;
    vec3 hot_color = mix(vec3(1.0, 0.50, 0.12), vec3(1.0, 0.94, 0.78), smoothstep(0.80, 1.0, v_pull));
    tex.rgb = mix(tex.rgb, hot_color * tex.a, clamp(heat * 0.75, 0.0, 0.75));

    // Absorción suave al cruzar el horizonte de sucesos
    float swallow = smoothstep(u_horizon_r * 0.45, u_horizon_r * 1.10, r) * smoothstep(1.0, 0.90, v_pull);
    if (swallow <= 0.002) {
        discard;
    }

    frag_color = tex * swallow;
}
"#;

const BH_VERT_SRC: &str = r#"#version 300 es
precision highp float;

uniform mat3 u_matrix;
uniform vec2 u_bh_center;
uniform float u_quad_half;

out vec2 v_world_pos;

void main() {
    float u = (gl_VertexID == 1 || gl_VertexID == 2 || gl_VertexID == 4) ? 1.0 : -1.0;
    float v = (gl_VertexID == 2 || gl_VertexID == 4 || gl_VertexID == 5) ? 1.0 : -1.0;

    vec2 world_pos = u_bh_center + vec2(u, v) * u_quad_half;
    v_world_pos = world_pos;

    vec3 ndc = u_matrix * vec3(world_pos, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
}
"#;

const BH_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_world_pos;
out vec4 frag_color;

uniform vec2 u_bh_center;
uniform float u_bh_radius;
uniform float u_spin_time;
uniform float u_spin_dir;
uniform float u_intensity;
uniform float u_pinch_flash;
uniform float u_fade;

void main() {
    float radius = max(u_bh_radius, 0.5);
    vec2 p = (v_world_pos - u_bh_center) / radius;
    float r_sphere = length(p);

    if (r_sphere > 2.5) {
        discard;
    }

    // Plano inclinado del disco de acreción (-0.22 rad)
    float tilt = -0.22;
    float ct = cos(tilt);
    float st = sin(tilt);
    vec2 pr = vec2(p.x * ct - p.y * st, p.x * st + p.y * ct);
    vec2 p_disk = vec2(pr.x, pr.y / 0.32);
    float r_disk = length(p_disk);

    // Filamentos espirales keplerianos (las órbitas interiores giran más rápido)
    float ang = atan(p_disk.y, p_disk.x);
    float omega = u_spin_time * (2.6 + 2.8 / max(r_disk, 0.65));
    float spiral_ang = ang - u_spin_dir * omega + 3.8 / max(r_disk, 0.55);
    float streak1 = 0.5 + 0.5 * sin(spiral_ang * 3.0 + r_disk * 7.5 - u_spin_time * 4.0);
    float streak2 = 0.5 + 0.5 * sin(spiral_ang * 7.0 - r_disk * 11.0 + u_spin_time * 6.5);
    float streak3 = 0.5 + 0.5 * sin(spiral_ang * 13.0 + r_disk * 16.0);
    float filaments = mix(0.45, 1.18, streak1 * 0.55 + streak2 * 0.30 + streak3 * 0.15);

    // Perfil radial del disco (equilibrado alrededor del botón rojo)
    float inner_edge = smoothstep(0.95, 1.15, r_disk);
    float outer_edge = exp(-pow(max(r_disk - 1.15, 0.0) / 0.72, 1.65));
    float disk_band = inner_edge * outer_edge * filaments;

    // Beaming relativista (Doppler): un lado del plasma luce más caliente y brillante
    float doppler = 1.0 + 0.42 * (p_disk.x / max(r_disk, 0.4)) * (-u_spin_dir);
    disk_band *= clamp(doppler, 0.52, 1.55);

    // Arcos de lente gravitacional alrededor del horizonte
    float lens_radial = smoothstep(0.95, 1.06, r_sphere) * exp(-pow(max(r_sphere - 1.06, 0.0) / 0.32, 1.75));
    float lens_ang = atan(pr.y, pr.x);
    float lens_spiral = 0.5 + 0.5 * sin(lens_ang * 5.0 - u_spin_dir * u_spin_time * 5.0 + r_sphere * 9.0);
    float pole_weight = mix(0.45, 1.15, abs(pr.y) / max(r_sphere, 0.1));
    float lensed_halo = lens_radial * mix(0.65, 1.15, lens_spiral) * pole_weight * 0.88;

    // Sombra del horizonte de sucesos + paso frontal vs trasero del disco
    float behind_sphere = smoothstep(1.03, 0.94, r_sphere);
    float front_mask = smoothstep(-0.18, 0.22, pr.y);
    float disk_vis = disk_band * mix(1.0 - behind_sphere, 1.0, front_mask);

    // Anillo de fotones muy brillante justo en el borde del horizonte
    float photon_ring = exp(-pow((r_sphere - 1.01) / 0.075, 2.0)) * 1.25;

    // Paleta física de cuerpo negro / plasma solar
    vec3 disk_hot = vec3(1.00, 0.96, 0.84);
    vec3 disk_mid = vec3(1.00, 0.58, 0.14);
    vec3 disk_cool = vec3(0.82, 0.22, 0.05);
    float temp = clamp(1.0 - (r_disk - 1.02) / 1.05, 0.0, 1.0);
    vec3 disk_col = mix(disk_cool, mix(disk_mid, disk_hot, smoothstep(0.45, 0.95, temp)), smoothstep(0.05, 0.65, temp));
    vec3 lens_col = mix(vec3(0.98, 0.54, 0.14), vec3(1.0, 0.92, 0.72), smoothstep(1.35, 1.02, r_sphere));
    vec3 ring_col = vec3(1.0, 0.95, 0.82);

    // Destello final de colapso en la singularidad
    float spark = exp(-pow(r_sphere / 0.85, 2.0)) * u_pinch_flash * 1.65;
    vec3 spark_col = vec3(0.95, 0.97, 1.0) * spark;

    // Corona sutil alrededor del disco
    float corona = exp(-r_sphere * 1.25) * 0.15 * (1.0 - behind_sphere * 0.85);
    vec3 corona_col = vec3(0.95, 0.42, 0.12) * corona;

    vec3 emissive_rgb = (disk_col * disk_vis + lens_col * lensed_halo + ring_col * photon_ring + corona_col) * u_intensity + spark_col;

    float core_occlude = behind_sphere * smoothstep(0.0, 0.12, u_intensity);
    float plasma_alpha = clamp((disk_vis * 0.95 + lensed_halo * 0.75 + photon_ring * 0.65) * u_intensity, 0.0, 1.0);
    float total_alpha = clamp(max(core_occlude, max(plasma_alpha, spark * 0.65)) * u_fade, 0.0, 1.0);

    frag_color = vec4(emissive_rgb * u_fade, total_alpha);
}
"#;

/// Pipeline GPU OpenGL ES 3.0 para dibujar la deformación en vórtice y el agujero negro.
pub struct BlackHolePipeline {
    warp_program: ffi::types::GLuint,
    u_warp_matrix: ffi::types::GLint,
    u_warp_win_rect: ffi::types::GLint,
    u_warp_bh_center: ffi::types::GLint,
    u_warp_max_dist: ffi::types::GLint,
    u_warp_suction: ffi::types::GLint,
    u_warp_spin_dir: ffi::types::GLint,
    u_warp_grid_cols: ffi::types::GLint,
    u_warp_grid_rows: ffi::types::GLint,
    u_warp_horizon_r: ffi::types::GLint,
    u_warp_tex: ffi::types::GLint,

    bh_program: ffi::types::GLuint,
    u_bh_matrix: ffi::types::GLint,
    u_bh_center: ffi::types::GLint,
    u_bh_radius: ffi::types::GLint,
    u_bh_quad_half: ffi::types::GLint,
    u_bh_spin_time: ffi::types::GLint,
    u_bh_spin_dir: ffi::types::GLint,
    u_bh_intensity: ffi::types::GLint,
    u_bh_pinch_flash: ffi::types::GLint,
    u_bh_fade: ffi::types::GLint,
}

impl BlackHolePipeline {
    pub fn new(gl: &ffi::Gles2) -> Option<Self> {
        unsafe {
            let warp_program = link_program(gl, WARP_VERT_SRC, WARP_FRAG_SRC, "Warp")?;
            let bh_program = match link_program(gl, BH_VERT_SRC, BH_FRAG_SRC, "BlackHole") {
                Some(p) => p,
                None => {
                    gl.DeleteProgram(warp_program);
                    return None;
                }
            };

            let get_loc = |program: ffi::types::GLuint, name: &str| -> ffi::types::GLint {
                let cname = CString::new(name).unwrap();
                gl.GetUniformLocation(program, cname.as_ptr())
            };

            Some(Self {
                u_warp_matrix: get_loc(warp_program, "u_matrix"),
                u_warp_win_rect: get_loc(warp_program, "u_win_rect"),
                u_warp_bh_center: get_loc(warp_program, "u_bh_center"),
                u_warp_max_dist: get_loc(warp_program, "u_max_dist"),
                u_warp_suction: get_loc(warp_program, "u_suction"),
                u_warp_spin_dir: get_loc(warp_program, "u_spin_dir"),
                u_warp_grid_cols: get_loc(warp_program, "u_grid_cols"),
                u_warp_grid_rows: get_loc(warp_program, "u_grid_rows"),
                u_warp_horizon_r: get_loc(warp_program, "u_horizon_r"),
                u_warp_tex: get_loc(warp_program, "u_tex"),
                warp_program,

                u_bh_matrix: get_loc(bh_program, "u_matrix"),
                u_bh_center: get_loc(bh_program, "u_bh_center"),
                u_bh_radius: get_loc(bh_program, "u_bh_radius"),
                u_bh_quad_half: get_loc(bh_program, "u_quad_half"),
                u_bh_spin_time: get_loc(bh_program, "u_spin_time"),
                u_bh_spin_dir: get_loc(bh_program, "u_spin_dir"),
                u_bh_intensity: get_loc(bh_program, "u_intensity"),
                u_bh_pinch_flash: get_loc(bh_program, "u_pinch_flash"),
                u_bh_fade: get_loc(bh_program, "u_fade"),
                bh_program,
            })
        }
    }

    /// Dibuja un efecto de cierre completo (ventana deformada + agujero negro con disco de acreción).
    pub fn render_effect(
        &self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        active: &ActiveBlackHoleClose,
        now: Instant,
    ) {
        if active.win_rect.size.w <= 0 || active.win_rect.size.h <= 0 {
            return;
        }

        let raw_t = active.raw_progress(now);
        let peak_r = compute_peak_horizon_radius(active.win_rect.size, active.scale);
        let sample = BlackHoleTimelineSample::sample(raw_t, peak_r, active.scale);

        let win_x = active.win_rect.loc.x as f32;
        let win_y = active.win_rect.loc.y as f32;
        let win_w = active.win_rect.size.w as f32;
        let win_h = active.win_rect.size.h as f32;

        // Giro coherente con la posición del botón: izquierdo gira horario, derecho antihorario
        let spin_dir = if active.origin_phys.0 < win_x + win_w * 0.5 {
            1.0f32
        } else {
            -1.0f32
        };

        let mut max_dist = 1.0f32;
        for &(cx, cy) in &[
            (win_x, win_y),
            (win_x + win_w, win_y),
            (win_x, win_y + win_h),
            (win_x + win_w, win_y + win_h),
        ] {
            let d = ((cx - active.origin_phys.0).powi(2) + (cy - active.origin_phys.1).powi(2))
                .sqrt();
            if d > max_dist {
                max_dist = d;
            }
        }

        unsafe {
            let mut prev_blend: ffi::types::GLint = 0;
            let mut prev_scissor: ffi::types::GLint = 0;
            let mut prev_cull: ffi::types::GLint = 0;
            let mut prev_active_tex: ffi::types::GLint = ffi::TEXTURE0 as ffi::types::GLint;
            gl.GetIntegerv(ffi::BLEND, &mut prev_blend);
            gl.GetIntegerv(ffi::SCISSOR_TEST, &mut prev_scissor);
            gl.GetIntegerv(ffi::CULL_FACE, &mut prev_cull);
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut prev_active_tex);

            gl.Disable(ffi::SCISSOR_TEST);
            gl.Disable(ffi::CULL_FACE);
            gl.Enable(ffi::BLEND);
            gl.BlendFuncSeparate(
                ffi::ONE,
                ffi::ONE_MINUS_SRC_ALPHA,
                ffi::ONE,
                ffi::ONE_MINUS_SRC_ALPHA,
            );

            if gl.VertexAttribDivisor.is_loaded() {
                gl.VertexAttribDivisor(0, 0);
                gl.VertexAttribDivisor(1, 0);
            }
            gl.DisableVertexAttribArray(0);
            gl.DisableVertexAttribArray(1);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);

            // Pase 1: Malla deformada de la ventana siendo absorbida en espiral
            if sample.suction < 0.999 {
                gl.UseProgram(self.warp_program);
                gl.ActiveTexture(ffi::TEXTURE0);
                gl.BindTexture(ffi::TEXTURE_2D, active.texture.tex_id());
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_MIN_FILTER,
                    ffi::LINEAR as ffi::types::GLint,
                );
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_MAG_FILTER,
                    ffi::LINEAR as ffi::types::GLint,
                );
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_S,
                    ffi::CLAMP_TO_EDGE as ffi::types::GLint,
                );
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_T,
                    ffi::CLAMP_TO_EDGE as ffi::types::GLint,
                );

                gl.UniformMatrix3fv(self.u_warp_matrix, 1, ffi::FALSE, projection.as_ptr());
                gl.Uniform4f(self.u_warp_win_rect, win_x, win_y, win_w, win_h);
                gl.Uniform2f(
                    self.u_warp_bh_center,
                    active.origin_phys.0,
                    active.origin_phys.1,
                );
                gl.Uniform1f(self.u_warp_max_dist, max_dist);
                gl.Uniform1f(self.u_warp_suction, sample.suction);
                gl.Uniform1f(self.u_warp_spin_dir, spin_dir);
                gl.Uniform1i(self.u_warp_grid_cols, GRID_COLS);
                gl.Uniform1i(self.u_warp_grid_rows, GRID_ROWS);
                gl.Uniform1f(self.u_warp_horizon_r, sample.horizon_radius_px);
                gl.Uniform1i(self.u_warp_tex, 0);

                gl.DrawArrays(ffi::TRIANGLES, 0, GRID_COLS * GRID_ROWS * 6);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
            }

            // Pase 2: Agujero negro + disco de acreción relativista sobre el punto de cierre
            if sample.horizon_radius_px > 0.25 && sample.fade > 0.002 {
                let quad_half = sample.horizon_radius_px * 2.5;
                gl.UseProgram(self.bh_program);
                gl.UniformMatrix3fv(self.u_bh_matrix, 1, ffi::FALSE, projection.as_ptr());
                gl.Uniform2f(
                    self.u_bh_center,
                    active.origin_phys.0,
                    active.origin_phys.1,
                );
                gl.Uniform1f(self.u_bh_radius, sample.horizon_radius_px);
                gl.Uniform1f(self.u_bh_quad_half, quad_half);
                gl.Uniform1f(self.u_bh_spin_time, sample.spin_phase);
                gl.Uniform1f(self.u_bh_spin_dir, spin_dir);
                gl.Uniform1f(self.u_bh_intensity, sample.disk_intensity);
                gl.Uniform1f(self.u_bh_pinch_flash, sample.pinch_flash);
                gl.Uniform1f(self.u_bh_fade, sample.fade);

                gl.DrawArrays(ffi::TRIANGLES, 0, 6);
            }

            gl.UseProgram(0);
            gl.ActiveTexture(prev_active_tex as ffi::types::GLenum);
            gl.BlendFuncSeparate(
                ffi::ONE,
                ffi::ONE_MINUS_SRC_ALPHA,
                ffi::ONE,
                ffi::ONE_MINUS_SRC_ALPHA,
            );
            if prev_blend == ffi::FALSE as ffi::types::GLint {
                gl.Disable(ffi::BLEND);
            }
            if prev_scissor != ffi::FALSE as ffi::types::GLint {
                gl.Enable(ffi::SCISSOR_TEST);
            }
            if prev_cull != ffi::FALSE as ffi::types::GLint {
                gl.Enable(ffi::CULL_FACE);
            }
        }
    }
}

unsafe fn compile_shader(
    gl: &ffi::Gles2,
    shader_type: ffi::types::GLenum,
    src: &str,
    label: &str,
) -> Option<ffi::types::GLuint> {
    let shader = gl.CreateShader(shader_type);
    if shader == 0 {
        return None;
    }
    let length = src.len() as ffi::types::GLint;
    gl.ShaderSource(
        shader,
        1,
        &(src.as_ptr() as *const ffi::types::GLchar),
        &length,
    );
    gl.CompileShader(shader);

    let mut success = 0;
    gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut success);
    if success == 0 {
        let mut len = 0;
        gl.GetShaderiv(shader, ffi::INFO_LOG_LENGTH, &mut len);
        let mut buffer: Vec<u8> = vec![0; len.max(1) as usize];
        gl.GetShaderInfoLog(
            shader,
            len,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut ffi::types::GLchar,
        );
        error!(
            "❌ [BlackHolePipeline:{}] Error compilando shader: {}",
            label,
            String::from_utf8_lossy(&buffer)
        );
        gl.DeleteShader(shader);
        return None;
    }

    Some(shader)
}

pub(crate) unsafe fn link_program(
    gl: &ffi::Gles2,
    vert_src: &str,
    frag_src: &str,
    label: &str,
) -> Option<ffi::types::GLuint> {
    let vs = compile_shader(gl, ffi::VERTEX_SHADER, vert_src, label)?;
    let fs = match compile_shader(gl, ffi::FRAGMENT_SHADER, frag_src, label) {
        Some(s) => s,
        None => {
            gl.DeleteShader(vs);
            return None;
        }
    };

    let program = gl.CreateProgram();
    gl.AttachShader(program, vs);
    gl.AttachShader(program, fs);
    gl.LinkProgram(program);
    gl.DeleteShader(vs);
    gl.DeleteShader(fs);

    let mut status = 0;
    gl.GetProgramiv(program, ffi::LINK_STATUS, &mut status);
    if status == 0 {
        let mut len = 0;
        gl.GetProgramiv(program, ffi::INFO_LOG_LENGTH, &mut len);
        let mut buffer: Vec<u8> = vec![0; len.max(1) as usize];
        gl.GetProgramInfoLog(
            program,
            len,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut ffi::types::GLchar,
        );
        error!(
            "❌ [BlackHolePipeline:{}] Error enlazando programa GL: {}",
            label,
            String::from_utf8_lossy(&buffer)
        );
        gl.DeleteProgram(program);
        return None;
    }

    Some(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_ignition_suction_and_collapse_phases() {
        let peak_r = CLOSE_BUTTON_CORE_RADIUS_LOGICAL;
        let s0 = BlackHoleTimelineSample::sample(0.0, peak_r, 1.0);
        assert_eq!(s0.suction, 0.0);
        assert_eq!(s0.collapse, 0.0);
        assert_eq!(s0.fade, 0.0);

        // Ignición: botón se abre en agujero negro con disco antes de succionar por completo
        let s_ignite = BlackHoleTimelineSample::sample(0.14, peak_r, 1.0);
        assert!(s_ignite.ignite > 0.9, "ignite={}", s_ignite.ignite);
        assert!(s_ignite.disk_intensity > 0.9, "disk={}", s_ignite.disk_intensity);
        assert!(s_ignite.suction < 0.05, "suction={}", s_ignite.suction);
        assert!(s_ignite.horizon_radius_px > 6.3 && s_ignite.horizon_radius_px <= 7.1);

        // Succión monótona durante fase intermedia
        let mut prev_suction = 0.0;
        for step in 10..=76 {
            let t = step as f32 / 100.0;
            let sample = BlackHoleTimelineSample::sample(t, peak_r, 1.0);
            assert!(
                sample.suction >= prev_suction,
                "suction debe ser monótona en t={t}: {} < {}",
                sample.suction,
                prev_suction
            );
            prev_suction = sample.suction;
        }
        let s_swallowed = BlackHoleTimelineSample::sample(0.76, peak_r, 1.0);
        assert!(s_swallowed.suction > 0.99);

        // Colapso y desaparición total al final
        let s_mid_collapse = BlackHoleTimelineSample::sample(0.84, peak_r, 1.0);
        assert!(s_mid_collapse.pinch_flash > 0.1);
        assert!(s_mid_collapse.horizon_radius_px < peak_r);

        let s_end = BlackHoleTimelineSample::sample(1.0, peak_r, 1.0);
        assert_eq!(s_end.suction, 1.0);
        assert_eq!(s_end.collapse, 1.0);
        assert_eq!(s_end.horizon_radius_px, 0.0);
        assert_eq!(s_end.fade, 0.0);
        assert_eq!(s_end.disk_intensity, 0.0);
    }

    #[test]
    fn peak_horizon_radius_matches_close_button_across_scales() {
        let std_dpi = compute_peak_horizon_radius(Size::from((120, 90)), 1.0);
        assert!((std_dpi - 6.5).abs() < 1e-4);

        let hidpi = compute_peak_horizon_radius(Size::from((3840, 2160)), 2.0);
        assert!((hidpi - 13.0).abs() < 1e-4);
    }
}
