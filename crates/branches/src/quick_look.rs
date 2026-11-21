use crate::fs_backend::{open_file, FileItem};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{Key, MemoryFormat, MemoryTexture, ModifierType, Texture};
use gtk4::gio;
use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, Box as GtkBox, Button, ContentFit, EventControllerKey, GestureClick, Grid,
    Image, Label, Orientation, Overlay, Picture, PolicyType, PropagationPhase, ScrolledWindow,
    Separator, Spinner, TextView, Window,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type PageStepCallback = Rc<dyn Fn(i32) -> bool>;

#[derive(Clone)]
pub struct MediaPlaybackHandle {
    stop_flag: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<Child>>>,
}

impl MediaPlaybackHandle {
    fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn register_child(&self, mut child: Child) {
        if self.stop_flag.load(Ordering::SeqCst) {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return;
        }
        if let Ok(mut guard) = self.children.lock() {
            guard.push(child);
        }
    }

    fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let mut drained = Vec::new();
        if let Ok(mut guard) = self.children.lock() {
            for mut child in guard.drain(..) {
                let _ = child.kill();
                drained.push(child);
            }
        }
        if !drained.is_empty() {
            std::thread::spawn(move || {
                for mut child in drained {
                    let _ = child.wait();
                }
            });
        }
    }
}

impl Drop for MediaPlaybackHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct QuickLook {
    window: RefCell<Option<Window>>,
    host_overlay: RefCell<Option<Overlay>>,
    backdrop: RefCell<Option<GtkBox>>,
    container: RefCell<Option<GtkBox>>,
    closing: Cell<bool>,
    anim_gen: Cell<u64>,
    current_path: RefCell<Option<PathBuf>>,
    items_list: RefCell<Vec<FileItem>>,
    current_index: Cell<usize>,
    content_holder: RefCell<Option<GtkBox>>,
    title_label: RefCell<Option<Label>>,
    subtitle_label: RefCell<Option<Label>>,
    open_btn_path: Rc<RefCell<PathBuf>>,
    media_handles: Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: Rc<RefCell<Option<PageStepCallback>>>,
    preview_gen: Rc<Cell<u64>>,
    cache: Arc<Mutex<PreviewCache>>,
}

