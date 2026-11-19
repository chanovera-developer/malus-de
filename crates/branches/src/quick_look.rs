use crate::fs_backend::{open_file, FileItem};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{Key, MemoryFormat, MemoryTexture, ModifierType, Texture};
use gtk4::gio;
use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, ContentFit, EventControllerKey,
    Grid, Image, Label, Orientation, Picture, PolicyType, PropagationPhase, ScrolledWindow,
    Separator, TextView, Window,
};
use std::cell::{Cell, RefCell};
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
            let _ = child.wait();
            return;
        }
        if let Ok(mut guard) = self.children.lock() {
            guard.push(child);
        }
    }

    fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.children.lock() {
            for mut child in guard.drain(..) {
                let _ = child.kill();
                let _ = child.wait();
            }
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
    current_path: RefCell<Option<PathBuf>>,
    items_list: RefCell<Vec<FileItem>>,
    current_index: Cell<usize>,
    content_holder: RefCell<Option<GtkBox>>,
    title_label: RefCell<Option<Label>>,
    subtitle_label: RefCell<Option<Label>>,
    open_btn_path: Rc<RefCell<PathBuf>>,
    media_handles: Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: Rc<RefCell<Option<PageStepCallback>>>,
}

impl QuickLook {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            window: RefCell::new(None),
            current_path: RefCell::new(None),
            items_list: RefCell::new(Vec::new()),
            current_index: Cell::new(0),
            content_holder: RefCell::new(None),
            title_label: RefCell::new(None),
            subtitle_label: RefCell::new(None),
            open_btn_path: Rc::new(RefCell::new(PathBuf::new())),
            media_handles: Rc::new(RefCell::new(Vec::new())),
            page_step_cb: Rc::new(RefCell::new(None)),
        })
    }

    pub fn is_open(&self) -> bool {
        self.window.borrow().is_some()
    }

    pub fn close(&self) {
        self.stop_active_media();
        let win_opt = self.window.borrow_mut().take();
        self.clear_refs();
        if let Some(win) = win_opt {
            win.close();
        }
    }

    #[allow(dead_code)]
    pub fn toggle(self: &Rc<Self>, parent: &ApplicationWindow, item: Option<FileItem>) {
        let items = item.iter().cloned().collect::<Vec<_>>();
        self.toggle_with_items(parent, item, items);
    }

    pub fn toggle_with_items(
        self: &Rc<Self>,
        parent: &ApplicationWindow,
        item: Option<FileItem>,
        items: Vec<FileItem>,
    ) {
        if self.is_open() {
            self.close();
            return;
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
        self.show(parent, &item);
    }

    fn stop_active_media(&self) {
        for handle in self.media_handles.borrow_mut().drain(..) {
            handle.stop();
        }
    }

    fn clear_refs(&self) {
        self.stop_active_media();
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
                while let Some(child) = holder.first_child() {
                    holder.remove(&child);
                }
                let new_content =
                    create_preview_content(item, &self.media_handles, &self.page_step_cb);
                holder.append(&new_content);
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

    fn step_item(&self, delta: isize) {
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
        }
    }

    pub fn show_standalone(self: &Rc<Self>, app: &Application, item: &FileItem) {
        let win = Window::builder()
            .application(app)
            .title(&item.name)
            .default_width(800)
            .default_height(580)
            .css_classes(["branches-quicklook-window"])
            .build();
        *self.items_list.borrow_mut() = vec![item.clone()];
        self.current_index.set(0);
        self.setup_window(&win, item);
    }

    fn show(self: &Rc<Self>, parent: &ApplicationWindow, item: &FileItem) {
        let win = Window::builder()
            .transient_for(parent)
            .destroy_with_parent(true)
            .title(&item.name)
            .default_width(800)
            .default_height(580)
            .css_classes(["branches-quicklook-window"])
            .build();
        self.setup_window(&win, item);
    }

    fn setup_window(self: &Rc<Self>, win: &Window, item: &FileItem) {
        let root_box = GtkBox::new(Orientation::Vertical, 0);
        root_box.add_css_class("branches-quicklook-container");

        // Barra superior de Quick Look
        let header_box = GtkBox::new(Orientation::Horizontal, 8);
        header_box.add_css_class("branches-quicklook-header");

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

        // Contenedor dinámico de previsualización
        let content_holder = GtkBox::new(Orientation::Vertical, 0);
        content_holder.set_vexpand(true);
        content_holder.set_hexpand(true);
        content_holder.add_css_class("branches-quicklook-content");

        let preview_content =
            create_preview_content(item, &self.media_handles, &self.page_step_cb);
        content_holder.append(&preview_content);
        root_box.append(&content_holder);

        win.set_child(Some(&root_box));

        // Controlador en fase Capture: garantiza que Espacio y Escape siempre cierren Quick Look
        // sin ser consumidos por botones o barras de desplazamiento hijas.
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
            if !has_mod && keyval == Key::Page_Up {
                if let Some(cb) = self_key.page_step_cb.borrow().as_ref() {
                    if cb(-1) {
                        return glib::Propagation::Stop;
                    }
                }
            }
            if !has_mod && keyval == Key::Page_Down {
                if let Some(cb) = self_key.page_step_cb.borrow().as_ref() {
                    if cb(1) {
                        return glib::Propagation::Stop;
                    }
                }
            }
            glib::Propagation::Proceed
        });
        win.add_controller(key_controller);

        let self_destroy = self.clone();
        win.connect_close_request(move |_| {
            self_destroy.stop_active_media();
            *self_destroy.window.borrow_mut() = None;
            self_destroy.clear_refs();
            glib::Propagation::Proceed
        });

        *self.title_label.borrow_mut() = Some(title_label);
        *self.subtitle_label.borrow_mut() = Some(subtitle_label);
        *self.content_holder.borrow_mut() = Some(content_holder);
        *self.current_path.borrow_mut() = Some(item.path.clone());
        *self.window.borrow_mut() = Some(win.clone());

        win.present();
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
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "flv" | "ts" | "mpg" | "mpeg" | "3gp" | "ogv"
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

