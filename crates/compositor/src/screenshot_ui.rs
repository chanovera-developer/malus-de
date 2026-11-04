use cairo::{Context, Format, ImageSurface, LinearGradient, Operator};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    input::keyboard::{keysyms, Keysym, ModifiersState},
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;

pub const SQUIRCLE_EXPONENT: f64 = 4.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenshotPointerResult {
    Ignored,
    Consumed,
    TriggerAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenshotUiMode {
    CaptureFull,
    CaptureWindow,
    CaptureArea,
    RecordFull,
    RecordArea,
}

impl ScreenshotUiMode {
    pub fn is_video(&self) -> bool {
        matches!(self, Self::RecordFull | Self::RecordArea)
    }

    pub fn is_area(&self) -> bool {
        matches!(self, Self::CaptureArea | Self::RecordArea)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    Desktop,
    Documents,
    Clipboard,
    Screenshots,
}

#[derive(Clone, Debug)]
pub struct UiOptions {
    pub save_to: SaveTarget,
    pub timer_seconds: u32,
    pub remember_selection: bool,
    pub include_audio: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectionArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HandleType {
    None,
    MoveBox,
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NewSelection,
}

pub struct ScreenshotUiState {
    pub is_open: bool,
    pub mode: ScreenshotUiMode,
    pub options: UiOptions,
    pub show_options_menu: bool,
    pub target_output: Option<String>,
    pub selection: SelectionArea,
    pub is_recording: bool,
    pub recording_start: Option<Instant>,
    pub recording_dest: Option<PathBuf>,

    // Estado de interacción
    active_handle: HandleType,
    drag_start_pointer: (f64, f64),
    drag_start_area: SelectionArea,
    hovered_item: Option<u8>, // 0: Cerrar, 1..5: Modos, 6: Opciones, 7: Acción, 8: Parar Grabación
    hovered_menu_item: Option<usize>, // 0..8 para el menú emergente

    // Buffers de renderizado del compositor
    render_buffer: Option<MemoryRenderBuffer>,
    render_buffer_size: (i32, i32),
    mask_buffer: Vec<u8>,
}

impl ScreenshotUiState {
    pub fn new() -> Self {
        Self {
            is_open: false,
            mode: ScreenshotUiMode::CaptureArea,
            options: UiOptions {
                save_to: SaveTarget::Desktop,
                timer_seconds: 0,
                remember_selection: true,
                include_audio: false,
            },
            show_options_menu: false,
            target_output: None,
            selection: SelectionArea {
                x: 200.0,
                y: 180.0,
                width: 800.0,
                height: 520.0,
            },
            is_recording: false,
            recording_start: None,
            recording_dest: None,
            active_handle: HandleType::None,
            drag_start_pointer: (0.0, 0.0),
            drag_start_area: SelectionArea {
                x: 200.0,
                y: 180.0,
                width: 800.0,
                height: 520.0,
            },
            hovered_item: None,
            hovered_menu_item: None,
            render_buffer: None,
            render_buffer_size: (0, 0),
            mask_buffer: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open || self.is_recording
    }

    pub fn open(&mut self, output_geo: Rectangle<i32, Logical>, output_name: String) {
        self.is_open = true;
        self.target_output = Some(output_name);
        self.show_options_menu = false;
        self.hovered_menu_item = None;

        let sw = output_geo.size.w as f64;
        let sh = output_geo.size.h as f64;

        if !self.options.remember_selection
            || self.selection.width <= 10.0
            || self.selection.height <= 10.0
        {
            let init_w = (sw * 0.55).min(960.0).max(320.0);
            let init_h = (sh * 0.50).min(640.0).max(240.0);
            self.selection = SelectionArea {
                x: ((sw - init_w) * 0.5).round(),
                y: ((sh - init_h) * 0.5 - 20.0).max(40.0).round(),
                width: init_w.round(),
                height: init_h.round(),
            };
        }
        info!("📸 Barra de captura squircle abierta en pantalla {}", sw);
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.show_options_menu = false;
        self.hovered_menu_item = None;
        self.active_handle = HandleType::None;
        info!("📸 Barra de captura cerrada");
    }

    pub fn should_render_on_output(&self, output_name: &str) -> bool {
        self.is_open()
            && self
                .target_output
                .as_deref()
                .map_or(true, |target| target == output_name)
    }

    pub fn update_pointer_motion(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        point: Point<f64, Logical>,
    ) {
        if !self.is_open() {
            return;
        }

        let lx = point.x - output_geo.loc.x as f64;
        let ly = point.y - output_geo.loc.y as f64;
        let sw = output_geo.size.w as f64;
        let sh = output_geo.size.h as f64;

        if self.is_recording {
            let (rx, ry, rw, rh) = recording_indicator_geometry(sw);
            if lx >= rx && lx <= rx + rw && ly >= ry && ly <= ry + rh {
                self.hovered_item = Some(8);
            } else {
                self.hovered_item = None;
            }
            return;
        }

        // Si hay arrastre activo sobre el área de selección
        if self.active_handle != HandleType::None {
            let offset_x = lx - self.drag_start_pointer.0;
            let offset_y = ly - self.drag_start_pointer.1;
            let orig = self.drag_start_area;

            match self.active_handle {
                HandleType::MoveBox => {
                    self.selection.x = (orig.x + offset_x).clamp(0.0, (sw - orig.width).max(0.0));
                    self.selection.y = (orig.y + offset_y).clamp(0.0, (sh - orig.height).max(0.0));
                }
                HandleType::NewSelection => {
                    let sx = self.drag_start_pointer.0;
                    let sy = self.drag_start_pointer.1;
                    let cur_x = lx.clamp(0.0, sw);
                    let cur_y = ly.clamp(0.0, sh);
                    self.selection = SelectionArea {
                        x: sx.min(cur_x),
                        y: sy.min(cur_y),
                        width: (sx - cur_x).abs().max(10.0),
                        height: (sy - cur_y).abs().max(10.0),
                    };
                }
                HandleType::NorthWest => {
                    let right = orig.x + orig.width;
                    let bottom = orig.y + orig.height;
                    let new_x = (orig.x + offset_x).clamp(0.0, right - 24.0);
                    let new_y = (orig.y + offset_y).clamp(0.0, bottom - 24.0);
                    self.selection.x = new_x;
                    self.selection.y = new_y;
                    self.selection.width = right - new_x;
                    self.selection.height = bottom - new_y;
                }
                HandleType::North => {
                    let bottom = orig.y + orig.height;
                    let new_y = (orig.y + offset_y).clamp(0.0, bottom - 24.0);
                    self.selection.y = new_y;
                    self.selection.height = bottom - new_y;
                }
                HandleType::NorthEast => {
                    let bottom = orig.y + orig.height;
                    let new_y = (orig.y + offset_y).clamp(0.0, bottom - 24.0);
                    let new_w = (orig.width + offset_x).clamp(24.0, sw - orig.x);
                    self.selection.y = new_y;
                    self.selection.height = bottom - new_y;
                    self.selection.width = new_w;
                }
                HandleType::East => {
                    self.selection.width = (orig.width + offset_x).clamp(24.0, sw - orig.x);
                }
                HandleType::SouthEast => {
                    self.selection.width = (orig.width + offset_x).clamp(24.0, sw - orig.x);
                    self.selection.height = (orig.height + offset_y).clamp(24.0, sh - orig.y);
                }
                HandleType::South => {
                    self.selection.height = (orig.height + offset_y).clamp(24.0, sh - orig.y);
                }
                HandleType::SouthWest => {
                    let right = orig.x + orig.width;
                    let new_x = (orig.x + offset_x).clamp(0.0, right - 24.0);
                    self.selection.x = new_x;
                    self.selection.width = right - new_x;
                    self.selection.height = (orig.height + offset_y).clamp(24.0, sh - orig.y);
                }
                HandleType::West => {
                    let right = orig.x + orig.width;
                    let new_x = (orig.x + offset_x).clamp(0.0, right - 24.0);
                    self.selection.x = new_x;
                    self.selection.width = right - new_x;
                }
                HandleType::None => {}
            }
            return;
        }

        // Comprobar menú opciones si está abierto
        if self.show_options_menu {
            let (menu_x, menu_y, menu_w, menu_h) = options_menu_geometry(sw, sh);
            if lx >= menu_x && lx <= menu_x + menu_w && ly >= menu_y && ly <= menu_y + menu_h {
                self.hovered_menu_item = hit_test_options_menu(ly - menu_y);
                self.hovered_item = Some(6);
                return;
            } else {
                self.hovered_menu_item = None;
            }
        }

        // Detección de elementos interactivos de la barra flotante
        let (tb_x, tb_y, tb_w, tb_h) = toolbar_geometry(sw, sh);
        if lx >= tb_x && lx <= tb_x + tb_w && ly >= tb_y && ly <= tb_y + tb_h {
            let rel_x = lx - tb_x;
            if rel_x < 44.0 {
                self.hovered_item = Some(0); // Cerrar
            } else if rel_x < 88.0 {
                self.hovered_item = Some(1); // Capturar Pantalla
            } else if rel_x < 132.0 {
                self.hovered_item = Some(2); // Capturar Ventana
            } else if rel_x < 176.0 {
                self.hovered_item = Some(3); // Capturar Área
            } else if rel_x < 222.0 {
                self.hovered_item = Some(4); // Grabar Pantalla
            } else if rel_x < 268.0 {
                self.hovered_item = Some(5); // Grabar Área
            } else if rel_x < 376.0 {
                self.hovered_item = Some(6); // Opciones ▾
            } else {
                self.hovered_item = Some(7); // Acción (Capturar / Grabar)
            }
            return;
        }

        self.hovered_item = None;
    }

    pub fn handle_pointer_button(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        point: Point<f64, Logical>,
        _button: u32,
        state: smithay::backend::input::ButtonState,
    ) -> ScreenshotPointerResult {
        if !self.is_open() {
            return ScreenshotPointerResult::Ignored;
        }

        let is_press = state == smithay::backend::input::ButtonState::Pressed;
        let lx = point.x - output_geo.loc.x as f64;
        let ly = point.y - output_geo.loc.y as f64;
        let sw = output_geo.size.w as f64;
        let sh = output_geo.size.h as f64;

        if !is_press {
            let was_dragging = self.active_handle != HandleType::None;
            self.active_handle = HandleType::None;
            return if was_dragging {
                ScreenshotPointerResult::Consumed
            } else {
                ScreenshotPointerResult::Ignored
            };
        }

        // Si está grabando y pulsa el botón de detener
        if self.is_recording {
            let (rx, ry, rw, rh) = recording_indicator_geometry(sw);
            if lx >= rx && lx <= rx + rw && ly >= ry && ly <= ry + rh {
                self.is_recording = false;
                self.recording_start = None;
                return ScreenshotPointerResult::TriggerAction;
            }
            return ScreenshotPointerResult::Ignored;
        }

        // 1. Clic en menú desplegable de opciones (si está abierto)
        if self.show_options_menu {
            let (mx, my, mw, mh) = options_menu_geometry(sw, sh);
            if lx >= mx && lx <= mx + mw && ly >= my && ly <= my + mh {
                if let Some(item_idx) = hit_test_options_menu(ly - my) {
                    match item_idx {
                        0 => self.options.save_to = SaveTarget::Desktop,
                        1 => self.options.save_to = SaveTarget::Documents,
                        2 => self.options.save_to = SaveTarget::Clipboard,
                        3 => self.options.save_to = SaveTarget::Screenshots,
                        4 => self.options.timer_seconds = 0,
                        5 => self.options.timer_seconds = 5,
                        6 => self.options.timer_seconds = 10,
                        7 => self.options.remember_selection = !self.options.remember_selection,
                        8 => self.options.include_audio = !self.options.include_audio,
                        _ => {}
                    }
                }
                return ScreenshotPointerResult::Consumed;
            } else {
                self.show_options_menu = false;
                self.hovered_menu_item = None;
            }
        }

        // 2. Clic dentro de la barra de herramientas
        let (tb_x, tb_y, tb_w, tb_h) = toolbar_geometry(sw, sh);
        if lx >= tb_x && lx <= tb_x + tb_w && ly >= tb_y && ly <= tb_y + tb_h {
            let rel_x = lx - tb_x;
            if rel_x < 44.0 {
                self.close();
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 88.0 {
                self.mode = ScreenshotUiMode::CaptureFull;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 132.0 {
                self.mode = ScreenshotUiMode::CaptureWindow;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 176.0 {
                self.mode = ScreenshotUiMode::CaptureArea;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 222.0 {
                self.mode = ScreenshotUiMode::RecordFull;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 268.0 {
                self.mode = ScreenshotUiMode::RecordArea;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 376.0 {
                self.show_options_menu = !self.show_options_menu;
                self.hovered_menu_item = None;
                return ScreenshotPointerResult::Consumed;
            } else {
                // Clic en botón "Capturar" / "Grabar"
                self.show_options_menu = false;
                return ScreenshotPointerResult::TriggerAction;
            }
        }

        // 3. Clic para ajustar área de recorte (tiradores o arrastre)
        if self.mode.is_area() {
            let r = self.selection;
            let hit = 18.0;
            let handles = [
                (HandleType::NorthWest, r.x, r.y),
                (HandleType::North, r.x + r.width * 0.5, r.y),
                (HandleType::NorthEast, r.x + r.width, r.y),
                (HandleType::East, r.x + r.width, r.y + r.height * 0.5),
                (HandleType::SouthEast, r.x + r.width, r.y + r.height),
                (HandleType::South, r.x + r.width * 0.5, r.y + r.height),
                (HandleType::SouthWest, r.x, r.y + r.height),
                (HandleType::West, r.x, r.y + r.height * 0.5),
            ];

            let mut matched = HandleType::None;
            for (h, hx, hy) in handles {
                let dx = lx - hx;
                let dy = ly - hy;
                if dx * dx + dy * dy <= hit * hit {
                    matched = h;
                    break;
                }
            }

            if matched == HandleType::None {
                if lx >= r.x && lx <= r.x + r.width && ly >= r.y && ly <= r.y + r.height {
                    matched = HandleType::MoveBox;
                } else {
                    matched = HandleType::NewSelection;
                    self.selection = SelectionArea {
                        x: lx,
                        y: ly,
                        width: 4.0,
                        height: 4.0,
                    };
                }
            }

            self.active_handle = matched;
            self.drag_start_pointer = (lx, ly);
            self.drag_start_area = self.selection;
            return ScreenshotPointerResult::Consumed;
        }

        ScreenshotPointerResult::Ignored
    }

    pub fn handle_key_press(&mut self, sym: Keysym, _modifiers: ModifiersState) -> bool {
        if !self.is_open() {
            return false;
        }

        if sym == keysyms::KEY_Escape.into() {
            if self.is_recording {
                self.is_recording = false;
                self.recording_start = None;
            } else {
                self.close();
            }
            return true;
        }

        if sym == keysyms::KEY_Return.into() || sym == keysyms::KEY_KP_Enter.into() {
            return true; // Dispara acción
        }

        false
    }

    /// Renderiza la superficie visual nativa y genera la máscara de desenfoque Dual Kawase
    pub fn prepare_frame(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        scale: f64,
        is_dark: bool,
    ) -> Option<(Rectangle<i32, Physical>, Option<&[u8]>, (i32, i32), f32)> {
        if !self.is_open() {
            return None;
        }

        let sw = output_geo.size.w as f64;
        let sh = output_geo.size.h as f64;
        let phys_w = (sw * scale).round() as i32;
        let phys_h = (sh * scale).round() as i32;

        if phys_w <= 0 || phys_h <= 0 {
            return None;
        }

        let mut surface = ImageSurface::create(Format::ARgb32, phys_w, phys_h).ok()?;
        let mut mask_surface = ImageSurface::create(Format::ARgb32, phys_w, phys_h).ok()?;

        {
            let ctx = Context::new(&surface).ok()?;
            let mask_ctx = Context::new(&mask_surface).ok()?;
            ctx.scale(scale, scale);
            mask_ctx.scale(scale, scale);

            mask_ctx.set_operator(Operator::Source);
            mask_ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = mask_ctx.paint();
            mask_ctx.set_operator(Operator::Over);

            if self.is_recording {
                self.draw_recording_indicator(&ctx, &mask_ctx, sw, is_dark);
            } else {
                // 1. Fondo oscurecido con recorte nítido para selección
                self.draw_scrim_and_selection(&ctx, sw, sh);

                // 2. Barra de herramientas flotante squircle
                self.draw_toolbar(&ctx, &mask_ctx, sw, sh, is_dark);

                // 3. Menú de opciones emergente squircle si está abierto
                if self.show_options_menu {
                    self.draw_options_menu(&ctx, &mask_ctx, sw, sh, is_dark);
                }
            }
        }

        surface.flush();
        mask_surface.flush();

        if let Ok(mask_data) = mask_surface.data() {
            self.mask_buffer = mask_data.to_vec();
        }

        if let Ok(data) = surface.data() {
            if self.render_buffer.is_none() || self.render_buffer_size != (phys_w, phys_h) {
                self.render_buffer = Some(MemoryRenderBuffer::new(
                    Fourcc::Argb8888,
                    Size::from((phys_w, phys_h)),
                    1,
                    smithay::utils::Transform::Normal,
                    None,
                ));
                self.render_buffer_size = (phys_w, phys_h);
            }
            if let Some(ref mut buf) = self.render_buffer {
                let mut rctx = buf.render();
                let _ = rctx.draw(|dst| {
                    if dst.len() == data.len() {
                        dst.copy_from_slice(&data);
                    }
                    Ok::<_, ()>(vec![Rectangle::from_size((phys_w, phys_h).into())])
                });
            }
        }

        let full_phys = Rectangle::new((0, 0).into(), (phys_w, phys_h).into());
        Some((
            full_phys,
            Some(&self.mask_buffer),
            (phys_w, phys_h),
            1.0,
        ))
    }

    pub fn create_render_element(
        &self,
        renderer: &mut GlesRenderer,
        loc: Point<f64, Physical>,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let buffer = self.render_buffer.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc,
            buffer,
            Some(1.0),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }

    fn draw_scrim_and_selection(&self, ctx: &Context, sw: f64, sh: f64) {
        match self.mode {
            ScreenshotUiMode::CaptureFull | ScreenshotUiMode::RecordFull => {
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.12);
                let _ = ctx.paint();
            }
            ScreenshotUiMode::CaptureWindow => {
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.32);
                let _ = ctx.paint();
            }
            ScreenshotUiMode::CaptureArea | ScreenshotUiMode::RecordArea => {
                // Fondo oscuro translúcido
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.38);
                let _ = ctx.paint();

                let r = self.selection;
                let rx = r.x.clamp(0.0, sw);
                let ry = r.y.clamp(0.0, sh);
                let rw = r.width.clamp(10.0, sw - rx);
                let rh = r.height.clamp(10.0, sh - ry);

                // Recorte 100% transparente
                ctx.set_operator(Operator::Clear);
                ctx.rectangle(rx, ry, rw, rh);
                let _ = ctx.fill();

                ctx.set_operator(Operator::Over);

                // Línea exterior sutil oscura para contraste
                ctx.rectangle(rx - 0.5, ry - 0.5, rw + 1.0, rh + 1.0);
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.35);
                ctx.set_line_width(1.0);
                let _ = ctx.stroke();

                // Borde blanco nítido
                ctx.rectangle(rx, ry, rw, rh);
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.98);
                ctx.set_line_width(1.5);
                let _ = ctx.stroke();

                // 8 tiradores circulares estilo macOS
                let handles = [
                    (rx, ry),
                    (rx + rw * 0.5, ry),
                    (rx + rw, ry),
                    (rx + rw, ry + rh * 0.5),
                    (rx + rw, ry + rh),
                    (rx + rw * 0.5, ry + rh),
                    (rx, ry + rh),
                    (rx, ry + rh * 0.5),
                ];
                for (hx, hy) in handles {
                    ctx.arc(hx, hy, 4.5, 0.0, TAU);
                    ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    let _ = ctx.fill_preserve();
                    ctx.set_source_rgba(0.0, 0.0, 0.0, 0.45);
                    ctx.set_line_width(1.2);
                    let _ = ctx.stroke();
                }

                // Etiqueta de dimensiones en píldora squircle
                let badge_text = format!("{} × {}", rw.round() as i32, rh.round() as i32);
                ctx.set_font_size(11.5);
                if let Ok(ext) = ctx.text_extents(&badge_text) {
                    let pad_x = 8.0;
                    let pad_y = 4.0;
                    let bw = ext.width() + pad_x * 2.0;
                    let bh = ext.height() + pad_y * 2.0;

                    let mut by = ry + rh + 8.0;
                    if by + bh > sh - 90.0 {
                        by = (ry - bh - 8.0).max(10.0);
                    }
                    let bx = (rx + (rw - bw) * 0.5).clamp(10.0, sw - bw - 10.0);

                    path_squircle_rect(ctx, bx, by, bw, bh, 6.0);
                    ctx.set_source_rgba(0.10, 0.10, 0.12, 0.88);
                    let _ = ctx.fill();

                    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.96);
                    ctx.move_to(bx + pad_x, by + bh - pad_y);
                    let _ = ctx.show_text(&badge_text);
                }
            }
        }
    }

    fn draw_toolbar(
        &self,
        ctx: &Context,
        mask_ctx: &Context,
        sw: f64,
        sh: f64,
        is_dark: bool,
    ) {
        let (x, y, w, h) = toolbar_geometry(sw, sh);
        let r = 14.0;

        // Máscara para Dual-Kawase blur con geometría squircle exacta
        path_squircle_rect(mask_ctx, x, y, w, h, r);
        mask_ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = mask_ctx.fill();

        // 1. Sombra suave superelíptica
        draw_soft_shadow(ctx, x, y, w, h, r, is_dark);

        // 2. Fondo de cristal translúcido (frosted glass orgánico)
        path_squircle_rect(ctx, x, y, w, h, r);
        if is_dark {
            ctx.set_source_rgba(0.16, 0.17, 0.20, 0.68);
        } else {
            ctx.set_source_rgba(0.96, 0.96, 0.98, 0.62);
        }
        let _ = ctx.fill_preserve();

        // Borde exterior sutil
        ctx.set_line_width(1.0);
        if is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.16);
        } else {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.45);
        }
        let _ = ctx.stroke();

        // Brillo superior tenue (specular rim sutil)
        path_squircle_rect(ctx, x + 0.5, y + 0.5, w - 1.0, h - 1.0, r - 0.5);
        let rim = LinearGradient::new(0.0, y, 0.0, y + 14.0);
        rim.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if is_dark { 0.18 } else { 0.35 });
        rim.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
        let _ = ctx.set_source(&rim);
        ctx.set_line_width(1.0);
        let _ = ctx.stroke();

        let fg = if is_dark { [0.96, 0.96, 0.98] } else { [0.12, 0.12, 0.14] };

        // Botón Cerrar (✕ dentro de círculo)
        draw_close_button(
            ctx,
            x + 22.0,
            y + 22.0,
            self.hovered_item == Some(0),
            fg,
            is_dark,
        );

        // Separador 1
        draw_v_sep(ctx, x + 44.0, y + 8.0, 28.0, is_dark);

        // Modos de Foto: Pantalla, Ventana, Área
        let btn1_act = self.mode == ScreenshotUiMode::CaptureFull;
        let btn2_act = self.mode == ScreenshotUiMode::CaptureWindow;
        let btn3_act = self.mode == ScreenshotUiMode::CaptureArea;

        draw_mode_button_plate(ctx, x + 66.0, y + 6.0, btn1_act, self.hovered_item == Some(1), is_dark);
        draw_monitor_icon(ctx, x + 66.0, y + 22.0, fg, false);

        draw_mode_button_plate(ctx, x + 110.0, y + 6.0, btn2_act, self.hovered_item == Some(2), is_dark);
        draw_window_icon(ctx, x + 110.0, y + 22.0, fg);

        draw_mode_button_plate(ctx, x + 154.0, y + 6.0, btn3_act, self.hovered_item == Some(3), is_dark);
        draw_marquee_icon(ctx, x + 154.0, y + 22.0, fg, false);

        // Separador 2
        draw_v_sep(ctx, x + 176.0, y + 8.0, 28.0, is_dark);

        // Modos de Video: Grabar Pantalla, Grabar Área
        let btn4_act = self.mode == ScreenshotUiMode::RecordFull;
        let btn5_act = self.mode == ScreenshotUiMode::RecordArea;

        draw_mode_button_plate(ctx, x + 199.0, y + 6.0, btn4_act, self.hovered_item == Some(4), is_dark);
        draw_monitor_icon(ctx, x + 199.0, y + 22.0, fg, true);

        draw_mode_button_plate(ctx, x + 245.0, y + 6.0, btn5_act, self.hovered_item == Some(5), is_dark);
        draw_marquee_icon(ctx, x + 245.0, y + 22.0, fg, true);

        // Separador 3
        draw_v_sep(ctx, x + 268.0, y + 8.0, 28.0, is_dark);

        // Botón Opciones ▾
        let opt_hov = self.hovered_item == Some(6);
        draw_options_button(
            ctx,
            x + 274.0,
            y + 6.0,
            96.0,
            32.0,
            self.show_options_menu,
            opt_hov,
            fg,
            is_dark,
        );

        // Separador 4
        draw_v_sep(ctx, x + 376.0, y + 8.0, 28.0, is_dark);

        // Botón de Acción Principal ("Capturar" o "Grabar")
        let act_label = if self.mode.is_video() { "Grabar" } else { "Capturar" };
        let act_hov = self.hovered_item == Some(7);
        draw_action_button(
            ctx,
            x + 382.0,
            y + 6.0,
            94.0,
            32.0,
            act_label,
            act_hov,
            fg,
            is_dark,
        );
    }

    fn draw_options_menu(
        &self,
        ctx: &Context,
        mask_ctx: &Context,
        sw: f64,
        sh: f64,
        is_dark: bool,
    ) {
        let (x, y, w, h) = options_menu_geometry(sw, sh);
        let r = 12.0;

        path_squircle_rect(mask_ctx, x, y, w, h, r);
        mask_ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = mask_ctx.fill();

        draw_soft_shadow(ctx, x, y, w, h, r, is_dark);

        path_squircle_rect(ctx, x, y, w, h, r);
        if is_dark {
            ctx.set_source_rgba(0.16, 0.17, 0.20, 0.88);
        } else {
            ctx.set_source_rgba(0.96, 0.96, 0.98, 0.88);
        }
        let _ = ctx.fill_preserve();
        ctx.set_line_width(1.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.18 } else { 0.45 });
        let _ = ctx.stroke();

        let fg = if is_dark { [0.96, 0.96, 0.98] } else { [0.12, 0.12, 0.14] };

        // Cabecera: GUARDAR EN
        draw_menu_header(ctx, x + 28.0, y + 19.0, "Guardar en", fg);

        // Items de destino
        let items_dest = [
            (SaveTarget::Desktop, "Escritorio"),
            (SaveTarget::Documents, "Documentos"),
            (SaveTarget::Clipboard, "Portapapeles"),
            (SaveTarget::Screenshots, "Carpeta Capturas"),
        ];

        for (i, (tgt, label)) in items_dest.iter().enumerate() {
            let iy = y + 24.0 + i as f64 * 20.0;
            let checked = self.options.save_to == *tgt;
            let hovered = self.hovered_menu_item == Some(i);
            draw_menu_row(ctx, x + 5.0, iy, w - 10.0, 20.0, checked, label, hovered, fg);
        }

        // Separador 1
        draw_menu_h_sep(ctx, x + 12.0, y + 108.0, w - 24.0, fg);

        // Cabecera: TEMPORIZADOR
        draw_menu_header(ctx, x + 28.0, y + 125.0, "Temporizador", fg);

        let timers = [(0, "Ninguno"), (5, "5 segundos"), (10, "10 segundos")];
        for (i, (sec, label)) in timers.iter().enumerate() {
            let item_idx = 4 + i;
            let iy = y + 130.0 + i as f64 * 20.0;
            let checked = self.options.timer_seconds == *sec;
            let hovered = self.hovered_menu_item == Some(item_idx);
            draw_menu_row(ctx, x + 5.0, iy, w - 10.0, 20.0, checked, label, hovered, fg);
        }

        // Separador 2
        draw_menu_h_sep(ctx, x + 12.0, y + 194.0, w - 24.0, fg);

        // Cabecera: OPCIONES
        draw_menu_header(ctx, x + 28.0, y + 211.0, "Opciones", fg);

        let opts = [
            (7, self.options.remember_selection, "Recordar selección"),
            (8, self.options.include_audio, "Grabar micrófono"),
        ];
        for (i, (item_idx, checked, label)) in opts.iter().enumerate() {
            let iy = y + 216.0 + i as f64 * 20.0;
            let hovered = self.hovered_menu_item == Some(*item_idx);
            draw_menu_row(ctx, x + 5.0, iy, w - 10.0, 20.0, *checked, label, hovered, fg);
        }
    }

    fn draw_recording_indicator(
        &self,
        ctx: &Context,
        mask_ctx: &Context,
        sw: f64,
        is_dark: bool,
    ) {
        let (rx, ry, rw, rh) = recording_indicator_geometry(sw);
        let r = 14.0;

        path_squircle_rect(mask_ctx, rx, ry, rw, rh, r);
        mask_ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = mask_ctx.fill();

        draw_soft_shadow(ctx, rx, ry, rw, rh, r, is_dark);

        path_squircle_rect(ctx, rx, ry, rw, rh, r);
        ctx.set_source_rgba(0.12, 0.12, 0.14, 0.90);
        let _ = ctx.fill_preserve();
        ctx.set_line_width(1.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, 0.20);
        let _ = ctx.stroke();

        // Punto rojo parpadeante
        let pulse = Instant::now().elapsed().as_millis() % 1000 < 500;
        ctx.arc(rx + 15.0, ry + 16.0, 4.5, 0.0, TAU);
        if pulse {
            ctx.set_source_rgba(0.96, 0.24, 0.22, 1.0);
        } else {
            ctx.set_source_rgba(0.96, 0.24, 0.22, 0.45);
        }
        let _ = ctx.fill();

        // Tiempo transcurrido
        let secs = self.recording_start.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let timer_str = format!("{:02}:{:02}", secs / 60, secs % 60);
        ctx.set_font_size(11.5);
        ctx.set_source_rgba(1.0, 1.0, 1.0, 0.96);
        ctx.move_to(rx + 26.0, ry + 20.0);
        let _ = ctx.show_text(&timer_str);

        // Botón Detener
        let stop_x = rx + 72.0;
        let stop_y = ry + 5.0;
        let stop_w = 48.0;
        let stop_h = 22.0;
        path_squircle_rect(ctx, stop_x, stop_y, stop_w, stop_h, 6.0);
        if self.hovered_item == Some(8) {
            ctx.set_source_rgba(0.96, 0.28, 0.26, 0.95);
        } else {
            ctx.set_source_rgba(0.88, 0.22, 0.20, 0.88);
        }
        let _ = ctx.fill();
        ctx.set_source_rgba(1.0, 1.0, 1.0, 0.98);
        ctx.set_font_size(10.5);
        ctx.move_to(stop_x + 7.0, stop_y + 15.0);
        let _ = ctx.show_text("⏹ Parar");
    }
}

