use cairo::{Context, Format, ImageSurface, LinearGradient, Operator};
use gio::prelude::AppInfoExt;
use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
            RendererSuper,
        },
    },
    input::keyboard::{keysyms, Keysym, ModifiersState},
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tracing::info;

const WIDTH: i32 = 626;
const HEIGHT: i32 = 110;
const GUTTER: f64 = 28.0;
const SEARCH_WIDTH: f64 = 430.0;
const BUTTON_SIZE: f64 = 54.0;
const GAP: f64 = 16.0;
const SEPARATION_DURATION: Duration = Duration::from_millis(480);

// Dimensiones lógicas del lienzo unificado de Spotlight en el compositor
pub const CANVAS_LOGICAL_W: i32 = 706;
pub const CANVAS_LOGICAL_H: i32 = 540;
pub const CONTENT_W: f64 = 650.0;
pub const CAPSULE_W: f64 = 426.0;
pub const CAPSULE_H: f64 = 54.0;
pub const MODE_BTN_SIZE: f64 = 48.0;
pub const MODE_BTN_GAP: f64 = 8.0;
pub const CARD_RADIUS: f64 = 30.0;
pub const SQUIRCLE_EXPONENT: f64 = 4.2;
pub const SPOTLIGHT_TOP_MARGIN: i32 = 160;

#[derive(Debug, Clone)]
pub struct SpotlightControlsTransition {
    started_at: Instant,
    from: f64,
    to: f64,
}

impl SpotlightControlsTransition {
    pub fn new(from: f64, to: f64, started_at: Instant) -> Self {
        Self {
            started_at,
            from: from.clamp(0.0, 1.0),
            to: to.clamp(0.0, 1.0),
        }
    }

    pub fn progress_at(&self, now: Instant) -> f64 {
        let progress = (now.saturating_duration_since(self.started_at).as_secs_f64()
            / SEPARATION_DURATION.as_secs_f64())
        .clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        self.from + (self.to - self.from) * eased
    }
}

#[derive(Debug, Clone)]
pub struct SmoothTransition {
    started_at: Instant,
    duration: Duration,
    from: f64,
    to: f64,
}

impl SmoothTransition {
    pub fn settled(val: f64) -> Self {
        let now = Instant::now();
        Self {
            started_at: now - Duration::from_secs(10),
            duration: Duration::from_millis(1),
            from: val,
            to: val,
        }
    }

    pub fn new(from: f64, to: f64, now: Instant, duration: Duration) -> Self {
        Self {
            started_at: now,
            duration: duration.max(Duration::from_millis(1)),
            from,
            to,
        }
    }

    pub fn retarget(&mut self, new_to: f64, now: Instant, duration: Duration) {
        if (self.to - new_to).abs() < 1e-4 {
            return;
        }
        let current = self.value_at(now);
        self.from = current;
        self.to = new_to;
        self.started_at = now;
        self.duration = duration.max(Duration::from_millis(1));
    }

    pub fn value_at(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.started_at).as_secs_f64();
        let total = self.duration.as_secs_f64().max(0.001);
        let t = (elapsed / total).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started_at) < self.duration
            && (self.to - self.from).abs() > 1e-4
    }

    pub fn target(&self) -> f64 {
        self.to
    }
}

