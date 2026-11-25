use crate::fs_backend::{open_file, FileItem};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, GestureClick, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow,
};
use malus_de_common::config::BranchesConfig;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Anchos (px) de las columnas redimensionables: fecha, tamaño y clase.
type ColumnWidths = Rc<Cell<[i32; 3]>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortCol {
    Name,
    Date,
    Size,
    Kind,
}

/// Orden elegido por el usuario al pulsar una cabecera: (columna, ascendente).
type SortState = Rc<Cell<Option<(SortCol, bool)>>>;

fn sort_items(items: &mut [FileItem], state: Option<(SortCol, bool)>) {
    let Some((col, asc)) = state else { return };
    items.sort_by(|a, b| {
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        let ord = match col {
            SortCol::Name => b.is_dir.cmp(&a.is_dir).then_with(by_name),
            SortCol::Date => a.modified.cmp(&b.modified).then_with(by_name),
            SortCol::Size => a.size.cmp(&b.size).then_with(by_name),
            SortCol::Kind => a
                .kind_str
                .to_lowercase()
                .cmp(&b.kind_str.to_lowercase())
                .then_with(by_name),
        };
        if asc || (col == SortCol::Name && a.is_dir != b.is_dir) {
            ord
        } else {
            ord.reverse()
        }
    });
}

fn populate_list(
    list_box: &ListBox,
    items: &[FileItem],
    groups: Option<&[String]>,
    items_ref: &Rc<RefCell<Vec<FileItem>>>,
    icon_size: i32,
    widths: [i32; 3],
    on_navigate: &NavigateCallback,
) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
    let tags_map = crate::views::load_tags_map();
    let mut last_group: Option<&str> = None;
    for (idx, item) in items.iter().enumerate() {
        if let Some(grp) = groups.and_then(|g| g.get(idx)).map(|s| s.as_str()) {
            if last_group != Some(grp) {
                last_group = Some(grp);
                let hdr_lbl = Label::builder()
                    .label(grp)
                    .halign(Align::Start)
                    .css_classes(["branches-list-group-label"])
                    .build();
                let hdr_row = ListBoxRow::builder()
                    .child(&hdr_lbl)
                    .selectable(false)
                    .activatable(false)
                    .css_classes(["branches-list-group-row"])
                    .build();
                list_box.append(&hdr_row);
            }
        }
        let row = create_list_row(
            item,
            items_ref.clone(),
            icon_size,
            widths,
            &tags_map,
            on_navigate.clone(),
        );
        unsafe {
            row.set_data("list_item_idx", idx);
        }
        list_box.append(&row);
    }
}

fn row_for_item_index(list_box: &ListBox, target_idx: usize) -> Option<ListBoxRow> {
    let mut child = list_box.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<ListBoxRow>() {
            let idx = unsafe { row.data::<usize>("list_item_idx").map(|p| *p.as_ref()) };
            if idx == Some(target_idx) {
                return Some(row.clone());
            }
        }
        child = w.next_sibling();
    }
    None
}

const DEFAULT_COLUMN_WIDTHS: [i32; 3] = [170, 90, 150];
const MIN_COLUMN_WIDTH: i32 = 50;
const MAX_COLUMN_WIDTH: i32 = 600;

pub struct ListView {
    container: GtkBox,
    header: GtkBox,
    scrolled: ScrolledWindow,
    list_box: ListBox,
    items: Rc<RefCell<Vec<FileItem>>>,
    groups: Rc<RefCell<Option<Vec<String>>>>,
    current_folder: Rc<RefCell<Option<PathBuf>>>,
    pending_select_name: Rc<RefCell<Option<String>>>,
    icon_size: Rc<Cell<i32>>,
    col_widths: ColumnWidths,
    sort_state: SortState,
    on_navigate: NavigateCallback,
    #[allow(dead_code)]
    on_selection_changed: SelectionCallback,
}