fn toolbar_geometry(sw: f64, sh: f64) -> (f64, f64, f64, f64) {
    let w = 484.0;
    let h = 44.0;
    let x = ((sw - w) * 0.5).round();
    let y = (sh - h - 80.0).round().max(10.0);
    (x, y, w, h)
}

fn options_menu_geometry(sw: f64, sh: f64) -> (f64, f64, f64, f64) {
    let (tb_x, tb_y, _, _) = toolbar_geometry(sw, sh);
    let mw = 224.0;
    let mh = 262.0;
    let mx = (tb_x + 268.0 + (108.0 - mw) * 0.5).clamp(10.0, sw - mw - 10.0);
    let my = tb_y - mh - 8.0;
    (mx, my, mw, mh)
}

fn recording_indicator_geometry(sw: f64) -> (f64, f64, f64, f64) {
    let rw = 126.0;
    let rh = 32.0;
    let rx = sw - rw - 24.0;
    let ry = 26.0;
    (rx, ry, rw, rh)
}

fn hit_test_options_menu(rel_y: f64) -> Option<usize> {
    if rel_y >= 24.0 && rel_y < 44.0 {
        Some(0) // Escritorio
    } else if rel_y >= 44.0 && rel_y < 64.0 {
        Some(1) // Documentos
    } else if rel_y >= 64.0 && rel_y < 84.0 {
        Some(2) // Portapapeles
    } else if rel_y >= 84.0 && rel_y < 104.0 {
        Some(3) // Capturas
    } else if rel_y >= 130.0 && rel_y < 150.0 {
        Some(4) // 0s
    } else if rel_y >= 150.0 && rel_y < 170.0 {
        Some(5) // 5s
    } else if rel_y >= 170.0 && rel_y < 190.0 {
        Some(6) // 10s
    } else if rel_y >= 216.0 && rel_y < 236.0 {
        Some(7) // Recordar selección
    } else if rel_y >= 236.0 && rel_y < 256.0 {
        Some(8) // Micrófono
    } else {
        None
    }
}