impl QuickLook {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            window: RefCell::new(None),
            host_overlay: RefCell::new(None),
            backdrop: RefCell::new(None),
            container: RefCell::new(None),
            closing: Cell::new(false),
            anim_gen: Cell::new(0),
            current_path: RefCell::new(None),
            items_list: RefCell::new(Vec::new()),
            current_index: Cell::new(0),
            content_holder: RefCell::new(None),
            title_label: RefCell::new(None),
            subtitle_label: RefCell::new(None),
            open_btn_path: Rc::new(RefCell::new(PathBuf::new())),
            media_handles: Rc::new(RefCell::new(Vec::new())),
            page_step_cb: Rc::new(RefCell::new(None)),
            preview_gen: Rc::new(Cell::new(0)),
            cache: Arc::new(Mutex::new(PreviewCache::new(24))),
        })
    }

    pub fn is_open(&self) -> bool {
        (self.backdrop.borrow().is_some() || self.window.borrow().is_some()) && !self.closing.get()
    }

    pub fn close(self: &Rc<Self>) {
        if self.closing.get() {
            return;
        }
        let has_overlay = self.backdrop.borrow().is_some();
        let has_window = self.window.borrow().is_some();
        if !has_overlay && !has_window {
            return;
        }

        self.closing.set(true);
        let next_gen = self.anim_gen.get().wrapping_add(1);
        self.anim_gen.set(next_gen);
        self.preview_gen.set(self.preview_gen.get().wrapping_add(1));
        self.stop_active_media();

        if let Some(bd) = self.backdrop.borrow().as_ref() {
            bd.remove_css_class("ql-open");
            bd.add_css_class("ql-closing");
        }
        if let Some(card) = self.container.borrow().as_ref() {
            card.remove_css_class("ql-open");
            card.add_css_class("ql-closing");
        }

        let self_c = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(140), move || {
            if self_c.anim_gen.get() != next_gen {
                return;
            }
            self_c.finish_close_immediately();
        });
    }

    fn finish_close_immediately(&self) {
        self.stop_active_media();
        if let (Some(overlay), Some(bd)) = (
            self.host_overlay.borrow_mut().take(),
            self.backdrop.borrow_mut().take(),
        ) {
            overlay.remove_overlay(&bd);
        } else {
            *self.host_overlay.borrow_mut() = None;
            *self.backdrop.borrow_mut() = None;
        }
        *self.container.borrow_mut() = None;
        let win_opt = self.window.borrow_mut().take();
        self.clear_refs();
        self.closing.set(false);
        if let Some(win) = win_opt {
            win.close();
        }
    }

    #[allow(dead_code)]
    pub fn toggle(self: &Rc<Self>, overlay: &Overlay, item: Option<FileItem>) {
        let items = item.iter().cloned().collect::<Vec<_>>();
        self.toggle_in_overlay(overlay, item, items);
    }

    pub fn toggle_in_overlay(
        self: &Rc<Self>,
        overlay: &Overlay,
        item: Option<FileItem>,
        items: Vec<FileItem>,
    ) {
        if self.is_open() {
            self.close();
            return;
        }
        // Si seguía en medio de la animación de cierre, limpiar antes de reabrir
        if self.closing.get() {
            let next_gen = self.anim_gen.get().wrapping_add(1);
            self.anim_gen.set(next_gen);
            self.finish_close_immediately();
        }

        let Some(item) = item else { return };

        // Puente con aplicación dedicada `malus-de-preview` (ADR 0007) cuando esté disponible
        if try_external_preview_dbus(&item.path, &items) {
            return;
        }

        let idx = items
            .iter()
            .position(|it| it.path == item.path)
            .unwrap_or(0);
        *self.items_list.borrow_mut() = if items.is_empty() {
            vec![item.clone()]
        } else {
            items
        };
        self.current_index.set(idx);
        self.show_in_overlay(overlay, &item);
        self.prefetch_neighbors();
    }

    /// Precarga silenciosamente en segundo plano el ítem seleccionado antes de pulsar Espacio
    /// (imitando la caché predictiva de Quick Look en macOS Finder).
    pub fn prefetch_item(&self, item: &FileItem) {
        if item.is_dir {
            return;
        }
        let ext = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !needs_background_preview(&ext, &item.kind_str) {
            return;
        }
        spawn_prefetch_into_cache(&self.cache, item.path.clone(), ext, item.kind_str.clone());
    }

    fn prefetch_neighbors(&self) {
        let items = self.items_list.borrow();
        let len = items.len();
        if len <= 1 {
            return;
        }
        let idx = self.current_index.get().min(len - 1);
        let next_idx = (idx + 1) % len;
        let prev_idx = (idx + len - 1) % len;
        if next_idx != idx {
            self.prefetch_item(&items[next_idx]);
        }
        if prev_idx != idx && prev_idx != next_idx {
            self.prefetch_item(&items[prev_idx]);
        }
    }

    fn stop_active_media(&self) {
        for handle in self.media_handles.borrow_mut().drain(..) {
            handle.stop();
        }
    }

    fn clear_refs(&self) {
        self.stop_active_media();
        self.preview_gen.set(self.preview_gen.get().wrapping_add(1));
        *self.page_step_cb.borrow_mut() = None;
        *self.current_path.borrow_mut() = None;
        *self.content_holder.borrow_mut() = None;
        *self.title_label.borrow_mut() = None;
        *self.subtitle_label.borrow_mut() = None;
        self.items_list.borrow_mut().clear();
    }

    pub fn update_if_open(&self, item: &FileItem) {
        if self.is_open() {
            self.stop_active_media();
            *self.page_step_cb.borrow_mut() = None;
            if let Some(holder) = self.content_holder.borrow().as_ref() {
                populate_preview_async(
                    item,
                    holder,
                    &self.media_handles,
                    &self.page_step_cb,
                    &self.preview_gen,
                    &self.cache,
                    0,
                );
            }
            if let Some(tl) = self.title_label.borrow().as_ref() {
                tl.set_label(&item.name);
            }
            if let Some(sl) = self.subtitle_label.borrow().as_ref() {
                sl.set_label(&format!("{} • {}", item.kind_str, item.size_str));
            }
            *self.open_btn_path.borrow_mut() = item.path.clone();
            *self.current_path.borrow_mut() = Some(item.path.clone());
            if let Some(win) = self.window.borrow().as_ref() {
                win.set_title(Some(&item.name));
            }
        }
    }

    pub fn step_item(&self, delta: isize) {
        let len = self.items_list.borrow().len();
        if len <= 1 {
            return;
        }
        let curr = self.current_index.get() as isize;
        let next = (curr + delta).rem_euclid(len as isize) as usize;
        self.current_index.set(next);
        let item = self.items_list.borrow().get(next).cloned();
        if let Some(item) = item {
            self.update_if_open(&item);
            self.prefetch_neighbors();
        }
    }

    pub fn step_pdf_page(&self, delta: i32) -> bool {
        if let Some(cb) = self.page_step_cb.borrow().as_ref() {
            return cb(delta);
        }
        false
    }

    pub fn show_standalone(self: &Rc<Self>, app: &Application, item: &FileItem) {
        let win = Window::builder()
            .application(app)
            .title(&item.name)
            .default_width(820)
            .default_height(600)
            .decorated(false)
            .css_classes(["branches-quicklook-window"])
            .build();
        *self.items_list.borrow_mut() = vec![item.clone()];
        self.current_index.set(0);

        let root_box = self.build_preview_card(item);
        root_box.set_margin_top(18);
        root_box.set_margin_bottom(18);
        root_box.set_margin_start(18);
        root_box.set_margin_end(18);
        win.set_child(Some(&root_box));

        let key_controller = EventControllerKey::new();
        key_controller.set_propagation_phase(PropagationPhase::Capture);
        let self_key = self.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            let has_mod = state.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK);
            if !has_mod && matches!(keyval, Key::space | Key::KP_Space | Key::Escape) {
                self_key.close();
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Left {
                self_key.step_item(-1);
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Right {
                self_key.step_item(1);
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Page_Up && self_key.step_pdf_page(-1) {
                return glib::Propagation::Stop;
            }
            if !has_mod && keyval == Key::Page_Down && self_key.step_pdf_page(1) {
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(key_controller);

        let self_destroy = self.clone();
        win.connect_close_request(move |_| {
            self_destroy.stop_active_media();
            *self_destroy.container.borrow_mut() = None;
            *self_destroy.window.borrow_mut() = None;
            self_destroy.clear_refs();
            self_destroy.closing.set(false);
            glib::Propagation::Proceed
        });

        *self.container.borrow_mut() = Some(root_box.clone());
        *self.window.borrow_mut() = Some(win.clone());
        win.present();
        Self::trigger_macos_enter_animation(&root_box, None);
    }

    fn show_in_overlay(self: &Rc<Self>, overlay: &Overlay, item: &FileItem) {
        let next_gen = self.anim_gen.get().wrapping_add(1);
        self.anim_gen.set(next_gen);
        self.closing.set(false);

        let backdrop = GtkBox::new(Orientation::Vertical, 0);
        backdrop.add_css_class("branches-quicklook-backdrop");
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);

        let card = self.build_preview_card(item);
        let ow = overlay.width();
        let oh = overlay.height();
        let target_w = if ow > 0 {
            ((ow as f64) * 0.80).round() as i32
        } else {
            780
        }
        .clamp(460, 840);
        let target_h = if oh > 0 {
            ((oh as f64) * 0.84).round() as i32
        } else {
            540
        }
        .clamp(360, 620);
        card.set_size_request(target_w, target_h);
        card.set_halign(Align::Center);
        card.set_valign(Align::Center);
        card.set_vexpand(true);
        card.set_margin_top(24);
        card.set_margin_bottom(24);
        card.set_margin_start(24);
        card.set_margin_end(24);

        // Impedir que los clics dentro de la tarjeta cierren Quick Look
        let card_click = GestureClick::new();
        card_click.set_button(0);
        card_click.connect_pressed(|g, _, _, _| {
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        card.add_controller(card_click);

        // Clic fuera de la tarjeta (en el fondo atenuado) cierra con efecto macOS
        let bd_click = GestureClick::new();
        bd_click.set_button(gtk4::gdk::BUTTON_PRIMARY);
        let ql_bd = self.clone();
        bd_click.connect_released(move |_, _, _, _| {
            ql_bd.close();
        });
        backdrop.add_controller(bd_click);

        backdrop.append(&card);
        *self.host_overlay.borrow_mut() = Some(overlay.clone());
        *self.backdrop.borrow_mut() = Some(backdrop.clone());
        *self.container.borrow_mut() = Some(card.clone());

        overlay.add_overlay(&backdrop);
        Self::trigger_macos_enter_animation(&card, Some(&backdrop));
    }

    fn trigger_macos_enter_animation(card: &GtkBox, backdrop: Option<&GtkBox>) {
        let bd_opt = backdrop.cloned();
        card.add_tick_callback(move |widget, _| {
            widget.add_css_class("ql-open");
            if let Some(ref bd) = bd_opt {
                bd.add_css_class("ql-open");
            }
            glib::ControlFlow::Break
        });
    }

    fn build_preview_card(self: &Rc<Self>, item: &FileItem) -> GtkBox {
        let root_box = GtkBox::new(Orientation::Vertical, 0);
        root_box.add_css_class("branches-quicklook-container");

        // Barra superior de Quick Look
        let header_box = GtkBox::new(Orientation::Horizontal, 8);
        header_box.add_css_class("branches-quicklook-header");

        let close_btn = Button::builder()
            .icon_name("window-close-symbolic")
            .has_frame(false)
            .can_focus(false)
            .valign(Align::Center)
            .css_classes(["branches-quicklook-close-btn"])
            .tooltip_text("Cerrar Vista Rápida (Espacio / Esc)")
            .build();
        let ql_close = self.clone();
        close_btn.connect_clicked(move |_| ql_close.close());
        header_box.append(&close_btn);

        let has_neighbors = self.items_list.borrow().len() > 1;
        if has_neighbors {
            let prev_btn = Button::builder()
                .icon_name("go-previous-symbolic")
                .has_frame(false)
                .can_focus(false)
                .valign(Align::Center)
                .tooltip_text("Anterior (Flecha izquierda)")
                .build();
            let ql_prev = self.clone();
            prev_btn.connect_clicked(move |_| ql_prev.step_item(-1));

            let next_btn = Button::builder()
                .icon_name("go-next-symbolic")
                .has_frame(false)
                .can_focus(false)
                .valign(Align::Center)
                .tooltip_text("Siguiente (Flecha derecha)")
                .build();
            let ql_next = self.clone();
            next_btn.connect_clicked(move |_| ql_next.step_item(1));

            header_box.append(&prev_btn);
            header_box.append(&next_btn);
        }

        let title_info = GtkBox::new(Orientation::Vertical, 2);
        title_info.set_hexpand(true);
        title_info.set_valign(Align::Center);

        let title_label = Label::builder()
            .label(&item.name)
            .halign(Align::Start)
            .ellipsize(pango::EllipsizeMode::Middle)
            .css_classes(["bold", "branches-quicklook-title"])
            .build();

        let subtitle_label = Label::builder()
            .label(&format!("{} • {}", item.kind_str, item.size_str))
            .halign(Align::Start)
            .css_classes(["dim-label", "caption"])
            .build();

        title_info.append(&title_label);
        title_info.append(&subtitle_label);
        header_box.append(&title_info);

        *self.open_btn_path.borrow_mut() = item.path.clone();
        let p_for_open = self.open_btn_path.clone();
        let open_btn = Button::builder()
            .label("Abrir con...")
            .can_focus(false)
            .css_classes(["suggested-action", "branches-quicklook-btn"])
            .valign(Align::Center)
            .build();
        open_btn.connect_clicked(move |_| {
            let path = p_for_open.borrow().clone();
            let _ = open_file(&path);
        });
        header_box.append(&open_btn);

        root_box.append(&header_box);

        // Contenedor dinámico de previsualización (carga asíncrona sin bloquear la transición)
        let content_holder = GtkBox::new(Orientation::Vertical, 0);
        content_holder.set_vexpand(true);
        content_holder.set_hexpand(true);
        content_holder.add_css_class("branches-quicklook-content");

        populate_preview_async(
            item,
            &content_holder,
            &self.media_handles,
            &self.page_step_cb,
            &self.preview_gen,
            &self.cache,
            185,
        );
        root_box.append(&content_holder);

        *self.title_label.borrow_mut() = Some(title_label);
        *self.subtitle_label.borrow_mut() = Some(subtitle_label);
        *self.content_holder.borrow_mut() = Some(content_holder);
        *self.current_path.borrow_mut() = Some(item.path.clone());

        root_box
    }
}

/// Intenta delegar la vista previa al servicio dedicado `dev.malus.Preview` (ADR 0007).
fn try_external_preview_dbus(path: &Path, items: &[FileItem]) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let paths: Vec<String> = items
        .iter()
        .map(|it| it.path.to_string_lossy().into_owned())
        .collect();
    let target = path.to_string_lossy().into_owned();
    let args = (target, paths).to_variant();
    conn.call_sync(
        Some("dev.malus.Preview"),
        "/dev/malus/Preview",
        "dev.malus.Preview",
        "TogglePreview",
        Some(&args),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        150,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

pub fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "svg"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "avif"
            | "heic"
            | "jxl"
    )
}

pub fn is_video_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp4"
            | "mkv"
            | "webm"
            | "mov"
            | "avi"
            | "m4v"
            | "wmv"
            | "flv"
            | "ts"
            | "mpg"
            | "mpeg"
            | "3gp"
            | "ogv"
    )
}

