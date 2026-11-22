use crate::folder_pane::{FolderPane, PaneId};
use crate::fs_backend::get_free_space;
use crate::job_ui::JobUiController;
use crate::jobs::{ArchiveFormat, JobRequest, UndoEntry};
use crate::path_bar::PathBar;
use crate::quick_look::QuickLook;
use crate::sidebar::Sidebar;
use crate::status_bar::StatusBar;
use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, DrawingArea,
    EventControllerFocus, EventControllerKey, GestureClick, Label, MenuButton, Orientation,
    Overlay, PackType, Paned, SearchEntry, Stack, StackTransitionType, ToggleButton,
    WindowControls, WindowHandle,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

const BRANCHES_SEARCH_HEIGHT: i32 = 32;
const BRANCHES_SEARCH_COLLAPSED_WIDTH: i32 = 32;
const BRANCHES_SEARCH_EXPANDED_WIDTH: i32 = 196;
const BRANCHES_SEARCH_ANIM_MS: f64 = 180.0;
const BRANCHES_SEARCH_SQUIRCLE_RADIUS: f64 = 16.0;
const SEARCH_SQUIRCLE_EXPONENT: f64 = 4.2;
const SEARCH_SQUIRCLE_SEGMENTS: usize = 20;

fn squircle_path(context: &gtk4::cairo::Context, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(1.0);
    let power = 2.0 / SEARCH_SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);

    for step in 1..=SEARCH_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SEARCH_SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);

    for step in 1..=SEARCH_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SEARCH_SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);

    for step in 1..=SEARCH_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SEARCH_SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);

    for step in 1..=SEARCH_SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SEARCH_SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }
    context.close_path();
}

/// Envuelve el contenido de un botón de menú en un Overlay con DrawingArea Cairo Squircle azul semitransparente.
pub(crate) fn attach_squircle_menu_hover(btn: &Button) {
    let Some(inner) = btn.child() else {
        return;
    };
    btn.add_css_class("malus-squircle-menu-item");
    btn.set_child(gtk4::Widget::NONE);
    inner.set_margin_top(3);
    inner.set_margin_bottom(3);
    inner.set_margin_start(8);
    inner.set_margin_end(8);

    let bg = gtk4::DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(false);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let Some(b) = btn_weak.upgrade() else {
            return;
        };
        let flags = b.state_flags();
        if flags.contains(gtk4::StateFlags::INSENSITIVE) {
            return;
        }
        let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
        let active = flags.contains(gtk4::StateFlags::ACTIVE);
        if !hovered && !active {
            return;
        }
        let _ = cr.save();
        squircle_path(cr, width, height, 14.0);
        let (red, green, blue) = malus_de_common::config::load_accent_color();
        if active {
            cr.set_source_rgba(red * 0.78, green * 0.78, blue * 0.78, 0.86);
        } else {
            cr.set_source_rgba(red, green, blue, 0.74);
        }
        let _ = cr.fill();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = Overlay::builder()
        .hexpand(true)
        .vexpand(false)
        .valign(Align::Center)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner);
    overlay.set_measure_overlay(&inner, true);
    btn.set_vexpand(false);
    btn.set_valign(Align::Start);
    btn.set_child(Some(&overlay));
}

/// Envuelve un `SearchEntry` en la misma cápsula squircle en Cairo que el filtro de Ajustes.
fn create_squircle_search_shell(entry: &SearchEntry) -> Overlay {
    let shell = Overlay::builder()
        .height_request(BRANCHES_SEARCH_HEIGHT)
        .valign(Align::Center)
        .vexpand(false)
        .css_classes(["branches-search-shell"])
        .build();

    let bg = DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .build();

    let focus_ctrl = EventControllerFocus::new();
    let focus_state = focus_ctrl.clone();
    let bg_draw = bg.clone();
    focus_ctrl.connect_enter(move |_| bg_draw.queue_draw());
    let bg_draw_leave = bg.clone();
    focus_ctrl.connect_leave(move |_| bg_draw_leave.queue_draw());
    entry.add_controller(focus_ctrl);

    let bg_weak = bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    bg.set_draw_func(move |_, cr, w, h| {
        let w = w as f64;
        let h = h as f64;
        if w <= 2.0 || h <= 2.0 {
            return;
        }
        let is_dark = libadwaita::StyleManager::default().is_dark();
        let focused = focus_state.contains_focus();

        if cr.save().is_ok() {
            squircle_path(cr, w, h, BRANCHES_SEARCH_SQUIRCLE_RADIUS);
            if is_dark {
                cr.set_source_rgba(1.0, 1.0, 1.0, if focused { 0.11 } else { 0.08 });
            } else {
                cr.set_source_rgba(0.0, 0.0, 0.0, if focused { 0.075 } else { 0.055 });
            }
            let _ = cr.fill();
            let _ = cr.restore();
        }

        if cr.save().is_ok() {
            let inset = 0.75;
            cr.translate(inset, inset);
            squircle_path(
                cr,
                (w - inset * 2.0).max(1.0),
                (h - inset * 2.0).max(1.0),
                (BRANCHES_SEARCH_SQUIRCLE_RADIUS - inset).max(2.0),
            );
            if focused {
                let (red, green, blue) = malus_de_common::config::load_accent_color();
                if is_dark {
                    cr.set_source_rgba(red, green, blue, 0.85);
                } else {
                    cr.set_source_rgba(red, green, blue, 0.80);
                }
                cr.set_line_width(1.5);
            } else {
                if is_dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.15);
                } else {
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.13);
                }
                cr.set_line_width(1.0);
            }
            let _ = cr.stroke();
            let _ = cr.restore();
        }
    });

    shell.set_overflow(gtk4::Overflow::Hidden);
    shell.set_halign(Align::End);
    shell.set_child(Some(&bg));
    shell.add_overlay(entry);
    shell.set_measure_overlay(entry, false);
    shell
}

struct BranchesTab {
    id: u64,
    primary_pane: Rc<FolderPane>,
    secondary_pane: Rc<FolderPane>,
    split_paned: Paned,
    active_pane_id: Rc<RefCell<PaneId>>,
    split_view_active: Rc<RefCell<bool>>,
    tab_widget: GtkBox,
    tab_title_label: Label,
}

thread_local! {
    static OPEN_BRANCHES_WINDOWS: RefCell<Vec<Weak<BranchesWindow>>> =
        const { RefCell::new(Vec::new()) };
}

pub fn reload_all_open_windows() {
    let active: Vec<Rc<BranchesWindow>> = OPEN_BRANCHES_WINDOWS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|w| w.strong_count() > 0);
        list.iter().filter_map(Weak::upgrade).collect()
    });
    for w in active {
        w.reload_all_panes();
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BackgroundMenuState {
    pub view_mode: &'static str,
    pub sort_by: crate::state::SortBy,
    pub use_groups: bool,
    pub search_criteria_visible: bool,
}

pub fn background_menu_state_for_widget(
    widget: &impl gtk4::prelude::IsA<gtk4::Widget>,
) -> Option<BackgroundMenuState> {
    let root_widget = widget.root()?.upcast::<gtk4::Widget>();
    let bw = OPEN_BRANCHES_WINDOWS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|w| w.strong_count() > 0);
        list.iter()
            .filter_map(Weak::upgrade)
            .find(|w| w.window.upcast_ref::<gtk4::Widget>() == &root_widget)
    })?;
    let pane = bw.active_pane();
    let view_mode = match pane.view_mode().as_str() {
        "list" => "list",
        "columns" => "columns",
        "gallery" => "gallery",
        _ => "icons",
    };
    let (sort_by, _) = pane.sort_settings();
    Some(BackgroundMenuState {
        view_mode,
        sort_by,
        use_groups: pane.use_groups(),
        search_criteria_visible: bw.search_criteria_bar.is_visible(),
    })
}

pub struct BranchesWindow {
    window: ApplicationWindow,
    root_overlay: Overlay,
    sidebar: Rc<Sidebar>,
    path_bar: Rc<PathBar>,
    status_bar: Rc<StatusBar>,
    tabs: Rc<RefCell<Vec<BranchesTab>>>,
    active_tab_id: Rc<RefCell<u64>>,
    next_tab_id: Rc<Cell<u64>>,
    win_weak_holder: Rc<RefCell<Weak<BranchesWindow>>>,
    tab_bar_box: GtkBox,
    tabs_strip: GtkBox,
    trash_banner_box: GtkBox,
    search_criteria_bar: GtkBox,
    criteria_kind_btns: Rc<Vec<(String, ToggleButton)>>,
    criteria_recursive_btn: ToggleButton,
    view_options_window: Rc<RefCell<Option<gtk4::Window>>>,
    tab_stack: Stack,
    split_btn: Button,
    back_btn: Button,
    forward_btn: Button,
    title_label: Label,
    btn_icon_view: ToggleButton,
    btn_list_view: ToggleButton,
    btn_col_view: ToggleButton,
    btn_gallery_view: ToggleButton,
    quick_look: Rc<QuickLook>,
    job_ui: Rc<JobUiController>,
    sort_by_action: gio::SimpleAction,
    sort_order_action: gio::SimpleAction,
    search_recursive_action: gio::SimpleAction,
    search_shell: Overlay,
    search_entry: SearchEntry,
    search_toggle_btn: Button,
    search_expanded: Rc<Cell<bool>>,
    search_anim_progress: Rc<Cell<f64>>,
    search_anim_gen: Rc<Cell<u64>>,
    syncing_ui: Rc<Cell<bool>>,
    _config_monitor: Option<gio::FileMonitor>,
}