/// Dibuja la curva squircle (superelipse n=4.2) idéntica a la fórmula del Dock de Malus DE
pub fn path_squircle_rect(ctx: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.0 {
        ctx.rectangle(x, y, w, h);
        return;
    }
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter_turn = FRAC_PI_2;
    let segments = 24usize;

    ctx.new_sub_path();
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);

    // Esquina superior derecha: desde (x + w - r, y) hasta (x + w, y + r)
    for step in 1..=segments {
        let angle = quarter_turn * (step as f64 / segments as f64);
        ctx.line_to(
            x + w - r + r * angle.sin().powf(power),
            y + r - r * angle.cos().powf(power),
        );
    }
    ctx.line_to(x + w, y + h - r);

    // Esquina inferior derecha: desde (x + w, y + h - r) hasta (x + w - r, y + h)
    for step in 1..=segments {
        let angle = quarter_turn * (step as f64 / segments as f64);
        ctx.line_to(
            x + w - r + r * angle.cos().powf(power),
            y + h - r + r * angle.sin().powf(power),
        );
    }
    ctx.line_to(x + r, y + h);

    // Esquina inferior izquierda: desde (x + r, y + h) hasta (x, y + h - r)
    for step in 1..=segments {
        let angle = quarter_turn * (step as f64 / segments as f64);
        ctx.line_to(
            x + r - r * angle.sin().powf(power),
            y + h - r + r * angle.cos().powf(power),
        );
    }
    ctx.line_to(x, y + r);

    // Esquina superior izquierda: desde (x, y + r) hasta (x + r, y)
    for step in 1..=segments {
        let angle = quarter_turn * (step as f64 / segments as f64);
        ctx.line_to(
            x + r - r * angle.cos().powf(power),
            y + r - r * angle.sin().powf(power),
        );
    }

    ctx.close_path();
}

