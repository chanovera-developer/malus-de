use crate::folder_pane::{FolderPane, PaneId};
use crate::fs_backend::get_free_space;
use crate::path_bar::PathBar;
use crate::quick_look::QuickLook;
use crate::sidebar::Sidebar;
use crate::status_bar::StatusBar;
use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, EventControllerKey,
    GestureClick, Label, MenuButton, Orientation, PackType, Paned, Revealer,
    RevealerTransitionType, SearchEntry, Stack, StackTransitionType, ToggleButton, WindowControls,
    WindowHandle,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

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

pub struct BranchesWindow {
    window: ApplicationWindow,
    sidebar: Rc<Sidebar>,
    path_bar: Rc<PathBar>,
    status_bar: Rc<StatusBar>,
    tabs: Rc<RefCell<Vec<BranchesTab>>>,
    active_tab_id: Rc<RefCell<u64>>,
    next_tab_id: Rc<Cell<u64>>,
    win_weak_holder: Rc<RefCell<Weak<BranchesWindow>>>,
    tab_bar_box: GtkBox,
    tabs_strip: GtkBox,
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
    search_entry: SearchEntry,
    search_toggle_btn: ToggleButton,
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

        let share_model = gio::Menu::new();
        share_model.append(Some("Copiar ruta al portapapeles"), Some("win.copy_path"));
        share_model.append(Some("KDE Connect / Enviar a dispositivo"), Some("win.share_kdeconnect"));
        share_model.append(Some("Enviar por correo electrónico"), Some("win.share_email"));
        share_menu_btn.set_menu_model(Some(&share_model));

        // 6. Botón de Información / Quick Look (Inspector "i")
        let info_btn = Button::builder()
            .icon_name("dialog-information-symbolic")
            .tooltip_text("Obtener información / Vista Rápida (Espacio / Ctrl+I)")
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
        
        let menu_model = gio::Menu::new();
        menu_model.append(Some("Nueva Pestaña"), Some("win.new_tab"));
        menu_model.append(Some("Nueva Carpeta"), Some("win.new_folder"));
        menu_model.append(Some("Copiar"), Some("win.copy"));
        menu_model.append(Some("Cortar"), Some("win.cut"));
        menu_model.append(Some("Pegar"), Some("win.paste"));
        menu_model.append(Some("Duplicar"), Some("win.duplicate"));
        menu_model.append(Some("Mover a la Papelera"), Some("win.trash"));
        menu_model.append(Some("Abrir en Terminal"), Some("win.open_terminal"));
        menu_model.append(Some("Mostrar Archivos Ocultos"), Some("win.toggle_hidden"));
        menu_model.append(Some("Recargar"), Some("win.reload"));

        let tag_menu = gio::Menu::new();
        tag_menu.append(Some("Rojo"), Some("win.tag_red"));
        tag_menu.append(Some("Verde"), Some("win.tag_green"));
        tag_menu.append(Some("Azul"), Some("win.tag_blue"));
        menu_model.append_submenu(Some("Etiquetar"), &tag_menu);

        action_menu_btn.set_menu_model(Some(&menu_model));

        // 8. Búsqueda expandible macOS (Icono de lupa a la derecha que revela el campo)
        let search_container = GtkBox::new(Orientation::Horizontal, 4);
        search_container.set_valign(Align::Center);
        search_container.add_css_class("branches-search-container");

        let search_entry = SearchEntry::builder()
            .placeholder_text("Buscar...")
            .width_chars(16)
            .valign(Align::Center)
            .css_classes(["branches-search-entry"])
            .build();