pub fn is_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "aac" | "opus" | "wma"
    )
}

fn is_archive_extension(ext: &str) -> bool {
    matches!(ext, "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z")
}

fn is_text_preview_candidate(ext: &str, kind_str: &str) -> bool {
    [
        "rs", "toml", "json", "md", "txt", "sh", "c", "cpp", "h", "py", "js", "ts", "html", "css",
        "yaml", "yml", "xml", "ini", "conf", "log",
    ]
    .iter()
    .any(|e| *e == ext)
        || kind_str.to_lowercase().contains("texto")
        || kind_str.to_lowercase().contains("código")
}

fn needs_background_preview(ext: &str, kind_str: &str) -> bool {
    is_image_extension(ext)
        || ext == "pdf"
        || is_video_extension(ext)
        || is_audio_extension(ext)
        || is_archive_extension(ext)
        || is_text_preview_candidate(ext, kind_str)
}

#[derive(Clone)]
struct DecodedRgbaFrame {
    width: i32,
    height: i32,
    stride: usize,
    bytes: glib::Bytes,
}

impl DecodedRgbaFrame {
    fn to_texture(&self) -> Texture {
        MemoryTexture::new(
            self.width,
            self.height,
            MemoryFormat::R8g8b8a8,
            &self.bytes,
            self.stride,
        )
        .upcast()
    }
}

#[derive(Clone)]
enum PreparedPreview {
    Image {
        frame: Option<DecodedRgbaFrame>,
    },
    Pdf {
        total_pages: u32,
        first_page: Option<DecodedRgbaFrame>,
    },
    Video {
        meta: MediaMetadata,
        poster: Option<DecodedRgbaFrame>,
    },
    Audio {
        meta: MediaMetadata,
    },
    Text(String),
    Generic,
}

type CacheKey = (PathBuf, u64);

struct PreviewCache {
    max_entries: usize,
    entries: VecDeque<(CacheKey, PreparedPreview)>,
    in_flight: HashSet<CacheKey>,
}

impl PreviewCache {
    fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            entries: VecDeque::new(),
            in_flight: HashSet::new(),
        }
    }

    fn get_if_fresh(&mut self, key: &CacheKey) -> Option<PreparedPreview> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        let (k, val) = self.entries.remove(pos)?;
        let out = val.clone();
        self.entries.push_back((k, val));
        Some(out)
    }

    fn insert(&mut self, key: CacheKey, payload: PreparedPreview) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos);
        }
        while self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back((key, payload));
    }

    fn try_begin_fetch(&mut self, key: CacheKey) -> bool {
        if self.entries.iter().any(|(k, _)| k == &key) || self.in_flight.contains(&key) {
            return false;
        }
        self.in_flight.insert(key);
        true
    }

    fn finish_fetch(&mut self, key: CacheKey, payload: PreparedPreview) {
        self.in_flight.remove(&key);
        self.insert(key, payload);
    }
}

