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

use std::sync::Arc;

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement},
        gles::{ffi, GlesError, GlesFrame, GlesRenderer},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer, Physical, Rectangle, Scale},
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
uniform float u_contact_strength;

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
    float contact = exp(-pow(d_contact / contact_sigma, 2.0)) * u_contact_strength;

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

const SHADOW_BLIT_VERT_SRC: &str = r#"#version 300 es
precision highp float;

uniform mat3 u_matrix;
uniform vec4 u_quad_rect;

out vec2 v_uv;

void main() {
    float u = (gl_VertexID == 1 || gl_VertexID == 3) ? 1.0 : 0.0;
    float v = (gl_VertexID == 2 || gl_VertexID == 3) ? 1.0 : 0.0;

    vec2 pos = u_quad_rect.xy + vec2(u * u_quad_rect.z, v * u_quad_rect.w);
    v_uv = vec2(u, v);

    vec3 ndc = u_matrix * vec3(pos, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
}
"#;

const SHADOW_BLIT_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
out vec4 frag_color;

uniform sampler2D u_texture;

void main() {
    frag_color = texture(u_texture, v_uv);
}
"#;

const SHADOW_SLOT_COUNT: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShadowCacheKey {
    w: i32,
    h: i32,
    corner_radius: u32,
    blur_radius: u32,
    spread: u32,
    offset_y: u32,
    opacity: u32,
    contact_strength: u32,
}

#[derive(Debug, Clone, Copy)]
struct ShadowSlot {
    texture: ffi::types::GLuint,
    fbo: ffi::types::GLuint,
    tex_size: (i32, i32),
    key: Option<ShadowCacheKey>,
    pad_x: i32,
    pad_y_top: i32,
    pad_y_bottom: i32,
    last_used: u64,
}

#[derive(Debug)]
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
    u_contact_strength: ffi::types::GLint,

    blit_program: ffi::types::GLuint,
    blit_u_matrix: ffi::types::GLint,
    blit_u_quad_rect: ffi::types::GLint,
    blit_u_texture: ffi::types::GLint,

    slots: std::sync::Mutex<Vec<ShadowSlot>>,
    use_counter: std::sync::atomic::AtomicU64,
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
            let u_contact_strength = gl.GetUniformLocation(
                program,
                c"u_contact_strength".as_ptr() as *const ffi::types::GLchar,
            );

            // Compilar shader de blit rápido para la caché FBO de sombras
            let blit_vert = compile_shader(gl, ffi::VERTEX_SHADER, SHADOW_BLIT_VERT_SRC)?;
            let blit_frag = compile_shader(gl, ffi::FRAGMENT_SHADER, SHADOW_BLIT_FRAG_SRC)?;

            let blit_program = gl.CreateProgram();
            gl.AttachShader(blit_program, blit_vert);
            gl.AttachShader(blit_program, blit_frag);
            gl.LinkProgram(blit_program);

            gl.DeleteShader(blit_vert);
            gl.DeleteShader(blit_frag);

            let mut blit_status = 0;
            gl.GetProgramiv(blit_program, ffi::LINK_STATUS, &mut blit_status);
            if blit_status == 0 {
                let mut len = 0;
                gl.GetProgramiv(blit_program, ffi::INFO_LOG_LENGTH, &mut len);
                let mut buffer = vec![0u8; len as usize];
                gl.GetProgramInfoLog(blit_program, len, std::ptr::null_mut(), buffer.as_mut_ptr() as *mut ffi::types::GLchar);
                error!("❌ [WindowShadowPipeline] Error en enlace de shader blit: {}", String::from_utf8_lossy(&buffer));
                gl.DeleteProgram(program);
                gl.DeleteProgram(blit_program);
                return None;
            }

            let blit_u_matrix = gl.GetUniformLocation(blit_program, c"u_matrix".as_ptr() as *const ffi::types::GLchar);
            let blit_u_quad_rect = gl.GetUniformLocation(blit_program, c"u_quad_rect".as_ptr() as *const ffi::types::GLchar);
            let blit_u_texture = gl.GetUniformLocation(blit_program, c"u_texture".as_ptr() as *const ffi::types::GLchar);

            info!("✨ [WindowShadowPipeline] Pipeline de sombras squircle superelípticas (n=4.2) con caché FBO inicializado en GPU");

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
                u_contact_strength,
                blit_program,
                blit_u_matrix,
                blit_u_quad_rect,
                blit_u_texture,
                slots: std::sync::Mutex::new(Vec::with_capacity(SHADOW_SLOT_COUNT)),
                use_counter: std::sync::atomic::AtomicU64::new(0),
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
                8.0 * scale as f32,
                0.0f32,
                2.0 * scale as f32,
                0.04f32,
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
            0.28,
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

    pub fn render_menu_shadow(
        &self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        menu_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        scale: f64,
    ) {
        let blur_radius = 28.0 * scale as f32;
        let spread = 0.0f32;
        let offset_y = 0.0f32; // Sombra central uniforme y difusa
        let opacity = 0.12f32;
        self.render_shadow_with_style(
            gl,
            projection,
            menu_rect,
            corner_radius,
            blur_radius,
            spread,
            offset_y,
            opacity,
            0.15,
        );
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
        contact_strength: f32,
    ) {
        if win_rect.size.w <= 0 || win_rect.size.h <= 0 {
            return;
        }

        // Expansión del quad de sombra para dar margen completo a la penumbra difusa:
        // Puesto que la luz viene de arriba, la sombra cae hacia abajo (+Y):
        let pad_x = (blur_radius * 2.5).round() as i32;
        let pad_y_top = (blur_radius * 1.8).round() as i32;
        let pad_y_bottom = (blur_radius * 2.8 + offset_y).round() as i32;

        let quad_w = win_rect.size.w + pad_x * 2;
        let quad_h = win_rect.size.h + pad_y_top + pad_y_bottom;
        if quad_w <= 0 || quad_h <= 0 {
            return;
        }

        let key = ShadowCacheKey {
            w: win_rect.size.w,
            h: win_rect.size.h,
            corner_radius: corner_radius.to_bits(),
            blur_radius: blur_radius.to_bits(),
            spread: spread.to_bits(),
            offset_y: offset_y.to_bits(),
            opacity: opacity.to_bits(),
            contact_strength: contact_strength.to_bits(),
        };

        let current_use = self
            .use_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;

        let mut slots_guard = match self.slots.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        // 1. Buscar si la sombra exacta ya está renderizada en la caché FBO
        let found_idx = slots_guard
            .iter()
            .position(|s| s.key == Some(key) && s.tex_size == (quad_w, quad_h));

        let slot_idx = if let Some(idx) = found_idx {
            slots_guard[idx].last_used = current_use;
            idx
        } else {
            // 2. Si no existe, seleccionar un slot disponible o el menos recientemente usado (LRU)
            let chosen_idx = if slots_guard.len() < SHADOW_SLOT_COUNT {
                let mut texture = 0;
                let mut fbo = 0;
                unsafe {
                    gl.GenTextures(1, &mut texture);
                    gl.GenFramebuffers(1, &mut fbo);
                }
                slots_guard.push(ShadowSlot {
                    texture,
                    fbo,
                    tex_size: (0, 0),
                    key: None,
                    pad_x,
                    pad_y_top,
                    pad_y_bottom,
                    last_used: current_use,
                });
                slots_guard.len() - 1
            } else {
                let mut lru_idx = 0;
                let mut oldest = u64::MAX;
                for (i, s) in slots_guard.iter().enumerate() {
                    if s.last_used < oldest {
                        oldest = s.last_used;
                        lru_idx = i;
                    }
                }
                slots_guard[lru_idx].last_used = current_use;
                lru_idx
            };

            let slot = &mut slots_guard[chosen_idx];

            unsafe {
                // Asegurar tamaño exacto de la textura del FBO
                if slot.tex_size != (quad_w, quad_h) {
                    gl.BindTexture(ffi::TEXTURE_2D, slot.texture);
                    gl.TexImage2D(
                        ffi::TEXTURE_2D,
                        0,
                        ffi::RGBA as i32,
                        quad_w,
                        quad_h,
                        0,
                        ffi::RGBA,
                        ffi::UNSIGNED_BYTE,
                        std::ptr::null(),
                    );
                    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                    gl.TexParameteri(
                        ffi::TEXTURE_2D,
                        ffi::TEXTURE_WRAP_S,
                        ffi::CLAMP_TO_EDGE as i32,
                    );
                    gl.TexParameteri(
                        ffi::TEXTURE_2D,
                        ffi::TEXTURE_WRAP_T,
                        ffi::CLAMP_TO_EDGE as i32,
                    );

                    gl.BindFramebuffer(ffi::FRAMEBUFFER, slot.fbo);
                    gl.FramebufferTexture2D(
                        ffi::FRAMEBUFFER,
                        ffi::COLOR_ATTACHMENT0,
                        ffi::TEXTURE_2D,
                        slot.texture,
                        0,
                    );
                    slot.tex_size = (quad_w, quad_h);
                }

                // Guardar estado GL previo
                let mut prev_fbo = 0;
                gl.GetIntegerv(ffi::FRAMEBUFFER_BINDING, &mut prev_fbo);
                let mut prev_viewport = [0; 4];
                gl.GetIntegerv(ffi::VIEWPORT, prev_viewport.as_mut_ptr());
                let prev_scissor = gl.IsEnabled(ffi::SCISSOR_TEST);

                // Renderizar la sombra SDF analítica en el FBO
                gl.BindFramebuffer(ffi::FRAMEBUFFER, slot.fbo);
                gl.Viewport(0, 0, quad_w, quad_h);
                gl.Disable(ffi::SCISSOR_TEST);
                gl.Disable(ffi::BLEND);
                gl.ClearColor(0.0, 0.0, 0.0, 0.0);
                gl.Clear(ffi::COLOR_BUFFER_BIT);

                gl.UseProgram(self.program);

                let fbo_matrix: [f32; 9] = [
                    2.0 / quad_w as f32, 0.0, 0.0,
                    0.0, 2.0 / quad_h as f32, 0.0,
                    -1.0, -1.0, 1.0,
                ];

                gl.UniformMatrix3fv(self.u_matrix, 1, ffi::FALSE, fbo_matrix.as_ptr());
                gl.Uniform4f(self.u_quad_rect, 0.0, 0.0, quad_w as f32, quad_h as f32);
                gl.Uniform2f(self.u_win_size, win_rect.size.w as f32, win_rect.size.h as f32);
                gl.Uniform2f(self.u_pad, pad_x as f32, pad_y_top as f32);
                gl.Uniform1f(self.u_corner_radius, corner_radius);
                gl.Uniform1f(self.u_exponent, 4.2f32);
                gl.Uniform1f(self.u_blur_radius, blur_radius);
                gl.Uniform1f(self.u_spread, spread);
                gl.Uniform1f(self.u_offset_y, offset_y);
                gl.Uniform1f(self.u_opacity, opacity);
                gl.Uniform1f(self.u_contact_strength, contact_strength);

                if gl.VertexAttribDivisor.is_loaded() {
                    gl.VertexAttribDivisor(0, 0);
                    gl.VertexAttribDivisor(1, 0);
                }
                gl.DisableVertexAttribArray(0);
                gl.DisableVertexAttribArray(1);
                gl.BindBuffer(ffi::ARRAY_BUFFER, 0);

                gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

                // Restaurar FBO y viewport de la pantalla
                gl.BindFramebuffer(ffi::FRAMEBUFFER, prev_fbo as u32);
                gl.Viewport(prev_viewport[0], prev_viewport[1], prev_viewport[2], prev_viewport[3]);
                if prev_scissor != ffi::FALSE {
                    gl.Enable(ffi::SCISSOR_TEST);
                }
            }

            slot.key = Some(key);
            slot.pad_x = pad_x;
            slot.pad_y_top = pad_y_top;
            slot.pad_y_bottom = pad_y_bottom;
            chosen_idx
        };

        let cached_slot = slots_guard[slot_idx];
        drop(slots_guard);

        // 3. Proyectar la sombra cacheada como quad texturizado ultraligero
        let quad_x = win_rect.loc.x - cached_slot.pad_x;
        let quad_y = win_rect.loc.y - cached_slot.pad_y_top;

        unsafe {
            let prev_blend = gl.IsEnabled(ffi::BLEND);
            let mut prev_src_rgb = 0;
            let mut prev_dst_rgb = 0;
            let mut prev_src_alpha = 0;
            let mut prev_dst_alpha = 0;
            gl.GetIntegerv(ffi::BLEND_SRC_RGB, &mut prev_src_rgb);
            gl.GetIntegerv(ffi::BLEND_DST_RGB, &mut prev_dst_rgb);
            gl.GetIntegerv(ffi::BLEND_SRC_ALPHA, &mut prev_src_alpha);
            gl.GetIntegerv(ffi::BLEND_DST_ALPHA, &mut prev_dst_alpha);

            gl.Enable(ffi::BLEND);
            gl.BlendFuncSeparate(
                ffi::SRC_ALPHA,
                ffi::ONE_MINUS_SRC_ALPHA,
                ffi::ZERO,
                ffi::ONE,
            );

            gl.UseProgram(self.blit_program);
            gl.UniformMatrix3fv(self.blit_u_matrix, 1, ffi::FALSE, projection.as_ptr());
            gl.Uniform4f(
                self.blit_u_quad_rect,
                quad_x as f32,
                quad_y as f32,
                cached_slot.tex_size.0 as f32,
                cached_slot.tex_size.1 as f32,
            );

            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, cached_slot.texture);
            gl.Uniform1i(self.blit_u_texture, 0);

            if gl.VertexAttribDivisor.is_loaded() {
                gl.VertexAttribDivisor(0, 0);
                gl.VertexAttribDivisor(1, 0);
            }
            gl.DisableVertexAttribArray(0);
            gl.DisableVertexAttribArray(1);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);

            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

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

/// RenderElement declarativo para sombras de ventana en Smithay.
///
/// Permite que `OutputDamageTracker` rastree los cambios de posición, tamaño,
/// foco y commits de la ventana, calculando el daño exacto y omitiendo el
/// redibujado de la sombra cuando la ventana está estática.
#[derive(Debug, Clone)]
pub struct WindowShadowRenderElement {
    id: Id,
    commit: CommitCounter,
    pipeline: Arc<WindowShadowPipeline>,
    win_rect: Rectangle<i32, Physical>,
    corner_radius: f32,
    is_active: bool,
    scale: f64,
    geometry: Rectangle<i32, Physical>,
    src: Rectangle<f64, Buffer>,
}

impl WindowShadowRenderElement {
    pub fn new(
        id: impl Into<Id>,
        commit: CommitCounter,
        pipeline: Arc<WindowShadowPipeline>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        is_active: bool,
        scale: f64,
    ) -> Self {
        let (blur_radius, offset_y) = if is_active {
            (36.0 * scale as f32, 10.0 * scale as f32)
        } else {
            (8.0 * scale as f32, 2.0 * scale as f32)
        };

        let pad_x = (blur_radius * 2.5).round() as i32;
        let pad_y_top = (blur_radius * 1.8).round() as i32;
        let pad_y_bottom = (blur_radius * 2.8 + offset_y).round() as i32;

        let quad_x = win_rect.loc.x - pad_x;
        let quad_y = win_rect.loc.y - pad_y_top;
        let quad_w = win_rect.size.w + pad_x * 2;
        let quad_h = win_rect.size.h + pad_y_top + pad_y_bottom;

        let geometry: Rectangle<i32, Physical> =
            Rectangle::new((quad_x, quad_y).into(), (quad_w, quad_h).into());
        let src: Rectangle<f64, Buffer> =
            Rectangle::from_size((quad_w as f64, quad_h as f64).into());

        Self {
            id: id.into(),
            commit,
            pipeline,
            win_rect,
            corner_radius,
            is_active,
            scale,
            geometry,
            src,
        }
    }
}

impl Element for WindowShadowRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // La sombra proyectada es translúcida en su totalidad
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit != Some(self.current_commit()) {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        } else {
            DamageSet::default()
        }
    }
}

