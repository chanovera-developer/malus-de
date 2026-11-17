use crate::fs_backend::{open_file, FileItem};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, GestureClick, Image, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow,
};
use malus_de_common::config::BranchesConfig;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct ListView {
    container: GtkBox,
    list_box: ListBox,
    items: Rc<RefCell<Vec<FileItem>>>,
    current_folder: Rc<RefCell<Option<PathBuf>>>,
    pending_select_name: Rc<RefCell<Option<String>>>,
    icon_size: Rc<Cell<i32>>,
    on_navigate: NavigateCallback,
    #[allow(dead_code)]
    on_selection_changed: SelectionCallback,
}

impl ListView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
        let icon_size = Rc::new(Cell::new(BranchesConfig::load().clamped_list_icon_size()));
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

        let current_folder = Rc::new(RefCell::new(None::<PathBuf>));
        let items = Rc::new(RefCell::new(Vec::<FileItem>::new()));
        let pending_select_name = Rc::new(RefCell::new(None::<String>));

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .build();

        let bg_click = GestureClick::new();
        bg_click.set_button(gdk::BUTTON_SECONDARY);
        let scrolled_menu = scrolled.clone();
        let folder_bg = current_folder.clone();
        let on_nav_bg = on_navigate.clone();
        let items_bg = items.clone();
        let list_box_bg = list_box.clone();
        let pending_bg = pending_select_name.clone();
        bg_click.connect_pressed(move |g, _n, x, y| {
            let Some(dir) = folder_bg.borrow().clone() else {
                return;
            };
            g.set_state(gtk4::EventSequenceState::Claimed);
            let nav = on_nav_bg.clone();
            let dir_reload = dir.clone();
            let items_lookup = items_bg.clone();
            let lb_lookup = list_box_bg.clone();
            let pending_create = pending_bg.clone();
            crate::views::show_background_create_menu(
                &scrolled_menu,
                x,
                y,
                dir,
                Rc::new(move |created_path| {
                    if let Some(new_name) =
                        created_path.file_name().map(|n| n.to_string_lossy().into_owned())
                    {
                        *pending_create.borrow_mut() = Some(new_name);
                    }
                    nav(dir_reload.clone());

                    let items_ref = items_lookup.clone();
                    let lb_ref = lb_lookup.clone();
                    let target_path = created_path;
                    let pending_rename = pending_create.clone();
                    let nav_rename = nav.clone();
                    let dir_rename = dir_reload.clone();
                    let on_renamed: crate::views::RenameCallback = Rc::new(move |renamed_path| {
                        if let Some(renamed_name) = renamed_path.file_name() {
                            *pending_rename.borrow_mut() =
                                Some(renamed_name.to_string_lossy().into_owned());
                        }
                        nav_rename(dir_rename.clone());
                    });
                    crate::views::schedule_inline_rename(
                        move || {
                            let items_guard = items_ref.borrow();
                            let (idx, item) = items_guard
                                .iter()
                                .enumerate()
                                .find(|(_, it)| it.path == target_path)?;
                            let row = lb_ref.row_at_index(idx as i32)?;
                            Some((item.path.clone(), item.is_dir, row.upcast()))
                        },
                        Some(on_renamed),
                    );
                }),
            );
        });
        scrolled.add_controller(bg_click);

        container.append(&scrolled);

        Self {
            container,
            list_box,
            items,
            current_folder,
            pending_select_name,
            icon_size,
            on_navigate,
            on_selection_changed,
        }
    }

    pub fn set_current_folder(&self, folder: Option<&Path>) {
        *self.current_folder.borrow_mut() = folder.map(Path::to_path_buf);
    }

    pub fn set_icon_size(&self, size: i32) {
        let clamped = size.clamp(
            BranchesConfig::MIN_LIST_ICON_SIZE as i32,
            BranchesConfig::MAX_LIST_ICON_SIZE as i32,
        );
        self.icon_size.set(clamped);
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn get_selected_item_and_widget(&self) -> Option<(FileItem, gtk4::Widget)> {
        let row = self.list_box.selected_rows().into_iter().next()?;
        let idx = row.index() as usize;
        let item = self.items.borrow().get(idx).cloned()?;
        Some((item, row.upcast()))
    }

    pub fn queue_select_name(&self, name: &str) {
        *self.pending_select_name.borrow_mut() = Some(name.to_string());
    }

    pub fn set_items(&self, items: &[FileItem]) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        *self.items.borrow_mut() = items.to_vec();

        let icon_size = self.icon_size.get();
        for item in items {
            let row = create_list_row(item, icon_size, self.on_navigate.clone());
            self.list_box.append(&row);
        }

        if let Some(target_name) = self.pending_select_name.borrow_mut().take() {
            if let Some(idx) = items.iter().position(|it| it.name == target_name) {
                if let Some(row) = self.list_box.row_at_index(idx as i32) {
                    self.list_box.select_row(Some(&row));
                    row.grab_focus();
                }
            }
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

fn create_list_row(item: &FileItem, icon_size: i32, on_navigate: NavigateCallback) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.add_css_class("branches-list-row");

    // Columna 1: Icono + Nombre
    let name_box = GtkBox::new(Orientation::Horizontal, 8);
    name_box.set_hexpand(true);
    name_box.set_halign(Align::Start);

    let icon = Image::from_gicon(&item.gicon);
    icon.set_pixel_size(icon_size);
    name_box.append(&icon);

    let label_name = Label::builder()
        .label(&item.name)
        .halign(Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["branches-list-name-label"])
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

    // Gestos: Doble clic, segundo clic lento para renombrar (estilo Finder) y Clic derecho
    let gesture = GestureClick::new();
    gesture.set_button(0);
    let path = item.path.clone();
    let is_dir = item.is_dir;
    let on_nav_click = on_navigate.clone();

    let row_for_menu = row.clone();
    let label_for_click = label_name.clone();
    let pending_slow_rename: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let pending_slow_click = pending_slow_rename.clone();
    gesture.connect_pressed(move |g, n_press, x, y| {
        let button = g.current_button();
        if let Some(source_id) = pending_slow_click.take() {
            source_id.remove();
        }

        if button == gdk::BUTTON_PRIMARY {
            if n_press == 1 && row_for_menu.is_selected() {
                let clicked_on_name = label_for_click
                    .compute_bounds(&row_for_menu)
                    .map(|b| x <= (b.x() + b.width().max(180.0)) as f64)
                    .unwrap_or(true);
                if clicked_on_name {
                    let row_ren = row_for_menu.clone();
                    let path_ren = path.clone();
                    let on_nav_ren = on_nav_click.clone();
                    let slot = pending_slow_click.clone();
                    let id = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(450),
                        move || {
                            slot.set(None);
                            if !row_ren.is_selected() {
                                return;
                            }
                            let parent_dir = path_ren.parent().map(|p| p.to_path_buf());
                            let on_renamed = parent_dir.map(|dir| {
                                Rc::new(move |_new_path: PathBuf| {
                                    on_nav_ren(dir.clone());
                                }) as crate::views::RenameCallback
                            });
                            crate::views::show_rename_popover(
                                &row_ren,
                                path_ren,
                                is_dir,
                                None,
                                on_renamed,
                            );
                        },
                    );
                    pending_slow_click.set(Some(id));
                }
            } else if n_press >= 2 {
                g.set_state(gtk4::EventSequenceState::Claimed);
                if is_dir {
                    on_nav_click(path.clone());
                } else {
                    let _ = open_file(&path);
                }
            }
        } else if button == gdk::BUTTON_SECONDARY {
            g.set_state(gtk4::EventSequenceState::Claimed);
            if let Some(lb) = row_for_menu.parent().and_then(|p| p.downcast::<ListBox>().ok()) {
                if !row_for_menu.is_selected() {
                    lb.unselect_all();
                    lb.select_row(Some(&row_for_menu));
                }
            }
            show_item_context_menu(&hbox, &path, is_dir, Some((x, y)), Some(&on_nav_click), None);
        }
    });

    row.add_controller(gesture);

    // Arrastrar archivo (Drag & Drop con icono translúcido estilo macOS Finder)
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
    let path_for_drag = item.path.clone();
    drag_source.connect_prepare(move |_, _x, _y| {
        Some(crate::views::build_file_drag_content_provider(&path_for_drag))
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
    let cancel_slow_drag = pending_slow_rename;
    drag_source.connect_drag_begin(move |source, _| {
        if let Some(source_id) = cancel_slow_drag.take() {
            source_id.remove();
        }
        row_drag_begin.add_css_class("branches-icon-dragging");
        if let Some(ref paintable) = icon_paintable_begin {
            source.set_icon(Some(paintable), 16, 16);
        }
    });

    let row_drag_end = row.clone();
    let path_end = item.path.clone();
    let on_nav_end = on_navigate.clone();
    drag_source.connect_drag_end(move |_, drag, _| {
        row_drag_end.remove_css_class("branches-icon-dragging");
        if let Some(parent) = path_end.parent() {
            let _ = crate::views::refresh_if_moved_out(drag, &path_end, parent, &on_nav_end);
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
