use crate::fs_backend::{load_directory, open_file, open_in_terminal, FileItem};
use crate::state::{SortBy, SortOrder};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{self, Key, Texture};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, EventControllerKey, GestureClick, Grid, Image, Label, ListBox,
    ListBoxRow, Orientation, PolicyType, ScrolledWindow, Separator,
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
                    if let Some(hbox) = row.child() {
                        crate::views::sync_cut_css_class(&hbox, &item.path);
                    }
                }
            }
        }
    }

    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
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

        let inner = Rc::new(ColumnViewInner {
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

    pub fn widget(&self) -> &ScrolledWindow {
        &self.inner.scrolled
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
        if let (Some(k), true) = (
            existing_idx,
            self.inner.pending_select_name.borrow().is_none(),
        ) {
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
        if let Some(col) = columns.get(target_idx) {
            self.inner.active_col_idx.set(Some(target_idx));
            self.inner.syncing_selection.set(true);
            col.list_box.select_all();
            self.inner.syncing_selection.set(false);
            self.inner.prune_after(target_idx);
            let count = col.list_box.selected_rows().len();
            if count == 1 {
                if let Some(item) = col.items.first() {
                    self.inner.on_item_selected(target_idx, item);
                    return;
                }
            }
            (self.inner.on_selection_changed)(count);
        }
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
        if let Some(prev) = self.preview_box.borrow_mut().take() {
            self.columns_box.remove(&prev);
        }
        for col in self.columns.borrow_mut().drain(..) {
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
        if let Some(prev) = self.preview_box.borrow_mut().take() {
            self.columns_box.remove(&prev);
        }
        let mut cols = self.columns.borrow_mut();
        while cols.len() > col_idx + 1 {
            if let Some(removed) = cols.pop() {
                self.columns_box.remove(&removed.container);
            }
        }
    }

    fn create_column_widget(
        self: &Rc<Self>,
        col_idx: usize,
        path: PathBuf,
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
        let self_bg = self.clone();
        let scrolled_bg = scrolled.clone();
        let col_dir = path.clone();
        bg_click.connect_pressed(move |g, _n, x, y| {
            if !self_bg.can_create.get() {
                return;
            }
            g.set_state(gtk4::EventSequenceState::Claimed);
            let nav = self_bg.on_navigate.clone();
            let dir_reload = col_dir.clone();
            let self_create = self_bg.clone();
            crate::views::show_background_create_menu(
                &scrolled_bg,
                x,
                y,
                col_dir.clone(),
                Rc::new(move |created_path| {
                    if let Some(new_name) = created_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                    {
                        *self_create.pending_select_name.borrow_mut() = Some(new_name);
                    }
                    nav(dir_reload.clone());

                    let inner_lookup = self_create.clone();
                    let target_path = created_path;
                    let inner_rename = self_create.clone();
                    let nav_rename = nav.clone();
                    let dir_rename = dir_reload.clone();
                    let on_renamed: crate::views::RenameCallback = Rc::new(move |renamed_path| {
                        if let Some(renamed_name) = renamed_path.file_name() {
                            *inner_rename.pending_select_name.borrow_mut() =
                                Some(renamed_name.to_string_lossy().into_owned());
                        }
                        nav_rename(dir_rename.clone());
                    });
                    crate::views::schedule_inline_rename(
                        move || {
                            let cols = inner_lookup.columns.borrow();
                            cols.iter().rev().find_map(|col| {
                                let (idx, item) = col
                                    .items
                                    .iter()
                                    .enumerate()
                                    .find(|(_, it)| it.path == target_path)?;
                                let row = col.list_box.row_at_index(idx as i32)?;
                                Some((item.path.clone(), item.is_dir, row.upcast()))
                            })
                        },
                        Some(on_renamed),
                    );
                }),
            );
        });
        scrolled.add_controller(bg_click);

        col_box.append(&scrolled);

        // Controlador de selección de filas (soporta selección múltiple y rango)
        let self_for_sel = self.clone();
        let items_for_sel = items_rc.clone();
        list_box.connect_selected_rows_changed(move |lb| {
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
                    let cols = self_for_key.columns.borrow();
                    if cols.len() > col_idx + 1 {
                        let next_lb = &cols[col_idx + 1].list_box;
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
                        let cols = self_for_key.columns.borrow();
                        if let Some(prev_col) = cols.get(col_idx - 1) {
                            prev_col.list_box.grab_focus();
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
            self.columns_box.append(&preview);
            slide_in_from_right(&preview, 280);
            *self.preview_box.borrow_mut() = Some(preview);
            self.scroll_to_end();
        }
    }

    fn create_preview_pane(self: &Rc<Self>, item: &FileItem) -> GtkBox {
        let preview_box = GtkBox::new(Orientation::Vertical, 0);
        preview_box.add_css_class("branches-column-preview");
        preview_box.set_size_request(280, -1);
        preview_box.set_vexpand(true);

        let content_vbox = GtkBox::new(Orientation::Vertical, 10);
        content_vbox.set_margin_top(16);
        content_vbox.set_margin_bottom(16);
        content_vbox.set_margin_start(14);
        content_vbox.set_margin_end(14);

        // 1. Miniatura o Icono grande
        let thumb_box = GtkBox::new(Orientation::Vertical, 0);
        thumb_box.set_halign(Align::Center);
        thumb_box.add_css_class("branches-preview-thumb-box");

        let is_img = ["png", "jpg", "jpeg", "webp", "gif", "svg", "bmp"]
            .iter()
            .any(|ext| {
                item.path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase() == *ext)
                    .unwrap_or(false)
            });

        let thumb_widget: gtk4::Widget = if is_img {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&item.path, 160, 160, true) {
                let texture = Texture::for_pixbuf(&pixbuf);
                let img = Image::from_paintable(Some(&texture));
                img.add_css_class("branches-preview-image");
                img.upcast()
            } else {
                let img = Image::from_gicon(&item.gicon);
                crate::views::apply_folder_icon_tint(&img, item);
                img.set_pixel_size(96);
                img.upcast()
            }
        } else {
            let img = Image::from_gicon(&item.gicon);
            crate::views::apply_folder_icon_tint(&img, item);
            img.set_pixel_size(96);
            img.upcast()
        };
        thumb_box.append(&thumb_widget);
        content_vbox.append(&thumb_box);

        // 2. Nombre del archivo
        let name_label = Label::builder()
            .label(&item.name)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .justify(gtk4::Justification::Center)
            .css_classes(["branches-preview-title"])
            .halign(Align::Center)
            .build();
        content_vbox.append(&name_label);

        // 3. Tipo / Clase
        let kind_label = Label::builder()
            .label(&item.kind_str)
            .css_classes(["branches-preview-subtitle", "dim-label"])
            .halign(Align::Center)
            .build();
        content_vbox.append(&kind_label);

        // 4. Separador
        let sep = Separator::new(Orientation::Horizontal);
        sep.set_margin_top(4);
        sep.set_margin_bottom(4);
        content_vbox.append(&sep);

        // 5. Tabla de Metadatos
        let grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .halign(Align::Fill)
            .build();

        let mut row_idx = 0;
        let add_meta_row = |g: &Grid, row: &mut i32, key: &str, val: &str| {
            let k_label = Label::builder()
                .label(key)
                .css_classes(["branches-preview-meta-key"])
                .halign(Align::End)
                .build();
            let v_label = Label::builder()
                .label(val)
                .css_classes(["branches-preview-meta-val"])
                .halign(Align::Start)
                .hexpand(true)
                .ellipsize(pango::EllipsizeMode::Middle)
                .selectable(true)
                .build();
            g.attach(&k_label, 0, *row, 1, 1);
            g.attach(&v_label, 1, *row, 1, 1);
            *row += 1;
        };

        add_meta_row(&grid, &mut row_idx, "Tamaño:", &item.size_str);
        add_meta_row(&grid, &mut row_idx, "Modificado:", &item.modified_str);

        let perms = format_posix_permissions(&item.path);
        add_meta_row(&grid, &mut row_idx, "Permisos:", &perms);

        let path_str = item.path.to_string_lossy().to_string();
        add_meta_row(&grid, &mut row_idx, "Ubicación:", &path_str);

        content_vbox.append(&grid);

        // 6. Botones de acción inferior
        let actions_box = GtkBox::new(Orientation::Vertical, 8);
        actions_box.set_margin_top(16);
        actions_box.set_halign(Align::Center);

        let open_btn = Button::builder()
            .label("Abrir archivo")
            .css_classes(["suggested-action", "branches-preview-btn"])
            .build();
        let p_for_open = item.path.clone();
        open_btn.connect_clicked(move |_| {
            let _ = open_file(&p_for_open);
        });
        actions_box.append(&open_btn);

        let p_for_term = item.path.clone();
        let term_btn = Button::builder()
            .label("Abrir en Terminal")
            .icon_name("utilities-terminal-symbolic")
            .css_classes(["flat", "branches-preview-btn"])
            .build();
        term_btn.connect_clicked(move |_| {
            open_in_terminal(&p_for_term);
        });
        actions_box.append(&term_btn);

        content_vbox.append(&actions_box);

        let scrolled_preview = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&content_vbox)
            .build();

        preview_box.append(&scrolled_preview);
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
    hbox.set_margin_top(3);
    hbox.set_margin_bottom(3);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

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

    row.set_child(Some(&hbox));

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
fn format_posix_permissions(path: &Path) -> String {
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
fn format_posix_permissions(_path: &Path) -> String {
    "--".to_string()
}
