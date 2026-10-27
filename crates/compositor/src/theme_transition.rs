use smithay::{
    backend::renderer::gles::ffi,
    utils::{Physical, Point, Size},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const VERTEX_SHADER: &str = r#"#version 300 es
precision highp float;

out vec2 v_tex_coords;

void main() {
    float u = float(gl_VertexID & 1);
    float v = float(gl_VertexID >> 1);
    v_tex_coords = vec2(u, v);
    gl_Position = vec4(u * 2.0 - 1.0, v * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 300 es
precision highp float;

in vec2 v_tex_coords;
out vec4 frag_color;

uniform sampler2D u_snapshot;
uniform vec2 u_origin;
uniform float u_radius;

void main() {
    float distance_from_origin = distance(gl_FragCoord.xy, u_origin);
    float edge = max(fwidth(distance_from_origin) * 2.0, 1.0);
    float old_frame_alpha = smoothstep(u_radius - edge, u_radius + edge, distance_from_origin);
    vec4 old_frame = texture(u_snapshot, v_tex_coords);
    frag_color = vec4(old_frame.rgb, old_frame_alpha);
}
"#;

const DURATION: Duration = Duration::from_millis(620);

struct Snapshot {
    texture: ffi::types::GLuint,
    size: (i32, i32),
    origin: (f32, f32),
}

pub struct ThemeTransitionPipeline {
    program: ffi::types::GLuint,
    snapshot_uniform: ffi::types::GLint,
    origin_uniform: ffi::types::GLint,
    radius_uniform: ffi::types::GLint,
    snapshots: HashMap<String, Snapshot>,
}

impl ThemeTransitionPipeline {
    pub fn new(gl: &ffi::Gles2) -> Option<Self> {
        unsafe {
            let vertex = compile_shader(gl, ffi::VERTEX_SHADER, VERTEX_SHADER)?;
            let fragment = compile_shader(gl, ffi::FRAGMENT_SHADER, FRAGMENT_SHADER)?;
            let program = link_program(gl, vertex, fragment)?;
            gl.DeleteShader(vertex);
            gl.DeleteShader(fragment);

            Some(Self {
                program,
                snapshot_uniform: gl.GetUniformLocation(program, c"u_snapshot".as_ptr()),
                origin_uniform: gl.GetUniformLocation(program, c"u_origin".as_ptr()),
                radius_uniform: gl.GetUniformLocation(program, c"u_radius".as_ptr()),
                snapshots: HashMap::new(),
            })
        }
    }

    pub fn capture_snapshot(
        &mut self,
        gl: &ffi::Gles2,
        output_name: &str,
        size: Size<i32, Physical>,
        is_drm: bool,
        origin: Point<i32, Physical>,
    ) -> bool {
        if size.w <= 0 || size.h <= 0 {
            return false;
        }

        unsafe {
            let mut previous_active_texture = 0;
            let mut previous_texture = 0;
            let mut previous_read_buffer = 0;
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut previous_active_texture);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut previous_texture);
            gl.GetIntegerv(ffi::READ_BUFFER, &mut previous_read_buffer);

            let snapshot = self
                .snapshots
                .entry(output_name.to_string())
                .or_insert_with(|| {
                    let mut texture = 0;
                    gl.GenTextures(1, &mut texture);
                    Snapshot {
                        texture,
                        size: (0, 0),
                        origin: (0.0, 0.0),
                    }
                });
            gl.BindTexture(ffi::TEXTURE_2D, snapshot.texture);
            if snapshot.size != (size.w, size.h) {
                gl.TexImage2D(
                    ffi::TEXTURE_2D,
                    0,
                    ffi::RGBA as i32,
                    size.w,
                    size.h,
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
                snapshot.size = (size.w, size.h);
            }

            gl.ReadBuffer(if is_drm {
                ffi::COLOR_ATTACHMENT0
            } else {
                ffi::BACK
            });
            gl.CopyTexSubImage2D(ffi::TEXTURE_2D, 0, 0, 0, 0, 0, size.w, size.h);
            gl.ReadBuffer(previous_read_buffer as u32);
            gl.BindTexture(ffi::TEXTURE_2D, previous_texture as u32);
            gl.ActiveTexture(previous_active_texture as u32);

            snapshot.origin = (origin.x as f32, origin.y as f32);
        }

        true
    }

    pub fn draw(
        &self,
        gl: &ffi::Gles2,
        output_name: &str,
        size: Size<i32, Physical>,
        started_at: Instant,
        now: Instant,
    ) -> bool {
        let Some(snapshot) = self.snapshots.get(output_name) else {
            return false;
        };
        let elapsed = now.saturating_duration_since(started_at);
        if elapsed >= DURATION
            || size.w <= 0
            || size.h <= 0
            || snapshot.size != (size.w, size.h)
        {
            return false;
        }

        let progress = (elapsed.as_secs_f32() / DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let eased_progress = progress * progress * (3.0 - 2.0 * progress);
        let farthest_x = snapshot.origin.0.max(size.w as f32 - snapshot.origin.0);
        let farthest_y = snapshot.origin.1.max(size.h as f32 - snapshot.origin.1);
        let max_radius = farthest_x.hypot(farthest_y);

        unsafe {
            let mut previous_program = 0;
            let mut previous_viewport = [0; 4];
            let mut previous_active_texture = 0;
            gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut previous_program);
            gl.GetIntegerv(ffi::VIEWPORT, previous_viewport.as_mut_ptr());
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut previous_active_texture);

            gl.ActiveTexture(ffi::TEXTURE0);
            let mut previous_texture = 0;
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut previous_texture);

            let previous_blend = gl.IsEnabled(ffi::BLEND);
            let mut previous_src_rgb = 0;
            let mut previous_dst_rgb = 0;
            let mut previous_src_alpha = 0;
            let mut previous_dst_alpha = 0;
            gl.GetIntegerv(ffi::BLEND_SRC_RGB, &mut previous_src_rgb);
            gl.GetIntegerv(ffi::BLEND_DST_RGB, &mut previous_dst_rgb);
            gl.GetIntegerv(ffi::BLEND_SRC_ALPHA, &mut previous_src_alpha);
            gl.GetIntegerv(ffi::BLEND_DST_ALPHA, &mut previous_dst_alpha);
            let previous_scissor = gl.IsEnabled(ffi::SCISSOR_TEST);

            gl.Viewport(0, 0, size.w, size.h);
            gl.Disable(ffi::SCISSOR_TEST);
            gl.Enable(ffi::BLEND);
            gl.BlendFuncSeparate(
                ffi::SRC_ALPHA,
                ffi::ONE_MINUS_SRC_ALPHA,
                ffi::ONE,
                ffi::ONE_MINUS_SRC_ALPHA,
            );
            gl.UseProgram(self.program);
            gl.BindTexture(ffi::TEXTURE_2D, snapshot.texture);
            gl.Uniform1i(self.snapshot_uniform, 0);
            gl.Uniform2f(self.origin_uniform, snapshot.origin.0, snapshot.origin.1);
            gl.Uniform1f(self.radius_uniform, max_radius * eased_progress);
            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            gl.BindTexture(ffi::TEXTURE_2D, previous_texture as u32);
            gl.ActiveTexture(previous_active_texture as u32);
            gl.UseProgram(previous_program as u32);
            gl.Viewport(
                previous_viewport[0],
                previous_viewport[1],
                previous_viewport[2],
                previous_viewport[3],
            );
            gl.BlendFuncSeparate(
                previous_src_rgb as u32,
                previous_dst_rgb as u32,
                previous_src_alpha as u32,
                previous_dst_alpha as u32,
            );
            if previous_blend == ffi::FALSE {
                gl.Disable(ffi::BLEND);
            }
            if previous_scissor != ffi::FALSE {
                gl.Enable(ffi::SCISSOR_TEST);
            }
        }

        true
    }
}