/// Devuelve `(locale_completo, codigo_idioma)` de la sesión activa (p. ej. `("es_MX", "es")`).
fn session_locale_codes() -> (String, String) {
    let raw = std::env::var("LC_ALL")
        .ok()
        .filter(|v| !v.trim().is_empty() && v != "C" && v != "POSIX")
        .or_else(|| {
            std::env::var("LC_MESSAGES")
                .ok()
                .filter(|v| !v.trim().is_empty() && v != "C" && v != "POSIX")
        })
        .or_else(|| {
            std::env::var("LANGUAGE")
                .ok()
                .and_then(|v| v.split(':').next().map(str::to_string))
                .filter(|v| !v.trim().is_empty() && v != "C" && v != "POSIX")
        })
        .or_else(|| {
            std::env::var("LANG")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "en_US.UTF-8".to_string());

    let without_encoding = raw.split('.').next().unwrap_or(&raw).split('@').next().unwrap_or(&raw).trim();
    let lang = without_encoding
        .split(['_', '-'])
        .next()
        .unwrap_or("en")
        .to_lowercase();
    (without_encoding.to_string(), lang)
}

pub fn is_spanish_session() -> bool {
    session_locale_codes().1 == "es"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotlightMode {
    Search,
    Applications,
    Files,
    Actions,
    Clipboard,
}

impl SpotlightMode {
    pub fn from_button_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Applications),
            1 => Some(Self::Files),
            2 => Some(Self::Actions),
            3 => Some(Self::Clipboard),
            _ => None,
        }
    }

    pub fn button_index(self) -> Option<usize> {
        match self {
            Self::Search => None,
            Self::Applications => Some(0),
            Self::Files => Some(1),
            Self::Actions => Some(2),
            Self::Clipboard => Some(3),
        }
    }

    pub fn title(self) -> &'static str {
        let es = is_spanish_session();
        match self {
            Self::Search => if es { "Búsqueda de Spotlight" } else { "Spotlight Search" },
            Self::Applications => if es { "Aplicaciones" } else { "Applications" },
            Self::Files => if es { "Archivos" } else { "Files" },
            Self::Actions => if es { "Acciones" } else { "Actions" },
            Self::Clipboard => if es { "Portapapeles" } else { "Clipboard" },
        }
    }

    pub fn placeholder(self) -> &'static str {
        let es = is_spanish_session();
        match self {
            Self::Search => if es { "Búsqueda de Spotlight" } else { "Spotlight Search" },
            Self::Applications => if es { "Buscar aplicaciones" } else { "Search Applications" },
            Self::Files => if es { "Buscar archivos" } else { "Search Files" },
            Self::Actions => if es { "Buscar acciones" } else { "Search Actions" },
            Self::Clipboard => if es { "Buscar en portapapeles" } else { "Search Clipboard" },
        }
    }

    pub fn shortcut_badge(self) -> &'static str {
        match self {
            Self::Search => "",
            Self::Applications => "⌘1",
            Self::Files => "⌘2",
            Self::Actions => "⌘3",
            Self::Clipboard => "⌘4",
        }
    }

    pub fn filter_chips(self) -> &'static [&'static str] {
        let es = is_spanish_session();
        match self {
            Self::Search => {
                if es {
                    &["Todo", "Apps", "Archivos", "Acciones"]
                } else {
                    &["All", "Apps", "Files", "Actions"]
                }
            }
            Self::Applications => {
                if es {
                    &["Todo", "Productividad", "Desarrollo", "Multimedia", "Sistema"]
                } else {
                    &["All", "Productivity", "Development", "Media", "System"]
                }
            }
            Self::Files => {
                if es {
                    &["Todo", "PDF", "Imágenes", "Documentos", "Código"]
                } else {
                    &["All", "PDF", "Images", "Documents", "Code"]
                }
            }
            Self::Actions => {
                if es {
                    &["Todo", "Sistema", "Nota rápida", "Escritorio"]
                } else {
                    &["All", "System", "Quick Note", "Desktop"]
                }
            }
            Self::Clipboard => {
                if es {
                    &["Todo", "Texto", "Enlaces"]
                } else {
                    &["All", "Text", "Links"]
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub name: String,
    pub exec: String,
    pub desktop_file: String,
    pub icon: String,
    pub comment: String,
    pub categories: String,
    pub search_terms: String,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub display_path: String,
    pub extension: String,
    pub modified_secs: u64,
}

#[derive(Clone, Debug)]
pub struct ActionTemplateState {
    pub action_id: &'static str,
    pub label_prefix: &'static str,
    pub field1_placeholder: &'static str,
    pub field1_value: String,
    pub label_mid: &'static str,
    pub field2_placeholder: &'static str,
    pub field2_value: String,
    pub active_field: usize,
}

#[derive(Clone, Debug)]
pub enum ResultExecuteAction {
    LaunchDesktopApp {
        desktop_file: String,
        exec: String,
    },
    OpenFile(PathBuf),
    RunCommand(String),
    OpenTemplate {
        action_id: &'static str,
        label_prefix: &'static str,
        field1_placeholder: &'static str,
        default_field2: &'static str,
        label_mid: &'static str,
        field2_placeholder: &'static str,
    },
    CopyText(String),
    ToggleTheme,
    SystemLogout,
    SystemReboot,
    SystemPowerOff,
}

#[derive(Clone, Debug)]
pub struct SpotlightRowItem {
    pub title: String,
    pub subtitle: String,
    pub badge: String,
    pub icon_kind: RowIconKind,
    pub action: ResultExecuteAction,
}

#[derive(Clone, Debug)]
pub enum RowIconKind {
    Calculator,
    App(String),
    File(String),
    Action(&'static str),
    Clipboard,
}

#[derive(Deserialize)]
struct PersistedClipboardHistory {
    #[serde(default)]
    entries: Vec<PersistedClipboardEntry>,
}

#[derive(Deserialize)]
struct PersistedClipboardEntry {
    #[serde(default)]
    preview: String,
    #[serde(default)]
    copied_at_unix: u64,
    #[serde(default)]
    text: Option<String>,
}

pub struct SpotlightState {
    is_open: bool,
    active_output_name: Option<String>,
    pub query: String,
    pub mode: SpotlightMode,
    pub hover_transition: SmoothTransition,
    pub expand_transition: SmoothTransition,
    pub height_transition: SmoothTransition,
    pub open_transition: SmoothTransition,
    pub hovered_button: Option<usize>,
    pub hovered_row: Option<usize>,
    pub hovered_filter: Option<usize>,
    pub hover_armed: bool,
    pub selected_index: usize,
    pub scroll_offset: f64,
    pub active_filter: usize,
    pub active_template: Option<ActionTemplateState>,

    pub apps: Vec<AppEntry>,
    pub files: Vec<FileEntry>,
    pub clipboard_items: Vec<(String, String)>,
    icon_cache: HashMap<String, Option<ImageSurface>>,

    visual_buffer: Option<MemoryRenderBuffer>,
    visual_buffer_size: (i32, i32),
    mask_pixels: Vec<u8>,
    mask_size: (i32, i32),
    mask_dirty: bool,
    last_render_signature: u64,
}

impl Default for SpotlightState {
    fn default() -> Self {
        Self::new()
    }
}

impl SpotlightState {
    pub fn new() -> Self {
        let apps = index_applications();
        let files = index_files();
        let clipboard_items = load_clipboard_items();
        Self {
            is_open: false,
            active_output_name: None,
            query: String::new(),
            mode: SpotlightMode::Search,
            hover_transition: SmoothTransition::settled(0.0),
            expand_transition: SmoothTransition::settled(0.0),
            height_transition: SmoothTransition::settled(CAPSULE_H),
            open_transition: SmoothTransition::settled(0.0),
            hovered_button: None,
            hovered_row: None,
            hovered_filter: None,
            hover_armed: false,
            selected_index: 0,
            scroll_offset: 0.0,
            active_filter: 0,
            active_template: None,
            apps,
            files,
            clipboard_items,
            icon_cache: HashMap::new(),
            visual_buffer: None,
            visual_buffer_size: (0, 0),
            mask_pixels: Vec::new(),
            mask_size: (0, 0),
            mask_dirty: true,
            last_render_signature: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn active_output_name(&self) -> Option<&str> {
        self.active_output_name.as_deref()
    }

    pub fn should_render_on_output(&self, output_name: &str) -> bool {
        if !self.is_open {
            return false;
        }
        match self.active_output_name.as_deref() {
            Some(target) => target == output_name,
            None => true,
        }
    }

    /// Abre Spotlight reiniciando siempre a su estado compacto original:
    /// solo la barra de entrada, sin botones laterales desplegados y sin modo/búsqueda previa.
    pub fn open(&mut self) {
        let now = Instant::now();
        self.is_open = true;
        self.query.clear();
        self.mode = SpotlightMode::Search;
        self.hover_transition = SmoothTransition::settled(0.0);
        self.expand_transition = SmoothTransition::settled(0.0);
        self.height_transition = SmoothTransition::settled(CAPSULE_H);
        self.open_transition = SmoothTransition::new(0.0, 1.0, now, Duration::from_millis(180));
        self.hovered_button = None;
        self.hovered_row = None;
        self.hovered_filter = None;
        self.hover_armed = false;
        self.selected_index = 0;
        self.scroll_offset = 0.0;
        self.active_filter = 0;
        self.active_template = None;
        self.clipboard_items = load_clipboard_items();
        self.files = index_files();
        self.mask_dirty = true;
        self.last_render_signature = 0;
    }

    pub fn open_on_output(&mut self, output_name: Option<String>) {
        self.active_output_name = output_name;
        self.open();
    }

    pub fn close(&mut self) {
        if !self.is_open {
            return;
        }
        self.is_open = false;
        self.active_output_name = None;
        self.query.clear();
        self.mode = SpotlightMode::Search;
        self.hover_transition = SmoothTransition::settled(0.0);
        self.expand_transition = SmoothTransition::settled(0.0);
        self.height_transition = SmoothTransition::settled(CAPSULE_H);
        self.hovered_button = None;
        self.hovered_row = None;
        self.hovered_filter = None;
        self.hover_armed = false;
        self.selected_index = 0;
        self.scroll_offset = 0.0;
        self.active_filter = 0;
        self.active_template = None;
        self.mask_dirty = true;
        self.last_render_signature = 0;
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn toggle_on_output(&mut self, output_name: Option<String>) {
        if self.is_open {
            self.close();
        } else {
            self.open_on_output(output_name);
        }
    }

    pub fn is_expanded(&self) -> bool {
        self.mode != SpotlightMode::Search
            || !self.query.trim().is_empty()
            || self.active_template.is_some()
    }

    pub fn canvas_logical_rect(&self, output_geo: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let x = output_geo.loc.x + ((output_geo.size.w - CANVAS_LOGICAL_W) / 2).max(0);
        let y = output_geo.loc.y + SPOTLIGHT_TOP_MARGIN.min((output_geo.size.h / 5).max(80));
        Rectangle::new((x, y).into(), (CANVAS_LOGICAL_W, CANVAS_LOGICAL_H).into())
    }

    fn capsule_rect_at(&self, hover_t: f64, expand_t: f64, card_h: f64) -> (f64, f64, f64, f64, f64) {
        let x = GUTTER;
        let y = GUTTER;
        let split_t = (hover_t * (1.0 - expand_t)).clamp(0.0, 1.0);
        let w = CONTENT_W - (CONTENT_W - CAPSULE_W) * split_t;
        let h = CAPSULE_H + (card_h - CAPSULE_H) * expand_t;
        let r = (CAPSULE_H * 0.5) + (CARD_RADIUS - CAPSULE_H * 0.5) * expand_t;
        (x, y, w, h, r)
    }

    fn mode_button_centers_at(&self, hover_t: f64, expand_t: f64) -> [(f64, f64, f64); 4] {
        let center_y = GUTTER + CAPSULE_H * 0.5;
        let btn_r = MODE_BTN_SIZE * 0.5;
        let vis = (hover_t * (1.0 - expand_t)).clamp(0.0, 1.0);

        let mut out = [(0.0, center_y, btn_r); 4];
        for (i, slot) in out.iter_mut().enumerate() {
            let target_cx = GUTTER
                + CAPSULE_W
                + MODE_BTN_GAP
                + btn_r
                + (i as f64) * (MODE_BTN_SIZE + MODE_BTN_GAP);
            let start_cx = target_cx - ((3 - i) as f64) * MODE_BTN_GAP;
            let cx = start_cx + (target_cx - start_cx) * vis;
            *slot = (cx, center_y, btn_r);
        }
        out
    }

    fn sync_transitions(&mut self, now: Instant) {
        let expanded = self.is_expanded();
        let target_expand = if expanded { 1.0 } else { 0.0 };
        self.expand_transition
            .retarget(target_expand, now, Duration::from_millis(260));

        if expanded {
            self.hover_transition
                .retarget(0.0, now, Duration::from_millis(220));
            self.hovered_button = None;
        }

        let target_h = if !expanded {
            CAPSULE_H
        } else if self.mode == SpotlightMode::Applications {
            let apps = self.filtered_apps();
            let rows = apps.len().div_ceil(5).clamp(1, 3);
            (116.0 + rows as f64 * 98.0 + 18.0).clamp(220.0, 430.0)
        } else {
            let rows = self.current_rows();
            let count = rows.len();
            if count == 0 {
                180.0
            } else {
                (112.0 + (count.min(6) as f64) * 56.0 + 16.0).clamp(180.0, 456.0)
            }
        };
        self.height_transition
            .retarget(target_h, now, Duration::from_millis(250));
    }

    pub fn update_pointer_motion(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        pointer_loc: Point<f64, Logical>,
    ) {
        if !self.is_open {
            return;
        }
        let now = Instant::now();
        self.sync_transitions(now);

        let canvas = self.canvas_logical_rect(output_geo);
        let lx = pointer_loc.x - canvas.loc.x as f64;
        let ly = pointer_loc.y - canvas.loc.y as f64;

        let hover_t = self.hover_transition.value_at(now);
        let expand_t = self.expand_transition.value_at(now);
        let card_h = self.height_transition.value_at(now);
        let (cap_x, cap_y, cap_w, cap_h, _) = self.capsule_rect_at(hover_t, expand_t, card_h);

        if !self.is_expanded() {
            // En modo compacto, el ancho total ocupado es siempre CONTENT_W (al pasar el cursor se divide sin crecer)
            let right_bound = (cap_x + cap_w).max(GUTTER + CONTENT_W) + 6.0;
            let inside_bar = lx >= cap_x - 6.0
                && lx <= right_bound
                && ly >= cap_y - 8.0
                && ly <= cap_y + CAPSULE_H + 8.0;

            if inside_bar {
                self.hover_armed = true;
                self.hover_transition
                    .retarget(1.0, now, Duration::from_millis(380));
            } else if self.hover_armed || self.hover_transition.target() > 0.0 {
                self.hover_transition
                    .retarget(0.0, now, Duration::from_millis(320));
                self.hovered_button = None;
            }

            let updated_hover_t = self.hover_transition.value_at(now);
            let mut hit_btn = None;
            if updated_hover_t > 0.25 {
                let centers = self.mode_button_centers_at(updated_hover_t, expand_t);
                for (i, (cx, cy, r)) in centers.iter().enumerate() {
                    let dx = lx - *cx;
                    let dy = ly - *cy;
                    if dx * dx + dy * dy <= (r + 3.0) * (r + 3.0) {
                        hit_btn = Some(i);
                        break;
                    }
                }
            }
            self.hovered_button = hit_btn;
            self.hovered_row = None;
            self.hovered_filter = None;
        } else {
            // En modo expandido, detectar hover sobre chips de filtro y sobre filas o celdas de apps
            self.hovered_button = None;
            let chips = self.mode.filter_chips();
            let mut filter_hit = None;
            if (60.0..=96.0).contains(&ly) {
                let mut chip_x = GUTTER + 20.0;
                for (idx, label) in chips.iter().enumerate() {
                    let chip_w = (label.len() as f64 * 7.6 + 24.0).max(48.0);
                    if lx >= chip_x && lx <= chip_x + chip_w {
                        filter_hit = Some(idx);
                        break;
                    }
                    chip_x += chip_w + 8.0;
                }
            }
            self.hovered_filter = filter_hit;

            let content_top = GUTTER + 104.0;
            let content_bottom = cap_y + cap_h - 12.0;
            if lx >= GUTTER + 14.0 && lx <= GUTTER + CONTENT_W - 14.0 && ly >= content_top && ly <= content_bottom {
                let rel_y = ly - content_top + self.scroll_offset;
                if self.mode == SpotlightMode::Applications {
                    let col_w = (CONTENT_W - 36.0) / 5.0;
                    let col = ((lx - (GUTTER + 18.0)) / col_w).floor() as isize;
                    let row = (rel_y / 98.0).floor() as isize;
                    if (0..5).contains(&col) && row >= 0 {
                        let idx = (row as usize) * 5 + (col as usize);
                        let apps_len = self.filtered_apps().len();
                        if idx < apps_len {
                            self.hovered_row = Some(idx);
                            self.selected_index = idx;
                        } else {
                            self.hovered_row = None;
                        }
                    }
                } else {
                    let idx = (rel_y / 56.0).floor() as isize;
                    let rows_len = self.current_rows().len();
                    if idx >= 0 && (idx as usize) < rows_len {
                        self.hovered_row = Some(idx as usize);
                        self.selected_index = idx as usize;
                    } else {
                        self.hovered_row = None;
                    }
                }
            } else {
                self.hovered_row = None;
            }
        }
    }

    /// Procesa un clic izquierdo del ratón.
    /// Devuelve `true` si el clic ocurrió dentro de Spotlight (o si Spotlight estaba abierto y lo consumió/cerró).
    pub fn handle_pointer_button(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        pointer_loc: Point<f64, Logical>,
        button_code: u32,
    ) -> bool {
        if !self.is_open {
            return false;
        }
        let now = Instant::now();
        self.sync_transitions(now);

        let canvas = self.canvas_logical_rect(output_geo);
        let lx = pointer_loc.x - canvas.loc.x as f64;
        let ly = pointer_loc.y - canvas.loc.y as f64;

        let hover_t = self.hover_transition.value_at(now);
        let expand_t = self.expand_transition.value_at(now);
        let card_h = self.height_transition.value_at(now);
        let (cap_x, cap_y, cap_w, cap_h, _) = self.capsule_rect_at(hover_t, expand_t, card_h);

        if !self.is_expanded() {
            // Comprobar clic en cualquiera de los 4 botones circulares de modo
            if hover_t > 0.25 {
                let centers = self.mode_button_centers_at(hover_t, expand_t);
                for (i, (cx, cy, r)) in centers.iter().enumerate() {
                    let dx = lx - *cx;
                    let dy = ly - *cy;
                    if dx * dx + dy * dy <= (r + 3.0) * (r + 3.0) {
                        if button_code == 0x110 {
                            if let Some(target_mode) = SpotlightMode::from_button_index(i) {
                                self.set_mode(target_mode, now);
                            }
                        }
                        return true;
                    }
                }
            }

            let inside_capsule =
                lx >= cap_x && lx <= cap_x + cap_w && ly >= cap_y && ly <= cap_y + cap_h;
            if inside_capsule {
                return true;
            }

            // Clic fuera de Spotlight -> cerrar
            self.close();
            return false;
        }

        // Modo expandido: comprobar si el clic cayó dentro de la tarjeta squircle
        let inside_card =
            lx >= cap_x && lx <= cap_x + cap_w && ly >= cap_y && ly <= cap_y + cap_h;
        if !inside_card {
            self.close();
            return false;
        }

        if button_code != 0x110 {
            return true;
        }

        // Comprobar clic en el botón de modo activo / retorno en la esquina superior derecha de la tarjeta
        let mode_badge_x = cap_x + cap_w - 52.0;
        let mode_badge_y = cap_y + 11.0;
        if lx >= mode_badge_x
            && lx <= mode_badge_x + 38.0
            && ly >= mode_badge_y
            && ly <= mode_badge_y + 34.0
        {
            self.reset_to_compact_search(now);
            return true;
        }

        // Comprobar clic en chips de filtro
        if let Some(filter_idx) = self.hovered_filter {
            self.active_filter = filter_idx;
            self.selected_index = 0;
            self.scroll_offset = 0.0;
            return true;
        }

        // Comprobar clic en un elemento de la cuadrícula de aplicaciones o fila de resultados
        if let Some(idx) = self.hovered_row {
            self.selected_index = idx;
            self.activate_selected(now);
            return true;
        }

        true
    }

    pub fn handle_scroll(&mut self, vertical_delta: f64) -> bool {
        if !self.is_open || !self.is_expanded() {
            return false;
        }
        let max_scroll = if self.mode == SpotlightMode::Applications {
            let rows = self.filtered_apps().len().div_ceil(5);
            ((rows as f64) * 98.0 - 290.0).max(0.0)
        } else {
            let rows = self.current_rows().len();
            ((rows as f64) * 56.0 - 320.0).max(0.0)
        };
        self.scroll_offset = (self.scroll_offset + vertical_delta * 28.0).clamp(0.0, max_scroll);
        true
    }

    pub fn set_mode(&mut self, mode: SpotlightMode, now: Instant) {
        self.mode = mode;
        self.active_filter = 0;
        self.selected_index = 0;
        self.scroll_offset = 0.0;
        self.hovered_button = None;
        self.active_template = None;
        if mode == SpotlightMode::Clipboard {
            self.clipboard_items = load_clipboard_items();
        } else if mode == SpotlightMode::Files {
            self.files = index_files();
        }
        self.sync_transitions(now);
    }

    pub fn reset_to_compact_search(&mut self, now: Instant) {
        self.query.clear();
        self.mode = SpotlightMode::Search;
        self.active_filter = 0;
        self.selected_index = 0;
        self.scroll_offset = 0.0;
        self.active_template = None;
        self.hovered_row = None;
        self.hovered_filter = None;
        self.sync_transitions(now);
    }

    pub fn handle_key_press(&mut self, sym: Keysym, modifiers: ModifiersState) -> bool {
        if !self.is_open {
            return false;
        }
        let now = Instant::now();
        let raw = sym.raw();

        // Escape progresivo: si hay plantilla, modo o texto activo, vuelve a la cápsula compacta; si ya está compacta, cierra Spotlight
        if raw == keysyms::KEY_Escape {
            if self.active_template.is_some() {
                self.active_template = None;
                self.sync_transitions(now);
            } else if self.is_expanded() {
                self.reset_to_compact_search(now);
            } else {
                self.close();
            }
            return true;
        }

        // Atajos rápidos de modo: Super+1..4 o Ctrl+1..4
        if modifiers.logo || modifiers.ctrl {
            match raw {
                keysyms::KEY_1 => {
                    if self.mode == SpotlightMode::Applications {
                        self.reset_to_compact_search(now);
                    } else {
                        self.set_mode(SpotlightMode::Applications, now);
                    }
                    return true;
                }
                keysyms::KEY_2 => {
                    if self.mode == SpotlightMode::Files {
                        self.reset_to_compact_search(now);
                    } else {
                        self.set_mode(SpotlightMode::Files, now);
                    }
                    return true;
                }
                keysyms::KEY_3 => {
                    if self.mode == SpotlightMode::Actions {
                        self.reset_to_compact_search(now);
                    } else {
                        self.set_mode(SpotlightMode::Actions, now);
                    }
                    return true;
                }
                keysyms::KEY_4 => {
                    if self.mode == SpotlightMode::Clipboard {
                        self.reset_to_compact_search(now);
                    } else {
                        self.set_mode(SpotlightMode::Clipboard, now);
                    }
                    return true;
                }
                _ => {}
            }
        }

        // Enter / Return -> activar elemento o ejecutar plantilla inline
        if raw == keysyms::KEY_Return || raw == keysyms::KEY_KP_Enter {
            self.activate_selected(now);
            return true;
        }

        // Tab -> cambiar de campo en plantilla inline o aceptar autocompletado
        if raw == keysyms::KEY_Tab || raw == keysyms::KEY_ISO_Left_Tab {
            if let Some(ref mut tpl) = self.active_template {
                tpl.active_field = (tpl.active_field + 1) % 2;
                return true;
            }
            if let Some(completion) = self.inline_autocomplete() {
                self.query.push_str(&completion);
                self.selected_index = 0;
                self.sync_transitions(now);
                return true;
            }
            let chips_len = self.mode.filter_chips().len();
            if self.is_expanded() && chips_len > 1 {
                self.active_filter = (self.active_filter + 1) % chips_len;
                self.selected_index = 0;
                self.scroll_offset = 0.0;
            }
            return true;
        }

        // Backspace -> borrar carácter o salir de plantilla/modo si está vacío
        if raw == keysyms::KEY_BackSpace {
            if let Some(ref mut tpl) = self.active_template {
                let target = if tpl.active_field == 0 {
                    &mut tpl.field1_value
                } else {
                    &mut tpl.field2_value
                };
                if !target.is_empty() {
                    target.pop();
                } else {
                    self.active_template = None;
                }
                self.sync_transitions(now);
                return true;
            }
            if !self.query.is_empty() {
                self.query.pop();
                self.selected_index = 0;
                self.scroll_offset = 0.0;
                self.sync_transitions(now);
            } else if self.mode != SpotlightMode::Search {
                self.reset_to_compact_search(now);
            }
            return true;
        }

        // Navegación con flechas
        if raw == keysyms::KEY_Down {
            self.move_selection(1);
            return true;
        }
        if raw == keysyms::KEY_Up {
            self.move_selection(-1);
            return true;
        }
        if raw == keysyms::KEY_Right {
            if self.mode == SpotlightMode::Applications {
                self.move_selection(1);
                return true;
            }
            if let Some(completion) = self.inline_autocomplete() {
                self.query.push_str(&completion);
                self.selected_index = 0;
                self.sync_transitions(now);
                return true;
            }
        }
        if raw == keysyms::KEY_Left && self.mode == SpotlightMode::Applications {
            self.move_selection(-1);
            return true;
        }

        // Entrada de texto Unicode desde Keysym
        if !modifiers.ctrl && !modifiers.alt && !modifiers.logo {
            if let Some(ch) = keysym_to_char(sym) {
                if let Some(ref mut tpl) = self.active_template {
                    if tpl.active_field == 0 {
                        tpl.field1_value.push(ch);
                    } else {
                        tpl.field2_value.push(ch);
                    }
                } else {
                    self.query.push(ch);
                    self.selected_index = 0;
                    self.scroll_offset = 0.0;
                }
                self.sync_transitions(now);
                return true;
            }
        }

        true
    }

    fn move_selection(&mut self, delta: isize) {
        if self.mode == SpotlightMode::Applications {
            let count = self.filtered_apps().len();
            if count == 0 {
                self.selected_index = 0;
                return;
            }
            let step = if delta > 0 && delta == 1 && false { 1 } else { delta };
            let next = (self.selected_index as isize + step).clamp(0, count as isize - 1) as usize;
            self.selected_index = next;
            let row = next / 5;
            let row_top = row as f64 * 98.0;
            if row_top < self.scroll_offset {
                self.scroll_offset = row_top;
            } else if row_top + 98.0 > self.scroll_offset + 280.0 {
                self.scroll_offset = (row_top + 98.0 - 280.0).max(0.0);
            }
        } else {
            let count = self.current_rows().len();
            if count == 0 {
                self.selected_index = 0;
                return;
            }
            let next = (self.selected_index as isize + delta).clamp(0, count as isize - 1) as usize;
            self.selected_index = next;
            let row_top = next as f64 * 56.0;
            if row_top < self.scroll_offset {
                self.scroll_offset = row_top;
            } else if row_top + 56.0 > self.scroll_offset + 310.0 {
                self.scroll_offset = (row_top + 56.0 - 310.0).max(0.0);
            }
        }
    }

    fn activate_selected(&mut self, now: Instant) {
        if let Some(tpl) = self.active_template.clone() {
            execute_template_action(&tpl);
            self.close();
            return;
        }

        if self.mode == SpotlightMode::Applications {
            let apps = self.filtered_apps();
            if let Some(app) = apps.get(self.selected_index) {
                launch_desktop_app(&app.desktop_file, &app.exec);
                self.close();
            }
            return;
        }

        let rows = self.current_rows();
        let Some(row) = rows.get(self.selected_index).cloned() else {
            return;
        };

        match row.action {
            ResultExecuteAction::LaunchDesktopApp { desktop_file, exec } => {
                launch_desktop_app(&desktop_file, &exec);
                self.close();
            }
            ResultExecuteAction::OpenFile(path) => {
                open_file_path(&path);
                self.close();
            }
            ResultExecuteAction::RunCommand(cmd) => {
                spawn_shell_command(&cmd);
                self.close();
            }
            ResultExecuteAction::OpenTemplate {
                action_id,
                label_prefix,
                field1_placeholder,
                default_field2,
                label_mid,
                field2_placeholder,
            } => {
                self.active_template = Some(ActionTemplateState {
                    action_id,
                    label_prefix,
                    field1_placeholder,
                    field1_value: String::new(),
                    label_mid,
                    field2_placeholder,
                    field2_value: default_field2.to_string(),
                    active_field: 0,
                });
                self.query.clear();
                self.sync_transitions(now);
            }
            ResultExecuteAction::CopyText(text) => {
                copy_to_clipboard(&text);
                self.close();
            }
            ResultExecuteAction::ToggleTheme => {
                toggle_desktop_theme();
                self.close();
            }
            ResultExecuteAction::SystemLogout => {
                spawn_shell_command("malus-de-ctl logout");
                self.close();
            }
            ResultExecuteAction::SystemReboot => {
                spawn_shell_command("systemctl reboot");
                self.close();
            }
            ResultExecuteAction::SystemPowerOff => {
                spawn_shell_command("systemctl poweroff");
                self.close();
            }
        }
    }

    pub fn inline_autocomplete(&self) -> Option<String> {
        if self.mode != SpotlightMode::Search || self.active_template.is_some() {
            return None;
        }
        let q = self.query.trim();
        let q_chars = q.chars().count();
        if q_chars < 2 {
            return None;
        }
        let q_lower = q.to_lowercase();
        for app in &self.apps {
            let name_lower = app.name.to_lowercase();
            if name_lower.starts_with(&q_lower) && app.name.chars().count() > q_chars {
                return Some(app.name.chars().skip(q_chars).collect());
            }
        }
        for (name, _, _, _, _) in builtin_actions() {
            let name_lower = name.to_lowercase();
            if name_lower.starts_with(&q_lower) && name.chars().count() > q_chars {
                return Some(name.chars().skip(q_chars).collect());
            }
        }
        None
    }

    pub fn filtered_apps(&self) -> Vec<AppEntry> {
        let q = self.query.trim().to_lowercase();

        self.apps
            .iter()
            .filter(|app| {
                let matches_query = q.is_empty()
                    || app.name.to_lowercase().contains(&q)
                    || app.comment.to_lowercase().contains(&q)
                    || app.exec.to_lowercase().contains(&q)
                    || app.search_terms.contains(&q);
                if !matches_query {
                    return false;
                }
                match self.active_filter {
                    1 => {
                        app.categories.contains("Office")
                            || app.categories.contains("Utility")
                            || app.categories.contains("TextEditor")
                    }
                    2 => {
                        app.categories.contains("Development")
                            || app.categories.contains("IDE")
                            || app.name.to_lowercase().contains("code")
                            || app.name.to_lowercase().contains("terminal")
                    }
                    3 => {
                        app.categories.contains("AudioVideo")
                            || app.categories.contains("Graphics")
                            || app.categories.contains("Video")
                    }
                    4 => {
                        app.categories.contains("System")
                            || app.categories.contains("Settings")
                    }
                    _ => true,
                }
            })
            .cloned()
            .collect()
    }

    pub fn current_rows(&self) -> Vec<SpotlightRowItem> {
        let q = self.query.trim().to_lowercase();
        let es = is_spanish_session();

        match self.mode {
            SpotlightMode::Applications => Vec::new(),
            SpotlightMode::Files => self
                .files
                .iter()
                .filter(|f| {
                    let matches_q = q.is_empty()
                        || f.name.to_lowercase().contains(&q)
                        || f.extension.to_lowercase().contains(&q)
                        || f.display_path.to_lowercase().contains(&q);
                    if !matches_q {
                        return false;
                    }
                    match self.active_filter {
                        1 => f.extension.eq_ignore_ascii_case("pdf"),
                        2 => matches!(
                            f.extension.as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "svg" | "gif"
                        ),
                        3 => matches!(
                            f.extension.as_str(),
                            "pdf" | "md" | "txt" | "doc" | "docx" | "odt"
                        ),
                        4 => matches!(
                            f.extension.as_str(),
                            "rs" | "toml" | "json" | "js" | "ts" | "py" | "sh" | "css" | "html"
                        ),
                        _ => true,
                    }
                })
                .take(24)
                .map(|f| SpotlightRowItem {
                    title: f.name.clone(),
                    subtitle: f.display_path.clone(),
                    badge: f.extension.to_uppercase(),
                    icon_kind: RowIconKind::File(f.extension.clone()),
                    action: ResultExecuteAction::OpenFile(f.path.clone()),
                })
                .collect(),
            SpotlightMode::Actions => builtin_actions()
                .into_iter()
                .filter(|(title, subtitle, category_id, keywords, _)| {
                    let matches_q = q.is_empty()
                        || title.to_lowercase().contains(&q)
                        || subtitle.to_lowercase().contains(&q)
                        || keywords.iter().any(|k| k.contains(&q));
                    let matches_chip = match self.active_filter {
                        1 => *category_id == "system",
                        2 => *category_id == "quick_note",
                        3 => *category_id == "desktop",
                        _ => true,
                    };
                    matches_q && matches_chip
                })
                .map(|(title, subtitle, category_id, _, action)| {
                    let badge = match category_id {
                        "system" => if es { "Sistema" } else { "System" },
                        "quick_note" => if es { "Nota rápida" } else { "Quick Note" },
                        "desktop" => if es { "Escritorio" } else { "Desktop" },
                        _ => category_id,
                    };
                    SpotlightRowItem {
                        title: title.to_string(),
                        subtitle: subtitle.to_string(),
                        badge: badge.to_string(),
                        icon_kind: RowIconKind::Action(title),
                        action,
                    }
                })
                .collect(),
            SpotlightMode::Clipboard => self
                .clipboard_items
                .iter()
                .filter(|(preview, full)| {
                    let matches_q = q.is_empty()
                        || preview.to_lowercase().contains(&q)
                        || full.to_lowercase().contains(&q);
                    let matches_chip = match self.active_filter {
                        2 => full.contains("http://") || full.contains("https://"),
                        _ => true,
                    };
                    matches_q && matches_chip
                })
                .map(|(preview, full)| SpotlightRowItem {
                    title: preview.clone(),
                    subtitle: if es {
                        "Copiado al portapapeles — Pulsa Intro para copiar".to_string()
                    } else {
                        "Copied to Clipboard — Press Return to copy".to_string()
                    },
                    badge: if es {
                        "Portapapeles".to_string()
                    } else {
                        "Clipboard".to_string()
                    },
                    icon_kind: RowIconKind::Clipboard,
                    action: ResultExecuteAction::CopyText(full.clone()),
                })
                .collect(),
            SpotlightMode::Search => {
                let mut items = Vec::new();
                if q.is_empty() {
                    return items;
                }

                if self.active_filter == 0 {
                    if let Some(calc) = try_calculate(&self.query) {
                        items.push(SpotlightRowItem {
                            title: format!("= {calc}"),
                            subtitle: if es {
                                format!("Resultado de cálculo para {}", self.query.trim())
                            } else {
                                format!("Calculation result for {}", self.query.trim())
                            },
                            badge: if es {
                                "Copiar".to_string()
                            } else {
                                "Copy".to_string()
                            },
                            icon_kind: RowIconKind::Calculator,
                            action: ResultExecuteAction::CopyText(calc),
                        });
                    }
                }

                if self.active_filter == 0 || self.active_filter == 1 {
                    for app in self
                        .apps
                        .iter()
                        .filter(|a| {
                            a.name.to_lowercase().contains(&q)
                                || a.comment.to_lowercase().contains(&q)
                                || a.exec.to_lowercase().contains(&q)
                                || a.search_terms.contains(&q)
                        })
                        .take(6)
                    {
                        items.push(SpotlightRowItem {
                            title: app.name.clone(),
                            subtitle: if app.comment.is_empty() {
                                if es {
                                    "Aplicación".to_string()
                                } else {
                                    "Application".to_string()
                                }
                            } else {
                                app.comment.clone()
                            },
                            badge: "App".to_string(),
                            icon_kind: RowIconKind::App(app.icon.clone()),
                            action: ResultExecuteAction::LaunchDesktopApp {
                                desktop_file: app.desktop_file.clone(),
                                exec: app.exec.clone(),
                            },
                        });
                    }
                }

                if self.active_filter == 0 || self.active_filter == 3 {
                    for (title, subtitle, _, keywords, action) in builtin_actions() {
                        if title.to_lowercase().contains(&q)
                            || subtitle.to_lowercase().contains(&q)
                            || keywords.iter().any(|k| k.contains(&q))
                        {
                            items.push(SpotlightRowItem {
                                title: title.to_string(),
                                subtitle: subtitle.to_string(),
                                badge: if es {
                                    "Acción".to_string()
                                } else {
                                    "Action".to_string()
                                },
                                icon_kind: RowIconKind::Action(title),
                                action,
                            });
                        }
                    }
                }

                if self.active_filter == 0 || self.active_filter == 2 {
                    for f in self
                        .files
                        .iter()
                        .filter(|f| {
                            f.name.to_lowercase().contains(&q)
                                || f.extension.eq_ignore_ascii_case(&q)
                        })
                        .take(5)
                    {
                        items.push(SpotlightRowItem {
                            title: f.name.clone(),
                            subtitle: f.display_path.clone(),
                            badge: f.extension.to_uppercase(),
                            icon_kind: RowIconKind::File(f.extension.clone()),
                            action: ResultExecuteAction::OpenFile(f.path.clone()),
                        });
                    }
                }

                items
            }
        }
    }

    /// Prepara las superficies físicas (máscara de silueta exacta para GPU Blur + capa visual Cairo)
    /// y devuelve el rectángulo físico en pantalla, los píxeles de máscara (si cambiaron) y la opacidad.
    pub fn prepare_frame(
        &mut self,
        output_geo: Rectangle<i32, Logical>,
        scale: f64,
        is_dark_theme: bool,
    ) -> Option<(Rectangle<i32, Physical>, Option<&[u8]>, (i32, i32), f32)> {
        if !self.is_open {
            return None;
        }

        let now = Instant::now();
        self.sync_transitions(now);

        let scale = scale.max(1.0);
        let logical_rect = self.canvas_logical_rect(output_geo);
        let local_logical = Rectangle::new(
            (
                logical_rect.loc.x - output_geo.loc.x,
                logical_rect.loc.y - output_geo.loc.y,
            )
                .into(),
            logical_rect.size,
        );
        let phys_rect: Rectangle<i32, Physical> = local_logical.to_physical_precise_round(scale);
        let phys_w = phys_rect.size.w.max(2);
        let phys_h = phys_rect.size.h.max(2);

        let hover_t = self.hover_transition.value_at(now);
        let expand_t = self.expand_transition.value_at(now);
        let card_h = self.height_transition.value_at(now);
        let open_t = self.open_transition.value_at(now);

        let caret_blink = ((now.elapsed().as_millis() / 530) % 2) as u64;
        let animating = self.hover_transition.is_animating(now)
            || self.expand_transition.is_animating(now)
            || self.height_transition.is_animating(now)
            || self.open_transition.is_animating(now);

        let mut sig = 1469598103934665603u64;
        let mix = |s: &mut u64, v: u64| {
            *s ^= v;
            *s = s.wrapping_mul(1099511628211);
        };
        mix(&mut sig, (hover_t * 2000.0).round() as u64);
        mix(&mut sig, (expand_t * 2000.0).round() as u64);
        mix(&mut sig, (card_h * 20.0).round() as u64);
        mix(&mut sig, (open_t * 1000.0).round() as u64);
        mix(&mut sig, self.mode as u64);
        mix(&mut sig, self.hovered_button.unwrap_or(99) as u64);
        mix(&mut sig, self.hovered_row.unwrap_or(999) as u64);
        mix(&mut sig, self.hovered_filter.unwrap_or(99) as u64);
        mix(&mut sig, self.selected_index as u64);
        mix(&mut sig, (self.scroll_offset * 10.0).round() as u64);
        mix(&mut sig, self.active_filter as u64);
        mix(&mut sig, u64::from(is_dark_theme));
        mix(&mut sig, caret_blink);
        mix(&mut sig, phys_w as u64);
        mix(&mut sig, phys_h as u64);
        for b in self.query.as_bytes() {
            mix(&mut sig, u64::from(*b));
        }
        if let Some(ref tpl) = self.active_template {
            mix(&mut sig, tpl.active_field as u64 + 10);
            for b in tpl.field1_value.as_bytes() {
                mix(&mut sig, u64::from(*b));
            }
            for b in tpl.field2_value.as_bytes() {
                mix(&mut sig, u64::from(*b));
            }
        }

        if animating
            || sig != self.last_render_signature
            || self.visual_buffer.is_none()
            || self.visual_buffer_size != (phys_w, phys_h)
        {
            self.render_surfaces(phys_w, phys_h, scale, hover_t, expand_t, card_h, is_dark_theme);
            self.last_render_signature = sig;
            self.mask_dirty = true;
        }

        let mask_slice = if self.mask_dirty {
            self.mask_dirty = false;
            Some(self.mask_pixels.as_slice())
        } else {
            None
        };

        Some((phys_rect, mask_slice, self.mask_size, open_t as f32))
    }

    pub fn create_render_element(
        &self,
        renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        alpha: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let buffer = self.visual_buffer.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            buffer,
            Some(alpha.clamp(0.0, 1.0)),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }

    fn render_surfaces(
        &mut self,
        phys_w: i32,
        phys_h: i32,
        scale: f64,
        hover_t: f64,
        expand_t: f64,
        card_h: f64,
        is_dark: bool,
    ) {
        let Ok(mut mask_surface) = ImageSurface::create(Format::ARgb32, phys_w, phys_h) else {
            return;
        };
        let Ok(mask_ctx) = Context::new(&mask_surface) else {
            return;
        };
        mask_ctx.scale(scale, scale);
        mask_ctx.set_operator(Operator::Source);
        mask_ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = mask_ctx.paint();
        mask_ctx.set_operator(Operator::Over);

        // 1. Trazar y rellenar la silueta exacta en mask_surface (100% opaca dentro de la forma, 0% fuera)
        self.draw_silhouette_shapes(&mask_ctx, hover_t, expand_t, card_h);
        drop(mask_ctx);
        mask_surface.flush();

        // 2. Construir la superficie visual (sombra exterior + cuerpo Liquid Glass unificado + iconos y texto)
        let Ok(mut vis_surface) = ImageSurface::create(Format::ARgb32, phys_w, phys_h) else {
            return;
        };
        let Ok(vis_ctx) = Context::new(&vis_surface) else {
            return;
        };
        vis_ctx.scale(scale, scale);
        vis_ctx.set_operator(Operator::Source);
        vis_ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = vis_ctx.paint();
        vis_ctx.set_operator(Operator::Over);

        let (cap_x, cap_y, cap_w, cap_h, cap_r) = self.capsule_rect_at(hover_t, expand_t, card_h);
        let btn_vis = (hover_t * (1.0 - expand_t)).clamp(0.0, 1.0);
        let centers = self.mode_button_centers_at(hover_t, expand_t);

        // 2.1. Sombra exterior suave alrededor de cada forma activa, luego recortada dentro de la silueta
        draw_soft_drop_shadow(&vis_ctx, cap_x, cap_y, cap_w, cap_h, cap_r, expand_t, is_dark);
        if btn_vis > 0.02 {
            for (cx, cy, r) in &centers {
                draw_soft_circle_shadow(&vis_ctx, *cx, *cy, *r, btn_vis, is_dark);
            }
        }
        // Recortar el interior de la silueta para que la sombra jamás oscurezca el cristal translúcido
        vis_ctx.save().ok();
        vis_ctx.set_operator(Operator:: destination_out_operator());
        let _ = vis_ctx.set_source_surface(&mask_surface, 0.0, 0.0);
        // Como mask_surface está en píxeles físicos, reescalamos la matriz de origen
        vis_ctx.identity_matrix();
        let _ = vis_ctx.set_source_surface(&mask_surface, 0.0, 0.0);
        let _ = vis_ctx.paint();
        vis_ctx.restore().ok();

        // 2.2. Rellenar el cuerpo Liquid Glass usando mask_surface como máscara unificada
        // (Cero doble mezcla alfa en los puentes líquidos entre la cápsula y los círculos)
        let (glass_top, glass_bottom, fg_main, fg_muted, border_rgba) = if is_dark {
            (
                [0.19, 0.20, 0.23, 0.56],
                [0.12, 0.13, 0.15, 0.64],
                [0.96, 0.97, 0.99],
                [0.64, 0.66, 0.71],
                [1.0, 1.0, 1.0, 0.22],
            )
        } else {
            (
                [0.97, 0.98, 0.99, 0.64],
                [0.91, 0.93, 0.96, 0.72],
                [0.11, 0.12, 0.15],
                [0.42, 0.44, 0.49],
                [1.0, 1.0, 1.0, 0.65],
            )
        };

        vis_ctx.save().ok();
        let glass_grad = LinearGradient::new(0.0, cap_y, 0.0, cap_y + cap_h);
        glass_grad.add_color_stop_rgba(
            0.0,
            glass_top[0],
            glass_top[1],
            glass_top[2],
            glass_top[3],
        );
        glass_grad.add_color_stop_rgba(
            1.0,
            glass_bottom[0],
            glass_bottom[1],
            glass_bottom[2],
            glass_bottom[3],
        );
        let _ = vis_ctx.set_source(&glass_grad);
        vis_ctx.identity_matrix();
        let _ = vis_ctx.mask_surface(&mask_surface, 0.0, 0.0);
        vis_ctx.restore().ok();

        // 2.3. Borde especular Liquid Glass en la cápsula/tarjeta squircle y los 4 botones circulares
        path_card_shape(&vis_ctx, cap_x, cap_y, cap_w, cap_h, cap_r, expand_t);
        vis_ctx.set_line_width(1.15);
        vis_ctx.set_source_rgba(
            border_rgba[0],
            border_rgba[1],
            border_rgba[2],
            border_rgba[3],
        );
        let _ = vis_ctx.stroke();

        // Brillo cenital superior interno
        path_card_shape(
            &vis_ctx,
            cap_x + 1.0,
            cap_y + 1.0,
            (cap_w - 2.0).max(1.0),
            (cap_h - 2.0).max(1.0),
            (cap_r - 1.0).max(1.0),
            expand_t,
        );
        let rim_grad = LinearGradient::new(0.0, cap_y, 0.0, cap_y + 28.0);
        rim_grad.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if is_dark { 0.24 } else { 0.55 });
        rim_grad.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
        let _ = vis_ctx.set_source(&rim_grad);
        vis_ctx.set_line_width(1.0);
        let _ = vis_ctx.stroke();

        // 2.4. Dibujar los 4 botones circulares y sus iconos vectoriales cuando btn_vis > 0.01
        if btn_vis > 0.01 {
            for (i, (cx, cy, r)) in centers.iter().enumerate() {
                let is_hovered = self.hovered_button == Some(i);
                if is_hovered {
                    vis_ctx.new_sub_path();
                    vis_ctx.arc(*cx, *cy, *r - 0.5, 0.0, TAU);
                    vis_ctx.set_source_rgba(
                        1.0,
                        1.0,
                        1.0,
                        if is_dark { 0.14 } else { 0.38 } * btn_vis,
                    );
                    let _ = vis_ctx.fill();
                }

                vis_ctx.new_sub_path();
                vis_ctx.arc(*cx, *cy, *r, 0.0, TAU);
                vis_ctx.set_line_width(if is_hovered { 1.5 } else { 1.1 });
                vis_ctx.set_source_rgba(
                    border_rgba[0],
                    border_rgba[1],
                    border_rgba[2],
                    border_rgba[3] * btn_vis * if is_hovered { 1.35 } else { 1.0 },
                );
                let _ = vis_ctx.stroke();

                draw_mode_button_icon(&vis_ctx, i, *cx, *cy, fg_main, btn_vis);
            }
        }

        // 2.5. Dibujar el contenido de la barra de búsqueda superior (lupa, texto/placeholder/plantilla, autocompletado, badge)
        self.draw_search_bar_contents(
            &vis_ctx,
            cap_x,
            cap_y,
            cap_w,
            fg_main,
            fg_muted,
            is_dark,
        );

        // 2.6. Si está expandido (expand_t > 0.05), dibujar chips de filtro y el cuerpo de resultados / cuadrícula de apps
        if expand_t > 0.05 {
            self.draw_expanded_contents(
                &vis_ctx,
                cap_x,
                cap_y,
                cap_w,
                cap_h,
                cap_r,
                expand_t,
                fg_main,
                fg_muted,
                is_dark,
            );
        }

        drop(vis_ctx);
        vis_surface.flush();

        if let Ok(mask_data) = mask_surface.data() {
            self.mask_pixels.clear();
            self.mask_pixels.extend_from_slice(&mask_data);
            self.mask_size = (phys_w, phys_h);
        }

        if let Ok(vis_data) = vis_surface.data() {
            if self.visual_buffer.is_none() || self.visual_buffer_size != (phys_w, phys_h) {
                self.visual_buffer = Some(MemoryRenderBuffer::new(
                    Fourcc::Argb8888,
                    Size::from((phys_w, phys_h)),
                    1,
                    smithay::utils::Transform::Normal,
                    None,
                ));
                self.visual_buffer_size = (phys_w, phys_h);
            }
            if let Some(ref mut buf) = self.visual_buffer {
                let mut rctx = buf.render();
                let _ = rctx.draw(|dst| {
                    if dst.len() == vis_data.len() {
                        dst.copy_from_slice(&vis_data);
                    }
                    Ok::<_, ()>(vec![Rectangle::from_size((phys_w, phys_h).into())])
                });
            }
        };
    }

    fn draw_silhouette_shapes(&self, ctx: &Context, hover_t: f64, expand_t: f64, card_h: f64) {
        let (cap_x, cap_y, cap_w, cap_h, cap_r) = self.capsule_rect_at(hover_t, expand_t, card_h);
        ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);

        path_card_shape(ctx, cap_x, cap_y, cap_w, cap_h, cap_r, expand_t);
        let _ = ctx.fill();

        let btn_vis = (hover_t * (1.0 - expand_t)).clamp(0.0, 1.0);
        if btn_vis > 0.005 {
            let centers = self.mode_button_centers_at(hover_t, expand_t);
            for (cx, cy, r) in &centers {
                ctx.new_sub_path();
                ctx.arc(*cx, *cy, *r, 0.0, TAU);
                let _ = ctx.fill();
            }

            // Puentes líquidos metaball durante la separación/retracción (0.005 < btn_vis < 0.985)
            let neck_height = MODE_BTN_SIZE * (1.0 - btn_vis).powf(1.15);
            if neck_height > 0.5 {
                let btn_r = MODE_BTN_SIZE * 0.5;
                let half_neck = (neck_height * 0.5).min(btn_r - 0.5);
                let tangent_offset = (btn_r.powi(2) - half_neck.powi(2)).max(0.0).sqrt();
                let mut prev_cx = cap_x + cap_w - btn_r;
                let center_y = cap_y + CAPSULE_H * 0.5;

                for (cx, _, _) in &centers {
                    let left = prev_cx + tangent_offset;
                    let right = *cx - tangent_offset;
                    if right > left {
                        let _ = draw_liquid_bridge(
                            ctx,
                            left,
                            right,
                            center_y,
                            neck_height,
                            [1.0, 1.0, 1.0],
                        );
                        // Asegurar alfa 1.0 en la máscara de silueta
                        let half_h = neck_height * 0.5;
                        let width = right - left;
                        let r = half_h.min(width * 0.5);
                        if r > 0.0 {
                            ctx.new_path();
                            ctx.arc(right - r, center_y, r, -FRAC_PI_2, FRAC_PI_2);
                            ctx.arc(left + r, center_y, r, FRAC_PI_2, 3.0 * FRAC_PI_2);
                            ctx.close_path();
                            ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                            let _ = ctx.fill();
                        }
                    }
                    prev_cx = *cx;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_search_bar_contents(
        &self,
        ctx: &Context,
        cap_x: f64,
        cap_y: f64,
        cap_w: f64,
        fg_main: [f64; 3],
        fg_muted: [f64; 3],
        is_dark: bool,
    ) {
        let center_y = cap_y + CAPSULE_H * 0.5;
        let icon_cx = cap_x + 27.0;
        draw_magnifying_glass(ctx, icon_cx, center_y, fg_muted, 0.92);

        ctx.select_font_face(
            "Inter",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        );
        ctx.set_font_size(17.5);

        let text_x = cap_x + 48.0;
        let text_y = center_y + 6.0;

        // Si hay una plantilla de acción parametrizada activa (ej. Create Note)
        if let Some(ref tpl) = self.active_template {
            let mut cur_x = text_x;
            ctx.set_source_rgba(fg_main[0], fg_main[1], fg_main[2], 0.95);
            ctx.move_to(cur_x, text_y);
            let _ = ctx.show_text(tpl.label_prefix);
            if let Ok(ext) = ctx.text_extents(tpl.label_prefix) {
                cur_x += ext.x_advance() + 8.0;
            }

            cur_x = draw_template_pill(
                ctx,
                cur_x,
                center_y,
                if tpl.field1_value.is_empty() {
                    tpl.field1_placeholder
                } else {
                    &tpl.field1_value
                },
                !tpl.field1_value.is_empty(),
                tpl.active_field == 0,
                fg_main,
                is_dark,
            ) + 8.0;

            ctx.set_font_size(16.0);
            ctx.set_source_rgba(fg_muted[0], fg_muted[1], fg_muted[2], 0.9);
            ctx.move_to(cur_x, text_y);
            let _ = ctx.show_text(tpl.label_mid);
            if let Ok(ext) = ctx.text_extents(tpl.label_mid) {
                cur_x += ext.x_advance() + 8.0;
            }

            let _ = draw_template_pill(
                ctx,
                cur_x,
                center_y,
                if tpl.field2_value.is_empty() {
                    tpl.field2_placeholder
                } else {
                    &tpl.field2_value
                },
                !tpl.field2_value.is_empty(),
                tpl.active_field == 1,
                fg_main,
                is_dark,
            );
            return;
        }

        // Vista previa al hacer hover sobre uno de los 4 botones circulares
        if self.query.is_empty() && self.mode == SpotlightMode::Search {
            if let Some(btn_idx) = self.hovered_button {
                if let Some(preview_mode) = SpotlightMode::from_button_index(btn_idx) {
                    ctx.set_source_rgba(fg_main[0], fg_main[1], fg_main[2], 0.92);
                    ctx.move_to(text_x, text_y);
                    let _ = ctx.show_text(preview_mode.title());

                    draw_shortcut_pill(
                        ctx,
                        cap_x + cap_w - 54.0,
                        center_y - 12.0,
                        38.0,
                        24.0,
                        preview_mode.shortcut_badge(),
                        fg_muted,
                        is_dark,
                    );
                    return;
                }
            }
        }

        if self.query.is_empty() {
            let placeholder = self.mode.placeholder();
            ctx.set_source_rgba(fg_muted[0], fg_muted[1], fg_muted[2], 0.78);
            ctx.move_to(text_x, text_y);
            let _ = ctx.show_text(placeholder);
        } else {
            ctx.set_source_rgba(fg_main[0], fg_main[1], fg_main[2], 0.96);
            ctx.move_to(text_x, text_y);
            let _ = ctx.show_text(&self.query);

            let query_adv = ctx
                .text_extents(&self.query)
                .map(|e| e.x_advance())
                .unwrap_or(0.0);

            // Sugerencia de autocompletado inline en gris claro
            if let Some(completion) = self.inline_autocomplete() {
                ctx.set_source_rgba(fg_muted[0], fg_muted[1], fg_muted[2], 0.55);
                ctx.move_to(text_x + query_adv, text_y);
                let _ = ctx.show_text(&completion);
            }

            // Cursor de texto parpadeante
            let caret_x = text_x + query_adv + 1.5;
            ctx.set_source_rgba(0.28, 0.58, 1.0, 0.92);
            ctx.set_line_width(2.0);
            ctx.move_to(caret_x, center_y - 10.0);
            ctx.line_to(caret_x, center_y + 10.0);
            let _ = ctx.stroke();
        }

        // Si estamos en un modo específico, mostrar el botón de modo activo a la derecha del encabezado
        if let Some(mode_idx) = self.mode.button_index() {
            let bx = cap_x + cap_w - 33.0;
            let by = center_y;
            ctx.new_sub_path();
            ctx.arc(bx, by, 17.0, 0.0, TAU);
            ctx.set_source_rgba(0.25, 0.54, 0.98, if is_dark { 0.38 } else { 0.24 });
            let _ = ctx.fill();
            draw_mode_button_icon(ctx, mode_idx, bx, by, fg_main, 0.95);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_expanded_contents(
        &mut self,
        ctx: &Context,
        cap_x: f64,
        cap_y: f64,
        cap_w: f64,
        cap_h: f64,
        cap_r: f64,
        expand_t: f64,
        fg_main: [f64; 3],
        fg_muted: [f64; 3],
        is_dark: bool,
    ) {
        ctx.save().ok();
        path_card_shape(ctx, cap_x, cap_y, cap_w, cap_h, cap_r, expand_t);
        ctx.clip();

        let alpha = ((expand_t - 0.12) / 0.88).clamp(0.0, 1.0);

        // Línea divisoria sutil bajo la barra de entrada
        let sep_y = cap_y + 54.0;
        ctx.set_source_rgba(
            fg_main[0],
            fg_main[1],
            fg_main[2],
            (if is_dark { 0.09 } else { 0.08 }) * alpha,
        );
        ctx.set_line_width(1.0);
        ctx.move_to(cap_x + 18.0, sep_y);
        ctx.line_to(cap_x + cap_w - 18.0, sep_y);
        let _ = ctx.stroke();

        // Barra de chips de filtro y título de modo
        let chips = self.mode.filter_chips();
        let mut chip_x = cap_x + 20.0;
        let chip_y = cap_y + 64.0;
        ctx.set_font_size(12.5);
        for (idx, label) in chips.iter().enumerate() {
            let chip_w = (label.len() as f64 * 7.6 + 24.0).max(48.0);
            let is_active = self.active_filter == idx;
            let is_hovered = self.hovered_filter == Some(idx);

            path_squircle_rect(ctx, chip_x, chip_y, chip_w, 26.0, 10.0);
            if is_active {
                ctx.set_source_rgba(0.24, 0.53, 0.97, 0.82 * alpha);
                let _ = ctx.fill();
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.96 * alpha);
            } else {
                ctx.set_source_rgba(
                    fg_main[0],
                    fg_main[1],
                    fg_main[2],
                    (if is_hovered { 0.14 } else { 0.07 }) * alpha,
                );
                let _ = ctx.fill();
                ctx.set_source_rgba(fg_muted[0], fg_muted[1], fg_muted[2], 0.92 * alpha);
            }

            if let Ok(ext) = ctx.text_extents(label) {
                ctx.move_to(chip_x + (chip_w - ext.width()) * 0.5, chip_y + 17.5);
                let _ = ctx.show_text(label);
            }
            chip_x += chip_w + 8.0;
        }

        // Recorte para el área desplazable de resultados
        let content_top = cap_y + 102.0;
        let content_h = (cap_h - 114.0).max(0.0);
        ctx.rectangle(cap_x + 10.0, content_top, cap_w - 20.0, content_h);
        ctx.clip();

        if self.mode == SpotlightMode::Applications {
            let apps = self.filtered_apps();
            if apps.is_empty() {
                draw_empty_state(
                    ctx,
                    cap_x + cap_w * 0.5,
                    content_top + content_h * 0.48,
                    if is_spanish_session() {
                        "No se encontraron resultados."
                    } else {
                        "No results found."
                    },
                    fg_muted,
                    alpha,
                );
            } else {
                let col_w = (cap_w - 36.0) / 5.0;
                let cell_h = 98.0;
                for (idx, app) in apps.iter().enumerate() {
                    let col = idx % 5;
                    let row = idx / 5;
                    let cell_x = cap_x + 18.0 + col as f64 * col_w;
                    let cell_y = content_top + row as f64 * cell_h - self.scroll_offset;
                    if cell_y + cell_h < content_top || cell_y > content_top + content_h {
                        continue;
                    }

                    let is_selected = idx == self.selected_index;
                    if is_selected {
                        path_squircle_rect(
                            ctx,
                            cell_x + 4.0,
                            cell_y + 2.0,
                            col_w - 8.0,
                            cell_h - 4.0,
                            18.0,
                        );
                        ctx.set_source_rgba(
                            fg_main[0],
                            fg_main[1],
                            fg_main[2],
                            (if is_dark { 0.14 } else { 0.11 }) * alpha,
                        );
                        let _ = ctx.fill();
                    }

                    let icon_cx = cell_x + col_w * 0.5;
                    let icon_cy = cell_y + 34.0;
                    self.draw_app_icon(ctx, &app.icon, &app.name, icon_cx, icon_cy, 48.0, alpha);

                    ctx.select_font_face(
                        "Inter",
                        cairo::FontSlant::Normal,
                        cairo::FontWeight::Normal,
                    );
                    ctx.set_font_size(12.0);
                    ctx.set_source_rgba(fg_main[0], fg_main[1], fg_main[2], 0.92 * alpha);
                    let label = truncate_str(&app.name, 14);
                    if let Ok(ext) = ctx.text_extents(&label) {
                        ctx.move_to(icon_cx - ext.width() * 0.5, cell_y + 78.0);
                        let _ = ctx.show_text(&label);
                    }
                }
            }
        } else {
            let rows = self.current_rows();
            if rows.is_empty() {
                draw_empty_state(
                    ctx,
                    cap_x + cap_w * 0.5,
                    content_top + content_h * 0.48,
                    if is_spanish_session() {
                        "No se encontraron resultados."
                    } else {
                        "No results found."
                    },
                    fg_muted,
                    alpha,
                );
            } else {
                let row_h = 56.0;
                for (idx, row) in rows.iter().enumerate() {
                    let ry = content_top + idx as f64 * row_h - self.scroll_offset;
                    if ry + row_h < content_top || ry > content_top + content_h {
                        continue;
                    }
                    let rx = cap_x + 16.0;
                    let rw = cap_w - 32.0;

                    if idx == self.selected_index {
                        path_squircle_rect(ctx, rx, ry + 2.0, rw, row_h - 4.0, 15.0);
                        ctx.set_source_rgba(
                            0.24,
                            0.53,
                            0.97,
                            (if is_dark { 0.28 } else { 0.20 }) * alpha,
                        );
                        let _ = ctx.fill();
                    }

                    let icon_cx = rx + 28.0;
                    let icon_cy = ry + row_h * 0.5;
                    match &row.icon_kind {
                        RowIconKind::App(icon_name) => {
                            self.draw_app_icon(
                                ctx,
                                icon_name,
                                &row.title,
                                icon_cx,
                                icon_cy,
                                34.0,
                                alpha,
                            );
                        }
                        RowIconKind::Calculator => {
                            draw_badge_circle(ctx, icon_cx, icon_cy, 17.0, [0.95, 0.58, 0.16], alpha);
                            ctx.set_font_size(15.0);
                            ctx.set_source_rgba(1.0, 1.0, 1.0, 0.96 * alpha);
                            ctx.move_to(icon_cx - 5.0, icon_cy + 5.0);
                            let _ = ctx.show_text("=");
                        }
                        RowIconKind::File(ext) => {
                            let color = match ext.as_str() {
                                "pdf" => [0.90, 0.26, 0.24],
                                "png" | "jpg" | "jpeg" | "webp" | "svg" => [0.58, 0.34, 0.92],
                                "rs" | "toml" | "json" | "js" | "py" => [0.20, 0.68, 0.46],
                                _ => [0.26, 0.56, 0.96],
                            };
                            draw_badge_circle(ctx, icon_cx, icon_cy, 17.0, color, alpha);
                            draw_mode_button_icon(ctx, 1, icon_cx, icon_cy, [1.0, 1.0, 1.0], alpha);
                        }
                        RowIconKind::Action(_) => {
                            draw_badge_circle(ctx, icon_cx, icon_cy, 17.0, [0.88, 0.32, 0.54], alpha);
                            draw_mode_button_icon(ctx, 2, icon_cx, icon_cy, [1.0, 1.0, 1.0], alpha);
                        }
                        RowIconKind::Clipboard => {
                            draw_badge_circle(ctx, icon_cx, icon_cy, 17.0, [0.35, 0.62, 0.85], alpha);
                            draw_mode_button_icon(ctx, 3, icon_cx, icon_cy, [1.0, 1.0, 1.0], alpha);
                        }
                    }

                    ctx.select_font_face(
                        "Inter",
                        cairo::FontSlant::Normal,
                        cairo::FontWeight::Bold,
                    );
                    ctx.set_font_size(14.5);
                    ctx.set_source_rgba(fg_main[0], fg_main[1], fg_main[2], 0.96 * alpha);
                    let title_str = truncate_str(&row.title, 48);
                    ctx.move_to(rx + 56.0, ry + 24.0);
                    let _ = ctx.show_text(&title_str);

                    ctx.select_font_face(
                        "Inter",
                        cairo::FontSlant::Normal,
                        cairo::FontWeight::Normal,
                    );
                    ctx.set_font_size(12.0);
                    ctx.set_source_rgba(fg_muted[0], fg_muted[1], fg_muted[2], 0.82 * alpha);
                    let sub_str = truncate_str(&row.subtitle, 62);
                    ctx.move_to(rx + 56.0, ry + 42.0);
                    let _ = ctx.show_text(&sub_str);

                    if !row.badge.is_empty() {
                        let badge_w = (row.badge.len() as f64 * 7.0 + 18.0).max(40.0);
                        draw_shortcut_pill(
                            ctx,
                            rx + rw - badge_w - 10.0,
                            ry + (row_h - 22.0) * 0.5,
                            badge_w,
                            22.0,
                            &row.badge,
                            fg_muted,
                            is_dark,
                        );
                    }
                }
            }
        }

        ctx.restore().ok();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_app_icon(
        &mut self,
        ctx: &Context,
        icon_name: &str,
        app_name: &str,
        cx: f64,
        cy: f64,
        size: f64,
        alpha: f64,
    ) {
        let key = format!("{icon_name}@{}", size.round() as i32);
        if !self.icon_cache.contains_key(&key) {
            let loaded = load_icon_surface(icon_name, size.round() as i32);
            self.icon_cache.insert(key.clone(), loaded);
        }
        if let Some(Some(ref surf)) = self.icon_cache.get(&key) {
            let sw = surf.width() as f64;
            let sh = surf.height() as f64;
            if sw > 0.0 && sh > 0.0 {
                ctx.save().ok();
                ctx.translate(cx - size * 0.5, cy - size * 0.5);
                ctx.scale(size / sw, size / sh);
                let _ = ctx.set_source_surface(surf, 0.0, 0.0);
                let _ = ctx.paint_with_alpha(alpha);
                ctx.restore().ok();
                return;
            }
        }

        // Icono vectorial de respaldo con las iniciales de la aplicación
        let hash = app_name
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(u32::from(b)));
        let palette = [
            [0.22, 0.52, 0.96],
            [0.54, 0.34, 0.92],
            [0.18, 0.68, 0.52],
            [0.92, 0.46, 0.22],
            [0.86, 0.28, 0.48],
        ];
        let col = palette[(hash as usize) % palette.len()];
        path_squircle_rect(
            ctx,
            cx - size * 0.44,
            cy - size * 0.44,
            size * 0.88,
            size * 0.88,
            size * 0.24,
        );
        ctx.set_source_rgba(col[0], col[1], col[2], 0.92 * alpha);
        let _ = ctx.fill();

        let initial = app_name
            .chars()
            .find(|c| c.is_alphanumeric())
            .unwrap_or('A')
            .to_uppercase()
            .to_string();
        ctx.select_font_face("Inter", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        ctx.set_font_size(size * 0.42);
        ctx.set_source_rgba(1.0, 1.0, 1.0, 0.96 * alpha);
        if let Ok(ext) = ctx.text_extents(&initial) {
            ctx.move_to(cx - ext.width() * 0.5, cy + ext.height() * 0.36);
            let _ = ctx.show_text(&initial);
        }
    }
}

trait CairoDestOutExt {
    fn destination_out_operator() -> Operator;
}

impl CairoDestOutExt for Operator {
    fn destination_out_operator() -> Operator {
        Operator::DestOut
    }
}

fn path_rounded_rect(ctx: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let right = x + w;
    let bottom = y + h;
    ctx.new_sub_path();
    ctx.arc(right - r, y + r, r, -FRAC_PI_2, 0.0);
    ctx.arc(right - r, bottom - r, r, 0.0, FRAC_PI_2);
    ctx.arc(x + r, bottom - r, r, FRAC_PI_2, PI);
    ctx.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    ctx.close_path();
}

/// Traza un rectángulo con esquinas superelípticas (squircle macOS continuo).
/// Cuando `exponent == 2.0` equivale a arcos circulares; cuando `exponent == 4.2` produce un squircle puro.
fn path_superellipse_rect(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    exponent: f64,
) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.0 {
        ctx.rectangle(x, y, w.max(0.0), h.max(0.0));
        return;
    }
    if (exponent - 2.0).abs() < 0.02 {
        path_rounded_rect(ctx, x, y, w, h, r);
        return;
    }

    let power = 2.0 / exponent.max(2.0);
    let steps = 24usize;
    let right = x + w;
    let bottom = y + h;

    ctx.new_sub_path();
    // Esquina superior derecha: (right - r, y) -> (right, y + r)
    for i in 0..=steps {
        let t = (i as f64 / steps as f64) * FRAC_PI_2;
        let px = (right - r) + r * t.sin().powf(power);
        let py = (y + r) - r * t.cos().powf(power);
        if i == 0 {
            ctx.move_to(px, py);
        } else {
            ctx.line_to(px, py);
        }
    }
    // Esquina inferior derecha: (right, bottom - r) -> (right - r, bottom)
    for i in 0..=steps {
        let t = (i as f64 / steps as f64) * FRAC_PI_2;
        let px = (right - r) + r * t.cos().powf(power);
        let py = (bottom - r) + r * t.sin().powf(power);
        ctx.line_to(px, py);
    }
    // Esquina inferior izquierda: (x + r, bottom) -> (x, bottom - r)
    for i in 0..=steps {
        let t = (i as f64 / steps as f64) * FRAC_PI_2;
        let px = (x + r) - r * t.sin().powf(power);
        let py = (bottom - r) + r * t.cos().powf(power);
        ctx.line_to(px, py);
    }
    // Esquina superior izquierda: (x, y + r) -> (x + r, y)
    for i in 0..=steps {
        let t = (i as f64 / steps as f64) * FRAC_PI_2;
        let px = (x + r) - r * t.cos().powf(power);
        let py = (y + r) - r * t.sin().powf(power);
        ctx.line_to(px, py);
    }
    ctx.close_path();
}

fn path_squircle_rect(ctx: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    path_superellipse_rect(ctx, x, y, w, h, r, SQUIRCLE_EXPONENT);
}

/// Traza la silueta principal de Spotlight interpolando suavemente desde cápsula semicircular
/// (`expand_t == 0.0`, `exponent = 2.0`) hasta tarjeta squircle macOS (`expand_t > 0.0`, `exponent = 4.2`).
fn path_card_shape(ctx: &Context, x: f64, y: f64, w: f64, h: f64, r: f64, expand_t: f64) {
    let t = expand_t.clamp(0.0, 1.0);
    let exponent = 2.0 + (SQUIRCLE_EXPONENT - 2.0) * t;
    path_superellipse_rect(ctx, x, y, w, h, r, exponent);
}

#[allow(clippy::too_many_arguments)]
fn draw_soft_drop_shadow(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    expand_t: f64,
    is_dark: bool,
) {
    let base_alpha = if is_dark { 0.055 } else { 0.036 };
    for i in 1..=5 {
        let spread = i as f64 * 3.2;
        let offset_y = i as f64 * 1.4;
        path_card_shape(
            ctx,
            x - spread,
            y - spread * 0.65 + offset_y,
            w + spread * 2.0,
            h + spread * 1.65,
            r + spread,
            expand_t,
        );
        ctx.set_source_rgba(0.0, 0.0, 0.0, base_alpha * (6 - i) as f64 / 5.0);
        let _ = ctx.fill();
    }
}

fn draw_soft_circle_shadow(ctx: &Context, cx: f64, cy: f64, r: f64, vis: f64, is_dark: bool) {
    let base_alpha = (if is_dark { 0.05 } else { 0.032 }) * vis;
    for i in 1..=4 {
        let spread = i as f64 * 2.6;
        ctx.new_sub_path();
        ctx.arc(cx, cy + i as f64 * 1.2, r + spread, 0.0, TAU);
        ctx.set_source_rgba(0.0, 0.0, 0.0, base_alpha * (5 - i) as f64 / 4.0);
        let _ = ctx.fill();
    }
}

fn draw_magnifying_glass(ctx: &Context, cx: f64, cy: f64, col: [f64; 3], alpha: f64) {
    ctx.save().ok();
    ctx.set_source_rgba(col[0], col[1], col[2], alpha);
    ctx.set_line_width(2.0);
    ctx.set_line_cap(cairo::LineCap::Round);
    ctx.new_sub_path();
    ctx.arc(cx - 1.5, cy - 1.5, 6.5, 0.0, TAU);
    let _ = ctx.stroke();
    ctx.move_to(cx + 3.4, cy + 3.4);
    ctx.line_to(cx + 8.0, cy + 8.0);
    let _ = ctx.stroke();
    ctx.restore().ok();
}

fn draw_mode_button_icon(ctx: &Context, index: usize, cx: f64, cy: f64, col: [f64; 3], alpha: f64) {
    ctx.save().ok();
    ctx.set_source_rgba(col[0], col[1], col[2], 0.92 * alpha);
    ctx.set_line_width(1.7);
    ctx.set_line_cap(cairo::LineCap::Round);
    ctx.set_line_join(cairo::LineJoin::Round);

    match index {
        0 => {
            // Applications: cuadrícula de 4 squircles
            for (dx, dy) in [(-4.8, -4.8), (4.8, -4.8), (-4.8, 4.8), (4.8, 4.8)] {
                path_rounded_rect(ctx, cx + dx - 3.3, cy + dy - 3.3, 6.6, 6.6, 2.0);
                let _ = ctx.stroke();
            }
        }
        1 => {
            // Files: icono de carpeta macOS
            path_rounded_rect(ctx, cx - 8.5, cy - 5.5, 17.0, 12.0, 2.5);
            let _ = ctx.stroke();
            ctx.move_to(cx - 8.0, cy - 3.0);
            ctx.line_to(cx - 2.0, cy - 3.0);
            ctx.line_to(cx, cy - 5.5);
            let _ = ctx.stroke();
        }
        2 => {
            // Actions / Shortcuts: rayo / comando rápido
            ctx.new_path();
            ctx.move_to(cx + 1.5, cy - 8.0);
            ctx.line_to(cx - 5.5, cy + 1.0);
            ctx.line_to(cx + 0.5, cy + 1.0);
            ctx.line_to(cx - 1.5, cy + 8.0);
            ctx.line_to(cx + 5.5, cy - 1.0);
            ctx.line_to(cx - 0.5, cy - 1.0);
            ctx.close_path();
            let _ = ctx.stroke();
        }
        _ => {
            // Clipboard: portapapeles con clip superior
            path_rounded_rect(ctx, cx - 6.5, cy - 6.5, 13.0, 14.5, 2.4);
            let _ = ctx.stroke();
            path_rounded_rect(ctx, cx - 3.5, cy - 8.2, 7.0, 3.2, 1.2);
            let _ = ctx.fill();
        }
    }
    ctx.restore().ok();
}

#[allow(clippy::too_many_arguments)]
fn draw_shortcut_pill(
    ctx: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    text: &str,
    col: [f64; 3],
    is_dark: bool,
) {
    path_squircle_rect(ctx, x, y, w, h, h * 0.38);
    ctx.set_source_rgba(col[0], col[1], col[2], if is_dark { 0.16 } else { 0.14 });
    let _ = ctx.fill();

    ctx.select_font_face("Inter", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    ctx.set_font_size(11.5);
    ctx.set_source_rgba(col[0], col[1], col[2], 0.92);
    if let Ok(ext) = ctx.text_extents(text) {
        ctx.move_to(x + (w - ext.width()) * 0.5, y + h * 0.5 + 4.2);
        let _ = ctx.show_text(text);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_template_pill(
    ctx: &Context,
    x: f64,
    center_y: f64,
    text: &str,
    has_value: bool,
    is_focused: bool,
    fg_main: [f64; 3],
    is_dark: bool,
) -> f64 {
    ctx.set_font_size(14.5);
    let text_w = ctx.text_extents(text).map(|e| e.width()).unwrap_or(50.0);
    let pill_w = (text_w + 20.0).max(68.0);
    let pill_h = 28.0;
    let pill_y = center_y - pill_h * 0.5;

    path_squircle_rect(ctx, x, pill_y, pill_w, pill_h, 9.0);
    if is_focused {
        ctx.set_source_rgba(0.24, 0.53, 0.97, if is_dark { 0.34 } else { 0.22 });
    } else {
        ctx.set_source_rgba(fg_main[0], fg_main[1], fg_main[2], 0.12);
    }
    let _ = ctx.fill_preserve();
    ctx.set_line_width(1.2);
    ctx.set_source_rgba(0.28, 0.58, 1.0, if is_focused { 0.85 } else { 0.25 });
    let _ = ctx.stroke();

    ctx.set_source_rgba(
        fg_main[0],
        fg_main[1],
        fg_main[2],
        if has_value { 0.96 } else { 0.58 },
    );
    ctx.move_to(x + 10.0, center_y + 5.0);
    let _ = ctx.show_text(text);

    x + pill_w
}

fn draw_badge_circle(ctx: &Context, cx: f64, cy: f64, r: f64, rgb: [f64; 3], alpha: f64) {
    ctx.new_sub_path();
    ctx.arc(cx, cy, r, 0.0, TAU);
    ctx.set_source_rgba(rgb[0], rgb[1], rgb[2], 0.88 * alpha);
    let _ = ctx.fill();
}

fn draw_empty_state(
    ctx: &Context,
    cx: f64,
    cy: f64,
    message: &str,
    fg_muted: [f64; 3],
    alpha: f64,
) {
    ctx.select_font_face("Inter", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    ctx.set_font_size(15.0);
    ctx.set_source_rgba(fg_muted[0], fg_muted[1], fg_muted[2], 0.78 * alpha);
    if let Ok(ext) = ctx.text_extents(message) {
        ctx.move_to(cx - ext.width() * 0.5, cy);
        let _ = ctx.show_text(message);
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn load_icon_surface(icon_name: &str, target_size: i32) -> Option<ImageSurface> {
    if icon_name.is_empty() {
        return None;
    }
    let path = resolve_icon_path(icon_name)?;
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, target_size, target_size, true).ok()?;
    let w = pixbuf.width();
    let h = pixbuf.height();
    let stride = pixbuf.rowstride() as usize;
    let n_channels = pixbuf.n_channels() as usize;
    let pixels = unsafe { pixbuf.pixels() };

    let mut surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    let dst_stride = surface.stride() as usize;
    {
        let mut dst = surface.data().ok()?;
        for y in 0..h as usize {
            let src_row = &pixels[y * stride..];
            let dst_row = &mut dst[y * dst_stride..];
            for x in 0..w as usize {
                let r = src_row[x * n_channels] as u32;
                let g = src_row[x * n_channels + 1] as u32;
                let b = src_row[x * n_channels + 2] as u32;
                let a = if n_channels >= 4 {
                    src_row[x * n_channels + 3] as u32
                } else {
                    255
                };
                let pr = (r * a + 127) / 255;
                let pg = (g * a + 127) / 255;
                let pb = (b * a + 127) / 255;
                let di = x * 4;
                dst_row[di] = pb as u8;
                dst_row[di + 1] = pg as u8;
                dst_row[di + 2] = pr as u8;
                dst_row[di + 3] = a as u8;
            }
        }
    }
    surface.flush();
    Some(surface)
}

fn resolve_icon_path(icon_name: &str) -> Option<PathBuf> {
    let direct = Path::new(icon_name);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct.to_path_buf());
    }

    let mut bases = vec![
        PathBuf::from("/usr/share/icons/hicolor"),
        PathBuf::from("/usr/share/icons/Adwaita"),
        PathBuf::from("/var/lib/flatpak/exports/share/icons/hicolor"),
        PathBuf::from("/usr/local/share/icons/hicolor"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        bases.insert(0, home.join(".local/share/icons/hicolor"));
        bases.insert(
            1,
            home.join(".local/share/flatpak/exports/share/icons/hicolor"),
        );
    }

    let subdirs = [
        "scalable/apps",
        "128x128/apps",
        "64x64/apps",
        "48x48/apps",
        "256x256/apps",
    ];
    for base in &bases {
        for sub in &subdirs {
            for ext in ["svg", "png"] {
                let candidate = base.join(sub).join(format!("{icon_name}.{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    for ext in ["svg", "png"] {
        let pixmap = PathBuf::from(format!("/usr/share/pixmaps/{icon_name}.{ext}"));
        if pixmap.is_file() {
            return Some(pixmap);
        }
    }
    None
}

fn builtin_actions() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    ResultExecuteAction,
)> {
    let es = is_spanish_session();
    if es {
        vec![
            (
                "Crear nota",
                "Crear una nota rápida con contenido en ~/Notes",
                "quick_note",
                &["note", "nota", "crear nota", "create note", "apuntes"],
                ResultExecuteAction::OpenTemplate {
                    action_id: "create_note",
                    label_prefix: "Crear nota",
                    field1_placeholder: "Contenido",
                    default_field2: "Notes",
                    label_mid: "en",
                    field2_placeholder: "Carpeta",
                },
            ),
            (
                "Ejecutar comando",
                "Ejecutar un comando de shell directamente desde Spotlight",
                "desktop",
                &["run", "command", "terminal", "ejecutar", "comando"],
                ResultExecuteAction::OpenTemplate {
                    action_id: "run_cmd",
                    label_prefix: "Ejecutar",
                    field1_placeholder: "Comando",
                    default_field2: "Segundo plano",
                    label_mid: "modo",
                    field2_placeholder: "Modo",
                },
            ),
            (
                "Ajustes del Sistema",
                "Abrir Ajustes del Sistema de Malus DE",
                "system",
                &["settings", "ajustes", "configuracion", "configuración", "preferencias", "system"],
                ResultExecuteAction::RunCommand("malus-de-settings".to_string()),
            ),
            (
                "Capturar pantalla",
                "Tomar una captura de la pantalla actual",
                "desktop",
                &["screenshot", "captura", "pantalla", "foto"],
                ResultExecuteAction::RunCommand("malus-de-screenshot".to_string()),
            ),
            (
                "Alternar modo oscuro / claro",
                "Cambiar la apariencia del escritorio entre Liquid Glass oscuro y claro",
                "desktop",
                &["dark", "light", "theme", "tema", "oscuro", "claro", "aspecto"],
                ResultExecuteAction::ToggleTheme,
            ),
            (
                "Bloquear pantalla",
                "Bloquear la sesión actual de Malus DE inmediatamente",
                "system",
                &["lock", "bloquear", "pantalla"],
                ResultExecuteAction::RunCommand("malus-de-session-lock".to_string()),
            ),
            (
                "Cerrar sesión",
                "Cerrar la sesión actual de Malus DE",
                "system",
                &["logout", "log out", "cerrar sesion", "cerrar sesión", "salir"],
                ResultExecuteAction::SystemLogout,
            ),
            (
                "Reiniciar",
                "Reiniciar el equipo",
                "system",
                &["restart", "reboot", "reiniciar"],
                ResultExecuteAction::SystemReboot,
            ),
            (
                "Apagar",
                "Apagar el equipo",
                "system",
                &["shutdown", "shut down", "poweroff", "apagar"],
                ResultExecuteAction::SystemPowerOff,
            ),
        ]
    } else {
        vec![
            (
                "Create Note",
                "Create a quick note with title and content in ~/Notes",
                "quick_note",
                &["note", "nota", "crear nota", "create note", "apuntes"],
                ResultExecuteAction::OpenTemplate {
                    action_id: "create_note",
                    label_prefix: "Create note",
                    field1_placeholder: "Contents",
                    default_field2: "Notes",
                    label_mid: "in",
                    field2_placeholder: "Folder",
                },
            ),
            (
                "Run Command",
                "Execute a shell command directly from Spotlight",
                "desktop",
                &["run", "command", "terminal", "ejecutar", "comando"],
                ResultExecuteAction::OpenTemplate {
                    action_id: "run_cmd",
                    label_prefix: "Run",
                    field1_placeholder: "Command",
                    default_field2: "Background",
                    label_mid: "mode",
                    field2_placeholder: "Mode",
                },
            ),
            (
                "System Settings",
                "Open Malus DE System Settings",
                "system",
                &["settings", "ajustes", "configuracion", "preferencias", "system"],
                ResultExecuteAction::RunCommand("malus-de-settings".to_string()),
            ),
            (
                "Take Screenshot",
                "Capture screenshot of the current screen",
                "desktop",
                &["screenshot", "captura", "pantalla", "foto"],
                ResultExecuteAction::RunCommand("malus-de-screenshot".to_string()),
            ),
            (
                "Toggle Dark / Light Mode",
                "Switch desktop appearance between Dark and Light Liquid Glass",
                "desktop",
                &["dark", "light", "theme", "tema", "oscuro", "claro", "aspecto"],
                ResultExecuteAction::ToggleTheme,
            ),
            (
                "Lock Screen",
                "Lock the current Malus DE session immediately",
                "system",
                &["lock", "bloquear", "pantalla"],
                ResultExecuteAction::RunCommand("malus-de-session-lock".to_string()),
            ),
            (
                "Log Out",
                "Close current Malus DE session",
                "system",
                &["logout", "log out", "cerrar sesion", "cerrar sesión", "salir"],
                ResultExecuteAction::SystemLogout,
            ),
            (
                "Restart",
                "Reboot the computer",
                "system",
                &["restart", "reboot", "reiniciar"],
                ResultExecuteAction::SystemReboot,
            ),
            (
                "Shut Down",
                "Power off the computer",
                "system",
                &["shutdown", "shut down", "poweroff", "apagar"],
                ResultExecuteAction::SystemPowerOff,
            ),
        ]
    }
}

fn execute_template_action(tpl: &ActionTemplateState) {
    match tpl.action_id {
        "create_note" => {
            if tpl.field1_value.trim().is_empty() {
                return;
            }
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            let folder_name = if tpl.field2_value.trim().is_empty() {
                "Notes"
            } else {
                tpl.field2_value.trim()
            };
            let dir = home.join(folder_name);
            let _ = fs::create_dir_all(&dir);
            let file_path = dir.join("Spotlight-Notes.md");
            let entry = format!("- {}\n", tpl.field1_value.trim());
            let mut existing = fs::read_to_string(&file_path).unwrap_or_default();
            existing.push_str(&entry);
            let _ = fs::write(&file_path, existing);
            info!("📝 Nota rápida guardada en {:?}", file_path);
        }
        "run_cmd" => {
            let cmd = tpl.field1_value.trim();
            if !cmd.is_empty() {
                spawn_shell_command(cmd);
            }
        }
        _ => {}
    }
}

fn toggle_desktop_theme() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let keyfile_path = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config/malus-de"))
        .join("glib-2.0/settings/keyfile");

    let current = fs::read_to_string(&keyfile_path).unwrap_or_default();
    let is_currently_dark = !current.contains("prefer-light");
    let next_scheme = if is_currently_dark {
        "prefer-light"
    } else {
        "prefer-dark"
    };
    let next_theme = if is_currently_dark {
        "Adwaita"
    } else {
        "Adwaita-dark"
    };

    if let Some(parent) = keyfile_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let updated = format!(
        "[org/gnome/desktop/interface]\ncolor-scheme='{next_scheme}'\ngtk-theme='{next_theme}'\n"
    );
    let _ = fs::write(&keyfile_path, updated);
    let _ = malus_de_common::ipc::IpcClient::send_command(
        malus_de_common::ipc::IpcMessage::ThemeChanged {
            theme_name: next_theme.to_string(),
            is_dark: !is_currently_dark,
        },
    );
}

fn copy_to_clipboard(text: &str) {
    if let Ok(mut child) = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = std::io::Write::write_all(&mut stdin, text.as_bytes());
        }
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn open_file_path(path: &Path) {
    let mut cmd = Command::new("xdg-open");
    cmd.arg(path);
    apply_malus_app_env(&mut cmd);
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn launch_desktop_app(desktop_file: &str, fallback_exec: &str) {
    if Path::new(desktop_file)
        .file_name()
        .is_some_and(|name| name == "io.mpv.Mpv.desktop")
    {
        spawn_shell_command(
            "flatpak run io.mpv.Mpv --gpu-context=wayland --player-operation-mode=pseudo-gui",
        );
        return;
    }

    // Evitar gio::DesktopAppInfo::launch porque para aplicaciones con DBusActivatable=true
    // (como las de GNOME) activa el servicio vía D-Bus / systemd --user, donde XDG_CONFIG_HOME
    // y GSETTINGS_BACKEND no están aislados, haciendo que carguen los ajustes y botones de GNOME.
    if !fallback_exec.trim().is_empty() {
        spawn_shell_command(fallback_exec);
        return;
    }

    if let Some(app_info) = gio::DesktopAppInfo::from_filename(desktop_file) {
        if let Some(cmd_line) = app_info.commandline() {
            let clean = cmd_line
                .to_string_lossy()
                .split_whitespace()
                .filter(|part| !part.starts_with('%'))
                .collect::<Vec<_>>()
                .join(" ");
            if !clean.is_empty() {
                spawn_shell_command(&clean);
                return;
            }
        }
        let _ = app_info.launch(&[], None::<&gio::AppLaunchContext>);
    }
}

fn apply_malus_app_env(command: &mut Command) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".config/malus-de"));
    malus_de_common::config::set_cursor_environment(command);
    command.env("XDG_CONFIG_HOME", malus_config_dir);
    command.env("GSETTINGS_BACKEND", "keyfile");
    command.env("GTK_USE_PORTAL", "1");
    command.env("XDG_CURRENT_DESKTOP", "Malus:GNOME");
    command.env("GDK_BACKEND", "wayland");
    if let Ok(wayland_disp) = std::env::var("WAYLAND_DISPLAY") {
        command.env("WAYLAND_DISPLAY", wayland_disp);
    }
    if let Ok(xdg_rt) = std::env::var("XDG_RUNTIME_DIR") {
        command.env("XDG_RUNTIME_DIR", xdg_rt);
    }
}

fn spawn_shell_command(command_line: &str) {
    let parts: Vec<&str> = command_line.split_whitespace().collect();
    let Some((exe, args)) = parts.split_first() else {
        return;
    };

    let target_exe = if *exe == "gnome-control-center" {
        "malus-de-settings"
    } else {
        *exe
    };

    let local_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(target_exe)))
        .filter(|p| p.is_file());

    let mut command = if let Some(bin) = local_bin {
        Command::new(bin)
    } else {
        Command::new(target_exe)
    };
    command.args(args);
    apply_malus_app_env(&mut command);
    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

pub fn index_applications() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS");
    let dirs = application_dirs_from(
        home.as_deref(),
        xdg_data_home.as_deref(),
        xdg_data_dirs.as_deref(),
    );

    for dir in &dirs {
        if dir.exists() {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                        if let Some(app) = parse_desktop_file(&path) {
                            apps.push(app);
                        }
                    }
                }
            }
        }
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

fn application_dirs_from(
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    xdg_data_dirs: Option<&OsStr>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let user_data_home = xdg_data_home
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|path| path.join(".local/share")));

    if let Some(data_home) = user_data_home {
        dirs.push(data_home.join("applications"));
    }
    if let Some(home) = home {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }

    let mut data_dirs: Vec<PathBuf> = xdg_data_dirs
        .filter(|dirs| !dirs.is_empty())
        .map(std::env::split_paths)
        .map(Iterator::collect)
        .unwrap_or_default();
    data_dirs.retain(|path| path.is_absolute());
    if data_dirs.is_empty() {
        data_dirs = vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ];
    }
    dirs.extend(data_dirs.into_iter().map(|path| path.join("applications")));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    let mut unique_dirs = Vec::with_capacity(dirs.len());
    for dir in dirs {
        if !unique_dirs.contains(&dir) {
            unique_dirs.push(dir);
        }
    }
    unique_dirs
}

fn parse_desktop_file(path: &Path) -> Option<AppEntry> {
    let content = fs::read_to_string(path).ok()?;
    let (full_locale, lang_code) = session_locale_codes();
    let name_full_key = format!("Name[{full_locale}]");
    let name_lang_key = format!("Name[{lang_code}]");
    let comment_full_key = format!("Comment[{full_locale}]");
    let comment_lang_key = format!("Comment[{lang_code}]");
    let keywords_full_key = format!("Keywords[{full_locale}]");
    let keywords_lang_key = format!("Keywords[{lang_code}]");

    let mut in_desktop_entry = false;
    let mut name_default = None;
    let mut name_lang = None;
    let mut name_full = None;
    let mut exec = None;
    let mut icon = None;
    let mut comment_default = None;
    let mut comment_lang = None;
    let mut comment_full = None;
    let mut categories = String::new();
    let mut search_terms = Vec::new();
    let mut nodisplay = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let mut parts = line.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        if key == "Name" && name_default.is_none() {
            name_default = Some(value.to_string());
            search_terms.push(value.to_lowercase());
        } else if key == name_full_key {
            name_full = Some(value.to_string());
            search_terms.push(value.to_lowercase());
        } else if key == name_lang_key {
            name_lang = Some(value.to_string());
            search_terms.push(value.to_lowercase());
        } else if key == "Exec" && exec.is_none() {
            let clean = value
                .split_whitespace()
                .filter(|part| !part.starts_with('%'))
                .collect::<Vec<_>>()
                .join(" ");
            exec = Some(clean);
        } else if key == "Icon" && icon.is_none() {
            icon = Some(value.to_string());
        } else if key == "Comment" && comment_default.is_none() {
            comment_default = Some(value.to_string());
        } else if key == comment_full_key {
            comment_full = Some(value.to_string());
        } else if key == comment_lang_key {
            comment_lang = Some(value.to_string());
        } else if key == "Keywords" || key == keywords_full_key || key == keywords_lang_key {
            search_terms.push(value.to_lowercase());
        } else if key == "Categories" {
            categories = value.to_string();
        } else if key == "NoDisplay" {
            nodisplay = value.eq_ignore_ascii_case("true");
        }
    }

    if nodisplay {
        return None;
    }

    let name = name_full.or(name_lang).or(name_default)?;
    let comment = comment_full.or(comment_lang).or(comment_default).unwrap_or_default();

    Some(AppEntry {
        name,
        exec: exec?,
        desktop_file: path.to_string_lossy().into_owned(),
        icon: icon.unwrap_or_else(|| "application-x-executable".to_string()),
        comment,
        categories,
        search_terms: search_terms.join(" "),
    })
}

fn index_files() -> Vec<FileEntry> {
    let mut files = Vec::new();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return files;
    };

    let candidate_dirs = [
        home.join("Descargas"),
        home.join("Downloads"),
        home.join("Documentos"),
        home.join("Documents"),
        home.join("Escritorio"),
        home.join("Desktop"),
        home.join("Imágenes"),
        home.join("Pictures"),
        home.join("Proyectos"),
    ];

    let home_str = home.to_string_lossy();
    for dir in &candidate_dirs {
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten().take(40) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if path.is_file() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("file")
                    .to_lowercase();
                let modified_secs = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let full = path.to_string_lossy();
                let display_path = if full.starts_with(home_str.as_ref()) {
                    format!("~{}", &full[home_str.len()..])
                } else {
                    full.into_owned()
                };
                files.push(FileEntry {
                    name: name.to_string(),
                    path,
                    display_path,
                    extension: ext,
                    modified_secs,
                });
            }
        }
    }

    files.sort_by_key(|f| std::cmp::Reverse(f.modified_secs));
    files.dedup_by(|a, b| a.path == b.path);
    files.truncate(60);
    files
}

fn load_clipboard_items() -> Vec<(String, String)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let path = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("malus-de/clipboard-history.dat");

    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(mut parsed) = serde_json::from_str::<PersistedClipboardHistory>(&raw) else {
        return Vec::new();
    };
    parsed
        .entries
        .sort_by_key(|e| std::cmp::Reverse(e.copied_at_unix));
    parsed
        .entries
        .into_iter()
        .filter_map(|e| {
            let full = e.text.unwrap_or_else(|| e.preview.clone());
            let clean = e.preview.trim().replace('\n', " ");
            (!clean.is_empty()).then_some((clean, full))
        })
        .collect()
}

pub fn try_calculate(input: &str) -> Option<String> {
    let clean = input.trim();
    if clean.is_empty() || !clean.chars().any(|c| "+-*/".contains(c)) {
        return None;
    }
    if !clean
        .chars()
        .all(|c| c.is_ascii_digit() || "+-*/. ()".contains(c))
    {
        return None;
    }
    eval_expression(clean).map(|res| {
        if res.fract().abs() < 1e-9 {
            format!("{}", res as i64)
        } else {
            format!("{:.4}", res)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        }
    })
}

fn eval_expression(expr: &str) -> Option<f64> {
    let expr = expr.trim();
    if expr.starts_with('(') && expr.ends_with(')') && is_wrapped_in_parens(expr) {
        return eval_expression(&expr[1..expr.len() - 1]);
    }
    if let Some(idx) = find_outer_op(expr, '+') {
        let (l, r) = expr.split_at(idx);
        return Some(eval_expression(l)? + eval_expression(&r[1..])?);
    }
    if let Some(idx) = find_outer_sub(expr) {
        let (l, r) = expr.split_at(idx);
        return Some(eval_expression(l)? - eval_expression(&r[1..])?);
    }
    if let Some(idx) = find_outer_op(expr, '*') {
        let (l, r) = expr.split_at(idx);
        return Some(eval_expression(l)? * eval_expression(&r[1..])?);
    }
    if let Some(idx) = find_outer_op(expr, '/') {
        let (l, r) = expr.split_at(idx);
        let d = eval_expression(&r[1..])?;
        if d.abs() < f64::EPSILON {
            return None;
        }
        return Some(eval_expression(l)? / d);
    }
    expr.parse::<f64>().ok()
}

fn is_wrapped_in_parens(s: &str) -> bool {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i + 1 < s.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn find_outer_op(s: &str, target: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => depth -= 1,
            _ if depth == 0 && c == target => return Some(i),
            _ => {}
        }
    }
    None
}

fn find_outer_sub(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => depth -= 1,
            '-' if depth == 0 && i > 0 => {
                let prev = s[..i].trim_end();
                if !prev.ends_with(['+', '-', '*', '/']) {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn keysym_to_char(sym: Keysym) -> Option<char> {
    if let Some(ch) = sym.key_char() {
        if !ch.is_control() {
            return Some(ch);
        }
    }
    let raw = sym.raw();
    if (0x20..=0x7e).contains(&raw) {
        return char::from_u32(raw);
    }
    None
}

pub fn create_buffer() -> MemoryRenderBuffer {
    MemoryRenderBuffer::new(
        Fourcc::Argb8888,
        Size::from((WIDTH, HEIGHT)),
        1,
        smithay::utils::Transform::Normal,
        None,
    )
}

pub fn render_controls(
    buffer: &mut MemoryRenderBuffer,
    progress: f64,
    visible_buttons: [bool; 2],
    background_rgb: [f32; 3],
) -> Result<(), String> {
    let mut surface = ImageSurface::create(Format::ARgb32, WIDTH, HEIGHT)
        .map_err(|error| format!("No se pudo crear el buffer Cairo de Spotlight: {error}"))?;
    let context = Context::new(&surface)
        .map_err(|error| format!("No se pudo crear el contexto Cairo de Spotlight: {error}"))?;
    context.set_operator(Operator::Source);
    context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    context.paint().map_err(|error| error.to_string())?;
    context.set_operator(Operator::Over);

    let (background, foreground) = if background_rgb[0] < 0.5 {
        (
            [
                background_rgb[0] as f64,
                background_rgb[1] as f64,
                background_rgb[2] as f64,
            ],
            [0.93, 0.93, 0.94],
        )
    } else {
        (
            [
                background_rgb[0] as f64,
                background_rgb[1] as f64,
                background_rgb[2] as f64,
            ],
            [0.18, 0.18, 0.19],
        )
    };
    let progress = progress.clamp(0.0, 1.0);
    let neck_height = BUTTON_SIZE * (1.0 - progress);
    let center_y = HEIGHT as f64 / 2.0;
    let radius = BUTTON_SIZE / 2.0;
    draw_capsule(
        &context,
        GUTTER,
        center_y - radius,
        SEARCH_WIDTH,
        BUTTON_SIZE,
        radius,
        background,
        foreground,
    )?;

    let button_positions: Vec<(usize, f64)> = visible_buttons
        .into_iter()
        .enumerate()
        .filter_map(|(index, visible)| {
            visible.then_some((
                index,
                GUTTER + SEARCH_WIDTH + GAP * (index as f64 + 1.0) + BUTTON_SIZE * index as f64,
            ))
        })
        .collect();
    for (_, button_x) in &button_positions {
        draw_circle(
            &context,
            *button_x + radius,
            center_y,
            radius,
            background,
            foreground,
            1.0,
        )?;
    }
    if neck_height > 0.0 {
        let half_neck = neck_height / 2.0;
        let tangent_offset = (radius.powi(2) - half_neck.powi(2)).max(0.0).sqrt();
        let mut previous_center = GUTTER + SEARCH_WIDTH - radius;
        for (_, button_x) in button_positions {
            let button_center = button_x + radius;
            draw_liquid_bridge(
                &context,
                previous_center + tangent_offset,
                button_center - tangent_offset,
                center_y,
                neck_height,
                background,
            )?;
            previous_center = button_center;
        }
    }

    drop(context);
    surface.flush();
    let pixels = surface
        .data()
        .map_err(|error| format!("No se pudo leer el buffer Cairo de Spotlight: {error}"))?;
    let mut render_context = buffer.render();
    render_context
        .draw(|destination| {
            destination.copy_from_slice(&pixels);
            Ok::<_, ()>(vec![Rectangle::from_size((WIDTH, HEIGHT).into())])
        })
        .map_err(|_| "No se pudo actualizar el buffer visual de Spotlight".to_string())
}

fn draw_liquid_bridge(
    context: &Context,
    left: f64,
    right: f64,
    center_y: f64,
    height: f64,
    background: [f64; 3],
) -> Result<(), String> {
    let half_height = height / 2.0;
    let width = right - left;
    let radius = half_height.min(width / 2.0);
    if radius <= 0.0 || width <= 0.0 {
        return Ok(());
    }
    let top = center_y - radius;
    let bottom = center_y + radius;
    context.new_path();
    context.arc(right - radius, top + radius, radius, -FRAC_PI_2, 0.0);
    context.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    context.arc(left + radius, bottom - radius, radius, FRAC_PI_2, PI);
    context.arc(left + radius, top + radius, radius, PI, 3.0 * FRAC_PI_2);
    context.close_path();
    context.set_source_rgba(background[0], background[1], background[2], 0.42);
    context.fill().map_err(|error| error.to_string())
}

pub fn render_element(
    renderer: &mut GlesRenderer,
    buffer: &MemoryRenderBuffer,
    location: Point<f64, Physical>,
) -> Result<MemoryRenderBufferRenderElement<GlesRenderer>, <GlesRenderer as RendererSuper>::Error> {
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        location,
        buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_capsule(
    context: &Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    background: [f64; 3],
    foreground: [f64; 3],
) -> Result<(), String> {
    let right = x + width;
    let bottom = y + height;
    context.new_sub_path();
    context.arc(right - radius, y + radius, radius, -FRAC_PI_2, 0.0);
    context.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    context.arc(x + radius, bottom - radius, radius, FRAC_PI_2, PI);
    context.arc(x + radius, y + radius, radius, PI, 3.0 * FRAC_PI_2);
    context.close_path();
    let gradient = LinearGradient::new(x, y, x, bottom);
    gradient.add_color_stop_rgba(
        0.0,
        background[0] + 0.05,
        background[1] + 0.05,
        background[2] + 0.05,
        0.42,
    );
    gradient.add_color_stop_rgba(1.0, background[0], background[1], background[2], 0.42);
    context
        .set_source(&gradient)
        .map_err(|error| error.to_string())?;
    context.fill_preserve().map_err(|error| error.to_string())?;
    context.set_line_width(1.0);
    context.set_source_rgba(foreground[0], foreground[1], foreground[2], 0.18);
    context.stroke().map_err(|error| error.to_string())
}

fn draw_circle(
    context: &Context,
    x: f64,
    y: f64,
    radius: f64,
    background: [f64; 3],
    foreground: [f64; 3],
    opacity: f64,
) -> Result<(), String> {
    context.new_sub_path();
    context.arc(x, y, radius, 0.0, TAU);
    context.set_source_rgba(background[0], background[1], background[2], 0.42 * opacity);
    context.fill_preserve().map_err(|error| error.to_string())?;
    context.set_line_width(1.0);
    context.set_source_rgba(foreground[0], foreground[1], foreground[2], 0.14 * opacity);
    context.stroke().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_joined_and_separated_control_shapes() {
        let mut buffer = create_buffer();
        render_controls(&mut buffer, 0.0, [true, true], [36.0 / 255.0; 3]).unwrap();
        render_controls(&mut buffer, 0.5, [true, true], [36.0 / 255.0; 3]).unwrap();
        render_controls(&mut buffer, 1.0, [true, true], [250.0 / 255.0; 3]).unwrap();
    }

    #[test]
    fn renders_only_the_selected_mode_button() {
        let mut buffer = create_buffer();
        render_controls(&mut buffer, 1.0, [true, false], [36.0 / 255.0; 3]).unwrap();
    }

    #[test]
    fn transition_preserves_its_current_progress_when_retargeted() {
        let started_at = Instant::now();
        let transition = SpotlightControlsTransition::new(0.0, 1.0, started_at);
        let midpoint = started_at + SEPARATION_DURATION / 2;
        let current_progress = transition.progress_at(midpoint);
        let returning = SpotlightControlsTransition::new(current_progress, 0.0, midpoint);

        assert_eq!(transition.progress_at(started_at), 0.0);
        assert_eq!(
            transition.progress_at(started_at + SEPARATION_DURATION),
            1.0
        );
        assert_eq!(returning.progress_at(midpoint), current_progress);
    }

    #[test]
    fn spotlight_resets_to_compact_initial_state_when_reopened() {
        let mut spotlight = SpotlightState::new();
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        spotlight.open();
        assert!(spotlight.is_open());
        assert_eq!(spotlight.mode, SpotlightMode::Search);
        assert!(spotlight.query.is_empty());
        assert_eq!(spotlight.hover_transition.value_at(Instant::now()), 0.0);

        // Simular hover sobre la cápsula y seleccionar un modo
        let canvas = spotlight.canvas_logical_rect(output_geo);
        spotlight.update_pointer_motion(
            output_geo,
            Point::from((canvas.loc.x as f64 + 320.0, canvas.loc.y as f64 + 50.0)),
        );
        assert_eq!(spotlight.hover_transition.target(), 1.0);
        spotlight.set_mode(SpotlightMode::Applications, Instant::now());
        spotlight.query.push_str("term");
        assert!(spotlight.is_expanded());

        // Cerrar y volver a abrir debe regresar exactamente a su estado original
        spotlight.close();
        assert!(!spotlight.is_open());
        spotlight.open();
        assert!(spotlight.is_open());
        assert!(!spotlight.is_expanded());
        assert_eq!(spotlight.mode, SpotlightMode::Search);
        assert!(spotlight.query.is_empty());
        assert_eq!(spotlight.hover_transition.value_at(Instant::now()), 0.0);
        assert_eq!(spotlight.hover_transition.target(), 0.0);
        assert_eq!(spotlight.hovered_button, None);
    }

    #[test]
    fn mask_is_strictly_zero_outside_capsule_when_buttons_are_retracted() {
        let mut spotlight = SpotlightState::new();
        let output_geo = Rectangle::new((0, 0).into(), (1920, 1080).into());
        spotlight.open();
        let (_, mask_opt, (mw, mh), _) = spotlight
            .prepare_frame(output_geo, 1.0, true)
            .expect("frame prepared");
        let mask = mask_opt.expect("initial mask");
        assert_eq!((mw, mh), (CANVAS_LOGICAL_W, CANVAS_LOGICAL_H));

        // Esquina superior izquierda (en el margen de sombra fuera de la cápsula): alfa debe ser 0
        assert_eq!(mask[3], 0);
        // Margen derecho fuera de CONTENT_W (x = 695, y = 55): alfa debe ser 0
        let idx_outer_right = ((55 * mw + 695) * 4 + 3) as usize;
        assert_eq!(mask[idx_outer_right], 0);
        // Debajo de la cápsula en estado compacto (x = 350, y = 120): alfa debe ser 0
        let idx_below = ((120 * mw + 350) * 4 + 3) as usize;
        assert_eq!(mask[idx_below], 0);
        // En el estado básico (sin hover), el input es largo (CONTENT_W) y cubre tanto el centro (x = 350) como la derecha (x = 600)
        let idx_center = ((55 * mw + 350) * 4 + 3) as usize;
        assert_eq!(mask[idx_center], 255);
        let idx_right_inside = ((55 * mw + 600) * 4 + 3) as usize;
        assert_eq!(mask[idx_right_inside], 255);
    }

    #[test]
    fn spotlight_splits_on_hover_without_growing_total_width() {
        let spotlight = SpotlightState::new();
        let (x0, _, w0, _, _) = spotlight.capsule_rect_at(0.0, 0.0, CAPSULE_H);
        let (x1, _, w1, _, _) = spotlight.capsule_rect_at(1.0, 0.0, CAPSULE_H);
        let centers_hovered = spotlight.mode_button_centers_at(1.0, 0.0);
        let last_btn_right = centers_hovered[3].0 + centers_hovered[3].2;

        assert_eq!(x0, GUTTER);
        assert_eq!(w0, CONTENT_W);
        assert_eq!(x1, GUTTER);
        assert_eq!(w1, CAPSULE_W);
        // El borde derecho total al dividirse en cápsula + 4 botones sigue siendo exactamente GUTTER + CONTENT_W
        assert!((last_btn_right - (GUTTER + CONTENT_W)).abs() < 1e-6);
    }

    #[test]
    fn spotlight_only_renders_on_target_output() {
        let mut spotlight = SpotlightState::new();
        spotlight.open_on_output(Some("DP-1".to_string()));
        assert!(spotlight.should_render_on_output("DP-1"));
        assert!(!spotlight.should_render_on_output("HDMI-A-1"));
        spotlight.close();
        assert!(!spotlight.should_render_on_output("DP-1"));
    }
}
