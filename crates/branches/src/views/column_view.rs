use crate::fs_backend::{
    format_modified, format_size, load_directory, open_file, open_in_terminal, FileItem,
};
use crate::state::{SortBy, SortOrder};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gdk_pixbuf::{Pixbuf, PixbufRotation};
use gtk4::gdk::{self, Key};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, EventControllerKey, GestureClick, Image, Label,
    ListBox, ListBoxRow, Orientation, Overlay, PolicyType, Popover, ScrolledWindow,
};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct ColumnView {
    inner: Rc<ColumnViewInner>,
}

struct ColumnData {
    #[allow(dead_code)]
    path: PathBuf,
    items: Vec<FileItem>,
    container: GtkBox,
    list_box: ListBox,
}

struct ColumnViewInner {
    root_box: GtkBox,
    scrolled: ScrolledWindow,
    columns_box: GtkBox,
    columns: RefCell<Vec<ColumnData>>,
    active_col_idx: Cell<Option<usize>>,
    syncing_selection: Cell<bool>,
    preview_box: RefCell<Option<GtkBox>>,
    current_root: RefCell<PathBuf>,
    pending_select_name: RefCell<Option<String>>,
    can_create: std::cell::Cell<bool>,
    show_hidden: RefCell<bool>,
    sort_by: RefCell<SortBy>,
    sort_order: RefCell<SortOrder>,
    on_navigate: NavigateCallback,
    on_selection_changed: SelectionCallback,
}

impl ColumnView {
    pub fn refresh_cut_states(&self) {
        for col in self.inner.columns.borrow().iter() {
            for (idx, item) in col.items.iter().enumerate() {
                if let Some(row) = col.list_box.row_at_index(idx as i32) {
                    if let Some(child) = row.child() {
                        let target = child
                            .downcast_ref::<Overlay>()
                            .and_then(|ov| ov.last_child())
                            .unwrap_or(child);
                        crate::views::sync_cut_css_class(&target, &item.path);
                    }
                }
            }
        }
    }

    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
        let root_box = GtkBox::new(Orientation::Horizontal, 0);
        root_box.add_css_class("branches-column-view");
        root_box.set_hexpand(true);
        root_box.set_vexpand(true);

        let columns_box = GtkBox::new(Orientation::Horizontal, 0);
        columns_box.add_css_class("branches-columns-box");
        columns_box.set_hexpand(true);
        columns_box.set_vexpand(true);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .css_classes(["branches-column-view"])
            .child(&columns_box)
            .build();
        root_box.append(&scrolled);

        let inner = Rc::new(ColumnViewInner {
            root_box,
            scrolled,
            columns_box,
            columns: RefCell::new(Vec::new()),
            active_col_idx: Cell::new(None),
            syncing_selection: Cell::new(false),
            preview_box: RefCell::new(None),
            current_root: RefCell::new(PathBuf::from("/")),
            pending_select_name: RefCell::new(None),
            can_create: std::cell::Cell::new(false),
            show_hidden: RefCell::new(false),
            sort_by: RefCell::new(SortBy::Name),
            sort_order: RefCell::new(SortOrder::Ascending),
            on_navigate,
            on_selection_changed,
        });

