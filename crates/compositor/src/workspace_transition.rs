//! Transición animada horizontal estilo macOS Spaces al cambiar entre escritorios virtuales.

use crate::theme_transition::{compile_shader, link_program};
use smithay::{
    backend::renderer::gles::ffi,
    utils::{Physical, Size},
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

uniform sampler2D u_outgoing;
uniform sampler2D u_incoming;
uniform float u_progress;
uniform float u_direction;
uniform float u_gap;

void main() {
    float shift = u_progress * (1.0 + u_gap) * u_direction;
    float out_u = v_tex_coords.x + shift;
    float in_u = v_tex_coords.x + shift - u_direction * (1.0 + u_gap);

    float out_dim = mix(1.0, 0.88, u_progress);
    float in_dim = mix(0.92, 1.0, u_progress);

    vec3 color = vec3(0.025, 0.028, 0.035);
    if (out_u >= 0.0 && out_u <= 1.0) {
        color = texture(u_outgoing, vec2(out_u, v_tex_coords.y)).rgb * out_dim;
    } else if (in_u >= 0.0 && in_u <= 1.0) {
        color = texture(u_incoming, vec2(in_u, v_tex_coords.y)).rgb * in_dim;
    }

    frag_color = vec4(color, 1.0);
}
"#;

pub const DURATION: Duration = Duration::from_millis(340);
pub const DESKTOP_GAP_RATIO: f32 = 0.022;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingWorkspaceSwitch {
    pub target_index: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkspaceTransitionState {
    pub started_at: Instant,
    /// `+1.0` al avanzar hacia la derecha (el saliente sale a la izquierda), `-1.0` al retroceder.
    pub direction: f32,
}

struct OutputSnapshots {
    outgoing_tex: ffi::types::GLuint,
    incoming_tex: ffi::types::GLuint,
    size: (i32, i32),
    has_outgoing: bool,
}

pub struct WorkspaceTransitionPipeline {
    program: ffi::types::GLuint,
    outgoing_uniform: ffi::types::GLint,
    incoming_uniform: ffi::types::GLint,
    progress_uniform: ffi::types::GLint,
    direction_uniform: ffi::types::GLint,
    gap_uniform: ffi::types::GLint,
    snapshots: HashMap<String, OutputSnapshots>,
}

impl WorkspaceTransitionPipeline {
    pub fn new(gl: &ffi::Gles2) -> Option<Self> {
        unsafe {
            let vertex = compile_shader(gl, ffi::VERTEX_SHADER, VERTEX_SHADER)?;
            let fragment = compile_shader(gl, ffi::FRAGMENT_SHADER, FRAGMENT_SHADER)?;
            let program = link_program(gl, vertex, fragment)?;
            gl.DeleteShader(vertex);
            gl.DeleteShader(fragment);

            Some(Self {
                program,
                outgoing_uniform: gl.GetUniformLocation(program, c"u_outgoing".as_ptr()),
                incoming_uniform: gl.GetUniformLocation(program, c"u_incoming".as_ptr()),
                progress_uniform: gl.GetUniformLocation(program, c"u_progress".as_ptr()),
                direction_uniform: gl.GetUniformLocation(program, c"u_direction".as_ptr()),
                gap_uniform: gl.GetUniformLocation(program, c"u_gap".as_ptr()),
                snapshots: HashMap::new(),
            })
        }
    }

    /// Captura el escritorio saliente actual antes de intercambiar las ventanas en `Space`.
    pub fn capture_outgoing(
        &mut self,
        gl: &ffi::Gles2,
        output_name: &str,
        size: Size<i32, Physical>,
        is_drm: bool,
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

            let entry = self
                .snapshots
                .entry(output_name.to_string())
                .or_insert_with(|| {
                    let mut textures = [0_u32; 2];
                    gl.GenTextures(2, textures.as_mut_ptr());
                    OutputSnapshots {
                        outgoing_tex: textures[0],
                        incoming_tex: textures[1],
                        size: (0, 0),
                        has_outgoing: false,
                    }
                });

            ensure_snapshot_textures(gl, entry, size);
            gl.BindTexture(ffi::TEXTURE_2D, entry.outgoing_tex);
            gl.ReadBuffer(if is_drm {
                ffi::COLOR_ATTACHMENT0
            } else {
                ffi::BACK
            });
            gl.CopyTexSubImage2D(ffi::TEXTURE_2D, 0, 0, 0, 0, 0, size.w, size.h);
            entry.has_outgoing = true;

            gl.ReadBuffer(previous_read_buffer as u32);
            gl.BindTexture(ffi::TEXTURE_2D, previous_texture as u32);
            gl.ActiveTexture(previous_active_texture as u32);
        }

        true
    }

    /// Dibuja la transición combinando la captura del escritorio saliente con el framebuffer entrante.
    pub fn draw(
        &mut self,
        gl: &ffi::Gles2,
        output_name: &str,
        size: Size<i32, Physical>,
        is_drm: bool,
        transition: WorkspaceTransitionState,
        now: Instant,
    ) -> bool {
        let Some(entry) = self.snapshots.get_mut(output_name) else {
            return false;
        };
        let elapsed = now.saturating_duration_since(transition.started_at);
        if elapsed >= DURATION
            || !entry.has_outgoing
            || size.w <= 0
            || size.h <= 0
            || entry.size != (size.w, size.h)
        {
            entry.has_outgoing = false;
            return false;
        }

        let eased = eased_progress(elapsed);

        unsafe {
            let mut previous_program = 0;
            let mut previous_viewport = [0; 4];
            let mut previous_active_texture = 0;
            let mut previous_read_buffer = 0;
            gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut previous_program);
            gl.GetIntegerv(ffi::VIEWPORT, previous_viewport.as_mut_ptr());
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut previous_active_texture);
            gl.GetIntegerv(ffi::READ_BUFFER, &mut previous_read_buffer);

            gl.ActiveTexture(ffi::TEXTURE0);
            let mut previous_tex0 = 0;
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut previous_tex0);

            gl.ActiveTexture(ffi::TEXTURE1);
            let mut previous_tex1 = 0;
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut previous_tex1);

            let previous_blend = gl.IsEnabled(ffi::BLEND);
            let previous_scissor = gl.IsEnabled(ffi::SCISSOR_TEST);

            // Capturar el escritorio entrante recién renderizado en el framebuffer actual
            gl.BindTexture(ffi::TEXTURE_2D, entry.incoming_tex);
            gl.ReadBuffer(if is_drm {
                ffi::COLOR_ATTACHMENT0
            } else {
                ffi::BACK
            });
            gl.CopyTexSubImage2D(ffi::TEXTURE_2D, 0, 0, 0, 0, 0, size.w, size.h);

            gl.Viewport(0, 0, size.w, size.h);
            gl.Disable(ffi::SCISSOR_TEST);
            gl.Disable(ffi::BLEND);
            gl.UseProgram(self.program);

            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, entry.outgoing_tex);
            gl.Uniform1i(self.outgoing_uniform, 0);

            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, entry.incoming_tex);
            gl.Uniform1i(self.incoming_uniform, 1);

            gl.Uniform1f(self.progress_uniform, eased);
            gl.Uniform1f(self.direction_uniform, transition.direction);
            gl.Uniform1f(self.gap_uniform, DESKTOP_GAP_RATIO);
            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, 4);

            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, previous_tex1 as u32);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, previous_tex0 as u32);
            gl.ActiveTexture(previous_active_texture as u32);
            gl.ReadBuffer(previous_read_buffer as u32);
            gl.UseProgram(previous_program as u32);
            gl.Viewport(
                previous_viewport[0],
                previous_viewport[1],
                previous_viewport[2],
                previous_viewport[3],
            );
            if previous_blend != ffi::FALSE {
                gl.Enable(ffi::BLEND);
            }
            if previous_scissor != ffi::FALSE {
                gl.Enable(ffi::SCISSOR_TEST);
            }
        }

        true
    }
}

unsafe fn ensure_snapshot_textures(
    gl: &ffi::Gles2,
    entry: &mut OutputSnapshots,
    size: Size<i32, Physical>,
) {
    if entry.size == (size.w, size.h) {
        return;
    }
    for tex in [entry.outgoing_tex, entry.incoming_tex] {
        gl.BindTexture(ffi::TEXTURE_2D, tex);
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
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
    }
    entry.size = (size.w, size.h);
}

/// Curva `ease-out` cuártica para arranque instantáneo y frenado suave al cambiar de espacio.
pub fn eased_progress(elapsed: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / DURATION.as_secs_f32()).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eased_progress_is_monotonic_and_bounded() {
        assert_eq!(eased_progress(Duration::ZERO), 0.0);
        let mid = eased_progress(DURATION / 2);
        assert!(mid > 0.5 && mid < 1.0);
        assert_eq!(eased_progress(DURATION), 1.0);
        assert_eq!(eased_progress(DURATION + Duration::from_millis(50)), 1.0);
    }
}

