use crate::fs_backend::{open_file, open_in_terminal, FileItem};
use crate::views::{NavigateCallback, SelectionCallback};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{Key, Texture};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, EventControllerKey, GestureClick, Image, Label,
    Orientation, PolicyType, ScrolledWindow, Separator,
};
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

pub struct GalleryView {
    container: GtkBox,
    canvas_holder: GtkBox,
    carousel_box: GtkBox,
    #[allow(dead_code)]
    carousel_scrolled: ScrolledWindow,
    items: RefCell<Vec<FileItem>>,
    current_index: RefCell<Option<usize>>,
    carousel_widgets: RefCell<Vec<GtkBox>>,
    on_navigate: NavigateCallback,
    on_selection_changed: SelectionCallback,
}

impl GalleryView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Rc<Self> {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.add_css_class("branches-gallery-view");
        container.set_hexpand(true);
        container.set_vexpand(true);

        // 1. Lienzo central superior a gran escala
        let canvas_holder = GtkBox::new(Orientation::Vertical, 0);
        canvas_holder.add_css_class("branches-gallery-canvas");
        canvas_holder.set_vexpand(true);
        canvas_holder.set_hexpand(true);
        container.append(&canvas_holder);

        let sep = Separator::new(Orientation::Horizontal);
        container.append(&sep);

        // 2. Carrusel horizontal inferior
        let carousel_box = GtkBox::new(Orientation::Horizontal, 8);
        carousel_box.add_css_class("branches-gallery-carousel");
        carousel_box.set_valign(Align::Center);
        carousel_box.set_margin_top(8);
        carousel_box.set_margin_bottom(8);
        carousel_box.set_margin_start(12);
        carousel_box.set_margin_end(12);