fn draw_soft_shadow(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    is_dark: bool,
) {
    let alpha = if is_dark { 0.04 } else { 0.025 };
    for i in 1..=4 {
        let spread = i as f64 * 2.5;
        path_squircle_rect(
            ctx,
            x - spread,
            y - spread * 0.5 + i as f64 * 2.0,
            w + spread * 2.0,
            h + spread * 2.0,
            r + spread * 0.5,
        );
        ctx.set_source_rgba(0.0, 0.0, 0.0, alpha);
        let _ = ctx.fill();
    }
}

fn draw_close_button(
    ctx: &Context,
    cx: f64,
    cy: f64,
    is_hover: bool,
    fg: [f64; 3],
    is_dark: bool,
) {
    let r = 8.5;
    ctx.arc(cx, cy, r, 0.0, TAU);
    if is_hover {
        if is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.22);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.14);
        }
        let _ = ctx.fill_preserve();
    } else {
        if is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.08);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.06);
        }
        let _ = ctx.fill_preserve();
    }
    ctx.set_line_width(0.9);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], if is_dark { 0.20 } else { 0.12 });
    let _ = ctx.stroke();

    // ✕
    ctx.set_line_width(1.3);
    ctx.set_line_cap(cairo::LineCap::Round);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.85);
    ctx.move_to(cx - 2.8, cy - 2.8);
    ctx.line_to(cx + 2.8, cy + 2.8);
    ctx.move_to(cx + 2.8, cy - 2.8);
    ctx.line_to(cx - 2.8, cy + 2.8);
    let _ = ctx.stroke();
}

