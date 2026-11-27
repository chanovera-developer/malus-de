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
pub const TOOLBAR_WIDTH: f64 = 544.0;
pub const TOOLBAR_HEIGHT: f64 = 44.0;
pub const TOOLBAR_RADIUS: f64 = 22.0; // Cápsula squircle a altura de 44 px (44 / 2 = 22 px)
pub const TOOLBAR_PADDING: f64 = 6.0;
pub const ACTION_BTN_RADIUS: f64 = TOOLBAR_RADIUS - TOOLBAR_PADDING; // 16.0 px concéntrico al contorno de 22 px
const TOOLBAR_GRIP_WIDTH: f64 = 20.0;
const MIN_SELECTION_SIZE: f64 = 10.0;
const MIN_RESIZE_HANDLE_SIZE: f64 = 24.0;

fn clamp_selection_to_bounds(area: SelectionArea, sw: f64, sh: f64) -> SelectionArea {
    let sw = if sw.is_finite() && sw > 0.0 { sw } else { 1.0 };
    let sh = if sh.is_finite() && sh > 0.0 { sh } else { 1.0 };
    let min_w = MIN_SELECTION_SIZE.min(sw);
    let min_h = MIN_SELECTION_SIZE.min(sh);

    let raw_x = if area.x.is_finite() { area.x } else { 0.0 };
    let raw_y = if area.y.is_finite() { area.y } else { 0.0 };
    let raw_w = if area.width.is_finite() {
        area.width
    } else {
        min_w
    };
    let raw_h = if area.height.is_finite() {
        area.height
    } else {
        min_h
    };

    let max_x = (sw - min_w).max(0.0);
    let max_y = (sh - min_h).max(0.0);
    let x = raw_x.clamp(0.0, max_x);
    let y = raw_y.clamp(0.0, max_y);
    let max_w = (sw - x).max(min_w);
    let max_h = (sh - y).max(min_h);
    let width = raw_w.clamp(min_w, max_w);
    let height = raw_h.clamp(min_h, max_h);

    SelectionArea {
        x,
        y,
        width,
        height,
    }
}

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
    OcrArea,
    RecordFull,
    RecordArea,
}

impl ScreenshotUiMode {
    pub fn is_video(&self) -> bool {
        matches!(self, Self::RecordFull | Self::RecordArea)
    }

    pub fn is_area(&self) -> bool {
        matches!(self, Self::CaptureArea | Self::RecordArea | Self::OcrArea)
    }

