use crate::fs_backend::{load_directory, FileItem};
use crate::state::AppState;
use crate::views::column_view::ColumnView;
use crate::views::gallery_view::GalleryView;
use crate::views::icon_view::IconView;
use crate::views::list_view::ListView;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, GestureClick, Image, Label, Orientation, Stack};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

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
    search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    search_gen: Rc<Cell<u64>>,
    on_activated: Rc<dyn Fn(PaneId)>,
    on_path_changed: Rc<dyn Fn(PaneId, PathBuf)>,
    on_selection_changed: Rc<dyn Fn(PaneId, usize, usize, PathBuf)>,
    #[allow(dead_code)]
    on_close_split: Option<Rc<dyn Fn()>>,
}

impl FolderPane {
    pub fn new(
        id: PaneId,
        initial_path: PathBuf,
        on_activated: Rc<dyn Fn(PaneId)>,
        on_path_changed: Rc<dyn Fn(PaneId, PathBuf)>,
        on_selection_changed: Rc<dyn Fn(PaneId, usize, usize, PathBuf)>,
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
        let on_selection = Rc::new(move |selected_count: usize| {
            if let Some(p) = self_for_sel.borrow().as_ref() {
                let curr = p.state.borrow().current_path.clone();
                let total = p.items_cache.borrow().len();
                (p.on_selection_changed)(p.id, total, selected_count, curr);
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
            search_cancel: RefCell::new(None),
            search_gen: Rc::new(Cell::new(0)),
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
        self.refresh_cut_states();
        (self.on_selection_changed)(
            self.id,
            self.items_cache.borrow().len(),
            self.get_selected_items().len(),
            self.current_path(),
        );
    }

    pub fn view_mode(&self) -> String {
        self.view_stack
            .visible_child_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "icons".to_string())
    }

    pub fn search_query(&self) -> String {
        self.state.borrow().search_query.clone()
    }

    pub fn is_recents(&self) -> bool {
        *self.virtual_folder.borrow() == Some(VirtualFolder::Recents)
    }

    pub fn is_applications(&self) -> bool {
        *self.virtual_folder.borrow() == Some(VirtualFolder::Applications)
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
        self.icon_view
            .set_layout_preferences(icon_size, grid_spacing);
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

    pub fn go_back_steps(&self, steps: usize) -> bool {
        let target = self.state.borrow_mut().go_back_steps(steps);
        self.step_history(target)
    }

    pub fn go_forward_steps(&self, steps: usize) -> bool {
        let target = self.state.borrow_mut().go_forward_steps(steps);
        self.step_history(target)
    }

    pub fn history_back_list(&self) -> Vec<PathBuf> {
        self.state
            .borrow()
            .history_back
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    pub fn history_forward_list(&self) -> Vec<PathBuf> {
        self.state
            .borrow()
            .history_forward
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    pub fn sort_settings(&self) -> (crate::state::SortBy, crate::state::SortOrder) {
        let s = self.state.borrow();
        (s.sort_by, s.sort_order)
    }

    pub fn set_sort(&self, sort_by: crate::state::SortBy, sort_order: crate::state::SortOrder) {
        {
            let mut s = self.state.borrow_mut();
            s.sort_by = sort_by;
            s.sort_order = sort_order;
        }
        self.reload();
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

    pub fn search_recursive(&self) -> bool {
        self.state.borrow().search_recursive
    }

    pub fn set_search_recursive(&self, recursive: bool) {
        self.state.borrow_mut().search_recursive = recursive;
        if !self.state.borrow().search_query.trim().is_empty() {
            self.reload();
        }
    }

    pub fn items_snapshot(&self) -> Vec<FileItem> {
        self.items_cache.borrow().clone()
    }

    fn cancel_pending_search(&self) {
        if let Some(flag) = self.search_cancel.borrow_mut().take() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn reload(&self) {
        self.cancel_pending_search();
        let next_gen = self.search_gen.get().wrapping_add(1);
        self.search_gen.set(next_gen);

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

        let (current_path, show_hidden, sort_by, sort_order, search_query, search_recursive) = {
            let s = self.state.borrow();
            (
                s.current_path.clone(),
                s.show_hidden,
                s.sort_by,
                s.sort_order,
                s.search_query.clone(),
                s.search_recursive,
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

        if search_recursive && !search_query.trim().is_empty() {
            let cancel_flag = Arc::new(AtomicBool::new(false));
            *self.search_cancel.borrow_mut() = Some(cancel_flag.clone());
            let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
            let root = current_path.clone();
            std::thread::spawn(move || {
                let paths = crate::fs_backend::search_paths_recursive(
                    &root,
                    &search_query,
                    show_hidden,
                    250,
                    &cancel_flag,
                );
                let _ = tx.send(paths);
            });
            let weak_self = self.self_weak_ref.borrow().clone();
            let gen_ref = self.search_gen.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
                if gen_ref.get() != next_gen {
                    return glib::ControlFlow::Break;
                }
                match rx.try_recv() {
                    Ok(paths) => {
                        if let Some(pane) = weak_self.upgrade() {
                            if pane.search_gen.get() == next_gen {
                                let mut items: Vec<FileItem> = paths
                                    .into_iter()
                                    .filter_map(|p| FileItem::from_path(&p))
                                    .collect();
                                crate::fs_backend::sort_file_items(&mut items, sort_by, sort_order);
                                pane.apply_items_snapshot(items);
                            }
                        }
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                }
            });
            return;
        }

        match load_directory(
            &current_path,
            show_hidden,
            sort_by,
            sort_order,
            &search_query,
        ) {
            Ok(items) => {
                let count = items.len();
                self.icon_view.set_current_folder(Some(&current_path));
                self.list_view.set_current_folder(Some(&current_path));
                self.column_view.set_current_folder(Some(&current_path));
                self.gallery_view.set_current_folder(Some(&current_path));
                self.icon_view.set_items(&items);
                self.list_view.set_items(&items);
                self.column_view
                    .set_root(&current_path, &items, show_hidden, sort_by, sort_order);
                self.gallery_view.set_items(&items);
                *self.items_cache.borrow_mut() = items;
                (self.on_selection_changed)(
                    self.id,
                    count,
                    self.get_selected_items().len(),
                    current_path.clone(),
                );
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
            let source_id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
                    pending_for_timer.set(None);
                    if let Some(p) = weak_for_timer.upgrade() {
                        p.sync_from_filesystem();
                    }
                });
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
        let Ok(items) = load_directory(
            &current_path,
            show_hidden,
            sort_by,
            sort_order,
            &search_query,
        ) else {
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
        (self.on_selection_changed)(
            self.id,
            count,
            self.get_selected_items().len(),
            current_path,
        );
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

    pub fn queue_select_name_active(&self, name: &str) {
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
                let created_str = created_path.to_string_lossy().to_string();
                let _ = self.container.activate_action(
                    "win.record_undo_created",
                    Some(&vec![created_str].to_variant()),
                );
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
        match mode.as_str() {
            "icons" => self
                .icon_view
                .get_selected_items(&self.items_cache.borrow()),
            "list" => self.list_view.get_selected_items(),
            "columns" => self.column_view.get_selected_items(),
            "gallery" => self.gallery_view.get_selected_items(),
            _ => self
                .get_selected_item_and_widget()
                .map(|(item, _)| vec![item])
                .unwrap_or_default(),
        }
    }

    pub fn select_all(&self) {
        let mode = self.view_stack.visible_child_name().unwrap_or_default();
        match mode.as_str() {
            "icons" => self.icon_view.select_all(),
            "list" => self.list_view.select_all(),
            "columns" => self.column_view.select_all(),
            "gallery" => self.gallery_view.select_all(),
            _ => {}
        }
    }

    #[allow(dead_code)]
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

    pub fn copy_selected(&self) -> bool {
        let paths: Vec<PathBuf> = self
            .get_selected_items()
            .into_iter()
            .map(|i| i.path)
            .collect();
        if paths.is_empty() {
            return false;
        }
        crate::operations::set_clipboard(crate::operations::ClipboardAction::Copy, paths);
        true
    }

    pub fn cut_selected(&self) -> bool {
        let paths: Vec<PathBuf> = self
            .get_selected_items()
            .into_iter()
            .map(|i| i.path)
            .collect();
        if paths.is_empty() {
            return false;
        }
        crate::operations::set_clipboard(crate::operations::ClipboardAction::Cut, paths);
        true
    }

    pub fn refresh_cut_states(&self) {
        self.icon_view.refresh_cut_states();
        self.list_view.refresh_cut_states();
        self.column_view.refresh_cut_states();
        self.gallery_view.refresh_cut_states();
    }

    #[allow(dead_code)]
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
            (
                s.current_path.clone(),
                s.show_hidden,
                s.sort_by,
                s.sort_order,
            )
        };
        let count = items.len();
        self.icon_view.set_current_folder(None);
        self.list_view.set_current_folder(None);
        self.column_view.set_current_folder(None);
        self.gallery_view.set_current_folder(None);
        self.icon_view.set_items(&items);
        self.list_view.set_items(&items);
        self.column_view
            .set_root(&current_path, &items, show_hidden, sort_by, sort_order);
        self.gallery_view.set_items(&items);
        *self.items_cache.borrow_mut() = items;
        (self.on_selection_changed)(
            self.id,
            count,
            self.get_selected_items().len(),
            current_path,
        );
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
            let es = malus_de_common::is_spanish_locale();
            let display = crate::db::tag_effective_label(&conn, tag_name, es);
            self.pane_title.set_label(&format!(
                "{}: {}",
                if es { "Etiqueta" } else { "Tag" },
                display
            ));
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

    pub fn toggle_tag_on_paths(&self, tag_id: &str, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        if let Ok(conn) = crate::db::open_db() {
            for path in paths {
                let _ = crate::db::toggle_file_tag(&conn, path, tag_id);
            }
        }
        self.reload();
    }

    pub fn clear_tags_on_paths(&self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        if let Ok(conn) = crate::db::open_db() {
            for path in paths {
                let _ = crate::db::clear_file_tags(&conn, path);
            }
        }
        self.reload();
    }
}

fn setup_drop_target<F: Fn() + 'static>(
    widget: &impl IsA<gtk4::Widget>,
    state: Rc<RefCell<AppState>>,
    on_reload: F,
) {
    let drop_target = gtk4::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::MOVE | gdk::DragAction::COPY,
    );
    drop_target.connect_enter(move |_, _, _| gdk::DragAction::MOVE);
    drop_target.connect_motion(move |_, _, _| gdk::DragAction::MOVE);
    let on_reload = Rc::new(on_reload);

    let widget_for_action = widget.as_ref().clone();
    drop_target.connect_drop(move |drop, value, _x, _y| {
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let dest_dir = state.borrow().current_path.clone();
            let paths: Vec<PathBuf> = file_list
                .files()
                .into_iter()
                .filter_map(|f| f.path())
                .filter(|src| src != &dest_dir && !(src.is_dir() && dest_dir.starts_with(src)))
                .collect();
            if paths.is_empty() {
                return false;
            }
            let is_copy = drop
                .current_drop()
                .is_some_and(|d| d.actions() == gdk::DragAction::COPY);
            let action_name = if is_copy {
                "win.copy_paths_to"
            } else {
                "win.move_paths_to"
            };
            let dest_str = dest_dir.to_string_lossy().to_string();
            let path_strs: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            if widget_for_action
                .activate_action(action_name, Some(&(dest_str, path_strs).to_variant()))
                .is_ok()
            {
                return true;
            }

            if crate::operations::move_paths_into_folder(&paths, &dest_dir) {
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