fn draw_v_sep(ctx: &Context, x: f64, y: f64, h: f64, is_dark: bool) {
    ctx.set_line_width(1.0);
    ctx.set_source_rgba(
        if is_dark { 1.0 } else { 0.0 },
        if is_dark { 1.0 } else { 0.0 },
        if is_dark { 1.0 } else { 0.0 },
        0.10,
    );
    ctx.move_to(x, y);
    ctx.line_to(x, y + h);
    let _ = ctx.stroke();
}

fn draw_mode_button_plate(
    ctx: &Context,
    cx: f64,
    y: f64,
    is_active: bool,
    is_hover: bool,
    is_dark: bool,
) {
    let w = 32.0;
    let h = 32.0;
    let x = cx - w * 0.5;
    if is_active {
        path_squircle_rect(ctx, x, y, w, h, 6.0);
        if is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.20);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.13);
        }
        let _ = ctx.fill();
    } else if is_hover {
        path_squircle_rect(ctx, x, y, w, h, 6.0);
        if is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.09);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.06);
        }
        let _ = ctx.fill();
    }
}

fn draw_monitor_icon(ctx: &Context, cx: f64, cy: f64, fg: [f64; 3], with_rec_dot: bool) {
    ctx.set_line_width(1.3);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.90);
    // Pantalla
    path_squircle_rect(ctx, cx - 10.0, cy - 8.0, 20.0, 13.0, 2.5);
    let _ = ctx.stroke();
    // Base/Pie
    ctx.move_to(cx - 3.5, cy + 8.0);
    ctx.line_to(cx + 3.5, cy + 8.0);
    ctx.move_to(cx, cy + 5.0);
    ctx.line_to(cx, cy + 8.0);
    let _ = ctx.stroke();

    if with_rec_dot {
        let dot_x = cx + 6.0;
        let dot_y = cy + 2.5;
        ctx.arc(dot_x, dot_y, 3.5, 0.0, TAU);
        ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.95);
        let _ = ctx.fill();
    }
}