fn file_mtime_nanos(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn spawn_prefetch_into_cache(
    cache: &Arc<Mutex<PreviewCache>>,
    path: PathBuf,
    ext: String,
    kind_str: String,
) {
    let key = (path.clone(), file_mtime_nanos(&path));
    {
        let Ok(mut guard) = cache.lock() else {
            return;
        };
        if !guard.try_begin_fetch(key.clone()) {
            return;
        }
    }
    let cache_clone = cache.clone();
    std::thread::spawn(move || {
        let payload = prepare_preview_payload(&path, &ext, &kind_str);
        if let Ok(mut guard) = cache_clone.lock() {
            guard.finish_fetch(key, payload);
        }
    });
}

fn create_loading_placeholder(item: &FileItem) -> gtk4::Widget {
    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_vexpand(true);
    vbox.set_hexpand(true);

    let icon = Image::from_gicon(&item.gicon);
    crate::views::apply_folder_icon_tint(&icon, item);
    icon.set_pixel_size(72);
    vbox.append(&icon);

    let spinner = Spinner::new();
    spinner.set_size_request(24, 24);
    spinner.start();
    vbox.append(&spinner);

    vbox.upcast()
}

fn populate_preview_async(
    item: &FileItem,
    holder: &GtkBox,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
    preview_gen: &Rc<Cell<u64>>,
    cache: &Arc<Mutex<PreviewCache>>,
    delay_playback_ms: u64,
) {
    let next_gen = preview_gen.get().wrapping_add(1);
    preview_gen.set(next_gen);

    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }

    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if !needs_background_preview(&ext, &item.kind_str) {
        holder.append(&create_generic_preview(item));
        return;
    }

    let key = (item.path.clone(), file_mtime_nanos(&item.path));

    // Ruta rápida macOS (0 ms): si ya estaba precargado/cacheado en LRU, montarlo antes del Frame 0
    if let Ok(mut guard) = cache.lock() {
        if let Some(cached) = guard.get_if_fresh(&key) {
            let widget = build_widget_from_payload(
                item,
                cached,
                media_handles,
                page_step_cb,
                preview_gen,
                next_gen,
                delay_playback_ms,
            );
            holder.append(&widget);
            return;
        }
    }

    holder.append(&create_loading_placeholder(item));
    spawn_prefetch_into_cache(cache, item.path.clone(), ext, item.kind_str.clone());

    let opened_at = std::time::Instant::now();
    let holder_weak = holder.downgrade();
    let item_clone = item.clone();
    let mh = media_handles.clone();
    let psc = page_step_cb.clone();
    let gen_ref = preview_gen.clone();
    let cache_poll = cache.clone();

    glib::timeout_add_local(Duration::from_millis(8), move || {
        if gen_ref.get() != next_gen {
            return glib::ControlFlow::Break;
        }
        let cached_opt = cache_poll
            .lock()
            .ok()
            .and_then(|mut guard| guard.get_if_fresh(&key));
        if let Some(payload) = cached_opt {
            if let Some(h) = holder_weak.upgrade() {
                if gen_ref.get() == next_gen {
                    let elapsed_ms = opened_at.elapsed().as_millis() as u64;
                    let remaining_delay = delay_playback_ms.saturating_sub(elapsed_ms);
                    let widget = build_widget_from_payload(
                        &item_clone,
                        payload,
                        &mh,
                        &psc,
                        &gen_ref,
                        next_gen,
                        remaining_delay,
                    );
                    while let Some(child) = h.first_child() {
                        h.remove(&child);
                    }
                    h.append(&widget);
                }
            }
            return glib::ControlFlow::Break;
        }
        let still_fetching = cache_poll
            .lock()
            .ok()
            .is_some_and(|guard| guard.in_flight.contains(&key));
        if still_fetching {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

fn prepare_preview_payload(path: &Path, ext: &str, kind_str: &str) -> PreparedPreview {
    if is_image_extension(ext) {
        return PreparedPreview::Image {
            frame: decode_image_rgba_offthread(path, ext),
        };
    }
    if ext == "pdf" {
        let total_pages = query_pdf_page_count(path);
        let first_page = render_pdf_page_rgba_offthread(path, 1);
        return PreparedPreview::Pdf {
            total_pages,
            first_page,
        };
    }
    if is_video_extension(ext) {
        let meta = probe_media_metadata(path);
        let poster = extract_video_poster_rgba_offthread(path, meta.duration_secs);
        return PreparedPreview::Video { meta, poster };
    }
    if is_audio_extension(ext) {
        let meta = probe_media_metadata(path);
        return PreparedPreview::Audio { meta };
    }
    if is_archive_extension(ext) {
        if let Some(text) = read_archive_listing_text(path, ext) {
            return PreparedPreview::Text(text);
        }
    }
    if is_text_preview_candidate(ext, kind_str) {
        return PreparedPreview::Text(read_text_preview_string(path));
    }
    PreparedPreview::Generic
}

fn build_widget_from_payload(
    item: &FileItem,
    payload: PreparedPreview,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
    preview_gen: &Rc<Cell<u64>>,
    expected_gen: u64,
    remaining_delay_ms: u64,
) -> gtk4::Widget {
    match payload {
        PreparedPreview::Image { frame } => {
            create_image_preview_from_frame(&item.path, item, frame)
        }
        PreparedPreview::Pdf {
            total_pages,
            first_page,
        } => create_pdf_preview_from_frame(&item.path, total_pages, first_page, page_step_cb)
            .unwrap_or_else(|| create_generic_preview(item)),
        PreparedPreview::Video { meta, poster } => create_video_preview_from_payload(
            item,
            meta,
            poster,
            media_handles,
            preview_gen,
            expected_gen,
            remaining_delay_ms,
        ),
        PreparedPreview::Audio { meta } => create_audio_preview_from_payload(
            item,
            meta,
            media_handles,
            preview_gen,
            expected_gen,
            remaining_delay_ms,
        ),
        PreparedPreview::Text(content) => create_text_widget_from_string(&content),
        PreparedPreview::Generic => create_generic_preview(item),
    }
}

fn unique_temp_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "malus_ql_{}_{}_{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext
    ))
}

fn decode_pixbuf_to_rgba_frame(pixbuf: &Pixbuf) -> Option<DecodedRgbaFrame> {
    let oriented = pixbuf.apply_embedded_orientation();
    let pb = oriented.as_ref().unwrap_or(pixbuf);
    let width = pb.width();
    let height = pb.height();
    let rowstride = pb.rowstride();
    if width <= 0 || height <= 0 || rowstride <= 0 {
        return None;
    }
    let has_alpha = pb.has_alpha();
    let src_bytes = pb.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let w = width as usize;
    let h = height as usize;
    let stride = rowstride as usize;
    let out_stride = w * 4;
    let mut rgba = vec![0u8; out_stride * h];

    if has_alpha {
        for y in 0..h {
            let s0 = y * stride;
            let d0 = y * out_stride;
            if s0 + out_stride > src.len() {
                return None;
            }
            rgba[d0..d0 + out_stride].copy_from_slice(&src[s0..s0 + out_stride]);
        }
    } else {
        for y in 0..h {
            let s0 = y * stride;
            let d0 = y * out_stride;
            if s0 + w * 3 > src.len() {
                return None;
            }
            for x in 0..w {
                let sp = s0 + x * 3;
                let dp = d0 + x * 4;
                rgba[dp] = src[sp];
                rgba[dp + 1] = src[sp + 1];
                rgba[dp + 2] = src[sp + 2];
                rgba[dp + 3] = 255;
            }
        }
    }

    Some(DecodedRgbaFrame {
        width,
        height,
        stride: out_stride,
        bytes: glib::Bytes::from_owned(rgba),
    })
}

fn decode_image_rgba_offthread(path: &Path, ext: &str) -> Option<DecodedRgbaFrame> {
    if !matches!(ext, "heic" | "avif" | "jxl") {
        if let Ok(pb) = Pixbuf::from_file_at_scale(path, 1280, 920, true) {
            if let Some(frame) = decode_pixbuf_to_rgba_frame(&pb) {
                return Some(frame);
            }
        }
    }

    let tmp_png = unique_temp_path("img", "png");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale='min(1280,iw)':'min(920,ih)':force_original_aspect_ratio=decrease",
        ])
        .arg(&tmp_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let frame = if ok && tmp_png.exists() {
        Pixbuf::from_file_at_scale(&tmp_png, 1280, 920, true)
            .ok()
            .and_then(|pb| decode_pixbuf_to_rgba_frame(&pb))
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    frame
}

fn create_image_preview_from_frame(
    path: &Path,
    item: &FileItem,
    frame_opt: Option<DecodedRgbaFrame>,
) -> gtk4::Widget {
    let texture = frame_opt
        .map(|f| f.to_texture())
        .or_else(|| Texture::from_file(&gio::File::for_path(path)).ok());

    let Some(texture) = texture else {
        return create_generic_preview(item);
    };

    let container = GtkBox::new(Orientation::Vertical, 0);
    container.set_vexpand(true);
    container.set_hexpand(true);
    container.set_margin_top(14);
    container.set_margin_bottom(14);
    container.set_margin_start(14);
    container.set_margin_end(14);

    let picture = Picture::for_paintable(&texture);
    picture.set_content_fit(ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_vexpand(true);
    picture.set_hexpand(true);
    picture.add_css_class("branches-quicklook-image");
    container.append(&picture);

    container.upcast()
}

pub fn parse_pdfinfo_page_count(stdout: &str) -> u32 {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return n.max(1);
            }
        }
    }
    1
}

