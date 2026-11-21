use crate::genie::GenieRect;
use malus_de_common::config::DockPosition;
use smithay::backend::renderer::gles::{ffi, GlesTexture};
use std::ffi::CString;

pub struct MagicLampAnimation {
    window: GenieRect,
    target: GenieRect,
    position: DockPosition,
    k: f32,
    j: f32,
}

#[inline]
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[inline]
fn smootherstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

impl MagicLampAnimation {
    pub fn compute_kj(raw_progress: f32, is_minimizing: bool) -> (f32, f32) {
        let raw_progress = raw_progress.clamp(0.0, 1.0);

        // En macOS Genie:
        // - Al minimizar (is_minimizing = true):
        //   Fase 1 (k: 0 -> 1, j = 0): el borde inferior se estrecha y desciende en curva S hasta el Dock (ease-out).
        //   Fase 2 (k = 1, j: 0 -> 1): la ventana se desliza y comprime a través del embudo hacia el Dock.
        // - Al restaurar (is_minimizing = false):
        //   Invierte el orden espacial manteniendo ease-out perceptual en el tiempo:
        //   Fase 1 (0.0 -> 0.66): k = 1, j: 1 -> 0 emerge desde el Dock asentándose suavemente en win_top.
        //   Fase 2 (0.66 -> 1.0): j = 0, k: 1 -> 0 retrae el cuello de inmediato y aterriza sin golpe en win_bottom.
        let split = 0.34;
        if is_minimizing {
            if raw_progress <= split {
                let p = (raw_progress / split).clamp(0.0, 1.0);
                let eased_k = 1.0 - (1.0 - p).powf(2.1);
                (eased_k, 0.0)
            } else {
                let p = ((raw_progress - split) / (1.0 - split)).clamp(0.0, 1.0);
                // Combinación de arranque progresivo y succión fluida hacia el final
                let eased_j = smoothstep(p) * 0.45 + p.powf(1.35) * 0.55;
                (1.0, eased_j.clamp(0.0, 1.0))
            }
        } else {
            let restore_split = 1.0 - split;
            if raw_progress <= restore_split {
                let q = (raw_progress / restore_split).clamp(0.0, 1.0);
                let inv_j = 1.0 - (smoothstep(q) * 0.45 + (1.0 - (1.0 - q).powf(1.45)) * 0.55);
                (1.0, inv_j.clamp(0.0, 1.0))
            } else {
                let q = ((raw_progress - restore_split) / split).clamp(0.0, 1.0);
                let eased_k = (1.0 - q).powf(2.1);
                (eased_k.clamp(0.0, 1.0), 0.0)
            }
        }
    }

    pub fn new(
        window: GenieRect,
        target: GenieRect,
        raw_progress: f32,
        is_minimizing: bool,
    ) -> Self {
        Self::new_oriented(
            window,
            target,
            raw_progress,
            is_minimizing,
            DockPosition::Bottom,
        )
    }

    pub fn new_oriented(
        window: GenieRect,
        target: GenieRect,
        raw_progress: f32,
        is_minimizing: bool,
        position: DockPosition,
    ) -> Self {
        let (k, j) = Self::compute_kj(raw_progress, is_minimizing);

        Self {
            window,
            target: GenieRect {
                x: target.x,
                y: target.y,
                width: target.width.max(24.0),
                height: target.height.max(24.0),
            },
            position,
            k,
            j,
        }
    }

    pub fn params(&self) -> (f32, f32, f32) {
        (self.k, self.j, self.alpha())
    }