fn draw_window_icon(ctx: &Context, cx: f64, cy: f64, fg: [f64; 3]) {
    ctx.set_line_width(1.3);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.90);
    path_squircle_rect(ctx, cx - 10.0, cy - 8.0, 20.0, 16.0, 2.5);
    let _ = ctx.stroke();
    // Línea de cabecera
    ctx.move_to(cx - 10.0, cy - 3.5);
    ctx.line_to(cx + 10.0, cy - 3.5);
    let _ = ctx.stroke();
    // 3 puntos
    for i in 0..3 {
        ctx.arc(cx - 6.0 + i as f64 * 3.0, cy - 5.5, 0.7, 0.0, TAU);
        let _ = ctx.fill();
    }
}

fn draw_marquee_icon(ctx: &Context, cx: f64, cy: f64, fg: [f64; 3], with_rec_dot: bool) {
    ctx.set_line_width(1.3);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.90);
    ctx.set_dash(&[2.5, 2.0], 0.0);
    path_squircle_rect(ctx, cx - 9.0, cy - 8.0, 18.0, 16.0, 2.5);
    let _ = ctx.stroke();
    ctx.set_dash(&[], 0.0);

    if with_rec_dot {
        let dot_x = cx + 6.0;
        let dot_y = cy + 4.5;
        ctx.arc(dot_x, dot_y, 3.5, 0.0, TAU);
        ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.95);
        let _ = ctx.fill();
    }
}