        let carousel_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Never)
            .css_classes(["branches-gallery-scrolled"])
            .min_content_height(120)
            .child(&carousel_box)
            .build();

        container.append(&carousel_scrolled);

        let gallery = Rc::new(Self {
            container,
            canvas_holder,
            carousel_box,
            carousel_scrolled,
            items: RefCell::new(Vec::new()),
            current_index: RefCell::new(None),
            carousel_widgets: RefCell::new(Vec::new()),
            on_navigate,
            on_selection_changed,
        });

        // Controlador de teclado: Flecha Izq / Der navega el carrusel
        let key_controller = EventControllerKey::new();
        let g_weak = gallery.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, _| {
            match keyval {
                Key::Left => {
                    g_weak.select_prev();
                    glib::Propagation::Stop
                }
                Key::Right => {
                    g_weak.select_next();
                    glib::Propagation::Stop
                }
                Key::Return => {
                    if let Some(idx) = *g_weak.current_index.borrow() {
                        let items = g_weak.items.borrow();
                        if let Some(item) = items.get(idx) {
                            if item.is_dir {
                                (g_weak.on_navigate)(item.path.clone());
                            } else {
                                let _ = open_file(&item.path);
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        gallery.container.add_controller(key_controller);

        gallery
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_items(self: &Rc<Self>, items: &[FileItem]) {
        *self.items.borrow_mut() = items.to_vec();
        *self.current_index.borrow_mut() = None;

        // Limpiar carrusel previo
        while let Some(child) = self.carousel_box.first_child() {
            self.carousel_box.remove(&child);
        }
        self.carousel_widgets.borrow_mut().clear();

        if items.is_empty() {
            self.show_empty_canvas();
            return;
        }

        // Poblar carrusel
        for (idx, item) in items.iter().enumerate() {
            let card = self.create_carousel_card(idx, item);
            self.carousel_box.append(&card);
            self.carousel_widgets.borrow_mut().push(card);
        }

        // Seleccionar primer elemento por defecto
        self.select_index(0);
    }

    pub fn select_prev(self: &Rc<Self>) {
        if let Some(curr) = *self.current_index.borrow() {
            if curr > 0 {
                self.select_index(curr - 1);
            }
        }
    }

    pub fn select_next(self: &Rc<Self>) {
        let total = self.items.borrow().len();
        if let Some(curr) = *self.current_index.borrow() {
            if curr + 1 < total {
                self.select_index(curr + 1);
            }
        }
    }

    pub fn select_index(self: &Rc<Self>, index: usize) {
        let items = self.items.borrow();
        if let Some(item) = items.get(index) {
            *self.current_index.borrow_mut() = Some(index);

            // Actualizar estilo activo en las tarjetas del carrusel
            let widgets = self.carousel_widgets.borrow();
            for (i, widget) in widgets.iter().enumerate() {
                if i == index {
                    widget.add_css_class("branches-carousel-active");
                } else {
                    widget.remove_css_class("branches-carousel-active");
                }
            }

            // Actualizar lienzo superior
            self.show_item_in_canvas(item);
            (self.on_selection_changed)(1);
        }
    }

    fn show_empty_canvas(&self) {
        while let Some(child) = self.canvas_holder.first_child() {
            self.canvas_holder.remove(&child);
        }

        let vbox = GtkBox::new(Orientation::Vertical, 10);
        vbox.set_valign(Align::Center);
        vbox.set_halign(Align::Center);

        let icon = Image::from_icon_name("folder-open-symbolic");
        icon.set_pixel_size(64);
        icon.add_css_class("dim-label");
        vbox.append(&icon);

        let label = Label::builder()
            .label("Carpeta vacía")
            .css_classes(["title-3", "dim-label"])
            .build();
        vbox.append(&label);

        self.canvas_holder.append(&vbox);
    }

    fn show_item_in_canvas(&self, item: &FileItem) {
        while let Some(child) = self.canvas_holder.first_child() {
            self.canvas_holder.remove(&child);
        }

        let main_box = GtkBox::new(Orientation::Vertical, 10);
        main_box.set_valign(Align::Center);
        main_box.set_halign(Align::Center);
        main_box.set_margin_top(16);
        main_box.set_margin_bottom(16);
        main_box.set_margin_start(20);
        main_box.set_margin_end(20);

        let ext = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let is_img = ["png", "jpg", "jpeg", "webp", "gif", "svg", "bmp"]
            .iter()
            .any(|e| *e == ext);

        if is_img {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&item.path, 520, 360, true) {
                let texture = Texture::for_pixbuf(&pixbuf);
                let img = Image::from_paintable(Some(&texture));
                img.add_css_class("branches-gallery-large-image");
                main_box.append(&img);
            } else {
                let icon = Image::from_gicon(&item.gicon);
                icon.set_pixel_size(128);
                main_box.append(&icon);
            }
        } else if ["rs", "toml", "json", "md", "txt", "sh", "py", "c", "cpp", "js", "html", "css"]
            .iter()
            .any(|e| *e == ext)
        {
            let text_card = GtkBox::new(Orientation::Vertical, 6);
            text_card.add_css_class("branches-gallery-text-card");
            text_card.set_size_request(420, 260);

            let preview_snippet = match fs::read(&item.path) {
                Ok(bytes) => {
                    let len = bytes.len().min(400);
                    String::from_utf8_lossy(&bytes[..len]).to_string()
                }
                Err(_) => "(Texto no disponible)".to_string(),
            };

            let lbl = Label::builder()
                .label(&preview_snippet)
                .wrap(true)
                .wrap_mode(pango::WrapMode::WordChar)
                .halign(Align::Start)
                .valign(Align::Start)
                .css_classes(["branches-gallery-snippet"])
                .build();
            text_card.append(&lbl);
            main_box.append(&text_card);
        } else {
            let icon = Image::from_gicon(&item.gicon);
            icon.set_pixel_size(128);
            main_box.append(&icon);
        }

        // Título del archivo
        let name_label = Label::builder()
            .label(&item.name)
            .css_classes(["title-2", "bold"])
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .justify(gtk4::Justification::Center)
            .build();
        main_box.append(&name_label);

        // Subtítulo
        let sub = Label::builder()
            .label(&format!("{} • {}", item.kind_str, item.size_str))
            .css_classes(["dim-label"])
            .build();
        main_box.append(&sub);

        // Botones de acción rápida
        let btn_box = GtkBox::new(Orientation::Horizontal, 8);
        btn_box.set_halign(Align::Center);
        btn_box.set_margin_top(6);

        let p_for_open = item.path.clone();
        let open_btn = Button::builder()
            .label("Abrir archivo")
            .css_classes(["suggested-action"])
            .build();
        open_btn.connect_clicked(move |_| {
            let _ = open_file(&p_for_open);
        });
        btn_box.append(&open_btn);

        let p_for_term = item.path.clone();
        let term_btn = Button::builder()
            .label("Terminal")
            .icon_name("utilities-terminal-symbolic")
            .css_classes(["flat"])
            .build();
        term_btn.connect_clicked(move |_| {
            open_in_terminal(&p_for_term);
        });
        btn_box.append(&term_btn);

        main_box.append(&btn_box);
        self.canvas_holder.append(&main_box);
    }

    fn create_carousel_card(self: &Rc<Self>, index: usize, item: &FileItem) -> GtkBox {
        let card = GtkBox::new(Orientation::Vertical, 4);
        card.add_css_class("branches-carousel-card");
        card.set_size_request(84, 100);
        card.set_valign(Align::Center);

        let ext = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let is_img = ["png", "jpg", "jpeg", "webp", "gif", "svg", "bmp"]
            .iter()
            .any(|e| *e == ext);

        let thumb_widget: gtk4::Widget = if is_img {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&item.path, 48, 48, true) {
                let texture = Texture::for_pixbuf(&pixbuf);
                let img = Image::from_paintable(Some(&texture));
                img.add_css_class("branches-carousel-thumb");
                img.upcast()
            } else {
                let img = Image::from_gicon(&item.gicon);
                img.set_pixel_size(42);
                img.upcast()
            }
        } else {
            let img = Image::from_gicon(&item.gicon);
            img.set_pixel_size(42);
            img.upcast()
        };

        card.append(&thumb_widget);

        let lbl = Label::builder()
            .label(&item.name)
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(10)
            .halign(Align::Center)
            .css_classes(["branches-carousel-label"])
            .build();
        card.append(&lbl);

        let click = GestureClick::new();
        let self_click = self.clone();
        let p_for_dbl = item.path.clone();
        let is_dir = item.is_dir;
        let on_nav = self.on_navigate.clone();

        click.connect_pressed(move |g, n_press, _, _| {
            if n_press == 1 {
                self_click.select_index(index);
            } else if n_press >= 2 {
                if is_dir {
                    on_nav(p_for_dbl.clone());
                } else {
                    let _ = open_file(&p_for_dbl);
                }
            }
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        card.add_controller(click);

        card
    }
}