        let search_revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideLeft)
            .transition_duration(180)
            .reveal_child(false)
            .child(&search_entry)
            .build();

        let search_toggle_btn = ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Buscar archivos (Ctrl+F)")
            .valign(Align::Center)
            .css_classes(["branches-tool-btn", "branches-search-btn"])
            .build();

        let stb_stop = search_toggle_btn.clone();
        search_entry.connect_stop_search(move |_| {
            stb_stop.set_active(false);
        });

        search_container.append(&search_revealer);
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
        right_header_box.append(&split_pill_box);
        right_header_box.append(&actions_pill_box);
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

        let win_weak_holder: Rc<RefCell<Weak<BranchesWindow>>> = Rc::new(RefCell::new(Weak::new()));
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
                        w.active_pane().show_tag_filter(&tag_name);
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
            .spacing(4)
            .css_classes(["branches-tab-bar"])
            .hexpand(true)
            .visible(false)
            .build();

        let tabs_strip = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .homogeneous(true)
            .hexpand(true)
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

        let traffic_handle = WindowHandle::builder()
            .child(&controls_box)
            .build();
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
        right_pane.append(&header_handle);
        right_pane.append(&tab_bar_box);
        right_pane.append(&tab_stack);
        right_pane.append(path_bar.widget());
        right_pane.append(status_bar.widget());

        hpaned.set_start_child(Some(&left_pane));
        hpaned.set_end_child(Some(&right_pane));

        window.set_child(Some(&hpaned));

        let config_file = gio::File::for_path(malus_de_common::config::BranchesConfig::config_path());
        let config_monitor = config_file
            .monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            .ok();

        let branches_window = Rc::new(Self {
            window,
            sidebar,
            path_bar,
            status_bar,
            tabs: Rc::new(RefCell::new(Vec::new())),
            active_tab_id: Rc::new(RefCell::new(0)),
            next_tab_id: Rc::new(Cell::new(1)),
            win_weak_holder: win_weak_holder.clone(),
            tab_bar_box,
            tabs_strip,
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
            search_entry: search_entry.clone(),
            search_toggle_btn,
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

        // Conectar botones de navegación
        let bw_back = branches_window.clone();
        branches_window.back_btn.connect_clicked(move |_| {
            bw_back.active_pane().go_back();
        });

        let bw_fwd = branches_window.clone();
        branches_window.forward_btn.connect_clicked(move |_| {
            bw_fwd.active_pane().go_forward();
        });

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
        branches_window.btn_gallery_view.connect_toggled(move |btn| {
            if !bw_v4.syncing_ui.get() && btn.is_active() {
                bw_v4.active_pane().set_view_mode("gallery");
            }
        });

        // Conectar Split View toggle
        let bw_split = branches_window.clone();
        branches_window.split_btn.connect_clicked(move |_| {
            bw_split.toggle_split_view();
        });

        // Conectar botón de Información / Quick Look
        let bw_info = branches_window.clone();
        info_btn.connect_clicked(move |_| {
            bw_info.toggle_quick_look();
        });

        // Conectar campo y botón de búsqueda
        let bw_search_toggle = branches_window.clone();
        let se_toggle = search_entry.clone();
        let rev_toggle = search_revealer.clone();
        branches_window.search_toggle_btn.connect_toggled(move |btn| {
            let active = btn.is_active();
            rev_toggle.set_reveal_child(active);
            if bw_search_toggle.syncing_ui.get() {
                return;
            }
            if active {
                se_toggle.grab_focus();
            } else {
                se_toggle.set_text("");
            }
        });

        let bw_search = branches_window.clone();
        search_entry.connect_search_changed(move |entry| {
            if !bw_search.syncing_ui.get() {
                let query = entry.text().to_string();
                bw_search.active_pane().set_search_query(query);
            }
        });

        // Conectar acciones del menú
        branches_window.setup_actions();

        // Conectar atajos de teclado
        branches_window.setup_keybindings();

        // Crear la primera pestaña con la ruta inicial
        branches_window.open_new_tab(Some(initial_path));

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
            Rc::new(move |id: PaneId, count: usize, path: PathBuf| {
                if let Some(w) = w_for_sel.borrow().upgrade() {
                    if *w.active_tab_id.borrow() == tab_id && *tab_pane_id.borrow() == id {
                        let free = get_free_space(&path);
                        w.status_bar.update(count, 0, &free);
                        if ql_for_sel.is_open() {
                            if let Some(item) = w.active_pane().get_selected_item() {
                                ql_for_sel.update_if_open(&item);
                            }
                        }
                    }
                }
            })
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
            .css_classes(["branches-tab"])
            .build();

        let tab_title = Self::pane_display_title(&primary_pane);
        let tab_title_label = Label::builder()
            .label(&tab_title)
            .halign(Align::Center)
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

        tab_widget.append(&tab_title_label);
        tab_widget.append(&close_btn);

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

        self.tab_stack.set_visible_child_name(&format!("tab-{tab_id}"));
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
        let query = pane.search_query();
        self.search_entry.set_text(&query);
        self.search_toggle_btn.set_active(!query.is_empty());
        self.syncing_ui.set(false);

        if pane.is_recents() {
            let title = Self::pane_display_title(&pane);
            self.title_label.set_label(&title);
            self.sidebar.highlight_virtual("target_recents");
            self.back_btn.set_sensitive(pane.can_go_back());
            self.forward_btn.set_sensitive(pane.can_go_forward());
            let free = get_free_space(&pane.current_path());
            self.status_bar.update(pane.items_count(), 0, &free);
        } else if pane.is_applications() {
            let title = Self::pane_display_title(&pane);
            self.title_label.set_label(&title);
            self.sidebar.highlight_virtual("target_applications");
            self.back_btn.set_sensitive(pane.can_go_back());
            self.forward_btn.set_sensitive(pane.can_go_forward());
            let free = get_free_space(&pane.current_path());
            self.status_bar.update(pane.items_count(), 0, &free);
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
        let item = self.active_pane().get_selected_item();
        self.quick_look.toggle(&self.window, item);
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
        self.title_label.set_label(title);
        self.sidebar.highlight_virtual("target_applications");
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

        self.path_bar.update_path(path);
        self.sidebar.highlight_path(path);

        let free = get_free_space(path);
        self.status_bar.update(active.items_count(), 0, &free);
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

    fn copy_or_cut_item_with_selection(
        &self,
        action: crate::operations::ClipboardAction,
        clicked_path: PathBuf,
    ) {
        let selected: Vec<PathBuf> = self
            .active_pane()
            .get_selected_items()
            .into_iter()
            .map(|i| i.path)
            .collect();
        let paths = if selected.iter().any(|p| p == &clicked_path) && !selected.is_empty() {
            selected
        } else {
            vec![clicked_path]
        };
        crate::operations::set_clipboard(action, paths);
        self.refresh_all_cut_states();
    }

    fn paste_into_folder(&self, dest_dir: PathBuf) {
        let bw = self.clone_ref();
        crate::operations::paste_from_clipboard_async(dest_dir, move |changed| {
            if changed {
                bw.reload_all_panes();
            }
            bw.refresh_all_cut_states();
        });
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
        let act_open_tab =
            gio::SimpleAction::new("open_in_new_tab", Some(glib::VariantTy::STRING));
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
            bw_dup.active_pane().duplicate_selected();
        });
        actions.add_action(&act_dup);

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
        let act_copy_item =
            gio::SimpleAction::new("copy_item", Some(glib::VariantTy::STRING));
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
        let act_cut_item =
            gio::SimpleAction::new("cut_item", Some(glib::VariantTy::STRING));
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
        let act_paste_into =
            gio::SimpleAction::new("paste_into", Some(glib::VariantTy::STRING));
        act_paste_into.connect_activate(move |_, param| {
            if let Some(target) = param.and_then(|v| v.str()) {
                bw_paste_into.paste_into_folder(PathBuf::from(target));
            }
        });
        actions.add_action(&act_paste_into);

        let bw_trash = self.clone_ref();
        let act_trash = gio::SimpleAction::new("trash", None);
        act_trash.connect_activate(move |_, _| {
            bw_trash.active_pane().trash_selected();
        });
        actions.add_action(&act_trash);

        let bw_tag_r = self.clone_ref();
        let act_tag_r = gio::SimpleAction::new("tag_red", None);
        act_tag_r.connect_activate(move |_, _| {
            bw_tag_r.active_pane().set_tag_on_selected("Rojo", "#ff3b30");
        });
        actions.add_action(&act_tag_r);

        let bw_tag_g = self.clone_ref();
        let act_tag_g = gio::SimpleAction::new("tag_green", None);
        act_tag_g.connect_activate(move |_, _| {
            bw_tag_g.active_pane().set_tag_on_selected("Verde", "#34c759");
        });
        actions.add_action(&act_tag_g);

        let bw_tag_b = self.clone_ref();
        let act_tag_b = gio::SimpleAction::new("tag_blue", None);
        act_tag_b.connect_activate(move |_, _| {
            bw_tag_b.active_pane().set_tag_on_selected("Azul", "#007aff");
        });
        actions.add_action(&act_tag_b);

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
            let mailto = format!("mailto:?subject=Compartido desde Malus DE&body={}", path.to_string_lossy());
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(mailto);
            malus_de_common::config::apply_isolated_environment(&mut cmd);
            let _ = cmd.spawn();
        });
        actions.add_action(&act_email);

        self.window.insert_action_group("win", Some(&actions));
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

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            let is_alt = state.contains(gdk::ModifierType::ALT_MASK);
            let is_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let is_command = state.intersects(
                gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK,
            );
            let is_shift = state.contains(gdk::ModifierType::SHIFT_MASK);

            if keyval == gdk::Key::Escape && bw.search_toggle_btn.is_active() {
                bw.search_toggle_btn.set_active(false);
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && !is_shift && (keyval == gdk::Key::t || keyval == gdk::Key::T)
            {
                bw.open_new_tab_matching_active();
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && !is_shift && (keyval == gdk::Key::w || keyval == gdk::Key::W)
            {
                bw.close_active_tab();
                return glib::Propagation::Stop;
            }

            if is_ctrl && !is_alt && (keyval == gdk::Key::Tab || keyval == gdk::Key::ISO_Left_Tab)
            {
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
                let active = !bw.search_toggle_btn.is_active();
                bw.search_toggle_btn.set_active(active);
                return glib::Propagation::Stop;
            }

            if is_ctrl && (keyval == gdk::Key::i || keyval == gdk::Key::I) {
                bw.toggle_quick_look();
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
                    gdk::Key::F2
                        | gdk::Key::Return
                        | gdk::Key::KP_Enter
                        | gdk::Key::ISO_Enter
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

            if !focus_is_editable && keyval == gdk::Key::space {
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
            if (is_alt && keyval == gdk::Key::Up) || (!focus_is_editable && keyval == gdk::Key::BackSpace) {
                bw.active_pane().go_up();
                return glib::Propagation::Stop;
            }
            if is_ctrl && (keyval == gdk::Key::h || keyval == gdk::Key::H) {
                bw.active_pane().toggle_hidden();
                return glib::Propagation::Stop;
            }
            if (is_ctrl && (keyval == gdk::Key::r || keyval == gdk::Key::R)) || keyval == gdk::Key::F5 {
                bw.active_pane().reload();
                return glib::Propagation::Stop;
            }

            if !focus_is_editable && is_ctrl && !is_alt && !is_shift {
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
            }

            if is_ctrl && (keyval == gdk::Key::d || keyval == gdk::Key::D) {
                bw.active_pane().duplicate_selected();
                return glib::Propagation::Stop;
            }
            if !focus_is_editable && keyval == gdk::Key::Delete {
                bw.active_pane().trash_selected();
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

            glib::Propagation::Proceed
        });

        self.window.add_controller(key_controller);
    }

    fn clone_ref(&self) -> Rc<Self> {
        Rc::new(Self {
            window: self.window.clone(),
            sidebar: self.sidebar.clone(),
            path_bar: self.path_bar.clone(),
            status_bar: self.status_bar.clone(),
            tabs: self.tabs.clone(),
            active_tab_id: self.active_tab_id.clone(),
            next_tab_id: self.next_tab_id.clone(),
            win_weak_holder: self.win_weak_holder.clone(),
            tab_bar_box: self.tab_bar_box.clone(),
            tabs_strip: self.tabs_strip.clone(),
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
            search_entry: self.search_entry.clone(),
            search_toggle_btn: self.search_toggle_btn.clone(),
            syncing_ui: self.syncing_ui.clone(),
            _config_monitor: self._config_monitor.clone(),
        })
    }
}