fn draw_options_button(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    is_active: bool,
    is_hover: bool,
    fg: [f64; 3],
    is_dark: bool,
) {
    if is_active {
        path_squircle_rect(ctx, x, y, w, h, 6.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.20 } else { 0.40 });
        let _ = ctx.fill();
    } else if is_hover {
        path_squircle_rect(ctx, x, y, w, h, 6.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.10 } else { 0.22 });
        let _ = ctx.fill();
    }

    ctx.set_font_size(12.5);
    let label = "Opciones";
    if let Ok(ext) = ctx.text_extents(label) {
        let total_w = ext.width() + 5.0 + 6.0;
        let start_x = x + (w - total_w) * 0.5;
        let text_y = y + h * 0.5 + ext.height() * 0.5 - 1.0;

        ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.90);
        ctx.move_to(start_x, text_y);
        let _ = ctx.show_text(label);

        // Chevron ⌵
        let ch_cx = start_x + ext.width() + 5.0 + 3.0;
        let ch_cy = y + h * 0.5;
        ctx.set_line_width(1.3);
        ctx.set_line_cap(cairo::LineCap::Round);
        ctx.set_line_join(cairo::LineJoin::Round);
        ctx.move_to(ch_cx - 3.0, ch_cy - 1.5);
        ctx.line_to(ch_cx, ch_cy + 1.5);
        ctx.line_to(ch_cx + 3.0, ch_cy - 1.5);
        let _ = ctx.stroke();
    }
}