    pub fn is_ocr(&self) -> bool {
        matches!(self, Self::OcrArea)
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
    pub lossless: bool,
    pub transparent_window_bg: bool,
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
    MoveToolbar,
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
    pub hovered_window_rect: Option<Rectangle<i32, Logical>>,
    pub is_recording: bool,
    pub recording_start: Option<Instant>,
    pub recording_dest: Option<PathBuf>,

    // Posición arrastrable de la barra flotante (coordenadas lógicas relativas al monitor)
    toolbar_pos: Option<(f64, f64)>,

    // Estado de interacción
    active_handle: HandleType,
    drag_start_pointer: (f64, f64),
    drag_start_area: SelectionArea,
    drag_start_toolbar: (f64, f64),
    hovered_item: Option<u8>, // 0: Cerrar, 1..5: Modos, 6: Opciones, 7: Acción, 8: Sin pérdidas, 9: Agarre barra, 10: OCR
    hovered_menu_item: Option<usize>, // 0..10 para el menú emergente

    // Buffers de renderizado del compositor
    render_buffer: Option<MemoryRenderBuffer>,
    render_buffer_size: (i32, i32),
    mask_buffer: Vec<u8>,
    last_render_signature: u64,
    last_render_rect: Rectangle<i32, Physical>,
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
                lossless: false,
                transparent_window_bg: true,
            },
            show_options_menu: false,
            target_output: None,
            selection: SelectionArea {
                x: 200.0,
                y: 180.0,
                width: 800.0,
                height: 520.0,
            },
            hovered_window_rect: None,
            is_recording: false,
            recording_start: None,
            recording_dest: None,
            toolbar_pos: None,
            active_handle: HandleType::None,
            drag_start_pointer: (0.0, 0.0),
            drag_start_area: SelectionArea {
                x: 200.0,
                y: 180.0,
                width: 800.0,
                height: 520.0,
            },
            drag_start_toolbar: (0.0, 0.0),
            hovered_item: None,
            hovered_menu_item: None,
            render_buffer: None,
            render_buffer_size: (0, 0),
            mask_buffer: Vec::new(),
            last_render_signature: 0,
            last_render_rect: Rectangle::default(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn open_with_mode(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        output_name: String,
        mode: ScreenshotUiMode,
    ) {
        self.mode = mode;
        self.open(output_geo, output_name);
    }

    pub fn toolbar_rect(&self, sw: f64, sh: f64) -> (f64, f64, f64, f64) {
        let (def_x, def_y, w, h) = toolbar_geometry(sw, sh);
        if let Some((px, py)) = self.toolbar_pos {
            let max_x = (sw - w - 10.0).max(10.0);
            let max_y = (sh - h - 10.0).max(40.0);
            let min_y = 40.0_f64.min(max_y);
            let cx = if px.is_finite() {
                px.clamp(10.0, max_x).round()
            } else {
                def_x
            };
            let cy = if py.is_finite() {
                py.clamp(min_y, max_y).round()
            } else {
                def_y
            };
            (cx, cy, w, h)
        } else {
            (def_x, def_y, w, h)
        }
    }

    fn lossless_pill_rect(&self, sw: f64, sh: f64) -> (f64, f64, f64, f64) {
        let (tb_x, tb_y, tb_w, tb_h) = self.toolbar_rect(sw, sh);
        let pw = 128.0;
        let ph = tb_h;
        let right_x = tb_x + tb_w + 10.0;
        if right_x + pw <= sw - 10.0 {
            (right_x, tb_y, pw, ph)
        } else if tb_x - pw - 10.0 >= 10.0 {
            (tb_x - pw - 10.0, tb_y, pw, ph)
        } else {
            let cx = (tb_x + (tb_w - pw) * 0.5).clamp(10.0, (sw - pw - 10.0).max(10.0));
            let above_y = tb_y - ph - 8.0;
            let py = if above_y >= 40.0 {
                above_y
            } else {
                (tb_y + tb_h + 8.0).min((sh - ph - 10.0).max(10.0))
            };
            (cx, py, pw, ph)
        }
    }

    fn options_menu_rect(&self, sw: f64, sh: f64) -> (f64, f64, f64, f64) {
        let (tb_x, tb_y, _, tb_h) = self.toolbar_rect(sw, sh);
        let mw = 248.0;
        let mh = 302.0;
        let max_mx = (sw - mw - 10.0).max(10.0);
        let mx = (tb_x + 326.0 + (108.0 - mw) * 0.5).clamp(10.0, max_mx);
        let above_y = tb_y - mh - 8.0;
        let my = if above_y >= 40.0 {
            above_y
        } else {
            (tb_y + tb_h + 8.0).min((sh - mh - 10.0).max(10.0))
        };
        (mx, my, mw, mh)
    }

    pub fn open(&mut self, output_geo: Rectangle<i32, Logical>, output_name: String) {
        self.is_open = true;
        self.target_output = Some(output_name);
        self.show_options_menu = false;
        self.hovered_menu_item = None;
        self.hovered_window_rect = None;

        let sw = (output_geo.size.w as f64).max(1.0);
        let sh = (output_geo.size.h as f64).max(1.0);
        let min_w = MIN_SELECTION_SIZE.min(sw);
        let min_h = MIN_SELECTION_SIZE.min(sh);

        let selection_invalid_or_offscreen = !self.selection.x.is_finite()
            || !self.selection.y.is_finite()
            || !self.selection.width.is_finite()
            || !self.selection.height.is_finite()
            || self.selection.width <= min_w
            || self.selection.height <= min_h
            || self.selection.x >= sw - min_w
            || self.selection.y >= sh - min_h
            || self.selection.x + self.selection.width <= min_w
            || self.selection.y + self.selection.height <= min_h;

        if !self.options.remember_selection || selection_invalid_or_offscreen {
            let init_w = (sw * 0.55).min(960.0).max(320.0).min(sw);
            let init_h = (sh * 0.50).min(640.0).max(240.0).min(sh);
            self.selection = SelectionArea {
                x: ((sw - init_w) * 0.5).max(0.0).round(),
                y: ((sh - init_h) * 0.5 - 20.0)
                    .max(40.0)
                    .min((sh - init_h).max(0.0))
                    .round(),
                width: init_w.round(),
                height: init_h.round(),
            };
        }
        self.selection = clamp_selection_to_bounds(self.selection, sw, sh);
        info!("📸 Barra de captura squircle abierta en pantalla {}", sw);
    }

    pub fn clamped_selection(&self, output_geo: Rectangle<i32, Logical>) -> SelectionArea {
        clamp_selection_to_bounds(
            self.selection,
            output_geo.size.w as f64,
            output_geo.size.h as f64,
        )
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.show_options_menu = false;
        self.hovered_menu_item = None;
        self.hovered_window_rect = None;
        self.active_handle = HandleType::None;
        info!("📸 Barra de captura cerrada");
    }

    pub fn should_render_on_output(&self, output_name: &str) -> bool {
        self.is_open
            && self
                .target_output
                .as_deref()
                .map_or(true, |target| target == output_name)
    }

    pub fn update_hovered_window(&mut self, rect: Option<Rectangle<i32, Logical>>) {
        self.hovered_window_rect = rect;
    }

    pub fn update_pointer_motion(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        point: Point<f64, Logical>,
    ) {
        if !self.is_open {
            return;
        }

        let lx = point.x - output_geo.loc.x as f64;
        let ly = point.y - output_geo.loc.y as f64;
        let sw = output_geo.size.w as f64;
        let sh = output_geo.size.h as f64;
        if sw <= 0.0 || sh <= 0.0 {
            return;
        }

        // Si hay arrastre activo sobre la barra flotante o el área de selección
        if self.active_handle == HandleType::MoveToolbar {
            let offset_x = lx - self.drag_start_pointer.0;
            let offset_y = ly - self.drag_start_pointer.1;
            let max_x = (sw - TOOLBAR_WIDTH - 10.0).max(10.0);
            let max_y = (sh - TOOLBAR_HEIGHT - 10.0).max(40.0);
            let min_y = 40.0_f64.min(max_y);
            let nx = (self.drag_start_toolbar.0 + offset_x).clamp(10.0, max_x);
            let ny = (self.drag_start_toolbar.1 + offset_y).clamp(min_y, max_y);
            self.toolbar_pos = Some((nx.round(), ny.round()));
            return;
        }

        if self.active_handle != HandleType::None {
            let offset_x = lx - self.drag_start_pointer.0;
            let offset_y = ly - self.drag_start_pointer.1;
            let orig = clamp_selection_to_bounds(self.drag_start_area, sw, sh);
            let right = (orig.x + orig.width).clamp(0.0, sw);
            let bottom = (orig.y + orig.height).clamp(0.0, sh);
            let max_left_x = (right - MIN_RESIZE_HANDLE_SIZE).max(0.0);
            let max_top_y = (bottom - MIN_RESIZE_HANDLE_SIZE).max(0.0);
            let avail_w = (sw - orig.x).max(0.0);
            let avail_h = (sh - orig.y).max(0.0);
            let min_resize_w = MIN_RESIZE_HANDLE_SIZE.min(avail_w);
            let min_resize_h = MIN_RESIZE_HANDLE_SIZE.min(avail_h);

            match self.active_handle {
                HandleType::MoveBox => {
                    self.selection.x = (orig.x + offset_x).clamp(0.0, (sw - orig.width).max(0.0));
                    self.selection.y = (orig.y + offset_y).clamp(0.0, (sh - orig.height).max(0.0));
                }
                HandleType::NewSelection => {
                    let sx = self.drag_start_pointer.0.clamp(0.0, sw);
                    let sy = self.drag_start_pointer.1.clamp(0.0, sh);
                    let cur_x = lx.clamp(0.0, sw);
                    let cur_y = ly.clamp(0.0, sh);
                    self.selection = SelectionArea {
                        x: sx.min(cur_x),
                        y: sy.min(cur_y),
                        width: (sx - cur_x).abs().max(MIN_SELECTION_SIZE),
                        height: (sy - cur_y).abs().max(MIN_SELECTION_SIZE),
                    };
                }
                HandleType::NorthWest => {
                    let new_x = (orig.x + offset_x).clamp(0.0, max_left_x);
                    let new_y = (orig.y + offset_y).clamp(0.0, max_top_y);
                    self.selection.x = new_x;
                    self.selection.y = new_y;
                    self.selection.width = (right - new_x).max(MIN_SELECTION_SIZE.min(sw));
                    self.selection.height = (bottom - new_y).max(MIN_SELECTION_SIZE.min(sh));
                }
                HandleType::North => {
                    let new_y = (orig.y + offset_y).clamp(0.0, max_top_y);
                    self.selection.y = new_y;
                    self.selection.height = (bottom - new_y).max(MIN_SELECTION_SIZE.min(sh));
                }
                HandleType::NorthEast => {
                    let new_y = (orig.y + offset_y).clamp(0.0, max_top_y);
                    let new_w = (orig.width + offset_x).clamp(min_resize_w, avail_w);
                    self.selection.y = new_y;
                    self.selection.height = (bottom - new_y).max(MIN_SELECTION_SIZE.min(sh));
                    self.selection.width = new_w;
                }
                HandleType::East => {
                    self.selection.width = (orig.width + offset_x).clamp(min_resize_w, avail_w);
                }
                HandleType::SouthEast => {
                    self.selection.width = (orig.width + offset_x).clamp(min_resize_w, avail_w);
                    self.selection.height = (orig.height + offset_y).clamp(min_resize_h, avail_h);
                }
                HandleType::South => {
                    self.selection.height = (orig.height + offset_y).clamp(min_resize_h, avail_h);
                }
                HandleType::SouthWest => {
                    let new_x = (orig.x + offset_x).clamp(0.0, max_left_x);
                    self.selection.x = new_x;
                    self.selection.width = (right - new_x).max(MIN_SELECTION_SIZE.min(sw));
                    self.selection.height = (orig.height + offset_y).clamp(min_resize_h, avail_h);
                }
                HandleType::West => {
                    let new_x = (orig.x + offset_x).clamp(0.0, max_left_x);
                    self.selection.x = new_x;
                    self.selection.width = (right - new_x).max(MIN_SELECTION_SIZE.min(sw));
                }
                HandleType::MoveToolbar | HandleType::None => {}
            }
            self.selection = clamp_selection_to_bounds(self.selection, sw, sh);
            return;
        }

        // Comprobar menú opciones si está abierto
        if self.show_options_menu {
            let (menu_x, menu_y, menu_w, menu_h) = self.options_menu_rect(sw, sh);
            if lx >= menu_x && lx <= menu_x + menu_w && ly >= menu_y && ly <= menu_y + menu_h {
                self.hovered_menu_item = hit_test_options_menu(ly - menu_y);
                self.hovered_item = Some(6);
                return;
            } else {
                self.hovered_menu_item = None;
            }
        }

        // Detección de elementos interactivos de la barra flotante arrastrable (544 × 44, radio 22 px)
        let (tb_x, tb_y, tb_w, tb_h) = self.toolbar_rect(sw, sh);
        if lx >= tb_x && lx <= tb_x + tb_w && ly >= tb_y && ly <= tb_y + tb_h {
            let rel_x = lx - tb_x;
            if rel_x < TOOLBAR_GRIP_WIDTH {
                self.hovered_item = Some(9); // Agarre para mover la barra
            } else if rel_x < 58.0 {
                self.hovered_item = Some(0); // Cerrar
            } else if rel_x < 102.0 {
                self.hovered_item = Some(1); // Capturar Pantalla
            } else if rel_x < 146.0 {
                self.hovered_item = Some(2); // Capturar Ventana
            } else if rel_x < 190.0 {
                self.hovered_item = Some(3); // Capturar Área
            } else if rel_x < 234.0 {
                self.hovered_item = Some(10); // Extraer Texto (OCR)
            } else if rel_x < 280.0 {
                self.hovered_item = Some(4); // Grabar Pantalla
            } else if rel_x < 326.0 {
                self.hovered_item = Some(5); // Grabar Área
            } else if rel_x < 434.0 {
                self.hovered_item = Some(6); // Opciones ▾
            } else {
                self.hovered_item = Some(7); // Acción (Capturar / Grabar / Extraer)
            }
            return;
        }

        // Botón auxiliar flotante "Sin pérdidas" (solo visible en modos de grabación)
        if self.mode.is_video() {
            let (lx_p, ly_p, lw_p, lh_p) = self.lossless_pill_rect(sw, sh);
            if lx >= lx_p && lx <= lx_p + lw_p && ly >= ly_p && ly <= ly_p + lh_p {
                self.hovered_item = Some(8);
                return;
            }
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
        if !self.is_open {
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

        // Ignorar clics fuera del monitor donde está abierta la barra (p. ej. en otro monitor)
        if sw <= 0.0 || sh <= 0.0 || lx < 0.0 || lx > sw || ly < 0.0 || ly > sh {
            self.show_options_menu = false;
            self.hovered_menu_item = None;
            return ScreenshotPointerResult::Ignored;
        }

        // 1. Clic en menú desplegable de opciones (si está abierto)
        if self.show_options_menu {
            let (mx, my, mw, mh) = self.options_menu_rect(sw, sh);
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
                        9 => self.options.lossless = !self.options.lossless,
                        10 => {
                            self.options.transparent_window_bg = !self.options.transparent_window_bg
                        }
                        _ => {}
                    }
                }
                return ScreenshotPointerResult::Consumed;
            } else {
                self.show_options_menu = false;
                self.hovered_menu_item = None;
            }
        }

        // 2. Clic dentro de la barra de herramientas arrastrable (544 × 44, radio 22 px)
        let (tb_x, tb_y, tb_w, tb_h) = self.toolbar_rect(sw, sh);
        if lx >= tb_x && lx <= tb_x + tb_w && ly >= tb_y && ly <= tb_y + tb_h {
            let rel_x = lx - tb_x;
            if rel_x < TOOLBAR_GRIP_WIDTH {
                // Iniciar arrastre libre de la barra flotante de captura/grabación
                self.show_options_menu = false;
                self.hovered_menu_item = None;
                self.active_handle = HandleType::MoveToolbar;
                self.drag_start_pointer = (lx.clamp(0.0, sw), ly.clamp(0.0, sh));
                self.drag_start_toolbar = (tb_x, tb_y);
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 58.0 {
                self.close();
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 102.0 {
                self.mode = ScreenshotUiMode::CaptureFull;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 146.0 {
                self.mode = ScreenshotUiMode::CaptureWindow;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 190.0 {
                self.mode = ScreenshotUiMode::CaptureArea;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 234.0 {
                self.mode = ScreenshotUiMode::OcrArea;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 280.0 {
                self.mode = ScreenshotUiMode::RecordFull;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 326.0 {
                self.mode = ScreenshotUiMode::RecordArea;
                self.show_options_menu = false;
                return ScreenshotPointerResult::Consumed;
            } else if rel_x < 434.0 {
                self.show_options_menu = !self.show_options_menu;
                self.hovered_menu_item = None;
                return ScreenshotPointerResult::Consumed;
            } else {
                // Clic en botón "Capturar" / "Grabar" / "Extraer"
                self.show_options_menu = false;
                return ScreenshotPointerResult::TriggerAction;
            }
        }

        // 2b. Clic en el botón flotante "Sin pérdidas" (solo en modos de grabación)
        if self.mode.is_video() {
            let (lx_p, ly_p, lw_p, lh_p) = self.lossless_pill_rect(sw, sh);
            if lx >= lx_p && lx <= lx_p + lw_p && ly >= ly_p && ly <= ly_p + lh_p {
                self.options.lossless = !self.options.lossless;
                return ScreenshotPointerResult::Consumed;
            }
        }

        // En modo Capturar Ventana, un clic fuera de la barra captura la ventana bajo el puntero
        if self.mode == ScreenshotUiMode::CaptureWindow {
            self.show_options_menu = false;
            return ScreenshotPointerResult::TriggerAction;
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
                    self.selection = clamp_selection_to_bounds(
                        SelectionArea {
                            x: lx,
                            y: ly,
                            width: MIN_SELECTION_SIZE,
                            height: MIN_SELECTION_SIZE,
                        },
                        sw,
                        sh,
                    );
                }
            }

            self.active_handle = matched;
            self.drag_start_pointer = (lx.clamp(0.0, sw), ly.clamp(0.0, sh));
            self.drag_start_area = clamp_selection_to_bounds(self.selection, sw, sh);
            return ScreenshotPointerResult::Consumed;
        }

        ScreenshotPointerResult::Ignored
    }

    pub fn handle_key_press(&mut self, sym: Keysym, _modifiers: ModifiersState) -> bool {
        if !self.is_open {
            return false;
        }

        if sym == keysyms::KEY_Escape.into() {
            self.close();
            return false;
        }

        if sym == keysyms::KEY_Return.into() || sym == keysyms::KEY_KP_Enter.into() {
            return true; // Dispara acción
        }

        // Espacio alterna entre Capturar Área y Capturar Ventana aislada (igual que macOS)
        if sym == keysyms::KEY_space.into() {
            match self.mode {
                ScreenshotUiMode::CaptureArea => {
                    self.mode = ScreenshotUiMode::CaptureWindow;
                    self.show_options_menu = false;
                }
                ScreenshotUiMode::CaptureWindow => {
                    self.mode = ScreenshotUiMode::CaptureArea;
                    self.show_options_menu = false;
                }
                _ => {}
            }
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
        if !self.is_open {
            return None;
        }

        let sw = output_geo.size.w as f64;
        let sh = output_geo.size.h as f64;
        let phys_w = (sw * scale).round() as i32;
        let phys_h = (sh * scale).round() as i32;

        if phys_w <= 0 || phys_h <= 0 {
            return None;
        }

        let phys_loc = Point::from((0, 0));
        let glass_scale = malus_de_common::config::current_liquid_glass_scale();
        let accent_idx = malus_de_common::config::load_accent_color_index() as u64;

        let mut sig = 1469598103934665603u64;
        let mix = |s: &mut u64, v: u64| {
            *s ^= v;
            *s = s.wrapping_mul(1099511628211);
        };
        mix(&mut sig, u64::from(is_dark));
        mix(&mut sig, accent_idx);
        mix(&mut sig, (scale * 1000.0).round() as u64);
        mix(&mut sig, (glass_scale * 1000.0).round() as u64);
        mix(&mut sig, phys_w as u64);
        mix(&mut sig, phys_h as u64);
        mix(&mut sig, self.hovered_item.unwrap_or(99) as u64);
        mix(&mut sig, self.mode as u64);
        mix(&mut sig, u64::from(self.show_options_menu));
        mix(&mut sig, self.hovered_menu_item.unwrap_or(999) as u64);
        mix(&mut sig, self.options.save_to as u64);
        mix(&mut sig, u64::from(self.options.timer_seconds));
        mix(&mut sig, u64::from(self.options.remember_selection));
        mix(&mut sig, u64::from(self.options.include_audio));
        mix(&mut sig, u64::from(self.options.lossless));
        mix(&mut sig, u64::from(self.options.transparent_window_bg));
        let (tb_x, tb_y, _, _) = self.toolbar_rect(sw, sh);
        mix(&mut sig, (tb_x * 2.0).round() as i64 as u64);
        mix(&mut sig, (tb_y * 2.0).round() as i64 as u64);
        mix(&mut sig, (self.selection.x * 2.0).round() as i64 as u64);
        mix(&mut sig, (self.selection.y * 2.0).round() as i64 as u64);
        mix(&mut sig, (self.selection.width * 2.0).round() as i64 as u64);
        mix(&mut sig, (self.selection.height * 2.0).round() as i64 as u64);
        if let Some(wr) = self.hovered_window_rect {
            mix(&mut sig, 1);
            mix(&mut sig, wr.loc.x as i64 as u64);
            mix(&mut sig, wr.loc.y as i64 as u64);
            mix(&mut sig, wr.size.w as i64 as u64);
            mix(&mut sig, wr.size.h as i64 as u64);
        } else {
            mix(&mut sig, 0);
        }

        let rect = Rectangle::new(phys_loc, (phys_w, phys_h).into());
        if sig == self.last_render_signature
            && self.render_buffer.is_some()
            && self.render_buffer_size == (phys_w, phys_h)
            && self.last_render_rect == rect
            && !self.mask_buffer.is_empty()
        {
            return Some((rect, Some(&self.mask_buffer), (phys_w, phys_h), 1.0));
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

            // 1. Fondo oscurecido con recorte nítido para selección o ventana
            self.draw_scrim_and_selection(&ctx, sw, sh);

            // 2. Barra de herramientas flotante squircle (border-radius 22 px)
            self.draw_toolbar(&ctx, &mask_ctx, sw, sh, is_dark);

            // 2b. Botón auxiliar "Sin pérdidas" en modo de grabación de video
            if self.mode.is_video() {
                self.draw_lossless_pill(&ctx, &mask_ctx, sw, sh, is_dark);
            }

            // 3. Menú de opciones emergente squircle si está abierto
            if self.show_options_menu {
                self.draw_options_menu(&ctx, &mask_ctx, sw, sh, is_dark);
            }
        }

        surface.flush();
        mask_surface.flush();

        if let Ok(mask_data) = mask_surface.data() {
            self.mask_buffer.clear();
            self.mask_buffer.extend_from_slice(&mask_data);
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

        self.last_render_signature = sig;
        self.last_render_rect = rect;
        Some((rect, Some(&self.mask_buffer), (phys_w, phys_h), 1.0))
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
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.28);
                let _ = ctx.paint();
                if let Some(wr) = self.hovered_window_rect {
                    let min_w = 16.0_f64.min(sw).max(1.0);
                    let min_h = 16.0_f64.min(sh).max(1.0);
                    let rx = (wr.loc.x as f64).clamp(0.0, (sw - min_w).max(0.0));
                    let ry = (wr.loc.y as f64).clamp(0.0, (sh - min_h).max(0.0));
                    let rw = (wr.size.w as f64).clamp(min_w, (sw - rx).max(min_w));
                    let rh = (wr.size.h as f64).clamp(min_h, (sh - ry).max(min_h));

                    ctx.set_operator(Operator::Clear);
                    path_squircle_rect(ctx, rx, ry, rw, rh, 14.0);
                    let _ = ctx.fill();

                    ctx.set_operator(Operator::Over);
                    let (ar, ag, ab) = malus_de_common::config::load_accent_color();
                    path_squircle_rect(ctx, rx, ry, rw, rh, 14.0);
                    ctx.set_source_rgba(ar, ag, ab, 0.14);
                    let _ = ctx.fill_preserve();
                    ctx.set_source_rgba(ar, ag, ab, 0.90);
                    ctx.set_line_width(2.5);
                    let _ = ctx.stroke();
                }
            }
            ScreenshotUiMode::CaptureArea
            | ScreenshotUiMode::RecordArea
            | ScreenshotUiMode::OcrArea => {
                // Fondo oscuro translúcido
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.38);
                let _ = ctx.paint();

                let r = clamp_selection_to_bounds(self.selection, sw, sh);
                let (rx, ry, rw, rh) = (r.x, r.y, r.width, r.height);

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

                // Borde blanco nítido (o acento en modo OCR)
                ctx.rectangle(rx, ry, rw, rh);
                if self.mode.is_ocr() {
                    let (ar, ag, ab) = malus_de_common::config::load_accent_color();
                    ctx.set_source_rgba(ar, ag, ab, 0.96);
                } else {
                    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.98);
                }
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
                let badge_text = if self.mode.is_ocr() {
                    format!("OCR · {} × {}", rw.round() as i32, rh.round() as i32)
                } else {
                    format!("{} × {}", rw.round() as i32, rh.round() as i32)
                };
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
                    let max_bx = (sw - bw - 10.0).max(10.0);
                    let bx = (rx + (rw - bw) * 0.5).clamp(10.0, max_bx);

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

    fn draw_toolbar(&self, ctx: &Context, mask_ctx: &Context, sw: f64, sh: f64, is_dark: bool) {
        let (x, y, w, h) = self.toolbar_rect(sw, sh);
        let r = TOOLBAR_RADIUS;
        let cy = y + h * 0.5;

        // Máscara para Dual-Kawase blur con geometría squircle exacta
        path_squircle_rect(mask_ctx, x, y, w, h, r);
        mask_ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = mask_ctx.fill();

        // 1. Sombra suave superelíptica
        draw_soft_shadow(ctx, x, y, w, h, r, is_dark);

        // 2. Fondo de cristal translúcido (frosted glass orgánico)
        let glass_scale = malus_de_common::config::current_liquid_glass_scale();
        path_squircle_rect(ctx, x, y, w, h, r);
        if is_dark {
            let a =
                malus_de_common::config::scaled_liquid_glass_alpha(0.68, glass_scale, 0.16, 0.95);
            ctx.set_source_rgba(0.16, 0.17, 0.20, a);
        } else {
            let a =
                malus_de_common::config::scaled_liquid_glass_alpha(0.62, glass_scale, 0.16, 0.95);
            ctx.set_source_rgba(0.96, 0.96, 0.98, a);
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

        let fg = if is_dark {
            [0.96, 0.96, 0.98]
        } else {
            [0.12, 0.12, 0.14]
        };

        // Agarre izquierdo para arrastrar la barra flotante (6 puntos sutiles)
        draw_drag_grip(
            ctx,
            x + 12.0,
            cy,
            self.hovered_item == Some(9) || self.active_handle == HandleType::MoveToolbar,
            fg,
        );

        // Botón Cerrar (✕)
        draw_close_button(
            ctx,
            x + 39.0,
            cy,
            self.hovered_item == Some(0),
            fg,
            is_dark,
        );

        // Separador 1
        draw_v_sep(ctx, x + 58.0, y + 8.0, h - 16.0, is_dark);

        // Modos de Foto y OCR: Pantalla, Ventana, Área, Texto (OCR)
        let btn1_act = self.mode == ScreenshotUiMode::CaptureFull;
        let btn2_act = self.mode == ScreenshotUiMode::CaptureWindow;
        let btn3_act = self.mode == ScreenshotUiMode::CaptureArea;
        let btn_ocr_act = self.mode == ScreenshotUiMode::OcrArea;

        draw_mode_button_plate(
            ctx,
            x + 80.0,
            y + TOOLBAR_PADDING,
            btn1_act,
            self.hovered_item == Some(1),
            is_dark,
        );
        draw_monitor_icon(ctx, x + 80.0, cy, fg, false);

        draw_mode_button_plate(
            ctx,
            x + 124.0,
            y + TOOLBAR_PADDING,
            btn2_act,
            self.hovered_item == Some(2),
            is_dark,
        );
        draw_window_icon(ctx, x + 124.0, cy, fg);

        draw_mode_button_plate(
            ctx,
            x + 168.0,
            y + TOOLBAR_PADDING,
            btn3_act,
            self.hovered_item == Some(3),
            is_dark,
        );
        draw_marquee_icon(ctx, x + 168.0, cy, fg, false);

        draw_mode_button_plate(
            ctx,
            x + 212.0,
            y + TOOLBAR_PADDING,
            btn_ocr_act,
            self.hovered_item == Some(10),
            is_dark,
        );
        draw_ocr_icon(ctx, x + 212.0, cy, fg);

        // Separador 2
        draw_v_sep(ctx, x + 234.0, y + 8.0, h - 16.0, is_dark);

        // Modos de Video: Grabar Pantalla, Grabar Área
        let btn4_act = self.mode == ScreenshotUiMode::RecordFull;
        let btn5_act = self.mode == ScreenshotUiMode::RecordArea;

        draw_mode_button_plate(
            ctx,
            x + 257.0,
            y + TOOLBAR_PADDING,
            btn4_act,
            self.hovered_item == Some(4),
            is_dark,
        );
        draw_monitor_icon(ctx, x + 257.0, cy, fg, true);

        draw_mode_button_plate(
            ctx,
            x + 303.0,
            y + TOOLBAR_PADDING,
            btn5_act,
            self.hovered_item == Some(5),
            is_dark,
        );
        draw_marquee_icon(ctx, x + 303.0, cy, fg, true);

        // Separador 3
        draw_v_sep(ctx, x + 326.0, y + 8.0, h - 16.0, is_dark);

        // Botón Opciones ▾
        let opt_hov = self.hovered_item == Some(6);
        draw_options_button(
            ctx,
            x + 332.0,
            y + TOOLBAR_PADDING,
            96.0,
            h - TOOLBAR_PADDING * 2.0,
            self.show_options_menu,
            opt_hov,
            fg,
            is_dark,
        );

        // Separador 4
        draw_v_sep(ctx, x + 434.0, y + 8.0, h - 16.0, is_dark);

        // Botón de Acción Principal ("Capturar" / "Grabar" / "Extraer")
        let is_video = self.mode.is_video();
        let act_label = if is_video {
            "Grabar"
        } else if self.mode.is_ocr() {
            "Extraer"
        } else {
            "Capturar"
        };
        let act_hov = self.hovered_item == Some(7);
        let act_x = x + 440.0;
        let act_y = y + TOOLBAR_PADDING;
        let act_w = w - 440.0 - TOOLBAR_PADDING;
        let act_h = h - TOOLBAR_PADDING * 2.0;
        draw_action_button(
            ctx,
            act_x,
            act_y,
            act_w,
            act_h,
            act_label,
            is_video,
            act_hov,
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
        let (x, y, w, h) = self.options_menu_rect(sw, sh);
        let r = 12.0;

        path_squircle_rect(mask_ctx, x, y, w, h, r);
        mask_ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = mask_ctx.fill();

        draw_soft_shadow(ctx, x, y, w, h, r, is_dark);

        let glass_scale = malus_de_common::config::current_liquid_glass_scale();
        let menu_a =
            malus_de_common::config::scaled_liquid_glass_alpha(0.88, glass_scale, 0.20, 0.96);
        path_squircle_rect(ctx, x, y, w, h, r);
        if is_dark {
            ctx.set_source_rgba(0.16, 0.17, 0.20, menu_a);
        } else {
            ctx.set_source_rgba(0.96, 0.96, 0.98, menu_a);
        }
        let _ = ctx.fill_preserve();
        ctx.set_line_width(1.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.18 } else { 0.45 });
        let _ = ctx.stroke();

        let fg = if is_dark {
            [0.96, 0.96, 0.98]
        } else {
            [0.12, 0.12, 0.14]
        };

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
            draw_menu_row(
                ctx,
                x + 5.0,
                iy,
                w - 10.0,
                20.0,
                checked,
                label,
                hovered,
                fg,
            );
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
            draw_menu_row(
                ctx,
                x + 5.0,
                iy,
                w - 10.0,
                20.0,
                checked,
                label,
                hovered,
                fg,
            );
        }

        // Separador 2
        draw_menu_h_sep(ctx, x + 12.0, y + 194.0, w - 24.0, fg);

        // Cabecera: OPCIONES
        draw_menu_header(ctx, x + 28.0, y + 211.0, "Opciones", fg);

        let opts = [
            (7, self.options.remember_selection, "Recordar selección"),
            (8, self.options.include_audio, "Grabar micrófono"),
            (9, self.options.lossless, "Video sin pérdidas"),
            (
                10,
                self.options.transparent_window_bg,
                "Fondo transparente (ventana)",
            ),
        ];
        for (i, (item_idx, checked, label)) in opts.iter().enumerate() {
            let iy = y + 216.0 + i as f64 * 20.0;
            let hovered = self.hovered_menu_item == Some(*item_idx);
            draw_menu_row(
                ctx,
                x + 5.0,
                iy,
                w - 10.0,
                20.0,
                *checked,
                label,
                hovered,
                fg,
            );
        }
    }

    fn draw_lossless_pill(
        &self,
        ctx: &Context,
        mask_ctx: &Context,
        sw: f64,
        sh: f64,
        is_dark: bool,
    ) {
        let (x, y, w, h) = self.lossless_pill_rect(sw, sh);
        let r = TOOLBAR_RADIUS;
        let is_active = self.options.lossless;
        let is_hover = self.hovered_item == Some(8);

        // Máscara para Dual-Kawase blur con la misma geometría squircle que la barra principal
        path_squircle_rect(mask_ctx, x, y, w, h, r);
        mask_ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = mask_ctx.fill();

        draw_soft_shadow(ctx, x, y, w, h, r, is_dark);

        let glass_scale = malus_de_common::config::current_liquid_glass_scale();
        path_squircle_rect(ctx, x, y, w, h, r);
        if is_dark {
            let a =
                malus_de_common::config::scaled_liquid_glass_alpha(0.68, glass_scale, 0.16, 0.95);
            ctx.set_source_rgba(0.16, 0.17, 0.20, a);
        } else {
            let a =
                malus_de_common::config::scaled_liquid_glass_alpha(0.62, glass_scale, 0.16, 0.95);
            ctx.set_source_rgba(0.96, 0.96, 0.98, a);
        }
        let _ = ctx.fill_preserve();

        ctx.set_line_width(1.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.16 } else { 0.45 });
        let _ = ctx.stroke();

        // Brillo superior sutil (specular rim)
        path_squircle_rect(ctx, x + 0.5, y + 0.5, w - 1.0, h - 1.0, r - 0.5);
        let rim = LinearGradient::new(0.0, y, 0.0, y + 14.0);
        rim.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if is_dark { 0.18 } else { 0.35 });
        rim.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
        let _ = ctx.set_source(&rim);
        ctx.set_line_width(1.0);
        let _ = ctx.stroke();

        // Placa interior concéntrica (radio 16 px) con color de énfasis cuando está activo
        let inner_x = x + TOOLBAR_PADDING;
        let inner_y = y + TOOLBAR_PADDING;
        let inner_w = w - TOOLBAR_PADDING * 2.0;
        let inner_h = h - TOOLBAR_PADDING * 2.0;

        if is_active {
            let (red, green, blue) = malus_de_common::config::load_accent_color();
            path_squircle_rect(ctx, inner_x, inner_y, inner_w, inner_h, ACTION_BTN_RADIUS);
            if is_hover {
                ctx.set_source_rgba(
                    (red * 1.08).min(1.0),
                    (green * 1.08).min(1.0),
                    (blue * 1.08).min(1.0),
                    1.0,
                );
            } else {
                ctx.set_source_rgba(red, green, blue, 0.94);
            }
            let _ = ctx.fill_preserve();
            ctx.set_line_width(0.8);
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.22);
            let _ = ctx.stroke();
        } else if is_hover {
            path_squircle_rect(ctx, inner_x, inner_y, inner_w, inner_h, ACTION_BTN_RADIUS);
            ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.12 } else { 0.26 });
            let _ = ctx.fill();
        }

        let fg = if is_active {
            [1.0, 1.0, 1.0]
        } else if is_dark {
            [0.96, 0.96, 0.98]
        } else {
            [0.12, 0.12, 0.14]
        };

        let cy = y + h * 0.5;
        let label = "Sin pérdidas";
        ctx.set_font_size(12.0);
        if let Ok(ext) = ctx.text_extents(label) {
            let icon_w = 14.0;
            let gap = 5.0;
            let total_w = icon_w + gap + ext.width();
            let start_x = x + (w - total_w) * 0.5;

            // Indicador visual: círculo con check cuando está activo, anillo cuando está inactivo
            let icx = start_x + icon_w * 0.5;
            if is_active {
                draw_checkmark(ctx, icx, cy, fg);
            } else {
                ctx.arc(icx, cy, 4.5, 0.0, TAU);
                ctx.set_line_width(1.4);
                ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.65);
                let _ = ctx.stroke();
            }

            let text_x = start_x + icon_w + gap - ext.x_bearing();
            let text_y = y + (h - ext.height()) * 0.5 - ext.y_bearing();
            ctx.set_source_rgba(fg[0], fg[1], fg[2], if is_active { 0.98 } else { 0.90 });
            ctx.move_to(text_x, text_y);
            let _ = ctx.show_text(label);
        }
    }

}

