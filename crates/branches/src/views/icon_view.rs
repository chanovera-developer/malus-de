use crate::db::{self, LayoutMode};
use crate::fs_backend::{open_file, open_in_terminal, FileItem};
use crate::views::{NavigateCallback, SelectionCallback};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Fixed, GestureClick, Image, Label,
    Orientation, PolicyType, Popover, ScrolledWindow,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

const GRID_CELL_W: i32 = 110;
const GRID_CELL_H: i32 = 110;
const MARGIN_LEFT: i32 = 24;
const MARGIN_TOP: i32 = 24;

struct ItemWidgetData {
    widget: GtkBox,
    item: FileItem,
    pos_x: i32,
    pos_y: i32,
}

pub struct IconView {
    container: ScrolledWindow,
    fixed_canvas: Fixed,
    current_folder: Rc<RefCell<PathBuf>>,
    layout_mode: Rc<RefCell<LayoutMode>>,
    item_widgets: Rc<RefCell<HashMap<String, Rc<RefCell<ItemWidgetData>>>>>,
    selected_items: Rc<RefCell<HashSet<String>>>,
    on_navigate: NavigateCallback,
    on_selection_changed: SelectionCallback,
}

impl IconView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
        let fixed_canvas = Fixed::builder()
            .hexpand(true)
            .vexpand(true)
            .css_classes(["branches-icon-canvas"])
            .build();

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .css_classes(["branches-icon-view"])
            .child(&fixed_canvas)
            .build();

        let current_folder = Rc::new(RefCell::new(PathBuf::from("/")));
        let layout_mode = Rc::new(RefCell::new(LayoutMode::SnapToGrid));
        let item_widgets = Rc::new(RefCell::new(HashMap::new()));
        let selected_items = Rc::new(RefCell::new(HashSet::new()));

        let view = Self {
            container,
            fixed_canvas,
            current_folder,
            layout_mode,
            item_widgets,
            selected_items,
            on_navigate,
            on_selection_changed,
        };

        // Gesto en el lienzo de fondo: deseleccionar al hacer clic fuera y menú contextual del lienzo
        view.setup_canvas_gestures();

        view
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.container
    }

    pub fn set_items(&self, items: &[FileItem]) {
        // Limpiar widgets anteriores
        while let Some(child) = self.fixed_canvas.first_child() {
            self.fixed_canvas.remove(&child);
        }
        self.item_widgets.borrow_mut().clear();
        self.selected_items.borrow_mut().clear();
        (self.on_selection_changed)(0);

        if items.is_empty() {
            self.fixed_canvas.set_size_request(400, 300);
            return;
        }

        let folder = items[0].path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/"));
        *self.current_folder.borrow_mut() = folder.clone();

        // Cargar posiciones y modo de layout desde SQLite
        let mut db_positions = HashMap::new();
        let mut current_mode = LayoutMode::SnapToGrid;

        if let Ok(conn) = db::open_db() {
            db_positions = db::get_positions(&conn, &folder);
            current_mode = db::get_layout_mode(&conn, &folder);
        }
        *self.layout_mode.borrow_mut() = current_mode;

        // Calcular colocación para cada elemento
        let mut occupied_slots: HashSet<(i32, i32)> = HashSet::new();
        for (_, (x, y)) in &db_positions {
            let col = (*x - MARGIN_LEFT) / GRID_CELL_W;
            let row = (*y - MARGIN_TOP) / GRID_CELL_H;
            occupied_slots.insert((col, row));
        }

        let mut next_col = 0;
        let mut next_row = 0;
        let mut max_x = 400;
        let mut max_y = 300;

        let mut new_positions_to_save: HashMap<String, (i32, i32)> = HashMap::new();

        for item in items {
            let (pos_x, pos_y) = if current_mode != LayoutMode::Auto && db_positions.contains_key(&item.name) {
                let pos = db_positions[&item.name];
                (pos.0, pos.1)
            } else {
                // Asignar siguiente celda libre
                while occupied_slots.contains(&(next_col, next_row)) {
                    next_col += 1;
                    if next_col >= 8 {
                        next_col = 0;
                        next_row += 1;
                    }
                }
                let x = MARGIN_LEFT + next_col * GRID_CELL_W;
                let y = MARGIN_TOP + next_row * GRID_CELL_H;
                occupied_slots.insert((next_col, next_row));

                next_col += 1;
                if next_col >= 8 {
                    next_col = 0;
                    next_row += 1;
                }

                if current_mode != LayoutMode::Auto {
                    new_positions_to_save.insert(item.name.clone(), (x, y));
                }
                (x, y)
            };

            max_x = max_x.max(pos_x + GRID_CELL_W);
            max_y = max_y.max(pos_y + GRID_CELL_H);

            let widget = self.create_spatial_icon_widget(item, pos_x, pos_y);
            self.fixed_canvas.put(&widget, pos_x as f64, pos_y as f64);

            let data = Rc::new(RefCell::new(ItemWidgetData {
                widget,
                item: item.clone(),
                pos_x,
                pos_y,
            }));
            self.item_widgets.borrow_mut().insert(item.name.clone(), data);
        }

        // Guardar en segundo plano las posiciones nuevas generadas
        if !new_positions_to_save.is_empty() {
            if let Ok(mut conn) = db::open_db() {
                let _ = db::save_all_positions(&mut conn, &folder, &new_positions_to_save);
            }
        }

        // Ajustar el tamaño del lienzo para permitir scroll amplio
        self.fixed_canvas.set_size_request(max_x + 80, max_y + 80);
    }

    fn create_spatial_icon_widget(&self, item: &FileItem, _init_x: i32, _init_y: i32) -> GtkBox {
        let vbox = GtkBox::new(Orientation::Vertical, 4);
        vbox.set_halign(Align::Center);
        vbox.set_valign(Align::Start);
        vbox.add_css_class("branches-icon-item");

        let img = Image::from_gicon(&item.gicon);
        img.set_pixel_size(48);
        vbox.append(&img);

        let label = Label::builder()
            .label(&item.name)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .max_width_chars(12)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .lines(2)
            .halign(Align::Center)
            .css_classes(["branches-icon-label"])
            .build();
        vbox.append(&label);

        // 1. Doble clic para abrir y Clic derecho para menú contextual
        let click_gesture = GestureClick::new();
        let path = item.path.clone();
        let is_dir = item.is_dir;
        let on_nav = self.on_navigate.clone();
        let sel_items = self.selected_items.clone();
        let name_for_click = item.name.clone();
        let vbox_for_click = vbox.clone();
        let on_sel_cb = self.on_selection_changed.clone();

        click_gesture.connect_pressed(move |g, n_press, _x, _y| {
            let button = g.current_button();

            if button == gdk::BUTTON_PRIMARY {
                if n_press == 1 {
                    // Selección simple
                    sel_items.borrow_mut().clear();
                    sel_items.borrow_mut().insert(name_for_click.clone());
                    vbox_for_click.add_css_class("selected");
                    on_sel_cb(1);
                } else if n_press == 2 {
                    if is_dir {
                        on_nav(path.clone());
                    } else {
                        let _ = open_file(&path);
                    }
                }
            } else if button == gdk::BUTTON_SECONDARY {
                show_context_menu(&vbox_for_click, &path, is_dir);
            }
        });
        vbox.add_controller(click_gesture);

        // 2. Drag & Drop entre ventanas/paneles (DragSource Wayland con icono translúcido estilo Finder)
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
                48,
                1,
                gtk4::TextDirection::Ltr,
                gtk4::IconLookupFlags::empty(),
            )
        });
        if let Some(ref paintable) = icon_paintable {
            drag_source.set_icon(Some(paintable), 24, 24);
        }

        let vbox_drag_begin = vbox.clone();
        let icon_paintable_begin = icon_paintable.clone();
        drag_source.connect_drag_begin(move |source, _drag| {
            vbox_drag_begin.add_css_class("branches-icon-dragging");
            if let Some(ref paintable) = icon_paintable_begin {
                source.set_icon(Some(paintable), 24, 24);
            }
        });

        let vbox_drag_end = vbox.clone();
        drag_source.connect_drag_end(move |_, _drag, _| {
            vbox_drag_end.remove_css_class("branches-icon-dragging");
        });
        vbox.add_controller(drag_source);

        // Si es carpeta, permitir soltar archivos encima con resaltado azul acrílico estilo Finder
        if is_dir {
            let folder_drop = gtk4::DropTarget::new(
                gdk::FileList::static_type(),
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );
            let vbox_hover = vbox.clone();
            folder_drop.connect_enter(move |_, _x, _y| {
                vbox_hover.add_css_class("folder-drop-hover");
                gdk::DragAction::MOVE
            });

            let vbox_leave = vbox.clone();
            folder_drop.connect_leave(move |_| {
                vbox_leave.remove_css_class("folder-drop-hover");
            });

            let vbox_drop = vbox.clone();
            let dest_dir = item.path.clone();
            let on_nav_reload = self.on_navigate.clone();
            let cur_folder = self.current_folder.clone();
            folder_drop.connect_drop(move |_, value, _, _| {
                vbox_drop.remove_css_class("folder-drop-hover");
                if let Ok(file_list) = value.get::<gdk::FileList>() {
                    let mut moved = false;
                    for file in file_list.files() {
                        if let Some(src) = file.path() {
                            if let Some(fname) = src.file_name() {
                                let target = dest_dir.join(fname);
                                if target != src {
                                    if std::fs::rename(&src, &target).is_ok() || std::fs::copy(&src, &target).is_ok() {
                                        moved = true;
                                    }
                                }
                            }
                        }
                    }
                    if moved {
                        let cf = cur_folder.borrow().clone();
                        on_nav_reload(cf);
                        return true;
                    }
                }
                false
            });
            vbox.add_controller(folder_drop);
        }

        vbox
    }

    fn setup_canvas_gestures(&self) {
        let click = GestureClick::new();
        let sel_items = self.selected_items.clone();
        let widgets = self.item_widgets.clone();
        let sel_cb = self.on_selection_changed.clone();
        let fixed = self.fixed_canvas.clone();
        let folder_ref = self.current_folder.clone();
        let layout_ref = self.layout_mode.clone();

        let self_weak: Rc<RefCell<Option<IconViewWeak>>> = Rc::new(RefCell::new(None));
        *self_weak.borrow_mut() = Some(IconViewWeak {
            fixed: self.fixed_canvas.clone(),
            current_folder: self.current_folder.clone(),
            layout_mode: self.layout_mode.clone(),
            item_widgets: self.item_widgets.clone(),
        });

        let weak_for_click = self_weak.clone();

        click.connect_pressed(move |g, _n, x, y| {
            let button = g.current_button();

            if button == gdk::BUTTON_PRIMARY {
                // Clic en fondo vacío: deseleccionar todo
                for (_, data) in widgets.borrow().iter() {
                    data.borrow().widget.remove_css_class("selected");
                }
                sel_items.borrow_mut().clear();
                sel_cb(0);
            } else if button == gdk::BUTTON_SECONDARY {
                // Menú contextual del lienzo de fondo
                if let Some(ref w) = *weak_for_click.borrow() {
                    show_canvas_context_menu(&fixed, x, y, folder_ref.clone(), layout_ref.clone(), w);
                }
            }
        });

        self.fixed_canvas.add_controller(click);

        // Destino para soltar archivos (tanto reorganización interna como desde Escritorio)
        let canvas_drop = gtk4::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        let folder_drop_ref = self.current_folder.clone();
        let layout_drop_ref = self.layout_mode.clone();
        let fixed_drop_canvas = self.fixed_canvas.clone();
        let widgets_drop_map = self.item_widgets.clone();
        let on_nav_drop = self.on_navigate.clone();

        canvas_drop.connect_drop(move |_, value, x, y| {
            if let Ok(file_list) = value.get::<gdk::FileList>() {
                let cur_folder = folder_drop_ref.borrow().clone();
                let mode = *layout_drop_ref.borrow();
                let mut moved_external = false;
                let mut moved_internal = false;

                for file in file_list.files() {
                    if let Some(src) = file.path() {
                        let is_internal = src.parent().map(|p| p == cur_folder).unwrap_or(false);
                        if is_internal {
                            // Reordenamiento local en el lienzo sin arrastrar el widget en movimiento
                            if let Some(fname) = src.file_name() {
                                let fname_str = fname.to_string_lossy().to_string();
                                let raw_x = (x - 40.0).max(10.0) as i32;
                                let raw_y = (y - 40.0).max(10.0) as i32;
                                let (final_x, final_y) = match mode {
                                    LayoutMode::SnapToGrid => {
                                        let col = ((raw_x - MARGIN_LEFT + GRID_CELL_W / 2) / GRID_CELL_W).max(0);
                                        let row = ((raw_y - MARGIN_TOP + GRID_CELL_H / 2) / GRID_CELL_H).max(0);
                                        (MARGIN_LEFT + col * GRID_CELL_W, MARGIN_TOP + row * GRID_CELL_H)
                                    }
                                    LayoutMode::Freeform => (raw_x, raw_y),
                                    LayoutMode::Auto => (raw_x, raw_y),
                                };

                                if let Some(item_data) = widgets_drop_map.borrow().get(&fname_str) {
                                    let mut d = item_data.borrow_mut();
                                    d.pos_x = final_x;
                                    d.pos_y = final_y;
                                    fixed_drop_canvas.move_(&d.widget, final_x as f64, final_y as f64);
                                    d.widget.add_css_class("landing");
                                    let w_clone = d.widget.clone();
                                    glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                                        w_clone.remove_css_class("landing");
                                    });
                                }

                                if let Ok(conn) = db::open_db() {
                                    let _ = db::save_position(&conn, &cur_folder, &fname_str, final_x, final_y);
                                }
                                moved_internal = true;
                            }
                        } else {
                            if let Some(fname) = src.file_name() {
                                let dest = cur_folder.join(fname);
                                if dest != src {
                                    if std::fs::rename(&src, &dest).is_ok() || std::fs::copy(&src, &dest).is_ok() {
                                        moved_external = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if moved_external {
                    on_nav_drop(cur_folder);
                    return true;
                }
                if moved_internal {
                    return true;
                }
            }
            false
        });
        self.fixed_canvas.add_controller(canvas_drop);
    }
}

struct IconViewWeak {
    fixed: Fixed,
    current_folder: Rc<RefCell<PathBuf>>,
    #[allow(dead_code)]
    layout_mode: Rc<RefCell<LayoutMode>>,
    item_widgets: Rc<RefCell<HashMap<String, Rc<RefCell<ItemWidgetData>>>>>,
}

impl IconViewWeak {
    fn rearrange(&self, by_name: bool) {
        let mut list: Vec<_> = self.item_widgets.borrow().values().cloned().collect();
        if by_name {
            list.sort_by(|a, b| a.borrow().item.name.to_lowercase().cmp(&b.borrow().item.name.to_lowercase()));
        }

        let mut positions = HashMap::new();
        let folder = self.current_folder.borrow().clone();

        for (idx, data_rc) in list.iter().enumerate() {
            let col = (idx % 8) as i32;
            let row = (idx / 8) as i32;
            let x = MARGIN_LEFT + col * GRID_CELL_W;
            let y = MARGIN_TOP + row * GRID_CELL_H;

            let mut d = data_rc.borrow_mut();
            d.pos_x = x;
            d.pos_y = y;
            self.fixed.move_(&d.widget, x as f64, y as f64);
            positions.insert(d.item.name.clone(), (x, y));
        }

        if let Ok(mut conn) = db::open_db() {
            let _ = db::save_all_positions(&mut conn, &folder, &positions);
        }
    }
}

fn show_canvas_context_menu(
    parent: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    folder_ref: Rc<RefCell<PathBuf>>,
    layout_ref: Rc<RefCell<LayoutMode>>,
    view_weak: &IconViewWeak,
) {
    let popover = Popover::new();
    popover.set_parent(parent);

    let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));

    let vbox = GtkBox::new(Orientation::Vertical, 4);
    vbox.set_margin_start(6);
    vbox.set_margin_end(6);
    vbox.set_margin_top(6);
    vbox.set_margin_bottom(6);

    let lbl_layout = Label::builder()
        .label("Disposición de iconos")
        .halign(Align::Start)
        .css_classes(["dim-label", "bold"])
        .build();
    vbox.append(&lbl_layout);

    // Opciones de layout
    let pop_clone = popover.clone();
    let l_ref = layout_ref.clone();
    let f_ref = folder_ref.clone();
    let btn_snap = gtk4::Button::with_label("Alinear a la cuadrícula (Snap to Grid)");
    btn_snap.connect_clicked(move |_| {
        pop_clone.popdown();
        *l_ref.borrow_mut() = LayoutMode::SnapToGrid;
        if let Ok(conn) = db::open_db() {
            let _ = db::set_layout_mode(&conn, &f_ref.borrow(), LayoutMode::SnapToGrid);
        }
    });
    vbox.append(&btn_snap);

    let pop_clone2 = popover.clone();
    let l_ref2 = layout_ref.clone();
    let f_ref2 = folder_ref.clone();
    let btn_free = gtk4::Button::with_label("Colocación libre (Freeform)");
    btn_free.connect_clicked(move |_| {
        pop_clone2.popdown();
        *l_ref2.borrow_mut() = LayoutMode::Freeform;
        if let Ok(conn) = db::open_db() {
            let _ = db::set_layout_mode(&conn, &f_ref2.borrow(), LayoutMode::Freeform);
        }
    });
    vbox.append(&btn_free);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    vbox.append(&sep);

    // Limpiar / Reorganizar automáticamente
    let pop_clone3 = popover.clone();
    let fixed_for_clean = view_weak.fixed.clone();
    let cur_folder = folder_ref.borrow().clone();
    let item_widgets_clean = view_weak.item_widgets.clone();

    let btn_clean = gtk4::Button::with_label("Reordenar cuadrícula");
    btn_clean.connect_clicked(move |_| {
        pop_clone3.popdown();
        let weak_helper = IconViewWeak {
            fixed: fixed_for_clean.clone(),
            current_folder: Rc::new(RefCell::new(cur_folder.clone())),
            layout_mode: Rc::new(RefCell::new(LayoutMode::SnapToGrid)),
            item_widgets: item_widgets_clean.clone(),
        };
        weak_helper.rearrange(true);
    });
    vbox.append(&btn_clean);

    popover.set_child(Some(&vbox));
    popover.popup();
}

fn show_context_menu(widget: &impl IsA<gtk4::Widget>, path: &PathBuf, is_dir: bool) {
    let popover = Popover::new();
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