fn query_pdf_page_count(path: &Path) -> u32 {
    let Ok(out) = Command::new("pdfinfo").arg(path).output() else {
        return 1;
    };
    if !out.status.success() {
        return 1;
    }
    parse_pdfinfo_page_count(&String::from_utf8_lossy(&out.stdout))
}

fn render_pdf_page_rgba_offthread(path: &Path, page: u32) -> Option<DecodedRgbaFrame> {
    let prefix = unique_temp_path("pdf", "tmp").with_extension("");
    let page_str = page.to_string();
    let status = Command::new("pdftoppm")
        .args([
            "-png",
            "-f",
            &page_str,
            "-l",
            &page_str,
            "-singlefile",
            "-scale-to",
            "1100",
        ])
        .arg(path)
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let png_path = PathBuf::from(format!("{}.png", prefix.display()));
    let frame = if png_path.exists() {
        Pixbuf::from_file(&png_path)
            .ok()
            .and_then(|pb| decode_pixbuf_to_rgba_frame(&pb))
    } else {
        None
    };
    let _ = fs::remove_file(&png_path);
    frame
}

fn create_pdf_preview_from_frame(
    path: &Path,
    total_pages: u32,
    first_page: Option<DecodedRgbaFrame>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
) -> Option<gtk4::Widget> {
    let initial_tex = first_page?.to_texture();

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.set_hexpand(true);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();

    let pic_box = GtkBox::new(Orientation::Vertical, 0);
    pic_box.set_margin_top(12);
    pic_box.set_margin_bottom(12);
    pic_box.set_margin_start(14);
    pic_box.set_margin_end(14);
    pic_box.set_vexpand(true);
    pic_box.set_hexpand(true);

    let picture = Picture::for_paintable(&initial_tex);
    picture.set_content_fit(ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_vexpand(true);
    picture.set_hexpand(true);
    picture.add_css_class("branches-quicklook-image");
    pic_box.append(&picture);

    scrolled.set_child(Some(&pic_box));
    root.append(&scrolled);

    if total_pages > 1 {
        let footer = GtkBox::new(Orientation::Horizontal, 10);
        footer.add_css_class("branches-quicklook-footer");
        footer.set_halign(Align::Center);
        footer.set_margin_top(8);
        footer.set_margin_bottom(10);

        let current_page = Rc::new(Cell::new(1u32));
        let page_gen = Rc::new(Cell::new(0u64));
        let prev_btn = Button::builder()
            .icon_name("go-previous-symbolic")
            .has_frame(false)
            .can_focus(false)
            .sensitive(false)
            .tooltip_text("Página anterior (Re Pág)")
            .build();
        let page_lbl = Label::builder()
            .label(&format!("Página 1 de {}", total_pages))
            .css_classes(["caption", "bold"])
            .build();
        let next_btn = Button::builder()
            .icon_name("go-next-symbolic")
            .has_frame(false)
            .can_focus(false)
            .tooltip_text("Página siguiente (Av Pág)")
            .build();

        let step_page: PageStepCallback = Rc::new({
            let path = path.to_path_buf();
            let current_page = current_page.clone();
            let page_gen = page_gen.clone();
            let picture = picture.clone();
            let page_lbl = page_lbl.clone();
            let prev_btn = prev_btn.clone();
            let next_btn = next_btn.clone();
            move |delta: i32| -> bool {
                let curr = current_page.get() as i32;
                let target = (curr + delta).clamp(1, total_pages as i32) as u32;
                if target == current_page.get() {
                    return false;
                }
                current_page.set(target);
                page_lbl.set_label(&format!("Página {} de {}", target, total_pages));
                prev_btn.set_sensitive(target > 1);
                next_btn.set_sensitive(target < total_pages);

                let gen = page_gen.get().wrapping_add(1);
                page_gen.set(gen);
                let (tx, rx) = mpsc::channel::<Option<DecodedRgbaFrame>>();
                let p = path.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(render_pdf_page_rgba_offthread(&p, target));
                });
                let pic_weak = picture.downgrade();
                let pg_ref = page_gen.clone();
                glib::timeout_add_local(Duration::from_millis(8), move || {
                    if pg_ref.get() != gen {
                        return glib::ControlFlow::Break;
                    }
                    match rx.try_recv() {
                        Ok(Some(frame)) => {
                            if let Some(pic) = pic_weak.upgrade() {
                                let tex = frame.to_texture();
                                pic.set_paintable(Some(&tex));
                            }
                            glib::ControlFlow::Break
                        }
                        Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    }
                });
                true
            }
        });

        let sp1 = step_page.clone();
        prev_btn.connect_clicked(move |_| {
            sp1(-1);
        });
        let sp2 = step_page.clone();
        next_btn.connect_clicked(move |_| {
            sp2(1);
        });
        *page_step_cb.borrow_mut() = Some(step_page);

        footer.append(&prev_btn);
        footer.append(&page_lbl);
        footer.append(&next_btn);
        root.append(&footer);
    }

    Some(root.upcast())
}