    /// Calcula `(pos_eje_primario, inicio_eje_transversal, extension_eje_transversal)` para `slice` en `[0, 1]`.
    /// - En `Bottom`: `(screen_y, slice_x, slice_w)`.
    /// - En `Left` / `Right`: `(screen_x, slice_y, slice_h)`.
    pub fn deform_slice(&self, slice: f32) -> (f32, f32, f32) {
        let s = slice.clamp(0.0, 1.0);

        let (
            win_trail,
            win_lead,
            dock_trail,
            dock_lead,
            win_cross_center,
            win_cross_span,
            dock_cross_center,
            dock_cross_span,
            axis_sign,
        ) = match self.position {
            DockPosition::Bottom => (
                self.window.y,
                self.window.y + self.window.height,
                self.target.y,
                self.target.y + self.target.height,
                self.window.x + self.window.width * 0.5,
                self.window.width,
                self.target.x + self.target.width * 0.5,
                self.target.width,
                1.0_f32,
            ),
            DockPosition::Left => (
                self.window.x + self.window.width,
                self.window.x,
                self.target.x + self.target.width,
                self.target.x,
                self.window.y + self.window.height * 0.5,
                self.window.height,
                self.target.y + self.target.height * 0.5,
                self.target.height,
                -1.0_f32,
            ),
            DockPosition::Right => (
                self.window.x,
                self.window.x + self.window.width,
                self.target.x,
                self.target.x + self.target.width,
                self.window.y + self.window.height * 0.5,
                self.window.height,
                self.target.y + self.target.height * 0.5,
                self.target.height,
                1.0_f32,
            ),
        };

        // 1. Posición a lo largo del eje del embudo (`Bottom` en Y; `Left`/`Right` en X):
        // Durante Fase 1 (k: 0 -> 1, j = 0), el borde trasero sigue en win_trail y el delantero avanza hacia dock_lead.
        // Durante Fase 2 (k = 1, j: 0 -> 1), el borde trasero viaja hacia dock_trail mientras las rebanadas
        // se comprimen progresivamente hacia el cuello del embudo.
        let trail_pos = win_trail
            + (dock_trail - win_trail) * if self.j <= 0.0 { 0.0 } else { self.j.powf(1.12) };
        let reach = self.k.max(self.j);
        let lead_pos = win_lead + (dock_lead - win_lead) * reach;

        // Ligera aceleración de las rebanadas hacia el cuello durante la succión
        let suction_warp = s + self.k * (1.0 - self.j) * 0.22 * s * (1.0 - s);
        let screen_primary = trail_pos + suction_warp * (lead_pos - trail_pos);

        // 2. Coordenada espacial normalizada dentro del embudo [win_trail .. dock_lead]
        let total_span = (dock_lead - win_trail).abs().max(1.0);
        let spatial_u = (((screen_primary - win_trail) * axis_sign) / total_span).clamp(0.0, 1.0);

        // El hombro del embudo acompaña el avance de la ventana durante la primera parte de `j`,
        // manteniendo ancha la zona trasera antes de estrecharse hacia el Dock.
        let neck_start = (self.j * 0.52).clamp(0.0, 0.72);
        let funnel_t = if self.j > 0.0 {
            ((spatial_u - neck_start) / (1.0 - neck_start).max(0.05)).clamp(0.0, 1.0)
        } else {
            s
        };

        // Combinar la coordenada espacial del embudo con la coordenada propia de la rebanada
        // para garantizar convergencia exacta en los extremos (s = 0 y j = 1).
        let tau = (funnel_t * (1.0 - self.j) + self.j).clamp(0.0, 1.0);

        // 3. Curva de estrechamiento (Pinch) y curva en S de desplazamiento transversal (Sweep):
        let base_s = smoothstep(tau);
        let pinch_curve = (1.0 - (1.0 - base_s).powf(1.85)) * self.k;
        let sweep_curve = smootherstep(tau) * self.k;

        // Colapso final hacia el tamaño y centro exactos del slot a medida que j -> 1
        let width_factor = (pinch_curve + (1.0 - pinch_curve) * self.j).clamp(0.0, 1.0);
        let center_factor = (sweep_curve + (1.0 - sweep_curve) * self.j).clamp(0.0, 1.0);

        let cross_span =
            (win_cross_span + (dock_cross_span - win_cross_span) * width_factor).max(2.0);
        let current_cross_center =
            win_cross_center + (dock_cross_center - win_cross_center) * center_factor;

        (
            screen_primary,
            current_cross_center - cross_span * 0.5,
            cross_span,
        )
    }

