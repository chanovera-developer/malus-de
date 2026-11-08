use crate::fs_backend::{open_file, open_in_terminal, FileItem};
use crate::views::{NavigateCallback, SelectionCallback};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, GestureClick, Image, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow,
};
use std::path::PathBuf;

pub struct ListView {
    container: GtkBox,
    list_box: ListBox,
    on_navigate: NavigateCallback,
    #[allow(dead_code)]
    on_selection_changed: SelectionCallback,
}

impl ListView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.add_css_class("branches-list-view");

        // Cabecera de columnas
        let header = create_list_header();
        container.append(&header);

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Multiple)
            .build();

        let sel_cb = on_selection_changed.clone();
        list_box.connect_selected_rows_changed(move |lb| {
            let count = lb.selected_rows().len();
            sel_cb(count);
        });

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .build();

        container.append(&scrolled);

        Self {
            container,
            list_box,
            on_navigate,
            on_selection_changed,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_items(&self, items: &[FileItem]) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        for item in items {
            let row = create_list_row(item, self.on_navigate.clone());
            self.list_box.append(&row);
        }
    }
}

fn create_list_header() -> GtkBox {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.add_css_class("branches-list-header");

    let col_name = Label::builder()
        .label("Nombre")
        .halign(Align::Start)
        .hexpand(true)
        .build();

    let col_date = Label::builder()
        .label("Fecha de modificación")
        .halign(Align::Start)
        .width_chars(20)
        .build();

    let col_size = Label::builder()
        .label("Tamaño")
        .halign(Align::End)
        .width_chars(12)
        .build();

    let col_kind = Label::builder()
        .label("Clase")
        .halign(Align::Start)
        .width_chars(16)
        .build();

    hbox.append(&col_name);
    hbox.append(&col_date);
    hbox.append(&col_size);
    hbox.append(&col_kind);

    hbox
}

