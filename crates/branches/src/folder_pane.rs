use crate::fs_backend::{load_directory, FileItem};
use crate::state::AppState;
use crate::views::column_view::ColumnView;
use crate::views::gallery_view::GalleryView;
use crate::views::icon_view::IconView;
use crate::views::list_view::ListView;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, GestureClick, Image, Label, Orientation, Stack,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    Primary,
    Secondary,
}

pub struct FolderPane {
    pub id: PaneId,
    pub state: Rc<RefCell<AppState>>,
    container: GtkBox,
    view_stack: Stack,
    icon_view: Rc<IconView>,
    list_view: Rc<ListView>,
    pub column_view: Rc<ColumnView>,
    pub gallery_view: Rc<GalleryView>,
    pane_header: GtkBox,
    pane_title: Label,
    items_cache: Rc<RefCell<Vec<FileItem>>>,
    on_activated: Rc<dyn Fn(PaneId)>,
    on_path_changed: Rc<dyn Fn(PaneId, PathBuf)>,
    on_selection_changed: Rc<dyn Fn(PaneId, usize, PathBuf)>,
    #[allow(dead_code)]
    on_close_split: Option<Rc<dyn Fn()>>,
}

impl FolderPane {
    pub fn new(
        id: PaneId,
        initial_path: PathBuf,
        on_activated: Rc<dyn Fn(PaneId)>,
        on_path_changed: Rc<dyn Fn(PaneId, PathBuf)>,
        on_selection_changed: Rc<dyn Fn(PaneId, usize, PathBuf)>,
        on_close_split: Option<Rc<dyn Fn()>>,
    ) -> Rc<Self> {
        let state = Rc::new(RefCell::new(AppState::new(initial_path.clone())));
        let items_cache = Rc::new(RefCell::new(Vec::new()));

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.add_css_class("branches-folder-pane");
        if id == PaneId::Primary {
            container.add_css_class("branches-pane-active");
        }

        // Mini cabecera de panel (se muestra en modo Split View)
        let pane_header = GtkBox::new(Orientation::Horizontal, 8);
        pane_header.add_css_class("branches-pane-header");
        pane_header.set_visible(false); // Oculta por defecto en modo individual

        let pane_icon = Image::from_icon_name("folder-symbolic");
        pane_icon.set_pixel_size(14);
        pane_header.append(&pane_icon);

        let pane_title = Label::builder()
            .label("Carpeta")
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["branches-pane-title"])
            .build();
        pane_header.append(&pane_title);

        // Si es el panel secundario, añadir botón para cerrar el split
        if let Some(close_cb) = on_close_split.clone() {
            let close_btn = Button::builder()
                .icon_name("window-close-symbolic")
                .tooltip_text("Cerrar división de pantalla")
                .valign(Align::Center)
                .css_classes(["flat", "circular", "branches-pane-close-btn"])
                .build();
            close_btn.connect_clicked(move |_| {
                close_cb();
            });
            pane_header.append(&close_btn);
        }

        container.append(&pane_header);

        // Vistas de contenido
        let self_weak: Rc<RefCell<Option<Rc<FolderPane>>>> = Rc::new(RefCell::new(None));
        let self_for_nav = self_weak.clone();

        let on_navigate = Rc::new(move |target: PathBuf| {
            if let Some(p) = self_for_nav.borrow().as_ref() {
                p.navigate_to(target);
            }
        });

        let self_for_sel = self_weak.clone();
        let on_selection = Rc::new(move |count: usize| {
            if let Some(p) = self_for_sel.borrow().as_ref() {
                let curr = p.state.borrow().current_path.clone();
                (p.on_selection_changed)(p.id, count, curr);
            }
        });

        let icon_view = Rc::new(IconView::new(on_navigate.clone(), on_selection.clone()));
        let list_view = Rc::new(ListView::new(on_navigate.clone(), on_selection.clone()));
        let column_view = Rc::new(ColumnView::new(on_navigate.clone(), on_selection.clone()));
        let gallery_view = GalleryView::new(on_navigate.clone(), on_selection.clone());

        let view_stack = Stack::builder()
            .vexpand(true)
            .hexpand(true)
            .css_classes(["branches-view-stack", "branches-content-area"])
            .build();

        view_stack.add_named(icon_view.widget(), Some("icons"));
        view_stack.add_named(list_view.widget(), Some("list"));
        view_stack.add_named(column_view.widget(), Some("columns"));
        view_stack.add_named(gallery_view.widget(), Some("gallery"));

        container.append(&view_stack);

        // Gesto para activar el panel al hacer clic en él
        let click_gesture = GestureClick::new();
        let act_cb = on_activated.clone();
        click_gesture.connect_pressed(move |_, _, _, _| {
            act_cb(id);
        });
        container.add_controller(click_gesture);

        // Soporte de Drag & Drop: recibir archivos soltados en este panel
        setup_drop_target(&container, state.clone(), {
            let self_for_drop = self_weak.clone();
            move || {
                if let Some(p) = self_for_drop.borrow().as_ref() {
                    p.reload();
                }
            }
        });

        let folder_pane = Rc::new(Self {
            id,
            state,
            container,
            view_stack,
            icon_view,
            list_view,
            column_view,
            gallery_view,
            pane_header,
            pane_title,
            items_cache,
            on_activated,
            on_path_changed,
            on_selection_changed,
            on_close_split,
        });

        *self_weak.borrow_mut() = Some(folder_pane.clone());
        folder_pane.reload();

        folder_pane
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_split_mode_active(&self, active: bool) {
        self.pane_header.set_visible(active);
    }

    pub fn set_is_active(&self, active: bool) {
        if active {
            self.container.add_css_class("branches-pane-active");
        } else {
            self.container.remove_css_class("branches-pane-active");
        }
    }