fn toolbar_geometry(sw: f64, sh: f64) -> (f64, f64, f64, f64) {
    let w = TOOLBAR_WIDTH;
    let h = TOOLBAR_HEIGHT;
    let x = ((sw - w) * 0.5).round();
    let y = (sh - h - 80.0).round().max(10.0);
    (x, y, w, h)
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
    } else if rel_y >= 256.0 && rel_y < 276.0 {
        Some(9) // Video sin pérdidas
    } else if rel_y >= 276.0 && rel_y < 296.0 {
        Some(10) // Fondo transparente en ventana aislada
    } else {
        None
    }
}

fn draw_drag_grip(ctx: &Context, cx: f64, cy: f64, is_active: bool, fg: [f64; 3]) {
    let alpha = if is_active { 0.85 } else { 0.40 };
    ctx.set_source_rgba(fg[0], fg[1], fg[2], alpha);
    for col in [-2.2_f64, 2.2] {
        for row in [-5.0_f64, 0.0, 5.0] {
            ctx.arc(cx + col, cy + row, 1.15, 0.0, TAU);
            let _ = ctx.fill();
        }
    }
}

fn draw_ocr_icon(ctx: &Context, cx: f64, cy: f64, fg: [f64; 3]) {
    // Esquinas tipo visor Live Text + letra 'T' nítida al centro
    let half = 7.5;
    let leg = 3.2;
    ctx.set_source_rgba(fg[0], fg[1], fg[2], 0.90);
    ctx.set_line_width(1.4);
    ctx.set_line_cap(cairo::LineCap::Round);
    ctx.set_line_join(cairo::LineJoin::Round);

    // 4 escuadras de esquina
    let corners = [(-1.0_f64, -1.0_f64), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    for (sx, sy) in corners {
        let ex = cx + sx * half;
        let ey = cy + sy * half;
        ctx.move_to(ex - sx * leg, ey);
        ctx.line_to(ex, ey);
        ctx.line_to(ex, ey - sy * leg);
        let _ = ctx.stroke();
    }

    // Glifo 'T' vectorial centrado
    ctx.set_line_width(1.5);
    ctx.move_to(cx - 3.2, cy - 3.0);
    ctx.line_to(cx + 3.2, cy - 3.0);
    ctx.move_to(cx, cy - 3.0);
    ctx.line_to(cx, cy + 3.6);
    let _ = ctx.stroke();
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

fn draw_soft_shadow(ctx: &Context, x: f64, y: f64, w: f64, h: f64, r: f64, is_dark: bool) {
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

fn draw_close_button(ctx: &Context, cx: f64, cy: f64, is_hover: bool, fg: [f64; 3], is_dark: bool) {
    let r = 9.5;
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
        path_squircle_rect(ctx, x, y, w, h, 10.0);
        if is_dark {
            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.20);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.13);
        }
        let _ = ctx.fill();
    } else if is_hover {
        path_squircle_rect(ctx, x, y, w, h, 10.0);
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
        path_squircle_rect(ctx, x, y, w, h, 10.0);
        ctx.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.20 } else { 0.40 });
        let _ = ctx.fill();
    } else if is_hover {
        path_squircle_rect(ctx, x, y, w, h, 10.0);
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
    is_video: bool,
    is_hover: bool,
) {
    // Radio concéntrico al borde externo de 22 px: 22 - 6 = 16 px
    path_squircle_rect(ctx, x, y, w, h, ACTION_BTN_RADIUS);
    if is_video {
        if is_hover {
            ctx.set_source_rgba(0.95, 0.26, 0.24, 0.98);
        } else {
            ctx.set_source_rgba(0.88, 0.20, 0.19, 0.94);
        }
    } else {
        let (red, green, blue) = malus_de_common::config::load_accent_color();
        if is_hover {
            ctx.set_source_rgba(
                (red * 1.08).min(1.0),
                (green * 1.08).min(1.0),
                (blue * 1.08).min(1.0),
                1.0,
            );
        } else {
            ctx.set_source_rgba(red, green, blue, 0.94);
        }
    }
    let _ = ctx.fill_preserve();

    // Borde especular sutil para integrarse con el acabado Liquid Glass
    ctx.set_line_width(0.8);
    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.22);
    let _ = ctx.stroke();

    ctx.set_font_size(12.5);
    if let Ok(ext) = ctx.text_extents(label) {
        let text_x = x + (w - ext.width()) * 0.5 - ext.x_bearing();
        let text_y = y + (h - ext.height()) * 0.5 - ext.y_bearing();
        ctx.set_source_rgba(1.0, 1.0, 1.0, 0.98);
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
        let (red, green, blue) = malus_de_common::config::load_accent_color();
        ctx.set_source_rgba(red, green, blue, 0.95);
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

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::input::ButtonState;

    #[test]
    fn toolbar_height_matches_previous_44px_design_with_concentric_action_button() {
        assert_eq!(TOOLBAR_HEIGHT, 44.0);
        assert_eq!(TOOLBAR_WIDTH, 544.0);
        assert_eq!(TOOLBAR_RADIUS, 22.0);
        assert_eq!(ACTION_BTN_RADIUS, TOOLBAR_RADIUS - TOOLBAR_PADDING);
        assert_eq!(ACTION_BTN_RADIUS, 16.0);
        let (_, _, w, h) = toolbar_geometry(1920.0, 1080.0);
        assert_eq!((w, h), (544.0, 44.0));

        let mut ui = ScreenshotUiState::new();
        ui.is_recording = true;
        assert!(
            !ui.is_open(),
            "La grabación activa no debe bloquear el teclado ni el escritorio"
        );
    }

    #[test]
    fn toolbar_can_be_dragged_and_clamped_and_options_flips_when_near_top() {
        let mut ui = ScreenshotUiState::new();
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        ui.open(output_geo, "DP-1".to_string());

        let (orig_x, orig_y, _, orig_h) = ui.toolbar_rect(1920.0, 1080.0);
        // Presionar el agarre izquierdo (rel_x = 10)
        let grip = Point::from((orig_x + 10.0, orig_y + orig_h * 0.5));
        let res = ui.handle_pointer_button(output_geo, grip, 0x110, ButtonState::Pressed);
        assert_eq!(res, ScreenshotPointerResult::Consumed);

        // Arrastrar hacia arriba a la esquina superior izquierda
        ui.update_pointer_motion(output_geo, Point::from((120.0, 60.0)));
        let (moved_x, moved_y, _, _) = ui.toolbar_rect(1920.0, 1080.0);
        assert!(moved_x < orig_x);
        assert!(moved_y < orig_y);
        assert!(moved_y >= 40.0);

        // Cuando la barra está pegada arriba, el menú de opciones debe abrirse debajo
        let (_, menu_y, _, _) = ui.options_menu_rect(1920.0, 1080.0);
        assert!(menu_y > moved_y);

        // Soltar termina el arrastre
        let rel = ui.handle_pointer_button(
            output_geo,
            Point::from((120.0, 60.0)),
            0x110,
            ButtonState::Released,
        );
        assert_eq!(rel, ScreenshotPointerResult::Consumed);
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());
    }

    #[test]
    fn space_toggles_area_and_window_and_ocr_mode_renders() {
        let mut ui = ScreenshotUiState::new();
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        ui.open_with_mode(output_geo, "DP-1".to_string(), ScreenshotUiMode::CaptureArea);
        assert_eq!(ui.mode, ScreenshotUiMode::CaptureArea);

        ui.handle_key_press(keysyms::KEY_space.into(), ModifiersState::default());
        assert_eq!(ui.mode, ScreenshotUiMode::CaptureWindow);

        ui.handle_key_press(keysyms::KEY_space.into(), ModifiersState::default());
        assert_eq!(ui.mode, ScreenshotUiMode::CaptureArea);

        ui.open_with_mode(output_geo, "DP-1".to_string(), ScreenshotUiMode::OcrArea);
        assert!(ui.mode.is_ocr());
        assert!(ui.mode.is_area());
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());
        assert!(ui.prepare_frame(output_geo, 1.0, false).is_some());
    }

    #[test]
    fn click_on_secondary_monitor_does_not_corrupt_selection_or_panic() {
        let mut ui = ScreenshotUiState::new();
        let output_geo = Rectangle::new((0, 0).into(), (2560, 1440).into());
        ui.open(output_geo, "DP-1".to_string());

        let result = ui.handle_pointer_button(
            output_geo,
            Point::from((3679.0, 789.0)),
            0x110,
            ButtonState::Pressed,
        );
        assert_eq!(result, ScreenshotPointerResult::Ignored);
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());
    }

    #[test]
    fn near_edge_click_and_out_of_bounds_selection_never_panic_on_render() {
        let mut ui = ScreenshotUiState::new();
        let output_geo = Rectangle::new((0, 0).into(), (2560, 1440).into());
        ui.open(output_geo, "DP-1".to_string());

        // Clic pegado a la esquina inferior derecha (a 1 px del borde)
        let result = ui.handle_pointer_button(
            output_geo,
            Point::from((2559.0, 1439.0)),
            0x110,
            ButtonState::Pressed,
        );
        assert_eq!(result, ScreenshotPointerResult::Consumed);
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());

        // Arrastre que sobresale hacia otro monitor a la derecha
        ui.update_pointer_motion(output_geo, Point::from((3679.0, 1800.0)));
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());

        // Selección guardada fuera de rango (ej. al pasar de un monitor de 2899 px a uno de 2560 px)
        ui.selection = SelectionArea {
            x: 2600.0,
            y: 1500.0,
            width: 200.0,
            height: 200.0,
        };
        assert!(ui.prepare_frame(output_geo, 1.0, false).is_some());

        // Reapertura con remember_selection activo en un monitor más pequeño
        ui.open(output_geo, "DP-1".to_string());
        assert!(ui.selection.x + ui.selection.width <= 2560.0);
        assert!(ui.selection.y + ui.selection.height <= 1440.0);
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());
    }

    #[test]
    fn resize_handles_near_edges_with_small_selection_never_panic() {
        let output_geo = Rectangle::new((0, 0).into(), (2560, 1440).into());
        let handles = [
            HandleType::MoveBox,
            HandleType::NewSelection,
            HandleType::NorthWest,
            HandleType::North,
            HandleType::NorthEast,
            HandleType::East,
            HandleType::SouthEast,
            HandleType::South,
            HandleType::SouthWest,
            HandleType::West,
        ];
        let corner_areas = [
            SelectionArea {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            SelectionArea {
                x: 2550.0,
                y: 1430.0,
                width: 10.0,
                height: 10.0,
            },
        ];

        for area in corner_areas {
            for handle in handles {
                let mut ui = ScreenshotUiState::new();
                ui.open(output_geo, "DP-1".to_string());
                ui.selection = area;
                ui.drag_start_area = area;
                ui.drag_start_pointer = (area.x, area.y);
                ui.active_handle = handle;

                ui.update_pointer_motion(output_geo, Point::from((-200.0, -200.0)));
                ui.update_pointer_motion(output_geo, Point::from((4000.0, 3000.0)));
                assert!(ui.selection.x >= 0.0 && ui.selection.y >= 0.0);
                assert!(ui.selection.x + ui.selection.width <= 2560.0);
                assert!(ui.selection.y + ui.selection.height <= 1440.0);
            }
        }
    }

    #[test]
    fn clamp_selection_handles_nan_and_tiny_outputs() {
        let clamped = clamp_selection_to_bounds(
            SelectionArea {
                x: f64::NAN,
                y: f64::INFINITY,
                width: f64::NEG_INFINITY,
                height: f64::NAN,
            },
            6.0,
            6.0,
        );
        assert!(clamped.x.is_finite() && clamped.y.is_finite());
        assert!(clamped.width.is_finite() && clamped.height.is_finite());
        assert!(clamped.x + clamped.width <= 6.0);
        assert!(clamped.y + clamped.height <= 6.0);
    }

    #[test]
    fn lossless_pill_toggles_only_in_video_mode_and_renders_cleanly() {
        let mut ui = ScreenshotUiState::new();
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        ui.open(output_geo, "DP-1".to_string());
        assert!(!ui.options.lossless);

        let (px, py, pw, ph) = ui.lossless_pill_rect(1920.0, 1080.0);
        let pill_center = Point::from((px + pw * 0.5, py + ph * 0.5));

        // En modo foto, el botón "Sin pérdidas" no debe activarse
        ui.mode = ScreenshotUiMode::CaptureFull;
        let _ = ui.handle_pointer_button(output_geo, pill_center, 0x110, ButtonState::Pressed);
        assert!(!ui.options.lossless);

        // En modo grabación, el botón directo alterna `options.lossless` y renderiza sin errores
        ui.mode = ScreenshotUiMode::RecordFull;
        ui.update_pointer_motion(output_geo, pill_center);
        assert_eq!(ui.hovered_item, Some(8));
        let res = ui.handle_pointer_button(output_geo, pill_center, 0x110, ButtonState::Pressed);
        assert_eq!(res, ScreenshotPointerResult::Consumed);
        assert!(ui.options.lossless);
        assert!(ui.prepare_frame(output_geo, 1.0, true).is_some());
        assert!(ui.prepare_frame(output_geo, 1.0, false).is_some());

        // Segundo clic lo desactiva
        let res = ui.handle_pointer_button(output_geo, pill_center, 0x110, ButtonState::Pressed);
        assert_eq!(res, ScreenshotPointerResult::Consumed);
        assert!(!ui.options.lossless);
    }
}
