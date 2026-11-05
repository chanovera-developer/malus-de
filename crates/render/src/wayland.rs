// ==============================================================================
// Malus DE: Cliente Wayland Layer-Shell Nativo (Top-Bar wlr-layer-shell)
// ==============================================================================

use crate::layout::UiTree;
use crate::squircle::SquirclePipeline;
use crate::text::TextSystem;
use crate::top_bar::{build_top_bar, TopBarConfig, TopBarModel, TopBarNodes};
use anyhow::{Context, Result};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use std::ptr::NonNull;
use taffy::NodeId;
use tracing::info;
use wayland_client::{
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};

/// Estado completo de la aplicación de barra superior conectada a Wayland.
pub struct WaylandTopBarApp {
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,

    pub layer: LayerSurface,
    pub wgpu_surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_format: wgpu::TextureFormat,

    pub pipeline: SquirclePipeline,
    pub text_system: TextSystem,
    pub ui: UiTree,
    pub nodes: Option<TopBarNodes>,
    pub config: TopBarConfig,
    pub model: TopBarModel,

    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub exit: bool,
    pub needs_redraw: bool,
    pub hovered_node: Option<NodeId>,
}

impl WaylandTopBarApp {
    /// Inicializa la conexión con Wayland, crea la superficie layer-shell y configura el contexto WGPU.
    pub fn new(
        conn: &Connection,
        qh: &QueueHandle<Self>,
        globals: &wayland_client::globals::GlobalList,
        config: TopBarConfig,
        model: TopBarModel,
    ) -> Result<Self> {
        let compositor_state =
            CompositorState::bind(globals, qh).context("Protocolo wl_compositor no disponible")?;
        let layer_shell_state =
            LayerShell::bind(globals, qh).context("Protocolo wlr-layer-shell no disponible en el compositor")?;

        let wl_surface = compositor_state.create_surface(qh);
        let layer = layer_shell_state.create_layer_surface(
            qh,
            wl_surface,
            Layer::Top,
            Some("malus-top-bar"),
            None,
        );

        // Anclar a la parte superior de la pantalla y extender a todo el ancho
        layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        // Altura total de la capa (barra + margen superior)
        let total_layer_height = (config.height + config.margin_top + 4.0) as u32;
        layer.set_size(0, total_layer_height);
        // Zona de exclusión para que las ventanas maximizadas no tapen la barra
        layer.set_exclusive_zone(total_layer_height as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        // ----------------------------------------------------------------------
        // Inicializar contexto WGPU sobre la superficie Wayland
        // ----------------------------------------------------------------------
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(conn.backend().display_ptr() as *mut _).context("Puntero de display Wayland nulo")?,
        ));
        let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(layer.wl_surface().id().as_ptr() as *mut _).context("Puntero de superficie Wayland nulo")?,
        ));

        let wgpu_surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle,
                    raw_window_handle,
                })
                .context("Error al crear wgpu::Surface nativa de Wayland")?
        };

        let adapter = futures_lite::future::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&wgpu_surface),
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
        }))
        .context("No se encontró adaptador compatible con la superficie Wayland")?;

        let (device, queue) = futures_lite::future::block_on(adapter.request_device(&Default::default(), None))
            .context("Error al inicializar dispositivo WGPU")?;

        let caps = wgpu_surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let pipeline = SquirclePipeline::new(&device, surface_format);
        let text_system = TextSystem::new(&device, &queue, surface_format);

        let mut ui = UiTree::new();
        let nodes = build_top_bar(&mut ui, &config, &model).ok();

        Ok(Self {
            registry_state: RegistryState::new(globals),
            seat_state: SeatState::new(globals, qh),
            output_state: OutputState::new(globals, qh),
            layer,
            wgpu_surface,
            adapter,
            device,
            queue,
            surface_format,
            pipeline,
            text_system,
            ui,
            nodes,
            config,
            model,
            width: 1920,
            height: total_layer_height,
            scale_factor: 1.0,
            exit: false,
            needs_redraw: true,
            hovered_node: None,
        })
    }

    /// Re-renderiza el frame completo con WGPU y lo presenta a Wayland.
    pub fn render(&mut self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Ok(());
        }

        let frame = match self.wgpu_surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Outdated) | Err(wgpu::SurfaceError::Lost) => {
                self.configure_surface();
                self.wgpu_surface.get_current_texture()?
            }
            Err(e) => return Err(anyhow::anyhow!("Error al obtener textura de la superficie: {:?}", e)),
        };

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 1. Recalcular disposición en Flexbox con Taffy
        self.ui.compute_layout(self.width as f32, self.height as f32)?;
        let instances = self.ui.generate_instances();
        let labels = self.ui.generate_text_labels();

        // 2. Preparar búferes de WGPU y atlas de glifos
        let screen_size = [self.width as f32, self.height as f32];
        self.pipeline.prepare(&self.device, &self.queue, screen_size, self.scale_factor, &instances);
        self.text_system.prepare(&self.device, &self.queue, screen_size, self.scale_factor, &labels)?;

        // 3. RenderPass unificado con fondo transparente
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Wayland TopBar Command Encoder"),
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wayland TopBar RenderPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Dibujar fondos geométricos superelípticos
            self.pipeline.render(&mut rpass);

            // Dibujar tipografía encima
            self.text_system.render(&mut rpass)?;
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.text_system.trim();
        self.needs_redraw = false;

        Ok(())
    }

    fn configure_surface(&mut self) {
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.surface_format,
            view_formats: vec![self.surface_format],
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            width: self.width.max(1),
            height: self.height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode: wgpu::PresentMode::Mailbox,
        };
        self.wgpu_surface.configure(&self.device, &surface_config);
    }
}