    pub fn set_view_mode(&self, mode_name: &str) {
        self.view_stack.set_visible_child_name(mode_name);
    }

    pub fn navigate_to(&self, target: PathBuf) {
        let changed = self.state.borrow_mut().navigate_to(target.clone());
        if changed {
            (self.on_activated)(self.id);
            self.reload();
            (self.on_path_changed)(self.id, target);
        }
    }

    pub fn go_back(&self) -> bool {
        if let Some(prev) = self.state.borrow_mut().go_back() {
            self.reload();
            (self.on_path_changed)(self.id, prev);
            true
        } else {
            false
        }
    }

    pub fn go_forward(&self) -> bool {
        if let Some(next) = self.state.borrow_mut().go_forward() {
            self.reload();
            (self.on_path_changed)(self.id, next);
            true
        } else {
            false
        }
    }

    pub fn go_up(&self) -> bool {
        if let Some(parent) = self.state.borrow_mut().go_up() {
            self.reload();
            (self.on_path_changed)(self.id, parent);
            true
        } else {
            false
        }
    }

    pub fn toggle_hidden(&self) {
        let curr = self.state.borrow().show_hidden;
        self.state.borrow_mut().show_hidden = !curr;
        self.reload();
    }

    pub fn set_search_query(&self, query: String) {
        self.state.borrow_mut().search_query = query;
        self.reload();
    }

    pub fn reload(&self) {
        let (current_path, show_hidden, sort_by, sort_order, search_query) = {
            let s = self.state.borrow();
            (
                s.current_path.clone(),
                s.show_hidden,
                s.sort_by,
                s.sort_order,
                s.search_query.clone(),
            )
        };

        let folder_name = if current_path == PathBuf::from("/") {
            "Macintosh HD".to_string()
        } else {
            current_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Carpeta".to_string())
        };
        self.pane_title.set_label(&folder_name);

        match load_directory(&current_path, show_hidden, sort_by, sort_order, &search_query) {
            Ok(items) => {
                let count = items.len();
                self.icon_view.set_items(&items);
                self.list_view.set_items(&items);
                self.column_view.set_root(&current_path, &items, show_hidden, sort_by, sort_order);
                self.gallery_view.set_items(&items);
                *self.items_cache.borrow_mut() = items;
                (self.on_selection_changed)(self.id, count, current_path);
            }
            Err(e) => {
                tracing::error!("Error al listar directorio en panel {:?}: {}", self.id, e);
            }
        }
    }

    pub fn current_path(&self) -> PathBuf {
        self.state.borrow().current_path.clone()
    }

    pub fn can_go_back(&self) -> bool {
        self.state.borrow().can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.state.borrow().can_go_forward()
    }

    pub fn items_count(&self) -> usize {
        self.items_cache.borrow().len()
    }

    pub fn get_selected_item(&self) -> Option<FileItem> {
        let mode = self.view_stack.visible_child_name().unwrap_or_default();
        if mode == "columns" {
            self.column_view.get_selected_item()
        } else {
            self.items_cache.borrow().first().cloned()
        }
    }

    pub fn duplicate_selected(&self) {
        if let Some(item) = self.get_selected_item() {
            if let Ok(_) = crate::operations::duplicate_item(&item.path) {
                self.reload();
            }
        }
    }

    pub fn trash_selected(&self) {
        if let Some(item) = self.get_selected_item() {
            if let Ok(_) = crate::operations::trash_item(&item.path) {
                self.reload();
            }
        }
    }

    pub fn show_tag_filter(&self, tag_name: &str) {
        if let Ok(conn) = crate::db::open_db() {
            let paths = crate::db::get_files_by_tag(&conn, tag_name);
            let mut items = Vec::new();
            for p in paths {
                if let Some(item) = FileItem::from_path(&p) {
                    items.push(item);
                }
            }
            self.pane_title.set_label(&format!("Etiqueta: {}", tag_name));
            let count = items.len();
            self.icon_view.set_items(&items);
            self.list_view.set_items(&items);
            let s = self.state.borrow();
            self.column_view.set_root(&s.current_path, &items, s.show_hidden, s.sort_by, s.sort_order);
            self.gallery_view.set_items(&items);
            *self.items_cache.borrow_mut() = items;
            (self.on_selection_changed)(self.id, count, s.current_path.clone());
        }
    }

    pub fn set_tag_on_selected(&self, tag_name: &str, tag_color: &str) {
        if let Some(item) = self.get_selected_item() {
            if let Ok(conn) = crate::db::open_db() {
                let _ = crate::db::set_file_tag(&conn, &item.path, tag_name, tag_color);
            }
        }
    }
}


fn setup_drop_target<F: Fn() + 'static>(
    widget: &impl IsA<gtk4::Widget>,
    state: Rc<RefCell<AppState>>,
    on_reload: F,
) {
    let drop_target = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let on_reload = Rc::new(on_reload);

    drop_target.connect_drop(move |_, value, _x, _y| {
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let dest_dir = state.borrow().current_path.clone();
            let mut moved_any = false;

            for file in file_list.files() {
                if let Some(src_path) = file.path() {
                    if let Some(file_name) = src_path.file_name() {
                        let target_path = dest_dir.join(file_name);
                        if target_path != src_path {
                            // Intentar renombrar (mover), o copiar si están en distintos sistemas de archivos
                            if std::fs::rename(&src_path, &target_path).is_ok() {
                                moved_any = true;
                            } else if std::fs::copy(&src_path, &target_path).is_ok() {
                                moved_any = true;
                            }
                        }
                    }
                }
            }

            if moved_any {
                on_reload();
                return true;
            }
        }
        false
    });

    widget.add_controller(drop_target);
}