impl ListView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
        let icon_size = Rc::new(Cell::new(BranchesConfig::load().clamped_list_icon_size()));
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.add_css_class("branches-list-view");

        let col_widths: ColumnWidths = Rc::new(Cell::new(DEFAULT_COLUMN_WIDTHS));

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Multiple)
            // Con activación por clic simple, GTK añade cada fila pulsada a la selección
            // múltiple; desactivarlo da el comportamiento estándar: clic reemplaza,
            // Ctrl+clic alterna y Mayús+clic extiende.
            .activate_on_single_click(false)
            .build();

        let sort_state: SortState = Rc::new(Cell::new(None));

        let sel_cb = on_selection_changed.clone();
        list_box.connect_selected_rows_changed(move |lb| {
            let count = lb.selected_rows().len();
            sel_cb(count);
        });

        let current_folder = Rc::new(RefCell::new(None::<PathBuf>));
        let items = Rc::new(RefCell::new(Vec::<FileItem>::new()));
        let groups = Rc::new(RefCell::new(None::<Vec<String>>));
        let pending_select_name = Rc::new(RefCell::new(None::<String>));

        // Franjas grises de columnas pares: capa inferior que llega hasta el final de la vista
        let bands = GtkBox::new(Orientation::Horizontal, 0);
        bands.add_css_class("branches-list-bands");
        bands.set_margin_start(4);
        bands.set_margin_end(4);
        bands.set_can_target(false);
        {
            let spacer = GtkBox::new(Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            bands.append(&spacer);
            for i in 0..3 {
                let b = GtkBox::new(Orientation::Horizontal, 0);
                if i % 2 == 0 {
                    b.add_css_class("branches-list-col-even");
                }
                bands.append(&b);
            }
            apply_widths_to_box(&bands, col_widths.get());
        }

        // Reordenar al pulsar el nombre de una columna
        let icon_size_sort = icon_size.clone();
        let on_sort: Rc<dyn Fn()> = {
            let list_box = list_box.clone();
            let items = items.clone();
            let widths = col_widths.clone();
            let state = sort_state.clone();
            let on_nav = on_navigate.clone();
            Rc::new(move || {
                let selected = crate::views::list_box_selected_items(&list_box, &items.borrow())
                    .into_iter()
                    .map(|i| i.path)
                    .collect::<Vec<_>>();
                sort_items(&mut items.borrow_mut(), state.get());
                let snapshot = items.borrow().clone();
                populate_list(
                    &list_box,
                    &snapshot,
                    None,
                    &items,
                    icon_size_sort.get(),
                    widths.get(),
                    &on_nav,
                );
                for (idx, item) in items.borrow().iter().enumerate() {
                    if selected.contains(&item.path) {
                        if let Some(row) = row_for_item_index(&list_box, idx) {
                            list_box.select_row(Some(&row));
                        }
                    }
                }
            })
        };

        // Cabecera de columnas (redimensionable y ordenable)
        let header = create_list_header(&col_widths, &list_box, &bands, &sort_state, on_sort);
        container.append(&header);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .css_classes(["branches-list-scroll"])
            .build();

        // Delegar el desplazamiento por teclado directamente al ListBox y desactivar
        // scroll-to-focus automático del Viewport para evitar saltos espurios al inicio
        // cuando cambian miniaturas, metadatos o foco de la ventana.
        list_box.set_adjustment(Some(&scrolled.vadjustment()));
        if let Some(vp) = scrolled
            .child()
            .and_then(|c| c.downcast::<gtk4::Viewport>().ok())
        {
            vp.set_scroll_to_focus(false);
        }

        let bg_click = GestureClick::new();
        bg_click.set_button(gdk::BUTTON_SECONDARY);
        let scrolled_menu = scrolled.clone();
        bg_click.connect_pressed(move |g, _n, x, y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            crate::views::show_background_context_menu(&scrolled_menu, x, y);
        });
        scrolled.add_controller(bg_click);

        let body = gtk4::Overlay::new();
        body.set_vexpand(true);
        body.set_child(Some(&bands));
        body.add_overlay(&scrolled);
        container.append(&body);

        Self {
            container,
            header,
            scrolled,
            list_box,
            items,
            groups,
            current_folder,
            pending_select_name,
            icon_size,
            col_widths,
            sort_state,
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
        let idx = unsafe { row.data::<usize>("list_item_idx").map(|p| *p.as_ref()) }
            .or_else(|| usize::try_from(row.index()).ok())?;
        let item = self.items.borrow().get(idx).cloned()?;
        Some((item, row.upcast()))
    }

    pub fn get_selected_items(&self) -> Vec<FileItem> {
        crate::views::list_box_selected_items(&self.list_box, &self.items.borrow())
    }

    pub fn select_all(&self) {
        self.list_box.select_all();
    }

    pub fn item_names_ordered(&self) -> Vec<String> {
        self.items.borrow().iter().map(|it| it.name.clone()).collect()
    }

    pub fn selected_item_name(&self) -> Option<String> {
        self.get_selected_items().into_iter().next().map(|it| it.name)
    }

    pub fn select_item_by_name(&self, name: &str) -> bool {
        let idx = self.items.borrow().iter().position(|it| it.name == name);
        let Some(idx) = idx else {
            return false;
        };
        let Some(row) = row_for_item_index(&self.list_box, idx) else {
            return false;
        };
        self.list_box.unselect_all();
        self.list_box.select_row(Some(&row));
        row.grab_focus();
        true
    }

    pub fn queue_select_name(&self, name: &str) {
        *self.pending_select_name.borrow_mut() = Some(name.to_string());
    }

    pub fn clear_sort_state(&self) {
        self.sort_state.set(None);
        update_header_titles(&self.header, None);
    }

    pub fn set_items_grouped(&self, items: &[FileItem], groups: Option<&[String]>) {
        let mut sorted = items.to_vec();
        let active_groups: Option<Vec<String>> = if self.sort_state.get().is_some() {
            sort_items(&mut sorted, self.sort_state.get());
            None
        } else {
            groups.map(|g| g.to_vec())
        };
        *self.items.borrow_mut() = sorted;
        *self.groups.borrow_mut() = active_groups.clone();
        let items = self.items.borrow().clone();
        let items = &items[..];

        populate_list(
            &self.list_box,
            items,
            active_groups.as_deref(),
            &self.items,
            self.icon_size.get(),
            self.col_widths.get(),
            &self.on_navigate,
        );

        if let Some(target_name) = self.pending_select_name.borrow_mut().take() {
            if let Some(idx) = items.iter().position(|it| it.name == target_name) {
                if let Some(row) = row_for_item_index(&self.list_box, idx) {
                    self.list_box.select_row(Some(&row));
                    row.grab_focus();
                }
            }
        }
    }

    /// Sincroniza cambios provenientes del monitor del sistema de archivos preservando
    /// la selección activa y la posición de desplazamiento vertical.
    pub fn sync_items_incremental(&self, items: &[FileItem], groups: Option<&[String]>) {
        let mut sorted = items.to_vec();
        let active_groups: Option<Vec<String>> = if self.sort_state.get().is_some() {
            sort_items(&mut sorted, self.sort_state.get());
            None
        } else {
            groups.map(|g| g.to_vec())
        };

        // Ruta rápida sin recrear filas: si las rutas y grupos coinciden exactamente,
        // actualizar solo las celdas de metadatos (fecha, tamaño, clase y miniatura) in-place.
        let same_structure = {
            let prev_items = self.items.borrow();
            let prev_groups = self.groups.borrow();
            rows_match_in_place(&prev_items, &sorted, prev_groups.as_deref(), active_groups.as_deref())
        };
        if same_structure {
            let icon_size = self.icon_size.get();
            let prev_items = self.items.borrow().clone();
            *self.items.borrow_mut() = sorted.clone();
            let mut child = self.list_box.first_child();
            while let Some(widget) = child {
                if let Some(row) = widget.downcast_ref::<ListBoxRow>() {
                    if let Some(idx) =
                        unsafe { row.data::<usize>("list_item_idx").map(|p| *p.as_ref()) }
                    {
                        if let (Some(old_it), Some(new_it)) = (prev_items.get(idx), sorted.get(idx))
                        {
                            if old_it.modified != new_it.modified
                                || old_it.size != new_it.size
                                || old_it.modified_str != new_it.modified_str
                                || old_it.size_str != new_it.size_str
                                || old_it.kind_str != new_it.kind_str
                            {
                                update_list_row_metadata(row, new_it, icon_size);
                            }
                        }
                    }
                }
                child = widget.next_sibling();
            }
            return;
        }

        // Cambio estructural (archivo creado/eliminado/reordenado): preservar selección,
        // foco y posición vertical del scroll antes de reconstruir las filas.
        let selected_paths: std::collections::HashSet<PathBuf> = self
            .get_selected_items()
            .into_iter()
            .map(|it| it.path)
            .collect();
        let focused_path: Option<PathBuf> = self
            .list_box
            .focus_child()
            .and_then(|w| w.downcast::<ListBoxRow>().ok())
            .and_then(|row| {
                let idx = unsafe { row.data::<usize>("list_item_idx").map(|p| *p.as_ref()) }?;
                self.items.borrow().get(idx).map(|it| it.path.clone())
            });
        let saved_scroll_y = self.scrolled.vadjustment().value();

        *self.items.borrow_mut() = sorted;
        *self.groups.borrow_mut() = active_groups.clone();
        let snapshot = self.items.borrow().clone();

        populate_list(
            &self.list_box,
            &snapshot,
            active_groups.as_deref(),
            &self.items,
            self.icon_size.get(),
            self.col_widths.get(),
            &self.on_navigate,
        );

        let pending_name = self.pending_select_name.borrow_mut().take();
        if let Some(target_name) = pending_name {
            if let Some(idx) = snapshot.iter().position(|it| it.name == target_name) {
                if let Some(row) = row_for_item_index(&self.list_box, idx) {
                    self.list_box.select_row(Some(&row));
                    row.grab_focus();
                }
            }
        } else if !selected_paths.is_empty() {
            for (idx, item) in snapshot.iter().enumerate() {
                if selected_paths.contains(&item.path) {
                    if let Some(row) = row_for_item_index(&self.list_box, idx) {
                        self.list_box.select_row(Some(&row));
                        if focused_path.as_ref() == Some(&item.path) {
                            row.grab_focus();
                        }
                    }
                }
            }
        }

        // Restaurar el desplazamiento vertical tras el nuevo layout del ListBox
        let scrolled_c = self.scrolled.clone();
        glib::idle_add_local_once(move || {
            let vadj = scrolled_c.vadjustment();
            let max_y = (vadj.upper() - vadj.page_size()).max(vadj.lower());
            vadj.set_value(saved_scroll_y.clamp(vadj.lower(), max_y));
        });
    }

    pub fn refresh_cut_states(&self) {
        let items = self.items.borrow();
        let mut child = self.list_box.first_child();
        while let Some(widget) = child {
            if let Ok(row) = widget.clone().downcast::<ListBoxRow>() {
                if let Some(idx) = unsafe { row.data::<usize>("list_item_idx").map(|p| *p.as_ref()) }
                {
                    if let Some(item) = items.get(idx) {
                        if let Some(hbox) = row.child() {
                            crate::views::sync_cut_css_class(&hbox, &item.path);
                        }
                    }
                }
            }
            child = widget.next_sibling();
        }
    }
}