// ==============================================================================
// Implementaciones de Manejadores de Protocolos SCTK
// ==============================================================================

impl LayerShellHandler for WaylandTopBarApp {
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (new_width, new_height) = configure.new_size;
        if new_width > 0 {
            self.width = new_width;
        }
        if new_height > 0 {
            self.height = new_height;
        }

        self.configure_surface();
        let _ = self.render();
    }

    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        info!("Superficie layer-shell cerrada por el compositor");
        self.exit = true;
    }
}

impl CompositorHandler for WaylandTopBarApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.scale_factor = new_factor as f32;
        self.needs_redraw = true;
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if self.needs_redraw {
            let _ = self.render();
        }
    }
}

impl OutputHandler for WaylandTopBarApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for WaylandTopBarApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for WaylandTopBarApp {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match event.kind {
                PointerEventKind::Motion { .. } => {
                    let (x, y) = event.position;
                    let hit = self.ui.hit_test([x as f32, y as f32]);
                    if hit != self.hovered_node {
                        // Restaurar estilo anterior
                        if let Some(prev) = self.hovered_node {
                            if let Some(data) = self.ui.node_data(prev) {
                                let mut style = data.style.clone();
                                if let Some(ref mut bg) = style.bg_color {
                                    bg[3] = (bg[3] - 0.15).max(0.1);
                                }
                                self.ui.set_visual_style(prev, style);
                            }
                        }
                        // Aplicar hover highlight
                        if let Some(curr) = hit {
                            if let Some(data) = self.ui.node_data(curr) {
                                let mut style = data.style.clone();
                                if let Some(ref mut bg) = style.bg_color {
                                    bg[3] = (bg[3] + 0.15).min(1.0);
                                }
                                self.ui.set_visual_style(curr, style);
                            }
                        }
                        self.hovered_node = hit;
                        self.needs_redraw = true;
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let (x, y) = event.position;
                    if let Some(hit) = self.ui.hit_test([x as f32, y as f32]) {
                        if let Some(data) = self.ui.node_data(hit) {
                            info!("Clic en botón Wayland (botón={}): {:?}", button, data.tag);
                        }
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.hovered_node = None;
                    self.needs_redraw = true;
                }
                _ => {}
            }
        }
    }
}

delegate_compositor!(WaylandTopBarApp);
delegate_output!(WaylandTopBarApp);
delegate_seat!(WaylandTopBarApp);
delegate_pointer!(WaylandTopBarApp);
delegate_layer!(WaylandTopBarApp);
delegate_registry!(WaylandTopBarApp);

impl ProvidesRegistryState for WaylandTopBarApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