impl RenderElement<GlesRenderer> for WindowShadowRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let projection = *frame.projection();
        frame.with_context(|gl| {
            self.pipeline.render_shadow(
                gl,
                &projection,
                self.win_rect,
                self.corner_radius,
                self.is_active,
                self.scale,
            );
        })
    }
}

/// RenderElement declarativo para la sombra proyectada del Dock.
#[derive(Debug, Clone)]
pub struct DockShadowRenderElement {
    id: Id,
    commit: CommitCounter,
    pipeline: Arc<WindowShadowPipeline>,
    dock_rect: Rectangle<i32, Physical>,
    corner_radius: f32,
    scale: f64,
    geometry: Rectangle<i32, Physical>,
    src: Rectangle<f64, Buffer>,
}

impl DockShadowRenderElement {
    pub fn new(
        id: impl Into<Id>,
        commit: CommitCounter,
        pipeline: Arc<WindowShadowPipeline>,
        dock_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        scale: f64,
    ) -> Self {
        let dock_shadow_pad = (80.0 * scale).round() as i32;
        let quad_x = dock_rect.loc.x - dock_shadow_pad;
        let quad_y = dock_rect.loc.y - dock_shadow_pad;
        let quad_w = dock_rect.size.w + dock_shadow_pad * 2;
        let quad_h = dock_rect.size.h + dock_shadow_pad * 2;

        let geometry: Rectangle<i32, Physical> =
            Rectangle::new((quad_x, quad_y).into(), (quad_w, quad_h).into());
        let src: Rectangle<f64, Buffer> =
            Rectangle::from_size((quad_w as f64, quad_h as f64).into());

        Self {
            id: id.into(),
            commit,
            pipeline,
            dock_rect,
            corner_radius,
            scale,
            geometry,
            src,
        }
    }
}

