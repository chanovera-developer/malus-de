//! Módulo de recorte squircle (superelipse n=4.2) acelerado por GPU para ventanas en Malus DE.
//!
//! Aplica una máscara matemática en coordenadas UV locales de textura (v_coords)
//! a la superficie raíz de las ventanas (Electron, VS Code, Brave, Firefox, Antigravity, terminales, etc.)
//! eliminando completamente la dependencia de coordenadas de pantalla (gl_FragCoord)
//! para garantizar cero vibración, cero desfase al mover ventanas y máxima fidelidad estética estilo macOS.

use std::sync::Arc;

use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement,
            surface::{WaylandSurfaceRenderElement, WaylandSurfaceTexture},
            Element, Id, Kind, RenderElement, UnderlyingStorage,
        },
        gles::{
            GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
            UniformValue,
        },
        utils::{draw_render_elements, CommitCounter, DamageSet, OpaqueRegions},
        Texture,
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform},
};
use tracing::{error, info};

pub const WINDOW_SQUIRCLE_FRAG_SRC: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// Parámetros locales a nivel de textura / superficie (invariante ante movimiento, viewporter src crop e inversión Y)
uniform mat3 u_inv_tex_matrix; // Inversa de tex_matrix de Smithay: convierte v_coords a píxeles físicos en dest (0..dest.size)
uniform vec2 u_offset;         // Desplazamiento del área visible de la ventana dentro del elemento destino (en px)
uniform vec2 u_size;           // Dimensiones físicas del área visible de la ventana (ancho, alto en px)
uniform float u_corner_radius; // Radio squircle configurado (ej. 32.0 px)
uniform float u_exponent;      // Exponente superelíptico (4.2 para squircle macOS continuo)
uniform float u_border_strength;
uniform float u_preserve_outside;

// Distancia con signo analítica a un squircle superelíptico
float squircle_sdf(vec2 p, vec2 b, float r, float exp_val) {
    vec2 d = abs(p) - b + vec2(r);
    if (d.x <= 0.0 || d.y <= 0.0) {
        return max(d.x, d.y) - r;
    }
    return pow(pow(d.x, exp_val) + pow(d.y, exp_val), 1.0 / exp_val) - r;
}

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    if (u_corner_radius > 1.0 && u_size.x > 0.0 && u_size.y > 0.0) {
        // Coordenada exacta en píxeles físicos dentro del rectángulo destino (`dest`)
        vec2 px = (u_inv_tex_matrix * vec3(v_coords, 1.0)).xy;
        vec2 rect_min = u_offset;
        vec2 rect_max = u_offset + u_size;
        bool inside_rect = px.x >= rect_min.x && px.x <= rect_max.x && px.y >= rect_min.y && px.y <= rect_max.y;
        if (u_preserve_outside > 0.5 && !inside_rect) {
            gl_FragColor = color;
            return;
        }

        // Centro y semiejes del contenido visible de la ventana
        vec2 center = u_offset + u_size * 0.5;
        vec2 half_size = u_size * 0.5;

        // Vector centrado en el contenido visible (invariante ante movimiento o CSD shadow margins)
        vec2 p = px - center;

        float effective_r = min(u_corner_radius, min(half_size.x, half_size.y));
        float dist = squircle_sdf(p, half_size, effective_r, u_exponent);

        // Suavizado subpixel analítico (anti-aliasing nítido y libre de artefactos a 1px físico).
        // Cuando `u_preserve_outside > 0.5` (Dock con headroom), no atenuar el interior (`dist <= 0.0`):
        // de lo contrario la transición en `dist in [-edge_aa, 0]` dibuja una línea invisible de 1 px
        // justo en el borde superior donde los iconos con zoom cruzan hacia el headroom.
        float edge_aa = 1.0;
        float mask = (u_preserve_outside > 0.5)
            ? (1.0 - smoothstep(0.0, edge_aa, dist))
            : (1.0 - smoothstep(-edge_aa, 0.0, dist));

        // Fino bisel reflectante continuo estilo macOS (1px en el contorno exterior)
        float border_mask = smoothstep(-1.2 - edge_aa, -1.2, dist);
        vec3 highlight_col = vec3(1.0, 1.0, 1.0);
        color.rgb = mix(color.rgb, highlight_col, border_mask * 0.16 * u_border_strength);

        color = color * mask;
    }

    gl_FragColor = color;
}
"#;