impl BranchesWindow {
    pub fn new(app: &Application, initial_path: PathBuf) -> Rc<Self> {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Branches")
            .default_width(1080)
            .default_height(680)
            .decorated(true)
            .css_classes(["branches-window"])
            .build();

        // Ocultar barra CSD tradicional para usar el layout auténtico de macOS Finder
        let dummy_titlebar = GtkBox::builder().visible(false).build();
        window.set_titlebar(Some(&dummy_titlebar));

        let win_weak_holder: Rc<RefCell<Weak<BranchesWindow>>> = Rc::new(RefCell::new(Weak::new()));
        let job_ui = JobUiController::new(
            window.clone(),
            {
                let w_ref = win_weak_holder.clone();
                Rc::new(move |select_name, clear_cut| {
                    if let Some(w) = w_ref.borrow().upgrade() {
                        if clear_cut {
                            crate::operations::clear_clipboard();
                        }
                        if let Some(name) = select_name {
                            w.active_pane().queue_select_name_active(&name);
                        }
                        w.reload_all_panes();
                        w.refresh_all_cut_states();
                    }
                })
            },
            {
                let w_for_err = window.clone();
                Rc::new(move |err| {
                    tracing::error!("Operación de archivo fallida: {}", err);
                    gtk4::AlertDialog::builder()
                        .message(err)
                        .build()
                        .show(Some(&w_for_err));
                })
            },
        );

        // 1. Botones de navegación [ < | > ] agrupados en cápsula estilo Finder
        let nav_box = GtkBox::new(Orientation::Horizontal, 0);
        nav_box.set_valign(Align::Center);
        nav_box.add_css_class("linked");
        nav_box.add_css_class("branches-toolbar-pill");
        nav_box.add_css_class("branches-nav-group");

        let back_btn = Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Atrás (Alt+Izquierda)")
            .css_classes(["branches-nav-btn"])
            .sensitive(false)
            .build();

        let forward_btn = Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text("Adelante (Alt+Derecha)")
            .css_classes(["branches-nav-btn"])
            .sensitive(false)
            .build();

        let nav_divider = GtkBox::new(Orientation::Vertical, 0);
        nav_divider.add_css_class("branches-pill-divider");
        nav_divider.set_valign(Align::Center);

        nav_box.append(&back_btn);
        nav_box.append(&nav_divider);
        nav_box.append(&forward_btn);

        // 2. Título de directorio alineado a la izquierda junto a la navegación
        let title_box = GtkBox::new(Orientation::Horizontal, 4);
        title_box.set_valign(Align::Center);
        title_box.add_css_class("branches-header-title-box");

        let title_label = Label::builder()
            .label("Branches")
            .css_classes(["branches-header-title"])
            .valign(Align::Center)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(26)
            .build();
        title_box.append(&title_label);

        // Contenedor izquierdo del header (Navegación + Título)
        let left_header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .valign(Align::Center)
            .halign(Align::Start)
            .build();
        left_header_box.append(&nav_box);
        left_header_box.append(&title_box);

        // Espaciador flexible central (macOS Finder deja el centro limpio)
        let center_spacer = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .hexpand(true)
            .build();

        // 3. Selector segmentado de 4 modos de vista en cápsula estilo Finder
        let view_box = GtkBox::new(Orientation::Horizontal, 0);
        view_box.add_css_class("linked");
        view_box.add_css_class("branches-toolbar-pill");
        view_box.add_css_class("branches-view-toggle");
        view_box.set_valign(Align::Center);

        let btn_icon_view = ToggleButton::builder()
            .icon_name("view-grid-symbolic")
            .tooltip_text("Vista de Iconos (Ctrl+1)")
            .active(true)
            .build();

        let btn_list_view = ToggleButton::builder()
            .icon_name("view-list-symbolic")
            .tooltip_text("Vista de Lista (Ctrl+2)")
            .group(&btn_icon_view)
            .build();

        let btn_col_view = ToggleButton::builder()
            .icon_name("view-dual-symbolic")
            .tooltip_text("Vista de Columnas Miller (Ctrl+3)")
            .group(&btn_icon_view)
            .build();

        let btn_gallery_view = ToggleButton::builder()
            .icon_name("view-paged-symbolic")
            .tooltip_text("Vista de Galería (Ctrl+4)")
            .group(&btn_icon_view)
            .build();

        view_box.append(&btn_icon_view);
        view_box.append(&btn_list_view);
        view_box.append(&btn_col_view);
        view_box.append(&btn_gallery_view);

        // 3b. Cápsula de ordenación (Ordenar por / Dirección)
        let es_ui = malus_de_common::is_spanish_locale();
        let sort_pill_box = GtkBox::new(Orientation::Horizontal, 0);
        sort_pill_box.add_css_class("branches-toolbar-pill");
        sort_pill_box.add_css_class("branches-toolbar-solo-pill");
        sort_pill_box.set_valign(Align::Center);

        let sort_menu_btn = MenuButton::builder()
            .icon_name("view-sort-ascending-symbolic")
            .tooltip_text(if es_ui {
                "Ordenar elementos"
            } else {
                "Sort items"
            })
            .valign(Align::Center)
            .css_classes(["branches-tool-btn"])
            .build();

        let sort_model = gio::Menu::new();
        let sort_by_sec = gio::Menu::new();
        for (id, lbl_es, lbl_en) in [
            ("name", "Nombre", "Name"),
            ("kind", "Clase", "Kind"),
            ("date_last_opened", "Fecha de última apertura", "Date Last Opened"),
            ("date_added", "Fecha en que se agregó", "Date Added"),
            ("date", "Fecha de modificación", "Date Modified"),
            ("date_created", "Fecha de creación", "Date Created"),
            ("size", "Tamaño", "Size"),
            ("tags", "Etiquetas", "Tags"),
        ] {
            let item = gio::MenuItem::new(Some(if es_ui { lbl_es } else { lbl_en }), None);
            item.set_action_and_target_value(Some("win.sort_by"), Some(&id.to_variant()));
            sort_by_sec.append_item(&item);
        }
        sort_model.append_section(
            Some(if es_ui { "Ordenar por" } else { "Sort By" }),
            &sort_by_sec,
        );

        let sort_dir_sec = gio::Menu::new();
        for (id, lbl_es, lbl_en) in [
            ("asc", "Ascendente", "Ascending"),
            ("desc", "Descendente", "Descending"),
        ] {
            let item = gio::MenuItem::new(Some(if es_ui { lbl_es } else { lbl_en }), None);
            item.set_action_and_target_value(Some("win.sort_order"), Some(&id.to_variant()));
            sort_dir_sec.append_item(&item);
        }
        sort_model.append_section(
            Some(if es_ui { "Dirección" } else { "Direction" }),
            &sort_dir_sec,
        );
        sort_menu_btn.set_menu_model(Some(&sort_model));
        if let Some(pop) = sort_menu_btn.popover() {
            pop.set_has_arrow(false);
            pop.add_css_class("malus-popover-menu");
        }
        sort_pill_box.append(&sort_menu_btn);

        // 4. Botón de división de pantalla (Split View Dual Pane) en su propia cápsula
        let split_pill_box = GtkBox::new(Orientation::Horizontal, 0);
        split_pill_box.add_css_class("branches-toolbar-pill");
        split_pill_box.add_css_class("branches-toolbar-solo-pill");
        split_pill_box.set_valign(Align::Center);

        let split_btn = Button::builder()
            .icon_name("sidebar-show-right-symbolic")
            .tooltip_text("Dividir pantalla (Dual Pane - F3)")
            .valign(Align::Center)
            .css_classes(["branches-tool-btn", "branches-split-btn"])
            .build();
        split_pill_box.append(&split_btn);

        // 5-7. Cápsula agrupada de Compartir, Información / Quick Look y Acciones (...)
        let actions_pill_box = GtkBox::new(Orientation::Horizontal, 0);
        actions_pill_box.add_css_class("linked");
        actions_pill_box.add_css_class("branches-toolbar-pill");
        actions_pill_box.add_css_class("branches-action-pill");
        actions_pill_box.set_valign(Align::Center);

        // 5. Botón de Compartir (Share)
        let share_menu_btn = MenuButton::builder()
            .icon_name("document-send-symbolic")
            .tooltip_text("Compartir...")
            .valign(Align::Center)
            .css_classes(["branches-tool-btn"])
            .build();

        let make_action_menu_btn =
            |label: &str, action: &str, target: Option<glib::Variant>, pop: &gtk4::Popover| {
                let btn = Button::builder()
                    .label(label)
                    .has_frame(false)
                    .css_classes(["popover-menu-item"])
                    .build();
                if let Some(lbl) = btn.child().and_then(|c| c.downcast::<Label>().ok()) {
                    lbl.set_xalign(0.0);
                }
                btn.set_action_name(Some(action));
                if let Some(val) = target {
                    btn.set_action_target_value(Some(&val));
                }
                attach_squircle_menu_hover(&btn);
                let p = pop.clone();
                btn.connect_clicked(move |_| p.popdown());
                btn
            };

        let share_popover = gtk4::Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        let share_box = GtkBox::new(Orientation::Vertical, 1);
        for (lbl, act) in [
            ("Copiar ruta al portapapeles", "win.copy_path"),
            ("Malus Link / Enviar a dispositivo…", "win.share_malus_link"),
            ("KDE Connect / Enviar a dispositivo", "win.share_kdeconnect"),
            ("Enviar por correo electrónico", "win.share_email"),
        ] {
            share_box.append(&make_action_menu_btn(lbl, act, None, &share_popover));
        }
        share_popover.set_child(Some(&share_box));
        share_menu_btn.set_popover(Some(&share_popover));

        // 6. Botón de Información (Inspector "Obtener información")
        let info_btn = Button::builder()
            .icon_name("dialog-information-symbolic")
            .tooltip_text(if es_ui {
                "Obtener información (Ctrl+I)"
            } else {
                "Get Info (Ctrl+I)"
            })
            .valign(Align::Center)
            .css_classes(["branches-tool-btn"])
            .build();

        // 7. Menú de Acciones y Opciones (...)
        let action_menu_btn = MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Acciones y opciones")
            .valign(Align::Center)
            .css_classes(["branches-tool-btn"])
            .build();

        let action_popover = gtk4::Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        let action_box = GtkBox::new(Orientation::Vertical, 1);
        for (lbl, act) in [
            ("Nueva Pestaña", "win.new_tab"),
            ("Nueva Carpeta", "win.new_folder"),
            ("Deshacer", "win.undo"),
            ("Copiar", "win.copy"),
            ("Cortar", "win.cut"),
            ("Pegar", "win.paste"),
            ("Duplicar", "win.duplicate"),
            ("Comprimir (.zip)", "win.compress_zip"),
            ("Comprimir (.tar.gz)", "win.compress_targz"),
            ("Extraer aquí", "win.extract"),
            ("Mover a la Papelera", "win.trash"),
            ("Abrir en Terminal", "win.open_terminal"),
            ("Conectar al servidor…", "win.connect_server"),
            ("Mostrar Archivos Ocultos", "win.toggle_hidden"),
            (
                if es_ui {
                    "Buscar en subcarpetas"
                } else {
                    "Search subfolders"
                },
                "win.search_recursive",
            ),
            ("Recargar", "win.reload"),
        ] {
            action_box.append(&make_action_menu_btn(lbl, act, None, &action_popover));
        }

        let es_menu = malus_de_common::is_spanish_locale();
        let tag_row = GtkBox::new(Orientation::Horizontal, 16);
        let tag_lbl = Label::builder()
            .label(if es_menu { "Etiquetar" } else { "Tags" })
            .xalign(0.0)
            .hexpand(true)
            .build();
        let tag_chev = Label::builder().label("›").xalign(1.0).build();
        tag_row.append(&tag_lbl);
        tag_row.append(&tag_chev);
        let tag_btn = Button::builder()
            .child(&tag_row)
            .has_frame(false)
            .css_classes(["popover-menu-item"])
            .build();
        attach_squircle_menu_hover(&tag_btn);

        let tag_popover = gtk4::Popover::builder()
            .has_arrow(false)
            .position(gtk4::PositionType::Right)
            .css_classes(["malus-popover-menu"])
            .build();
        tag_popover.set_parent(&tag_btn);
        let tag_box = GtkBox::new(Orientation::Vertical, 1);
        for def in crate::db::TAG_CATALOG.iter() {
            let label = if es_menu { def.label_es } else { def.label_en };
            let btn = make_action_menu_btn(
                label,
                "win.toggle_tag",
                Some(def.id.to_variant()),
                &tag_popover,
            );
            let parent_pop = action_popover.clone();
            btn.connect_clicked(move |_| parent_pop.popdown());
            tag_box.append(&btn);
        }
        let clear_tags_btn = make_action_menu_btn(
            if es_menu {
                "Quitar etiquetas"
            } else {
                "Clear Tags"
            },
            "win.clear_tags",
            None,
            &tag_popover,
        );
        let parent_pop_c = action_popover.clone();
        clear_tags_btn.connect_clicked(move |_| parent_pop_c.popdown());
        tag_box.append(&clear_tags_btn);
        tag_popover.set_child(Some(&tag_box));

        let tag_pop_open = tag_popover.clone();
        tag_btn.connect_clicked(move |_| {
            tag_pop_open.popup();
        });
        let tag_pop_close = tag_popover.clone();
        action_popover.connect_closed(move |_| {
            tag_pop_close.popdown();
        });
        action_box.append(&tag_btn);
        action_popover.set_child(Some(&action_box));
        action_menu_btn.set_popover(Some(&action_popover));

        // 8. Botón de búsqueda con crecimiento suave a campo squircle y repliegue al hacer clic fuera
        let search_container = GtkBox::new(Orientation::Horizontal, 0);
        search_container.set_valign(Align::Center);
        search_container.set_halign(Align::End);
        search_container.add_css_class("branches-search-container");

        let search_entry = SearchEntry::builder()
            .placeholder_text(if es_ui { "Buscar" } else { "Search" })
            .width_chars(1)
            .hexpand(true)
            .halign(Align::Fill)
            .height_request(BRANCHES_SEARCH_HEIGHT)
            .valign(Align::Center)
            .vexpand(false)
            .css_classes(["branches-search-entry"])
            .build();
        let search_shell = create_squircle_search_shell(&search_entry);
        search_shell.set_width_request(BRANCHES_SEARCH_COLLAPSED_WIDTH);
        search_shell.set_visible(false);

        let search_toggle_btn = Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Buscar archivos (Ctrl+F)")
            .valign(Align::Center)
            .halign(Align::End)
            .css_classes(["branches-tool-btn", "branches-search-btn"])
            .build();

        search_container.append(&search_shell);
        search_container.append(&search_toggle_btn);

        actions_pill_box.append(&share_menu_btn);
        actions_pill_box.append(&info_btn);
        actions_pill_box.append(&action_menu_btn);

        // Contenedor derecho del header (Cápsulas: Vistas + Split + Acciones + Búsqueda)
        let right_header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .halign(Align::End)
            .build();
        right_header_box.append(&view_box);
        right_header_box.append(&sort_pill_box);
        right_header_box.append(&split_pill_box);
        right_header_box.append(&actions_pill_box);
        right_header_box.append(job_ui.pill_widget());
        right_header_box.append(&search_container);

        // 9. Paned principal (Sidebar a la izquierda y Contenido a la derecha)
        // Mantener la barra lateral acotada a 212 px lógicos (región exacta del blur del compositor)
        // evitando que crezca al maximizar o al arrastrar el separador.
        const MAX_SIDEBAR_WIDTH: i32 = 212;
        let hpaned = Paned::builder()
            .orientation(Orientation::Horizontal)
            .position(210)
            .resize_start_child(false)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["branches-main-paned"])
            .build();
        hpaned.connect_position_notify(|paned| {
            if paned.position() > MAX_SIDEBAR_WIDTH {
                paned.set_position(MAX_SIDEBAR_WIDTH);
            }
        });

        // Barras inferiores
        let status_bar = Rc::new(StatusBar::new());
        let quick_look = QuickLook::new();

        let win_weak_for_nav = win_weak_holder.clone();

        let sidebar = Rc::new(Sidebar::new(
            {
                let w_clone = win_weak_for_nav.clone();
                move |p| {
                    if let Some(w) = w_clone.borrow().upgrade() {
                        w.active_pane().navigate_to(p);
                    }
                }
            },
            {
                let w_clone = win_weak_for_nav.clone();
                move || {
                    if let Some(w) = w_clone.borrow().upgrade() {
                        w.show_recents();
                    }
                }
            },
            {
                let w_clone = win_weak_for_nav.clone();
                move || {
                    if let Some(w) = w_clone.borrow().upgrade() {
                        w.show_applications();
                    }
                }
            },
            {
                let w_clone = win_weak_for_nav.clone();
                move |tag_name| {
                    if let Some(w) = w_clone.borrow().upgrade() {
                        w.show_tag(&tag_name);
                    }
                }
            },
            {
                let w_clone = win_weak_for_nav.clone();
                move || {
                    if let Some(w) = w_clone.borrow().upgrade() {
                        w.reload_all_panes();
                    }
                }
            },
            {
                let w_clone = win_weak_for_nav.clone();
                move || {
                    w_clone
                        .borrow()
                        .upgrade()
                        .map(|w| w.active_pane().current_path())
                        .unwrap_or_else(|| PathBuf::from("/"))
                }
            },
        ));

        let path_bar = Rc::new(PathBar::new({
            let w_clone = win_weak_for_nav.clone();
            move |p| {
                if let Some(w) = w_clone.borrow().upgrade() {
                    w.active_pane().navigate_to(p);
                }
            }
        }));

        let es = malus_de_common::is_spanish_locale();

        // Barra de pestañas estilo macOS Finder (se muestra automáticamente al abrir 2+ pestañas)
        let tab_bar_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .css_classes(["branches-tab-bar"])
            .hexpand(true)
            .visible(false)
            .build();

        let tabs_strip = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .homogeneous(true)
            .hexpand(true)
            .valign(Align::Center)
            .css_classes(["branches-tabs-strip"])
            .build();

        let new_tab_btn = Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(if es {
                "Nueva pestaña (Ctrl+T)"
            } else {
                "New Tab (Ctrl+T)"
            })
            .valign(Align::Center)
            .has_frame(false)
            .css_classes(["branches-tab-add-btn"])
            .build();
        tab_bar_box.append(&tabs_strip);
        tab_bar_box.append(&new_tab_btn);

        // Barra contextual de Papelera (se muestra cuando la ruta activa es Trash/files)
        let trash_banner_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .css_classes(["branches-trash-banner"])
            .hexpand(true)
            .visible(false)
            .build();
        let trash_icon = gtk4::Image::from_icon_name("user-trash-symbolic");
        trash_icon.set_pixel_size(16);
        let trash_banner_lbl = Label::builder()
            .label(if es {
                "Papelera — Puedes restaurar elementos o vaciarla permanentemente"
            } else {
                "Trash — Put items back or empty Trash permanently"
            })
            .halign(Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["branches-trash-banner-label"])
            .build();
        let restore_banner_btn = Button::builder()
            .label(if es { "Restaurar" } else { "Put Back" })
            .action_name("win.restore_trash")
            .valign(Align::Center)
            .build();
        let empty_banner_btn = Button::builder()
            .label(if es { "Vaciar Papelera" } else { "Empty Trash" })
            .action_name("win.empty_trash")
            .valign(Align::Center)
            .css_classes(["destructive-action"])
            .build();
        trash_banner_box.append(&trash_icon);
        trash_banner_box.append(&trash_banner_lbl);
        trash_banner_box.append(&restore_banner_btn);
        trash_banner_box.append(&empty_banner_btn);

        let tab_stack = Stack::builder()
            .transition_type(StackTransitionType::None)
            .hexpand(true)
            .vexpand(true)
            .build();

