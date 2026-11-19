use crate::fs_backend::{open_file, FileItem};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{Key, Texture};
use gtk4::gio;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, EventControllerKey, Grid, Image,
    Label, Orientation, PolicyType, ScrolledWindow, Separator, TextView, Window,
};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct QuickLook {
    window: RefCell<Option<Window>>,
    current_path: RefCell<Option<PathBuf>>,
    items_list: RefCell<Vec<FileItem>>,
    current_index: Cell<usize>,
    content_holder: RefCell<Option<GtkBox>>,
    title_label: RefCell<Option<Label>>,
    subtitle_label: RefCell<Option<Label>>,
    open_btn_path: Rc<RefCell<PathBuf>>,
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
        })
    }

    pub fn is_open(&self) -> bool {
        self.window.borrow().is_some()
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
        if let Some(win) = self.window.borrow_mut().take() {
            win.close();
            self.clear_refs();
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

    fn clear_refs(&self) {
        *self.current_path.borrow_mut() = None;
        *self.content_holder.borrow_mut() = None;
        *self.title_label.borrow_mut() = None;
        *self.subtitle_label.borrow_mut() = None;
        self.items_list.borrow_mut().clear();
    }

    pub fn update_if_open(&self, item: &FileItem) {
        if self.is_open() {
            if let Some(holder) = self.content_holder.borrow().as_ref() {
                while let Some(child) = holder.first_child() {
                    holder.remove(&child);
                }
                let new_content = create_preview_content(item);
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
            .default_width(740)
            .default_height(520)
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
            .default_width(740)
            .default_height(520)
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
                .valign(Align::Center)
                .tooltip_text("Anterior (Flecha izquierda)")
                .build();
            let ql_prev = self.clone();
            prev_btn.connect_clicked(move |_| ql_prev.step_item(-1));

            let next_btn = Button::builder()
                .icon_name("go-next-symbolic")
                .has_frame(false)
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

        let preview_content = create_preview_content(item);
        content_holder.append(&preview_content);
        root_box.append(&content_holder);

        win.set_child(Some(&root_box));

        // Controlador de teclado: Espacio / Escape cierran; Izquierda / Derecha navegan entre archivos
        let key_controller = EventControllerKey::new();
        let win_weak = win.clone();
        let self_key = self.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == Key::space || keyval == Key::Escape {
                win_weak.close();
                *self_key.window.borrow_mut() = None;
                self_key.clear_refs();
                return glib::Propagation::Stop;
            }
            if keyval == Key::Left {
                self_key.step_item(-1);
                return glib::Propagation::Stop;
            }
            if keyval == Key::Right {
                self_key.step_item(1);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(key_controller);

        let self_destroy = self.clone();
        win.connect_close_request(move |_| {
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

fn create_preview_content(item: &FileItem) -> gtk4::Widget {
    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let is_img = ["png", "jpg", "jpeg", "webp", "gif", "svg", "bmp"]
        .iter()
        .any(|e| *e == ext);
    if is_img {
        return create_image_preview(&item.path);
    }

    if ext == "pdf" {
        if let Some(pdf_widget) = create_pdf_first_page_preview(&item.path) {
            return pdf_widget;
        }
    }

    let is_media = [
        "mp4", "mkv", "webm", "mov", "avi", "mp3", "flac", "ogg", "wav", "m4a", "aac", "opus",
    ]
    .iter()
    .any(|e| *e == ext);
    if is_media {
        return create_media_preview(item);
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

fn create_pdf_first_page_preview(path: &Path) -> Option<gtk4::Widget> {
    let prefix = std::env::temp_dir().join(format!(
        "malus_ql_pdf_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let status = std::process::Command::new("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-singlefile", "-scale-to", "720"])
        .arg(path)
        .arg(&prefix)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let png_path = PathBuf::from(format!("{}.png", prefix.display()));
    let widget = if png_path.exists() {
        Some(create_image_preview(&png_path))
    } else {
        None
    };
    let _ = fs::remove_file(&png_path);
    widget
}

fn create_media_preview(item: &FileItem) -> gtk4::Widget {
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Ok(output) = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,bit_rate:stream=codec_name,codec_type,width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(&item.path)
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    match k.trim() {
                        "duration" => {
                            if let Ok(secs) = v.trim().parse::<f64>() {
                                let mins = (secs / 60.0).floor() as u64;
                                let rem = (secs % 60.0).round() as u64;
                                rows.push((
                                    "Duración:".to_string(),
                                    format!("{:02}:{:02}", mins, rem),
                                ));
                            }
                        }
                        "codec_name" => rows.push(("Códec:".to_string(), v.trim().to_string())),
                        "width" => rows.push(("Ancho:".to_string(), format!("{} px", v.trim()))),
                        "height" => rows.push(("Alto:".to_string(), format!("{} px", v.trim()))),
                        _ => {}
                    }
                }
            }
        }
    }

    if rows.is_empty() {
        return create_generic_preview(item);
    }

    let vbox = GtkBox::new(Orientation::Vertical, 10);
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

    let grid = Grid::builder()
        .row_spacing(6)
        .column_spacing(12)
        .halign(Align::Center)
        .build();
    for (idx, (k, v)) in rows.into_iter().take(6).enumerate() {
        let kl = Label::builder()
            .label(&k)
            .halign(Align::End)
            .css_classes(["dim-label", "bold"])
            .build();
        let vl = Label::builder().label(&v).halign(Align::Start).build();
        grid.attach(&kl, 0, idx as i32, 1, 1);
        grid.attach(&vl, 1, idx as i32, 1, 1);
    }
    vbox.append(&grid);

    let p_clone = item.path.clone();
    let btn = Button::builder()
        .label("Reproducir / Abrir")
        .css_classes(["suggested-action", "branches-quicklook-btn"])
        .margin_top(12)
        .halign(Align::Center)
        .build();
    btn.connect_clicked(move |_| {
        let _ = open_file(&p_clone);
    });
    vbox.append(&btn);

    vbox.upcast()
}

fn create_image_preview(path: &Path) -> gtk4::Widget {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.set_valign(Align::Center);
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    if let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 700, 480, true) {
        let texture = Texture::for_pixbuf(&pixbuf);
        let img = Image::from_paintable(Some(&texture));
        img.add_css_class("branches-quicklook-image");
        vbox.append(&img);
    }

    scrolled.set_child(Some(&vbox));
    scrolled.upcast()
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

    let kind = Label::builder()
        .label(&format!("{} • {}", item.kind_str, item.size_str))
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