/// Metadatos de muestreo de una textura Wayland para invertir con exactitud `tex_matrix`
/// cuando hay recorte `wp_viewporter` (`src != tex_size`), rotaciones o DMA-BUFs invertidos en Y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceTexParams {
    pub src: Rectangle<f64, Buffer>,
    pub tex_size: Size<i32, Buffer>,
    pub transform: Transform,
    pub y_inverted: bool,
}

fn mul_mat3_col_major(a: [f64; 9], b: [f64; 9]) -> [f64; 9] {
    let mut out = [0.0f64; 9];
    for col in 0..3 {
        for row in 0..3 {
            out[col * 3 + row] = a[0 * 3 + row] * b[col * 3 + 0]
                + a[1 * 3 + row] * b[col * 3 + 1]
                + a[2 * 3 + row] * b[col * 3 + 2];
        }
    }
    out
}

pub fn inv_tex_matrix_from_size(dest_size: Size<i32, Physical>) -> [f32; 9] {
    [
        dest_size.w.max(1) as f32,
        0.0,
        0.0,
        0.0,
        dest_size.h.max(1) as f32,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

impl SurfaceTexParams {
    pub fn from_wayland_element(elem: &WaylandSurfaceRenderElement<GlesRenderer>) -> Option<Self> {
        match elem.texture() {
            WaylandSurfaceTexture::Texture(tex) => Some(Self {
                src: elem.src(),
                tex_size: tex.size(),
                transform: elem.transform(),
                y_inverted: tex.is_y_inverted(),
            }),
            WaylandSurfaceTexture::SolidColor(_) => None,
        }
    }

    /// Calcula la matriz columna-mayor 3x3 inversa a `build_texture_mat` + `y_inverted` de Smithay.
    pub fn inv_tex_matrix(&self, dest_size: Size<i32, Physical>) -> [f32; 9] {
        if dest_size.w <= 0
            || dest_size.h <= 0
            || self.tex_size.w <= 0
            || self.tex_size.h <= 0
            || self.src.size.w <= 0.0
            || self.src.size.h <= 0.0
        {
            return inv_tex_matrix_from_size(dest_size);
        }

        let dst_src_size = self.transform.transform_size(self.src.size);
        let sx = dst_src_size.w / dest_size.w as f64;
        let sy = dst_src_size.h / dest_size.h as f64;

        let mut tex_mat = [sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0];

        let transform_mat = match self.transform {
            Transform::Normal => [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            Transform::_90 => [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            Transform::_180 => [-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0],
            Transform::_270 => [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            Transform::Flipped => [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            Transform::Flipped90 => [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            Transform::Flipped180 => [1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0],
            Transform::Flipped270 => [0.0, -1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        };
        let (tx, ty) = match self.transform {
            Transform::Normal => (0.0, 0.0),
            Transform::_90 => (0.0, dst_src_size.w),
            Transform::_180 => (dst_src_size.w, dst_src_size.h),
            Transform::_270 => (dst_src_size.h, 0.0),
            Transform::Flipped => (dst_src_size.w, 0.0),
            Transform::Flipped90 => (0.0, 0.0),
            Transform::Flipped180 => (0.0, dst_src_size.h),
            Transform::Flipped270 => (dst_src_size.h, dst_src_size.w),
        };
        tex_mat = mul_mat3_col_major(transform_mat, tex_mat);
        tex_mat = mul_mat3_col_major([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, 1.0], tex_mat);
        tex_mat = mul_mat3_col_major(
            [
                1.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                self.src.loc.x,
                self.src.loc.y,
                1.0,
            ],
            tex_mat,
        );
        tex_mat = mul_mat3_col_major(
            [
                1.0 / self.tex_size.w as f64,
                0.0,
                0.0,
                0.0,
                1.0 / self.tex_size.h as f64,
                0.0,
                0.0,
                0.0,
                1.0,
            ],
            tex_mat,
        );
        if self.y_inverted {
            tex_mat = mul_mat3_col_major([1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0], tex_mat);
        }

        let a = tex_mat[0];
        let b = tex_mat[1];
        let c = tex_mat[3];
        let d = tex_mat[4];
        let p = tex_mat[6];
        let q = tex_mat[7];
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return inv_tex_matrix_from_size(dest_size);
        }
        let inv_det = 1.0 / det;
        let ia = d * inv_det;
        let ib = -b * inv_det;
        let ic = -c * inv_det;
        let id = a * inv_det;
        let ip = -(ia * p + ic * q);
        let iq = -(ib * p + id * q);
        [
            ia as f32,
            ib as f32,
            0.0,
            ic as f32,
            id as f32,
            0.0,
            ip as f32,
            iq as f32,
            1.0,
        ]
    }
}

/// Cuando un cliente Chromium/Electron con `linux-dmabuf` usa una superficie raíz transparente 1x1
/// estirada vía `wp_viewporter` (`win_rect` se actualiza antes que la `wl_subsurface` GPU),
/// ajusta el rectángulo efectivo de sombra al área visible real dentro de `win_rect`.
pub fn effective_shadow_rect_for_window(
    win_surfaces: &[WaylandSurfaceRenderElement<GlesRenderer>],
    root_id: &Id,
    win_rect: Rectangle<i32, Physical>,
    scale: f64,
    is_translucent: bool,
    uses_ssd: bool,
) -> Rectangle<i32, Physical> {
    if uses_ssd || is_translucent || win_surfaces.len() <= 1 {
        return win_rect;
    }

    let Some(root_elem) = win_surfaces.iter().find(|elem| elem.id() == root_id) else {
        return win_rect;
    };
    let root_src = root_elem.src();
    let root_is_tiny_dummy = root_src.size.w <= 2.0 && root_src.size.h <= 2.0;
    let scale_f = Scale::from(scale);
    let root_has_opaque = !root_elem.opaque_regions(scale_f).is_empty();
    if !root_is_tiny_dummy || root_has_opaque {
        return win_rect;
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for elem in win_surfaces {
        if elem.id() == root_id {
            continue;
        }
        let geo = elem.geometry(scale_f);
        if geo.size.w <= 0 || geo.size.h <= 0 {
            continue;
        }
        if let Some(inter) = geo.intersection(win_rect) {
            if inter.size.w > 0 && inter.size.h > 0 {
                min_x = min_x.min(inter.loc.x);
                min_y = min_y.min(inter.loc.y);
                max_x = max_x.max(inter.loc.x + inter.size.w);
                max_y = max_y.max(inter.loc.y + inter.size.h);
            }
        }
    }

    if min_x < max_x && min_y < max_y {
        Rectangle::new(
            Point::from((min_x, min_y)),
            Size::from((max_x - min_x, max_y - min_y)),
        )
    } else {
        win_rect
    }
}

/// Determina si una `wl_subsurface` secundaria debe recortarse al contenedor `win_rect`
/// sin mutilar burbujas flotantes que sobresalen de la ventana (descargas, permisos, extensiones).
pub fn should_clip_subsurface_to_window(
    elem_geo: Rectangle<i32, Physical>,
    win_rect: Rectangle<i32, Physical>,
) -> bool {
    if elem_geo.size.w <= 0 || elem_geo.size.h <= 0 || win_rect.size.w <= 0 || win_rect.size.h <= 0
    {
        return false;
    }
    let elem_right = elem_geo.loc.x + elem_geo.size.w;
    let elem_bottom = elem_geo.loc.y + elem_geo.size.h;
    let win_right = win_rect.loc.x + win_rect.size.w;
    let win_bottom = win_rect.loc.y + win_rect.size.h;

    let contained_in_window = elem_geo.loc.x >= win_rect.loc.x - 2
        && elem_geo.loc.y >= win_rect.loc.y - 2
        && elem_right <= win_right + 2
        && elem_bottom <= win_bottom + 2;
    if contained_in_window {
        return true;
    }

    // Subsuperficie principal de contenido (p. ej. Chromium/Brave GPU) desfasada momentáneamente al encoger
    let anchored_to_window = (elem_geo.loc.x - win_rect.loc.x).abs() <= 4
        && (elem_geo.loc.y - win_rect.loc.y).abs() <= 160;
    let covers_major_body = elem_geo.size.w * 3 >= win_rect.size.w * 2
        && elem_geo.size.h * 3 >= win_rect.size.h * 2;
    anchored_to_window && covers_major_body
}

#[derive(Debug, Clone)]
pub struct WindowSquircleShader {
    program: GlesTexProgram,
}

impl WindowSquircleShader {
    pub fn new(renderer: &mut GlesRenderer) -> Option<Self> {
        let uniforms = [
            UniformName::new("u_inv_tex_matrix", UniformType::Matrix3x3),
            UniformName::new("u_offset", UniformType::_2f),
            UniformName::new("u_size", UniformType::_2f),
            UniformName::new("u_corner_radius", UniformType::_1f),
            UniformName::new("u_exponent", UniformType::_1f),
            UniformName::new("u_border_strength", UniformType::_1f),
            UniformName::new("u_preserve_outside", UniformType::_1f),
        ];

        match renderer.compile_custom_texture_shader(WINDOW_SQUIRCLE_FRAG_SRC, &uniforms) {
            Ok(program) => {
                info!("✨ [WindowSquircleShader] Pipeline de recorte Squircle UV local inicializado en GPU para ventanas");
                Some(Self { program })
            }
            Err(e) => {
                error!("❌ [WindowSquircleShader] Error al compilar shader custom de ventanas: {:?}", e);
                None
            }
        }
    }

    pub fn apply_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) {
        self.apply_surface_to_frame(frame, None, buf_size, offset, content_size, radius, 1.0, false);
    }

    pub fn apply_clip_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) {
        self.apply_surface_to_frame(frame, None, buf_size, offset, content_size, radius, 0.0, false);
    }

    pub fn apply_dock_clip_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) {
        self.apply_surface_to_frame(frame, None, buf_size, offset, content_size, radius, 0.0, true);
    }

    pub fn apply_wayland_element_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        elem: &WaylandSurfaceRenderElement<GlesRenderer>,
        dest_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
        border_strength: f32,
        preserve_outside: bool,
    ) {
        let tex_params = SurfaceTexParams::from_wayland_element(elem);
        self.apply_surface_to_frame(
            frame,
            tex_params,
            dest_size,
            offset,
            content_size,
            radius,
            border_strength,
            preserve_outside,
        );
    }

    pub fn apply_surface_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        tex_params: Option<SurfaceTexParams>,
        dest_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
        border_strength: f32,
        preserve_outside: bool,
    ) {
        let (actual_offset, actual_size) = if content_size.w > 0 && content_size.h > 0 {
            (offset, content_size)
        } else {
            (Point::from((0, 0)), dest_size)
        };
        let inv_mat = tex_params
            .map(|p| p.inv_tex_matrix(dest_size))
            .unwrap_or_else(|| inv_tex_matrix_from_size(dest_size));

        let uniforms = vec![
            Uniform::new(
                "u_inv_tex_matrix",
                UniformValue::Matrix3x3 {
                    matrices: vec![inv_mat],
                    transpose: false,
                },
            ),
            Uniform::new("u_offset", [actual_offset.x as f32, actual_offset.y as f32]),
            Uniform::new("u_size", [actual_size.w as f32, actual_size.h as f32]),
            Uniform::new("u_corner_radius", radius as f32),
            Uniform::new("u_exponent", 4.2f32),
            Uniform::new("u_border_strength", border_strength),
            Uniform::new("u_preserve_outside", if preserve_outside { 1.0f32 } else { 0.0f32 }),
        ];

        frame.override_default_tex_program(self.program.clone(), uniforms);
    }
}

/// Renderiza todas las superficies (y barra SSD opcional) de una ventana aplicando recorte squircle,
/// soporte exacto de `wp_viewporter` / `y_inverted` y recorte estricto 1:1 en `SnapTransition`.
#[allow(clippy::too_many_arguments)]
pub fn render_window_elements(
    frame: &mut GlesFrame<'_, '_>,
    scale: f64,
    damage: &[Rectangle<i32, Physical>],
    shader: Option<&WindowSquircleShader>,
    root_id: &Id,
    win_elems: &[WaylandSurfaceRenderElement<GlesRenderer>],
    ssd_elem: Option<&MemoryRenderBufferRenderElement<GlesRenderer>>,
    win_rect: Rectangle<i32, Physical>,
    win_phys_r: Option<i32>,
    should_round: bool,
    is_firefox: bool,
    transition_clip: Option<Rectangle<i32, Physical>>,
) {
    let scale_f = Scale::from(scale);
    let surface_rect = transition_clip.unwrap_or(win_rect);
    let clipped_surface_damage: Vec<Rectangle<i32, Physical>>;
    let surface_damage = if transition_clip.is_some() {
        clipped_surface_damage = damage
            .iter()
            .filter_map(|d| d.intersection(surface_rect))
            .collect();
        &clipped_surface_damage[..]
    } else {
        damage
    };
    let clipped_ssd_damage: Vec<Rectangle<i32, Physical>>;
    let ssd_damage = if transition_clip.is_some() {
        clipped_ssd_damage = damage
            .iter()
            .filter_map(|d| d.intersection(win_rect))
            .collect();
        &clipped_ssd_damage[..]
    } else {
        damage
    };

    let clip_target_rect = if ssd_elem.is_some() && transition_clip.is_some() {
        Rectangle::new(
            win_rect.loc,
            Size::from((
                surface_rect.size.w,
                (surface_rect.loc.y + surface_rect.size.h - win_rect.loc.y).max(0),
            )),
        )
    } else {
        surface_rect
    };

    let draw_surface = |frame: &mut GlesFrame<'_, '_>,
                        elem: &WaylandSurfaceRenderElement<GlesRenderer>,
                        clip_params: Option<(Point<i32, Physical>, Size<i32, Physical>, i32, f32)>| {
        if surface_damage.is_empty() {
            return;
        }
        let elem_geo = elem.geometry(scale_f);
        let applied_shader = if let (Some((offset, size, r, border_strength)), Some(sh)) =
            (clip_params, shader)
        {
            let tex_params = SurfaceTexParams::from_wayland_element(elem);
            sh.apply_surface_to_frame(
                frame,
                tex_params,
                elem_geo.size,
                offset,
                size,
                r,
                border_strength,
                false,
            );
            true
        } else {
            false
        };

        let _ = draw_render_elements(frame, scale, std::slice::from_ref(elem), surface_damage);

        if applied_shader {
            frame.clear_tex_program_override();
        }
    };

    if should_round {
        let clip_radius = if shader.is_some() { win_phys_r } else { None };
        let unified_tree_clip = is_firefox || ssd_elem.is_some();

        for elem in win_elems.iter().rev() {
            let clip_params = clip_radius.and_then(|r| {
                let elem_geo = elem.geometry(scale_f);
                if unified_tree_clip {
                    let sub_offset = Point::from((
                        clip_target_rect.loc.x - elem_geo.loc.x,
                        clip_target_rect.loc.y - elem_geo.loc.y,
                    ));
                    Some((sub_offset, clip_target_rect.size, r, 1.0))
                } else {
                    let is_root = elem.id() == root_id;
                    if is_root || should_clip_subsurface_to_window(elem_geo, clip_target_rect) {
                        let sub_offset = Point::from((
                            clip_target_rect.loc.x - elem_geo.loc.x,
                            clip_target_rect.loc.y - elem_geo.loc.y,
                        ));
                        let border_strength = if is_root || elem_geo == clip_target_rect {
                            1.0
                        } else {
                            0.0
                        };
                        Some((sub_offset, clip_target_rect.size, r, border_strength))
                    } else {
                        None
                    }
                }
            });
            draw_surface(frame, elem, clip_params);
        }

        if let Some(tb_elem) = ssd_elem {
            if !ssd_damage.is_empty() {
                if let (Some(r), Some(sh)) = (clip_radius, shader) {
                    let tb_geo = tb_elem.geometry(Scale::from(1.0));
                    let sub_offset = Point::from((
                        win_rect.loc.x - tb_geo.loc.x,
                        win_rect.loc.y - tb_geo.loc.y,
                    ));
                    sh.apply_to_frame(frame, tb_geo.size, sub_offset, win_rect.size, r);
                    let _ = draw_render_elements(frame, 1.0, std::slice::from_ref(tb_elem), ssd_damage);
                    frame.clear_tex_program_override();
                } else {
                    let _ = draw_render_elements(frame, 1.0, std::slice::from_ref(tb_elem), ssd_damage);
                }
            }
        }
    } else {
        if !surface_damage.is_empty() {
            let _ = draw_render_elements(frame, scale, win_elems, surface_damage);
        }
        if let Some(tb_elem) = ssd_elem {
            if !ssd_damage.is_empty() {
                let _ = draw_render_elements(frame, 1.0, std::slice::from_ref(tb_elem), ssd_damage);
            }
        }
    }
}

/// RenderElement declarativo que envuelve una superficie Wayland (`WaylandSurfaceRenderElement`)
/// y aplica el recorte Squircle (superelipse n=4.2) durante la pasada de renderizado de Smithay.
#[derive(Debug, Clone)]
pub struct SquircleSurfaceRenderElement<'a> {
    element: &'a WaylandSurfaceRenderElement<GlesRenderer>,
    shader: Option<Arc<WindowSquircleShader>>,
    buf_size: Size<i32, Physical>,
    offset: Point<i32, Physical>,
    content_size: Size<i32, Physical>,
    radius: i32,
    dock_clip: bool,
}

impl<'a> SquircleSurfaceRenderElement<'a> {
    pub fn new(
        element: &'a WaylandSurfaceRenderElement<GlesRenderer>,
        shader: Option<Arc<WindowSquircleShader>>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) -> Self {
        Self {
            element,
            shader,
            buf_size,
            offset,
            content_size,
            radius,
            dock_clip: false,
        }
    }

    pub fn with_dock_clip(
        element: &'a WaylandSurfaceRenderElement<GlesRenderer>,
        shader: Option<Arc<WindowSquircleShader>>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) -> Self {
        Self {
            element,
            shader,
            buf_size,
            offset,
            content_size,
            radius,
            dock_clip: true,
        }
    }
}

impl<'a> Element for SquircleSurfaceRenderElement<'a> {
    fn id(&self) -> &Id {
        self.element.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.element.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.element.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.element.geometry(scale)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if self.radius > 0 && self.shader.is_some() {
            OpaqueRegions::default()
        } else {
            self.element.opaque_regions(scale)
        }
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.element.damage_since(scale, commit)
    }
}

impl<'a> RenderElement<GlesRenderer> for SquircleSurfaceRenderElement<'a> {
    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.element.underlying_storage(renderer)
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        if let Some(shader) = &self.shader {
            shader.apply_wayland_element_to_frame(
                frame,
                self.element,
                self.buf_size,
                self.offset,
                self.content_size,
                self.radius,
                if self.dock_clip { 0.0 } else { 1.0 },
                self.dock_clip,
            );
            let res = self.element.draw(frame, src, dst, damage, opaque_regions);
            frame.clear_tex_program_override();
            res
        } else {
            self.element.draw(frame, src, dst, damage, opaque_regions)
        }
    }
}

/// RenderElement declarativo que envuelve un buffer SSD en memoria (`MemoryRenderBufferRenderElement`)
/// y aplica el recorte Squircle (superelipse n=4.2) durante la pasada de renderizado de Smithay.
#[derive(Debug, Clone)]
pub struct SquircleMemoryRenderElement<'a> {
    element: &'a MemoryRenderBufferRenderElement<GlesRenderer>,
    shader: Option<Arc<WindowSquircleShader>>,
    buf_size: Size<i32, Physical>,
    offset: Point<i32, Physical>,
    content_size: Size<i32, Physical>,
    radius: i32,
}

