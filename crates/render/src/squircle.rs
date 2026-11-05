// ==============================================================================
// Malus DE: Pipeline Instanciado de Curvas Superelípticas (Squircles)
// ==============================================================================

use bytemuck::{Pod, Zeroable};

/// Exponente por defecto para reproducir con fidelidad la curvatura continua G2 de macOS.
pub const MACOS_SQUIRCLE_POWER: f32 = 4.2;

/// Uniforms globales pasados al shader de renderizado.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SquircleUniforms {
    pub screen_size: [f32; 2],
    pub scale_factor: f32,
    pub _pad: f32,
}

/// Parámetros por instancia para cada elemento visual con forma de squircle.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SquircleInstance {
    pub position: [f32; 2],      // Posición superior izquierda (x, y) en píxeles
    pub size: [f32; 2],          // Ancho y alto (width, height) en píxeles
    pub bg_color: [f32; 4],      // Color de fondo [R, G, B, A] en rango 0.0..1.0
    pub border_color: [f32; 4],  // Color del contorno/borde reflectante [R, G, B, A]
    pub corner_radius: f32,       // Radio de curvatura en píxeles
    pub border_width: f32,        // Grosor del contorno en píxeles
    pub squircle_power: f32,      // Exponente de Lamé (4.2 para macOS, 2.0 para círculo)
    pub z_index: f32,             // Profundidad z
}

impl SquircleInstance {
    /// Crea un squircle con valores por defecto de macOS (potencia 4.2 y sin borde).
    pub fn new(x: f32, y: f32, width: f32, height: f32, corner_radius: f32, bg_color: [f32; 4]) -> Self {
        Self {
            position: [x, y],
            size: [width, height],
            bg_color,
            border_color: [0.0, 0.0, 0.0, 0.0],
            corner_radius,
            border_width: 0.0,
            squircle_power: MACOS_SQUIRCLE_POWER,
            z_index: 0.0,
        }
    }

    /// Añade un borde perimetral reflectante al squircle.
    pub fn with_border(mut self, width: f32, color: [f32; 4]) -> Self {
        self.border_width = width;
        self.border_color = color;
        self
    }

    /// Permite personalizar la potencia superelíptica (ej. 2.0 para radio circular estándar).
    pub fn with_power(mut self, power: f32) -> Self {
        self.squircle_power = power;
        self
    }

    /// Ajusta la profundidad z en el orden de renderizado.
    pub fn with_z(mut self, z: f32) -> Self {
        self.z_index = z;
        self
    }
}

/// Pipeline de renderizado GPU optimizado para dibujar múltiples squircles en un solo draw call.
pub struct SquirclePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: Option<wgpu::Buffer>,
    instance_capacity: usize,
    instance_count: usize,
}

impl SquirclePipeline {
    /// Inicializa los shaders, el layout de buffers y el pipeline de renderizado en WGPU.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Malus Squircle Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader/squircle.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Squircle Uniform Buffer"),
            size: std::mem::size_of::<SquircleUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Squircle Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Squircle Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Squircle Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Layout de atributos por instancia (64 bytes de stride)
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SquircleInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2, // position
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2, // size
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4, // bg_color
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4, // border_color
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32,   // corner_radius
                },
                wgpu::VertexAttribute {
                    offset: 52,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32,   // border_width
                },
                wgpu::VertexAttribute {
                    offset: 56,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32,   // squircle_power
                },
                wgpu::VertexAttribute {
                    offset: 60,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32,   // z_index
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Squircle Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer: None,
            instance_capacity: 0,
            instance_count: 0,
        }
    }

    /// Actualiza los uniforms globales y el buffer de instancias para el siguiente frame.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_size: [f32; 2],
        scale_factor: f32,
        instances: &[SquircleInstance],
    ) {
        let uniforms = SquircleUniforms {
            screen_size,
            scale_factor,
            _pad: 0.0,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        self.instance_count = instances.len();
        if self.instance_count == 0 {
            return;
        }

        // Reubicar o actualizar el buffer de instancias dinámico
        if self.instance_capacity < self.instance_count || self.instance_buffer.is_none() {
            let new_capacity = (self.instance_count * 2).max(16);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Squircle Instance Buffer"),
                size: (new_capacity * std::mem::size_of::<SquircleInstance>()) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_buffer = Some(buffer);
            self.instance_capacity = new_capacity;
        }

        if let Some(ref buffer) = self.instance_buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(instances));
        }
    }

    /// Ejecuta el dibujo por lotes instanciado dentro del render pass activo.
    pub fn render<'rp>(&'rp self, rpass: &mut wgpu::RenderPass<'rp>) {
        if self.instance_count == 0 {
            return;
        }

        if let Some(ref buffer) = self.instance_buffer {
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_vertex_buffer(0, buffer.slice(..));
            // 6 vértices procedurales generan 1 quad (2 triángulos) por instancia
            rpass.draw(0..6, 0..self.instance_count as u32);
        }
    }
}
