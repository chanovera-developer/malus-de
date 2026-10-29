//! Pipeline de sombras proyectadas de ventanas (Window Drop Shadows) aceleradas por GPU.
//!
//! Genera sombras volumétricas ultra suaves estilo macOS con modelo analítico SDF
//! siguiendo exactamente la superelipse continua (Squircle n=4.2) de cada ventana.
//!
//! Características:
//! - Modelo de doble lóbulo (Dual-Lobe): sombra de contacto nítida + penumbra difusa ambiental.
//! - Recorte analítico interno (Inner Cutout) para preservar transparencia y acrílico.
//! - Diferenciación dinámica entre ventana activa (enfocada) e inactivas.
//! - Cero asignación de texturas intermedias y renderizado a 60/120 FPS constante.

use smithay::{
    backend::renderer::gles::ffi,
    utils::{Physical, Rectangle},
};
use tracing::{error, info};

const SHADOW_VERT_SRC: &str = r#"#version 300 es
precision highp float;

uniform mat3 u_matrix;
uniform vec4 u_quad_rect; // (loc.x, loc.y, size.w, size.h) en coordenadas físicas de pantalla

out vec2 v_local_px; // Coordenadas locales en píxeles dentro del quad de sombra (0..w, 0..h)

void main() {
    // Generación analítica de quad directamente desde gl_VertexID (0..3) con TRIANGLE_STRIP:
    // 0: (0, 0) - superior-izquierda
    // 1: (1, 0) - superior-derecha
    // 2: (0, 1) - inferior-izquierda
    // 3: (1, 1) - inferior-derecha
    float u = (gl_VertexID == 1 || gl_VertexID == 3) ? 1.0 : 0.0;
    float v = (gl_VertexID == 2 || gl_VertexID == 3) ? 1.0 : 0.0;

    vec2 screen_pos = u_quad_rect.xy + vec2(u * u_quad_rect.z, v * u_quad_rect.w);
    v_local_px = vec2(u * u_quad_rect.z, v * u_quad_rect.w);

    vec3 ndc = u_matrix * vec3(screen_pos, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
}
"#;

const SHADOW_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_local_px;
out vec4 frag_color;

uniform vec2 u_win_size;        // Dimensiones físicas de la ventana visible
uniform vec2 u_pad;             // Margen de expansión (pad_x, pad_y_top)
uniform float u_corner_radius;  // Radio squircle de la ventana (ej. 32.0 px)
uniform float u_exponent;       // Exponente superelíptico (4.2 continuo)
uniform float u_blur_radius;    // Radio de dispersión gaussiana
uniform float u_spread;         // Expansión sólida de la sombra
uniform float u_offset_y;       // Desplazamiento de luz cenital (luz hacia abajo, u_offset_y > 0)
uniform float u_opacity;        // Opacidad pico de la sombra

// Distancia con signo analítica a un rectángulo con esquinas circulares perfectas (Euclidiana L2)
float rounded_box_sdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Distancia con signo analítica a un squircle superelíptico (n=4.2) para el recorte interior
float squircle_sdf(vec2 p, vec2 b, float r, float exp_val) {
    vec2 d = abs(p) - b + vec2(r);
    if (d.x <= 0.0 || d.y <= 0.0) {
        return max(d.x, d.y) - r;
    }
    return pow(pow(d.x, exp_val) + pow(d.y, exp_val), 1.0 / exp_val) - r;
}

void main() {
    // Centro físico y semiejes de la ventana dentro del quad de sombra
    vec2 win_center = u_pad + u_win_size * 0.5;
    vec2 win_half = u_win_size * 0.5;
    float effective_r = min(u_corner_radius, min(win_half.x, win_half.y));

    // 1. Recorte interior (Inner Cutout): la sombra sólo existe FUERA del contorno visible de la ventana.
    // Transición suave [-1.5, 0.0] que desliza la sombra justo bajo el borde anti-aliased
    // de la ventana para garantizar cero desfases ni líneas blancas visibles.
    vec2 p_win = v_local_px - win_center;
    float d_win = squircle_sdf(p_win, win_half, effective_r, u_exponent);
    float inner_mask = smoothstep(-1.5, 0.0, d_win);

    if (inner_mask <= 0.001) {
        discard;
    }

    // 2. Modelo de doble lóbulo estilo macOS (Sequoia / Sonoma):
    // Usamos distancia euclidiana L2 pura (rounded_box_sdf) para la difusión de la sombra,
    // garantizando que las esquinas de la penumbra se abran en abanico circular perfecto
    // y sin aristas ni picos diagonales.
    //
    // - Sombra de contacto: sutil y cercana al contorno
    vec2 contact_center = win_center + vec2(0.0, u_offset_y * 0.2);
    vec2 p_contact = v_local_px - contact_center;
    float d_contact = max(rounded_box_sdf(p_contact, win_half, effective_r), 0.0);
    float contact_sigma = max(u_blur_radius * 0.22, 2.0);
    float contact = exp(-pow(d_contact / contact_sigma, 2.0)) * 0.28;

    // - Sombra ambiental: amplia y etérea penumbra difusa que cae suavemente hacia abajo
    vec2 ambient_center = win_center + vec2(0.0, u_offset_y);
    vec2 p_ambient = v_local_px - ambient_center;
    vec2 ambient_half = win_half + vec2(u_spread);
    float d_ambient = max(rounded_box_sdf(p_ambient, ambient_half, effective_r + u_spread), 0.0);
    float ambient_sigma = max(u_blur_radius * 0.85, 6.0);
    float ambient = exp(-pow(d_ambient / ambient_sigma, 2.0)) * 0.72;

    float shadow_intensity = contact + ambient;
    float final_alpha = shadow_intensity * u_opacity * inner_mask;

    if (final_alpha <= 0.001) {
        discard;
    }

    frag_color = vec4(0.0, 0.0, 0.0, final_alpha);
}
"#;

pub struct WindowShadowPipeline {
    program: ffi::types::GLuint,
    u_matrix: ffi::types::GLint,
    u_quad_rect: ffi::types::GLint,
    u_win_size: ffi::types::GLint,
    u_pad: ffi::types::GLint,
    u_corner_radius: ffi::types::GLint,
    u_exponent: ffi::types::GLint,
    u_blur_radius: ffi::types::GLint,
    u_spread: ffi::types::GLint,
    u_offset_y: ffi::types::GLint,
    u_opacity: ffi::types::GLint,
}

impl WindowShadowPipeline {
    pub fn new(gl: &ffi::Gles2) -> Option<Self> {
        unsafe {
            let vert = compile_shader(gl, ffi::VERTEX_SHADER, SHADOW_VERT_SRC)?;
            let frag = compile_shader(gl, ffi::FRAGMENT_SHADER, SHADOW_FRAG_SRC)?;

            let program = gl.CreateProgram();
            gl.AttachShader(program, vert);
            gl.AttachShader(program, frag);
            gl.LinkProgram(program);

            gl.DeleteShader(vert);
            gl.DeleteShader(frag);

            let mut status = 0;
            gl.GetProgramiv(program, ffi::LINK_STATUS, &mut status);
            if status == 0 {
                let mut len = 0;
                gl.GetProgramiv(program, ffi::INFO_LOG_LENGTH, &mut len);
                let mut buffer = vec![0u8; len as usize];
                gl.GetProgramInfoLog(program, len, std::ptr::null_mut(), buffer.as_mut_ptr() as *mut ffi::types::GLchar);
                error!("❌ [WindowShadowPipeline] Error en enlace de shaders: {}", String::from_utf8_lossy(&buffer));
                gl.DeleteProgram(program);
                return None;
            }

            let u_matrix = gl.GetUniformLocation(program, c"u_matrix".as_ptr() as *const ffi::types::GLchar);
            let u_quad_rect = gl.GetUniformLocation(program, c"u_quad_rect".as_ptr() as *const ffi::types::GLchar);
            let u_win_size = gl.GetUniformLocation(program, c"u_win_size".as_ptr() as *const ffi::types::GLchar);
            let u_pad = gl.GetUniformLocation(program, c"u_pad".as_ptr() as *const ffi::types::GLchar);
            let u_corner_radius = gl.GetUniformLocation(program, c"u_corner_radius".as_ptr() as *const ffi::types::GLchar);
            let u_exponent = gl.GetUniformLocation(program, c"u_exponent".as_ptr() as *const ffi::types::GLchar);
            let u_blur_radius = gl.GetUniformLocation(program, c"u_blur_radius".as_ptr() as *const ffi::types::GLchar);
            let u_spread = gl.GetUniformLocation(program, c"u_spread".as_ptr() as *const ffi::types::GLchar);
            let u_offset_y = gl.GetUniformLocation(program, c"u_offset_y".as_ptr() as *const ffi::types::GLchar);
            let u_opacity = gl.GetUniformLocation(program, c"u_opacity".as_ptr() as *const ffi::types::GLchar);

            info!("✨ [WindowShadowPipeline] Pipeline de sombras squircle superelípticas (n=4.2) inicializado en GPU");

            Some(Self {
                program,
                u_matrix,
                u_quad_rect,
                u_win_size,
                u_pad,
                u_corner_radius,
                u_exponent,
                u_blur_radius,
                u_spread,
                u_offset_y,
                u_opacity,
            })
        }
    }

    /// Renderiza la sombra proyectada de una ventana
    pub fn render_shadow(
        &self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        is_active: bool,
        scale: f64,
    ) {
        // Parámetros de sombra calibrados para un efecto más discreto y menos intenso.
        // - Opacidad reducida para que la penumbra sea más sutil
        // - Luz cenital superior: la sombra se proyecta suavemente hacia abajo (offset_y)
        // - Penumbra amplia y circular que aporta flotabilidad tridimensional limpia y orgánica
        let (blur_radius, spread, offset_y, opacity) = if is_active {
            (
                36.0 * scale as f32,
                0.0f32,
                10.0 * scale as f32,
                0.12f32,
            )
        } else {
            (
                18.0 * scale as f32,
                0.0f32,
                4.0 * scale as f32,
                0.06f32,
            )
        };

        self.render_shadow_with_style(
            gl,
            projection,
            win_rect,
            corner_radius,
            blur_radius,
            spread,
            offset_y,
            opacity,
        );
    }

    pub fn render_dock_shadow(
        &self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        dock_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        scale: f64,
    ) {
        self.render_shadow(gl, projection, dock_rect, corner_radius, true, scale);
    }

    #[allow(clippy::too_many_arguments)]
    fn render_shadow_with_style(
        &self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        blur_radius: f32,
        spread: f32,
        offset_y: f32,
        opacity: f32,
    ) {
        if win_rect.size.w <= 0 || win_rect.size.h <= 0 {
            return;
        }

        // Expansión del quad de sombra para dar margen completo a la penumbra difusa:
        // Puesto que la luz viene de arriba, la sombra cae hacia abajo (+Y):
        let pad_x = (blur_radius * 2.5).round() as i32;
        let pad_y_top = (blur_radius * 1.8).round() as i32;
        let pad_y_bottom = (blur_radius * 2.8 + offset_y).round() as i32;

        let quad_x = win_rect.loc.x - pad_x;
        let quad_y = win_rect.loc.y - pad_y_top;
        let quad_w = win_rect.size.w + pad_x * 2;
        let quad_h = win_rect.size.h + pad_y_top + pad_y_bottom;

        unsafe {
            // Guardar estado GL previo
            let prev_blend = gl.IsEnabled(ffi::BLEND);
            let prev_scissor = gl.IsEnabled(ffi::SCISSOR_TEST);
            let prev_cull = gl.IsEnabled(ffi::CULL_FACE);

            let mut prev_src_rgb = 0;
            let mut prev_dst_rgb = 0;
            let mut prev_src_alpha = 0;
            let mut prev_dst_alpha = 0;
            gl.GetIntegerv(ffi::BLEND_SRC_RGB, &mut prev_src_rgb);
            gl.GetIntegerv(ffi::BLEND_DST_RGB, &mut prev_dst_rgb);
            gl.GetIntegerv(ffi::BLEND_SRC_ALPHA, &mut prev_src_alpha);
            gl.GetIntegerv(ffi::BLEND_DST_ALPHA, &mut prev_dst_alpha);

            // Desactivar scissor previo para que la penumbra no sea recortada
            gl.Disable(ffi::SCISSOR_TEST);
            gl.Disable(ffi::CULL_FACE);

            // Blending que oscurece el fondo suavemente según alpha sin degradar el canal alfa del framebuffer (mantiene 1.0)
            gl.Enable(ffi::BLEND);
            gl.BlendFuncSeparate(
                ffi::SRC_ALPHA,
                ffi::ONE_MINUS_SRC_ALPHA,
                ffi::ZERO,
                ffi::ONE,
            );

            gl.UseProgram(self.program);

            gl.UniformMatrix3fv(self.u_matrix, 1, ffi::FALSE, projection.as_ptr());
            gl.Uniform4f(self.u_quad_rect, quad_x as f32, quad_y as f32, quad_w as f32, quad_h as f32);
            gl.Uniform2f(self.u_win_size, win_rect.size.w as f32, win_rect.size.h as f32);
            gl.Uniform2f(self.u_pad, pad_x as f32, pad_y_top as f32);
            gl.Uniform1f(self.u_corner_radius, corner_radius);
            gl.Uniform1f(self.u_exponent, 4.2f32);
            gl.Uniform1f(self.u_blur_radius, blur_radius);
            gl.Uniform1f(self.u_spread, spread);
            gl.Uniform1f(self.u_offset_y, offset_y);
            gl.Uniform1f(self.u_opacity, opacity);

            // Desactivar atributos previos para renderizar directamente vía gl_VertexID procedural
            // e impedir cualquier interferencia de VertexAttribDivisor previo de Smithay
            if gl.VertexAttribDivisor.is_loaded() {
                gl.VertexAttribDivisor(0, 0);
                gl.VertexAttribDivisor(1, 0);
            }
            gl.DisableVertexAttribArray(0);
            gl.DisableVertexAttribArray(1);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);

            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            // Restaurar estado GL previo
            gl.UseProgram(0);

            gl.BlendFuncSeparate(
                prev_src_rgb as u32,
                prev_dst_rgb as u32,
                prev_src_alpha as u32,
                prev_dst_alpha as u32,
            );
            if prev_blend == ffi::FALSE {
                gl.Disable(ffi::BLEND);
            }
            if prev_scissor != ffi::FALSE {
                gl.Enable(ffi::SCISSOR_TEST);
            }
            if prev_cull != ffi::FALSE {
                gl.Enable(ffi::CULL_FACE);
            }
        }
    }
}

unsafe fn compile_shader(
    gl: &ffi::Gles2,
    shader_type: ffi::types::GLenum,
    src: &str,
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
        let mut buffer: Vec<u8> = vec![0; len as usize];
        gl.GetShaderInfoLog(
            shader,
            len,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut ffi::types::GLchar,
        );
        error!(
            "❌ [WindowShadowPipeline] Error compilando shader: {}",
            String::from_utf8_lossy(&buffer)
        );
        gl.DeleteShader(shader);
        return None;
    }

    Some(shader)
}