/// Comprueba si dos listas de ítems y grupos tienen exactamente la misma secuencia de rutas,
/// permitiendo actualizar metadatos in-place sin recrear ninguna fila del `ListBox`.
fn rows_match_in_place(
    prev_items: &[FileItem],
    next_items: &[FileItem],
    prev_groups: Option<&[String]>,
    next_groups: Option<&[String]>,
) -> bool {
    prev_items.len() == next_items.len()
        && prev_groups == next_groups
        && prev_items
            .iter()
            .zip(next_items.iter())
            .all(|(a, b)| a.path == b.path && a.name == b.name && a.is_dir == b.is_dir)
}

/// Actualiza fecha, tamaño, clase y miniatura de una fila existente sin reconstruir el `ListBoxRow`.
fn update_list_row_metadata(row: &ListBoxRow, item: &FileItem, icon_size: i32) {
    let Some(hbox) = row.child().and_then(|c| c.downcast::<GtkBox>().ok()) else {
        return;
    };
    let mut child = hbox.first_child();
    let mut col_idx = 0usize;
    while let Some(w) = child {
        match col_idx {
            0 => {
                if let Some(name_box) = w.downcast_ref::<GtkBox>() {
                    if let Some(img) = name_box
                        .first_child()
                        .and_then(|c| c.downcast::<gtk4::Image>().ok())
                    {
                        crate::views::update_item_image_thumbnail(
                            &img,
                            item,
                            icon_size,
                            Some("branches-row-thumb"),
                        );
                    }
                }
            }
            1 => {
                if let Some(lbl) = w.downcast_ref::<Label>() {
                    lbl.set_label(&item.modified_str);
                    lbl.set_tooltip_text(Some(&item.modified_str));
                }
            }
            2 => {
                if let Some(lbl) = w.downcast_ref::<Label>() {
                    lbl.set_label(&item.size_str);
                    lbl.set_tooltip_text(Some(&item.size_str));
                }
            }
            3 => {
                if let Some(lbl) = w.downcast_ref::<Label>() {
                    lbl.set_label(&item.kind_str);
                    lbl.set_tooltip_text(Some(&item.kind_str));
                }
            }
            _ => break,
        }
        child = w.next_sibling();
        col_idx += 1;
    }
}