impl Element for DockShadowRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit != Some(self.current_commit()) {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        } else {
            DamageSet::default()
        }
    }
}

impl RenderElement<GlesRenderer> for DockShadowRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let projection = *frame.projection();
        frame.with_context(|gl| {
            self.pipeline.render_dock_shadow(
                gl,
                &projection,
                self.dock_rect,
                self.corner_radius,
                self.scale,
            );
        })
    }
}

/// RenderElement declarativo para sombras de menús contextuales y popups.
#[derive(Debug, Clone)]
pub struct MenuShadowRenderElement {
    id: Id,
    commit: CommitCounter,
    pipeline: Arc<WindowShadowPipeline>,
    menu_rect: Rectangle<i32, Physical>,
    corner_radius: f32,
    scale: f64,
    geometry: Rectangle<i32, Physical>,
    src: Rectangle<f64, Buffer>,
}

impl MenuShadowRenderElement {
    pub fn new(
        id: impl Into<Id>,
        commit: CommitCounter,
        pipeline: Arc<WindowShadowPipeline>,
        menu_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        scale: f64,
    ) -> Self {
        let blur_radius = 28.0 * scale as f32;
        let pad_x = (blur_radius * 2.5).round() as i32;
        let pad_y = (blur_radius * 2.5).round() as i32;

        let quad_x = menu_rect.loc.x - pad_x;
        let quad_y = menu_rect.loc.y - pad_y;
        let quad_w = menu_rect.size.w + pad_x * 2;
        let quad_h = menu_rect.size.h + pad_y * 2;

        let geometry: Rectangle<i32, Physical> =
            Rectangle::new((quad_x, quad_y).into(), (quad_w, quad_h).into());
        let src: Rectangle<f64, Buffer> =
            Rectangle::from_size((quad_w as f64, quad_h as f64).into());

        Self {
            id: id.into(),
            commit,
            pipeline,
            menu_rect,
            corner_radius,
            scale,
            geometry,
            src,
        }
    }
}

impl Element for MenuShadowRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit != Some(self.current_commit()) {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        } else {
            DamageSet::default()
        }
    }
}

impl RenderElement<GlesRenderer> for MenuShadowRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let projection = *frame.projection();
        frame.with_context(|gl| {
            self.pipeline.render_menu_shadow(
                gl,
                &projection,
                self.menu_rect,
                self.corner_radius,
                self.scale,
            );
        })
    }
}