        Self { inner }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.inner.root_box
    }

    pub fn set_current_folder(&self, folder: Option<&Path>) {
        self.inner
            .can_create
            .set(folder.is_some_and(crate::views::can_create_in_folder));
    }

    pub fn set_root(
        &self,
        path: &Path,
        items: &[FileItem],
        show_hidden: bool,
        sort_by: SortBy,
        sort_order: SortOrder,
    ) {
        *self.inner.show_hidden.borrow_mut() = show_hidden;
        *self.inner.sort_by.borrow_mut() = sort_by;
        *self.inner.sort_order.borrow_mut() = sort_order;
        *self.inner.current_root.borrow_mut() = path.to_path_buf();

        // Entrar a una carpeta que ya se muestra como columna: se conservan las columnas
        // anteriores (el padre no se oculta si hay espacio). Si no caben, `scroll_to_end`
        // desplaza el contenedor a la izquierda con animación y el padre sale de vista.
        let existing_idx = self
            .inner
            .columns
            .borrow()
            .iter()
            .position(|c| c.path == path)
            .filter(|&k| k > 0);
        let has_pending_select = self.inner.pending_select_name.borrow().is_some();
        if let (Some(k), false) = (existing_idx, has_pending_select) {
            self.inner.syncing_selection.set(true);
            self.inner.prune_after(k);
            let list_box = self
                .inner
                .columns
                .borrow()
                .get(k)
                .map(|col| col.list_box.clone());
            if let Some(list_box) = list_box {
                list_box.unselect_all();
            }
            self.inner.active_col_idx.set(Some(k));
            self.inner.syncing_selection.set(false);
            self.inner.scroll_to_end();
            return;
        }

        self.inner.clear_all();
        self.inner
            .push_column(0, path.to_path_buf(), items.to_vec());
    }

    pub fn queue_select_name(&self, name: &str) {
        *self.inner.pending_select_name.borrow_mut() = Some(name.to_string());
    }

    pub fn get_selected_item_and_widget(&self) -> Option<(FileItem, gtk4::Widget)> {
        let columns = self.inner.columns.borrow();
        let col_idx = self.inner.target_column_index(&columns)?;
        let col = columns.get(col_idx)?;
        let selected_row = col.list_box.selected_rows().into_iter().next()?;
        let idx = selected_row.index() as usize;
        let item = col.items.get(idx).cloned()?;
        Some((item, selected_row.upcast()))
    }

    pub fn get_selected_items(&self) -> Vec<FileItem> {
        let columns = self.inner.columns.borrow();
        let Some(col_idx) = self.inner.target_column_index(&columns) else {
            return Vec::new();
        };
        let Some(col) = columns.get(col_idx) else {
            return Vec::new();
        };
        crate::views::list_box_selected_items(&col.list_box, &col.items)
    }

    pub fn select_all(&self) {
        let target = {
            let columns = self.inner.columns.borrow();
            if columns.is_empty() {
                return;
            }
            let target_idx = columns
                .iter()
                .position(|c| c.list_box.has_focus() || c.list_box.focus_child().is_some())
                .or_else(|| {
                    self.inner
                        .active_col_idx
                        .get()
                        .filter(|&idx| idx < columns.len())
                })
                .unwrap_or(columns.len() - 1);
            columns
                .get(target_idx)
                .map(|col| (target_idx, col.list_box.clone(), col.items.clone()))
        };
        if let Some((target_idx, list_box, col_items)) = target {
            self.inner.active_col_idx.set(Some(target_idx));
            self.inner.syncing_selection.set(true);
            list_box.select_all();
            self.inner.syncing_selection.set(false);
            let selected = crate::views::list_box_selected_items(&list_box, &col_items);
            match selected.len() {
                1 => {
                    self.inner.on_item_selected(target_idx, &selected[0]);
                    return;
                }
                n if n > 1 => {
                    self.inner.on_multiple_items_selected(target_idx, &selected);
                    return;
                }
                _ => {
                    self.inner.prune_after(target_idx);
                    (self.inner.on_selection_changed)(0);
                }
            }
        }
    }

    pub fn active_column_names(&self) -> Vec<String> {
        let columns = self.inner.columns.borrow();
        let Some(col_idx) = self.inner.target_column_index(&columns) else {
            return Vec::new();
        };
        columns
            .get(col_idx)
            .map(|c| c.items.iter().map(|it| it.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn selected_item_name(&self) -> Option<String> {
        self.get_selected_items().into_iter().next().map(|it| it.name)
    }

    pub fn select_item_by_name(&self, name: &str) -> bool {
        let (target_idx, item_opt, row_opt) = {
            let columns = self.inner.columns.borrow();
            let Some(col_idx) = self.inner.target_column_index(&columns) else {
                return false;
            };
            let Some(col) = columns.get(col_idx) else {
                return false;
            };
            let Some(pos) = col.items.iter().position(|it| it.name == name) else {
                return false;
            };
            let row = col.list_box.row_at_index(pos as i32);
            if let Some(ref r) = row {
                self.inner.active_col_idx.set(Some(col_idx));
                self.inner.syncing_selection.set(true);
                col.list_box.unselect_all();
                col.list_box.select_row(Some(r));
                self.inner.syncing_selection.set(false);
            }
            (col_idx, col.items.get(pos).cloned(), row)
        };
        if let Some(row) = row_opt {
            row.grab_focus();
            if let Some(item) = item_opt {
                self.inner.on_item_selected(target_idx, &item);
            }
            return true;
        }
        false
    }
}

impl ColumnViewInner {
    fn target_column_index(&self, cols: &[ColumnData]) -> Option<usize> {
        if let Some(idx) = self.active_col_idx.get() {
            if let Some(col) = cols.get(idx) {
                if !col.list_box.selected_rows().is_empty() {
                    return Some(idx);
                }
            }
        }
        cols.iter()
            .rposition(|col| !col.list_box.selected_rows().is_empty())
    }

    fn clear_all(&self) {
        self.active_col_idx.set(None);
        let prev = self.preview_box.borrow_mut().take();
        if let Some(prev) = prev {
            self.root_box.remove(&prev);
        }
        let removed: Vec<_> = self.columns.borrow_mut().drain(..).collect();
        for col in removed {
            self.columns_box.remove(&col.container);
        }
    }

    fn push_column(self: &Rc<Self>, col_idx: usize, path: PathBuf, items: Vec<FileItem>) {
        let (col_box, list_box) = self.create_column_widget(col_idx, path.clone(), items.clone());
        self.columns_box.append(&col_box);
        if col_idx > 0 {
            slide_in_from_right(&col_box, 240);
        }
        let select_idx = self
            .pending_select_name
            .borrow_mut()
            .take()
            .and_then(|target| items.iter().position(|it| it.name == target));
        self.columns.borrow_mut().push(ColumnData {
            path,
            items,
            container: col_box,
            list_box: list_box.clone(),
        });
        if let Some(idx) = select_idx {
            if let Some(row) = list_box.row_at_index(idx as i32) {
                list_box.select_row(Some(&row));
                row.grab_focus();
            }
        }
        self.scroll_to_end();
    }

    fn prune_after(self: &Rc<Self>, col_idx: usize) {
        let prev = self.preview_box.borrow_mut().take();
        if let Some(prev) = prev {
            self.root_box.remove(&prev);
        }
        let mut removed = Vec::new();
        {
            let mut cols = self.columns.borrow_mut();
            while cols.len() > col_idx + 1 {
                if let Some(col) = cols.pop() {
                    removed.push(col.container);
                }
            }
        }
        for container in removed {
            self.columns_box.remove(&container);
        }
    }

    fn create_column_widget(
        self: &Rc<Self>,
        col_idx: usize,
        _path: PathBuf,
        items: Vec<FileItem>,
    ) -> (GtkBox, ListBox) {
        let col_box = GtkBox::new(Orientation::Vertical, 0);
        col_box.add_css_class("branches-column");
        col_box.set_size_request(240, -1);
        col_box.set_vexpand(true);

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Multiple)
            .activate_on_single_click(false)
            .css_classes(["branches-column-list"])
            .build();

        let items_rc = Rc::new(items);
        let tags_map = crate::views::load_tags_map();
        for item in items_rc.iter() {
            let row = create_column_row(item, items_rc.clone(), &tags_map, &self.on_navigate);
            list_box.append(&row);
        }

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .build();

        let bg_click = GestureClick::new();
        bg_click.set_button(gdk::BUTTON_SECONDARY);
        let scrolled_bg = scrolled.clone();
        bg_click.connect_pressed(move |g, _n, x, y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            crate::views::show_background_context_menu(&scrolled_bg, x, y);
        });
        scrolled.add_controller(bg_click);

        col_box.append(&scrolled);

        // Controlador de selección de filas (soporta selección múltiple y rango)
        let self_for_sel = self.clone();
        let items_for_sel = items_rc.clone();
        list_box.connect_selected_rows_changed(move |lb| {
            // Redibujar filas vecinas para actualizar esquinas squircle en selecciones continuas
            redraw_all_column_row_bgs(lb);
            if self_for_sel.syncing_selection.get() {
                return;
            }
            self_for_sel.active_col_idx.set(Some(col_idx));
            let selected_rows = lb.selected_rows();
            let count = selected_rows.len();
            if count == 1 {
                if let Some(row) = selected_rows.first() {
                    let idx = row.index() as usize;
                    if let Some(item) = items_for_sel.get(idx) {
                        self_for_sel.on_item_selected(col_idx, item);
                        return;
                    }
                }
            } else if count > 1 {
                let selected = crate::views::list_box_selected_items(lb, &items_for_sel);
                if selected.len() > 1 {
                    self_for_sel.on_multiple_items_selected(col_idx, &selected);
                    return;
                }
            }
            self_for_sel.prune_after(col_idx);
            (self_for_sel.on_selection_changed)(count);
        });

        // Controlador de activación (doble clic o Enter)
        let items_for_act = items_rc;
        let on_nav = self.on_navigate.clone();
        list_box.connect_row_activated(move |_, row| {
            let idx = row.index() as usize;
            if let Some(item) = items_for_act.get(idx) {
                if item.is_dir {
                    on_nav(item.path.clone());
                } else {
                    let _ = open_file(&item.path);
                }
            }
        });

        // Controlador de teclado para navegación tipo macOS Finder (Flecha Izq / Der)
        let key_controller = EventControllerKey::new();
        let self_for_key = self.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            match key {
                Key::Right => {
                    let next_lb = {
                        let cols = self_for_key.columns.borrow();
                        cols.get(col_idx + 1).map(|c| c.list_box.clone())
                    };
                    if let Some(next_lb) = next_lb {
                        next_lb.grab_focus();
                        if next_lb.selected_rows().is_empty() {
                            if let Some(first_row) = next_lb.row_at_index(0) {
                                next_lb.select_row(Some(&first_row));
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                }
                Key::Left => {
                    if col_idx > 0 {
                        let prev_lb = {
                            let cols = self_for_key.columns.borrow();
                            cols.get(col_idx - 1).map(|c| c.list_box.clone())
                        };
                        if let Some(prev_lb) = prev_lb {
                            prev_lb.grab_focus();
                            return glib::Propagation::Stop;
                        }
                    }
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
        list_box.add_controller(key_controller);

        (col_box, list_box)
    }

    fn on_item_selected(self: &Rc<Self>, col_idx: usize, item: &FileItem) {
        self.prune_after(col_idx);
        (self.on_selection_changed)(1);

        if item.is_dir {
            let show_hidden = *self.show_hidden.borrow();
            let sort_by = *self.sort_by.borrow();
            let sort_order = *self.sort_order.borrow();

            if let Ok(child_items) =
                load_directory(&item.path, show_hidden, sort_by, sort_order, "")
            {
                self.push_column(col_idx + 1, item.path.clone(), child_items);
            }
        } else {
            let preview = self.create_preview_pane(item);
            self.root_box.append(&preview);
            fade_in_widget(&preview);
            *self.preview_box.borrow_mut() = Some(preview);
            self.scroll_to_end();
        }
    }

    fn on_multiple_items_selected(self: &Rc<Self>, col_idx: usize, items: &[FileItem]) {
        self.prune_after(col_idx);
        (self.on_selection_changed)(items.len());

        let preview = self.create_multi_preview_pane(items);
        self.root_box.append(&preview);
        fade_in_widget(&preview);
        *self.preview_box.borrow_mut() = Some(preview);
        self.scroll_to_end();
    }

    fn create_multi_preview_pane(self: &Rc<Self>, items: &[FileItem]) -> GtkBox {
        const MULTI_PREVIEW_MAX_LISTED: usize = 50;
        let es = malus_de_common::is_spanish_locale();
        let summary = summarize_multi_selection(items, es);

        let preview_box = GtkBox::new(Orientation::Vertical, 0);
        preview_box.add_css_class("branches-column-preview");
        preview_box.set_size_request(PREVIEW_PANE_WIDTH, -1);
        preview_box.set_halign(Align::End);
        preview_box.set_hexpand(false);
        preview_box.set_vexpand(true);

        let content_vbox = GtkBox::new(Orientation::Vertical, 0);
        content_vbox.set_margin_top(22);
        content_vbox.set_margin_bottom(12);
        content_vbox.set_margin_start(PREVIEW_CONTENT_MARGIN);
        content_vbox.set_margin_end(PREVIEW_CONTENT_MARGIN);

        // 1. Pila visual de elementos seleccionados (miniaturas en abanico si son imágenes, o pila de iconos)
        let thumb_box = GtkBox::new(Orientation::Vertical, 0);
        thumb_box.set_halign(Align::Fill);
        thumb_box.set_hexpand(true);
        thumb_box.add_css_class("branches-preview-thumb-box");

        let all_images = !items.is_empty() && items.iter().all(|it| !it.is_dir && is_preview_image_path(&it.path));
        let stack_pixbufs: Rc<RefCell<Vec<Pixbuf>>> = Rc::new(RefCell::new(Vec::new()));
        let stack_draw_area: Rc<RefCell<Option<DrawingArea>>> = Rc::new(RefCell::new(None));

        if all_images {
            let loaded: Vec<Pixbuf> = items
                .iter()
                .take(3)
                .filter_map(|it| Pixbuf::from_file_at_scale(&it.path, 480, 340, true).ok())
                .collect();
            if !loaded.is_empty() {
                *stack_pixbufs.borrow_mut() = loaded;
                let area = build_multi_image_stack_preview(stack_pixbufs.clone());
                *stack_draw_area.borrow_mut() = Some(area.clone());
                thumb_box.append(&area);
            } else {
                thumb_box.append(&build_multi_icon_stack(items));
            }
        } else {
            thumb_box.append(&build_multi_icon_stack(items));
        }
        content_vbox.append(&thumb_box);

        // 2. Título: «N elementos»
        let title_label = Label::builder()
            .label(&summary.title)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .max_width_chars(28)
            .xalign(0.0)
            .halign(Align::Fill)
            .margin_top(16)
            .css_classes(["branches-preview-title"])
            .build();
        content_vbox.append(&title_label);

        // 3. Subtítulo: «Tipo - Tamaño total»
        let sub_label = Label::builder()
            .label(&summary.subtitle)
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(36)
            .xalign(0.0)
            .halign(Align::Fill)
            .margin_top(2)
            .css_classes(["branches-preview-subtitle"])
            .build();
        content_vbox.append(&sub_label);

        // 4. Encabezado «Información» + enlace «Mostrar más»
        let info_header = GtkBox::new(Orientation::Horizontal, 8);
        info_header.set_margin_top(16);
        info_header.set_margin_bottom(6);
        info_header.append(
            &Label::builder()
                .label(if es { "Información" } else { "Information" })
                .xalign(0.0)
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-preview-section-title"])
                .build(),
        );
        let show_more_btn = Button::builder()
            .label(if es { "Mostrar más" } else { "Show More" })
            .has_frame(false)
            .css_classes(["branches-preview-more-link"])
            .build();
        info_header.append(&show_more_btn);
        content_vbox.append(&info_header);

        // 5. Tabla de metadatos agregados
        let meta_box = GtkBox::new(Orientation::Vertical, 0);
        let mut first_row = true;
        let append_meta_row = |parent: &GtkBox, first: &mut bool, key: &str, val: &str| {
            if !*first {
                let div = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(["branches-preview-meta-divider"])
                    .hexpand(true)
                    .build();
                parent.append(&div);
            }
            *first = false;

            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.add_css_class("branches-preview-meta-row");
            row.append(
                &Label::builder()
                    .label(key)
                    .halign(Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["branches-preview-meta-key"])
                    .build(),
            );
            row.append(
                &Label::builder()
                    .label(val)
                    .halign(Align::End)
                    .xalign(1.0)
                    .ellipsize(pango::EllipsizeMode::Middle)
                    .max_width_chars(22)
                    .selectable(true)
                    .css_classes(["branches-preview-meta-val"])
                    .build(),
            );
            parent.append(&row);
        };

        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Elementos" } else { "Items" },
            &summary.breakdown,
        );
        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Tamaño total" } else { "Total size" },
            &summary.total_size_str,
        );
        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Tipo" } else { "Kind" },
            &summary.kind_label,
        );
        if let Some(created) = summary.earliest_created {
            append_meta_row(
                &meta_box,
                &mut first_row,
                if es { "Creación" } else { "Created" },
                &format_modified(created),
            );
        }
        if let Some(modified) = summary.latest_modified {
            append_meta_row(
                &meta_box,
                &mut first_row,
                if es { "Modificación" } else { "Modified" },
                &format_modified(modified),
            );
        }
        if let Some(accessed) = summary.latest_accessed {
            append_meta_row(
                &meta_box,
                &mut first_row,
                if es {
                    "Última apertura"
                } else {
                    "Last opened"
                },
                &format_modified(accessed),
            );
        }
        content_vbox.append(&meta_box);

        // Metadatos adicionales ocultos hasta pulsar «Mostrar más»
        let extra_meta_box = GtkBox::new(Orientation::Vertical, 0);
        extra_meta_box.set_visible(false);
        let mut extra_first = false;
        let parent_dir_str = items
            .first()
            .and_then(|it| it.path.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Ubicación" } else { "Where" },
            &parent_dir_str,
        );
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Tamaño exacto" } else { "Exact size" },
            &format!("{} bytes", summary.total_bytes),
        );
        content_vbox.append(&extra_meta_box);

        {
            let extra_c = extra_meta_box.clone();
            let expanded = Cell::new(false);
            show_more_btn.connect_clicked(move |btn| {
                let next = !expanded.get();
                expanded.set(next);
                extra_c.set_visible(next);
                btn.set_label(if next {
                    if es {
                        "Mostrar menos"
                    } else {
                        "Show Less"
                    }
                } else if es {
                    "Mostrar más"
                } else {
                    "Show More"
                });
            });
        }

        // 6. Lista de archivos seleccionados con su tamaño / tipo
        let list_header = GtkBox::new(Orientation::Horizontal, 8);
        list_header.set_margin_top(16);
        list_header.set_margin_bottom(6);
        list_header.append(
            &Label::builder()
                .label(if es { "Seleccionados" } else { "Selected" })
                .xalign(0.0)
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-preview-section-title"])
                .build(),
        );
        list_header.append(
            &Label::builder()
                .label(items.len().to_string())
                .xalign(1.0)
                .halign(Align::End)
                .css_classes(["branches-preview-meta-key"])
                .build(),
        );
        content_vbox.append(&list_header);

        let selected_list_box = GtkBox::new(Orientation::Vertical, 0);
        for (idx, it) in items.iter().take(MULTI_PREVIEW_MAX_LISTED).enumerate() {
            if idx > 0 {
                let div = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(["branches-preview-meta-divider"])
                    .hexpand(true)
                    .build();
                selected_list_box.append(&div);
            }
            let row = GtkBox::new(Orientation::Horizontal, 8);
            row.add_css_class("branches-preview-meta-row");

            let icon = crate::views::create_item_image(it, 14, None);
            icon.set_size_request(14, 14);
            icon.set_valign(Align::Center);
            row.append(&icon);

            row.append(
                &Label::builder()
                    .label(&it.name)
                    .halign(Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(pango::EllipsizeMode::Middle)
                    .max_width_chars(24)
                    .css_classes(["branches-preview-meta-val"])
                    .build(),
            );
            let detail = if it.is_dir {
                it.kind_str.as_str()
            } else {
                it.size_str.as_str()
            };
            row.append(
                &Label::builder()
                    .label(detail)
                    .halign(Align::End)
                    .xalign(1.0)
                    .ellipsize(pango::EllipsizeMode::End)
                    .max_width_chars(12)
                    .css_classes(["branches-preview-meta-key"])
                    .build(),
            );
            selected_list_box.append(&row);
        }
        if items.len() > MULTI_PREVIEW_MAX_LISTED {
            let rem = items.len() - MULTI_PREVIEW_MAX_LISTED;
            let div = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .css_classes(["branches-preview-meta-divider"])
                .hexpand(true)
                .build();
            selected_list_box.append(&div);
            let more_lbl = Label::builder()
                .label(if es {
                    format!("+ {rem} elementos más")
                } else {
                    format!("+ {rem} more items")
                })
                .halign(Align::Start)
                .xalign(0.0)
                .margin_top(6)
                .css_classes(["branches-preview-meta-key"])
                .build();
            selected_list_box.append(&more_lbl);
        }
        content_vbox.append(&selected_list_box);

        let scrolled_preview = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .propagate_natural_width(false)
            .max_content_width(PREVIEW_PANE_WIDTH)
            .vexpand(true)
            .child(&content_vbox)
            .build();
        preview_box.append(&scrolled_preview);

        // 7. Barra inferior fija de Acciones Rápidas para selección múltiple
        let actions_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .halign(Align::Fill)
            .hexpand(true)
            .css_classes(["branches-preview-actions-bar"])
            .build();
        let actions_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(22)
            .halign(Align::Center)
            .hexpand(true)
            .build();

        let all_paths: Vec<PathBuf> = items.iter().map(|it| it.path.clone()).collect();
        let file_paths: Vec<PathBuf> = items
            .iter()
            .filter(|it| !it.is_dir)
            .map(|it| it.path.clone())
            .collect();
        let all_paths_joined = all_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");

        if all_images {
            let rotate_btn = make_quick_action_button(
                "object-rotate-left-symbolic",
                if es {
                    "Girar a la\nizquierda"
                } else {
                    "Rotate\nLeft"
                },
            );
            let paths_rot = all_paths.clone();
            let pb_state = stack_pixbufs.clone();
            let area_state = stack_draw_area.clone();
            rotate_btn.connect_clicked(move |_| {
                {
                    let mut guard = pb_state.borrow_mut();
                    for pb in guard.iter_mut() {
                        if let Some(rot) = pb.rotate_simple(PixbufRotation::Counterclockwise) {
                            *pb = rot;
                        }
                    }
                }
                if let Some(area) = area_state.borrow().as_ref() {
                    area.queue_draw();
                }
                for p in &paths_rot {
                    let ext = p
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    let fmt = match ext.as_str() {
                        "jpg" | "jpeg" => Some("jpeg"),
                        "bmp" => Some("bmp"),
                        "png" => Some("png"),
                        _ => None,
                    };
                    if let Some(fmt) = fmt {
                        if let Ok(full_pb) = Pixbuf::from_file(p) {
                            if let Some(rot_full) =
                                full_pb.rotate_simple(PixbufRotation::Counterclockwise)
                            {
                                let _ = rot_full.savev(p, fmt, &[]);
                            }
                        }
                    }
                }
            });
            actions_inner.append(&rotate_btn);

            let open_btn = make_quick_action_button(
                "document-open-symbolic",
                if es { "Abrir" } else { "Open" },
            );
            let paths_open = file_paths.clone();
            open_btn.connect_clicked(move |_| {
                for p in &paths_open {
                    let _ = open_file(p);
                }
            });
            actions_inner.append(&open_btn);
        } else {
            let open_btn = make_quick_action_button(
                "document-open-symbolic",
                if es { "Abrir" } else { "Open" },
            );
            let paths_open = file_paths.clone();
            let first_dir = items.iter().find(|it| it.is_dir).map(|it| it.path.clone());
            let on_nav = self.on_navigate.clone();
            open_btn.connect_clicked(move |_| {
                if !paths_open.is_empty() {
                    for p in &paths_open {
                        let _ = open_file(p);
                    }
                } else if let Some(ref dir) = first_dir {
                    on_nav(dir.clone());
                }
            });
            actions_inner.append(&open_btn);

            let copy_btn = make_quick_action_button(
                "edit-copy-symbolic",
                if es {
                    "Copiar\nrutas"
                } else {
                    "Copy\nPaths"
                },
            );
            let joined_c = all_paths_joined.clone();
            copy_btn.connect_clicked(move |_| {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&joined_c);
                }
            });
            actions_inner.append(&copy_btn);
        }

        // Acción 3: Más...
        let more_btn = make_quick_action_button(
            "view-more-horizontal-symbolic",
            if es { "Más..." } else { "More..." },
        );
        let more_pop = Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        more_pop.set_parent(&more_btn);

        let more_menu = GtkBox::new(Orientation::Vertical, 1);
        more_menu.set_margin_top(4);
        more_menu.set_margin_bottom(4);
        more_menu.set_margin_start(4);
        more_menu.set_margin_end(4);

        let add_pop_item = |menu: &GtkBox, label: &str| -> Button {
            let b = Button::builder()
                .label(label)
                .has_frame(false)
                .css_classes(["popover-menu-item"])
                .build();
            if let Some(l) = b.child().and_then(|c| c.downcast::<Label>().ok()) {
                l.set_xalign(0.0);
            }
            crate::app_window::attach_squircle_menu_hover(&b);
            menu.append(&b);
            b
        };

        let pop_open = add_pop_item(
            &more_menu,
            if es {
                "Abrir seleccionados"
            } else {
                "Open Selected"
            },
        );
        let pop_copy = add_pop_item(
            &more_menu,
            if es { "Copiar rutas" } else { "Copy Paths" },
        );
        let pop_term = add_pop_item(
            &more_menu,
            if es {
                "Abrir carpeta en Terminal"
            } else {
                "Open Folder in Terminal"
            },
        );
        more_pop.set_child(Some(&more_menu));

        {
            let paths = file_paths;
            let pop = more_pop.clone();
            pop_open.connect_clicked(move |_| {
                pop.popdown();
                for p in &paths {
                    let _ = open_file(p);
                }
            });
        }
        {
            let joined = all_paths_joined;
            let pop = more_pop.clone();
            pop_copy.connect_clicked(move |_| {
                pop.popdown();
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&joined);
                }
            });
        }
        {
            let dir = items
                .first()
                .and_then(|it| it.path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("/"));
            let pop = more_pop.clone();
            pop_term.connect_clicked(move |_| {
                pop.popdown();
                open_in_terminal(&dir);
            });
        }
        {
            let pop = more_pop.clone();
            more_btn.connect_clicked(move |_| {
                pop.popup();
            });
        }
        {
            let pop_weak = more_pop.downgrade();
            more_btn.connect_unrealize(move |_| {
                if let Some(p) = pop_weak.upgrade() {
                    p.popdown();
                    if p.parent().is_some() {
                        p.unparent();
                    }
                }
            });
        }

        actions_inner.append(&more_btn);
        actions_bar.append(&actions_inner);
        preview_box.append(&actions_bar);

        preview_box
    }

    fn create_preview_pane(self: &Rc<Self>, item: &FileItem) -> GtkBox {
        let es = malus_de_common::is_spanish_locale();

        let preview_box = GtkBox::new(Orientation::Vertical, 0);
        preview_box.add_css_class("branches-column-preview");
        preview_box.set_size_request(PREVIEW_PANE_WIDTH, -1);
        preview_box.set_halign(Align::End);
        preview_box.set_hexpand(false);
        preview_box.set_vexpand(true);

        let content_vbox = GtkBox::new(Orientation::Vertical, 0);
        content_vbox.set_margin_top(22);
        content_vbox.set_margin_bottom(12);
        content_vbox.set_margin_start(PREVIEW_CONTENT_MARGIN);
        content_vbox.set_margin_end(PREVIEW_CONTENT_MARGIN);

        let ext_lower = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let is_img = matches!(
            ext_lower.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp"
        );
        let can_save_rotation = matches!(ext_lower.as_str(), "png" | "jpg" | "jpeg" | "bmp");

        // 1. Vista previa grande centrada con esquinas redondeadas y sombra suave
        let thumb_box = GtkBox::new(Orientation::Vertical, 0);
        thumb_box.set_halign(Align::Fill);
        thumb_box.set_hexpand(true);
        thumb_box.add_css_class("branches-preview-thumb-box");

        let preview_pixbuf_state: Rc<RefCell<Option<Pixbuf>>> = Rc::new(RefCell::new(None));
        let preview_draw_area: Rc<RefCell<Option<DrawingArea>>> = Rc::new(RefCell::new(None));

        if is_img {
            if let Ok(pb) = Pixbuf::from_file_at_scale(&item.path, 640, 440, true) {
                *preview_pixbuf_state.borrow_mut() = Some(pb);
                let area = build_rounded_image_preview(preview_pixbuf_state.clone());
                *preview_draw_area.borrow_mut() = Some(area.clone());
                thumb_box.append(&area);
            } else {
                let img = crate::views::create_folder_aware_icon(item, 112);
                img.set_halign(Align::Center);
                thumb_box.append(&img);
            }
        } else if !item.is_dir && malus_de_common::video_thumb::is_video_path(&item.path) {
            let (media_w, _) =
                crate::views::create_item_media_widget(item, 220, Some("branches-preview-image"));
            media_w.set_halign(Align::Center);
            thumb_box.append(&media_w);
        } else {
            let img = crate::views::create_folder_aware_icon(item, 112);
            img.set_halign(Align::Center);
            thumb_box.append(&img);
        }
        content_vbox.append(&thumb_box);

        // 2. Nombre del archivo (alineado a la izquierda, estilo Finder)
        let name_label = Label::builder()
            .label(&item.name)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .max_width_chars(28)
            .xalign(0.0)
            .halign(Align::Fill)
            .margin_top(16)
            .css_classes(["branches-preview-title"])
            .build();
        content_vbox.append(&name_label);

        // 3. Subtítulo: «Tipo - Tamaño» (ej. Imagen PNG - 8.1 MB)
        let subtitle_text = format_preview_subtitle(&item.kind_str, &item.size_str);
        let kind_label = Label::builder()
            .label(&subtitle_text)
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(36)
            .xalign(0.0)
            .halign(Align::Fill)
            .margin_top(2)
            .css_classes(["branches-preview-subtitle"])
            .build();
        content_vbox.append(&kind_label);

        // 4. Encabezado «Información» + enlace «Mostrar más» / «Mostrar menos»
        let info_header = GtkBox::new(Orientation::Horizontal, 8);
        info_header.set_margin_top(16);
        info_header.set_margin_bottom(6);
        info_header.append(
            &Label::builder()
                .label(if es { "Información" } else { "Information" })
                .xalign(0.0)
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-preview-section-title"])
                .build(),
        );
        let show_more_btn = Button::builder()
            .label(if es { "Mostrar más" } else { "Show More" })
            .has_frame(false)
            .css_classes(["branches-preview-more-link"])
            .build();
        info_header.append(&show_more_btn);
        content_vbox.append(&info_header);

        // 5. Filas de metadatos estilo Finder (clave a la izquierda, valor a la derecha, divisor sutil)
        let meta_box = GtkBox::new(Orientation::Vertical, 0);
        let mut first_row = true;
        let append_meta_row = |parent: &GtkBox, first: &mut bool, key: &str, val: &str| -> Label {
            if !*first {
                let div = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(["branches-preview-meta-divider"])
                    .hexpand(true)
                    .build();
                parent.append(&div);
            }
            *first = false;

            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.add_css_class("branches-preview-meta-row");
            row.append(
                &Label::builder()
                    .label(key)
                    .halign(Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["branches-preview-meta-key"])
                    .build(),
            );
            let val_lbl = Label::builder()
                .label(val)
                .halign(Align::End)
                .xalign(1.0)
                .ellipsize(pango::EllipsizeMode::Middle)
                .max_width_chars(22)
                .selectable(true)
                .css_classes(["branches-preview-meta-val"])
                .build();
            row.append(&val_lbl);
            parent.append(&row);
            val_lbl
        };

        let created_str = item
            .created
            .map(format_modified)
            .unwrap_or_else(|| item.modified_str.clone());
        let accessed_str = item
            .accessed
            .map(format_modified)
            .unwrap_or_else(|| item.modified_str.clone());

        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Creación" } else { "Created" },
            &created_str,
        );
        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Modificación" } else { "Modified" },
            &item.modified_str,
        );
        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Última apertura" } else { "Last opened" },
            &accessed_str,
        );

        let mut dims_val_label: Option<Label> = None;
        if is_img {
            if let Some((_, w, h)) = Pixbuf::file_info(&item.path) {
                let dims_lbl = append_meta_row(
                    &meta_box,
                    &mut first_row,
                    if es { "Dimensiones" } else { "Dimensions" },
                    &format_dimensions(w, h),
                );
                dims_val_label = Some(dims_lbl);
            }
            let (dpi_x, dpi_y) = read_image_dpi(&item.path).unwrap_or((72, 72));
            append_meta_row(
                &meta_box,
                &mut first_row,
                if es { "Resolución" } else { "Resolution" },
                &format_dimensions(dpi_x as i32, dpi_y as i32),
            );
        }
        content_vbox.append(&meta_box);

        // Metadatos adicionales ocultos hasta pulsar «Mostrar más»
        let extra_meta_box = GtkBox::new(Orientation::Vertical, 0);
        extra_meta_box.set_visible(false);
        let mut extra_first = false; // siempre lleva divisor superior para continuar la tabla
        let perms = format_posix_permissions(&item.path);
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Permisos" } else { "Permissions" },
            &perms,
        );
        let parent_dir_str = item
            .path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Ubicación" } else { "Where" },
            &parent_dir_str,
        );
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Tamaño exacto" } else { "Exact size" },
            &format!("{} bytes", item.size),
        );
        content_vbox.append(&extra_meta_box);

        {
            let extra_c = extra_meta_box.clone();
            let expanded = Cell::new(false);
            show_more_btn.connect_clicked(move |btn| {
                let next = !expanded.get();
                expanded.set(next);
                extra_c.set_visible(next);
                btn.set_label(if next {
                    if es {
                        "Mostrar menos"
                    } else {
                        "Show Less"
                    }
                } else if es {
                    "Mostrar más"
                } else {
                    "Show More"
                });
            });
        }

        let scrolled_preview = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .propagate_natural_width(false)
            .max_content_width(PREVIEW_PANE_WIDTH)
            .vexpand(true)
            .child(&content_vbox)
            .build();
        preview_box.append(&scrolled_preview);

        // 6. Barra inferior fija de Acciones Rápidas (idéntica a macOS Tahoe Finder)
        let actions_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .halign(Align::Fill)
            .hexpand(true)
            .css_classes(["branches-preview-actions-bar"])
            .build();
        let actions_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(22)
            .halign(Align::Center)
            .hexpand(true)
            .build();

        if is_img {
            // Acción 1: Girar a la izquierda (en vivo)
            let rotate_btn = make_quick_action_button(
                "object-rotate-left-symbolic",
                if es {
                    "Girar a la\nizquierda"
                } else {
                    "Rotate\nLeft"
                },
            );
            let path_rot = item.path.clone();
            let pb_state = preview_pixbuf_state.clone();
            let area_state = preview_draw_area.clone();
            let dims_lbl_c = dims_val_label.clone();
            let ext_rot = ext_lower.clone();
            rotate_btn.connect_clicked(move |_| {
                if let Some(cur) = pb_state.borrow().as_ref() {
                    if let Some(rotated) = cur.rotate_simple(PixbufRotation::Counterclockwise) {
                        *pb_state.borrow_mut() = Some(rotated);
                        if let Some(area) = area_state.borrow().as_ref() {
                            area.queue_draw();
                        }
                    }
                }
                if can_save_rotation {
                    if let Ok(full_pb) = Pixbuf::from_file(&path_rot) {
                        if let Some(rot_full) =
                            full_pb.rotate_simple(PixbufRotation::Counterclockwise)
                        {
                            let fmt = match ext_rot.as_str() {
                                "jpg" | "jpeg" => "jpeg",
                                "bmp" => "bmp",
                                _ => "png",
                            };
                            if rot_full.savev(&path_rot, fmt, &[]).is_ok() {
                                if let Some(ref lbl) = dims_lbl_c {
                                    lbl.set_label(&format_dimensions(
                                        rot_full.width(),
                                        rot_full.height(),
                                    ));
                                }
                            }
                        }
                    }
                }
            });
            actions_inner.append(&rotate_btn);

            // Acción 2: Marcado (abre el archivo en su editor/visor)
            let markup_btn = make_quick_action_button(
                "document-edit-symbolic",
                if es { "Marcado" } else { "Markup" },
            );
            let path_markup = item.path.clone();
            markup_btn.connect_clicked(move |_| {
                let _ = open_file(&path_markup);
            });
            actions_inner.append(&markup_btn);
        } else {
            // Acción 1 (no-imagen): Abrir
            let open_btn = make_quick_action_button(
                "document-open-symbolic",
                if es { "Abrir" } else { "Open" },
            );
            let path_open = item.path.clone();
            open_btn.connect_clicked(move |_| {
                let _ = open_file(&path_open);
            });
            actions_inner.append(&open_btn);

            // Acción 2 (no-imagen): Vista rápida
            let ql_btn = make_quick_action_button(
                "view-reveal-symbolic",
                if es { "Vista rápida" } else { "Quick Look" },
            );
            let path_ql = item.path.to_string_lossy().to_string();
            ql_btn.connect_clicked(move |b| {
                let _ = b.activate_action("win.quick_look_item", Some(&path_ql.to_variant()));
            });
            actions_inner.append(&ql_btn);
        }

        // Acción 3: Más... (popover con acciones reales)
        let more_btn = make_quick_action_button(
            "view-more-horizontal-symbolic",
            if es { "Más..." } else { "More..." },
        );
        let more_pop = Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        more_pop.set_parent(&more_btn);

        let more_menu = GtkBox::new(Orientation::Vertical, 1);
        more_menu.set_margin_top(4);
        more_menu.set_margin_bottom(4);
        more_menu.set_margin_start(4);
        more_menu.set_margin_end(4);

        let add_pop_item = |menu: &GtkBox, label: &str| -> Button {
            let b = Button::builder()
                .label(label)
                .has_frame(false)
                .css_classes(["popover-menu-item"])
                .build();
            if let Some(l) = b.child().and_then(|c| c.downcast::<Label>().ok()) {
                l.set_xalign(0.0);
            }
            crate::app_window::attach_squircle_menu_hover(&b);
            menu.append(&b);
            b
        };

        let pop_open = add_pop_item(
            &more_menu,
            if es { "Abrir archivo" } else { "Open File" },
        );
        let pop_ql = add_pop_item(
            &more_menu,
            if es { "Vista rápida" } else { "Quick Look" },
        );
        let pop_info = add_pop_item(
            &more_menu,
            if es {
                "Obtener información"
            } else {
                "Get Info"
            },
        );
        let pop_term = add_pop_item(
            &more_menu,
            if es {
                "Abrir en Terminal"
            } else {
                "Open in Terminal"
            },
        );
        let pop_copy = add_pop_item(
            &more_menu,
            if es { "Copiar ruta" } else { "Copy Path" },
        );
        more_pop.set_child(Some(&more_menu));

        {
            let p = item.path.clone();
            let pop = more_pop.clone();
            pop_open.connect_clicked(move |_| {
                pop.popdown();
                let _ = open_file(&p);
            });
        }
        {
            let p_str = item.path.to_string_lossy().to_string();
            let pop = more_pop.clone();
            let anchor = more_btn.clone();
            pop_ql.connect_clicked(move |_| {
                pop.popdown();
                let _ = anchor.activate_action("win.quick_look_item", Some(&p_str.to_variant()));
            });
        }
        {
            let p_str = item.path.to_string_lossy().to_string();
            let pop = more_pop.clone();
            let anchor = more_btn.clone();
            pop_info.connect_clicked(move |_| {
                pop.popdown();
                let _ = anchor.activate_action("win.get_info_item", Some(&p_str.to_variant()));
            });
        }
        {
            let p = item.path.clone();
            let pop = more_pop.clone();
            pop_term.connect_clicked(move |_| {
                pop.popdown();
                open_in_terminal(&p);
            });
        }
        {
            let p_str = item.path.to_string_lossy().to_string();
            let pop = more_pop.clone();
            pop_copy.connect_clicked(move |_| {
                pop.popdown();
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&p_str);
                }
            });
        }
        {
            let pop = more_pop.clone();
            more_btn.connect_clicked(move |_| {
                pop.popup();
            });
        }
        {
            let pop_weak = more_pop.downgrade();
            more_btn.connect_unrealize(move |_| {
                if let Some(p) = pop_weak.upgrade() {
                    p.popdown();
                    if p.parent().is_some() {
                        p.unparent();
                    }
                }
            });
        }

        actions_inner.append(&more_btn);
        actions_bar.append(&actions_inner);
        preview_box.append(&actions_bar);

        preview_box
    }

    /// Desplaza suavemente el contenedor hacia la izquierda (hasta la última columna),
    /// de modo que la columna anterior salga de vista al entrar en una carpeta.
    fn scroll_to_end(&self) {
        thread_local! {
            static SCROLL_GEN: Cell<u64> = const { Cell::new(0) };
        }
        const DURATION_US: f64 = 320_000.0;

        let generation = SCROLL_GEN.with(|g| {
            g.set(g.get().wrapping_add(1));
            g.get()
        });
        let hadj = self.scrolled.hadjustment();
        let start_value = hadj.value();
        let start_time: Cell<Option<i64>> = Cell::new(None);
        self.scrolled.add_tick_callback(move |_, clock| {
            if SCROLL_GEN.with(|g| g.get()) != generation {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let t0 = match start_time.get() {
                Some(v) => v,
                None => {
                    start_time.set(Some(now));
                    now
                }
            };
            let t = ((now - t0) as f64 / DURATION_US).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            // El destino se recalcula en cada fotograma: el layout de la nueva columna
            // puede terminar de medirse durante la animación.
            let target = (hadj.upper() - hadj.page_size()).max(0.0);
            if target > start_value {
                hadj.set_value(start_value + (target - start_value) * eased);
            }
            if t >= 1.0 {
                if target > 0.0 {
                    hadj.set_value(target);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

/// Fundido suave para el panel lateral derecho de previsualización (sin desplazar las columnas).
fn fade_in_widget(widget: &impl IsA<gtk4::Widget>) {
    const DURATION_US: f64 = 180_000.0;
    let widget = widget.as_ref();
    widget.set_opacity(0.0);
    let start: Cell<Option<i64>> = Cell::new(None);
    widget.add_tick_callback(move |w, clock| {
        let now = clock.frame_time();
        let t0 = match start.get() {
            Some(v) => v,
            None => {
                start.set(Some(now));
                now
            }
        };
        let t = ((now - t0) as f64 / DURATION_US).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        w.set_opacity(eased);
        if t >= 1.0 {
            w.set_opacity(1.0);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// Anima la entrada de una columna nueva: aparece deslizándose desde la derecha
/// (y con fundido), de modo que se note que se entró a la carpeta aunque quepan todas
/// las columnas en pantalla.
fn slide_in_from_right(widget: &impl IsA<gtk4::Widget>, distance: i32) {
    const DURATION_US: f64 = 300_000.0;
    let widget = widget.as_ref();
    widget.set_margin_start(distance);
    widget.set_opacity(0.0);
    let start: Cell<Option<i64>> = Cell::new(None);
    widget.add_tick_callback(move |w, clock| {
        let now = clock.frame_time();
        let t0 = match start.get() {
            Some(v) => v,
            None => {
                start.set(Some(now));
                now
            }
        };
        let t = ((now - t0) as f64 / DURATION_US).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        w.set_margin_start(((1.0 - eased) * distance as f64).round() as i32);
        w.set_opacity(eased);
        if t >= 1.0 {
            w.set_margin_start(0);
            w.set_opacity(1.0);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn redraw_all_column_row_bgs(list_box: &ListBox) {
    let mut child = list_box.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<ListBoxRow>() {
            if let Some(bg) = row
                .child()
                .and_then(|c| c.downcast::<Overlay>().ok())
                .and_then(|ov| ov.first_child())
                .and_then(|c| c.downcast::<DrawingArea>().ok())
            {
                bg.queue_draw();
            }
        }
        child = w.next_sibling();
    }
}

fn create_column_row(
    item: &FileItem,
    col_items: Rc<Vec<FileItem>>,
    tags_map: &std::collections::HashMap<PathBuf, Vec<(String, String)>>,
    on_navigate: &NavigateCallback,
) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.add_css_class("branches-column-row");

    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    if item.is_hidden {
        hbox.add_css_class("branches-hidden-item");
    }
    if crate::operations::is_path_cut(&item.path) {
        hbox.add_css_class("branches-cut-item");
    }
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(10);
    hbox.set_margin_end(10);

    let icon = crate::views::create_item_image(item, 16, Some("branches-row-thumb"));
    icon.set_size_request(16, 16);
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    hbox.append(&icon);

    let label = Label::builder()
        .label(&item.name)
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["branches-column-name-label"])
        .build();
    hbox.append(&label);

    if let Some(tag_dots) = crate::views::create_item_tag_dots(&item.path, tags_map) {
        hbox.append(&tag_dots);
    }

    if item.is_dir {
        let chevron = Image::from_icon_name("pan-end-symbolic");
        chevron.set_pixel_size(12);
        chevron.add_css_class("branches-column-chevron");
        hbox.append(&chevron);
    }

    // Fondo squircle (superelipse n = 4.2) con esquinas redondeadas solo al inicio y fin de bloques contiguos
    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let row_weak = row.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let Some(r) = row_weak.upgrade() else {
            return;
        };
        let flags = r.state_flags();
        let selected = flags.contains(gtk4::StateFlags::SELECTED);
        let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
        if !selected && !hovered {
            return;
        }
        let radius = (height * 0.38).clamp(7.0, 11.0);
        let _ = cr.save();
        if selected {
            let prev_selected = r
                .prev_sibling()
                .and_then(|s| s.downcast::<ListBoxRow>().ok())
                .is_some_and(|prev| prev.is_selectable() && prev.is_selected());
            let next_selected = r
                .next_sibling()
                .and_then(|s| s.downcast::<ListBoxRow>().ok())
                .is_some_and(|next| next.is_selectable() && next.is_selected());
            crate::app_window::squircle_path_corners(
                cr,
                0.0,
                0.0,
                width,
                height,
                radius,
                !prev_selected,
                !next_selected,
            );
            let (red, green, blue) = malus_de_common::config::load_accent_color();
            cr.set_source_rgba(red, green, blue, 0.90);
        } else {
            crate::app_window::squircle_path_corners(
                cr, 0.0, 0.0, width, height, radius, true, true,
            );
            let fg = r.color();
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.06);
        }
        let _ = cr.fill();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    row.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = Overlay::builder().hexpand(true).vexpand(false).build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&hbox);
    overlay.set_measure_overlay(&hbox, true);
    row.set_child(Some(&overlay));

    let click = GestureClick::new();
    click.set_button(0);
    let path_for_menu = item.path.clone();
    let is_dir_for_menu = item.is_dir;
    let on_nav_menu = on_navigate.clone();
    let row_for_menu = row.clone();
    let pending_slow_rename: Rc<std::cell::Cell<Option<glib::SourceId>>> =
        Rc::new(std::cell::Cell::new(None));
    let pending_slow_click = pending_slow_rename.clone();
    click.connect_pressed(move |g, n_press, x, y| {
        let button = g.current_button();
        if let Some(source_id) = pending_slow_click.take() {
            source_id.remove();
        }
        if button == gdk::BUTTON_PRIMARY {
            let is_single_selected = row_for_menu
                .parent()
                .and_then(|p| p.downcast::<ListBox>().ok())
                .is_some_and(|lb| lb.selected_rows().len() == 1);
            if n_press == 1
                && row_for_menu.is_selected()
                && is_single_selected
                && !crate::views::is_multi_select_modifier(g.current_event_state())
            {
                let row_ren = row_for_menu.clone();
                let path_ren = path_for_menu.clone();
                let on_nav_ren = on_nav_menu.clone();
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
                            is_dir_for_menu,
                            None,
                            on_renamed,
                        );
                    },
                );
                pending_slow_click.set(Some(id));
            } else if n_press == 2 {
                if is_dir_for_menu {
                    on_nav_menu(path_for_menu.clone());
                } else {
                    let _ = open_file(&path_for_menu);
                }
            }
        } else if button == gdk::BUTTON_SECONDARY {
            g.set_state(gtk4::EventSequenceState::Claimed);
            if let Some(lb) = row_for_menu
                .parent()
                .and_then(|p| p.downcast::<ListBox>().ok())
            {
                if !row_for_menu.is_selected() {
                    lb.unselect_all();
                    lb.select_row(Some(&row_for_menu));
                }
            }
            show_item_context_menu(
                &row_for_menu,
                &path_for_menu,
                is_dir_for_menu,
                Some((x, y)),
                Some(&on_nav_menu),
                None,
            );
        }
    });
    row.add_controller(click);

    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
    let path_for_drag = item.path.clone();
    let row_for_drag = row.clone();
    drag_source.connect_prepare(move |_, _x, _y| {
        Some(crate::views::prepare_list_row_drag(
            &row_for_drag,
            &path_for_drag,
            &col_items,
        ))
    });
    let cancel_slow_drag = pending_slow_rename;
    drag_source.connect_drag_begin(move |_, _| {
        if let Some(source_id) = cancel_slow_drag.take() {
            source_id.remove();
        }
    });
    let path_end = item.path.clone();
    let on_nav_end = on_navigate.clone();
    drag_source.connect_drag_end(move |_, drag, _| {
        if let Some(parent) = path_end.parent() {
            let _ = crate::views::refresh_if_moved_out(drag, &path_end, parent, &on_nav_end);
        }
    });
    if let Some(display) = gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        let paintable = theme.lookup_by_gicon(
            &item.gicon,
            24,
            1,
            gtk4::TextDirection::Ltr,
            gtk4::IconLookupFlags::empty(),
        );
        drag_source.set_icon(Some(&paintable), 12, 12);
    }
    row.add_controller(drag_source);

    row
}

#[cfg(unix)]
pub(super) fn format_posix_permissions(path: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        let mode = meta.permissions().mode();
        let is_dir = meta.is_dir();
        let is_symlink = meta.file_type().is_symlink();
        let ftype = if is_dir {
            'd'
        } else if is_symlink {
            'l'
        } else {
            '-'
        };
        let rwx = |r: u32, w: u32, x: u32| -> String {
            format!(
                "{}{}{}",
                if mode & r != 0 { "r" } else { "-" },
                if mode & w != 0 { "w" } else { "-" },
                if mode & x != 0 { "x" } else { "-" },
            )
        };
        format!(
            "{}{}{}{} ({:04o})",
            ftype,
            rwx(0o400, 0o200, 0o100),
            rwx(0o040, 0o020, 0o010),
            rwx(0o004, 0o002, 0o001),
            mode & 0o7777
        )
    } else {
        "--".to_string()
    }
}

#[cfg(not(unix))]
pub(super) fn format_posix_permissions(_path: &Path) -> String {
    "--".to_string()
}

pub(super) fn format_preview_subtitle(kind: &str, size: &str) -> String {
    let k = kind.trim();
    let s = size.trim();
    if s.is_empty() || s == "--" {
        k.to_string()
    } else if k.is_empty() {
        s.to_string()
    } else {
        format!("{k} - {s}")
    }
}

pub(super) fn format_dimensions(w: i32, h: i32) -> String {
    format!("{w} × {h}")
}

pub(super) fn make_quick_action_button(icon_name: &str, label_text: &str) -> Button {
    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .halign(Align::Center)
        .valign(Align::Start)
        .build();
    let icon = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .halign(Align::Center)
        .css_classes(["branches-preview-action-icon"])
        .build();
    let lbl = Label::builder()
        .label(label_text)
        .justify(gtk4::Justification::Center)
        .halign(Align::Center)
        .css_classes(["branches-preview-action-label"])
        .build();
    vbox.append(&icon);
    vbox.append(&lbl);

    Button::builder()
        .has_frame(false)
        .child(&vbox)
        .css_classes(["branches-preview-quick-btn"])
        .valign(Align::Start)
        .build()
}

pub(super) fn rounded_rect_path(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
) {
    let r = r.min(w * 0.5).min(h * 0.5);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

const PREVIEW_PANE_WIDTH: i32 = 380;
const PREVIEW_CONTENT_MARGIN: i32 = 22;
const PREVIEW_THUMB_WIDTH: i32 = PREVIEW_PANE_WIDTH - PREVIEW_CONTENT_MARGIN * 2;

/// Dibuja la miniatura de imagen con esquinas redondeadas de 8 px y sombra suave estilo Finder.
fn build_rounded_image_preview(pixbuf_state: Rc<RefCell<Option<Pixbuf>>>) -> DrawingArea {
    let area = DrawingArea::builder()
        .content_width(PREVIEW_THUMB_WIDTH)
        .content_height(210)
        .hexpand(false)
        .vexpand(false)
        .build();

    area.set_draw_func(move |_, cr, width, height| {
        let borrow = pixbuf_state.borrow();
        let Some(ref pb) = *borrow else {
            return;
        };
        let pw = pb.width() as f64;
        let ph = pb.height() as f64;
        if pw <= 0.0 || ph <= 0.0 {
            return;
        }
        let pad_x = 4.0;
        let pad_top = 4.0;
        let pad_bot = 10.0;
        let avail_w = (width as f64 - pad_x * 2.0).max(16.0);
        let avail_h = (height as f64 - pad_top - pad_bot).max(16.0);
        let scale = (avail_w / pw).min(avail_h / ph).min(1.0);
        let draw_w = (pw * scale).round();
        let draw_h = (ph * scale).round();
        let dx = ((width as f64 - draw_w) * 0.5).round();
        let dy = (pad_top + (avail_h - draw_h) * 0.5).round();
        let radius = 8.0;

        // Sombra suave debajo de la imagen
        for (expand, offset_y, alpha) in [(3.0, 4.0, 0.05), (1.5, 2.5, 0.08), (0.5, 1.0, 0.07)] {
            rounded_rect_path(
                cr,
                dx - expand,
                dy - expand + offset_y,
                draw_w + expand * 2.0,
                draw_h + expand * 2.0,
                radius + expand * 0.5,
            );
            cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
            let _ = cr.fill();
        }

        // Recorte redondeado + dibujo escalado con alta calidad
        let _ = cr.save();
        rounded_rect_path(cr, dx, dy, draw_w, draw_h, radius);
        cr.clip();
        cr.translate(dx, dy);
        cr.scale(draw_w / pw, draw_h / ph);
        gtk4::prelude::GdkCairoContextExt::set_source_pixbuf(cr, pb, 0.0, 0.0);
        let _ = cr.paint();
        let _ = cr.restore();

        // Borde hairline sutil
        rounded_rect_path(cr, dx + 0.5, dy + 0.5, draw_w - 1.0, draw_h - 1.0, radius);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.08);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    });

    area
}

/// Extrae DPI embebido en encabezados PNG (`pHYs`) o JPEG (JFIF APP0).
fn parse_png_phys_dpi(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 20 || &bytes[..8] != PNG_SIG {
        return None;
    }
    let mut off = 8usize;
    while off + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[off..off + 4].try_into().ok()?) as usize;
        let chunk_type = &bytes[off + 4..off + 8];
        let data_start = off + 8;
        let data_end = data_start.checked_add(len)?;
        if data_end > bytes.len() {
            break;
        }
        if chunk_type == b"pHYs" && len == 9 {
            let ppm_x = u32::from_be_bytes(bytes[data_start..data_start + 4].try_into().ok()?);
            let ppm_y =
                u32::from_be_bytes(bytes[data_start + 4..data_start + 8].try_into().ok()?);
            let unit = bytes[data_start + 8];
            if unit == 1 && ppm_x > 0 && ppm_y > 0 {
                let dpi_x = ((ppm_x as f64) * 0.0254).round() as u32;
                let dpi_y = ((ppm_y as f64) * 0.0254).round() as u32;
                return Some((dpi_x.max(1), dpi_y.max(1)));
            }
        }
        if chunk_type == b"IDAT" || chunk_type == b"IEND" {
            break;
        }
        off = data_end.checked_add(4)?;
    }
    None
}

fn parse_jpeg_jfif_dpi(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 20 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut off = 2usize;
    while off + 4 <= bytes.len() {
        if bytes[off] != 0xFF {
            break;
        }
        let marker = bytes[off + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let seg_len = u16::from_be_bytes([bytes[off + 2], bytes[off + 3]]) as usize;
        if seg_len < 2 || off + 2 + seg_len > bytes.len() {
            break;
        }
        let data = &bytes[off + 4..off + 2 + seg_len];
        if marker == 0xE0 && data.len() >= 12 && &data[..5] == b"JFIF\0" {
            let units = data[7];
            let x_den = u16::from_be_bytes([data[8], data[9]]) as f64;
            let y_den = u16::from_be_bytes([data[10], data[11]]) as f64;
            if x_den > 0.0 && y_den > 0.0 {
                return match units {
                    1 => Some((x_den.round() as u32, y_den.round() as u32)),
                    2 => Some(((x_den * 2.54).round() as u32, (y_den * 2.54).round() as u32)),
                    _ => None,
                };
            }
        }
        off += 2 + seg_len;
    }
    None
}

pub(super) fn read_image_dpi(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    parse_png_phys_dpi(&buf).or_else(|| parse_jpeg_jfif_dpi(&buf))
}

pub(super) fn is_preview_image_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp"
    )
}

pub(super) struct MultiSelectionSummary {
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) breakdown: String,
    pub(super) kind_label: String,
    pub(super) total_bytes: u64,
    pub(super) total_size_str: String,
    pub(super) latest_modified: Option<std::time::SystemTime>,
    pub(super) earliest_created: Option<std::time::SystemTime>,
    pub(super) latest_accessed: Option<std::time::SystemTime>,
}

