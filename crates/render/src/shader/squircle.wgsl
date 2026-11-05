// ==============================================================================
// Malus DE: Superellipse (Squircle SDF) Shader para WGPU
// ==============================================================================
// Implementa curvas continuas G2 con exponente de Lamé (n ≈ 4.2)
// y antialiasing analítico de subpíxel por gradiente de distancia.

struct Uniforms {
    screen_size: vec2<f32>,
    scale_factor: f32,
    _pad: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct InstanceInput {
    @location(0) position: vec2<f32>,      // Coordenadas superiores izquierdas (px)
    @location(1) size: vec2<f32>,          // Ancho y alto (px)
    @location(2) bg_color: vec4<f32>,      // Color de fondo RGBA
    @location(3) border_color: vec4<f32>,  // Color de borde reflectante RGBA
    @location(4) corner_radius: f32,       // Radio de curvatura (px)
    @location(5) border_width: f32,        // Ancho del borde (px)
    @location(6) squircle_power: f32,      // Exponente n (ej: 4.2 para squircle macOS, 2.0 para círculo)
    @location(7) z_index: f32,             // Profundidad z
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) bg_color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) corner_radius: f32,
    @location(5) border_width: f32,
    @location(6) squircle_power: f32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: InstanceInput
) -> VertexOutput {
    // Generación procedural de un quad (2 triángulos, 6 vértices)
    var unit_pos = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), // Sup-Izq
        vec2<f32>(1.0, 0.0), // Sup-Der
        vec2<f32>(1.0, 1.0), // Inf-Der
        vec2<f32>(0.0, 0.0), // Sup-Izq
        vec2<f32>(1.0, 1.0), // Inf-Der
        vec2<f32>(0.0, 1.0)  // Inf-Izq
    );

    let uv = unit_pos[vertex_index];
    let world_px = instance.position + uv * instance.size;
    let half_sz = instance.size * 0.5;
    let local = (uv - vec2<f32>(0.5, 0.5)) * instance.size;

    // Conversión de coordenadas de píxeles de pantalla a espacio de recorte (NDC)
    let ndc_x = (world_px.x / uniforms.screen_size.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (world_px.y / uniforms.screen_size.y) * 2.0;
    let ndc_z = clamp(instance.z_index * 0.005, 0.0, 0.95);

    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc_x, ndc_y, ndc_z, 1.0);
    out.local_pos = local;
    out.half_size = half_sz;
    out.bg_color = instance.bg_color;
    out.border_color = instance.border_color;
    out.corner_radius = min(instance.corner_radius, min(half_sz.x, half_sz.y));
    out.border_width = instance.border_width;
    out.squircle_power = max(instance.squircle_power, 1.0);
    return out;
}

// Distancia con signo analítica a un squircle superelíptico
fn squircle_sdf(p: vec2<f32>, b: vec2<f32>, r: f32, n: f32) -> f32 {
    let d = abs(p) - b + vec2<f32>(r, r);
    if (d.x <= 0.0 || d.y <= 0.0) {
        return max(d.x, d.y) - r;
    }
    // Aproximación de Lamé superelíptica: (x^n + y^n)^(1/n) - r
    return pow(pow(d.x, n) + pow(d.y, n), 1.0 / n) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = squircle_sdf(in.local_pos, in.half_size, in.corner_radius, in.squircle_power);

    if (in.border_width < -0.1) {
        // Modo Sombra Difusa Analítica (Drop Shadow suave con desvanecimiento gaussiano continuo)
        let blur_radius = -in.border_width;
        let shadow_factor = smoothstep(0.0, -blur_radius, dist);
        let final_alpha = in.bg_color.a * shadow_factor;
        if (final_alpha <= 0.0005) {
            discard;
        }
        return vec4<f32>(in.bg_color.rgb * final_alpha, final_alpha);
    }

    // Suavizado analítico de subpíxel mediante la tasa de cambio local de la distancia
    let edge_aa = max(fwidth(dist), 0.0001);
    let alpha = 1.0 - smoothstep(-edge_aa, 0.0, dist);

    if (alpha <= 0.0001) {
        discard;
    }

    var col = in.bg_color;
    if (in.border_width >= 1.5) {
        // Doble línea perimetral exacta estilo macOS (menu-bar.jpg):
        // 1. Línea exterior oscura semitransparente (dist de -1.0 a 0.0)
        // 2. Línea interior clara con mucha menor transparencia (dist de -2.0 a -1.0)
        // 3. Fondo interior del menú (dist < -2.0)
        let outer_mask = smoothstep(-1.0 - edge_aa, -1.0, dist);
        let inner_mask = smoothstep(-in.border_width - edge_aa, -in.border_width, dist) * (1.0 - outer_mask);
        let specular_clear = vec4<f32>(1.0, 1.0, 1.0, 0.75);

        col = mix(col, specular_clear, inner_mask);
        col = mix(col, in.border_color, outer_mask);
    } else if (in.border_width > 0.0) {
        let border_mask = smoothstep(-in.border_width - edge_aa, -in.border_width, dist);
        col = mix(in.bg_color, in.border_color, border_mask);
    }

    // Efecto sutil de Liquid Glass: brillo especular en el borde perimetral superior
    if (in.bg_color.a > 0.01) {
        let rim_specular = smoothstep(in.half_size.y * 0.6, in.half_size.y, -in.local_pos.y) * 0.06 * in.bg_color.a;
        col = vec4<f32>(min(col.rgb + vec3<f32>(rim_specular), vec3<f32>(1.0)), col.a);
    }

    // Retornar color pre-multiplicado por alpha para evitar halos oscuros en transparencia
    let final_alpha = col.a * alpha;
    return vec4<f32>(col.rgb * final_alpha, final_alpha);
}