/// Aplica `widths` a las columnas 2..4 de un `GtkBox` de fila o cabecera.
fn apply_widths_to_box(hbox: &GtkBox, widths: [i32; 3]) {
    let mut child = hbox.first_child();
    let mut idx = 0usize;
    while let Some(w) = child {
        if (1..=3).contains(&idx) {
            w.set_size_request(widths[idx - 1], -1);
        }
        child = w.next_sibling();
        idx += 1;
    }
}

fn apply_column_widths(header: &GtkBox, list_box: &ListBox, bands: &GtkBox, widths: [i32; 3]) {
    apply_widths_to_box(header, widths);
    apply_widths_to_box(bands, widths);
    let mut child = list_box.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<ListBoxRow>() {
            if let Some(hbox) = row.child().and_then(|c| c.downcast::<GtkBox>().ok()) {
                apply_widths_to_box(&hbox, widths);
            }
        }
        child = w.next_sibling();
    }
}

/// Crea una celda de texto de ancho fijo, alineada y truncada con `…` en el centro.
fn create_cell_label(text: &str, align_end: bool) -> Label {
    Label::builder()
        .label(text)
        .halign(Align::Fill)
        .xalign(if align_end { 1.0 } else { 0.0 })
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .width_chars(1)
        .max_width_chars(1)
        .tooltip_text(text)
        .css_classes(["branches-list-cell"])
        .build()
}

