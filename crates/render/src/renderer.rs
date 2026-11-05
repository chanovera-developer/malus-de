// ==============================================================================
// Malus DE: Contexto de Renderizado GPU y Render Offscreen
// ==============================================================================

use crate::squircle::{SquircleInstance, SquirclePipeline};
use crate::text::{TextLabel, TextSystem};
use anyhow::{Context, Result};
use futures_lite::future::block_on;

/// Contexto principal que gestiona el dispositivo WGPU y el pipeline de squircles.
pub struct SquircleRenderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub pipeline: SquirclePipeline,
    pub target_format: wgpu::TextureFormat,
}

impl SquircleRenderer {
    /// Inicializa un contexto WGPU headless (compatible con cualquier GPU o renderizado por software Mesa).
    pub fn new_headless(target_format: wgpu::TextureFormat) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .context("No se encontró un adaptador gráfico compatible en el sistema")?;

        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Malus DE Render Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .context("No se pudo inicializar el dispositivo WGPU")?;

        let pipeline = SquirclePipeline::new(&device, target_format);

        Ok(Self {
            device,
            queue,
            pipeline,
            target_format,
        })
    }

    /// Crea un renderizador a partir de un dispositivo y cola existentes (ej. compartidos con el compositor).
    pub fn from_device_and_queue(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let pipeline = SquirclePipeline::new(&device, target_format);
        Self {
            device,
            queue,
            pipeline,
            target_format,
        }
    }

    /// Renderiza un lote de squircles directamente sobre una vista de textura dada (ej. superficie Wayland).
    pub fn render_to_view(
        &mut self,
        target_view: &wgpu::TextureView,
        screen_size: [f32; 2],
        scale_factor: f32,
        instances: &[SquircleInstance],
        clear_color: Option<wgpu::Color>,
    ) {
        self.pipeline.prepare(&self.device, &self.queue, screen_size, scale_factor, instances);

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Squircle Render Command Encoder"),
        });

        {
            let load_op = match clear_color {
                Some(c) => wgpu::LoadOp::Clear(c),
                None => wgpu::LoadOp::Load,
            };

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Squircle Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: load_op,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.pipeline.render(&mut rpass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Renderiza tanto el fondo de squircles como las etiquetas tipográficas en un único RenderPass.
    pub fn render_scene_to_view(
        &mut self,
        text_system: &mut TextSystem,
        target_view: &wgpu::TextureView,
        screen_size: [f32; 2],
        scale_factor: f32,
        instances: &[SquircleInstance],
        labels: &[(TextLabel, [f32; 2], [f32; 2])],
        clear_color: Option<wgpu::Color>,
    ) -> Result<()> {
        self.pipeline.prepare(&self.device, &self.queue, screen_size, scale_factor, instances);
        text_system.prepare(&self.device, &self.queue, screen_size, scale_factor, labels)?;

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Scene Render Command Encoder"),
        });

        {
            let load_op = match clear_color {
                Some(c) => wgpu::LoadOp::Clear(c),
                None => wgpu::LoadOp::Load,
            };

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: load_op,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // 1. Dibujar geometrías y fondos superelípticos
            self.pipeline.render(&mut rpass);

            // 2. Dibujar tipografía por encima
            text_system.render(&mut rpass)?;
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        text_system.trim();

        Ok(())
    }

    /// Renderiza una escena de prueba a un búfer RGBA de memoria (headless), ideal para pruebas visuales y CI.
    pub fn render_to_rgba_image(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f32,
        instances: &[SquircleInstance],
    ) -> Result<Vec<u8>> {
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Squircle Render Target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let texture = self.device.create_texture(&texture_desc);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.render_to_view(
            &view,
            [width as f32, height as f32],
            scale_factor,
            instances,
            Some(wgpu::Color::TRANSPARENT),
        );

        self.download_texture_bytes(&texture, width, height)
    }

    /// Renderiza la escena completa (squircles + texto) a un búfer RGBA de memoria.
    pub fn render_scene_to_rgba_image(
        &mut self,
        text_system: &mut TextSystem,
        width: u32,
        height: u32,
        scale_factor: f32,
        instances: &[SquircleInstance],
        labels: &[(TextLabel, [f32; 2], [f32; 2])],
    ) -> Result<Vec<u8>> {
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Scene Render Target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let texture = self.device.create_texture(&texture_desc);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.render_scene_to_view(
            text_system,
            &view,
            [width as f32, height as f32],
            scale_factor,
            instances,
            labels,
            Some(wgpu::Color::TRANSPARENT),
        )?;

        self.download_texture_bytes(&texture, width, height)
    }

    fn download_texture_bytes(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        let bytes_per_pixel = 4u32;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) / align * align;

        let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Offscreen Download Staging Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Texture Copy Encoder"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = async_channel::bounded(1);
        buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send_blocking(res);
        });

        self.device.poll(wgpu::Maintain::Wait);
        block_on(rx.recv())?.context("Error al mapear buffer de textura en GPU")?;

        let padded_data = buffer_slice.get_mapped_range();
        let mut unpadded_data = Vec::with_capacity((width * height * bytes_per_pixel) as usize);

        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            unpadded_data.extend_from_slice(&padded_data[start..end]);
        }

        drop(padded_data);
        output_buffer.unmap();

        Ok(unpadded_data)
    }
}
