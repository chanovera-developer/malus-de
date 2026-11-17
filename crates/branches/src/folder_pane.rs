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
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VirtualFolder {
    Recents,
    Applications,
}

pub struct FolderPane {
    pub id: PaneId,
    pub state: Rc<RefCell<AppState>>,
    virtual_folder: RefCell<Option<VirtualFolder>>,
    container: GtkBox,
    view_stack: Stack,
    icon_view: Rc<IconView>,
    list_view: Rc<ListView>,
    pub column_view: Rc<ColumnView>,
    pub gallery_view: Rc<GalleryView>,
    pane_header: GtkBox,
    pane_title: Label,
    items_cache: Rc<RefCell<Vec<FileItem>>>,
    self_weak_ref: RefCell<Weak<Self>>,
    dir_monitor: RefCell<Option<(PathBuf, gio::FileMonitor)>>,
    pending_sync: Rc<Cell<Option<glib::SourceId>>>,
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
                    p.sync_from_filesystem();
                }
            }
        });

        let folder_pane = Rc::new(Self {
            id,
            state,
            virtual_folder: RefCell::new(None),
            container,
            view_stack,
            icon_view,
            list_view,
            column_view,
            gallery_view,
            pane_header,
            pane_title,
            items_cache,
            self_weak_ref: RefCell::new(Weak::new()),
            dir_monitor: RefCell::new(None),
            pending_sync: Rc::new(Cell::new(None)),
            on_activated,
            on_path_changed,
            on_selection_changed,
            on_close_split,
        });

        *folder_pane.self_weak_ref.borrow_mut() = Rc::downgrade(&folder_pane);
        *self_weak.borrow_mut() = Some(folder_pane.clone());
        folder_pane.reload();

        folder_pane
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_split_mode_active(&self, active: bool) {
        self.pane_header.set_visible(active);
        if active {
            self.container.add_css_class("branches-split-mode");
        } else {
            self.container.remove_css_class("branches-split-mode");
        }
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

    /// Aplica cambios de configuración de Branches en vivo (tamaño de carpetas/iconos y espaciado).
    pub fn apply_branches_config(&self, cfg: &malus_de_common::config::BranchesConfig) {
        let icon_size = cfg.clamped_icon_size();
        let grid_spacing = cfg.clamped_grid_spacing();
        {
            let mut s = self.state.borrow_mut();
            s.icon_size = icon_size;
            s.grid_spacing = grid_spacing;
        }
        self.icon_view.set_layout_preferences(icon_size, grid_spacing);
        self.list_view.set_icon_size(cfg.clamped_list_icon_size());
    }

    pub fn navigate_to(&self, target: PathBuf) {
        if !target.exists() {
            let _ = std::fs::create_dir_all(&target);
        }
        let was_virtual = self.virtual_folder.borrow_mut().take().is_some();
        let changed = self.state.borrow_mut().navigate_to(target.clone());
        if changed || was_virtual {
            (self.on_activated)(self.id);
            self.reload();
            (self.on_path_changed)(self.id, target);
        } else {
            (self.on_activated)(self.id);
            self.reload();
        }
    }

    fn step_history(&self, target: Option<PathBuf>) -> bool {
        let Some(path) = target else {
            return false;
        };
        *self.virtual_folder.borrow_mut() = None;
        self.reload();
        (self.on_path_changed)(self.id, path);
        true
    }

    pub fn go_back(&self) -> bool {
        // Soltar el RefMut antes de reload() y callbacks para evitar RefCell already mutably borrowed.
        let prev = self.state.borrow_mut().go_back();
        self.step_history(prev)
    }

    pub fn go_forward(&self) -> bool {
        let next = self.state.borrow_mut().go_forward();
        self.step_history(next)
    }

    pub fn go_up(&self) -> bool {
        let parent = self.state.borrow_mut().go_up();
        self.step_history(parent)
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
        match *self.virtual_folder.borrow() {
            Some(VirtualFolder::Applications) => {
                self.clear_directory_monitor();
                self.reload_applications();
                return;
            }
            Some(VirtualFolder::Recents) => {
                self.clear_directory_monitor();
                self.reload_recents();
                return;
            }
            None => {}
        }

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
            "Malus SSD".to_string()
        } else if current_path.ends_with("Trash/files") {
            "Papelera".to_string()
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
                self.icon_view.set_current_folder(Some(&current_path));
                self.list_view.set_current_folder(Some(&current_path));
                self.column_view.set_current_folder(Some(&current_path));
                self.gallery_view.set_current_folder(Some(&current_path));
                self.icon_view.set_items(&items);
                self.list_view.set_items(&items);
                self.column_view.set_root(&current_path, &items, show_hidden, sort_by, sort_order);
                self.gallery_view.set_items(&items);
                *self.items_cache.borrow_mut() = items;
                (self.on_selection_changed)(self.id, count, current_path.clone());
                self.ensure_directory_monitor(&current_path);
            }
            Err(e) => {
                tracing::error!("Error al listar directorio en panel {:?}: {}", self.id, e);
                self.clear_directory_monitor();
            }
        }
    }

    fn clear_directory_monitor(&self) {
        if let Some(id) = self.pending_sync.take() {
            id.remove();
        }
        *self.dir_monitor.borrow_mut() = None;
    }

    fn ensure_directory_monitor(&self, dir: &Path) {
        if self
            .dir_monitor
            .borrow()
            .as_ref()
            .is_some_and(|(watched, _)| watched == dir)
        {
            return;
        }
        self.clear_directory_monitor();

        let weak_pane = self.self_weak_ref.borrow().clone();
        if weak_pane.strong_count() == 0 {
            return;
        }

        let gfile = gio::File::for_path(dir);
        let Ok(monitor) =
            gfile.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        else {
            return;
        };

        let watched_dir = dir.to_path_buf();
        let pending_sync_ref = self.pending_sync.clone();
        monitor.connect_changed(move |_, file, _other_file, event_type| {
            let Some(pane) = weak_pane.upgrade() else {
                return;
            };
            if pane.virtual_folder.borrow().is_some()
                || pane.state.borrow().current_path != watched_dir
            {
                return;
            }
            let event_path = file.path();
            if event_path.as_ref() == Some(&watched_dir)
                && matches!(
                    event_type,
                    gio::FileMonitorEvent::Changed
                        | gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::AttributeChanged
                )
            {
                return;
            }
            if !pane.state.borrow().show_hidden {
                if let Some(name) = event_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                {
                    if name.starts_with('.') {
                        return;
                    }
                }
            }

            if let Some(prev) = pending_sync_ref.take() {
                prev.remove();
            }
            let weak_for_timer = Rc::downgrade(&pane);
            let pending_for_timer = pending_sync_ref.clone();
            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(40),
                move || {
                    pending_for_timer.set(None);
                    if let Some(p) = weak_for_timer.upgrade() {
                        p.sync_from_filesystem();
                    }
                },
            );
            pending_sync_ref.set(Some(source_id));
        });

        *self.dir_monitor.borrow_mut() = Some((dir.to_path_buf(), monitor));
    }

    pub fn sync_from_filesystem(&self) {
        if self.virtual_folder.borrow().is_some() {
            return;
        }
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
        let Ok(items) = load_directory(&current_path, show_hidden, sort_by, sort_order, &search_query)
        else {
            return;
        };

        let same_snapshot = {
            let prev = self.items_cache.borrow();
            snapshots_equal(&prev, &items)
        };
        if same_snapshot {
            return;
        }

        let count = items.len();
        self.icon_view.sync_items_incremental(&items);
        self.list_view.set_items(&items);
        self.column_view
            .set_root(&current_path, &items, show_hidden, sort_by, sort_order);
        self.gallery_view.set_items(&items);
        *self.items_cache.borrow_mut() = items;
        (self.on_selection_changed)(self.id, count, current_path);
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

    fn get_selected_item_and_widget(&self) -> Option<(FileItem, gtk4::Widget)> {
        let mode = self.view_stack.visible_child_name().unwrap_or_default();
        match mode.as_str() {
            "icons" => self
                .icon_view
                .get_selected_item_and_widget(&self.items_cache.borrow()),
            "list" => self.list_view.get_selected_item_and_widget(),
            "columns" => self.column_view.get_selected_item_and_widget(),
            "gallery" => self.gallery_view.get_selected_item_and_widget(),
            _ => None,
        }
    }

    pub fn get_selected_item(&self) -> Option<FileItem> {
        self.get_selected_item_and_widget()
            .map(|(item, _)| item)
            .or_else(|| self.items_cache.borrow().first().cloned())
    }

    pub fn open_selected(&self) {
        if let Some((item, _)) = self.get_selected_item_and_widget() {
            if item.is_dir {
                self.navigate_to(item.path);
            } else {
                let _ = crate::fs_backend::open_file(&item.path);
            }
        }
    }

    pub fn is_icon_view_active(&self) -> bool {
        self.view_stack.visible_child_name().as_deref() == Some("icons")
    }

    pub fn navigate_icon_grid(&self, key: gdk::Key) -> bool {
        if self.is_icon_view_active() {
            self.icon_view.navigate_by_arrow(key)
        } else {
            false
        }
    }

    fn queue_select_name_active(&self, name: &str) {
        match self.view_stack.visible_child_name().as_deref() {
            Some("icons") => self.icon_view.queue_select_name(name),
            Some("list") => self.list_view.queue_select_name(name),
            Some("columns") => self.column_view.queue_select_name(name),
            Some("gallery") => self.gallery_view.queue_select_name(name),
            _ => self.icon_view.queue_select_name(name),
        }
    }

    pub fn create_new_folder_and_rename(self: &Rc<Self>) {
        let current_path = self.current_path();
        if !crate::views::can_create_in_folder(&current_path) {
            return;
        }
        let default_name = if malus_de_common::is_spanish_locale() {
            "Nueva carpeta"
        } else {
            "New Folder"
        };
        match crate::operations::create_new_folder(&current_path, default_name) {
            Ok(created_path) => {
                if let Some(new_name) = created_path.file_name() {
                    self.queue_select_name_active(&new_name.to_string_lossy());
                }
                self.reload();

                let pane_lookup = Rc::clone(self);
                let pane_rename = Rc::clone(self);
                let target_path = created_path;
                let on_renamed: crate::views::RenameCallback = Rc::new(move |new_path| {
                    if let Some(new_name) = new_path.file_name() {
                        pane_rename.queue_select_name_active(&new_name.to_string_lossy());
                    }
                    pane_rename.reload();
                });
                crate::views::schedule_inline_rename(
                    move || {
                        let (item, widget) = pane_lookup.get_selected_item_and_widget()?;
                        if item.path != target_path {
                            return None;
                        }
                        Some((item.path, item.is_dir, widget))
                    },
                    Some(on_renamed),
                );
            }
            Err(e) => {
                tracing::error!("Error al crear carpeta: {}", e);
            }
        }
    }

    pub fn rename_selected(self: &Rc<Self>) {
        if self.current_path().ends_with("Trash/files") {
            return;
        }
        let Some((item, widget)) = self.get_selected_item_and_widget() else {
            return;
        };
        let pane = Rc::clone(self);
        crate::views::show_rename_popover(
            &widget,
            item.path,
            item.is_dir,
            None,
            Some(Rc::new(move |new_path| {
                if let Some(new_name) = new_path.file_name() {
                    pane.queue_select_name_active(&new_name.to_string_lossy());
                }
                pane.reload();
            })),
        );
    }

    pub fn get_selected_items(&self) -> Vec<FileItem> {
        let mode = self.view_stack.visible_child_name().unwrap_or_default();
        if mode.as_str() == "icons" {
            let items = self.icon_view.get_selected_items(&self.items_cache.borrow());
            if !items.is_empty() {
                return items;
            }
        }
        self.get_selected_item_and_widget()
            .map(|(item, _)| vec![item])
            .unwrap_or_default()
    }

    pub fn select_all(&self) {
        if self.is_icon_view_active() {
            self.icon_view.select_all();
        }
    }

    pub fn duplicate_selected(&self) {
        let items = self.get_selected_items();
        let mut changed = false;
        for item in items {
            if crate::operations::duplicate_item(&item.path).is_ok() {
                changed = true;
            }
        }
        if changed {
            self.reload();
        }
    }

    pub fn trash_selected(&self) {
        let items = self.get_selected_items();
        let mut changed = false;
        for item in items {
            if crate::operations::trash_item(&item.path).is_ok() {
                changed = true;
            }
        }
        if changed {
            self.reload();
        }
    }

    fn apply_items_snapshot(&self, items: Vec<FileItem>) {
        let (current_path, show_hidden, sort_by, sort_order) = {
            let s = self.state.borrow();
            (s.current_path.clone(), s.show_hidden, s.sort_by, s.sort_order)
        };
        let count = items.len();
        self.icon_view.set_current_folder(None);
        self.list_view.set_current_folder(None);
        self.column_view.set_current_folder(None);
        self.gallery_view.set_current_folder(None);
        self.icon_view.set_items(&items);
        self.list_view.set_items(&items);
        self.column_view.set_root(&current_path, &items, show_hidden, sort_by, sort_order);
        self.gallery_view.set_items(&items);
        *self.items_cache.borrow_mut() = items;
        (self.on_selection_changed)(self.id, count, current_path);
    }

    pub fn show_tag_filter(&self, tag_name: &str) {
        *self.virtual_folder.borrow_mut() = None;
        self.clear_directory_monitor();
        if let Ok(conn) = crate::db::open_db() {
            let paths = crate::db::get_files_by_tag(&conn, tag_name);
            let mut items = Vec::new();
            for p in paths {
                if let Some(item) = FileItem::from_path(&p) {
                    items.push(item);
                }
            }
            self.pane_title.set_label(&format!("Etiqueta: {}", tag_name));
            self.apply_items_snapshot(items);
        }
    }

    pub fn show_recents(&self) {
        *self.virtual_folder.borrow_mut() = Some(VirtualFolder::Recents);
        self.reload_recents();
    }

    fn reload_recents(&self) {
        let query = self.state.borrow().search_query.trim().to_lowercase();
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        let search_dirs = [
            home.join("Escritorio"),
            home.join("Desktop"),
            home.join("Documentos"),
            home.join("Documents"),
            home.join("Descargas"),
            home.join("Downloads"),
            home.join("Imágenes"),
            home.join("Pictures"),
            home.join("Música"),
            home.join("Music"),
            home.join("Vídeos"),
            home.join("Videos"),
        ];
        let mut items = crate::fs_backend::load_recent_files(&home, &search_dirs, 60);
        if !query.is_empty() {
            items.retain(|item| item.name.to_lowercase().contains(&query));
        }
        let title = if malus_de_common::is_spanish_locale() {
            "Recientes"
        } else {
            "Recents"
        };
        self.pane_title.set_label(title);
        self.apply_items_snapshot(items);
    }

    pub fn show_applications(&self) {
        *self.virtual_folder.borrow_mut() = Some(VirtualFolder::Applications);
        self.reload_applications();
    }

    fn reload_applications(&self) {
        let query = self.state.borrow().search_query.clone();
        let items = crate::fs_backend::load_applications_items(&query);
        let title = if malus_de_common::is_spanish_locale() {
            "Aplicaciones"
        } else {
            "Applications"
        };
        self.pane_title.set_label(title);
        self.apply_items_snapshot(items);
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
    let drop_target = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::MOVE | gdk::DragAction::COPY);
    drop_target.connect_enter(move |_, _, _| gdk::DragAction::MOVE);
    drop_target.connect_motion(move |_, _, _| gdk::DragAction::MOVE);
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
                            let dest_file = gio::File::for_path(&target_path);
                            if file
                                .move_(
                                    &dest_file,
                                    gio::FileCopyFlags::NONE,
                                    gio::Cancellable::NONE,
                                    None,
                                )
                                .is_ok()
                                || std::fs::rename(&src_path, &target_path).is_ok()
                            {
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

impl Drop for FolderPane {
    fn drop(&mut self) {
        self.clear_directory_monitor();
    }
}

fn snapshots_equal(prev: &[FileItem], next: &[FileItem]) -> bool {
    if prev.len() != next.len() {
        return false;
    }
    prev.iter().zip(next.iter()).all(|(a, b)| {
        a.name == b.name
            && a.path == b.path
            && a.is_dir == b.is_dir
            && a.is_symlink == b.is_symlink
            && a.size == b.size
            && a.modified == b.modified
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_equal_detects_content_and_metadata_changes() {
        let mk = |name: &str, size: u64| FileItem {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir: false,
            is_hidden: false,
            is_symlink: false,
            size,
            size_str: format!("{size} B"),
            modified: None,
            modified_str: "--".to_string(),
            kind_str: "Archivo".to_string(),
            gicon: gio::ThemedIcon::new("text-x-generic").upcast(),
        };
        let a = vec![mk("uno.txt", 10), mk("dos.txt", 20)];
        let b = vec![mk("uno.txt", 10), mk("dos.txt", 20)];
        let c = vec![mk("uno.txt", 10), mk("dos.txt", 25)];
        let d = vec![mk("uno.txt", 10)];

        assert!(super::snapshots_equal(&a, &b));
        assert!(!super::snapshots_equal(&a, &c));
        assert!(!super::snapshots_equal(&a, &d));
    }

    #[test]
    fn history_navigation_releases_borrow_before_reload() {
        let src = include_str!("folder_pane.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        assert!(!src.contains("if let Some(prev) = self.state.borrow_mut().go_back()"));
        assert!(!src.contains("if let Some(next) = self.state.borrow_mut().go_forward()"));
        assert!(!src.contains("if let Some(parent) = self.state.borrow_mut().go_up()"));

        let state = RefCell::new(AppState::new(PathBuf::from("/tmp/a")));
        assert!(state.borrow_mut().navigate_to(PathBuf::from("/tmp/a/b")));

        let prev = state.borrow_mut().go_back();
        assert_eq!(prev, Some(PathBuf::from("/tmp/a")));
        assert!(!state.borrow().can_go_back());
        assert!(state.borrow().can_go_forward());

        let next = state.borrow_mut().go_forward();
        assert_eq!(next, Some(PathBuf::from("/tmp/a/b")));
        assert!(state.borrow().can_go_back());

        let parent = state.borrow_mut().go_up();
        assert_eq!(parent, Some(PathBuf::from("/tmp/a")));
        assert_eq!(state.borrow().current_path, PathBuf::from("/tmp/a"));
    }
}