const HEADER_TITLES: [(&str, SortCol); 4] = [
    ("Nombre", SortCol::Name),
    ("Fecha de modificación", SortCol::Date),
    ("Tamaño", SortCol::Size),
    ("Clase", SortCol::Kind),
];

fn header_label(hbox: &GtkBox, idx: usize) -> Option<Label> {
    let mut child = hbox.first_child();
    for _ in 0..idx {
        child = child?.next_sibling();
    }
    let w = child?;
    if idx == 0 {
        w.downcast::<Label>().ok()
    } else {
        w.downcast::<gtk4::Overlay>()
            .ok()?
            .child()?
            .downcast::<Label>()
            .ok()
    }
}

fn update_header_titles(hbox: &GtkBox, state: Option<(SortCol, bool)>) {
    for (idx, (title, col)) in HEADER_TITLES.iter().enumerate() {
        if let Some(label) = header_label(hbox, idx) {
            let arrow = match state {
                Some((c, asc)) if c == *col => {
                    if asc {
                        " ▲"
                    } else {
                        " ▼"
                    }
                }
                _ => "",
            };
            label.set_label(&format!("{title}{arrow}"));
        }
    }
}

fn create_list_header(
    widths: &ColumnWidths,
    list_box: &ListBox,
    bands: &GtkBox,
    sort_state: &SortState,
    on_sort: Rc<dyn Fn()>,
) -> GtkBox {
    let hbox = GtkBox::new(Orientation::Horizontal, 0);
    hbox.add_css_class("branches-list-header");

    let col_name = Label::builder()
        .label(HEADER_TITLES[0].0)
        .halign(Align::Start)
        .hexpand(true)
        .css_classes(["branches-list-cell"])
        .build();
    hbox.append(&col_name);

    let mut click_targets: Vec<(gtk4::Widget, SortCol)> =
        vec![(col_name.clone().upcast(), SortCol::Name)];

    for i in 0..3 {
        let (title, col) = HEADER_TITLES[i + 1];
        let overlay = gtk4::Overlay::new();
        // Columnas pares (fecha y clase) con fondo gris muy claro
        if i % 2 == 0 {
            overlay.add_css_class("branches-list-col-even");
        }
        overlay.set_child(Some(&create_cell_label(title, col == SortCol::Size)));

        // Asa de redimensionado en el borde izquierdo de la columna
        let handle = GtkBox::new(Orientation::Horizontal, 0);
        handle.set_size_request(8, -1);
        handle.set_halign(Align::Start);
        handle.set_vexpand(true);
        handle.add_css_class("branches-list-col-handle");
        handle.set_cursor_from_name(Some("col-resize"));
        unsafe {
            handle.set_data("col_idx", i);
        }
        overlay.add_overlay(&handle);

        click_targets.push((overlay.clone().upcast(), col));
        hbox.append(&overlay);
    }

    // Un solo gesto de arrastre sobre la cabecera (que no se mueve): los desplazamientos
    // son estables y el contenido no "vibra" al redimensionar.
    {
        let drag = gtk4::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        drag.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let active: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let start_width = Rc::new(Cell::new(0));
        let weak_header = hbox.downgrade();
        {
            let active = active.clone();
            let start_width = start_width.clone();
            let widths = widths.clone();
            let weak_header = weak_header.clone();
            drag.connect_drag_begin(move |g, x, y| {
                let idx = weak_header
                    .upgrade()
                    .and_then(|h| h.pick(x, y, gtk4::PickFlags::DEFAULT))
                    .filter(|w| w.has_css_class("branches-list-col-handle"))
                    .and_then(|w| unsafe { w.data::<usize>("col_idx").map(|p| *p.as_ref()) });
                match idx {
                    Some(i) => {
                        active.set(Some(i));
                        start_width.set(widths.get()[i]);
                        g.set_state(gtk4::EventSequenceState::Claimed);
                    }
                    None => {
                        active.set(None);
                        g.set_state(gtk4::EventSequenceState::Denied);
                    }
                }
            });
        }
        {
            let active = active.clone();
            let widths = widths.clone();
            let list_box = list_box.clone();
            let bands = bands.clone();
            drag.connect_drag_update(move |_, dx, _| {
                let Some(i) = active.get() else { return };
                let Some(header) = weak_header.upgrade() else {
                    return;
                };
                let new_w = (start_width.get() - dx.round() as i32)
                    .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
                let mut w = widths.get();
                if w[i] != new_w {
                    w[i] = new_w;
                    widths.set(w);
                    apply_column_widths(&header, &list_box, &bands, w);
                }
            });
        }
        drag.connect_drag_end(move |_, _, _| active.set(None));
        hbox.add_controller(drag);
    }

    // Clic en el nombre de la columna: ordenar (ascendente → descendente)
    for (widget, col) in click_targets {
        let click = GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let state = sort_state.clone();
        let weak_header = hbox.downgrade();
        let on_sort = on_sort.clone();
        click.connect_released(move |_, n_press, _, _| {
            if n_press != 1 {
                return;
            }
            let next = match state.get() {
                Some((c, asc)) if c == col => (col, !asc),
                _ => (col, true),
            };
            state.set(Some(next));
            if let Some(h) = weak_header.upgrade() {
                update_header_titles(&h, Some(next));
            }
            on_sort();
        });
        widget.add_controller(click);
    }

    apply_widths_to_box(&hbox, widths.get());
    hbox
}

