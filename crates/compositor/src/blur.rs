//! Pipeline de desenfoque de fondo acelerado por GPU (Dual Kawase Background Blur)
//!
//! Implementa el algoritmo Dual Kawase para desenfoque en tiempo real de alto rendimiento
//! con soporte para recorte squircle continuo (superelipse n=4.2) y anclaje proyectado.

use smithay::{
    backend::renderer::gles::ffi,
    utils::{Physical, Rectangle, Size},
};
use tracing::{error, info};

const QUAD_VERT_SRC: &str = r#"#version 300 es
precision highp float;

out vec2 v_tex_coords;

void main() {
    float u = (gl_VertexID == 1 || gl_VertexID == 3) ? 1.0 : 0.0;
    float v = (gl_VertexID == 2 || gl_VertexID == 3) ? 1.0 : 0.0;
    v_tex_coords = vec2(u, v);
    gl_Position = vec4(u * 2.0 - 1.0, v * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const DUAL_KAWASE_DOWN_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_tex_coords;
out vec4 frag_color;

uniform sampler2D u_texture;
uniform vec2 u_halfpixel;
uniform float u_offset;
uniform vec4 u_source_rect;

void main() {
    vec2 source_coords = u_source_rect.xy + v_tex_coords * u_source_rect.zw;
    vec4 sum = texture(u_texture, source_coords) * 4.0;
    sum += texture(u_texture, source_coords - u_halfpixel.xy * u_offset);
    sum += texture(u_texture, source_coords + u_halfpixel.xy * u_offset);
    sum += texture(u_texture, source_coords + vec2(u_halfpixel.x, -u_halfpixel.y) * u_offset);
    sum += texture(u_texture, source_coords - vec2(u_halfpixel.x, -u_halfpixel.y) * u_offset);
    frag_color = sum / 8.0;
}
"#;

const DUAL_KAWASE_UP_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_tex_coords;
out vec4 frag_color;

uniform sampler2D u_texture;
uniform vec2 u_halfpixel;
uniform float u_offset;

void main() {
    vec4 sum = vec4(0.0);
    sum += texture(u_texture, v_tex_coords + vec2(-u_halfpixel.x * 2.0, 0.0) * u_offset);
    sum += texture(u_texture, v_tex_coords + vec2(-u_halfpixel.x, u_halfpixel.y) * u_offset) * 2.0;
    sum += texture(u_texture, v_tex_coords + vec2(0.0, u_halfpixel.y * 2.0) * u_offset);
    sum += texture(u_texture, v_tex_coords + vec2(u_halfpixel.x, u_halfpixel.y) * u_offset) * 2.0;
    sum += texture(u_texture, v_tex_coords + vec2(u_halfpixel.x * 2.0, 0.0) * u_offset);
    sum += texture(u_texture, v_tex_coords + vec2(u_halfpixel.x, -u_halfpixel.y) * u_offset) * 2.0;
    sum += texture(u_texture, v_tex_coords + vec2(0.0, -u_halfpixel.y * 2.0) * u_offset);
    sum += texture(u_texture, v_tex_coords + vec2(-u_halfpixel.x, -u_halfpixel.y) * u_offset) * 2.0;
    frag_color = sum / 12.0;
}
"#;

const BLIT_VERT_SRC: &str = r#"#version 300 es
precision highp float;

uniform mat3 u_matrix;
uniform vec4 u_blit_rect; // (loc.x, loc.y, size.w, size.h) visible en coordenadas físicas de pantalla
uniform vec4 u_win_rect;  // (loc.x, loc.y, size.w, size.h) ventana completa
uniform int u_is_drm;

out vec2 v_tex_coords;
out vec2 v_win_px;

void main() {
    float u = (gl_VertexID == 1 || gl_VertexID == 3) ? 1.0 : 0.0;
    float v = (gl_VertexID == 2 || gl_VertexID == 3) ? 1.0 : 0.0;

    vec2 screen_pos = u_blit_rect.xy + vec2(u * u_blit_rect.z, v * u_blit_rect.w);
    v_win_px = screen_pos - u_win_rect.xy;

    float tex_v = (u_is_drm == 1) ? v : (1.0 - v);
    v_tex_coords = vec2(u, tex_v);

    vec3 ndc = u_matrix * vec3(screen_pos, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
}
"#;

const BLIT_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_tex_coords;
in vec2 v_win_px;
out vec4 frag_color;

uniform sampler2D u_texture;
uniform float u_solid_mode;
uniform vec4 u_solid_color;
uniform vec2 u_win_size;
uniform float u_corner_radius;
uniform float u_corner_exponent;

float rounded_shape_sdf(vec2 p, vec2 b, float r) {
    vec2 d = abs(p) - b + vec2(r);
    if (d.x <= 0.0 || d.y <= 0.0) {
        return max(d.x, d.y) - r;
    }
    return pow(pow(d.x, u_corner_exponent) + pow(d.y, u_corner_exponent), 1.0 / u_corner_exponent) - r;
}

void main() {
    if (u_solid_mode > 0.5) {
        float alpha = 1.0;
        if (u_corner_radius > 1.0) {
            vec2 half_size = u_win_size * 0.5;
            vec2 p = v_win_px - half_size;
            float effective_r = min(u_corner_radius, min(half_size.x, half_size.y));
            float dist = rounded_shape_sdf(p, half_size, effective_r);
            float edge_aa = max(fwidth(dist), 1.0);
            alpha = 1.0 - smoothstep(-edge_aa, 0.0, dist);
        }
        frag_color = vec4(u_solid_color.rgb, u_solid_color.a * alpha);
        return;
    }

    vec4 col = texture(u_texture, v_tex_coords);
    if (u_corner_radius > 1.0) {
        vec2 half_size = u_win_size * 0.5;
        vec2 p = v_win_px - half_size;
        float effective_r = min(u_corner_radius, min(half_size.x, half_size.y));
        float dist = rounded_shape_sdf(p, half_size, effective_r);
        float edge_aa = max(fwidth(dist), 1.0);
        float alpha = 1.0 - smoothstep(-edge_aa, 0.0, dist);
        frag_color = vec4(col.rgb, alpha);
    } else {
        frag_color = vec4(col.rgb, 1.0);
    }
}
"#;

pub struct DualKawaseBlur {
    down_program: ffi::types::GLuint,
    up_program: ffi::types::GLuint,
    blit_program: ffi::types::GLuint,

    down_u_texture: ffi::types::GLint,
    down_u_halfpixel: ffi::types::GLint,
    down_u_offset: ffi::types::GLint,
    down_u_source_rect: ffi::types::GLint,

    up_u_texture: ffi::types::GLint,
    up_u_halfpixel: ffi::types::GLint,
    up_u_offset: ffi::types::GLint,

    blit_u_matrix: ffi::types::GLint,
    blit_u_blit_rect: ffi::types::GLint,
    blit_u_win_rect: ffi::types::GLint,
    blit_u_win_size: ffi::types::GLint,
    blit_u_is_drm: ffi::types::GLint,
    blit_u_texture: ffi::types::GLint,
    blit_u_corner_radius: ffi::types::GLint,
    blit_u_corner_exponent: ffi::types::GLint,
    blit_u_solid_mode: ffi::types::GLint,
    blit_u_solid_color: ffi::types::GLint,

    source_tex: ffi::types::GLuint,
    source_tex_size: (i32, i32),
    snapshot_tex: ffi::types::GLuint,
    snapshot_size: (i32, i32),

    fbo_textures: [ffi::types::GLuint; 2],
    fbos: [ffi::types::GLuint; 2],
    fbo_sizes: [(i32, i32); 2],
}

impl DualKawaseBlur {
    pub fn new(gl: &ffi::Gles2) -> Option<Self> {
        unsafe {
            let quad_vert = compile_shader(gl, ffi::VERTEX_SHADER, QUAD_VERT_SRC)?;
            let down_frag = compile_shader(gl, ffi::FRAGMENT_SHADER, DUAL_KAWASE_DOWN_SRC)?;
            let up_frag = compile_shader(gl, ffi::FRAGMENT_SHADER, DUAL_KAWASE_UP_SRC)?;

            let blit_vert = compile_shader(gl, ffi::VERTEX_SHADER, BLIT_VERT_SRC)?;
            let blit_frag = compile_shader(gl, ffi::FRAGMENT_SHADER, BLIT_FRAG_SRC)?;

            let down_program = link_program(gl, quad_vert, down_frag)?;
            let up_program = link_program(gl, quad_vert, up_frag)?;
            let blit_program = link_program(gl, blit_vert, blit_frag)?;

            gl.DeleteShader(quad_vert);
            gl.DeleteShader(down_frag);
            gl.DeleteShader(up_frag);
            gl.DeleteShader(blit_vert);
            gl.DeleteShader(blit_frag);

            let down_u_texture = gl.GetUniformLocation(down_program, c"u_texture".as_ptr());
            let down_u_halfpixel = gl.GetUniformLocation(down_program, c"u_halfpixel".as_ptr());
            let down_u_offset = gl.GetUniformLocation(down_program, c"u_offset".as_ptr());
            let down_u_source_rect = gl.GetUniformLocation(down_program, c"u_source_rect".as_ptr());

            let up_u_texture = gl.GetUniformLocation(up_program, c"u_texture".as_ptr());
            let up_u_halfpixel = gl.GetUniformLocation(up_program, c"u_halfpixel".as_ptr());
            let up_u_offset = gl.GetUniformLocation(up_program, c"u_offset".as_ptr());

            let blit_u_matrix = gl.GetUniformLocation(blit_program, c"u_matrix".as_ptr());
            let blit_u_blit_rect = gl.GetUniformLocation(blit_program, c"u_blit_rect".as_ptr());
            let blit_u_win_rect = gl.GetUniformLocation(blit_program, c"u_win_rect".as_ptr());
            let blit_u_win_size = gl.GetUniformLocation(blit_program, c"u_win_size".as_ptr());
            let blit_u_is_drm = gl.GetUniformLocation(blit_program, c"u_is_drm".as_ptr());
            let blit_u_texture = gl.GetUniformLocation(blit_program, c"u_texture".as_ptr());
            let blit_u_corner_radius =
                gl.GetUniformLocation(blit_program, c"u_corner_radius".as_ptr());
            let blit_u_corner_exponent =
                gl.GetUniformLocation(blit_program, c"u_corner_exponent".as_ptr());
            let blit_u_solid_mode = gl.GetUniformLocation(blit_program, c"u_solid_mode".as_ptr());
            let blit_u_solid_color = gl.GetUniformLocation(blit_program, c"u_solid_color".as_ptr());

            let mut source_tex = 0;
            gl.GenTextures(1, &mut source_tex);
            let mut snapshot_tex = 0;
            gl.GenTextures(1, &mut snapshot_tex);

            let mut fbos = [0; 2];
            let mut fbo_textures = [0; 2];
            gl.GenFramebuffers(2, fbos.as_mut_ptr());
            gl.GenTextures(2, fbo_textures.as_mut_ptr());

            info!("✨ [DualKawaseBlur] Pipeline de desenfoque por GPU inicializado con éxito.");

            Some(Self {
                down_program,
                up_program,
                blit_program,
                down_u_texture,
                down_u_halfpixel,
                down_u_offset,
                down_u_source_rect,
                up_u_texture,
                up_u_halfpixel,
                up_u_offset,
                blit_u_matrix,
                blit_u_blit_rect,
                blit_u_win_rect,
                blit_u_win_size,
                blit_u_is_drm,
                blit_u_texture,
                blit_u_corner_radius,
                blit_u_corner_exponent,
                blit_u_solid_mode,
                blit_u_solid_color,
                source_tex,
                source_tex_size: (0, 0),
                snapshot_tex,
                snapshot_size: (0, 0),
                fbo_textures,
                fbos,
                fbo_sizes: [(0, 0), (0, 0)],
            })
        }
    }

    /// Aplica desenfoque Dual Kawase a la región de pantalla de la ventana dada
    pub fn render_blur(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
    ) {
        self.render_blur_with_exponent(
            gl,
            projection,
            screen_size,
            win_rect,
            corner_radius,
            4.2,
            false,
        );
    }

    /// Aplica desenfoque con esquinas circulares a la región de pantalla indicada.
    pub fn render_blur_round(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
    ) {
        self.render_blur_with_exponent(
            gl,
            projection,
            screen_size,
            win_rect,
            corner_radius,
            2.0,
            false,
        );
    }

    /// Captura el fondo sin blur para que las capas translúcidas no se contaminen entre sí.
    pub fn capture_background(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
    ) {
        if screen_size.w < 1 || screen_size.h < 1 {
            return;
        }

        unsafe {
            let mut prev_active_texture = 0;
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut prev_active_texture);
            gl.ActiveTexture(ffi::TEXTURE0);
            let mut prev_texture = 0;
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut prev_texture);

            let mut prev_read_buffer = 0;
            gl.GetIntegerv(ffi::READ_BUFFER, &mut prev_read_buffer);
            let is_drm = projection[4] > 0.0;
            gl.ReadBuffer(if is_drm {
                ffi::COLOR_ATTACHMENT0
            } else {
                ffi::BACK
            });

            gl.BindTexture(ffi::TEXTURE_2D, self.snapshot_tex);
            if self.snapshot_size != (screen_size.w, screen_size.h) {
                gl.TexImage2D(
                    ffi::TEXTURE_2D,
                    0,
                    ffi::RGBA as i32,
                    screen_size.w,
                    screen_size.h,
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
                self.snapshot_size = (screen_size.w, screen_size.h);
            }
            gl.CopyTexSubImage2D(ffi::TEXTURE_2D, 0, 0, 0, 0, 0, screen_size.w, screen_size.h);
            gl.ReadBuffer(prev_read_buffer as u32);
            gl.BindTexture(ffi::TEXTURE_2D, prev_texture as u32);
            gl.ActiveTexture(prev_active_texture as u32);
        }
    }

    pub fn render_captured_blur(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
    ) {
        self.render_blur_with_exponent(
            gl,
            projection,
            screen_size,
            win_rect,
            corner_radius,
            4.2,
            true,
        );
    }

    pub fn render_captured_blur_round(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
    ) {
        self.render_blur_with_exponent(
            gl,
            projection,
            screen_size,
            win_rect,
            corner_radius,
            2.0,
            true,
        );
    }

    fn render_blur_with_exponent(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        corner_exponent: f32,
        from_capture: bool,
    ) {
        let screen_rect = Rectangle::from_size(screen_size);
        let visible_rect = match win_rect.intersection(screen_rect) {
            Some(r) if r.size.w >= 4 && r.size.h >= 4 => r,
            _ => return,
        };

        let w = visible_rect.size.w;
        let h = visible_rect.size.h;

        // En DRM (Transform::Normal), la matriz de proyección mapea Y positivo hacia abajo en NDC
        // y el framebuffer tiene origen superior (y = 0 en la parte superior de pantalla).
        // En Winit (Transform::Flipped180), la coordenada GL está invertida respecto al origen Wayland.
        let is_drm = projection[4] > 0.0;
        let copy_x = visible_rect.loc.x;
        let copy_y = if is_drm {
            visible_rect.loc.y
        } else {
            screen_size.h - (visible_rect.loc.y + h)
        };
        if from_capture && self.snapshot_size != (screen_size.w, screen_size.h) {
            return;
        }
        let (source_size, source_rect) = if from_capture {
            (
                (screen_size.w, screen_size.h),
                [
                    copy_x as f32 / screen_size.w as f32,
                    copy_y as f32 / screen_size.h as f32,
                    w as f32 / screen_size.w as f32,
                    h as f32 / screen_size.h as f32,
                ],
            )
        } else {
            ((w, h), [0.0, 0.0, 1.0, 1.0])
        };

        unsafe {
            // 1. Guardar estado GL previo
            let mut prev_fbo = 0;
            let mut prev_viewport = [0; 4];
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

            gl.GetIntegerv(ffi::FRAMEBUFFER_BINDING, &mut prev_fbo);
            gl.GetIntegerv(ffi::VIEWPORT, prev_viewport.as_mut_ptr());

            // 2. Asegurar tamaño exacto de textura origen
            if !from_capture && self.source_tex_size != (w, h) {
                gl.BindTexture(ffi::TEXTURE_2D, self.source_tex);
                gl.TexImage2D(
                    ffi::TEXTURE_2D,
                    0,
                    ffi::RGBA as i32,
                    w,
                    h,
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
                self.source_tex_size = (w, h);
            }

            // 3. Copiar región del framebuffer de pantalla actual a source_tex
            if from_capture {
                gl.BindTexture(ffi::TEXTURE_2D, self.snapshot_tex);
            } else {
                gl.BindTexture(ffi::TEXTURE_2D, self.source_tex);
                let mut prev_read_buffer = 0;
                gl.GetIntegerv(ffi::READ_BUFFER, &mut prev_read_buffer);
                gl.ReadBuffer(if is_drm {
                    ffi::COLOR_ATTACHMENT0
                } else {
                    ffi::BACK
                });
                gl.CopyTexSubImage2D(ffi::TEXTURE_2D, 0, 0, 0, copy_x, copy_y, w, h);
                gl.ReadBuffer(prev_read_buffer as u32);
            }

            // 4. Asegurar tamaños exactos de texturas FBO intermedias
            let w0 = (w / 2).max(2);
            let h0 = (h / 2).max(2);
            let w1 = (w / 4).max(2);
            let h1 = (h / 4).max(2);

            let targets = [(w0, h0), (w1, h1)];
            for i in 0..2 {
                if self.fbo_sizes[i] != targets[i] {
                    gl.BindTexture(ffi::TEXTURE_2D, self.fbo_textures[i]);
                    gl.TexImage2D(
                        ffi::TEXTURE_2D,
                        0,
                        ffi::RGBA as i32,
                        targets[i].0,
                        targets[i].1,
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

                    gl.BindFramebuffer(ffi::FRAMEBUFFER, self.fbos[i]);
                    gl.FramebufferTexture2D(
                        ffi::FRAMEBUFFER,
                        ffi::COLOR_ATTACHMENT0,
                        ffi::TEXTURE_2D,
                        self.fbo_textures[i],
                        0,
                    );
                    self.fbo_sizes[i] = targets[i];
                }
            }

            if gl.VertexAttribDivisor.is_loaded() {
                gl.VertexAttribDivisor(0, 0);
                gl.VertexAttribDivisor(1, 0);
            }
            gl.DisableVertexAttribArray(0);
            gl.DisableVertexAttribArray(1);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
            gl.Disable(ffi::SCISSOR_TEST);
            gl.Disable(ffi::CULL_FACE);
            gl.Disable(ffi::BLEND);

            // PASO 1: Downsample inicial desde source_tex -> FBO 0
            gl.BindFramebuffer(ffi::FRAMEBUFFER, self.fbos[0]);
            gl.Viewport(0, 0, w0, h0);
            gl.UseProgram(self.down_program);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(
                ffi::TEXTURE_2D,
                if from_capture {
                    self.snapshot_tex
                } else {
                    self.source_tex
                },
            );
            gl.Uniform1i(self.down_u_texture, 0);
            gl.Uniform2f(
                self.down_u_halfpixel,
                0.5 / source_size.0 as f32,
                0.5 / source_size.1 as f32,
            );
            gl.Uniform1f(self.down_u_offset, 5.0);
            gl.Uniform4f(
                self.down_u_source_rect,
                source_rect[0],
                source_rect[1],
                source_rect[2],
                source_rect[3],
            );
            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            // PASO 2: Downsample adicional desde FBO 0 -> FBO 1
            gl.BindFramebuffer(ffi::FRAMEBUFFER, self.fbos[1]);
            gl.Viewport(0, 0, w1, h1);
            gl.BindTexture(ffi::TEXTURE_2D, self.fbo_textures[0]);
            gl.Uniform2f(self.down_u_halfpixel, 0.5 / w0 as f32, 0.5 / h0 as f32);
            gl.Uniform1f(self.down_u_offset, 8.0);
            gl.Uniform4f(self.down_u_source_rect, 0.0, 0.0, 1.0, 1.0);
            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            // PASO 3: Upsample desde FBO 1 -> FBO 0
            gl.BindFramebuffer(ffi::FRAMEBUFFER, self.fbos[0]);
            gl.Viewport(0, 0, w0, h0);
            gl.UseProgram(self.up_program);
            gl.BindTexture(ffi::TEXTURE_2D, self.fbo_textures[1]);
            gl.Uniform1i(self.up_u_texture, 0);
            gl.Uniform2f(self.up_u_halfpixel, 0.5 / w1 as f32, 0.5 / h1 as f32);
            gl.Uniform1f(self.up_u_offset, 8.0);
            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            // PASO 4: Blit final desde FBO 0 hacia la pantalla con soporte squircle y anclaje proyectado
            gl.BindFramebuffer(ffi::FRAMEBUFFER, prev_fbo as u32);
            gl.Viewport(
                prev_viewport[0],
                prev_viewport[1],
                prev_viewport[2],
                prev_viewport[3],
            );
            gl.Enable(ffi::BLEND);
            gl.BlendFuncSeparate(
                ffi::SRC_ALPHA,
                ffi::ONE_MINUS_SRC_ALPHA,
                ffi::ZERO,
                ffi::ONE,
            );

            gl.UseProgram(self.blit_program);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, self.fbo_textures[0]);
            gl.Uniform1i(self.blit_u_texture, 0);

            gl.UniformMatrix3fv(self.blit_u_matrix, 1, ffi::FALSE, projection.as_ptr());
            gl.Uniform4f(
                self.blit_u_blit_rect,
                visible_rect.loc.x as f32,
                visible_rect.loc.y as f32,
                visible_rect.size.w as f32,
                visible_rect.size.h as f32,
            );
            gl.Uniform4f(
                self.blit_u_win_rect,
                win_rect.loc.x as f32,
                win_rect.loc.y as f32,
                win_rect.size.w as f32,
                win_rect.size.h as f32,
            );
            gl.Uniform2f(
                self.blit_u_win_size,
                win_rect.size.w as f32,
                win_rect.size.h as f32,
            );
            gl.Uniform1i(self.blit_u_is_drm, if is_drm { 1 } else { 0 });
            gl.Uniform1f(self.blit_u_corner_radius, corner_radius);
            gl.Uniform1f(self.blit_u_corner_exponent, corner_exponent);
            gl.Uniform1f(self.blit_u_solid_mode, 0.0);

            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            // 5. Restaurar estado GL previo
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            gl.UseProgram(0);

            if prev_blend == ffi::FALSE {
                gl.Disable(ffi::BLEND);
            } else {
                gl.BlendFuncSeparate(
                    prev_src_rgb as u32,
                    prev_dst_rgb as u32,
                    prev_src_alpha as u32,
                    prev_dst_alpha as u32,
                );
            }
            if prev_scissor != ffi::FALSE {
                gl.Enable(ffi::SCISSOR_TEST);
            }
            if prev_cull != ffi::FALSE {
                gl.Enable(ffi::CULL_FACE);
            }
        }
    }

    /// Dibuja un fondo opaco de tema bajo una ventana, conservando el recorte squircle.
    pub fn render_solid(
        &mut self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        screen_size: Size<i32, Physical>,
        win_rect: Rectangle<i32, Physical>,
        corner_radius: f32,
        color: [f32; 3],
    ) {
        let screen_rect = Rectangle::from_size(screen_size);
        let Some(visible_rect) = win_rect.intersection(screen_rect) else {
            return;
        };
        if visible_rect.size.w < 1 || visible_rect.size.h < 1 {
            return;
        }

        let is_drm = projection[4] > 0.0;

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
            gl.Uniform1i(self.blit_u_texture, 0);
            gl.Uniform1f(self.blit_u_solid_mode, 1.0);
            gl.Uniform4f(self.blit_u_solid_color, color[0], color[1], color[2], 1.0);
            gl.Uniform1f(self.blit_u_corner_exponent, 4.2);
            gl.UniformMatrix3fv(self.blit_u_matrix, 1, ffi::FALSE, projection.as_ptr());
            gl.Uniform4f(
                self.blit_u_blit_rect,
                visible_rect.loc.x as f32,
                visible_rect.loc.y as f32,
                visible_rect.size.w as f32,
                visible_rect.size.h as f32,
            );
            gl.Uniform4f(
                self.blit_u_win_rect,
                win_rect.loc.x as f32,
                win_rect.loc.y as f32,
                win_rect.size.w as f32,
                win_rect.size.h as f32,
            );
            gl.Uniform2f(
                self.blit_u_win_size,
                win_rect.size.w as f32,
                win_rect.size.h as f32,
            );
            gl.Uniform1i(self.blit_u_is_drm, if is_drm { 1 } else { 0 });
            gl.Uniform1f(self.blit_u_corner_radius, corner_radius);
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

impl Drop for DualKawaseBlur {
    fn drop(&mut self) {
        // En Rust/Smithay la limpieza de recursos GL debe hacerse con el contexto EGL activo
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
            "❌ [DualKawaseBlur] Error al compilar shader GLSL: {}",
            String::from_utf8_lossy(&buffer)
        );
        gl.DeleteShader(shader);
        return None;
    }
    Some(shader)
}

unsafe fn link_program(
    gl: &ffi::Gles2,
    vert: ffi::types::GLuint,
    frag: ffi::types::GLuint,
) -> Option<ffi::types::GLuint> {
    let program = gl.CreateProgram();
    gl.AttachShader(program, vert);
    gl.AttachShader(program, frag);
    gl.LinkProgram(program);

    let mut success = 0;
    gl.GetProgramiv(program, ffi::LINK_STATUS, &mut success);
    if success == 0 {
        let mut len = 0;
        gl.GetProgramiv(program, ffi::INFO_LOG_LENGTH, &mut len);
        let mut buffer: Vec<u8> = vec![0; len as usize];
        gl.GetProgramInfoLog(
            program,
            len,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut ffi::types::GLchar,
        );
        error!(
            "❌ [DualKawaseBlur] Error al enlazar programa GLSL: {}",
            String::from_utf8_lossy(&buffer)
        );
        gl.DeleteProgram(program);
        return None;
    }
    Some(program)
}