#[derive(Clone, Default)]
struct MediaMetadata {
    duration_secs: Option<f64>,
    duration_str: Option<String>,
    codec: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    has_audio: bool,
}

fn probe_media_metadata(path: &Path) -> MediaMetadata {
    let mut meta = MediaMetadata::default();
    let Ok(output) = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_name,codec_type,width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
    else {
        return meta;
    };
    if !output.status.success() {
        return meta;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let val = v.trim();
        match k.trim() {
            "duration" => {
                if let Ok(secs) = val.parse::<f64>() {
                    meta.duration_secs = Some(secs);
                    let mins = (secs / 60.0).floor() as u64;
                    let rem = (secs % 60.0).round() as u64;
                    meta.duration_str = Some(format!("{:02}:{:02}", mins, rem));
                }
            }
            "codec_type" if val == "audio" => {
                meta.has_audio = true;
            }
            "codec_name" if meta.codec.is_none() => {
                meta.codec = Some(val.to_string());
            }
            "width" if meta.width.is_none() => {
                if let Ok(w) = val.parse::<u32>() {
                    if w > 0 {
                        meta.width = Some(w);
                    }
                }
            }
            "height" if meta.height.is_none() => {
                if let Ok(h) = val.parse::<u32>() {
                    if h > 0 {
                        meta.height = Some(h);
                    }
                }
            }
            _ => {}
        }
    }
    meta
}

/// Ajusta las dimensiones del frame de video dentro de un máximo manteniendo relación de aspecto
/// y asegurando ancho/alto pares para alineación RGBA limpia.
pub fn fit_video_dimensions(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let sw = src_w.max(2) as f64;
    let sh = src_h.max(2) as f64;
    let scale = (max_w as f64 / sw).min(max_h as f64 / sh).min(1.0);
    let mut w = ((sw * scale).round() as u32).max(2);
    let mut h = ((sh * scale).round() as u32).max(2);
    if w % 2 != 0 {
        w += 1;
    }
    if h % 2 != 0 {
        h += 1;
    }
    (w, h)
}

fn extract_video_poster_rgba_offthread(
    path: &Path,
    duration_secs: Option<f64>,
) -> Option<DecodedRgbaFrame> {
    let seek_arg = match duration_secs {
        Some(d) if d > 2.0 => "0.5",
        _ => "0",
    };
    let tmp_png = unique_temp_path("vposter", "png");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-ss", seek_arg, "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", "scale='min(760,iw)':-1"])
        .arg(&tmp_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let frame = if ok && tmp_png.exists() {
        Pixbuf::from_file(&tmp_png)
            .ok()
            .and_then(|pb| decode_pixbuf_to_rgba_frame(&pb))
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    frame
}

fn spawn_video_playback(
    path: &Path,
    width: u32,
    height: u32,
    play_audio: bool,
    picture: &Picture,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    on_finished: Rc<dyn Fn()>,
) -> Rc<MediaPlaybackHandle> {
    let handle = Rc::new(MediaPlaybackHandle::new());
    media_handles.borrow_mut().push(handle.clone());

    let scale_filter = format!("scale={}:{}:flags=fast_bilinear,fps=24", width, height);
    let child_video = Command::new("ffmpeg")
        .args(["-re", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &scale_filter,
            "-an",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    if play_audio {
        if let Ok(audio_child) = Command::new("ffplay")
            .args(["-nodisp", "-autoexit", "-loglevel", "quiet"])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            handle.register_child(audio_child);
        }
    }

    let Ok(mut child) = child_video else {
        on_finished();
        return handle;
    };

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        on_finished();
        return handle;
    };
    handle.register_child(child);

    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(2);
    let stop_flag = handle.stop_flag.clone();
    let frame_bytes = (width as usize) * (height as usize) * 4;

    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::SeqCst) {
            let mut buf = vec![0u8; frame_bytes];
            if stdout.read_exact(&mut buf).is_err() {
                break;
            }
            match tx.try_send(buf) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        }
    });

    let stop_ui = handle.stop_flag.clone();
    let pic_weak = picture.downgrade();
    let stride = (width as usize) * 4;
    let w_i32 = width as i32;
    let h_i32 = height as i32;

    glib::timeout_add_local(Duration::from_millis(20), move || {
        if stop_ui.load(Ordering::SeqCst) {
            return glib::ControlFlow::Break;
        }
        let Some(pic) = pic_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let mut latest_frame = None;
        loop {
            match rx.try_recv() {
                Ok(frame) => latest_frame = Some(frame),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(frame) = latest_frame {
                        let bytes = glib::Bytes::from_owned(frame);
                        let tex = MemoryTexture::new(
                            w_i32,
                            h_i32,
                            MemoryFormat::R8g8b8a8,
                            &bytes,
                            stride,
                        );
                        pic.set_paintable(Some(&tex));
                    }
                    on_finished();
                    return glib::ControlFlow::Break;
                }
            }
        }

        if let Some(frame) = latest_frame {
            let bytes = glib::Bytes::from_owned(frame);
            let tex = MemoryTexture::new(w_i32, h_i32, MemoryFormat::R8g8b8a8, &bytes, stride);
            pic.set_paintable(Some(&tex));
        }

        glib::ControlFlow::Continue
    });

    handle
}