pub(super) fn summarize_multi_selection(items: &[FileItem], es: bool) -> MultiSelectionSummary {
    let count = items.len();
    let dir_count = items.iter().filter(|it| it.is_dir).count();
    let file_count = count.saturating_sub(dir_count);

    let title = if es {
        format!("{count} elementos")
    } else {
        format!("{count} items")
    };

    let breakdown = if es {
        match (file_count, dir_count) {
            (f, 0) => format!("{f} {}", if f == 1 { "archivo" } else { "archivos" }),
            (0, d) => format!("{d} {}", if d == 1 { "carpeta" } else { "carpetas" }),
            (f, d) => format!(
                "{f} {}, {d} {}",
                if f == 1 { "archivo" } else { "archivos" },
                if d == 1 { "carpeta" } else { "carpetas" }
            ),
        }
    } else {
        match (file_count, dir_count) {
            (f, 0) => format!("{f} {}", if f == 1 { "file" } else { "files" }),
            (0, d) => format!("{d} {}", if d == 1 { "folder" } else { "folders" }),
            (f, d) => format!(
                "{f} {}, {d} {}",
                if f == 1 { "file" } else { "files" },
                if d == 1 { "folder" } else { "folders" }
            ),
        }
    };

    let total_bytes: u64 = items
        .iter()
        .filter(|it| !it.is_dir)
        .fold(0u64, |acc, it| acc.saturating_add(it.size));
    let total_size_str = if file_count == 0 {
        "--".to_string()
    } else {
        format_size(total_bytes)
    };

    let first_kind = items.first().map(|it| it.kind_str.as_str()).unwrap_or("");
    let all_same_kind =
        !first_kind.is_empty() && items.iter().all(|it| it.kind_str == first_kind);
    let kind_label = if all_same_kind {
        first_kind.to_string()
    } else if es {
        "Varios tipos".to_string()
    } else {
        "Multiple kinds".to_string()
    };

    let subtitle = format_preview_subtitle(&kind_label, &total_size_str);
    let latest_modified = items.iter().filter_map(|it| it.modified).max();
    let earliest_created = items
        .iter()
        .filter_map(|it| it.created.or(it.modified))
        .min();
    let latest_accessed = items.iter().filter_map(|it| it.accessed).max();

    MultiSelectionSummary {
        title,
        subtitle,
        breakdown,
        kind_label,
        total_bytes,
        total_size_str,
        latest_modified,
        earliest_created,
        latest_accessed,
    }
}