unsafe fn compile_shader(
    gl: &ffi::Gles2,
    shader_type: ffi::types::GLenum,
    source: &str,
) -> Option<ffi::types::GLuint> {
    let shader = gl.CreateShader(shader_type);
    let source = std::ffi::CString::new(source).ok()?;
    gl.ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
    gl.CompileShader(shader);

    let mut compiled = 0;
    gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut compiled);
    if compiled == ffi::FALSE as i32 {
        let mut length = 0;
        gl.GetShaderiv(shader, ffi::INFO_LOG_LENGTH, &mut length);
        let mut log = vec![0_u8; length.max(1) as usize];
        gl.GetShaderInfoLog(
            shader,
            length,
            std::ptr::null_mut(),
            log.as_mut_ptr().cast(),
        );
        tracing::error!(
            "Error compilando shader de transición de tema: {}",
            String::from_utf8_lossy(&log)
        );
        gl.DeleteShader(shader);
        None
    } else {
        Some(shader)
    }
}

unsafe fn link_program(
    gl: &ffi::Gles2,
    vertex: ffi::types::GLuint,
    fragment: ffi::types::GLuint,
) -> Option<ffi::types::GLuint> {
    let program = gl.CreateProgram();
    gl.AttachShader(program, vertex);
    gl.AttachShader(program, fragment);
    gl.LinkProgram(program);

    let mut linked = 0;
    gl.GetProgramiv(program, ffi::LINK_STATUS, &mut linked);
    if linked == ffi::FALSE as i32 {
        let mut length = 0;
        gl.GetProgramiv(program, ffi::INFO_LOG_LENGTH, &mut length);
        let mut log = vec![0_u8; length.max(1) as usize];
        gl.GetProgramInfoLog(
            program,
            length,
            std::ptr::null_mut(),
            log.as_mut_ptr().cast(),
        );
        tracing::error!(
            "Error enlazando shader de transición de tema: {}",
            String::from_utf8_lossy(&log)
        );
        gl.DeleteProgram(program);
        None
    } else {
        Some(program)
    }
}

pub fn output_origin(
    origin: (f64, f64),
    output_loc: (i32, i32),
    scale: f64,
    size: Size<i32, Physical>,
) -> Point<i32, Physical> {
    let x = ((origin.0 - output_loc.0 as f64) * scale).round() as i32;
    let y = ((origin.1 - output_loc.1 as f64) * scale).round() as i32;
    Point::from((x.clamp(0, size.w), (size.h - y).clamp(0, size.h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_pointer_to_output_physical_coordinates() {
        let origin = output_origin((150.0, 120.0), (100, 50), 2.0, Size::from((400, 300)));

        assert_eq!(origin, Point::from((100, 160)));
    }

    #[test]
    fn clamps_pointer_outside_output_to_its_edge() {
        let origin = output_origin((500.0, -100.0), (100, 50), 1.0, Size::from((400, 300)));

        assert_eq!(origin, Point::from((400, 300)));
    }
}
