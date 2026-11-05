// ==============================================================================
// Malus DE: Cliente Wayland Layer-Shell Nativo (Top-Bar wlr-layer-shell)
// ==============================================================================

use crate::dbus_menu::DbusMenuCategory;
use crate::layout::UiTree;
use crate::popover::{PopoverConfig, PopoverMenuModel};
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
    pub dbus_categories: Vec<DbusMenuCategory>,

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
            dbus_categories: Vec::new(),
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

    /// Actualiza dinámicamente la aplicación activa y las categorías del Menú Global D-Bus.
    pub fn update_active_app(&mut self, app_name: String, menus: Vec<String>) -> Result<()> {
        self.model.app_name = app_name;
        if !menus.is_empty() {
            self.model.menus = menus;
        } else {
            self.model.menus = vec![
                "Archivo".to_string(),
                "Edición".to_string(),
                "Visualización".to_string(),
                "Ventana".to_string(),
                "Ayuda".to_string(),
            ];
        }

        self.ui = UiTree::new();
        self.nodes = build_top_bar(&mut self.ui, &self.config, &self.model).ok();
        self.needs_redraw = true;

        Ok(())
    }

    /// Actualiza las categorías D-Bus con sus ítems deserializados para los popovers.
    pub fn update_dbus_categories(&mut self, categories: Vec<DbusMenuCategory>) {
        self.dbus_categories = categories;
    }

    /// Reconstruye el árbol de la interfaz de usuario y marca necesidad de redibujado.
    pub fn rebuild_ui(&mut self) -> Result<()> {
        self.ui = UiTree::new();
        self.nodes = build_top_bar(&mut self.ui, &self.config, &self.model).ok();
        self.needs_redraw = true;
        Ok(())
    }

    /// Alterna el estado abierto/cerrado del Menú del Sistema Apple.
    pub fn toggle_apple_menu(&mut self) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "apple");

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_x = self.config.margin_x + 8.0;
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = PopoverMenuModel::apple_menu(anchor_x, anchor_y);
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Alterna el estado abierto/cerrado de una categoría de menú específico.
    pub fn toggle_menu(&mut self, menu_title: &str, anchor_x: f32) -> Result<()> {
        let menu_id = format!("menu:{}", menu_title);
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == menu_id);

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = if let Some(cat) = self
                .dbus_categories
                .iter()
                .find(|c| c.title.eq_ignore_ascii_case(menu_title))
            {
                PopoverMenuModel::from_dbus_category(anchor_x, anchor_y, cat)
            } else {
                PopoverMenuModel::sample_file_menu(anchor_x, anchor_y)
            };
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Despliega un menú popover adaptando la altura de la superficie en Wayland sin alterar la zona de exclusión.
    pub fn open_popover(&mut self, popover: PopoverMenuModel) -> Result<()> {
        let palette = crate::luminance::AdaptivePalette::new(self.model.is_dark_background);
        let popover_h = popover.total_height(&PopoverConfig::for_palette(&palette));
        let required_h = (self.config.height + self.config.margin_top + 8.0 + popover_h + 16.0) as u32;

        self.model.active_popover = Some(popover);
        if required_h > self.height {
            self.height = required_h;
            self.layer.set_size(0, self.height);
            self.layer.commit();
            self.configure_surface();
        }

        self.rebuild_ui()?;
        Ok(())
    }

    /// Cierra cualquier menú popover activo y restituye la altura mínima de la superficie en Wayland.
    pub fn close_popover(&mut self) -> Result<()> {
        if self.model.active_popover.is_some() {
            self.model.active_popover = None;
            let base_h = (self.config.height + self.config.margin_top + 4.0) as u32;
            self.height = base_h;
            self.layer.set_size(0, self.height);
            self.layer.commit();
            self.configure_surface();
            self.rebuild_ui()?;
        }
        Ok(())
    }

    /// Ejecuta la acción asociada al ítem de popover seleccionado y cierra el menú.
    pub fn handle_popover_action(&mut self, item_id: i32) {
        info!("🎯 Ejecutando acción de ítem de popover (id={})", item_id);
        match item_id {
            1 => {
                info!("ℹ️ Mostrando Acerca de Malus DE");
            }
            2 => {
                info!("⚙️ Abriendo Preferencias del Sistema (malus-de-settings)...");
                let _ = std::process::Command::new("malus-de-settings").spawn();
            }
            3 => {
                info!("📊 Abriendo Monitor del Sistema...");
                let _ = std::process::Command::new("gnome-system-monitor").spawn();
            }
            4 => {
                info!("🔄 Recargando Shell de Malus DE...");
                let _ = std::process::Command::new("malus-de-ctl").arg("reload").spawn();
            }
            5 => {
                info!("🔒 Bloqueando sesión con malus-de-session-lock...");
                let _ = std::process::Command::new("malus-de-session-lock").spawn();
            }
            6 => {
                info!("⚡ Invocando reinicio del sistema...");
                let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            }
            7 => {
                info!("🛑 Invocando apagado del equipo...");
                let _ = std::process::Command::new("systemctl").arg("poweroff").spawn();
            }
            _ => {
                info!("Ejecutando acción de menú con id={}", item_id);
            }
        }
        let _ = self.close_popover();
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
                    let hit_tag = hit
                        .and_then(|n| self.ui.node_data(n))
                        .and_then(|d| d.tag.clone());

                    if self.model.active_popover.is_some() {
                        let mut needs_refresh = false;
                        let mut new_menu_switch = None;

                        if let Some(ref tag) = hit_tag {
                            if tag.starts_with("popover_item:") {
                                let parts: Vec<&str> = tag.split(':').collect();
                                if parts.len() >= 3 {
                                    if let Ok(idx) = parts[1].parse::<usize>() {
                                        if let Some(ref mut popover) = self.model.active_popover {
                                            if popover.hovered_index != Some(idx) {
                                                popover.hovered_index = Some(idx);
                                                needs_refresh = true;
                                            }
                                        }
                                    }
                                }
                            } else if tag == "apple_menu" {
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != "apple" {
                                        new_menu_switch = Some(("apple", 0.0));
                                    }
                                }
                            } else if tag.starts_with("menu:") {
                                let menu_title = &tag[5..];
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != *tag {
                                        let anchor_x = hit
                                            .and_then(|n| self.ui.taffy.layout(n).ok())
                                            .map(|l| l.location.x)
                                            .unwrap_or(80.0);
                                        new_menu_switch = Some((menu_title, anchor_x));
                                    }
                                }
                            }
                        } else {
                            if let Some(ref mut popover) = self.model.active_popover {
                                if popover.hovered_index.is_some() {
                                    popover.hovered_index = None;
                                    needs_refresh = true;
                                }
                            }
                        }

                        if let Some((menu, ax)) = new_menu_switch {
                            if menu == "apple" {
                                let _ = self.toggle_apple_menu();
                            } else {
                                let _ = self.toggle_menu(menu, ax);
                            }
                        } else if needs_refresh {
                            let _ = self.rebuild_ui();
                        }
                    } else if hit != self.hovered_node {
                        let palette = crate::luminance::AdaptivePalette::new(self.model.is_dark_background);
                        // Restaurar estilo anterior al salir del botón
                        if let Some(prev) = self.hovered_node {
                            if let Some(data) = self.ui.node_data(prev) {
                                let mut style = data.style.clone();
                                style.bg_color = Some(palette.button_idle_bg);
                                style.border_color = None;
                                style.border_width = 0.0;
                                self.ui.set_visual_style(prev, style);
                            }
                        }
                        // Aplicar hover highlight estilo escarcha líquida
                        if let Some(curr) = hit {
                            if let Some(data) = self.ui.node_data(curr) {
                                let mut style = data.style.clone();
                                style.bg_color = Some(palette.button_hover_bg);
                                style.corner_radius = 6.0;
                                self.ui.set_visual_style(curr, style);
                            }
                        }
                        self.hovered_node = hit;
                        self.needs_redraw = true;
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let (x, y) = event.position;
                    let hit = self.ui.hit_test([x as f32, y as f32]);
                    let hit_tag = hit
                        .and_then(|n| self.ui.node_data(n))
                        .and_then(|d| d.tag.clone());

                    if let Some(tag) = hit_tag {
                        info!("Clic en botón Wayland (botón={}): {}", button, tag);
                        if tag == "apple_menu" {
                            let _ = self.toggle_apple_menu();
                        } else if tag.starts_with("menu:") {
                            let menu_title = &tag[5..];
                            let anchor_x = hit
                                .and_then(|n| self.ui.taffy.layout(n).ok())
                                .map(|l| l.location.x)
                                .unwrap_or(80.0);
                            let _ = self.toggle_menu(menu_title, anchor_x);
                        } else if tag.starts_with("popover_item:") {
                            let parts: Vec<&str> = tag.split(':').collect();
                            if parts.len() >= 3 {
                                let item_id: i32 = parts[2].parse().unwrap_or(-1);
                                self.handle_popover_action(item_id);
                            }
                        }
                    } else if self.model.active_popover.is_some() {
                        let _ = self.close_popover();
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