/// Dibuja una pila en abanico de hasta 3 imágenes con esquinas redondeadas y sombra suave.
fn build_multi_image_stack_preview(pixbufs: Rc<RefCell<Vec<Pixbuf>>>) -> DrawingArea {
    let area = DrawingArea::builder()
        .content_width(PREVIEW_THUMB_WIDTH)
        .content_height(210)
        .hexpand(false)
        .vexpand(false)
        .build();

    area.set_draw_func(move |_, cr, width, height| {
        let borrow = pixbufs.borrow();
        let n = borrow.len().min(3);
        if n == 0 {
            return;
        }
        let cx = width as f64 * 0.5;
        let cy = height as f64 * 0.5;
        let avail_w = (width as f64 - 44.0).max(40.0);
        let avail_h = (height as f64 - 36.0).max(40.0);
        let radius = 8.0;

        // Transformaciones (ángulo en rad, offset_x, offset_y, escala) de frente (0) hacia atrás
        let layouts: &[(f64, f64, f64, f64)] = match n {
            1 => &[(0.0, 0.0, 0.0, 1.0)],
            2 => &[(0.0, -5.0, 4.0, 0.96), (0.075, 9.0, -4.0, 0.92)],
            _ => &[
                (0.0, 0.0, 5.0, 0.95),
                (0.085, 11.0, -1.0, 0.91),
                (-0.095, -11.0, -5.0, 0.88),
            ],
        };

        for idx in (0..n).rev() {
            let pb = &borrow[idx];
            let pw = pb.width() as f64;
            let ph = pb.height() as f64;
            if pw <= 0.0 || ph <= 0.0 {
                continue;
            }
            let (angle, ox, oy, layer_scale) = layouts[idx];
            let scale = (avail_w / pw).min(avail_h / ph).min(1.0) * layer_scale;
            let draw_w = (pw * scale).round();
            let draw_h = (ph * scale).round();
            let dx = -draw_w * 0.5;
            let dy = -draw_h * 0.5;

            let _ = cr.save();
            cr.translate(cx + ox, cy + oy);
            cr.rotate(angle);

            for (expand, offset_y, alpha) in [(3.0, 4.0, 0.05), (1.5, 2.5, 0.08), (0.5, 1.0, 0.07)]
            {
                rounded_rect_path(
                    cr,
                    dx - expand,
                    dy - expand + offset_y,
                    draw_w + expand * 2.0,
                    draw_h + expand * 2.0,
                    radius + expand * 0.5,
                );
                cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
                let _ = cr.fill();
            }

            let _ = cr.save();
            rounded_rect_path(cr, dx, dy, draw_w, draw_h, radius);
            cr.clip();
            cr.translate(dx, dy);
            cr.scale(draw_w / pw, draw_h / ph);
            gtk4::prelude::GdkCairoContextExt::set_source_pixbuf(cr, pb, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();

            rounded_rect_path(cr, dx + 0.5, dy + 0.5, draw_w - 1.0, draw_h - 1.0, radius);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let _ = cr.restore();
        }
    });

    area
}

/// Pila centrada de hasta 3 iconos para selecciones múltiples de documentos, carpetas o tipos mixtos.
fn build_multi_icon_stack(items: &[FileItem]) -> Overlay {
    let overlay = Overlay::builder()
        .height_request(148)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let shown: Vec<&FileItem> = items.iter().take(3).collect();
    let offsets: &[(i32, i32)] = match shown.len() {
        1 => &[(0, 0)],
        2 => &[(0, 6), (26, 0)],
        _ => &[(18, 10), (36, 4), (0, 0)],
    };
    let mut base_set = false;
    for (idx, it) in shown.iter().enumerate().rev() {
        let icon = crate::views::create_folder_aware_icon(it, 92);
        let (mx, my) = offsets[idx];
        icon.set_halign(Align::Start);
        icon.set_valign(Align::Start);
        icon.set_margin_start(mx);
        icon.set_margin_top(my);
        icon.set_margin_end(36 - mx);
        icon.set_margin_bottom(14 - my);
        if !base_set {
            overlay.set_child(Some(&icon));
            base_set = true;
        } else {
            overlay.add_overlay(&icon);
        }
    }
    overlay
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_test_item(name: &str, is_dir: bool, size: u64, kind: &str) -> FileItem {
        FileItem {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir,
            is_hidden: false,
            is_symlink: false,
            size,
            size_str: if is_dir {
                "--".to_string()
            } else {
                format_size(size)
            },
            modified: None,
            accessed: None,
            added: None,
            created: None,
            modified_str: "ayer, 3:15 pm".to_string(),
            kind_str: kind.to_string(),
            gicon: gtk4::gio::ThemedIcon::new("text-x-generic").upcast(),
        }
    }

    #[test]
    fn parse_png_phys_dpi_extracts_exact_resolution() {
        // 144 DPI = 5669 px/m
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&9u32.to_be_bytes());
        png.extend_from_slice(b"pHYs");
        png.extend_from_slice(&5669u32.to_be_bytes());
        png.extend_from_slice(&5669u32.to_be_bytes());
        png.push(1); // meters
        png.extend_from_slice(&0u32.to_be_bytes()); // crc placeholder
        assert_eq!(parse_png_phys_dpi(&png), Some((144, 144)));
    }

    #[test]
    fn format_dimensions_and_subtitle_match_finder_style() {
        assert_eq!(
            format_preview_subtitle("Imagen PNG", "8.1 MB"),
            "Imagen PNG - 8.1 MB"
        );
        assert_eq!(format_dimensions(3360, 2100), "3360 × 2100");
    }

    #[test]
    fn summarize_multi_selection_computes_totals_and_kinds() {
        let imgs = vec![
            mk_test_item("a.png", false, 1024 * 1024, "Imagen PNG"),
            mk_test_item("b.png", false, 2 * 1024 * 1024, "Imagen PNG"),
        ];
        let s1 = summarize_multi_selection(&imgs, true);
        assert_eq!(s1.title, "2 elementos");
        assert_eq!(s1.breakdown, "2 archivos");
        assert_eq!(s1.kind_label, "Imagen PNG");
        assert_eq!(s1.total_bytes, 3 * 1024 * 1024);
        assert!(s1.subtitle.starts_with("Imagen PNG - "));

        let mixed = vec![
            mk_test_item("a.png", false, 2048, "Imagen PNG"),
            mk_test_item("docs", true, 4096, "Carpeta"),
        ];
        let s2 = summarize_multi_selection(&mixed, true);
        assert_eq!(s2.title, "2 elementos");
        assert_eq!(s2.breakdown, "1 archivo, 1 carpeta");
        assert_eq!(s2.kind_label, "Varios tipos");
        assert_eq!(s2.total_bytes, 2048);

        let dirs = vec![
            mk_test_item("d1", true, 4096, "Carpeta"),
            mk_test_item("d2", true, 4096, "Carpeta"),
        ];
        let s3 = summarize_multi_selection(&dirs, true);
        assert_eq!(s3.breakdown, "2 carpetas");
        assert_eq!(s3.total_size_str, "--");
        assert_eq!(s3.subtitle, "Carpeta");
    }
}