        // ==========================================
        // ENSAMBLAJE DEL PANEL IZQUIERDO (SIDEBAR ACRÍLICA)
        // ==========================================
        let left_pane = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["branches-sidebar-pane"])
            .hexpand(false)
            .vexpand(true)
            .build();

        let window_controls_start = WindowControls::builder()
            .side(PackType::Start)
            .valign(Align::Center)
            .build();

        let controls_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["branches-sidebar-windowcontrols"])
            .valign(Align::Center)
            .build();
        controls_box.append(&window_controls_start);

        let traffic_handle = WindowHandle::builder().child(&controls_box).build();
        left_pane.append(&traffic_handle);

        sidebar.widget().set_vexpand(true);
        sidebar.widget().set_hexpand(true);
        left_pane.append(sidebar.widget());

        // ==========================================
        // ENSAMBLAJE DEL PANEL DERECHO (CONTENIDO OPACO)
        // ==========================================
        let right_pane = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["branches-content-pane"])
            .hexpand(true)
            .vexpand(true)
            .build();

        let header_toolbar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .css_classes(["branches-header-toolbar"])
            .valign(Align::Center)
            .build();

        header_toolbar.append(&left_header_box);
        header_toolbar.append(&center_spacer);
        header_toolbar.append(&right_header_box);

        let header_handle = WindowHandle::builder()
            .child(&header_toolbar)
            .hexpand(true)
            .build();
        let search_criteria_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .visible(false)
            .css_classes(["branches-search-criteria-bar"])
            .build();
        let criteria_lbl = Label::builder()
            .label(if es_ui { "Criterios:" } else { "Criteria:" })
            .css_classes(["branches-criteria-label"])
            .build();
        search_criteria_bar.append(&criteria_lbl);

        let mut criteria_kind_btns: Vec<(String, ToggleButton)> = Vec::new();
        for (id, lbl_es, lbl_en) in [
            ("all", "Todo", "All"),
            ("folders", "Carpetas", "Folders"),
            ("images", "Imágenes", "Images"),
            ("documents", "Documentos", "Documents"),
            ("videos", "Vídeos", "Videos"),
            ("audio", "Audio", "Audio"),
            ("archives", "Archivos", "Archives"),
        ] {
            let chip = ToggleButton::builder()
                .label(if es_ui { lbl_es } else { lbl_en })
                .active(id == "all")
                .css_classes(["branches-criteria-chip"])
                .build();
            search_criteria_bar.append(&chip);
            criteria_kind_btns.push((id.to_string(), chip));
        }
        let criteria_kind_btns = Rc::new(criteria_kind_btns);
        for (id, chip) in criteria_kind_btns.iter() {
            let bw_chip = win_weak_holder.clone();
            let chips_ref = criteria_kind_btns.clone();
            let filter_id = id.clone();
            chip.connect_toggled(move |btn| {
                if !btn.is_active() {
                    if !chips_ref.iter().any(|(_, b)| b.is_active()) {
                        btn.set_active(true);
                    }
                    return;
                }
                for (other_id, other_btn) in chips_ref.iter() {
                    if *other_id != filter_id && other_btn.is_active() {
                        other_btn.set_active(false);
                    }
                }
                if let Some(bw) = bw_chip.borrow().upgrade() {
                    if !bw.syncing_ui.get() {
                        let filter = if filter_id == "all" {
                            None
                        } else {
                            Some(filter_id.clone())
                        };
                        bw.active_pane().set_kind_filter(filter);
                        bw.sync_active_tab_ui();
                    }
                }
            });
        }

        let criteria_spacer = GtkBox::new(Orientation::Horizontal, 0);
        criteria_spacer.set_hexpand(true);
        search_criteria_bar.append(&criteria_spacer);

        let criteria_recursive_btn = ToggleButton::builder()
            .label(if es_ui { "Subcarpetas" } else { "Subfolders" })
            .css_classes(["branches-criteria-chip"])
            .build();
        {
            let bw_rec = win_weak_holder.clone();
            criteria_recursive_btn.connect_toggled(move |btn| {
                if let Some(bw) = bw_rec.borrow().upgrade() {
                    if !bw.syncing_ui.get() {
                        bw.active_pane().set_search_recursive(btn.is_active());
                    }
                }
            });
        }
        search_criteria_bar.append(&criteria_recursive_btn);

        let criteria_close_btn = Button::builder()
            .icon_name("window-close-symbolic")
            .has_frame(false)
            .tooltip_text(if es_ui {
                "Ocultar criterios de búsqueda"
            } else {
                "Hide Search Criteria"
            })
            .build();
        {
            let bar_ref = search_criteria_bar.clone();
            let bw_close = win_weak_holder.clone();
            criteria_close_btn.connect_clicked(move |_| {
                bar_ref.set_visible(false);
                if let Some(bw) = bw_close.borrow().upgrade() {
                    bw.active_pane().set_kind_filter(None);
                    bw.sync_active_tab_ui();
                }
            });
        }
        search_criteria_bar.append(&criteria_close_btn);

        right_pane.append(&header_handle);
        right_pane.append(&tab_bar_box);
        right_pane.append(&trash_banner_box);
        right_pane.append(&search_criteria_bar);
        right_pane.append(&tab_stack);
        right_pane.append(path_bar.widget());
        right_pane.append(status_bar.widget());

        hpaned.set_start_child(Some(&left_pane));
        hpaned.set_end_child(Some(&right_pane));

        let root_overlay = Overlay::new();
        root_overlay.set_child(Some(&hpaned));
        window.set_child(Some(&root_overlay));

        let config_file =
            gio::File::for_path(malus_de_common::config::BranchesConfig::config_path());
        let config_monitor = config_file
            .monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            .ok();

        let sort_by_action = gio::SimpleAction::new_stateful(
            "sort_by",
            Some(glib::VariantTy::STRING),
            &"name".to_variant(),
        );
        let sort_order_action = gio::SimpleAction::new_stateful(
            "sort_order",
            Some(glib::VariantTy::STRING),
            &"asc".to_variant(),
        );
        let search_recursive_action =
            gio::SimpleAction::new_stateful("search_recursive", None, &false.to_variant());

        let branches_window = Rc::new(Self {
            window,
            root_overlay,
            sidebar,
            path_bar,
            status_bar,
            tabs: Rc::new(RefCell::new(Vec::new())),
            active_tab_id: Rc::new(RefCell::new(0)),
            next_tab_id: Rc::new(Cell::new(1)),
            win_weak_holder: win_weak_holder.clone(),
            tab_bar_box,
            tabs_strip,
            trash_banner_box,
            search_criteria_bar,
            criteria_kind_btns,
            criteria_recursive_btn,
            view_options_window: Rc::new(RefCell::new(None)),
            tab_stack,
            split_btn,
            back_btn,
            forward_btn,
            title_label,
            btn_icon_view,
            btn_list_view,
            btn_col_view,
            btn_gallery_view,
            quick_look,
            job_ui,
            sort_by_action,
            sort_order_action,
            search_recursive_action,
            search_shell: search_shell.clone(),
            search_entry: search_entry.clone(),
            search_toggle_btn: search_toggle_btn.clone(),
            search_expanded: Rc::new(Cell::new(false)),
            search_anim_progress: Rc::new(Cell::new(0.0)),
            search_anim_gen: Rc::new(Cell::new(0)),
            syncing_ui: Rc::new(Cell::new(false)),
            _config_monitor: config_monitor.clone(),
        });

        *win_weak_holder.borrow_mut() = Rc::downgrade(&branches_window);

        if let Some(ref monitor) = config_monitor {
            let bw_cfg = Rc::downgrade(&branches_window);
            monitor.connect_changed(move |_, _, _, _| {
                if let Some(bw) = bw_cfg.upgrade() {
                    let cfg = malus_de_common::config::BranchesConfig::load();
                    bw.apply_branches_config(&cfg);
                }
            });
        }

        // Conectar botón "+" de la barra de pestañas
        let bw_new_tab = branches_window.clone();
        new_tab_btn.connect_clicked(move |_| {
            bw_new_tab.open_new_tab_matching_active();
        });

        // Conectar botones de navegación (clic + historial con pulsación larga o clic derecho)
        let bw_back = branches_window.clone();
        branches_window.back_btn.connect_clicked(move |_| {
            bw_back.active_pane().go_back();
        });
        Self::attach_history_menu(&branches_window, &branches_window.back_btn, true);

        let bw_fwd = branches_window.clone();
        branches_window.forward_btn.connect_clicked(move |_| {
            bw_fwd.active_pane().go_forward();
        });
        Self::attach_history_menu(&branches_window, &branches_window.forward_btn, false);

        // Conectar conmutadores de vista
        let bw_v1 = branches_window.clone();
        branches_window.btn_icon_view.connect_toggled(move |btn| {
            if !bw_v1.syncing_ui.get() && btn.is_active() {
                bw_v1.active_pane().set_view_mode("icons");
            }
        });

        let bw_v2 = branches_window.clone();
        branches_window.btn_list_view.connect_toggled(move |btn| {
            if !bw_v2.syncing_ui.get() && btn.is_active() {
                bw_v2.active_pane().set_view_mode("list");
            }
        });

        let bw_v3 = branches_window.clone();
        branches_window.btn_col_view.connect_toggled(move |btn| {
            if !bw_v3.syncing_ui.get() && btn.is_active() {
                bw_v3.active_pane().set_view_mode("columns");
            }
        });

        let bw_v4 = branches_window.clone();
        branches_window
            .btn_gallery_view
            .connect_toggled(move |btn| {
                if !bw_v4.syncing_ui.get() && btn.is_active() {
                    bw_v4.active_pane().set_view_mode("gallery");
                }
            });

        // Conectar Split View toggle
        let bw_split = branches_window.clone();
        branches_window.split_btn.connect_clicked(move |_| {
            bw_split.toggle_split_view();
        });

        // Conectar botón de Información (Obtener Información / Get Info)
        let bw_info = branches_window.clone();
        info_btn.connect_clicked(move |_| {
            bw_info.open_get_info(None);
        });

        // Conectar botón de búsqueda <-> input squircle y repliegue al hacer clic fuera
        let bw_search_open = branches_window.clone();
        branches_window.search_toggle_btn.connect_clicked(move |_| {
            bw_search_open.expand_search();
        });

        let bw_stop = branches_window.clone();
        search_entry.connect_stop_search(move |_| {
            bw_stop.collapse_search(true);
        });

        let bw_search = branches_window.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            if query.is_empty() {
                bw_search.search_toggle_btn.remove_css_class("has-query");
            } else {
                bw_search.search_toggle_btn.add_css_class("has-query");
            }
            if !bw_search.syncing_ui.get() {
                bw_search.active_pane().set_search_query(query);
            }
        });

        let outside_click = GestureClick::new();
        outside_click.set_button(0);
        outside_click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let bw_outside = branches_window.clone();
        outside_click.connect_pressed(move |g, _n, x, y| {
            if !bw_outside.is_search_expanded() {
                return;
            }
            if let Some(win_w) = g.widget() {
                if let Some(picked) = win_w.pick(x, y, gtk4::PickFlags::DEFAULT) {
                    let shell_w: &gtk4::Widget = bw_outside.search_shell.upcast_ref();
                    if picked == *shell_w || picked.is_ancestor(shell_w) {
                        return;
                    }
                }
            }
            bw_outside.collapse_search(false);
        });
        branches_window.window.add_controller(outside_click);

        // Conectar acciones del menú
        branches_window.setup_actions();

        // Conectar atajos de teclado
        branches_window.setup_keybindings();

        // Crear la primera pestaña con la ruta inicial
        branches_window.open_new_tab(Some(initial_path));

        OPEN_BRANCHES_WINDOWS.with(|cell| {
            cell.borrow_mut().push(Rc::downgrade(&branches_window));
        });

        branches_window
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn apply_branches_config(&self, cfg: &malus_de_common::config::BranchesConfig) {
        let panes: Vec<Rc<FolderPane>> = self
            .tabs
            .borrow()
            .iter()
            .flat_map(|t| [t.primary_pane.clone(), t.secondary_pane.clone()])
            .collect();
        for pane in panes {
            pane.apply_branches_config(cfg);
        }
    }

    pub fn reload_all_panes(&self) {
        let panes: Vec<Rc<FolderPane>> = self
            .tabs
            .borrow()
            .iter()
            .flat_map(|t| {
                if *t.split_view_active.borrow() {
                    vec![t.primary_pane.clone(), t.secondary_pane.clone()]
                } else {
                    vec![t.primary_pane.clone()]
                }
            })
            .collect();
        for pane in panes {
            pane.reload();
        }
    }

    fn folder_display_title(path: &PathBuf) -> String {
        let es = malus_de_common::is_spanish_locale();
        if path == &PathBuf::from("/") {
            "Malus SSD".to_string()
        } else if path.ends_with("Trash/files") {
            if es {
                "Papelera".to_string()
            } else {
                "Trash".to_string()
            }
        } else {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Branches".to_string())
        }
    }

    fn pane_display_title(pane: &FolderPane) -> String {
        let es = malus_de_common::is_spanish_locale();
        if pane.is_recents() {
            if es {
                "Recientes".to_string()
            } else {
                "Recents".to_string()
            }
        } else if pane.is_applications() {
            if es {
                "Aplicaciones".to_string()
            } else {
                "Applications".to_string()
            }
        } else if let Some(tag_id) = pane.active_tag() {
            crate::db::open_db()
                .map(|c| crate::db::tag_effective_label(&c, &tag_id, es))
                .unwrap_or_else(|_| crate::db::tag_display_label(&tag_id, es))
        } else {
            Self::folder_display_title(&pane.current_path())
        }
    }

    pub fn active_pane(&self) -> Rc<FolderPane> {
        let active_id = *self.active_tab_id.borrow();
        let tabs = self.tabs.borrow();
        let tab = tabs
            .iter()
            .find(|t| t.id == active_id)
            .or_else(|| tabs.first())
            .expect("Debe existir al menos una pestaña activa");
        if *tab.active_pane_id.borrow() == PaneId::Secondary {
            tab.secondary_pane.clone()
        } else {
            tab.primary_pane.clone()
        }
    }

    pub fn open_target_in_new_tab(&self, spec: &str) {
        if spec == "@recents" {
            self.open_new_tab(None);
            self.show_recents();
        } else if spec == "@applications" {
            self.open_new_tab(None);
            self.show_applications();
        } else if let Some(tag_id) = spec.strip_prefix("@tag:") {
            self.open_new_tab(None);
            self.show_tag(tag_id);
        } else {
            self.open_new_tab(Some(PathBuf::from(spec)));
        }
    }

    pub fn open_new_tab_matching_active(&self) {
        let has_tabs = !self.tabs.borrow().is_empty();
        if !has_tabs {
            let home = std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"));
            self.open_new_tab(Some(home));
            return;
        }
        let active = self.active_pane();
        if active.is_recents() {
            self.open_target_in_new_tab("@recents");
        } else if active.is_applications() {
            self.open_target_in_new_tab("@applications");
        } else if let Some(tag_id) = active.active_tag() {
            self.open_target_in_new_tab(&format!("@tag:{tag_id}"));
        } else {
            self.open_new_tab(Some(active.current_path()));
        }
    }

    pub fn open_new_tab(&self, target: Option<PathBuf>) {
        let inherited_view = {
            let tabs = self.tabs.borrow();
            let active_id = *self.active_tab_id.borrow();
            tabs.iter()
                .find(|t| t.id == active_id)
                .or_else(|| tabs.first())
                .map(|t| {
                    if *t.active_pane_id.borrow() == PaneId::Secondary {
                        t.secondary_pane.view_mode()
                    } else {
                        t.primary_pane.view_mode()
                    }
                })
        };

        let initial_path = target.unwrap_or_else(|| {
            if self.tabs.borrow().is_empty() {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("/"))
            } else {
                self.active_pane().current_path()
            }
        });

        let tab_id = self.next_tab_id.get();
        self.next_tab_id.set(tab_id + 1);

        let active_pane_id = Rc::new(RefCell::new(PaneId::Primary));
        let split_view_active = Rc::new(RefCell::new(false));
        let win_weak = self.win_weak_holder.clone();
        let ql_for_sel = self.quick_look.clone();

        let on_activated = {
            let w_for_act = win_weak.clone();
            Rc::new(move |id: PaneId| {
                if let Some(w) = w_for_act.borrow().upgrade() {
                    if *w.active_tab_id.borrow() == tab_id {
                        w.set_active_pane(id);
                    }
                }
            })
        };

        let on_path_changed = {
            let w_for_path = win_weak.clone();
            let tab_pane_id = active_pane_id.clone();
            Rc::new(move |id: PaneId, path: PathBuf| {
                if let Some(w) = w_for_path.borrow().upgrade() {
                    if *tab_pane_id.borrow() == id {
                        w.update_tab_title(tab_id);
                        if *w.active_tab_id.borrow() == tab_id {
                            w.sync_active_ui(&path);
                        }
                    }
                }
            })
        };

        let on_selection_changed = {
            let w_for_sel = win_weak.clone();
            let tab_pane_id = active_pane_id.clone();
            Rc::new(
                move |id: PaneId, total_count: usize, selected_count: usize, path: PathBuf| {
                    if let Some(w) = w_for_sel.borrow().upgrade() {
                        if *w.active_tab_id.borrow() == tab_id && *tab_pane_id.borrow() == id {
                            let free = get_free_space(&path);
                            w.status_bar.update(total_count, selected_count, &free);
                            if ql_for_sel.is_open() {
                                if let Some(item) = w.active_pane().get_selected_item() {
                                    ql_for_sel.update_if_open(&item);
                                }
                            } else if selected_count > 0 {
                                if let Some(item) = w.active_pane().get_selected_item() {
                                    ql_for_sel.prefetch_item(&item);
                                }
                            }
                        }
                    }
                },
            )
        };

        let on_close_split = {
            let w_for_close = win_weak.clone();
            Rc::new(move || {
                if let Some(w) = w_for_close.borrow().upgrade() {
                    w.set_tab_split_view(tab_id, false);
                }
            })
        };

        let primary_pane = FolderPane::new(
            PaneId::Primary,
            initial_path.clone(),
            on_activated.clone(),
            on_path_changed.clone(),
            on_selection_changed.clone(),
            None,
        );

        let secondary_path = initial_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| initial_path.clone());
        let secondary_pane = FolderPane::new(
            PaneId::Secondary,
            secondary_path,
            on_activated,
            on_path_changed,
            on_selection_changed,
            Some(on_close_split),
        );
        secondary_pane.widget().set_visible(false);

        if let Some(ref view_mode) = inherited_view {
            primary_pane.set_view_mode(view_mode);
            secondary_pane.set_view_mode(view_mode);
        }

        let cfg = malus_de_common::config::BranchesConfig::load();
        primary_pane.apply_branches_config(&cfg);
        secondary_pane.apply_branches_config(&cfg);

        let split_paned = Paned::builder()
            .orientation(Orientation::Horizontal)
            .position(420)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["branches-split-paned"])
            .build();
        split_paned.set_start_child(Some(primary_pane.widget()));
        split_paned.set_end_child(Some(secondary_pane.widget()));

        // Widget de pestaña en la barra de pestañas
        let tab_widget = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .hexpand(true)
            .valign(Align::Center)
            .css_classes(["branches-tab"])
            .build();

        let tab_title = Self::pane_display_title(&primary_pane);
        let tab_title_label = Label::builder()
            .label(&tab_title)
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["branches-tab-label"])
            .build();

        let es = malus_de_common::is_spanish_locale();
        let close_btn = Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(if es {
                "Cerrar pestaña (Ctrl+W)"
            } else {
                "Close Tab (Ctrl+W)"
            })
            .valign(Align::Center)
            .has_frame(false)
            .css_classes(["branches-tab-close-btn"])
            .build();

        let right_spacer = GtkBox::builder()
            .valign(Align::Center)
            .css_classes(["branches-tab-spacer"])
            .build();

        {
            let bw_close = self.clone_ref();
            close_btn.connect_clicked(move |_| {
                bw_close.close_tab(tab_id);
            });
        }

        {
            let click = GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            let bw_sel = self.clone_ref();
            click.connect_released(move |_, _, _, _| {
                bw_sel.select_tab(tab_id);
            });
            tab_widget.add_controller(click);
        }

        {
            let mid_click = GestureClick::new();
            mid_click.set_button(gdk::BUTTON_MIDDLE);
            let bw_mid = self.clone_ref();
            mid_click.connect_released(move |g, _, _, _| {
                g.set_state(gtk4::EventSequenceState::Claimed);
                bw_mid.close_tab(tab_id);
            });
            tab_widget.add_controller(mid_click);
        }

        tab_widget.append(&close_btn);
        tab_widget.append(&tab_title_label);
        tab_widget.append(&right_spacer);

        let stack_name = format!("tab-{tab_id}");
        self.tab_stack.add_named(&split_paned, Some(&stack_name));
        self.tabs_strip.append(&tab_widget);

        self.tabs.borrow_mut().push(BranchesTab {
            id: tab_id,
            primary_pane,
            secondary_pane,
            split_paned,
            active_pane_id,
            split_view_active,
            tab_widget,
            tab_title_label,
        });

        self.refresh_tab_bar_visibility();
        self.select_tab(tab_id);
    }

    fn refresh_tab_bar_visibility(&self) {
        let count = self.tabs.borrow().len();
        self.tab_bar_box.set_visible(count > 1);
    }

    pub fn select_tab(&self, tab_id: u64) {
        let found = {
            let tabs = self.tabs.borrow();
            tabs.iter().any(|t| t.id == tab_id)
        };
        if !found {
            return;
        }
        *self.active_tab_id.borrow_mut() = tab_id;

        {
            let tabs = self.tabs.borrow();
            for tab in tabs.iter() {
                if tab.id == tab_id {
                    tab.tab_widget.add_css_class("branches-tab-active");
                } else {
                    tab.tab_widget.remove_css_class("branches-tab-active");
                }
            }
        }

        self.tab_stack
            .set_visible_child_name(&format!("tab-{tab_id}"));
        self.sync_active_tab_ui();
        self.refresh_all_cut_states();
    }

    pub fn close_tab(&self, tab_id: u64) {
        let (removed, next_id) = {
            let mut tabs = self.tabs.borrow_mut();
            if tabs.len() <= 1 {
                drop(tabs);
                self.window.close();
                return;
            }
            let Some(pos) = tabs.iter().position(|t| t.id == tab_id) else {
                return;
            };
            let removed = tabs.remove(pos);
            let next_pos = pos.min(tabs.len() - 1);
            let next_id = tabs[next_pos].id;
            (removed, next_id)
        };

        self.tabs_strip.remove(&removed.tab_widget);
        self.tab_stack.remove(&removed.split_paned);
        self.refresh_tab_bar_visibility();

        if *self.active_tab_id.borrow() == tab_id {
            self.select_tab(next_id);
        }
    }

    pub fn close_active_tab(&self) {
        let active_id = *self.active_tab_id.borrow();
        self.close_tab(active_id);
    }

    pub fn cycle_tab(&self, delta: isize) {
        let target_id = {
            let tabs = self.tabs.borrow();
            if tabs.len() <= 1 {
                return;
            }
            let active_id = *self.active_tab_id.borrow();
            let pos = tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
            let next_pos = (pos as isize + delta).rem_euclid(tabs.len() as isize) as usize;
            tabs[next_pos].id
        };
        self.select_tab(target_id);
    }

    fn update_tab_title(&self, tab_id: u64) {
        let tabs = self.tabs.borrow();
        if let Some(tab) = tabs.iter().find(|t| t.id == tab_id) {
            let pane = if *tab.active_pane_id.borrow() == PaneId::Secondary {
                &tab.secondary_pane
            } else {
                &tab.primary_pane
            };
            tab.tab_title_label
                .set_label(&Self::pane_display_title(pane));
        }
    }

    fn sync_active_tab_ui(&self) {
        let (pane, split_active) = {
            let active_id = *self.active_tab_id.borrow();
            let tabs = self.tabs.borrow();
            let Some(tab) = tabs.iter().find(|t| t.id == active_id) else {
                return;
            };
            let pane = if *tab.active_pane_id.borrow() == PaneId::Secondary {
                tab.secondary_pane.clone()
            } else {
                tab.primary_pane.clone()
            };
            let split_active = *tab.split_view_active.borrow();
            (pane, split_active)
        };

        if split_active {
            self.split_btn.add_css_class("active");
        } else {
            self.split_btn.remove_css_class("active");
        }

        self.syncing_ui.set(true);
        match pane.view_mode().as_str() {
            "list" => self.btn_list_view.set_active(true),
            "columns" => self.btn_col_view.set_active(true),
            "gallery" => self.btn_gallery_view.set_active(true),
            _ => self.btn_icon_view.set_active(true),
        }
        let (sort_by, sort_order) = pane.sort_settings();
        self.sort_by_action
            .set_state(&sort_by.as_str().to_variant());
        self.sort_order_action
            .set_state(&sort_order.as_str().to_variant());
        let query = pane.search_query();
        self.search_entry.set_text(&query);
        self.search_recursive_action
            .set_state(&pane.search_recursive().to_variant());
        let active_kind = pane.kind_filter().unwrap_or_else(|| "all".to_string());
        for (id, btn) in self.criteria_kind_btns.iter() {
            btn.set_active(*id == active_kind);
        }
        self.criteria_recursive_btn.set_active(pane.search_recursive());
        if query.is_empty() {
            self.search_toggle_btn.remove_css_class("has-query");
            if !self.search_entry.has_focus() && self.is_search_expanded() {
                self.collapse_search(false);
            }
        } else {
            self.search_toggle_btn.add_css_class("has-query");
        }
        self.syncing_ui.set(false);

        if pane.is_recents() {
            self.trash_banner_box.set_visible(false);
            self.path_bar.widget().set_visible(false);
            let title = Self::pane_display_title(&pane);
            self.title_label.set_label(&title);
            self.sidebar.highlight_virtual("target_recents");
            self.back_btn.set_sensitive(pane.can_go_back());
            self.forward_btn.set_sensitive(pane.can_go_forward());
            let free = get_free_space(&pane.current_path());
            self.status_bar
                .update(pane.items_count(), pane.get_selected_items().len(), &free);
        } else if pane.is_applications() {
            self.trash_banner_box.set_visible(false);
            self.path_bar.widget().set_visible(false);
            let title = Self::pane_display_title(&pane);
            self.title_label.set_label(&title);
            self.sidebar.highlight_virtual("target_applications");
            self.back_btn.set_sensitive(pane.can_go_back());
            self.forward_btn.set_sensitive(pane.can_go_forward());
            let free = get_free_space(&pane.current_path());
            self.status_bar
                .update(pane.items_count(), pane.get_selected_items().len(), &free);
        } else if let Some(tag_id) = pane.active_tag() {
            self.trash_banner_box.set_visible(false);
            self.path_bar.widget().set_visible(false);
            let title = Self::pane_display_title(&pane);
            self.title_label.set_label(&title);
            self.sidebar.highlight_tag(&tag_id);
            self.back_btn.set_sensitive(pane.can_go_back());
            self.forward_btn.set_sensitive(pane.can_go_forward());
            let free = get_free_space(&pane.current_path());
            self.status_bar
                .update(pane.items_count(), pane.get_selected_items().len(), &free);
        } else {
            let curr = pane.current_path();
            self.sync_active_ui(&curr);
        }
    }

    pub fn set_active_pane(&self, id: PaneId) {
        let active_tab_id = *self.active_tab_id.borrow();
        let curr = {
            let tabs = self.tabs.borrow();
            let Some(tab) = tabs.iter().find(|t| t.id == active_tab_id) else {
                return;
            };
            *tab.active_pane_id.borrow_mut() = id;
            tab.primary_pane.set_is_active(id == PaneId::Primary);
            tab.secondary_pane.set_is_active(id == PaneId::Secondary);
            let active = if id == PaneId::Secondary {
                &tab.secondary_pane
            } else {
                &tab.primary_pane
            };
            tab.tab_title_label
                .set_label(&Self::pane_display_title(active));
            active.current_path()
        };
        self.sync_active_tab_ui();
        let _ = curr;
    }

    pub fn toggle_split_view(&self) {
        let active_id = *self.active_tab_id.borrow();
        let current = {
            let tabs = self.tabs.borrow();
            tabs.iter()
                .find(|t| t.id == active_id)
                .map(|t| *t.split_view_active.borrow())
                .unwrap_or(false)
        };
        self.set_tab_split_view(active_id, !current);
    }

    pub fn toggle_quick_look(&self) {
        let pane = self.active_pane();
        let item = pane.get_selected_item();
        let items = pane.items_snapshot();
        self.quick_look
            .toggle_in_overlay(&self.root_overlay, item, items);
    }

    pub fn open_quick_look_for_path(&self, target: &std::path::Path) {
        let pane = self.active_pane();
        let items = pane.items_snapshot();
        let item = items
            .iter()
            .find(|i| i.path == target)
            .cloned()
            .or_else(|| crate::fs_backend::FileItem::from_path(target));
        self.quick_look
            .toggle_in_overlay(&self.root_overlay, item, items);
    }

    pub fn make_alias_for_paths(&self, sources: Vec<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        let mut created_any = false;
        let mut last_err: Option<String> = None;
        for src in sources {
            let Some(parent) = src.parent() else {
                continue;
            };
            let is_dir = src.is_dir();
            let file_name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "item".to_string());
            let (stem, ext) = if is_dir {
                (file_name.clone(), String::new())
            } else if let Some((s, e)) = file_name.rsplit_once('.') {
                if s.is_empty() {
                    (file_name.clone(), String::new())
                } else {
                    (s.to_string(), format!(".{}", e))
                }
            } else {
                (file_name.clone(), String::new())
            };

            let mut candidate = parent.join(format!("{} alias{}", stem, ext));
            let mut idx = 2usize;
            while candidate.symlink_metadata().is_ok() {
                candidate = parent.join(format!("{} alias {}{}", stem, idx, ext));
                idx += 1;
            }
            match std::os::unix::fs::symlink(&src, &candidate) {
                Ok(()) => created_any = true,
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        if created_any {
            self.reload_all_panes();
        }
        if let Some(err) = last_err {
            gtk4::AlertDialog::builder()
                .message(err)
                .build()
                .show(Some(&self.window));
        }
    }

    fn share_via_malus_link(&self) {
        let mut paths = self.selected_paths();
        if paths.is_empty() {
            paths.push(self.active_pane().current_path());
        }
        share_paths_via_malus_link(Some(self.window.upcast_ref()), None, paths);
    }

    pub fn open_get_info(&self, explicit_path: Option<PathBuf>) {
        let target = explicit_path.or_else(|| {
            self.active_pane()
                .get_selected_item()
                .map(|i| i.path)
                .or_else(|| Some(self.active_pane().current_path()))
        });
        let Some(path) = target else { return };
        let bw = self.clone_ref();
        crate::info_window::show_get_info_window(
            &self.window,
            path,
            Rc::new(move || {
                bw.reload_all_panes();
            }),
        );
    }

    fn attach_history_menu(bw: &Rc<Self>, btn: &Button, is_back: bool) {
        let long_press = gtk4::GestureLongPress::new();
        let bw_lp = bw.clone();
        let btn_lp = btn.clone();
        long_press.connect_pressed(move |_, _, _| {
            bw_lp.show_history_popover(&btn_lp, is_back);
        });
        btn.add_controller(long_press);

        let right_click = GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        let bw_rc = bw.clone();
        let btn_rc = btn.clone();
        right_click.connect_pressed(move |g, _, _, _| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            bw_rc.show_history_popover(&btn_rc, is_back);
        });
        btn.add_controller(right_click);
    }

    fn show_history_popover(&self, anchor: &Button, is_back: bool) {
        let entries = if is_back {
            self.active_pane().history_back_list()
        } else {
            self.active_pane().history_forward_list()
        };
        if entries.is_empty() {
            return;
        }
        let popover = gtk4::Popover::builder()
            .has_arrow(true)
            .css_classes(["malus-popover-menu"])
            .build();
        popover.set_parent(anchor);
        popover.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);
        vbox.set_margin_top(4);
        vbox.set_margin_bottom(4);
        vbox.set_margin_start(4);
        vbox.set_margin_end(4);

        for (idx, path) in entries.into_iter().take(12).enumerate() {
            let label = Self::folder_display_title(&path);
            let item_btn = Button::builder()
                .label(&label)
                .has_frame(false)
                .css_classes(["popover-menu-item"])
                .build();
            if let Some(child) = item_btn.child() {
                if let Some(lbl) = child.downcast_ref::<Label>() {
                    lbl.set_xalign(0.0);
                }
            }
            attach_squircle_menu_hover(&item_btn);
            let bw = self.clone_ref();
            let pop = popover.clone();
            let steps = idx + 1;
            item_btn.connect_clicked(move |_| {
                pop.popdown();
                if is_back {
                    bw.active_pane().go_back_steps(steps);
                } else {
                    bw.active_pane().go_forward_steps(steps);
                }
            });
            vbox.append(&item_btn);
        }

        popover.set_child(Some(&vbox));
        popover.popup();
    }

    pub fn show_recents(&self) {
        self.active_pane().show_recents();
        let title = if malus_de_common::is_spanish_locale() {
            "Recientes"
        } else {
            "Recents"
        };
        self.title_label.set_label(title);
        self.sidebar.highlight_virtual("target_recents");
        let active_id = *self.active_tab_id.borrow();
        self.update_tab_title(active_id);
    }

    pub fn show_applications(&self) {
        self.active_pane().show_applications();
        let title = if malus_de_common::is_spanish_locale() {
            "Aplicaciones"
        } else {
            "Applications"
        };
        self.path_bar.widget().set_visible(false);
        self.title_label.set_label(title);
        self.sidebar.highlight_virtual("target_applications");
        let active_id = *self.active_tab_id.borrow();
        self.update_tab_title(active_id);
    }

    pub fn show_tag(&self, tag_name: &str) {
        self.active_pane().show_tag_filter(tag_name);
        self.sync_active_tab_ui();
        let active_id = *self.active_tab_id.borrow();
        self.update_tab_title(active_id);
    }

    fn set_tab_split_view(&self, tab_id: u64, active: bool) {
        let (secondary, split_paned, is_current_tab) = {
            let tabs = self.tabs.borrow();
            let Some(tab) = tabs.iter().find(|t| t.id == tab_id) else {
                return;
            };
            *tab.split_view_active.borrow_mut() = active;
            tab.secondary_pane.widget().set_visible(active);
            tab.primary_pane.set_split_mode_active(active);
            tab.secondary_pane.set_split_mode_active(active);
            (
                tab.secondary_pane.clone(),
                tab.split_paned.clone(),
                *self.active_tab_id.borrow() == tab_id,
            )
        };

        if active {
            if is_current_tab {
                self.split_btn.add_css_class("active");
            }
            split_paned.set_position(430);
            secondary.reload();
        } else {
            if is_current_tab {
                self.split_btn.remove_css_class("active");
                self.set_active_pane(PaneId::Primary);
            } else {
                let tabs = self.tabs.borrow();
                if let Some(tab) = tabs.iter().find(|t| t.id == tab_id) {
                    *tab.active_pane_id.borrow_mut() = PaneId::Primary;
                    tab.primary_pane.set_is_active(true);
                    tab.secondary_pane.set_is_active(false);
                }
            }
        }
    }

    pub fn sync_active_ui(&self, path: &PathBuf) {
        let active = self.active_pane();

        self.back_btn.set_sensitive(active.can_go_back());
        self.forward_btn.set_sensitive(active.can_go_forward());

        let folder_name = Self::folder_display_title(path);
        self.title_label.set_label(&folder_name);

        let active_id = *self.active_tab_id.borrow();
        self.update_tab_title(active_id);

        self.path_bar.widget().set_visible(true);
        self.path_bar.update_path(path);
        self.sidebar.highlight_path(path);
        self.trash_banner_box
            .set_visible(crate::operations::is_in_trash_dir(path));

        let free = get_free_space(path);
        self.status_bar.update(
            active.items_count(),
            active.get_selected_items().len(),
            &free,
        );
    }

    fn refresh_all_cut_states(&self) {
        let panes: Vec<Rc<FolderPane>> = self
            .tabs
            .borrow()
            .iter()
            .flat_map(|t| [t.primary_pane.clone(), t.secondary_pane.clone()])
            .collect();
        for pane in panes {
            pane.refresh_cut_states();
        }
    }

    fn copy_selected(&self) {
        if self.active_pane().copy_selected() {
            self.refresh_all_cut_states();
        }
    }

    fn cut_selected(&self) {
        if self.active_pane().cut_selected() {
            self.refresh_all_cut_states();
        }
    }

    fn selected_paths_or(&self, fallback_path: PathBuf) -> Vec<PathBuf> {
        let selected: Vec<PathBuf> = self
            .active_pane()
            .get_selected_items()
            .into_iter()
            .map(|i| i.path)
            .collect();
        if selected.iter().any(|p| p == &fallback_path) && !selected.is_empty() {
            selected
        } else {
            vec![fallback_path]
        }
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.active_pane()
            .get_selected_items()
            .into_iter()
            .map(|i| i.path)
            .collect()
    }

    fn copy_or_cut_item_with_selection(
        &self,
        action: crate::operations::ClipboardAction,
        clicked_path: PathBuf,
    ) {
        let paths = self.selected_paths_or(clicked_path);
        crate::operations::set_clipboard(action, paths);
        self.refresh_all_cut_states();
    }

    fn paste_into_folder(&self, dest_dir: PathBuf) {
        if !dest_dir.is_dir() {
            return;
        }
        let job_ui = self.job_ui.clone();
        crate::operations::read_clipboard_payload_async(move |payload| {
            let Some(payload) = payload else { return };
            if payload.paths.is_empty() {
                return;
            }
            match payload.action {
                crate::operations::ClipboardAction::Copy => {
                    job_ui.start_job(
                        JobRequest::Copy {
                            sources: payload.paths,
                            dest_dir,
                        },
                        false,
                    );
                }
                crate::operations::ClipboardAction::Cut => {
                    job_ui.start_job(
                        JobRequest::Move {
                            sources: payload.paths,
                            dest_dir,
                        },
                        true,
                    );
                }
            }
        });
    }

    fn duplicate_paths(&self, sources: Vec<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        self.job_ui
            .start_job(JobRequest::Duplicate { sources }, false);
    }

    fn duplicate_selected(&self) {
        self.duplicate_paths(self.selected_paths());
    }

    fn trash_paths(&self, sources: Vec<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        if sources
            .iter()
            .any(|p| crate::operations::is_in_trash_dir(p))
        {
            self.confirm_delete_permanent(sources);
            return;
        }
        self.job_ui.start_job(JobRequest::Trash { sources }, false);
    }

    fn trash_selected(&self) {
        self.trash_paths(self.selected_paths());
    }

    fn restore_trash_paths(&self, sources: Vec<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        let mut restored_any = false;
        let mut last_err = None;
        for path in sources {
            match crate::operations::restore_from_trash(&path) {
                Ok(_) => restored_any = true,
                Err(e) => last_err = Some(e),
            }
        }
        if restored_any {
            self.reload_all_panes();
        }
        if let Some(err) = last_err {
            gtk4::AlertDialog::builder()
                .message(err)
                .build()
                .show(Some(&self.window));
        }
    }

    fn confirm_destructive_dialog<F: FnOnce() + 'static>(
        &self,
        title: &str,
        body: &str,
        confirm_label: &str,
        on_confirm: F,
    ) {
        let es = malus_de_common::is_spanish_locale();
        let dialog = gtk4::Window::builder()
            .transient_for(&self.window)
            .destroy_with_parent(true)
            .modal(true)
            .resizable(false)
            .title(title)
            .default_width(380)
            .build();

        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_margin_top(16);
        vbox.set_margin_bottom(16);
        vbox.set_margin_start(18);
        vbox.set_margin_end(18);

        let lbl = Label::builder().label(body).wrap(true).xalign(0.0).build();
        vbox.append(&lbl);

        let btns = GtkBox::new(Orientation::Horizontal, 8);
        btns.set_halign(Align::End);
        let cancel_btn = Button::with_label(if es { "Cancelar" } else { "Cancel" });
        let confirm_btn = Button::builder()
            .label(confirm_label)
            .css_classes(["destructive-action"])
            .build();
        btns.append(&cancel_btn);
        btns.append(&confirm_btn);
        vbox.append(&btns);
        dialog.set_child(Some(&vbox));

        let d_cancel = dialog.clone();
        cancel_btn.connect_clicked(move |_| d_cancel.close());

        let cb_slot = std::cell::RefCell::new(Some(on_confirm));
        let d_ok = dialog.clone();
        confirm_btn.connect_clicked(move |_| {
            d_ok.close();
            if let Some(f) = cb_slot.borrow_mut().take() {
                f();
            }
        });

        dialog.present();
    }

    fn confirm_delete_permanent(&self, sources: Vec<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        let es = malus_de_common::is_spanish_locale();
        let body = if sources.len() == 1 {
            let name = sources[0]
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if es {
                format!("¿Seguro que quieres eliminar «{}» permanentemente? Esta acción no se puede deshacer.", name)
            } else {
                format!(
                    "Are you sure you want to permanently delete \"{}\"? This cannot be undone.",
                    name
                )
            }
        } else if es {
            format!("¿Seguro que quieres eliminar {} elementos permanentemente? Esta acción no se puede deshacer.", sources.len())
        } else {
            format!(
                "Are you sure you want to permanently delete {} items? This cannot be undone.",
                sources.len()
            )
        };
        let bw = self.clone_ref();
        self.confirm_destructive_dialog(
            if es {
                "Eliminar permanentemente"
            } else {
                "Delete Permanently"
            },
            &body,
            if es { "Eliminar" } else { "Delete" },
            move || {
                let mut changed = false;
                let mut last_err = None;
                for p in sources {
                    match crate::operations::delete_permanent(&p) {
                        Ok(()) => changed = true,
                        Err(e) => last_err = Some(e),
                    }
                }
                if changed {
                    bw.reload_all_panes();
                }
                if let Some(err) = last_err {
                    gtk4::AlertDialog::builder()
                        .message(err)
                        .build()
                        .show(Some(&bw.window));
                }
            },
        );
    }

    fn confirm_empty_trash(&self) {
        let es = malus_de_common::is_spanish_locale();
        let files_dir = crate::operations::trash_files_dir();
        let count = std::fs::read_dir(&files_dir)
            .map(|iter| iter.count())
            .unwrap_or(0);
        if count == 0 {
            return;
        }
        let body = if es {
            format!("¿Vaciar los {} elementos de la Papelera permanentemente? Esta acción no se puede deshacer.", count)
        } else {
            format!(
                "Permanently empty {} item(s) from the Trash? This cannot be undone.",
                count
            )
        };
        let bw = self.clone_ref();
        self.confirm_destructive_dialog(
            if es { "Vaciar Papelera" } else { "Empty Trash" },
            &body,
            if es { "Vaciar Papelera" } else { "Empty Trash" },
            move || {
                if let Err(err) = crate::operations::empty_trash() {
                    gtk4::AlertDialog::builder()
                        .message(err)
                        .build()
                        .show(Some(&bw.window));
                }
                bw.reload_all_panes();
            },
        );
    }

    fn compress_paths(&self, sources: Vec<PathBuf>, format: ArchiveFormat) {
        if sources.is_empty() {
            return;
        }
        let dest_dir = sources[0]
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.active_pane().current_path());
        self.job_ui.start_job(
            JobRequest::Compress {
                sources,
                dest_dir,
                format,
            },
            false,
        );
    }

    fn extract_archive(&self, archive: PathBuf) {
        if !crate::jobs::is_supported_archive(&archive) {
            return;
        }
        let dest_dir = archive
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.active_pane().current_path());
        self.job_ui
            .start_job(JobRequest::Extract { archive, dest_dir }, false);
    }

    fn move_or_copy_paths_to(&self, dest_dir: PathBuf, sources: Vec<PathBuf>, is_copy: bool) {
        if sources.is_empty() || !dest_dir.is_dir() {
            return;
        }
        let req = if is_copy {
            JobRequest::Copy { sources, dest_dir }
        } else {
            JobRequest::Move { sources, dest_dir }
        };
        self.job_ui.start_job(req, false);
    }

    fn paste_into_active_pane(&self) {
        let active = self.active_pane();
        if active.is_recents() || active.is_applications() {
            return;
        }
        self.paste_into_folder(active.current_path());
    }

    fn setup_actions(&self) {
        let actions = gio::SimpleActionGroup::new();

        let bw_new_tab = self.clone_ref();
        let act_new_tab = gio::SimpleAction::new("new_tab", None);
        act_new_tab.connect_activate(move |_, _| {
            bw_new_tab.open_new_tab_matching_active();
        });
        actions.add_action(&act_new_tab);

        let bw_open_tab = self.clone_ref();
        let act_open_tab = gio::SimpleAction::new("open_in_new_tab", Some(glib::VariantTy::STRING));
        act_open_tab.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_open_tab.open_target_in_new_tab(target);
            }
        });
        actions.add_action(&act_open_tab);

        let bw_reload = self.clone_ref();
        let act_reload = gio::SimpleAction::new("reload", None);
        act_reload.connect_activate(move |_, _| {
            bw_reload.active_pane().reload();
        });
        actions.add_action(&act_reload);

        let bw_hidden = self.clone_ref();
        let act_hidden = gio::SimpleAction::new("toggle_hidden", None);
        act_hidden.connect_activate(move |_, _| {
            bw_hidden.active_pane().toggle_hidden();
        });
        actions.add_action(&act_hidden);

        let bw_term = self.clone_ref();
        let act_term = gio::SimpleAction::new("open_terminal", None);
        act_term.connect_activate(move |_, _| {
            let p = bw_term.active_pane().current_path();
            crate::fs_backend::open_in_terminal(&p);
        });
        actions.add_action(&act_term);

        let bw_mkdir = self.clone_ref();
        let act_mkdir = gio::SimpleAction::new("new_folder", None);
        act_mkdir.connect_activate(move |_, _| {
            bw_mkdir.prompt_new_folder();
        });
        actions.add_action(&act_mkdir);

        let bw_dup = self.clone_ref();
        let act_dup = gio::SimpleAction::new("duplicate", None);
        act_dup.connect_activate(move |_, _| {
            bw_dup.duplicate_selected();
        });
        actions.add_action(&act_dup);

        let bw_dup_item = self.clone_ref();
        let act_dup_item = gio::SimpleAction::new("duplicate_item", Some(glib::VariantTy::STRING));
        act_dup_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_dup_item.selected_paths_or(PathBuf::from(target));
                bw_dup_item.duplicate_paths(paths);
            }
        });
        actions.add_action(&act_dup_item);

        let bw_alias_item = self.clone_ref();
        let act_alias_item =
            gio::SimpleAction::new("make_alias_item", Some(glib::VariantTy::STRING));
        act_alias_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_alias_item.selected_paths_or(PathBuf::from(target));
                bw_alias_item.make_alias_for_paths(paths);
            }
        });
        actions.add_action(&act_alias_item);

        let bw_ql_item = self.clone_ref();
        let act_ql_item =
            gio::SimpleAction::new("quick_look_item", Some(glib::VariantTy::STRING));
        act_ql_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_ql_item.open_quick_look_for_path(std::path::Path::new(target));
            }
        });
        actions.add_action(&act_ql_item);

        let bw_undo = self.clone_ref();
        let act_undo = gio::SimpleAction::new("undo", None);
        act_undo.connect_activate(move |_, _| {
            bw_undo.job_ui.undo_last();
        });
        actions.add_action(&act_undo);

        let bw_undo_ren = self.clone_ref();
        let act_undo_ren = gio::SimpleAction::new(
            "record_undo_rename",
            Some(glib::VariantTy::new("(ss)").expect("valid variant type")),
        );
        act_undo_ren.connect_activate(move |_, param| {
            if let Some((old_s, new_s)) = param.and_then(|v| v.get::<(String, String)>()) {
                bw_undo_ren.job_ui.push_undo(UndoEntry::Rename {
                    from: PathBuf::from(old_s),
                    to: PathBuf::from(new_s),
                });
            }
        });
        actions.add_action(&act_undo_ren);

        let bw_undo_created = self.clone_ref();
        let act_undo_created =
            gio::SimpleAction::new("record_undo_created", Some(glib::VariantTy::STRING_ARRAY));
        act_undo_created.connect_activate(move |_, param| {
            if let Some(paths) = param.and_then(|v| v.get::<Vec<String>>()) {
                bw_undo_created.job_ui.push_undo(UndoEntry::CreatedPaths {
                    paths: paths.into_iter().map(PathBuf::from).collect(),
                });
            }
        });
        actions.add_action(&act_undo_created);

        let bw_move_to = self.clone_ref();
        let act_move_to = gio::SimpleAction::new(
            "move_paths_to",
            Some(glib::VariantTy::new("(sas)").expect("valid variant type")),
        );
        act_move_to.connect_activate(move |_, param| {
            if let Some((dest, paths)) = param.and_then(|v| v.get::<(String, Vec<String>)>()) {
                bw_move_to.move_or_copy_paths_to(
                    PathBuf::from(dest),
                    paths.into_iter().map(PathBuf::from).collect(),
                    false,
                );
            }
        });
        actions.add_action(&act_move_to);

        let bw_copy_to = self.clone_ref();
        let act_copy_to = gio::SimpleAction::new(
            "copy_paths_to",
            Some(glib::VariantTy::new("(sas)").expect("valid variant type")),
        );
        act_copy_to.connect_activate(move |_, param| {
            if let Some((dest, paths)) = param.and_then(|v| v.get::<(String, Vec<String>)>()) {
                bw_copy_to.move_or_copy_paths_to(
                    PathBuf::from(dest),
                    paths.into_iter().map(PathBuf::from).collect(),
                    true,
                );
            }
        });
        actions.add_action(&act_copy_to);

        let bw_comp_zip = self.clone_ref();
        let act_comp_zip = gio::SimpleAction::new("compress_zip", None);
        act_comp_zip.connect_activate(move |_, _| {
            bw_comp_zip.compress_paths(bw_comp_zip.selected_paths(), ArchiveFormat::Zip);
        });
        actions.add_action(&act_comp_zip);

        let bw_comp_targz = self.clone_ref();
        let act_comp_targz = gio::SimpleAction::new("compress_targz", None);
        act_comp_targz.connect_activate(move |_, _| {
            bw_comp_targz.compress_paths(bw_comp_targz.selected_paths(), ArchiveFormat::TarGz);
        });
        actions.add_action(&act_comp_targz);

        let bw_comp_item = self.clone_ref();
        let act_comp_item = gio::SimpleAction::new("compress_item", Some(glib::VariantTy::STRING));
        act_comp_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_comp_item.selected_paths_or(PathBuf::from(target));
                bw_comp_item.compress_paths(paths, ArchiveFormat::Zip);
            }
        });
        actions.add_action(&act_comp_item);

        let bw_extract = self.clone_ref();
        let act_extract = gio::SimpleAction::new("extract", None);
        act_extract.connect_activate(move |_, _| {
            if let Some(item) = bw_extract.active_pane().get_selected_item() {
                bw_extract.extract_archive(item.path);
            }
        });
        actions.add_action(&act_extract);

        let bw_extract_item = self.clone_ref();
        let act_extract_item =
            gio::SimpleAction::new("extract_item", Some(glib::VariantTy::STRING));
        act_extract_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_extract_item.extract_archive(PathBuf::from(target));
            }
        });
        actions.add_action(&act_extract_item);

        let bw_copy_sel = self.clone_ref();
        let act_copy = gio::SimpleAction::new("copy", None);
        act_copy.connect_activate(move |_, _| {
            bw_copy_sel.copy_selected();
        });
        actions.add_action(&act_copy);

        let bw_cut_sel = self.clone_ref();
        let act_cut = gio::SimpleAction::new("cut", None);
        act_cut.connect_activate(move |_, _| {
            bw_cut_sel.cut_selected();
        });
        actions.add_action(&act_cut);

        let bw_paste = self.clone_ref();
        let act_paste = gio::SimpleAction::new("paste", None);
        act_paste.connect_activate(move |_, _| {
            bw_paste.paste_into_active_pane();
        });
        actions.add_action(&act_paste);

        let bw_copy_item = self.clone_ref();
        let act_copy_item = gio::SimpleAction::new("copy_item", Some(glib::VariantTy::STRING));
        act_copy_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_copy_item.copy_or_cut_item_with_selection(
                    crate::operations::ClipboardAction::Copy,
                    PathBuf::from(target),
                );
            }
        });
        actions.add_action(&act_copy_item);

        let bw_cut_item = self.clone_ref();
        let act_cut_item = gio::SimpleAction::new("cut_item", Some(glib::VariantTy::STRING));
        act_cut_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_cut_item.copy_or_cut_item_with_selection(
                    crate::operations::ClipboardAction::Cut,
                    PathBuf::from(target),
                );
            }
        });
        actions.add_action(&act_cut_item);

        let bw_paste_into = self.clone_ref();
        let act_paste_into = gio::SimpleAction::new("paste_into", Some(glib::VariantTy::STRING));
        act_paste_into.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_paste_into.paste_into_folder(PathBuf::from(target));
            }
        });
        actions.add_action(&act_paste_into);

        let bw_trash = self.clone_ref();
        let act_trash = gio::SimpleAction::new("trash", None);
        act_trash.connect_activate(move |_, _| {
            bw_trash.trash_selected();
        });
        actions.add_action(&act_trash);

        let bw_trash_item = self.clone_ref();
        let act_trash_item = gio::SimpleAction::new("trash_item", Some(glib::VariantTy::STRING));
        act_trash_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_trash_item.selected_paths_or(PathBuf::from(target));
                bw_trash_item.trash_paths(paths);
            }
        });
        actions.add_action(&act_trash_item);

        let bw_get_info = self.clone_ref();
        let act_get_info = gio::SimpleAction::new("get_info", None);
        act_get_info.connect_activate(move |_, _| {
            bw_get_info.open_get_info(None);
        });
        actions.add_action(&act_get_info);

        let bw_get_info_item = self.clone_ref();
        let act_get_info_item =
            gio::SimpleAction::new("get_info_item", Some(glib::VariantTy::STRING));
        act_get_info_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_get_info_item.open_get_info(Some(PathBuf::from(target)));
            }
        });
        actions.add_action(&act_get_info_item);

        {
            let bw_sort = self.clone_ref();
            let act_sb = self.sort_by_action.clone();
            let act_so_for_sb = self.sort_order_action.clone();
            self.sort_by_action.connect_activate(move |_, param| {
                if let Some(val) = param.and_then(|v| v.str()) {
                    let new_by = crate::state::SortBy::from_str(val);
                    act_sb.set_state(&new_by.as_str().to_variant());
                    let pane = bw_sort.active_pane();
                    let (prev_by, prev_order) = pane.sort_settings();
                    let next_order = if new_by != prev_by {
                        match new_by {
                            crate::state::SortBy::DateLastOpened
                            | crate::state::SortBy::DateAdded
                            | crate::state::SortBy::DateModified
                            | crate::state::SortBy::DateCreated
                            | crate::state::SortBy::Size => crate::state::SortOrder::Descending,
                            crate::state::SortBy::Name
                            | crate::state::SortBy::Kind
                            | crate::state::SortBy::Tags => crate::state::SortOrder::Ascending,
                        }
                    } else {
                        prev_order
                    };
                    act_so_for_sb.set_state(&next_order.as_str().to_variant());
                    pane.set_sort(new_by, next_order);
                }
            });
            actions.add_action(&self.sort_by_action);

            let bw_dir = self.clone_ref();
            let act_so = self.sort_order_action.clone();
            self.sort_order_action.connect_activate(move |_, param| {
                if let Some(val) = param.and_then(|v| v.str()) {
                    act_so.set_state(&val.to_variant());
                    let pane = bw_dir.active_pane();
                    let (by, _) = pane.sort_settings();
                    pane.set_sort(by, crate::state::SortOrder::from_str(val));
                }
            });
            actions.add_action(&self.sort_order_action);
        }

        let bw_restore = self.clone_ref();
        let act_restore = gio::SimpleAction::new("restore_trash", None);
        act_restore.connect_activate(move |_, _| {
            bw_restore.restore_trash_paths(bw_restore.selected_paths());
        });
        actions.add_action(&act_restore);

        let bw_restore_item = self.clone_ref();
        let act_restore_item =
            gio::SimpleAction::new("restore_item", Some(glib::VariantTy::STRING));
        act_restore_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_restore_item.selected_paths_or(PathBuf::from(target));
                bw_restore_item.restore_trash_paths(paths);
            }
        });
        actions.add_action(&act_restore_item);

        let bw_empty_trash = self.clone_ref();
        let act_empty_trash = gio::SimpleAction::new("empty_trash", None);
        act_empty_trash.connect_activate(move |_, _| {
            bw_empty_trash.confirm_empty_trash();
        });
        actions.add_action(&act_empty_trash);

        let bw_del_perm = self.clone_ref();
        let act_del_perm = gio::SimpleAction::new("delete_permanent", None);
        act_del_perm.connect_activate(move |_, _| {
            bw_del_perm.confirm_delete_permanent(bw_del_perm.selected_paths());
        });
        actions.add_action(&act_del_perm);

        let bw_del_perm_item = self.clone_ref();
        let act_del_perm_item =
            gio::SimpleAction::new("delete_permanent_item", Some(glib::VariantTy::STRING));
        act_del_perm_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_del_perm_item.selected_paths_or(PathBuf::from(target));
                bw_del_perm_item.confirm_delete_permanent(paths);
            }
        });
        actions.add_action(&act_del_perm_item);

        let bw_toggle_tag = self.clone_ref();
        let act_toggle_tag = gio::SimpleAction::new("toggle_tag", Some(glib::VariantTy::STRING));
        act_toggle_tag.connect_activate(move |_, param| {
            if let Some(tag_id) = param.and_then(|v| v.str()) {
                let paths = bw_toggle_tag.selected_paths();
                bw_toggle_tag
                    .active_pane()
                    .toggle_tag_on_paths(tag_id, &paths);
            }
        });
        actions.add_action(&act_toggle_tag);

        let bw_toggle_tag_item = self.clone_ref();
        let act_toggle_tag_item = gio::SimpleAction::new(
            "toggle_tag_item",
            Some(glib::VariantTy::new("(ss)").expect("valid variant type")),
        );
        act_toggle_tag_item.connect_activate(move |_, param| {
            if let Some((tag_id, target)) = param.and_then(|v| v.get::<(String, String)>()) {
                let paths = bw_toggle_tag_item.selected_paths_or(PathBuf::from(target));
                bw_toggle_tag_item
                    .active_pane()
                    .toggle_tag_on_paths(&tag_id, &paths);
            }
        });
        actions.add_action(&act_toggle_tag_item);

        let bw_clear_tags = self.clone_ref();
        let act_clear_tags = gio::SimpleAction::new("clear_tags", None);
        act_clear_tags.connect_activate(move |_, _| {
            let paths = bw_clear_tags.selected_paths();
            bw_clear_tags.active_pane().clear_tags_on_paths(&paths);
        });
        actions.add_action(&act_clear_tags);

        let bw_clear_tags_item = self.clone_ref();
        let act_clear_tags_item =
            gio::SimpleAction::new("clear_tags_item", Some(glib::VariantTy::STRING));
        act_clear_tags_item.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                let paths = bw_clear_tags_item.selected_paths_or(PathBuf::from(target));
                bw_clear_tags_item.active_pane().clear_tags_on_paths(&paths);
            }
        });
        actions.add_action(&act_clear_tags_item);

        let bw_copy = self.clone_ref();
        let act_copy_path = gio::SimpleAction::new("copy_path", None);
        act_copy_path.connect_activate(move |_, _| {
            let path = if let Some(item) = bw_copy.active_pane().get_selected_item() {
                item.path
            } else {
                bw_copy.active_pane().current_path()
            };
            bw_copy.window.clipboard().set_text(&path.to_string_lossy());
        });
        actions.add_action(&act_copy_path);

        let bw_link = self.clone_ref();
        let act_link = gio::SimpleAction::new("share_malus_link", None);
        act_link.connect_activate(move |_, _| {
            bw_link.share_via_malus_link();
        });
        actions.add_action(&act_link);

        let bw_kde = self.clone_ref();
        let act_kde = gio::SimpleAction::new("share_kdeconnect", None);
        act_kde.connect_activate(move |_, _| {
            let path = if let Some(item) = bw_kde.active_pane().get_selected_item() {
                item.path
            } else {
                bw_kde.active_pane().current_path()
            };
            let _ = std::process::Command::new("kdeconnect-handler")
                .arg(&path)
                .spawn();
        });
        actions.add_action(&act_kde);

        let bw_email = self.clone_ref();
        let act_email = gio::SimpleAction::new("share_email", None);
        act_email.connect_activate(move |_, _| {
            let path = if let Some(item) = bw_email.active_pane().get_selected_item() {
                item.path
            } else {
                bw_email.active_pane().current_path()
            };
            let mailto = format!(
                "mailto:?subject=Compartido desde Malus DE&body={}",
                path.to_string_lossy()
            );
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(mailto);
            malus_de_common::config::apply_isolated_environment(&mut cmd);
            let _ = cmd.spawn();
        });
        actions.add_action(&act_email);

        let bw_rec = self.clone_ref();
        self.search_recursive_action
            .connect_activate(move |act, _| {
                let next = !act.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
                act.set_state(&next.to_variant());
                if !bw_rec.syncing_ui.get() {
                    bw_rec.active_pane().set_search_recursive(next);
                }
            });
        actions.add_action(&self.search_recursive_action);

        let bw_conn = self.clone_ref();
        let act_conn = gio::SimpleAction::new("connect_server", None);
        act_conn.connect_activate(move |_, _| {
            let bw_nav = bw_conn.clone_ref();
            crate::network::open_connect_server_dialog(
                bw_conn.window.upcast_ref::<gtk4::Window>(),
                move |path| {
                    bw_nav.active_pane().navigate_to(path);
                },
            );
        });
        actions.add_action(&act_conn);

        let bw_vm = self.clone_ref();
        let act_vm = gio::SimpleAction::new("set_view_mode", Some(glib::VariantTy::STRING));
        act_vm.connect_activate(move |_, param| {
            if let Some(mode) = param.and_then(|v| v.str()) {
                bw_vm.active_pane().set_view_mode(mode);
                bw_vm.sync_active_tab_ui();
            }
        });
        actions.add_action(&act_vm);

        let bw_grp = self.clone_ref();
        let act_grp = gio::SimpleAction::new("toggle_use_groups", None);
        act_grp.connect_activate(move |_, _| {
            let pane = bw_grp.active_pane();
            pane.set_use_groups(!pane.use_groups());
            bw_grp.sync_active_tab_ui();
        });
        actions.add_action(&act_grp);

        let bw_crit = self.clone_ref();
        let act_crit = gio::SimpleAction::new("toggle_search_criteria", None);
        act_crit.connect_activate(move |_, _| {
            let next = !bw_crit.search_criteria_bar.is_visible();
            bw_crit.search_criteria_bar.set_visible(next);
            if next {
                if !bw_crit.is_search_expanded() {
                    bw_crit.expand_search();
                } else {
                    bw_crit.search_entry.grab_focus();
                }
            } else {
                bw_crit.active_pane().set_kind_filter(None);
            }
            bw_crit.sync_active_tab_ui();
        });
        actions.add_action(&act_crit);

        let bw_align = self.clone_ref();
        let act_align = gio::SimpleAction::new("align_icons", None);
        act_align.connect_activate(move |_, _| {
            bw_align.active_pane().align_icon_grid();
        });
        actions.add_action(&act_align);

        let bw_vo = self.clone_ref();
        let act_vo = gio::SimpleAction::new("show_view_options", None);
        act_vo.connect_activate(move |_, _| {
            bw_vo.open_view_options_window();
        });
        actions.add_action(&act_vo);

        self.window.insert_action_group("win", Some(&actions));
    }

    fn open_view_options_window(&self) {
        if let Some(existing) = self.view_options_window.borrow_mut().take() {
            existing.close();
        }

        let es = malus_de_common::is_spanish_locale();
        let pane = self.active_pane();
        let folder_name = Self::pane_display_title(&pane);
        let cfg = malus_de_common::config::BranchesConfig::load();

        let win = gtk4::Window::builder()
            .title(if es {
                format!("Opciones de visualización — {folder_name}")
            } else {
                format!("View Options — {folder_name}")
            })
            .transient_for(&self.window)
            .resizable(false)
            .default_width(310)
            .css_classes(["branches-view-options-window"])
            .build();

        let vbox = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .css_classes(["branches-view-options-box"])
            .build();

        // 1. Usar grupos
        let row_groups = GtkBox::new(Orientation::Horizontal, 8);
        let lbl_groups = Label::builder()
            .label(if es { "Usar grupos" } else { "Use Groups" })
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["branches-view-options-section"])
            .build();
        let sw_groups = gtk4::Switch::builder()
            .active(pane.use_groups())
            .valign(Align::Center)
            .build();
        {
            let bw = self.clone_ref();
            sw_groups.connect_state_set(move |_, state| {
                bw.active_pane().set_use_groups(state);
                bw.sync_active_tab_ui();
                glib::Propagation::Proceed
            });
        }
        row_groups.append(&lbl_groups);
        row_groups.append(&sw_groups);
        vbox.append(&row_groups);

        // 2. Ordenar por
        let sort_options = [
            (crate::state::SortBy::Name, "Nombre", "Name"),
            (crate::state::SortBy::Kind, "Clase", "Kind"),
            (
                crate::state::SortBy::DateLastOpened,
                "Fecha de última apertura",
                "Date Last Opened",
            ),
            (
                crate::state::SortBy::DateAdded,
                "Fecha en que se agregó",
                "Date Added",
            ),
            (
                crate::state::SortBy::DateModified,
                "Fecha de modificación",
                "Date Modified",
            ),
            (
                crate::state::SortBy::DateCreated,
                "Fecha de creación",
                "Date Created",
            ),
            (crate::state::SortBy::Size, "Tamaño", "Size"),
            (crate::state::SortBy::Tags, "Etiquetas", "Tags"),
        ];
        let (curr_sort, _) = pane.sort_settings();
        let sort_labels: Vec<&str> = sort_options
            .iter()
            .map(|(_, es_l, en_l)| if es { *es_l } else { *en_l })
            .collect();
        let sort_dd = gtk4::DropDown::from_strings(&sort_labels);
        if let Some(pos) = sort_options.iter().position(|(s, _, _)| *s == curr_sort) {
            sort_dd.set_selected(pos as u32);
        }
        {
            let bw = self.clone_ref();
            let sort_ids: Vec<crate::state::SortBy> =
                sort_options.iter().map(|(s, _, _)| *s).collect();
            sort_dd.connect_selected_notify(move |dd| {
                let idx = dd.selected() as usize;
                if let Some(&sb) = sort_ids.get(idx) {
                    let p = bw.active_pane();
                    let (_, order) = p.sort_settings();
                    p.set_sort(sb, order);
                    bw.sort_by_action.set_state(&sb.as_str().to_variant());
                }
            });
        }
        let row_sort = GtkBox::new(Orientation::Horizontal, 8);
        row_sort.append(
            &Label::builder()
                .label(if es { "Ordenar por:" } else { "Sort By:" })
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-view-options-section"])
                .build(),
        );
        row_sort.append(&sort_dd);
        vbox.append(&row_sort);

        // 3. Disposición de iconos (para vista de íconos)
        let layout_modes = [
            (crate::db::LayoutMode::Auto, "Cuadrícula automática", "Auto Grid"),
            (
                crate::db::LayoutMode::SnapToGrid,
                "Alinear a la cuadrícula",
                "Snap to Grid",
            ),
            (crate::db::LayoutMode::Freeform, "Colocación libre", "Freeform"),
        ];
        let layout_labels: Vec<&str> = layout_modes
            .iter()
            .map(|(_, es_l, en_l)| if es { *es_l } else { *en_l })
            .collect();
        let layout_dd = gtk4::DropDown::from_strings(&layout_labels);
        let curr_layout = pane.icon_layout_mode();
        if let Some(pos) = layout_modes.iter().position(|(m, _, _)| *m == curr_layout) {
            layout_dd.set_selected(pos as u32);
        }
        {
            let bw = self.clone_ref();
            let modes: Vec<crate::db::LayoutMode> =
                layout_modes.iter().map(|(m, _, _)| *m).collect();
            layout_dd.connect_selected_notify(move |dd| {
                let idx = dd.selected() as usize;
                if let Some(&mode) = modes.get(idx) {
                    bw.active_pane().set_icon_layout_mode(mode);
                }
            });
        }
        let row_layout = GtkBox::new(Orientation::Horizontal, 8);
        row_layout.append(
            &Label::builder()
                .label(if es { "Disposición:" } else { "Arrange:" })
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-view-options-section"])
                .build(),
        );
        row_layout.append(&layout_dd);
        vbox.append(&row_layout);

        let btn_rearrange = Button::builder()
            .label(if es {
                "Reordenar cuadrícula"
            } else {
                "Clean Up Grid"
            })
            .build();
        {
            let bw = self.clone_ref();
            btn_rearrange.connect_clicked(move |_| {
                bw.active_pane().rearrange_icon_grid();
            });
        }
        vbox.append(&btn_rearrange);
        vbox.append(&gtk4::Separator::new(Orientation::Horizontal));

        // 4. Tamaño de los iconos
        let add_slider_row = |parent: &GtkBox,
                              title: &str,
                              min: f64,
                              max: f64,
                              step: f64,
                              initial: f64,
                              on_change: Box<dyn Fn(u32)>| {
            let section = GtkBox::new(Orientation::Vertical, 2);
            let top = GtkBox::new(Orientation::Horizontal, 6);
            let lbl = Label::builder()
                .label(title)
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-view-options-section"])
                .build();
            let val_lbl = Label::builder()
                .label(format!("{} px", initial.round() as u32))
                .css_classes(["dim-label"])
                .build();
            top.append(&lbl);
            top.append(&val_lbl);

            let scale = gtk4::Scale::with_range(Orientation::Horizontal, min, max, step);
            scale.set_value(initial);
            scale.set_draw_value(false);
            let val_lbl_c = val_lbl.clone();
            scale.connect_value_changed(move |s| {
                let v = s.value().round() as u32;
                val_lbl_c.set_label(&format!("{v} px"));
                on_change(v);
            });
            section.append(&top);
            section.append(&scale);
            parent.append(&section);
        };

        let bw_icon = self.clone_ref();
        add_slider_row(
            &vbox,
            if es { "Tamaño de los íconos" } else { "Icon Size" },
            malus_de_common::config::BranchesConfig::MIN_ICON_SIZE as f64,
            malus_de_common::config::BranchesConfig::MAX_ICON_SIZE as f64,
            4.0,
            cfg.icon_size as f64,
            Box::new(move |v| {
                let mut c = malus_de_common::config::BranchesConfig::load();
                c.icon_size = v;
                let _ = c.save();
                bw_icon.apply_branches_config(&c);
            }),
        );

        let bw_sp = self.clone_ref();
        add_slider_row(
            &vbox,
            if es {
                "Espacio en la cuadrícula"
            } else {
                "Grid Spacing"
            },
            malus_de_common::config::BranchesConfig::MIN_GRID_SPACING as f64,
            malus_de_common::config::BranchesConfig::MAX_GRID_SPACING as f64,
            2.0,
            cfg.grid_spacing as f64,
            Box::new(move |v| {
                let mut c = malus_de_common::config::BranchesConfig::load();
                c.grid_spacing = v;
                let _ = c.save();
                bw_sp.apply_branches_config(&c);
            }),
        );

        let bw_list_sz = self.clone_ref();
        add_slider_row(
            &vbox,
            if es {
                "Tamaño de lista"
            } else {
                "List Icon Size"
            },
            malus_de_common::config::BranchesConfig::MIN_LIST_ICON_SIZE as f64,
            malus_de_common::config::BranchesConfig::MAX_LIST_ICON_SIZE as f64,
            2.0,
            cfg.list_icon_size as f64,
            Box::new(move |v| {
                let mut c = malus_de_common::config::BranchesConfig::load();
                c.list_icon_size = v;
                let _ = c.save();
                bw_list_sz.apply_branches_config(&c);
            }),
        );

        // 5. Mostrar archivos ocultos
        let row_hidden = GtkBox::new(Orientation::Horizontal, 8);
        row_hidden.append(
            &Label::builder()
                .label(if es {
                    "Mostrar archivos ocultos"
                } else {
                    "Show Hidden Files"
                })
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-view-options-section"])
                .build(),
        );
        let sw_hidden = gtk4::Switch::builder()
            .active(pane.show_hidden())
            .valign(Align::Center)
            .build();
        {
            let bw_hid = self.clone_ref();
            sw_hidden.connect_state_set(move |_, state| {
                let mut c = malus_de_common::config::BranchesConfig::load();
                c.show_hidden = state;
                let _ = c.save();
                bw_hid.active_pane().set_show_hidden(state);
                glib::Propagation::Proceed
            });
        }
        row_hidden.append(&sw_hidden);
        vbox.append(&row_hidden);

        win.set_child(Some(&vbox));
        let holder = self.view_options_window.clone();
        win.connect_close_request(move |_| {
            *holder.borrow_mut() = None;
            glib::Propagation::Proceed
        });
        *self.view_options_window.borrow_mut() = Some(win.clone());
        win.present();
    }

    fn prompt_new_folder(&self) {
        self.active_pane().create_new_folder_and_rename();
    }

    fn setup_keybindings(&self) {
        let key_controller = EventControllerKey::new();
        // Fase Capture para interceptar Enter/Return (como en macOS Finder) antes de que
        // un GtkListBox hijo lo consuma como row-activated.
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let bw = self.clone_ref();
        let typeahead = Rc::new(RefCell::new(crate::views::TypeAheadState::new()));

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            let is_alt = state.contains(gdk::ModifierType::ALT_MASK);
            let is_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let is_command =
                state.intersects(gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK);
            let is_shift = state.contains(gdk::ModifierType::SHIFT_MASK);

            if bw.quick_look.is_open() {
                if !is_ctrl
                    && !is_alt
                    && matches!(
                        keyval,
                        gdk::Key::space | gdk::Key::KP_Space | gdk::Key::Escape
                    )
                {
                    bw.quick_look.close();
                    return glib::Propagation::Stop;
                }
                if !is_ctrl && !is_alt && !is_shift && keyval == gdk::Key::Left {
                    bw.quick_look.step_item(-1);
                    return glib::Propagation::Stop;
                }
                if !is_ctrl && !is_alt && !is_shift && keyval == gdk::Key::Right {
                    bw.quick_look.step_item(1);
                    return glib::Propagation::Stop;
                }
                if !is_ctrl && !is_alt && !is_shift && keyval == gdk::Key::Page_Up {
                    if bw.quick_look.step_pdf_page(-1) {
                        return glib::Propagation::Stop;
                    }
                }
                if !is_ctrl && !is_alt && !is_shift && keyval == gdk::Key::Page_Down {
                    if bw.quick_look.step_pdf_page(1) {
                        return glib::Propagation::Stop;
                    }
                }
            }

            if keyval == gdk::Key::Escape && bw.is_search_expanded() {
                bw.collapse_search(true);
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && !is_shift && (keyval == gdk::Key::t || keyval == gdk::Key::T) {
                bw.open_new_tab_matching_active();
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && !is_shift && (keyval == gdk::Key::w || keyval == gdk::Key::W) {
                bw.close_active_tab();
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && (keyval == gdk::Key::Tab || keyval == gdk::Key::ISO_Left_Tab) {
                bw.cycle_tab(if is_shift { -1 } else { 1 });
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && !is_shift && keyval == gdk::Key::Page_Down {
                bw.cycle_tab(1);
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && !is_shift && keyval == gdk::Key::Page_Up {
                bw.cycle_tab(-1);
                return glib::Propagation::Stop;
            }

            if is_ctrl && (keyval == gdk::Key::f || keyval == gdk::Key::F) {
                if bw.is_search_expanded() {
                    bw.collapse_search(true);
                } else {
                    bw.expand_search();
                }
                return glib::Propagation::Stop;
            }

            if (is_ctrl || is_command) && (keyval == gdk::Key::i || keyval == gdk::Key::I) {
                bw.open_get_info(None);
                return glib::Propagation::Stop;
            }

            if (is_ctrl || is_command)
                && !is_alt
                && !is_shift
                && (keyval == gdk::Key::k || keyval == gdk::Key::K)
            {
                let bw_nav = bw.clone_ref();
                crate::network::open_connect_server_dialog(
                    bw.window.upcast_ref::<gtk4::Window>(),
                    move |path| {
                        bw_nav.active_pane().navigate_to(path);
                    },
                );
                return glib::Propagation::Stop;
            }

            if keyval == gdk::Key::F3 {
                bw.toggle_split_view();
                return glib::Propagation::Stop;
            }

            let focus_widget = gtk4::prelude::GtkWindowExt::focus(&bw.window);
            let focus_is_editable = focus_widget
                .as_ref()
                .is_some_and(|w| w.is::<gtk4::Editable>() || w.is::<gtk4::Text>());
            let focus_in_sidebar = focus_widget
                .as_ref()
                .and_then(|w| w.ancestor(gtk4::ListBox::static_type()))
                .is_some_and(|lb| lb.has_css_class("branches-sidebar"));

            let focus_in_header = focus_widget
                .as_ref()
                .and_then(|w| w.ancestor(gtk4::Box::static_type()))
                .is_some_and(|b| b.has_css_class("branches-header-toolbar"));

            // En la vista de rejilla/iconos, las teclas de dirección navegan entre carpetas/archivos
            // en lugar de escapar el foco hacia los botones superiores o de los paneles (accesibles con Tab / Shift+Tab).
            if !focus_is_editable
                && !focus_in_sidebar
                && !focus_in_header
                && !is_alt
                && !is_ctrl
                && !is_shift
                && matches!(
                    keyval,
                    gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down
                )
                && bw.active_pane().navigate_icon_grid(keyval)
            {
                return glib::Propagation::Stop;
            }

            // En macOS Finder, Enter / Return (y F2) activa el renombrado inline del archivo/carpeta
            // seleccionado o del marcador seleccionado en la barra lateral.
            if !focus_is_editable
                && !is_alt
                && !is_ctrl
                && !is_shift
                && matches!(
                    keyval,
                    gdk::Key::F2 | gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
                )
            {
                if focus_in_sidebar {
                    if bw.sidebar.rename_selected_bookmark() {
                        return glib::Propagation::Stop;
                    }
                } else {
                    bw.active_pane().rename_selected();
                    return glib::Propagation::Stop;
                }
            }

            if !focus_is_editable
                && ((is_ctrl && keyval == gdk::Key::Down)
                    || (is_ctrl && (keyval == gdk::Key::o || keyval == gdk::Key::O)))
            {
                bw.active_pane().open_selected();
                return glib::Propagation::Stop;
            }

            if !focus_is_editable
                && !is_ctrl
                && !is_alt
                && !is_command
                && matches!(keyval, gdk::Key::space | gdk::Key::KP_Space)
            {
                if !focus_in_sidebar && !focus_in_header && typeahead.borrow().is_active() {
                    let _ = bw
                        .active_pane()
                        .typeahead_select(' ', &mut typeahead.borrow_mut());
                    return glib::Propagation::Stop;
                }
                bw.toggle_quick_look();
                return glib::Propagation::Stop;
            }

            if is_alt && keyval == gdk::Key::Left {
                bw.active_pane().go_back();
                return glib::Propagation::Stop;
            }
            if is_alt && keyval == gdk::Key::Right {
                bw.active_pane().go_forward();
                return glib::Propagation::Stop;
            }
            if (is_alt && keyval == gdk::Key::Up)
                || (!focus_is_editable && keyval == gdk::Key::BackSpace)
            {
                bw.active_pane().go_up();
                return glib::Propagation::Stop;
            }
            if is_ctrl && (keyval == gdk::Key::h || keyval == gdk::Key::H) {
                bw.active_pane().toggle_hidden();
                return glib::Propagation::Stop;
            }
            if (is_ctrl && (keyval == gdk::Key::r || keyval == gdk::Key::R))
                || keyval == gdk::Key::F5
            {
                bw.active_pane().reload();
                return glib::Propagation::Stop;
            }

            if !focus_is_editable && (is_ctrl || is_command) && !is_alt && !is_shift {
                if keyval == gdk::Key::a || keyval == gdk::Key::A {
                    bw.active_pane().select_all();
                    return glib::Propagation::Stop;
                }
            }

            if !focus_is_editable && (is_ctrl || is_command) && !is_alt && !is_shift {
                if keyval == gdk::Key::c || keyval == gdk::Key::C {
                    bw.copy_selected();
                    return glib::Propagation::Stop;
                }
                if keyval == gdk::Key::x || keyval == gdk::Key::X {
                    bw.cut_selected();
                    return glib::Propagation::Stop;
                }
                if keyval == gdk::Key::v || keyval == gdk::Key::V {
                    bw.paste_into_active_pane();
                    return glib::Propagation::Stop;
                }
                if keyval == gdk::Key::z || keyval == gdk::Key::Z {
                    bw.job_ui.undo_last();
                    return glib::Propagation::Stop;
                }
            }

            if (is_ctrl || is_command)
                && !is_alt
                && !is_shift
                && (keyval == gdk::Key::d || keyval == gdk::Key::D)
            {
                bw.duplicate_selected();
                return glib::Propagation::Stop;
            }
            if !focus_is_editable
                && ((is_shift && keyval == gdk::Key::Delete)
                    || (is_command && is_alt && keyval == gdk::Key::BackSpace))
            {
                bw.confirm_delete_permanent(bw.selected_paths());
                return glib::Propagation::Stop;
            }
            if !focus_is_editable && keyval == gdk::Key::Delete {
                bw.trash_selected();
                return glib::Propagation::Stop;
            }

            if is_ctrl {
                match keyval {
                    gdk::Key::_1 => {
                        bw.active_pane().set_view_mode("icons");
                        bw.btn_icon_view.set_active(true);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::_2 => {
                        bw.active_pane().set_view_mode("list");
                        bw.btn_list_view.set_active(true);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::_3 => {
                        bw.active_pane().set_view_mode("columns");
                        bw.btn_col_view.set_active(true);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::_4 => {
                        bw.active_pane().set_view_mode("gallery");
                        bw.btn_gallery_view.set_active(true);
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            if keyval == gdk::Key::Escape && typeahead.borrow().is_active() {
                typeahead.borrow_mut().clear();
                return glib::Propagation::Stop;
            }

            if !focus_is_editable
                && !focus_in_sidebar
                && !focus_in_header
                && !is_ctrl
                && !is_alt
                && !is_command
            {
                if let Some(ch) = keyval.to_unicode() {
                    if !ch.is_control() && ch != ' ' {
                        if bw
                            .active_pane()
                            .typeahead_select(ch, &mut typeahead.borrow_mut())
                        {
                            return glib::Propagation::Stop;
                        }
                    }
                }
            }

            glib::Propagation::Proceed
        });

        self.window.add_controller(key_controller);
    }

    fn is_search_expanded(&self) -> bool {
        self.search_expanded.get()
    }

    fn animate_search_width(&self, expand: bool) {
        self.search_expanded.set(expand);
        let next_gen = self.search_anim_gen.get().wrapping_add(1);
        self.search_anim_gen.set(next_gen);

        let btn_h = self.search_toggle_btn.height();
        let target_h = if btn_h > 0 {
            btn_h
        } else {
            BRANCHES_SEARCH_HEIGHT
        };
        let btn_w = self.search_toggle_btn.width();
        let collapsed_w = if btn_w > 0 {
            btn_w
        } else {
            BRANCHES_SEARCH_COLLAPSED_WIDTH
        };
        self.search_shell.set_height_request(target_h);
        self.search_entry.set_height_request(target_h);

        let start_p = self.search_anim_progress.get();
        let target_p = if expand { 1.0 } else { 0.0 };
        if (start_p - target_p).abs() < 0.005 {
            self.search_anim_progress.set(target_p);
            if expand {
                self.search_toggle_btn.set_visible(false);
                self.search_shell
                    .set_width_request(BRANCHES_SEARCH_EXPANDED_WIDTH);
                self.search_shell.set_visible(true);
                self.search_entry.set_opacity(1.0);
                self.search_entry.grab_focus();
                if !self.search_entry.text().is_empty() {
                    self.search_entry.select_region(0, -1);
                }
            } else {
                self.search_shell.set_visible(false);
                self.search_shell.set_width_request(collapsed_w);
                self.search_toggle_btn.set_visible(true);
            }
            return;
        }

        if expand {
            self.search_toggle_btn.set_visible(false);
            let initial_w = (collapsed_w as f64
                + (BRANCHES_SEARCH_EXPANDED_WIDTH - collapsed_w) as f64 * start_p)
                .round() as i32;
            self.search_shell.set_width_request(initial_w);
            self.search_entry.set_opacity(start_p);
            self.search_shell.set_visible(true);
            self.search_entry.grab_focus();
            if !self.search_entry.text().is_empty() {
                self.search_entry.select_region(0, -1);
            }
        }

        let started_at = std::time::Instant::now();
        let duration_ms = BRANCHES_SEARCH_ANIM_MS * (target_p - start_p).abs();
        let gen_ref = self.search_anim_gen.clone();
        let prog_ref = self.search_anim_progress.clone();
        let shell = self.search_shell.clone();
        let entry = self.search_entry.clone();
        let btn = self.search_toggle_btn.clone();

        self.search_shell.add_tick_callback(move |_, _| {
            if gen_ref.get() != next_gen {
                return glib::ControlFlow::Break;
            }
            let raw_t = if duration_ms <= 1.0 {
                1.0
            } else {
                (started_at.elapsed().as_secs_f64() * 1000.0 / duration_ms).clamp(0.0, 1.0)
            };
            // Curva cúbica macOS (ease-out al expandir, suave al retraer)
            let eased = 1.0 - (1.0 - raw_t).powi(3);
            let p = start_p + (target_p - start_p) * eased;
            prog_ref.set(p);

            let width = (collapsed_w as f64
                + (BRANCHES_SEARCH_EXPANDED_WIDTH - collapsed_w) as f64 * p)
                .round() as i32;
            shell.set_width_request(width);
            shell.queue_draw();
            entry.set_opacity(((p - 0.12) / 0.88).clamp(0.0, 1.0));

            if raw_t >= 1.0 {
                prog_ref.set(target_p);
                if expand {
                    entry.set_opacity(1.0);
                    shell.set_width_request(BRANCHES_SEARCH_EXPANDED_WIDTH);
                } else {
                    shell.set_visible(false);
                    shell.set_width_request(collapsed_w);
                    btn.set_visible(true);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn expand_search(&self) {
        self.animate_search_width(true);
    }

    fn collapse_search(&self, clear_query: bool) {
        self.animate_search_width(false);
        if clear_query && !self.search_entry.text().is_empty() {
            self.search_entry.set_text("");
        }
    }

    fn clone_ref(&self) -> Rc<Self> {
        Rc::new(Self {
            window: self.window.clone(),
            root_overlay: self.root_overlay.clone(),
            sidebar: self.sidebar.clone(),
            path_bar: self.path_bar.clone(),
            status_bar: self.status_bar.clone(),
            tabs: self.tabs.clone(),
            active_tab_id: self.active_tab_id.clone(),
            next_tab_id: self.next_tab_id.clone(),
            win_weak_holder: self.win_weak_holder.clone(),
            tab_bar_box: self.tab_bar_box.clone(),
            tabs_strip: self.tabs_strip.clone(),
            trash_banner_box: self.trash_banner_box.clone(),
            search_criteria_bar: self.search_criteria_bar.clone(),
            criteria_kind_btns: self.criteria_kind_btns.clone(),
            criteria_recursive_btn: self.criteria_recursive_btn.clone(),
            view_options_window: self.view_options_window.clone(),
            tab_stack: self.tab_stack.clone(),
            split_btn: self.split_btn.clone(),
            back_btn: self.back_btn.clone(),
            forward_btn: self.forward_btn.clone(),
            title_label: self.title_label.clone(),
            btn_icon_view: self.btn_icon_view.clone(),
            btn_list_view: self.btn_list_view.clone(),
            btn_col_view: self.btn_col_view.clone(),
            btn_gallery_view: self.btn_gallery_view.clone(),
            quick_look: self.quick_look.clone(),
            job_ui: self.job_ui.clone(),
            sort_by_action: self.sort_by_action.clone(),
            sort_order_action: self.sort_order_action.clone(),
            search_recursive_action: self.search_recursive_action.clone(),
            search_shell: self.search_shell.clone(),
            search_entry: self.search_entry.clone(),
            search_toggle_btn: self.search_toggle_btn.clone(),
            search_expanded: self.search_expanded.clone(),
            search_anim_progress: self.search_anim_progress.clone(),
            search_anim_gen: self.search_anim_gen.clone(),
            syncing_ui: self.syncing_ui.clone(),
            _config_monitor: self._config_monitor.clone(),
        })
    }
}

pub fn share_paths_via_malus_link(
    parent: Option<&gtk4::Window>,
    app: Option<&gtk4::Application>,
    paths: Vec<PathBuf>,
) {
    let es = malus_de_common::is_spanish_locale();
    let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<(String, String, u32)>, String>>();
    std::thread::spawn(move || {
        let res = (|| -> Result<Vec<(String, String, u32)>, String> {
            let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                .map_err(|e| e.to_string())?;
            let reply = conn
                .call_sync(
                    Some("org.malus_de.Link"),
                    "/org/malus_de/Link",
                    "org.malus_de.Link",
                    "GetConnectedDevices",
                    None,
                    Some(glib::VariantTy::new("(a(ssu))").expect("valid variant")),
                    gio::DBusCallFlags::NONE,
                    1500,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| e.to_string())?;
            let (devices,) = reply
                .get::<(Vec<(String, String, u32)>,)>()
                .ok_or_else(|| "Respuesta inválida de Malus Link".to_string())?;
            Ok(devices)
        })();
        let _ = tx.send(res);
    });

    let parent_opt = parent.cloned();
    let app_opt = app.cloned();
    glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
        match rx.try_recv() {
            Ok(Ok(devices)) => {
                let mut builder = gtk4::Window::builder()
                    .modal(true)
                    .resizable(false)
                    .title("AirDrop / Malus Link")
                    .default_width(360);
                if let Some(ref p) = parent_opt {
                    builder = builder.transient_for(p).destroy_with_parent(true);
                }
                if let Some(ref a) = app_opt {
                    builder = builder.application(a);
                }
                let dialog = builder.build();
                let vbox = GtkBox::new(Orientation::Vertical, 8);
                vbox.set_margin_top(14);
                vbox.set_margin_bottom(14);
                vbox.set_margin_start(16);
                vbox.set_margin_end(16);
                let hdr = Label::builder()
                    .label(if es {
                        "Enviar selección vía AirDrop / Malus Link:"
                    } else {
                        "Send selection via AirDrop / Malus Link:"
                    })
                    .halign(Align::Start)
                    .css_classes(["heading"])
                    .build();
                vbox.append(&hdr);

                let wire_share_btn =
                    |btn: &Button, target_id: String, d_ref: gtk4::Window, p_list: Vec<PathBuf>| {
                        btn.connect_clicked(move |_| {
                            d_ref.close();
                            let id = target_id.clone();
                            let send_paths = p_list.clone();
                            std::thread::spawn(move || {
                                if let Ok(conn) = gio::bus_get_sync(
                                    gio::BusType::Session,
                                    gio::Cancellable::NONE,
                                ) {
                                    for p in send_paths {
                                        let args = (id.clone(), p.to_string_lossy().into_owned())
                                            .to_variant();
                                        let _ = conn.call_sync(
                                            Some("org.malus_de.Link"),
                                            "/org/malus_de/Link",
                                            "org.malus_de.Link",
                                            "ShareFile",
                                            Some(&args),
                                            None,
                                            gio::DBusCallFlags::NONE,
                                            5000,
                                            gio::Cancellable::NONE,
                                        );
                                    }
                                }
                            });
                        });
                    };

                for (dev_id, dev_name, battery) in &devices {
                    let label = if *battery == 0 {
                        format!("📡 {}", dev_name)
                    } else {
                        format!("{} ({}%)", dev_name, battery)
                    };
                    let btn = Button::builder().label(&label).halign(Align::Fill).build();
                    wire_share_btn(&btn, dev_id.clone(), dialog.clone(), paths.clone());
                    vbox.append(&btn);
                }

                let auto_btn = Button::builder()
                    .label(if es {
                        "📡 Enviar al primer dispositivo AirDrop cercano (Mac / iPhone)"
                    } else {
                        "📡 Send to first nearby AirDrop device (Mac / iPhone)"
                    })
                    .halign(Align::Fill)
                    .build();
                wire_share_btn(&auto_btn, "auto".to_string(), dialog.clone(), paths.clone());
                vbox.append(&auto_btn);

                if devices.is_empty() {
                    let hint = Label::builder()
                        .label(if es {
                            "Pon AirDrop en «Todos durante 10 min» en tu MacBook Air o iPhone y pulsa arriba para enviar."
                        } else {
                            "Set AirDrop to Everyone (10 min) on your MacBook Air or iPhone and click above to send."
                        })
                        .wrap(true)
                        .halign(Align::Start)
                        .css_classes(["dim-label"])
                        .build();
                    vbox.append(&hint);
                }

                dialog.set_child(Some(&vbox));
                dialog.present();
                glib::ControlFlow::Break
            }
            Ok(Err(err)) => {
                let mut alert = gtk4::AlertDialog::builder().message(if es {
                    format!("Malus Link no está disponible: {}", err)
                } else {
                    format!("Malus Link is unavailable: {}", err)
                });
                let _ = &mut alert;
                alert.build().show(parent_opt.as_ref());
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_shell_matches_search_button_height() {
        assert_eq!(BRANCHES_SEARCH_HEIGHT, BRANCHES_SEARCH_COLLAPSED_WIDTH);
        assert_eq!(BRANCHES_SEARCH_HEIGHT, 32);
        assert_eq!(BRANCHES_SEARCH_SQUIRCLE_RADIUS, 16.0);

        let css = include_str!("styles/macos-branches.css");
        let btn_block = css
            .split("button.branches-search-btn {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir button.branches-search-btn");
        // 30px contenido + 1px borde superior + 1px borde inferior = 32px totales
        assert!(btn_block.contains("min-height: 30px;"));
        assert!(btn_block.contains("border: 1px solid"));
    }
}