fn create_video_preview_from_payload(
    item: &FileItem,
    meta: MediaMetadata,
    poster_frame: Option<DecodedRgbaFrame>,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    preview_gen: &Rc<Cell<u64>>,
    expected_gen: u64,
    remaining_delay_ms: u64,
) -> gtk4::Widget {
    let (play_w, play_h) = fit_video_dimensions(
        meta.width.unwrap_or(720),
        meta.height.unwrap_or(420),
        760,
        440,
    );

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.set_hexpand(true);

    let stage = GtkBox::new(Orientation::Vertical, 0);
    stage.set_vexpand(true);
    stage.set_hexpand(true);
    stage.set_margin_top(12);
    stage.set_margin_bottom(8);
    stage.set_margin_start(14);
    stage.set_margin_end(14);
    stage.add_css_class("branches-quicklook-video-stage");

    let picture = Picture::new();
    picture.set_content_fit(ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_vexpand(true);
    picture.set_hexpand(true);
    if let Some(frame) = poster_frame {
        let poster = frame.to_texture();
        picture.set_paintable(Some(&poster));
    }
    stage.append(&picture);
    root.append(&stage);

    let footer = GtkBox::new(Orientation::Horizontal, 10);
    footer.add_css_class("branches-quicklook-footer");
    footer.set_margin_top(4);
    footer.set_margin_bottom(12);
    footer.set_margin_start(16);
    footer.set_margin_end(16);

    let is_playing = Rc::new(Cell::new(true));
    let is_muted = Rc::new(Cell::new(false));
    let current_handle: Rc<RefCell<Option<Rc<MediaPlaybackHandle>>>> = Rc::new(RefCell::new(None));

    let play_btn = Button::builder()
        .icon_name("media-playback-stop-symbolic")
        .label("Detener")
        .has_frame(true)
        .can_focus(false)
        .css_classes(["branches-quicklook-btn"])
        .build();

    let mute_btn = Button::builder()
        .icon_name("audio-volume-high-symbolic")
        .has_frame(false)
        .can_focus(false)
        .tooltip_text("Silenciar / Activar audio")
        .visible(meta.has_audio)
        .build();

    let mut info_parts = Vec::new();
    if let (Some(w), Some(h)) = (meta.width, meta.height) {
        info_parts.push(format!("{}×{}", w, h));
    }
    if let Some(ref d) = meta.duration_str {
        info_parts.push(d.clone());
    }
    if let Some(ref c) = meta.codec {
        info_parts.push(c.clone());
    }
    let meta_lbl = Label::builder()
        .label(&info_parts.join(" • "))
        .halign(Align::Start)
        .hexpand(true)
        .css_classes(["dim-label", "caption"])
        .build();

    footer.append(&play_btn);
    footer.append(&mute_btn);
    footer.append(&meta_lbl);
    root.append(&footer);

    let start_playback: Rc<dyn Fn()> = Rc::new({
        let path = item.path.clone();
        let picture = picture.clone();
        let media_handles = media_handles.clone();
        let is_playing = is_playing.clone();
        let is_muted = is_muted.clone();
        let current_handle = current_handle.clone();
        let play_btn = play_btn.clone();
        let has_audio = meta.has_audio;
        move || {
            if let Some(h) = current_handle.borrow_mut().take() {
                h.stop();
            }
            is_playing.set(true);
            play_btn.set_icon_name("media-playback-stop-symbolic");
            play_btn.set_label("Detener");

            let is_playing_end = is_playing.clone();
            let play_btn_end = play_btn.clone();
            let on_end: Rc<dyn Fn()> = Rc::new(move || {
                is_playing_end.set(false);
                play_btn_end.set_icon_name("media-playback-start-symbolic");
                play_btn_end.set_label("Reproducir");
            });

            let h = spawn_video_playback(
                &path,
                play_w,
                play_h,
                has_audio && !is_muted.get(),
                &picture,
                &media_handles,
                on_end,
            );
            *current_handle.borrow_mut() = Some(h);
        }
    });

    // Diferir arranque de ffmpeg/ffplay hasta completar la transición de apertura
    if remaining_delay_ms == 0 {
        start_playback();
    } else {
        let sp_delayed = start_playback.clone();
        let gen_ref = preview_gen.clone();
        glib::timeout_add_local_once(Duration::from_millis(remaining_delay_ms), move || {
            if gen_ref.get() == expected_gen {
                sp_delayed();
            }
        });
    }

    let sp_toggle = start_playback.clone();
    let ch_toggle = current_handle.clone();
    let ip_toggle = is_playing.clone();
    let pb_toggle = play_btn.clone();
    play_btn.connect_clicked(move |_| {
        if ip_toggle.get() {
            if let Some(h) = ch_toggle.borrow_mut().take() {
                h.stop();
            }
            ip_toggle.set(false);
            pb_toggle.set_icon_name("media-playback-start-symbolic");
            pb_toggle.set_label("Reproducir");
        } else {
            sp_toggle();
        }
    });

    let im_mute = is_muted.clone();
    let mb_mute = mute_btn.clone();
    let sp_mute = start_playback;
    let ip_mute = is_playing;
    mute_btn.connect_clicked(move |_| {
        let next_muted = !im_mute.get();
        im_mute.set(next_muted);
        mb_mute.set_icon_name(if next_muted {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
        if ip_mute.get() {
            sp_mute();
        }
    });

    root.upcast()
}

fn create_audio_preview_from_payload(
    item: &FileItem,
    meta: MediaMetadata,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    preview_gen: &Rc<Cell<u64>>,
    expected_gen: u64,
    remaining_delay_ms: u64,
) -> gtk4::Widget {
    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);

    let icon = Image::from_gicon(&item.gicon);
    crate::views::apply_folder_icon_tint(&icon, item);
    icon.set_pixel_size(96);
    vbox.append(&icon);

    let name = Label::builder()
        .label(&item.name)
        .css_classes(["title-2", "bold"])
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .build();
    vbox.append(&name);

    let mut info = Vec::new();
    if let Some(ref d) = meta.duration_str {
        info.push(format!("Duración: {}", d));
    }
    if let Some(ref c) = meta.codec {
        info.push(format!("Códec: {}", c));
    }
    if !info.is_empty() {
        let lbl = Label::builder()
            .label(&info.join(" • "))
            .css_classes(["dim-label"])
            .build();
        vbox.append(&lbl);
    }

    let is_playing = Rc::new(Cell::new(true));
    let current_handle: Rc<RefCell<Option<Rc<MediaPlaybackHandle>>>> = Rc::new(RefCell::new(None));
    let toggle_btn = Button::builder()
        .label("Detener audio")
        .icon_name("media-playback-stop-symbolic")
        .can_focus(false)
        .css_classes(["suggested-action", "branches-quicklook-btn"])
        .margin_top(10)
        .halign(Align::Center)
        .build();
    vbox.append(&toggle_btn);

    let start_audio: Rc<dyn Fn()> = Rc::new({
        let path = item.path.clone();
        let media_handles = media_handles.clone();
        let is_playing = is_playing.clone();
        let current_handle = current_handle.clone();
        let toggle_btn = toggle_btn.clone();
        move || {
            if let Some(h) = current_handle.borrow_mut().take() {
                h.stop();
            }
            let h = Rc::new(MediaPlaybackHandle::new());
            if let Ok(child) = Command::new("ffplay")
                .args(["-nodisp", "-autoexit", "-loglevel", "quiet"])
                .arg(&path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                h.register_child(child);
                media_handles.borrow_mut().push(h.clone());
                *current_handle.borrow_mut() = Some(h);
                is_playing.set(true);
                toggle_btn.set_label("Detener audio");
                toggle_btn.set_icon_name("media-playback-stop-symbolic");
            }
        }
    });

    if remaining_delay_ms == 0 {
        start_audio();
    } else {
        let sa_delayed = start_audio.clone();
        let gen_ref = preview_gen.clone();
        glib::timeout_add_local_once(Duration::from_millis(remaining_delay_ms), move || {
            if gen_ref.get() == expected_gen {
                sa_delayed();
            }
        });
    }

    let ip_btn = is_playing;
    let ch_btn = current_handle;
    let sa_btn = start_audio;
    let tb_btn = toggle_btn.clone();
    toggle_btn.connect_clicked(move |_| {
        if ip_btn.get() {
            if let Some(h) = ch_btn.borrow_mut().take() {
                h.stop();
            }
            ip_btn.set(false);
            tb_btn.set_label("Reproducir audio");
            tb_btn.set_icon_name("media-playback-start-symbolic");
        } else {
            sa_btn();
        }
    });

    vbox.upcast()
}

fn read_archive_listing_text(path: &Path, ext: &str) -> Option<String> {
    let out = match ext {
        "zip" => Command::new("unzip")
            .args(["-Z1"])
            .arg(path)
            .output()
            .ok()?,
        "tar" | "gz" | "tgz" | "bz2" | "xz" => {
            Command::new("tar").args(["-tf"]).arg(path).output().ok()?
        }
        "7z" => Command::new("7z")
            .args(["l", "-ba"])
            .arg(path)
            .output()
            .ok()?,
        _ => return None,
    };
    if !out.status.success() {
        return None;
    }

    let lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .take(500)
        .map(ToString::to_string)
        .collect();
    if lines.is_empty() {
        Some("(Archivo comprimido vacío)".to_string())
    } else {
        Some(lines.join("\n"))
    }
}

fn read_text_preview_string(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => {
            let truncated = if bytes.len() > 150_000 {
                &bytes[..150_000]
            } else {
                &bytes[..]
            };
            String::from_utf8_lossy(truncated).to_string()
        }
        Err(_) => "(No se pudo leer el archivo de texto)".to_string(),
    }
}

fn create_text_widget_from_string(content: &str) -> gtk4::Widget {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();

    let text_view = TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .can_focus(false)
        .monospace(true)
        .left_margin(16)
        .right_margin(16)
        .top_margin(14)
        .bottom_margin(14)
        .css_classes(["branches-quicklook-text"])
        .build();

    text_view.buffer().set_text(content);
    scrolled.set_child(Some(&text_view));
    scrolled.upcast()
}

fn create_generic_preview(item: &FileItem) -> gtk4::Widget {
    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(32);
    vbox.set_margin_end(32);

    let icon = Image::from_gicon(&item.gicon);
    crate::views::apply_folder_icon_tint(&icon, item);
    icon.set_pixel_size(128);
    vbox.append(&icon);

    let name = Label::builder()
        .label(&item.name)
        .css_classes(["title-2", "bold"])
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .justify(gtk4::Justification::Center)
        .build();
    vbox.append(&name);

    let subtitle_text = if item.is_dir {
        let count = fs::read_dir(&item.path)
            .map(|rd| rd.flatten().count())
            .unwrap_or(0);
        format!("{} • {} elementos", item.kind_str, count)
    } else {
        format!("{} • {}", item.kind_str, item.size_str)
    };
    let kind = Label::builder()
        .label(&subtitle_text)
        .css_classes(["dim-label"])
        .build();
    vbox.append(&kind);

    let sep = Separator::new(Orientation::Horizontal);
    sep.set_margin_top(8);
    sep.set_margin_bottom(8);
    vbox.append(&sep);

    let grid = Grid::builder()
        .row_spacing(6)
        .column_spacing(12)
        .halign(Align::Center)
        .build();

    let mut row = 0;
    let add_row = |g: &Grid, r: &mut i32, k: &str, v: &str| {
        let kl = Label::builder()
            .label(k)
            .halign(Align::End)
            .css_classes(["dim-label", "bold"])
            .build();
        let vl = Label::builder().label(v).halign(Align::Start).build();
        g.attach(&kl, 0, *r, 1, 1);
        g.attach(&vl, 1, *r, 1, 1);
        *r += 1;
    };

    add_row(&grid, &mut row, "Modificado:", &item.modified_str);
    add_row(&grid, &mut row, "Ubicación:", &item.path.to_string_lossy());

    vbox.append(&grid);

    let p_clone = item.path.clone();
    let btn = Button::builder()
        .label("Abrir archivo")
        .can_focus(false)
        .css_classes(["suggested-action", "branches-quicklook-btn"])
        .margin_top(16)
        .halign(Align::Center)
        .build();
    btn.connect_clicked(move |_| {
        let _ = open_file(&p_clone);
    });
    vbox.append(&btn);

    vbox.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_look_extension_classifiers_cover_images_videos_and_audio() {
        assert!(is_image_extension("png"));
        assert!(is_image_extension("webp"));
        assert!(is_image_extension("avif"));
        assert!(is_video_extension("mp4"));
        assert!(is_video_extension("mkv"));
        assert!(is_video_extension("webm"));
        assert!(is_audio_extension("mp3"));
        assert!(is_audio_extension("flac"));
        assert!(!is_image_extension("pdf"));
        assert!(!is_video_extension("pdf"));
    }

    #[test]
    fn parse_pdfinfo_page_count_extracts_total_pages_with_safe_default() {
        let sample = "Title: Demo\nAuthor: Test\nPages:          14\nEncrypted: no\n";
        assert_eq!(parse_pdfinfo_page_count(sample), 14);
        assert_eq!(parse_pdfinfo_page_count("no pages header"), 1);
    }

    #[test]
    fn fit_video_dimensions_keeps_aspect_ratio_and_even_stride() {
        let (w, h) = fit_video_dimensions(1920, 1080, 760, 440);
        assert!(w <= 760 && h <= 440);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert_eq!((w, h), (760, 428));
    }

    #[test]
    fn preview_cache_respects_lru_eviction_and_mtime_invalidation() {
        let mut cache = PreviewCache::new(2);
        let k1 = (PathBuf::from("/tmp/a.png"), 100);
        let k2 = (PathBuf::from("/tmp/b.png"), 200);
        let k3 = (PathBuf::from("/tmp/c.png"), 300);

        cache.insert(k1.clone(), PreparedPreview::Text("a".into()));
        cache.insert(k2.clone(), PreparedPreview::Text("b".into()));
        // Re-acceder k1 lo promueve a MRU; k2 pasa a ser LRU
        assert!(cache.get_if_fresh(&k1).is_some());
        cache.insert(k3.clone(), PreparedPreview::Text("c".into()));

        assert!(cache.get_if_fresh(&k1).is_some());
        assert!(cache.get_if_fresh(&k2).is_none());
        assert!(cache.get_if_fresh(&k3).is_some());
        // Cambio de mtime invalida la clave
        assert!(cache
            .get_if_fresh(&(PathBuf::from("/tmp/a.png"), 101))
            .is_none());
    }

    #[test]
    fn decode_pixbuf_to_rgba_frame_produces_aligned_rgba_bytes() {
        let pb = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 5, 3).unwrap();
        let frame = decode_pixbuf_to_rgba_frame(&pb).unwrap();
        assert_eq!(frame.width, 5);
        assert_eq!(frame.height, 3);
        assert_eq!(frame.stride, 20);
        assert_eq!(frame.bytes.len(), 60);
    }
}