fn create_list_row(
    item: &FileItem,
    items_ref: Rc<RefCell<Vec<FileItem>>>,
    icon_size: i32,
    widths: [i32; 3],
    tags_map: &std::collections::HashMap<PathBuf, Vec<(String, String)>>,
    on_navigate: NavigateCallback,
) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 0);
    hbox.add_css_class("branches-list-row");
    if item.is_hidden {
        hbox.add_css_class("branches-hidden-item");
    }
    if crate::operations::is_path_cut(&item.path) {
        hbox.add_css_class("branches-cut-item");
    }

    // Columna 1: Icono + Nombre
    let name_box = GtkBox::new(Orientation::Horizontal, 8);
    name_box.set_hexpand(true);
    name_box.set_halign(Align::Fill);
    name_box.add_css_class("branches-list-cell");

    let icon = crate::views::create_item_image(item, icon_size, Some("branches-row-thumb"));
    icon.set_size_request(icon_size, icon_size);
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    name_box.append(&icon);

    let label_name = Label::builder()
        .label(&item.name)
        .halign(Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .css_classes(["branches-list-name-label"])
        .build();
    name_box.append(&label_name);
    if let Some(tag_dots) = crate::views::create_item_tag_dots(&item.path, tags_map) {
        name_box.append(&tag_dots);
    }

    // Columnas 2-4: fecha, tamaño y clase (ancho fijo compartido con la cabecera)
    let label_date = create_cell_label(&item.modified_str, false);
    let label_size = create_cell_label(&item.size_str, true);
    let label_kind = create_cell_label(&item.kind_str, false);

    hbox.append(&name_box);
    hbox.append(&label_date);
    hbox.append(&label_size);
    hbox.append(&label_kind);
    apply_widths_to_box(&hbox, widths);

    let row = ListBoxRow::builder().child(&hbox).build();

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
            let is_single_selected = row_for_menu
                .parent()
                .and_then(|p| p.downcast::<ListBox>().ok())
                .is_some_and(|lb| lb.selected_rows().len() == 1);
            if n_press == 1
                && row_for_menu.is_selected()
                && is_single_selected
                && !crate::views::is_multi_select_modifier(g.current_event_state())
            {
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
                                &row_ren, path_ren, is_dir, None, on_renamed,
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
                &path,
                is_dir,
                Some((x, y)),
                Some(&on_nav_click),
                None,
            );
        }
    });

    row.add_controller(gesture);

    // Arrastrar archivo (Drag & Drop con soporte múltiple estilo macOS Finder)
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
    let path_for_drag = item.path.clone();
    let row_for_drag = row.clone();
    drag_source.connect_prepare(move |_, _x, _y| {
        Some(crate::views::prepare_list_row_drag(
            &row_for_drag,
            &path_for_drag,
            &items_ref.borrow(),
        ))
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
        folder_drop.connect_motion(move |_, _, _| gdk::DragAction::MOVE);

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
                                if file
                                    .move_(
                                        &dest_file,
                                        gio::FileCopyFlags::NONE,
                                        gio::Cancellable::NONE,
                                        None,
                                    )
                                    .is_ok()
                                    || std::fs::rename(&src, &target).is_ok()
                                {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_item(name: &str, size: u64) -> FileItem {
        FileItem {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir: false,
            is_hidden: false,
            is_symlink: false,
            size,
            size_str: format!("{size} B"),
            modified: None,
            accessed: None,
            added: None,
            created: None,
            modified_str: "--".to_string(),
            kind_str: "Archivo".to_string(),
            gicon: gtk4::gio::ThemedIcon::new("text-x-generic").upcast(),
        }
    }

    #[test]
    fn rows_match_in_place_allows_metadata_updates_without_rebuilding_rows() {
        let a = vec![mk_item("a.txt", 10), mk_item("b.txt", 20)];
        let b = vec![mk_item("a.txt", 10), mk_item("b.txt", 999)];
        let c = vec![mk_item("b.txt", 20), mk_item("a.txt", 10)];
        let d = vec![mk_item("a.txt", 10)];

        assert!(rows_match_in_place(&a, &b, None, None));
        assert!(!rows_match_in_place(&a, &c, None, None));
        assert!(!rows_match_in_place(&a, &d, None, None));
        let g1 = vec!["A".to_string(), "B".to_string()];
        let g2 = vec!["A".to_string(), "C".to_string()];
        assert!(rows_match_in_place(&a, &b, Some(&g1), Some(&g1)));
        assert!(!rows_match_in_place(&a, &b, Some(&g1), Some(&g2)));
    }
}