    pub fn alpha(&self) -> f32 {
        if self.j > 0.92 {
            ((1.0 - self.j) / 0.08).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

/// Parámetros de cuadro unificados para renderizar la minimización/restauración (Genie o Escala).
#[derive(Clone, Copy)]
pub struct MinimizeRenderFrame {
    pub win_rect: GenieRect,
    pub target_rect: GenieRect,
    pub direction: DockPosition,
    pub k: f32,
    pub j: f32,
    pub alpha: f32,
    pub shadow_k: f32,
}

/// Calcula los parámetros del efecto de minimización según la preferencia activa (`use_genie`)
/// y la posición actual del Dock (`position`: Bottom, Left o Right).
/// En modo escala (`use_genie == false`) interpola el rectángulo de forma uniforme hasta el slot
/// del Dock sin deformación en cuello ni curva S.
pub fn compute_minimize_frame(
    win_rect: GenieRect,
    target_rect: GenieRect,
    raw_progress: f32,
    is_minimizing: bool,
    use_genie: bool,
    position: DockPosition,
) -> MinimizeRenderFrame {
    if use_genie {
        let (k, j, alpha) = MagicLampAnimation::new_oriented(
            win_rect,
            target_rect,
            raw_progress,
            is_minimizing,
            position,
        )
        .params();
        MinimizeRenderFrame {
            win_rect,
            target_rect,
            direction: position,
            k,
            j,
            alpha,
            shadow_k: k,
        }
    } else {
        let t = raw_progress.clamp(0.0, 1.0);
        let progress_curve = 0.55 * (1.0 - (1.0 - t).powf(2.2)) + 0.45 * smootherstep(t);
        let eased = if is_minimizing {
            progress_curve
        } else {
            1.0 - progress_curve
        };
        let target_w = target_rect.width.max(24.0);
        let target_h = target_rect.height.max(24.0);
        let scaled_rect = GenieRect {
            x: win_rect.x + (target_rect.x - win_rect.x) * eased,
            y: win_rect.y + (target_rect.y - win_rect.y) * eased,
            width: (win_rect.width + (target_w - win_rect.width) * eased).max(2.0),
            height: (win_rect.height + (target_h - win_rect.height) * eased).max(2.0),
        };
        // Mantener la ventana opaca durante todo el encogimiento y cruzar suavemente al slot
        // en el último 6% del recorrido (cuando ya coincide con la miniatura del Dock) sin hueco transparente.
        let alpha = if is_minimizing && t > 0.94 {
            1.0 - 0.25 * ((t - 0.94) / 0.06).clamp(0.0, 1.0)
        } else {
            1.0
        };
        MinimizeRenderFrame {
            win_rect: scaled_rect,
            target_rect: scaled_rect,
            direction: position,
            k: 0.0,
            j: 0.0,
            alpha,
            shadow_k: (eased * 0.45).clamp(0.0, 0.45),
        }
    }
}

const GENIE_VERT_SRC: &str = r#"#version 300 es
precision highp float;

uniform mat3 u_matrix;
uniform vec4 u_win_rect;     // (x, y, w, h) en píxeles físicos del monitor
uniform vec4 u_target_rect;  // (x, y, w, h) del slot del Dock en píxeles físicos
uniform float u_k;
uniform float u_j;
uniform int u_n_rows;
uniform int u_direction;     // 0 = Bottom, 1 = Left, 2 = Right

out vec2 v_uv;

float safe_pow(float x, float e) {
    return x <= 0.0 ? 0.0 : pow(x, e);
}

float smooth_01(float x) {
    float c = clamp(x, 0.0, 1.0);
    return c * c * (3.0 - 2.0 * c);
}

float smoother_01(float x) {
    float c = clamp(x, 0.0, 1.0);
    return c * c * c * (c * (c * 6.0 - 15.0) + 10.0);
}

vec3 eval_genie_slice(float slice_s) {
    float s = clamp(slice_s, 0.0, 1.0);

    float win_trail;
    float win_lead;
    float dock_trail;
    float dock_lead;
    float win_cross_center;
    float win_cross_span;
    float dock_cross_center;
    float dock_cross_span;
    float axis_sign;

    if (u_direction == 1) {
        // Left: desde borde derecho (s=0) estirándose hacia la izquierda (s=1)
        win_trail = u_win_rect.x + u_win_rect.z;
        win_lead = u_win_rect.x;
        dock_trail = u_target_rect.x + u_target_rect.z;
        dock_lead = u_target_rect.x;
        win_cross_center = u_win_rect.y + u_win_rect.w * 0.5;
        win_cross_span = u_win_rect.w;
        dock_cross_center = u_target_rect.y + u_target_rect.w * 0.5;
        dock_cross_span = u_target_rect.w;
        axis_sign = -1.0;
    } else if (u_direction == 2) {
        // Right: desde borde izquierdo (s=0) estirándose hacia la derecha (s=1)
        win_trail = u_win_rect.x;
        win_lead = u_win_rect.x + u_win_rect.z;
        dock_trail = u_target_rect.x;
        dock_lead = u_target_rect.x + u_target_rect.z;
        win_cross_center = u_win_rect.y + u_win_rect.w * 0.5;
        win_cross_span = u_win_rect.w;
        dock_cross_center = u_target_rect.y + u_target_rect.w * 0.5;
        dock_cross_span = u_target_rect.w;
        axis_sign = 1.0;
    } else {
        // Bottom: desde borde superior (s=0) estirándose hacia abajo (s=1)
        win_trail = u_win_rect.y;
        win_lead = u_win_rect.y + u_win_rect.w;
        dock_trail = u_target_rect.y;
        dock_lead = u_target_rect.y + u_target_rect.w;
        win_cross_center = u_win_rect.x + u_win_rect.z * 0.5;
        win_cross_span = u_win_rect.z;
        dock_cross_center = u_target_rect.x + u_target_rect.z * 0.5;
        dock_cross_span = u_target_rect.z;
        axis_sign = 1.0;
    }

    float trail_pos = win_trail + (dock_trail - win_trail) * safe_pow(u_j, 1.12);
    float reach = max(u_k, u_j);
    float lead_pos = win_lead + (dock_lead - win_lead) * reach;

    float suction_warp = s + u_k * (1.0 - u_j) * 0.22 * s * (1.0 - s);
    float screen_primary = trail_pos + suction_warp * (lead_pos - trail_pos);

    float total_span = max(abs(dock_lead - win_trail), 1.0);
    float spatial_u = clamp(((screen_primary - win_trail) * axis_sign) / total_span, 0.0, 1.0);

    float neck_start = clamp(u_j * 0.52, 0.0, 0.72);
    float funnel_t = (u_j > 0.0)
        ? clamp((spatial_u - neck_start) / max(1.0 - neck_start, 0.05), 0.0, 1.0)
        : s;

    float tau = clamp(funnel_t * (1.0 - u_j) + u_j, 0.0, 1.0);

    float base_s = smooth_01(tau);
    float pinch_curve = (1.0 - safe_pow(1.0 - base_s, 1.85)) * u_k;
    float sweep_curve = smoother_01(tau) * u_k;

    float width_factor = clamp(pinch_curve + (1.0 - pinch_curve) * u_j, 0.0, 1.0);
    float center_factor = clamp(sweep_curve + (1.0 - sweep_curve) * u_j, 0.0, 1.0);

    float cross_span = max(win_cross_span + (dock_cross_span - win_cross_span) * width_factor, 2.0);
    float current_cross_center = win_cross_center + (dock_cross_center - win_cross_center) * center_factor;

    return vec3(screen_primary, current_cross_center - cross_span * 0.5, cross_span);
}

void main() {
    int row = gl_VertexID / 6;
    int tri_v = gl_VertexID % 6;
    float vx = (tri_v == 1 || tri_v == 2 || tri_v == 4) ? 1.0 : 0.0;
    float vy = (tri_v == 2 || tri_v == 4 || tri_v == 5) ? 1.0 : 0.0;

    float s = (float(row) + vy) / float(u_n_rows);
    vec3 sl = eval_genie_slice(s);
    vec2 pos;
    if (u_direction == 1) {
        pos = vec2(sl.x, sl.y + vx * sl.z);
        v_uv = vec2(1.0 - s, vx);
    } else if (u_direction == 2) {
        pos = vec2(sl.x, sl.y + vx * sl.z);
        v_uv = vec2(s, vx);
    } else {
        pos = vec2(sl.y + vx * sl.z, sl.x);
        v_uv = vec2(vx, s);
    }

    vec3 ndc = u_matrix * vec3(pos, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
}
"#;

const GENIE_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
out vec4 frag_color;

uniform sampler2D u_tex;
uniform float u_alpha;

void main() {
    vec4 tex = texture(u_tex, v_uv);
    if (tex.a <= 0.002) {
        discard;
    }
    frag_color = tex * u_alpha;
}
"#;

/// Pipeline GPU OpenGL ES 3.0 para renderizar la deformación Genie sobre un snapshot único.
pub struct GeniePipeline {
    program: ffi::types::GLuint,
    u_matrix: ffi::types::GLint,
    u_win_rect: ffi::types::GLint,
    u_target_rect: ffi::types::GLint,
    u_k: ffi::types::GLint,
    u_j: ffi::types::GLint,
    u_n_rows: ffi::types::GLint,
    u_direction: ffi::types::GLint,
    u_tex: ffi::types::GLint,
    u_alpha: ffi::types::GLint,
}

impl GeniePipeline {
    pub fn new(gl: &ffi::Gles2) -> Option<Self> {
        unsafe {
            let program =
                crate::black_hole::link_program(gl, GENIE_VERT_SRC, GENIE_FRAG_SRC, "Genie")?;
            let get_loc = |name: &str| -> ffi::types::GLint {
                let cname = CString::new(name).unwrap();
                gl.GetUniformLocation(program, cname.as_ptr())
            };

            Some(Self {
                u_matrix: get_loc("u_matrix"),
                u_win_rect: get_loc("u_win_rect"),
                u_target_rect: get_loc("u_target_rect"),
                u_k: get_loc("u_k"),
                u_j: get_loc("u_j"),
                u_n_rows: get_loc("u_n_rows"),
                u_direction: get_loc("u_direction"),
                u_tex: get_loc("u_tex"),
                u_alpha: get_loc("u_alpha"),
                program,
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        gl: &ffi::Gles2,
        projection: &[f32; 9],
        texture: &GlesTexture,
        win_rect: GenieRect,
        target_rect: GenieRect,
        direction: DockPosition,
        k: f32,
        j: f32,
        alpha: f32,
    ) {
        if alpha <= 0.002 || win_rect.width <= 1.0 || win_rect.height <= 1.0 {
            return;
        }

        let dir_id = match direction {
            DockPosition::Bottom => 0,
            DockPosition::Left => 1,
            DockPosition::Right => 2,
        };
        let n_rows = if direction == DockPosition::Bottom {
            ((win_rect.height / 2.0).round() as i32).clamp(120, 280)
        } else {
            ((win_rect.width / 2.0).round() as i32).clamp(120, 280)
        };
        let tw = target_rect.width.max(24.0);
        let th = target_rect.height.max(24.0);

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

            gl.UseProgram(self.program);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, texture.tex_id());
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

            gl.UniformMatrix3fv(self.u_matrix, 1, ffi::FALSE, projection.as_ptr());
            gl.Uniform4f(
                self.u_win_rect,
                win_rect.x,
                win_rect.y,
                win_rect.width,
                win_rect.height,
            );
            gl.Uniform4f(self.u_target_rect, target_rect.x, target_rect.y, tw, th);
            gl.Uniform1f(self.u_k, k);
            gl.Uniform1f(self.u_j, j);
            gl.Uniform1i(self.u_n_rows, n_rows);
            gl.Uniform1i(self.u_direction, dir_id);
            gl.Uniform1i(self.u_tex, 0);
            gl.Uniform1f(self.u_alpha, alpha);

            gl.DrawArrays(ffi::TRIANGLES, 0, n_rows * 6);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
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

#[cfg(test)]
mod tests {
    use super::MagicLampAnimation;
    use crate::genie::GenieRect;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> GenieRect {
        GenieRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn minimizing_starts_at_window_geometry() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let animation = MagicLampAnimation::new(window, target, 0.0, true);

        for slice in [0.0, 0.5, 1.0] {
            let (y, x, width) = animation.deform_slice(slice);
            assert!((y - (window.y + slice * window.height)).abs() < 0.001);
            assert!((x - window.x).abs() < 0.001);
            assert!((width - window.width).abs() < 0.001);
        }
        assert_eq!(animation.alpha(), 1.0);
    }

    #[test]
    fn minimizing_ends_at_dock_geometry_and_fades_out() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let animation = MagicLampAnimation::new(window, target, 1.0, true);

        for slice in [0.0, 0.5, 1.0] {
            let (y, x, width) = animation.deform_slice(slice);
            assert!((y - (target.y + slice * target.height)).abs() < 0.001);
            assert!((x - target.x).abs() < 0.001);
            assert!((width - target.width).abs() < 0.001);
        }
        assert_eq!(animation.alpha(), 0.0);
    }

    #[test]
    fn minimizing_tapers_the_window_toward_the_dock() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let animation = MagicLampAnimation::new(window, target, 0.2, true);

        let (top_y, _, top_width) = animation.deform_slice(0.0);
        let (bottom_y, _, bottom_width) = animation.deform_slice(1.0);

        assert!(bottom_width < top_width);
        assert!(bottom_y > window.y + window.height);
        assert!(top_y >= window.y);
    }

    #[test]
    fn restoring_starts_at_dock_and_ends_at_window_geometry() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let at_dock = MagicLampAnimation::new(window, target, 0.0, false);
        let at_window = MagicLampAnimation::new(window, target, 1.0, false);

        for slice in [0.0, 0.5, 1.0] {
            let (dock_y, dock_x, dock_width) = at_dock.deform_slice(slice);
            assert!((dock_y - (target.y + slice * target.height)).abs() < 0.001);
            assert!((dock_x - target.x).abs() < 0.001);
            assert!((dock_width - target.width).abs() < 0.001);

            let (window_y, window_x, window_width) = at_window.deform_slice(slice);
            assert!((window_y - (window.y + slice * window.height)).abs() < 0.001);
            assert!((window_x - window.x).abs() < 0.001);
            assert!((window_width - window.width).abs() < 0.001);
        }
        assert_eq!(at_dock.alpha(), 0.0);
        assert_eq!(at_window.alpha(), 1.0);
    }

    #[test]
    fn gpu_slice_formula_matches_cpu_deform_slice() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        for is_minimizing in [true, false] {
            for t in [0.0_f32, 0.2, 0.34, 0.5, 0.8, 1.0] {
                let anim = MagicLampAnimation::new(window, target, t, is_minimizing);
                let (k, j, _) = anim.params();
                for s in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
                    let (cpu_y, cpu_x, cpu_w) = anim.deform_slice(s);
                    let top_y = window.y
                        + (target.y - window.y) * if j <= 0.0 { 0.0 } else { j.powf(1.12) };
                    let bottom_y = (window.y + window.height)
                        + ((target.y + target.height) - (window.y + window.height)) * k.max(j);
                    let screen_y =
                        top_y + (s + k * (1.0 - j) * 0.22 * s * (1.0 - s)) * (bottom_y - top_y);
                    let spatial_u = ((screen_y - window.y)
                        / ((target.y + target.height) - window.y).abs().max(1.0))
                    .clamp(0.0, 1.0);
                    let neck_start = (j * 0.52).clamp(0.0, 0.72);
                    let funnel_t = if j > 0.0 {
                        ((spatial_u - neck_start) / (1.0 - neck_start).max(0.05)).clamp(0.0, 1.0)
                    } else {
                        s
                    };
                    let tau: f32 = (funnel_t * (1.0 - j) + j).clamp(0.0, 1.0);
                    let base_s: f32 = super::smoothstep(tau);
                    let pinch = (1.0_f32
                        - if 1.0_f32 - base_s <= 0.0 {
                            0.0
                        } else {
                            (1.0_f32 - base_s).powf(1.85)
                        })
                        * k;
                    let sweep = super::smootherstep(tau) * k;
                    let slice_w = (window.width
                        + (target.width - window.width)
                            * (pinch + (1.0 - pinch) * j).clamp(0.0, 1.0))
                    .max(2.0);
                    let center_x = (window.x + window.width * 0.5)
                        + ((target.x + target.width * 0.5) - (window.x + window.width * 0.5))
                            * (sweep + (1.0 - sweep) * j).clamp(0.0, 1.0);
                    assert!((cpu_y - screen_y).abs() < 0.001);
                    assert!((cpu_x - (center_x - slice_w * 0.5)).abs() < 0.001);
                    assert!((cpu_w - slice_w).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn restoring_retracts_immediately_after_split_and_lands_smoothly() {
        let mut prev_j = 1.0_f32;
        let mut prev_k = 1.0_f32;

        // Fase 1 (0.0 -> 0.66): k permanece en 1.0 mientras j emerge de forma monótona hasta 0.0.
        for step in 0..=16 {
            let t = (step as f32 / 16.0) * 0.66;
            let (k, j) = MagicLampAnimation::compute_kj(t, false);
            assert!((k - 1.0).abs() < 0.001);
            assert!(j <= prev_j + 0.0001);
            prev_j = j;
        }
        assert!(prev_j < 0.001);

        // Fase 2 (0.66 -> 1.0): j = 0 y k se retrae enseguida (ease-out) sin estancarse en el codo.
        let (k_early, j_early) = MagicLampAnimation::compute_kj(0.76, false);
        assert_eq!(j_early, 0.0);
        assert!(k_early < 0.6, "esperaba k < 0.6 en t=0.76, fue {k_early}");

        for step in 0..=10 {
            let t = 0.66 + (step as f32 / 10.0) * 0.34;
            let (k, j) = MagicLampAnimation::compute_kj(t, false);
            assert_eq!(j, 0.0);
            assert!(k <= prev_k + 0.0001);
            prev_k = k;
        }
        assert!(prev_k < 0.001);
    }

    #[test]
    fn scale_minimize_effect_interpolates_uniformly_without_genie_warp() {
        use malus_de_common::config::DockPosition;
        let window = rect(100.0, 80.0, 800.0, 600.0);
        let target = rect(500.0, 960.0, 64.0, 48.0);

        let start =
            super::compute_minimize_frame(window, target, 0.0, true, false, DockPosition::Bottom);
        assert_eq!(start.k, 0.0);
        assert_eq!(start.j, 0.0);
        assert!((start.win_rect.x - window.x).abs() < 0.001);
        assert!((start.win_rect.width - window.width).abs() < 0.001);
        assert!((start.alpha - 1.0).abs() < 0.001);

        let mid =
            super::compute_minimize_frame(window, target, 0.5, true, false, DockPosition::Bottom);
        assert_eq!(mid.k, 0.0);
        assert_eq!(mid.j, 0.0);
        assert!((mid.alpha - 1.0).abs() < 0.001);
        assert!(mid.win_rect.width < window.width && mid.win_rect.width > target.width);
        assert!(mid.win_rect.y > window.y && mid.win_rect.y < target.y);

        let near_end =
            super::compute_minimize_frame(window, target, 0.9, true, false, DockPosition::Bottom);
        assert!((near_end.alpha - 1.0).abs() < 0.001);

        let end =
            super::compute_minimize_frame(window, target, 1.0, true, false, DockPosition::Bottom);
        assert!(end.alpha >= 0.74);
        assert!((end.win_rect.x - target.x).abs() < 0.001);
        assert!((end.win_rect.y - target.y).abs() < 0.001);
        assert!((end.win_rect.width - target.width).abs() < 0.001);
        assert!((end.win_rect.height - target.height).abs() < 0.001);

        let restore_start =
            super::compute_minimize_frame(window, target, 0.0, false, false, DockPosition::Bottom);
        assert!((restore_start.alpha - 1.0).abs() < 0.001);
        assert!((restore_start.win_rect.width - target.width).abs() < 0.001);
        let restore_end =
            super::compute_minimize_frame(window, target, 1.0, false, false, DockPosition::Bottom);
        assert!((restore_end.win_rect.width - window.width).abs() < 0.001);
    }

    #[test]
    fn side_genie_stretches_horizontally_to_left_and_right_dock_slots() {
        use malus_de_common::config::DockPosition;
        let window = rect(400.0, 140.0, 800.0, 600.0);
        let left_target = rect(16.0, 520.0, 54.0, 44.0);
        let right_target = rect(1840.0, 520.0, 54.0, 44.0);

        for (pos, target) in [
            (DockPosition::Left, left_target),
            (DockPosition::Right, right_target),
        ] {
            let start = MagicLampAnimation::new_oriented(window, target, 0.0, true, pos);
            for s in [0.0, 0.5, 1.0] {
                let (x, y, h) = start.deform_slice(s);
                let expected_x = if pos == DockPosition::Left {
                    window.x + window.width * (1.0 - s)
                } else {
                    window.x + window.width * s
                };
                assert!((x - expected_x).abs() < 0.001);
                assert!((y - window.y).abs() < 0.001);
                assert!((h - window.height).abs() < 0.001);
            }

            let mid = MagicLampAnimation::new_oriented(window, target, 0.2, true, pos);
            let (_, _, trail_h) = mid.deform_slice(0.0);
            let (lead_x, _, lead_h) = mid.deform_slice(1.0);
            assert!(lead_h < trail_h, "el extremo hacia el dock debe estrecharse");
            if pos == DockPosition::Left {
                assert!(lead_x < window.x, "en Dock Left debe estirarse a la izquierda");
            } else {
                assert!(
                    lead_x > window.x + window.width,
                    "en Dock Right debe estirarse a la derecha"
                );
            }

            let end = MagicLampAnimation::new_oriented(window, target, 1.0, true, pos);
            for s in [0.0, 0.5, 1.0] {
                let (x, y, h) = end.deform_slice(s);
                let expected_x = if pos == DockPosition::Left {
                    target.x + target.width * (1.0 - s)
                } else {
                    target.x + target.width * s
                };
                assert!((x - expected_x).abs() < 0.001);
                assert!((y - target.y).abs() < 0.001);
                assert!((h - target.height).abs() < 0.001);
            }
            assert_eq!(end.alpha(), 0.0);
        }
    }
}