impl<'a> SquircleMemoryRenderElement<'a> {
    pub fn new(
        element: &'a MemoryRenderBufferRenderElement<GlesRenderer>,
        shader: Option<Arc<WindowSquircleShader>>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) -> Self {
        Self {
            element,
            shader,
            buf_size,
            offset,
            content_size,
            radius,
        }
    }
}

impl<'a> Element for SquircleMemoryRenderElement<'a> {
    fn id(&self) -> &Id {
        self.element.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.element.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.element.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.element.geometry(scale)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if self.radius > 0 && self.shader.is_some() {
            OpaqueRegions::default()
        } else {
            self.element.opaque_regions(scale)
        }
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.element.damage_since(scale, commit)
    }
}

impl<'a> RenderElement<GlesRenderer> for SquircleMemoryRenderElement<'a> {
    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.element.underlying_storage(renderer)
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        if let Some(shader) = &self.shader {
            shader.apply_to_frame(frame, self.buf_size, self.offset, self.content_size, self.radius);
            let res = self.element.draw(frame, src, dst, damage, opaque_regions);
            frame.clear_tex_program_override();
            res
        } else {
            self.element.draw(frame, src, dst, damage, opaque_regions)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_inv(mat: [f32; 9], u: f64, v: f64) -> (f64, f64) {
        let x = mat[0] as f64 * u + mat[3] as f64 * v + mat[6] as f64;
        let y = mat[1] as f64 * u + mat[4] as f64 * v + mat[7] as f64;
        (x, y)
    }

    #[test]
    fn test_inv_tex_matrix_viewporter_crop_and_y_inverted() {
        // Pooled 1600x1200 DMA-BUF cropped to 900x700 starting at (40, 20), stretched to dest 1000x800, y_inverted=true
        let params = SurfaceTexParams {
            src: Rectangle::new((40.0, 20.0).into(), (900.0, 700.0).into()),
            tex_size: Size::from((1600, 1200)),
            transform: Transform::Normal,
            y_inverted: true,
        };
        let inv = params.inv_tex_matrix(Size::from((1000, 800)));

        // Top-left UV in GL (40/1600, -20/1200) must map back to (0, 0) physical dest pixels
        let (x0, y0) = eval_inv(inv, 40.0 / 1600.0, -20.0 / 1200.0);
        assert!((x0 - 0.0).abs() < 1e-3, "x0={x0}");
        assert!((y0 - 0.0).abs() < 1e-3, "y0={y0}");

        // Bottom-right UV in GL (940/1600, -720/1200) must map back to (1000, 800) physical dest pixels
        let (x1, y1) = eval_inv(inv, 940.0 / 1600.0, -720.0 / 1200.0);
        assert!((x1 - 1000.0).abs() < 1e-3, "x1={x1}");
        assert!((y1 - 800.0).abs() < 1e-3, "y1={y1}");
    }

    #[test]
    fn test_inv_tex_matrix_with_transform_90() {
        let params = SurfaceTexParams {
            src: Rectangle::new((0.0, 0.0).into(), (800.0, 600.0).into()),
            tex_size: Size::from((800, 600)),
            transform: Transform::_90,
            y_inverted: false,
        };
        // With Transform::_90, dst_src_size = (600, 800). Map to dest (600, 800).
        // Position (0, 0) -> src (0, 600) in rotated space -> UV (0, 1) -> must map back to (0, 0).
        // Wait: Smithay _90 maps (x,y) -> (y, dst_src_size.w - x) = (y, 600 - x):
        // dest (0, 0) -> texel (0, 600) -> UV (0.0, 1.0)
        // dest (600, 800) -> texel (800, 0) -> UV (1.0, 0.0)
        let inv = params.inv_tex_matrix(Size::from((600, 800)));
        let (x0, y0) = eval_inv(inv, 0.0, 1.0);
        assert!((x0 - 0.0).abs() < 1e-3, "x0={x0}");
        assert!((y0 - 0.0).abs() < 1e-3, "y0={y0}");

        let (x1, y1) = eval_inv(inv, 1.0, 0.0);
        assert!((x1 - 600.0).abs() < 1e-3, "x1={x1}");
        assert!((y1 - 800.0).abs() < 1e-3, "y1={y1}");
    }

    #[test]
    fn test_should_clip_subsurface_to_window() {
        let win = Rectangle::new((100, 100).into(), (1000, 800).into());
        // Interior web content subsurface
        assert!(should_clip_subsurface_to_window(
            Rectangle::new((100, 180).into(), (1000, 720).into()),
            win,
        ));
        // Oversized Chromium GPU subsurface lagging during shrink
        assert!(should_clip_subsurface_to_window(
            Rectangle::new((100, 180).into(), (1250, 950).into()),
            win,
        ));
        // Floating extension/download bubble extending outside bottom/right of window
        assert!(!should_clip_subsurface_to_window(
            Rectangle::new((820, 160).into(), (340, 420).into()),
            win,
        ));
    }
}
