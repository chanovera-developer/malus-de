//! Módulo de recorte squircle (superelipse n=4.2) acelerado por GPU para ventanas en Malus DE.
//!
//! Aplica una máscara matemática en coordenadas UV locales de textura (v_coords)
//! a la superficie raíz de las ventanas (Electron, VS Code, Brave, Firefox, Antigravity, terminales, etc.)
//! eliminando completamente la dependencia de coordenadas de pantalla (gl_FragCoord)
//! para garantizar cero vibración, cero desfase al mover ventanas y máxima fidelidad estética estilo macOS.

use smithay::{
    backend::renderer::gles::{
        GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    },
    utils::{Physical, Point, Size},
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

// Parámetros locales a nivel de textura / superficie (completamente invariante ante movimiento de ventana)
uniform vec2 u_buf_size;       // Dimensiones físicas totales del buffer/textura (ancho, alto en px)
uniform vec2 u_offset;         // Desplazamiento del área visible de la ventana dentro del buffer (en px)
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

    if (u_corner_radius > 1.0 && u_size.x > 0.0 && u_size.y > 0.0 && u_buf_size.x > 0.0 && u_buf_size.y > 0.0) {
        // Coordenada en píxeles físicos dentro del buffer de textura
        vec2 px = v_coords * u_buf_size;
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

        // Suavizado subpixel analítico (anti-aliasing nítido y libre de artefactos a 1px físico)
        float edge_aa = 1.0;
        float mask = 1.0 - smoothstep(-edge_aa, 0.0, dist);

        // Fino bisel reflectante continuo estilo macOS (1px en el contorno exterior)
        float border_mask = smoothstep(-1.2 - edge_aa, -1.2, dist);
        vec3 highlight_col = vec3(1.0, 1.0, 1.0);
        color.rgb = mix(color.rgb, highlight_col, border_mask * 0.16 * u_border_strength);

        color = color * mask;
    }

    gl_FragColor = color;
}
"#;

pub struct WindowSquircleShader {
    program: GlesTexProgram,
}

impl WindowSquircleShader {
    pub fn new(renderer: &mut GlesRenderer) -> Option<Self> {
        let uniforms = [
            UniformName::new("u_buf_size", UniformType::_2f),
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
        self.apply_to_frame_with_border(frame, buf_size, offset, content_size, radius, 1.0, false);
    }

    pub fn apply_clip_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) {
        self.apply_to_frame_with_border(frame, buf_size, offset, content_size, radius, 0.0, false);
    }

    pub fn apply_dock_clip_to_frame(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
    ) {
        self.apply_to_frame_with_border(frame, buf_size, offset, content_size, radius, 0.0, true);
    }

    fn apply_to_frame_with_border(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        buf_size: Size<i32, Physical>,
        offset: Point<i32, Physical>,
        content_size: Size<i32, Physical>,
        radius: i32,
        border_strength: f32,
        preserve_outside: bool,
    ) {
        let (actual_offset, actual_size) = if content_size.w > 0 && content_size.h > 0 {
            (offset, content_size)
        } else {
            (Point::from((0, 0)), buf_size)
        };

        let uniforms = vec![
            Uniform::new("u_buf_size", [buf_size.w as f32, buf_size.h as f32]),
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