fn create_preview_content(
    item: &FileItem,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
) -> gtk4::Widget {
    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if is_image_extension(&ext) {
        return create_image_preview(&item.path, item);
    }

    if ext == "pdf" {
        if let Some(pdf_widget) = create_pdf_preview(&item.path, page_step_cb) {
            return pdf_widget;
        }
    }

    if is_video_extension(&ext) {
        return create_video_preview(item, media_handles);
    }

    if is_audio_extension(&ext) {
        return create_audio_preview(item, media_handles);
    }

    if is_archive_extension(&ext) {
        if let Some(arch_widget) = create_archive_preview(&item.path) {
            return arch_widget;
        }
    }

    let is_text = [
        "rs", "toml", "json", "md", "txt", "sh", "c", "cpp", "h", "py", "js", "ts", "html", "css",
        "yaml", "yml", "xml", "ini", "conf", "log",
    ]
    .iter()
    .any(|e| *e == ext)
        || item.kind_str.to_lowercase().contains("texto")
        || item.kind_str.to_lowercase().contains("código");

    if is_text {
        return create_text_preview(&item.path);
    }

    create_generic_preview(item)
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

fn load_image_texture(path: &Path) -> Option<Texture> {
    let gfile = gio::File::for_path(path);
    if let Ok(tex) = Texture::from_file(&gfile) {
        return Some(tex);
    }
    if let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 1280, 920, true) {
        return Some(Texture::for_pixbuf(&pixbuf));
    }
    // Respaldo con ffmpeg para formatos poco comunes (HEIC/AVIF/JXL)
    let tmp_png = unique_temp_path("img", "png");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1"])
        .arg(&tmp_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let tex = if ok && tmp_png.exists() {
        Texture::from_file(&gio::File::for_path(&tmp_png)).ok()
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    tex
}

fn create_image_preview(path: &Path, item: &FileItem) -> gtk4::Widget {
    let Some(texture) = load_image_texture(path) else {
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

fn render_pdf_page_texture(path: &Path, page: u32) -> Option<Texture> {
    let prefix = unique_temp_path("pdf", "tmp").with_extension("");
    let page_str = page.to_string();
    let status = Command::new("pdftoppm")
        .args(["-png", "-f", &page_str, "-l", &page_str, "-singlefile", "-scale-to", "1100"])
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
    let tex = if png_path.exists() {
        Texture::from_file(&gio::File::for_path(&png_path)).ok()
    } else {
        None
    };
    let _ = fs::remove_file(&png_path);
    tex
}

fn create_pdf_preview(
    path: &Path,
    page_step_cb: &Rc<RefCell<Option<PageStepCallback>>>,
) -> Option<gtk4::Widget> {
    let initial_tex = render_pdf_page_texture(path, 1)?;
    let total_pages = query_pdf_page_count(path);

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
                if let Some(tex) = render_pdf_page_texture(&path, target) {
                    current_page.set(target);
                    picture.set_paintable(Some(&tex));
                    page_lbl.set_label(&format!("Página {} de {}", target, total_pages));
                    prev_btn.set_sensitive(target > 1);
                    next_btn.set_sensitive(target < total_pages);
                    return true;
                }
                false
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

#[derive(Default)]
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

fn extract_video_poster_texture(path: &Path, duration_secs: Option<f64>) -> Option<Texture> {
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
    let tex = if ok && tmp_png.exists() {
        Texture::from_file(&gio::File::for_path(&tmp_png)).ok()
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    tex
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
            let tex =
                MemoryTexture::new(w_i32, h_i32, MemoryFormat::R8g8b8a8, &bytes, stride);
            pic.set_paintable(Some(&tex));
        }

        glib::ControlFlow::Continue
    });

    handle
}

fn create_video_preview(
    item: &FileItem,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
) -> gtk4::Widget {
    let meta = probe_media_metadata(&item.path);
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
    if let Some(poster) = extract_video_poster_texture(&item.path, meta.duration_secs) {
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

    // Iniciar reproducción inmediatamente al abrir Vista Rápida
    start_playback();

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

fn create_audio_preview(
    item: &FileItem,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
) -> gtk4::Widget {
    let meta = probe_media_metadata(&item.path);

    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);

    let icon = Image::from_gicon(&item.gicon);
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

    start_audio();

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

fn create_archive_preview(path: &Path) -> Option<gtk4::Widget> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let out = match ext.as_str() {
        "zip" => Command::new("unzip").args(["-Z1"]).arg(path).output().ok()?,
        "tar" | "gz" | "tgz" | "bz2" | "xz" => Command::new("tar").args(["-tf"]).arg(path).output().ok()?,
        "7z" => Command::new("7z").args(["l", "-ba"]).arg(path).output().ok()?,
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

    let text = if lines.is_empty() {
        "(Archivo comprimido vacío)".to_string()
    } else {
        lines.join("\n")
    };
    text_view.buffer().set_text(&text);
    scrolled.set_child(Some(&text_view));
    Some(scrolled.upcast())
}

fn create_text_preview(path: &Path) -> gtk4::Widget {
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

    let content = match fs::read(path) {
        Ok(bytes) => {
            let truncated = if bytes.len() > 150_000 {
                &bytes[..150_000]
            } else {
                &bytes[..]
            };
            String::from_utf8_lossy(truncated).to_string()
        }
        Err(_) => "(No se pudo leer el archivo de texto)".to_string(),
    };

    text_view.buffer().set_text(&content);
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
}