fn create_list_row(item: &FileItem, on_navigate: NavigateCallback) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.add_css_class("branches-list-row");

    // Columna 1: Icono + Nombre
    let name_box = GtkBox::new(Orientation::Horizontal, 8);
    name_box.set_hexpand(true);
    name_box.set_halign(Align::Start);

    let icon = Image::from_gicon(&item.gicon);
    icon.set_pixel_size(18);
    name_box.append(&icon);

    let label_name = Label::builder()
        .label(&item.name)
        .halign(Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name_box.append(&label_name);

    // Columna 2: Fecha de modificación
    let label_date = Label::builder()
        .label(&item.modified_str)
        .halign(Align::Start)
        .width_chars(20)
        .build();

    // Columna 3: Tamaño
    let label_size = Label::builder()
        .label(&item.size_str)
        .halign(Align::End)
        .width_chars(12)
        .build();

    // Columna 4: Clase / Tipo
    let label_kind = Label::builder()
        .label(&item.kind_str)
        .halign(Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .width_chars(16)
        .build();

    hbox.append(&name_box);
    hbox.append(&label_date);
    hbox.append(&label_size);
    hbox.append(&label_kind);

    let row = ListBoxRow::builder()
        .child(&hbox)
        .build();

    // Gestos: Doble clic y Clic derecho
    let gesture = GestureClick::new();
    let path = item.path.clone();
    let is_dir = item.is_dir;
    let on_nav_click = on_navigate.clone();

    gesture.connect_pressed(move |g, n_press, _x, _y| {
        let button = g.current_button();

        if button == gdk::BUTTON_PRIMARY && n_press == 2 {
            if is_dir {
                on_nav_click(path.clone());
            } else {
                let _ = open_file(&path);
            }
        } else if button == gdk::BUTTON_SECONDARY {
            show_context_menu(&hbox, &path, is_dir);
        }
    });

    row.add_controller(gesture);

    // Arrastrar archivo (Drag & Drop con icono translúcido estilo macOS Finder)
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let path_for_drag = item.path.clone();
    drag_source.connect_prepare(move |_, _x, _y| {
        let file = gio::File::for_path(&path_for_drag);
        let file_list = gdk::FileList::from_array(&[file]);
        Some(gdk::ContentProvider::for_value(&file_list.to_value()))
    });

    // Pre-adjuntar el icono para respuesta visual instantánea (sin lapso de cursor vacío)
    let display = gdk::Display::default();
    let icon_paintable = display.map(|d| {
        let theme = gtk4::IconTheme::for_display(&d);
        theme.lookup_by_gicon(
            &item.gicon,
            32,
            1,
            gtk4::TextDirection::Ltr,
            gtk4::IconLookupFlags::empty(),
        )
    });
    if let Some(ref paintable) = icon_paintable {
        drag_source.set_icon(Some(paintable), 16, 16);
    }

    let row_drag_begin = row.clone();
    let icon_paintable_begin = icon_paintable.clone();
    drag_source.connect_drag_begin(move |source, _| {
        row_drag_begin.add_css_class("branches-icon-dragging");
        if let Some(ref paintable) = icon_paintable_begin {
            source.set_icon(Some(paintable), 16, 16);
        }
    });

    let row_drag_end = row.clone();
    let path_drag_end = item.path.clone();
    let on_nav_drag_end = on_navigate.clone();
    drag_source.connect_drag_end(move |_, _, delete_data| {
        row_drag_end.remove_css_class("branches-icon-dragging");
        if delete_data {
            let file = gio::File::for_path(&path_drag_end);
            if file.trash(gio::Cancellable::NONE).is_err() {
                if path_drag_end.is_dir() {
                    let _ = std::fs::remove_dir_all(&path_drag_end);
                } else {
                    let _ = std::fs::remove_file(&path_drag_end);
                }
            }
            if let Some(parent) = path_drag_end.parent() {
                on_nav_drag_end(parent.to_path_buf());
            }
        }
    });
    row.add_controller(drag_source);

    // Si es carpeta, permitir soltar archivos encima con resaltado azul acrílico estilo Finder
    if is_dir {
        let folder_drop = gtk4::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        let row_hover = row.clone();
        folder_drop.connect_enter(move |_, _, _| {
            row_hover.add_css_class("folder-drop-hover");
            gdk::DragAction::MOVE
        });
        folder_drop.connect_motion(move |_, _, _| {
            gdk::DragAction::MOVE
        });

        let row_leave = row.clone();
        folder_drop.connect_leave(move |_| {
            row_leave.remove_css_class("folder-drop-hover");
        });

        let row_drop = row.clone();
        let target_dir = item.path.clone();
        let on_nav_reload = on_navigate.clone();
        folder_drop.connect_drop(move |_, value, _, _| {
            row_drop.remove_css_class("folder-drop-hover");
            if let Ok(file_list) = value.get::<gdk::FileList>() {
                let mut moved = false;
                for file in file_list.files() {
                    if let Some(src) = file.path() {
                        if let Some(fname) = src.file_name() {
                            let target = target_dir.join(fname);
                            if target != src {
                                let dest_file = gio::File::for_path(&target);
                                if file.move_(
                                    &dest_file,
                                    gio::FileCopyFlags::NONE,
                                    gio::Cancellable::NONE,
                                    None,
                                ).is_ok() || std::fs::rename(&src, &target).is_ok() {
                                    moved = true;
                                }
                            }
                        }
                    }
                }
                if moved {
                    if let Some(parent) = target_dir.parent() {
                        on_nav_reload(parent.to_path_buf());
                    }
                    return true;
                }
            }
            false
        });
        row.add_controller(folder_drop);
    }

    row
}

fn show_context_menu(widget: &impl IsA<gtk4::Widget>, path: &PathBuf, is_dir: bool) {
    let popover = gtk4::Popover::new();
    popover.set_parent(widget);

    let vbox = GtkBox::new(Orientation::Vertical, 2);
    vbox.set_margin_start(4);
    vbox.set_margin_end(4);
    vbox.set_margin_top(4);
    vbox.set_margin_bottom(4);

    let p = path.clone();
    let pop_clone = popover.clone();
    let open_btn = gtk4::Button::with_label(if is_dir { "Abrir carpeta" } else { "Abrir" });
    open_btn.connect_clicked(move |_| {
        pop_clone.popdown();
        let _ = open_file(&p);
    });
    vbox.append(&open_btn);

    let p_term = path.clone();
    let pop_clone = popover.clone();
    let term_btn = gtk4::Button::with_label("Abrir en Terminal");
    term_btn.connect_clicked(move |_| {
        pop_clone.popdown();
        open_in_terminal(&p_term);
    });
    vbox.append(&term_btn);

    let p_copy = path.clone();
    let pop_clone = popover.clone();
    let copy_btn = gtk4::Button::with_label("Copiar ruta");
    copy_btn.connect_clicked(move |_| {
        pop_clone.popdown();
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&p_copy.to_string_lossy());
        }
    });
    vbox.append(&copy_btn);

    popover.set_child(Some(&vbox));
    popover.popup();
}
