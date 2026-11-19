use crate::fs_backend::{open_file, open_in_terminal, FileItem};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{self, Key, Texture};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, EventControllerKey, GestureClick, Image, Label,
    Orientation, PolicyType, ScrolledWindow, Separator,
};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct GalleryView {
    container: GtkBox,
    canvas_holder: GtkBox,
    carousel_box: GtkBox,
    #[allow(dead_code)]
    carousel_scrolled: ScrolledWindow,
    items: RefCell<Vec<FileItem>>,
    current_folder: RefCell<Option<PathBuf>>,
    pending_select_name: RefCell<Option<String>>,
    current_index: RefCell<Option<usize>>,
    selected_indices: RefCell<BTreeSet<usize>>,
    selection_anchor: RefCell<Option<usize>>,
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
            current_folder: RefCell::new(None),
            pending_select_name: RefCell::new(None),
            current_index: RefCell::new(None),
            selected_indices: RefCell::new(BTreeSet::new()),
            selection_anchor: RefCell::new(None),
            carousel_widgets: RefCell::new(Vec::new()),
            on_navigate,
            on_selection_changed,
        });

        let bg_click = GestureClick::new();
        bg_click.set_button(gdk::BUTTON_SECONDARY);
        let g_bg = gallery.clone();
        bg_click.connect_pressed(move |g, _n, x, y| {
            let Some(dir) = g_bg.current_folder.borrow().clone() else {
                return;
            };
            g.set_state(gtk4::EventSequenceState::Claimed);
            let nav = g_bg.on_navigate.clone();
            let dir_reload = dir.clone();
            let g_create = g_bg.clone();
            crate::views::show_background_create_menu(
                &g_bg.canvas_holder,
                x,
                y,
                dir,
                Rc::new(move |created_path| {
                    if let Some(new_name) =
                        created_path.file_name().map(|n| n.to_string_lossy().into_owned())
                    {
                        g_create.queue_select_name(&new_name);
                    }
                    nav(dir_reload.clone());

                    let g_lookup = g_create.clone();
                    let target_path = created_path;
                    let g_rename = g_create.clone();
                    let nav_rename = nav.clone();
                    let dir_rename = dir_reload.clone();
                    let on_renamed: crate::views::RenameCallback = Rc::new(move |renamed_path| {
                        if let Some(renamed_name) = renamed_path.file_name() {
                            g_rename.queue_select_name(&renamed_name.to_string_lossy());
                        }
                        nav_rename(dir_rename.clone());
                    });
                    crate::views::schedule_inline_rename(
                        move || {
                            let (item, widget) = g_lookup.get_selected_item_and_widget()?;
                            if item.path != target_path {
                                return None;
                            }
                            Some((item.path, item.is_dir, widget))
                        },
                        Some(on_renamed),
                    );
                }),
            );
        });
        gallery.canvas_holder.add_controller(bg_click);

        // Controlador de teclado: Flecha Izq / Der navega el carrusel (Shift extiende rango)
        let key_controller = EventControllerKey::new();
        let g_weak = gallery.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            match keyval {
                Key::Left => {
                    if shift {
                        if let Some(curr) = *g_weak.current_index.borrow() {
                            if curr > 0 {
                                g_weak.select_range_to(curr - 1);
                            }
                        }
                    } else {
                        g_weak.select_prev();
                    }
                    glib::Propagation::Stop
                }
                Key::Right => {
                    if shift {
                        let total = g_weak.items.borrow().len();
                        if let Some(curr) = *g_weak.current_index.borrow() {
                            if curr + 1 < total {
                                g_weak.select_range_to(curr + 1);
                            }
                        }
                    } else {
                        g_weak.select_next();
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

    pub fn set_current_folder(&self, folder: Option<&Path>) {
        *self.current_folder.borrow_mut() = folder.map(Path::to_path_buf);
    }

    pub fn get_selected_item_and_widget(&self) -> Option<(FileItem, gtk4::Widget)> {
        let idx = (*self.current_index.borrow())
            .or_else(|| self.selected_indices.borrow().iter().next().copied())?;
        let item = self.items.borrow().get(idx).cloned()?;
        let widget = self
            .carousel_widgets
            .borrow()
            .get(idx)
            .map(|w| w.clone().upcast())
            .unwrap_or_else(|| self.container.clone().upcast());
        Some((item, widget))
    }

    pub fn get_selected_items(&self) -> Vec<FileItem> {
        let items = self.items.borrow();
        self.selected_indices
            .borrow()
            .iter()
            .filter_map(|&idx| items.get(idx).cloned())
            .collect()
    }

    pub fn select_all(self: &Rc<Self>) {
        let total = self.items.borrow().len();
        if total == 0 {
            return;
        }
        {
            let mut sel = self.selected_indices.borrow_mut();
            sel.clear();
            for idx in 0..total {
                sel.insert(idx);
            }
        }
        if self.current_index.borrow().is_none() {
            *self.current_index.borrow_mut() = Some(0);
        }
        if self.selection_anchor.borrow().is_none() {
            *self.selection_anchor.borrow_mut() = Some(0);
        }
        self.sync_selection_visuals();
    }

    pub fn queue_select_name(&self, name: &str) {
        *self.pending_select_name.borrow_mut() = Some(name.to_string());
    }

    pub fn refresh_cut_states(&self) {
        let items = self.items.borrow();
        let cards = self.carousel_widgets.borrow();
        for (item, card) in items.iter().zip(cards.iter()) {
            crate::views::sync_cut_css_class(card, &item.path);
        }
    }

    pub fn set_items(self: &Rc<Self>, items: &[FileItem]) {
        *self.items.borrow_mut() = items.to_vec();
        *self.current_index.borrow_mut() = None;
        self.selected_indices.borrow_mut().clear();
        *self.selection_anchor.borrow_mut() = None;

        // Limpiar carrusel previo
        while let Some(child) = self.carousel_box.first_child() {
            self.carousel_box.remove(&child);
        }
        self.carousel_widgets.borrow_mut().clear();

        if items.is_empty() {
            self.pending_select_name.borrow_mut().take();
            self.show_empty_canvas();
            (self.on_selection_changed)(0);
            return;
        }

        // Poblar carrusel
        for (idx, item) in items.iter().enumerate() {
            let card = self.create_carousel_card(idx, item);
            self.carousel_box.append(&card);
            self.carousel_widgets.borrow_mut().push(card);
        }

        let target_idx = self
            .pending_select_name
            .borrow_mut()
            .take()
            .and_then(|target| items.iter().position(|it| it.name == target))
            .unwrap_or(0);
        self.select_index(target_idx);
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
        let total = self.items.borrow().len();
        if index < total {
            *self.current_index.borrow_mut() = Some(index);
            *self.selection_anchor.borrow_mut() = Some(index);
            {
                let mut sel = self.selected_indices.borrow_mut();
                sel.clear();
                sel.insert(index);
            }
            self.sync_selection_visuals();
        }
    }

    pub fn toggle_select_index(self: &Rc<Self>, index: usize) {
        let total = self.items.borrow().len();
        if index >= total {
            return;
        }
        let next_focus = {
            let mut sel = self.selected_indices.borrow_mut();
            if sel.contains(&index) {
                sel.remove(&index);
                sel.iter().next_back().copied()
            } else {
                sel.insert(index);
                Some(index)
            }
        };
        if next_focus.is_some() {
            *self.current_index.borrow_mut() = next_focus;
            *self.selection_anchor.borrow_mut() = next_focus;
        } else {
            *self.current_index.borrow_mut() = None;
            *self.selection_anchor.borrow_mut() = None;
        }
        self.sync_selection_visuals();
    }

    pub fn select_range_to(self: &Rc<Self>, index: usize) {
        let total = self.items.borrow().len();
        if index >= total {
            return;
        }
        let anchor = self
            .selection_anchor
            .borrow()
            .or(*self.current_index.borrow())
            .unwrap_or(index);
        {
            let mut sel = self.selected_indices.borrow_mut();
            sel.clear();
            for i in crate::views::inclusive_range(anchor, index) {
                sel.insert(i);
            }
        }
        *self.selection_anchor.borrow_mut() = Some(anchor);
        *self.current_index.borrow_mut() = Some(index);
        self.sync_selection_visuals();
    }

    fn sync_selection_visuals(&self) {
        let sel = self.selected_indices.borrow();
        let widgets = self.carousel_widgets.borrow();
        for (i, widget) in widgets.iter().enumerate() {
            if sel.contains(&i) {
                widget.add_css_class("branches-carousel-active");
            } else {
                widget.remove_css_class("branches-carousel-active");
            }
        }
        let count = sel.len();
        drop(sel);

        let items = self.items.borrow();
        if let Some(idx) = *self.current_index.borrow() {
            if let Some(item) = items.get(idx) {
                self.show_item_in_canvas(item);
            }
        } else if items.is_empty() {
            self.show_empty_canvas();
        }
        (self.on_selection_changed)(count);
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
        if item.is_hidden {
            card.add_css_class("branches-hidden-item");
        }
        if crate::operations::is_path_cut(&item.path) {
            card.add_css_class("branches-cut-item");
        }
        card.set_size_request(84, 100);
        card.set_valign(Align::Center);

        let thumb_widget: gtk4::Widget = {
            let img = crate::views::create_item_image(item, 48, Some("branches-carousel-thumb"));
            img.set_size_request(48, 48);
            img.set_halign(Align::Center);
            img.set_valign(Align::Center);
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
        click.set_button(0);
        let self_click = self.clone();
        let p_for_dbl = item.path.clone();
        let is_dir = item.is_dir;
        let on_nav = self.on_navigate.clone();
        let card_for_menu = card.clone();
        let pending_slow_rename: Rc<std::cell::Cell<Option<glib::SourceId>>> =
            Rc::new(std::cell::Cell::new(None));
        let cancel_slow_drag = pending_slow_rename.clone();

        click.connect_pressed(move |g, n_press, x, y| {
            let button = g.current_button();
            let state = g.current_event_state();
            if let Some(source_id) = pending_slow_rename.take() {
                source_id.remove();
            }
            if button == gdk::BUTTON_SECONDARY {
                if !self_click.selected_indices.borrow().contains(&index) {
                    self_click.select_index(index);
                }
                show_item_context_menu(
                    &card_for_menu,
                    &p_for_dbl,
                    is_dir,
                    Some((x, y)),
                    Some(&on_nav),
                    None,
                );
            } else if n_press == 1 {
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                let ctrl = state.intersects(
                    gdk::ModifierType::CONTROL_MASK
                        | gdk::ModifierType::SUPER_MASK
                        | gdk::ModifierType::META_MASK,
                );
                if shift {
                    self_click.select_range_to(index);
                } else if ctrl {
                    self_click.toggle_select_index(index);
                } else {
                    let was_single_selected = {
                        let sel = self_click.selected_indices.borrow();
                        sel.len() == 1 && sel.contains(&index)
                    };
                    self_click.select_index(index);
                    if was_single_selected {
                        let card_ren = card_for_menu.clone();
                        let path_ren = p_for_dbl.clone();
                        let on_nav_ren = on_nav.clone();
                        let slot = pending_slow_rename.clone();
                        let id = glib::timeout_add_local_once(
                            std::time::Duration::from_millis(450),
                            move || {
                                slot.set(None);
                                let parent_dir = path_ren.parent().map(|p| p.to_path_buf());
                                let on_renamed = parent_dir.map(|dir| {
                                    Rc::new(move |_new_path: PathBuf| {
                                        on_nav_ren(dir.clone());
                                    }) as crate::views::RenameCallback
                                });
                                crate::views::show_rename_popover(
                                    &card_ren,
                                    path_ren,
                                    is_dir,
                                    None,
                                    on_renamed,
                                );
                            },
                        );
                        pending_slow_rename.set(Some(id));
                    }
                }
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

        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let self_drag = self.clone();
        let path_drag_fallback = item.path.clone();
        drag_source.connect_prepare(move |_, _x, _y| {
            if !self_drag.selected_indices.borrow().contains(&index) {
                self_drag.select_index(index);
            }
            let paths: Vec<PathBuf> = self_drag
                .get_selected_items()
                .into_iter()
                .map(|it| it.path)
                .collect();
            if paths.is_empty() {
                Some(crate::views::build_file_drag_content_provider(&path_drag_fallback))
            } else {
                Some(crate::views::build_files_drag_content_provider(&paths))
            }
        });
        drag_source.connect_drag_begin(move |_, _| {
            if let Some(source_id) = cancel_slow_drag.take() {
                source_id.remove();
            }
        });
        let path_end = item.path.clone();
        let on_nav_end = self.on_navigate.clone();
        drag_source.connect_drag_end(move |_, drag, _| {
            if let Some(parent) = path_end.parent() {
                let _ = crate::views::refresh_if_moved_out(drag, &path_end, parent, &on_nav_end);
            }
        });
        if let Some(display) = gdk::Display::default() {
            let theme = gtk4::IconTheme::for_display(&display);
            let paintable = theme.lookup_by_gicon(
                &item.gicon,
                32,
                1,
                gtk4::TextDirection::Ltr,
                gtk4::IconLookupFlags::empty(),
            );
            drag_source.set_icon(Some(&paintable), 16, 16);
        }
        card.add_controller(drag_source);

        card
    }
}
