use crate::fs_backend::{load_directory, open_file, open_in_terminal, FileItem};
use crate::state::{SortBy, SortOrder};
use crate::views::{NavigateCallback, SelectionCallback};
use gdk_pixbuf::Pixbuf;
use gtk4::gdk::{Key, Texture};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, EventControllerKey, Grid, Image, Label, ListBox,
    ListBoxRow, Orientation, PolicyType, ScrolledWindow, Separator,
};
use std::cell::RefCell;
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
    preview_box: RefCell<Option<GtkBox>>,
    current_root: RefCell<PathBuf>,
    show_hidden: RefCell<bool>,
    sort_by: RefCell<SortBy>,
    sort_order: RefCell<SortOrder>,
    on_navigate: NavigateCallback,
    on_selection_changed: SelectionCallback,
}

impl ColumnView {
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
            preview_box: RefCell::new(None),
            current_root: RefCell::new(PathBuf::from("/")),
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

        self.inner.clear_all();
        self.inner.push_column(0, path.to_path_buf(), items.to_vec());
    }

    pub fn get_selected_item(&self) -> Option<FileItem> {
        let columns = self.inner.columns.borrow();
        let last_col = columns.last()?;
        let selected_row = last_col.list_box.selected_row()?;
        let idx = selected_row.index() as usize;
        last_col.items.get(idx).cloned()
    }
}

impl ColumnViewInner {
    fn clear_all(&self) {
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
        self.columns.borrow_mut().push(ColumnData {
            path,
            items,
            container: col_box,
            list_box,
        });
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
        _path: PathBuf,
        items: Vec<FileItem>,
    ) -> (GtkBox, ListBox) {
        let col_box = GtkBox::new(Orientation::Vertical, 0);
        col_box.add_css_class("branches-column");
        col_box.set_size_request(240, -1);
        col_box.set_vexpand(true);

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["branches-column-list"])
            .build();

        for item in &items {
            let row = create_column_row(item);
            list_box.append(&row);
        }

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .build();

        col_box.append(&scrolled);

        // Controlador de selección de fila
        let self_for_sel = self.clone();
        let items_for_sel = items.clone();
        list_box.connect_row_selected(move |_, row_opt| {
            if let Some(row) = row_opt {
                let idx = row.index() as usize;
                if let Some(item) = items_for_sel.get(idx) {
                    self_for_sel.on_item_selected(col_idx, item);
                }
            }
        });

        // Controlador de activación (doble clic o Enter)
        let items_for_act = items.clone();
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
                        if next_lb.selected_row().is_none() {
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

            if let Ok(child_items) = load_directory(&item.path, show_hidden, sort_by, sort_order, "") {
                self.push_column(col_idx + 1, item.path.clone(), child_items);
            }
        } else {
            let preview = self.create_preview_pane(item);
            self.columns_box.append(&preview);
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
            .any(|ext| item.path.extension().map(|e| e.to_string_lossy().to_lowercase() == *ext).unwrap_or(false));

        let thumb_widget: gtk4::Widget = if is_img {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&item.path, 160, 160, true) {
                let texture = Texture::for_pixbuf(&pixbuf);
                let img = Image::from_paintable(Some(&texture));
                img.add_css_class("branches-preview-image");
                img.upcast()
            } else {
                let img = Image::from_gicon(&item.gicon);
                img.set_pixel_size(96);
                img.upcast()
            }
        } else {
            let img = Image::from_gicon(&item.gicon);
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

    fn scroll_to_end(&self) {
        let hadj = self.scrolled.hadjustment();
        glib::idle_add_local(move || {
            let max = hadj.upper() - hadj.page_size();
            if max > 0.0 {
                hadj.set_value(max);
            }
            glib::ControlFlow::Break
        });
    }
}

fn create_column_row(item: &FileItem) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.add_css_class("branches-column-row");

    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_top(3);
    hbox.set_margin_bottom(3);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

    let icon = Image::from_gicon(&item.gicon);
    icon.set_pixel_size(16);
    hbox.append(&icon);

    let label = Label::builder()
        .label(&item.name)
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    hbox.append(&label);

    if item.is_dir {
        let chevron = Image::from_icon_name("pan-end-symbolic");
        chevron.set_pixel_size(12);
        chevron.add_css_class("branches-column-chevron");
        hbox.append(&chevron);
    }

    row.set_child(Some(&hbox));
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