fn draw_action_button(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    label: &str,
    is_hover: bool,
    fg: [f64; 3],
    is_dark: bool,
) {
    if is_hover {
        path_squircle_rect(ctx, x, y, w, h, 6.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.12 } else { 0.28 });
        let _ = ctx.fill();
    }

    ctx.set_font_size(12.5);
    if let Ok(ext) = ctx.text_extents(label) {
        let text_x = x + (w - ext.width()) * 0.5;
        let text_y = y + h * 0.5 + ext.height() * 0.5 - 1.0;
        ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.95);
        ctx.move_to(text_x, text_y);
        let _ = ctx.show_text(label);
    }
}

fn draw_menu_header(ctx: &Context, x: f64, y: f64, text: &str, fg: [f64; 3]) {
    ctx.set_font_size(10.5);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.45);
    ctx.move_to(x, y);
    let _ = ctx.show_text(text);
}

fn draw_menu_h_sep(ctx: &Context, x: f64, y: f64, w: f64, fg: [f64; 3]) {
    ctx.set_line_width(1.0);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.10);
    ctx.move_to(x, y);
    ctx.line_to(x + w, y);
    let _ = ctx.stroke();
}

fn draw_menu_row(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    checked: bool,
    label: &str,
    is_hover: bool,
    fg: [f64; 3],
) {
    let item_fg = if is_hover {
        path_squircle_rect(ctx, x, y, w, h, 5.0);
        ctx.set_source_rgba(0.05, 0.48, 0.98, 0.95);
        let _ = ctx.fill();
        [1.0, 1.0, 1.0]
    } else {
        fg
    };

    if checked {
        draw_checkmark(ctx, x + 10.0, y + h * 0.5, item_fg);
    }

    ctx.set_font_size(12.0);
    ctx.set_source_rgba(item_fg[0], item_fg[1], item_fg[2], 0.95);
    ctx.move_to(x + 23.0, y + h * 0.5 + 4.0);
    let _ = ctx.show_text(label);
}

fn draw_checkmark(ctx: &Context, cx: f64, cy: f64, fg: [f64; 3]) {
    ctx.set_line_width(1.5);
    ctx.set_line_cap(cairo::LineCap::Round);
    ctx.set_line_join(cairo::LineJoin::Round);
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.95);
    ctx.move_to(cx - 3.5, cy);
    ctx.line_to(cx - 1.0, cy + 3.0);
    ctx.line_to(cx + 4.5, cy - 3.5);
    let _ = ctx.stroke();
}
