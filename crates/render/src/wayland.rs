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
    delegate_seat, delegate_shm,
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
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
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
    pub compositor_state: CompositorState,
    pub layer_shell_state: LayerShell,
    pub shm_state: Shm,
    pub slot_pool: Option<SlotPool>,
    pub qh: QueueHandle<Self>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub popover_blur: Option<(wl_surface::WlSurface, LayerSurface, Buffer)>,
    pub pending_blur_commit: bool,
    pub pending_configure: bool,

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
    pub is_dragging_slider: Option<String>,
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
        let shm_state =
            Shm::bind(globals, qh).context("Protocolo wl_shm no disponible")?;
        let slot_pool = SlotPool::new(1920 * 1080 * 4, &shm_state).ok();

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

        let mut seat_state = SeatState::new(globals, qh);
        let mut pointer = None;
        for seat in seat_state.seats() {
            if let Some(info) = seat_state.info(&seat) {
                if info.has_pointer {
                    info!("🖱️ [Wayland] Vinculando wl_pointer desde asiento existente");
                    pointer = seat_state.get_pointer(qh, &seat).ok();
                    break;
                }
            }
        }

        Ok(Self {
            registry_state: RegistryState::new(globals),
            seat_state,
            output_state: OutputState::new(globals, qh),
            compositor_state,
            layer_shell_state,
            shm_state,
            slot_pool,
            qh: qh.clone(),
            pointer,
            popover_blur: None,
            pending_blur_commit: false,
            pending_configure: false,
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
            is_dragging_slider: None,
        })
    }

    /// Re-renderiza el frame completo con WGPU y lo presenta a Wayland.
    pub fn render(&mut self) -> Result<()> {
        if self.pending_configure || self.width == 0 || self.height == 0 {
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

        // Si hay una superficie de desenfoque pendiente, presentar su buffer de forma
        // perfectamente sincronizada con la presentación del frame del popover.
        if self.pending_blur_commit {
            if let Some((ref b_surf, _, ref b_buf)) = self.popover_blur {
                b_surf.attach(Some(b_buf.wl_buffer()), 0, 0);
                b_surf.damage_buffer(0, 0, i32::MAX, i32::MAX);
                b_surf.commit();
                self.pending_blur_commit = false;
            }
        }

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
    pub fn toggle_apple_menu(&mut self, anchor_x: f32) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "apple");

        if is_open {
            self.close_popover()?;
        } else {
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

    /// Alterna el popover de estado de Red Wi-Fi.
    pub fn toggle_wifi_menu(&mut self, anchor_x: f32) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "status:wifi");

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = PopoverMenuModel::wifi_popover(anchor_x, anchor_y, "Mega_5G_B982", 100);
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Alterna el popover de ajuste de Sonido / Volumen.
    pub fn toggle_volume_menu(&mut self, anchor_x: f32) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "status:volume");

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = PopoverMenuModel::volume_popover(anchor_x, anchor_y, 62, false);
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Alterna el popover de estado de Batería y periféricos.
    pub fn toggle_battery_menu(&mut self, anchor_x: f32) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "status:battery");

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = PopoverMenuModel::battery_popover(anchor_x, anchor_y, 100, false);
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Alterna el popover del Centro de Control estilo macOS.
    pub fn toggle_control_center_menu(&mut self, anchor_x: f32) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "control_center");

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = PopoverMenuModel::control_center_popover(anchor_x, anchor_y);
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Alterna el popover de la Bandeja del Sistema (estilo Bartender / Hidden by the notch).
    pub fn toggle_tray_menu(&mut self, anchor_x: f32) -> Result<()> {
        let is_open = self
            .model
            .active_popover
            .as_ref()
            .map_or(false, |p| p.menu_id == "status:tray");

        if is_open {
            self.close_popover()?;
        } else {
            let anchor_y = self.config.height + self.config.margin_top + 4.0;
            let popover = PopoverMenuModel::tray_popover(anchor_x, anchor_y, &[]);
            self.open_popover(popover)?;
        }
        Ok(())
    }

    /// Ajusta dinámicamente el valor del deslizador según la posición horizontal del cursor.
    pub fn handle_slider_interaction(&mut self, tag: &str, cursor_x: f32, hit: Option<NodeId>) {
        let layout_info = hit.and_then(|n| self.ui.absolute_layout(n));
        let (sx, sw) = if let Some((x, _y, w, _h)) = layout_info {
            (x, w.max(40.0))
        } else {
            let panel_x = self.model.active_popover.as_ref().map(|p| p.anchor_x).unwrap_or(8.0);
            (panel_x + 36.0, 240.0)
        };

        let pct = (((cursor_x - sx) / sw).clamp(0.0, 1.0) * 100.0) as u32;

        if tag.contains("sound") {
            if let Some(ref mut p) = self.model.active_popover {
                if let Some(ref mut cc) = p.control_center {
                    cc.sound_volume = pct;
                    cc.sound_muted = false;
                }
            }
            let _ = std::process::Command::new("wpctl")
                .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{}%", pct)])
                .spawn();
            let _ = self.rebuild_ui();
            let _ = self.render();
        } else if tag.contains("display") {
            if let Some(ref mut p) = self.model.active_popover {
                if let Some(ref mut cc) = p.control_center {
                    cc.display_brightness = pct;
                }
            }
            let _ = std::process::Command::new("brightnessctl")
                .args(["set", &format!("{}%", pct)])
                .spawn();
            let _ = self.rebuild_ui();
            let _ = self.render();
        }
    }

    /// Alterna cíclicamente la distribución de teclado activa y actualiza el distintivo.
    pub fn cycle_keyboard_layout(&mut self) -> Result<()> {
        let mut cfg = malus_de_common::config::InputConfig::load();
        let next_layout = match cfg.keyboard_layout.as_str() {
            "latam" => "es",
            "es" => "us",
            _ => "latam",
        };
        cfg.keyboard_layout = next_layout.to_string();
        cfg.keyboard_variant = String::new();
        let _ = cfg.save();
        let _ = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::SetKeyboardLayout {
                layout: next_layout.to_string(),
                variant: String::new(),
            },
        );
        let badge = cfg.layout_short_badge();
        if let Some(item) = self.model.status_items.iter_mut().find(|i| i.tag == "status:keyboard") {
            item.text = badge.to_string();
        }
        self.rebuild_ui()?;
        let _ = self.render();
        Ok(())
    }

    /// Actualiza la paleta del tema de color dinámicamente si el contraste del fondo cambia.
    pub fn set_dark_background(&mut self, is_dark: bool) -> Result<()> {
        if self.model.is_dark_background != is_dark {
            info!("🎨 [TopBar] Actualizando paleta de color del sistema: is_dark_background={}", is_dark);
            self.model.is_dark_background = is_dark;
            self.rebuild_ui()?;
            let _ = self.render();
        }
        Ok(())
    }

    /// Despliega un menú popover adaptando la altura de la superficie en Wayland y activando desenfoque de fondo.
    pub fn open_popover(&mut self, popover: PopoverMenuModel) -> Result<()> {
        // Cerrar blur anterior si estuviera activo
        if let Some((surface, layer, buffer)) = self.popover_blur.take() {
            drop(buffer);
            drop(layer);
            drop(surface);
        }

        let palette = crate::luminance::AdaptivePalette::new(self.model.is_dark_background);
        let popover_cfg = PopoverConfig::for_palette(&palette);
        let popover_w = if popover.control_center.is_some() {
            320.0
        } else if popover.menu_id == "status:tray" {
            240.0
        } else {
            popover_cfg.width
        };
        let popover_h = popover.total_height(&popover_cfg);
        let required_h = (self.config.height + self.config.margin_top + 8.0 + popover_h + 16.0) as u32;

        info!(
            "📂 [TopBar] Desplegando popover '{}' en x={} (altura requerida: {}px)",
            popover.menu_id, popover.anchor_x, required_h
        );

        // Crear superficie auxiliar de desenfoque de fondo en el compositor (namespace="malus-notifications")
        let blur_surface = self.compositor_state.create_surface(&self.qh);
        if let Ok(empty_region) = smithay_client_toolkit::compositor::Region::new(&self.compositor_state) {
            blur_surface.set_input_region(Some(empty_region.wl_region()));
        }

        let blur_layer = self.layer_shell_state.create_layer_surface(
            &self.qh,
            blur_surface.clone(),
            Layer::Top,
            Some("malus-notifications"),
            None,
        );
        // El compositor ya desplaza las capas Top por la exclusive_zone de la barra superior.
        // El margen superior debe ser exactamente el desfase respecto al borde de la barra.
        let base_top_bar_h = (self.config.height + self.config.margin_top + 4.0) as i32;
        let blur_margin_top = (popover.anchor_y as i32 - base_top_bar_h).max(0);
        blur_layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        blur_layer.set_margin(blur_margin_top, 0, 0, popover.anchor_x as i32);
        blur_layer.set_size(popover_w as u32, popover_h as u32);
        blur_layer.set_exclusive_zone(0);
        blur_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        blur_layer.commit();

        if let Some(ref mut pool) = self.slot_pool {
            let stride = (popover_w as i32) * 4;
            if let Ok((buffer, canvas)) = pool.create_buffer(
                popover_w as i32,
                popover_h as i32,
                stride,
                wayland_client::protocol::wl_shm::Format::Argb8888,
            ) {
                canvas.fill(0);
                // NOTA: No hacemos attach ni commit aquí para evitar desfase visual.
                // Guardamos el buffer y diferimos el commit hasta que self.render()
                // presente el frame del menú con WGPU en la misma pasada del compositor.
                self.popover_blur = Some((blur_surface, blur_layer, buffer));
            }
        }

        let height_changed = self.height != required_h;
        self.model.active_popover = Some(popover);
        self.pending_blur_commit = true;

        if height_changed {
            self.pending_configure = true;
            self.layer.set_size(0, required_h);
            self.layer.commit();
        } else {
            self.rebuild_ui()?;
            self.configure_surface();
            let _ = self.render();
        }
        Ok(())
    }

    /// Cierra cualquier menú popover activo y restituye la altura mínima de la superficie en Wayland.
    pub fn close_popover(&mut self) -> Result<()> {
        self.pending_blur_commit = false;

        if self.model.active_popover.is_some() {
            info!("📁 [TopBar] Cerrando popover y restituyendo altura mínima de superficie");
            self.model.active_popover = None;
            let base_h = (self.config.height + self.config.margin_top + 4.0) as u32;
            let height_changed = self.height != base_h;

            if height_changed {
                self.pending_configure = true;
                // No destruimos popover_blur inmediatamente para evitar que la capa de desenfoque
                // desaparezca una fracción de segundo antes que el contenido del menú.
                // La destrucción se realiza en configure() en perfecta sincronía con el frame cerrado.
                self.layer.set_size(0, base_h);
                self.layer.commit();
            } else {
                if let Some((surface, layer, buffer)) = self.popover_blur.take() {
                    drop(buffer);
                    drop(layer);
                    drop(surface);
                }
                self.rebuild_ui()?;
                self.configure_surface();
                let _ = self.render();
            }
        } else if let Some((surface, layer, buffer)) = self.popover_blur.take() {
            drop(buffer);
            drop(layer);
            drop(surface);
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
            10 => {
                info!("🛍️ Abriendo Software...");
                let mut cmd = std::process::Command::new("gnome-software");
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
            }
            11 => {
                info!("🕒 Abriendo Ítems recientes...");
            }
            12 => {
                info!("💤 Poniendo el equipo en reposo...");
                let _ = std::process::Command::new("systemctl").arg("suspend").spawn();
            }
            13 => {
                info!("🚪 Cerrando sesión de usuario...");
                let _ = std::process::Command::new("gnome-session-quit").args(["--logout", "--no-prompt"]).spawn();
            }
            201 => {
                info!("📶 Alternando estado de Wi-Fi");
                let _ = std::process::Command::new("nmcli").args(["radio", "wifi"]).spawn();
            }
            202 => {
                info!("🌐 Abriendo Preferencias de Wi-Fi");
                let mut cmd = std::process::Command::new("gnome-control-center");
                cmd.arg("wifi");
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
            }
            212 => {
                info!("🔊 Alternando silencio de audio");
                let _ = std::process::Command::new("wpctl").args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]).spawn();
            }
            213 => {
                info!("🔊 Abriendo Preferencias de Sonido");
                let mut cmd = std::process::Command::new("gnome-control-center");
                cmd.arg("sound");
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
            }
            223 => {
                info!("🔋 Abriendo Preferencias de Batería y Energía");
                let mut cmd = std::process::Command::new("gnome-control-center");
                cmd.arg("power");
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
            }
            235 => {
                info!("⚙️ Abriendo Configuración del Sistema");
                let mut cmd = std::process::Command::new("malus-de-settings");
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
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

    /// Actualiza el ancho de la barra superior a partir de un monitor específico.
    pub fn update_width_for_output(&mut self, output: &wl_output::WlOutput) {
        if let Some(info) = self.output_state.info(output) {
            let detected_width = if let Some(logical_size) = info.logical_size {
                logical_size.0 as u32
            } else if let Some(mode) = info.modes.iter().find(|m| m.current) {
                let scale = info.scale_factor.max(1) as u32;
                (mode.dimensions.0 as u32) / scale
            } else {
                0
            };

            if detected_width > 0 && detected_width != self.width {
                info!("🖥️ [TopBar] Ancho de superficie ajustado al monitor activo: {}px (anterior: {}px)", detected_width, self.width);
                self.width = detected_width;
                self.configure_surface();
                let _ = self.render();
            }
        }
    }

    /// Actualiza el ancho de la barra superior a partir del primer monitor (primario).
    pub fn update_output_geometry(&mut self) {
        if let Some(output) = self.output_state.outputs().next() {
            self.update_width_for_output(&output);
        }
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
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if layer.wl_surface() != self.layer.wl_surface() {
            return;
        }

        let (new_width, new_height) = configure.new_size;
        if new_width > 0 {
            self.width = new_width;
        } else {
            self.update_output_geometry();
        }

        // Si hay un popover activo, la altura debe preservar el menú desplegado y no ser recortada
        if let Some(ref popover) = self.model.active_popover {
            let palette = crate::luminance::AdaptivePalette::new(self.model.is_dark_background);
            let popover_h = popover.total_height(&PopoverConfig::for_palette(&palette));
            self.height = (self.config.height + self.config.margin_top + 8.0 + popover_h + 16.0) as u32;
        } else {
            let base_h = (self.config.height + self.config.margin_top + 4.0) as u32;
            self.height = if new_height > 0 { new_height.min(base_h) } else { base_h };
            // Al cerrarse el popover, destruimos la capa de desenfoque exactamente en sincronía
            // con la reconfiguración y presentación del fotograma colapsado.
            if let Some((surface, layer, buffer)) = self.popover_blur.take() {
                drop(buffer);
                drop(layer);
                drop(surface);
            }
        }

        info!("📐 [LayerShell] configure recibido: new_size=({}, {}), altura asignada={}", new_width, new_height, self.height);
        self.pending_configure = false;
        self.configure_surface();
        let _ = self.rebuild_ui();
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
        output: &wl_output::WlOutput,
    ) {
        self.update_width_for_output(output);
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

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.update_output_geometry();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.update_output_geometry();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for WaylandTopBarApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.pointer.is_none() {
            if let Some(info) = self.seat_state.info(&seat) {
                if info.has_pointer {
                    info!("🖱️ [Wayland] Vinculando wl_pointer a nuevo asiento");
                    self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
                }
            }
        }
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            info!("🖱️ [Wayland] Asiento adquirió Capability::Pointer. Vinculando wl_pointer...");
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }

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

                    // Si estamos arrastrando activamente un deslizador interactivo (Volumen o Pantalla)
                    if let Some(ref slider_tag) = self.is_dragging_slider.clone() {
                        self.handle_slider_interaction(slider_tag, x as f32, hit);
                        continue;
                    }

                    if self.model.active_popover.is_some() {
                        let mut needs_refresh = false;
                        let mut new_menu_switch = None;

                        if let Some(ref tag) = hit_tag {
                            let (bx, _by, bw, _bh) = hit
                                .and_then(|n| self.ui.absolute_layout(n))
                                .unwrap_or((8.0, 0.0, 30.0, 24.0));
                            let popover_w = 265.0;
                            let left_ax = bx.clamp(8.0, (self.width as f32 - popover_w - 8.0).max(8.0));
                            let right_ax = (bx + bw - popover_w).clamp(8.0, (self.width as f32 - popover_w - 8.0).max(8.0));

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
                                        new_menu_switch = Some(("apple", left_ax));
                                    }
                                }
                            } else if tag.starts_with("menu:") {
                                let menu_title = &tag[5..];
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != *tag {
                                        new_menu_switch = Some((menu_title, left_ax));
                                    }
                                }
                            } else if tag == "status:wifi" {
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != "status:wifi" {
                                        new_menu_switch = Some(("status:wifi", right_ax));
                                    }
                                }
                            } else if tag == "status:volume" {
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != "status:volume" {
                                        new_menu_switch = Some(("status:volume", right_ax));
                                    }
                                }
                            } else if tag == "status:battery" {
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != "status:battery" {
                                        new_menu_switch = Some(("status:battery", right_ax));
                                    }
                                }
                            } else if tag == "control_center" || tag == "status:control_center" {
                                if let Some(ref popover) = self.model.active_popover {
                                    if popover.menu_id != "control_center" {
                                        new_menu_switch = Some(("control_center", right_ax));
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
                            match menu {
                                "apple" => { let _ = self.toggle_apple_menu(ax); },
                                "status:wifi" => { let _ = self.toggle_wifi_menu(ax); },
                                "status:volume" => { let _ = self.toggle_volume_menu(ax); },
                                "status:battery" => { let _ = self.toggle_battery_menu(ax); },
                                "control_center" => { let _ = self.toggle_control_center_menu(ax); },
                                other => { let _ = self.toggle_menu(other, ax); },
                            }
                        } else if needs_refresh {
                            let _ = self.rebuild_ui();
                            let _ = self.render();
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
                        // Aplicar hover highlight estilo escarcha líquida con radio suave de píldora
                        if let Some(curr) = hit {
                            if let Some(data) = self.ui.node_data(curr) {
                                let mut style = data.style.clone();
                                style.bg_color = Some(palette.button_hover_bg);
                                style.corner_radius = 13.0;
                                style.squircle_power = 2.2;
                                self.ui.set_visual_style(curr, style);
                            }
                        }
                        self.hovered_node = hit;
                        self.needs_redraw = true;
                        let _ = self.render();
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let (x, y) = event.position;
                    let hit = self.ui.hit_test([x as f32, y as f32]);
                    let hit_tag = hit
                        .and_then(|n| self.ui.node_data(n))
                        .and_then(|d| d.tag.clone());

                    if let Some(tag) = hit_tag {
                        info!("🖱️ [TopBar] Clic en elemento '{}' (botón={})", tag, button);
                        let (bx, _by, bw, _bh) = hit
                            .and_then(|n| self.ui.absolute_layout(n))
                            .unwrap_or((8.0, 0.0, 30.0, 24.0));
                        let popover_w = 265.0;
                        let left_ax = bx.clamp(8.0, (self.width as f32 - popover_w - 8.0).max(8.0));
                        let right_ax = (bx + bw - popover_w).clamp(8.0, (self.width as f32 - popover_w - 8.0).max(8.0));

                        if tag == "apple_menu" {
                            let _ = self.toggle_apple_menu(left_ax);
                        } else if tag.starts_with("menu:") {
                            let menu_title = &tag[5..];
                            let _ = self.toggle_menu(menu_title, left_ax);
                        } else if tag == "status:wifi" {
                            let _ = self.toggle_wifi_menu(right_ax);
                        } else if tag == "status:volume" {
                            let _ = self.toggle_volume_menu(right_ax);
                        } else if tag == "status:battery" {
                            let _ = self.toggle_battery_menu(right_ax);
                        } else if tag == "status:tray" {
                            let tray_w = 240.0;
                            let tray_ax = (bx + bw - tray_w).clamp(8.0, (self.width as f32 - tray_w - 8.0).max(8.0));
                            let _ = self.toggle_tray_menu(tray_ax);
                        } else if tag == "control_center" || tag == "status:control_center" {
                            let cc_w = 320.0;
                            let cc_ax = (bx + bw - cc_w).clamp(8.0, (self.width as f32 - cc_w - 8.0).max(8.0));
                            let _ = self.toggle_control_center_menu(cc_ax);
                        } else if tag == "status:keyboard" {
                            let _ = self.cycle_keyboard_layout();
                        } else if tag == "status:spotlight" {
                            let _ = std::process::Command::new("malus-de-spotlight").spawn();
                        } else if tag == "clock" {
                            let _ = std::process::Command::new("malus-de-notifications").spawn();
                        } else if tag.starts_with("slider:") || tag.starts_with("slider_track:") || tag.starts_with("slider_fill:") || tag.starts_with("slider_row:") || tag.starts_with("slider_box:") {
                            self.is_dragging_slider = Some(tag.clone());
                            self.handle_slider_interaction(&tag, x as f32, hit);
                        } else if tag == "cc:wifi" {
                            if let Some(ref mut p) = self.model.active_popover {
                                if let Some(ref mut cc) = p.control_center {
                                    cc.wifi_enabled = !cc.wifi_enabled;
                                }
                            }
                            let _ = std::process::Command::new("nmcli").args(["radio", "wifi"]).spawn();
                            let _ = self.rebuild_ui();
                            let _ = self.render();
                        } else if tag == "cc:bluetooth" {
                            if let Some(ref mut p) = self.model.active_popover {
                                if let Some(ref mut cc) = p.control_center {
                                    cc.bluetooth_enabled = !cc.bluetooth_enabled;
                                }
                            }
                            let _ = std::process::Command::new("rfkill").args(["toggle", "bluetooth"]).spawn();
                            let _ = self.rebuild_ui();
                            let _ = self.render();
                        } else if tag == "cc:airdrop" {
                            if let Some(ref mut p) = self.model.active_popover {
                                if let Some(ref mut cc) = p.control_center {
                                    cc.airdrop_enabled = !cc.airdrop_enabled;
                                }
                            }
                            let _ = std::process::Command::new("malus-de-link").spawn();
                            let _ = self.rebuild_ui();
                            let _ = self.render();
                        } else if tag == "cc:media_play" {
                            if let Some(ref mut p) = self.model.active_popover {
                                if let Some(ref mut cc) = p.control_center {
                                    cc.media_playing = !cc.media_playing;
                                }
                            }
                            let _ = std::process::Command::new("playerctl").args(["play-pause"]).spawn();
                            let _ = self.rebuild_ui();
                            let _ = self.render();
                        } else if tag == "cc:media_prev" {
                            let _ = std::process::Command::new("playerctl").args(["previous"]).spawn();
                        } else if tag == "cc:media_next" {
                            let _ = std::process::Command::new("playerctl").args(["next"]).spawn();
                        } else if tag == "cc:focus" {
                            if let Some(ref mut p) = self.model.active_popover {
                                if let Some(ref mut cc) = p.control_center {
                                    cc.focus_enabled = !cc.focus_enabled;
                                }
                            }
                            let _ = self.rebuild_ui();
                            let _ = self.render();
                        } else if tag == "cc:dark_mode" {
                            let is_dark = !self.model.is_dark_background;
                            malus_de_common::config::sync_system_appearance(is_dark);
                            let _ = malus_de_common::ipc::IpcClient::send_command(
                                malus_de_common::ipc::IpcMessage::ThemeChanged {
                                    theme_name: if is_dark { "Adwaita-Dark".to_string() } else { "Adwaita-Light".to_string() },
                                    is_dark,
                                },
                            );
                            let _ = self.set_dark_background(is_dark);
                            if let Some(ref mut p) = self.model.active_popover {
                                if let Some(ref mut cc) = p.control_center {
                                    cc.dark_mode = is_dark;
                                }
                            }
                            let _ = self.rebuild_ui();
                            let _ = self.render();
                        } else if tag == "cc:screenshot" {
                            let _ = std::process::Command::new("malus-de-screenshot").spawn();
                            let _ = self.close_popover();
                        } else if tag == "cc:keyboard" {
                            let _ = self.cycle_keyboard_layout();
                        } else if tag == "cc:lock" {
                            let _ = std::process::Command::new("malus-de-session-lock").spawn();
                            let _ = self.close_popover();
                        } else if tag == "cc:edit_controls" || tag == "tray:manage" {
                            let _ = std::process::Command::new("malus-de-settings").spawn();
                            let _ = self.close_popover();
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
                PointerEventKind::Release { .. } => {
                    self.is_dragging_slider = None;
                }
                PointerEventKind::Leave { .. } => {
                    self.hovered_node = None;
                    self.needs_redraw = true;
                    let _ = self.render();
                }
                _ => {}
            }
        }

        if self.needs_redraw {
            let _ = self.render();
        }
    }
}

impl ShmHandler for WaylandTopBarApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

delegate_compositor!(WaylandTopBarApp);
delegate_output!(WaylandTopBarApp);
delegate_seat!(WaylandTopBarApp);
delegate_pointer!(WaylandTopBarApp);
delegate_layer!(WaylandTopBarApp);
delegate_shm!(WaylandTopBarApp);
delegate_registry!(WaylandTopBarApp);

impl ProvidesRegistryState for WaylandTopBarApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
