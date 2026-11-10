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
    Align, Application, ApplicationWindow, Box as GtkBox, Button, EventControllerKey, Label,
    MenuButton, Orientation, PackType, Paned, Revealer, RevealerTransitionType, SearchEntry,
    ToggleButton, WindowControls, WindowHandle,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub struct BranchesWindow {
    window: ApplicationWindow,
    sidebar: Rc<Sidebar>,
    path_bar: Rc<PathBar>,
    status_bar: Rc<StatusBar>,
    primary_pane: Rc<FolderPane>,
    secondary_pane: Rc<FolderPane>,
    split_paned: Paned,
    active_pane_id: Rc<RefCell<PaneId>>,
    split_view_active: Rc<RefCell<bool>>,
    split_btn: Button,
    back_btn: Button,
    forward_btn: Button,
    title_label: Label,
    btn_icon_view: ToggleButton,
    btn_list_view: ToggleButton,
    btn_col_view: ToggleButton,
    btn_gallery_view: ToggleButton,
    quick_look: Rc<QuickLook>,
    search_toggle_btn: ToggleButton,
}

impl BranchesWindow {
    pub fn new(app: &Application, initial_path: PathBuf) -> Rc<Self> {
        let active_pane_id = Rc::new(RefCell::new(PaneId::Primary));
        let split_view_active = Rc::new(RefCell::new(false));

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

        // 1. Botones de navegación [ < ] y [ > ] (Chevron macOS)
        let nav_box = GtkBox::new(Orientation::Horizontal, 2);
        nav_box.set_valign(Align::Center);
        nav_box.add_css_class("linked");
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

        nav_box.append(&back_btn);
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

        // 3. Selector segmentado de 4 modos de vista
        let view_box = GtkBox::new(Orientation::Horizontal, 0);
        view_box.add_css_class("linked");
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

        // 4. Botón de división de pantalla (Split View Dual Pane)
        let split_btn = Button::builder()
            .icon_name("sidebar-show-right-symbolic")
            .tooltip_text("Dividir pantalla (Dual Pane - F3)")
            .valign(Align::Center)
            .css_classes(["branches-tool-btn", "branches-split-btn"])
            .build();

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
        menu_model.append(Some("Nueva Carpeta"), Some("win.new_folder"));
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

        let se_toggle = search_entry.clone();
        let rev_toggle = search_revealer.clone();
        search_toggle_btn.connect_toggled(move |btn| {
            let active = btn.is_active();
            rev_toggle.set_reveal_child(active);
            if active {
                se_toggle.grab_focus();
            } else {
                se_toggle.set_text("");
            }
        });

        let stb_stop = search_toggle_btn.clone();
        search_entry.connect_stop_search(move |_| {
            stb_stop.set_active(false);
        });

        search_container.append(&search_revealer);
        search_container.append(&search_toggle_btn);

        // Contenedor derecho del header (Vistas + Herramientas + Búsqueda)
        let right_header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .valign(Align::Center)
            .halign(Align::End)
            .build();
        right_header_box.append(&view_box);
        right_header_box.append(&split_btn);
        right_header_box.append(&share_menu_btn);
        right_header_box.append(&info_btn);
        right_header_box.append(&action_menu_btn);
        right_header_box.append(&search_container);

        // 9. Paned principal (Sidebar a la izquierda y Contenido a la derecha)
        let hpaned = Paned::builder()
            .orientation(Orientation::Horizontal)
            .position(210)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["branches-main-paned"])
            .build();

        // Barras inferiores
        let status_bar = Rc::new(StatusBar::new());
        let quick_look = QuickLook::new();

        let win_weak_holder: Rc<RefCell<Option<Rc<BranchesWindow>>>> = Rc::new(RefCell::new(None));
        let win_weak_for_nav = win_weak_holder.clone();

        let sidebar = Rc::new(Sidebar::new(
            {
                let w_clone = win_weak_for_nav.clone();
                move |p| {
                    if let Some(w) = w_clone.borrow().as_ref() {
                        w.active_pane().navigate_to(p);
                    }
                }
            },
            {
                let w_clone = win_weak_for_nav.clone();
                move |tag_name| {
                    if let Some(w) = w_clone.borrow().as_ref() {
                        w.active_pane().show_tag_filter(&tag_name);
                    }
                }
            },
        ));

        let path_bar = Rc::new(PathBar::new({
            let w_clone = win_weak_for_nav.clone();
            move |p| {
                if let Some(w) = w_clone.borrow().as_ref() {
                    w.active_pane().navigate_to(p);
                }
            }
        }));

        // Callback cuando se activa un panel
        let w_for_act = win_weak_for_nav.clone();
        let on_activated = Rc::new(move |id: PaneId| {
            if let Some(w) = w_for_act.borrow().as_ref() {
                w.set_active_pane(id);
            }
        });

        // Callback cuando cambia la ruta de un panel
        let w_for_path = win_weak_for_nav.clone();
        let on_path_changed = Rc::new(move |id: PaneId, path: PathBuf| {
            if let Some(w) = w_for_path.borrow().as_ref() {
                if *w.active_pane_id.borrow() == id {
                    w.sync_active_ui(&path);
                }
            }
        });

        // Callback cuando cambia la selección o conteo de un panel
        let w_for_sel = win_weak_for_nav.clone();
        let ql_for_sel = quick_look.clone();
        let on_selection_changed = Rc::new(move |id: PaneId, count: usize, path: PathBuf| {
            if let Some(w) = w_for_sel.borrow().as_ref() {
                if *w.active_pane_id.borrow() == id {
                    let free = get_free_space(&path);
                    w.status_bar.update(count, 0, &free);
                    if ql_for_sel.is_open() {
                        if let Some(item) = w.active_pane().get_selected_item() {
                            ql_for_sel.update_if_open(&item);
                        }
                    }
                }
            }
        });

        // Callback para cerrar split view desde la mini-cabecera del panel secundario
        let w_for_close = win_weak_for_nav.clone();
        let on_close_split = Rc::new(move || {
            if let Some(w) = w_for_close.borrow().as_ref() {
                w.set_split_view(false);
            }
        });

        // ==========================================
        // CREACIÓN DE PANELES (PRIMARY Y SECONDARY)
        // ==========================================
        let primary_pane = FolderPane::new(
            PaneId::Primary,
            initial_path.clone(),
            on_activated.clone(),
            on_path_changed.clone(),
            on_selection_changed.clone(),
            None,
        );

        let secondary_path = initial_path.parent().map(|p| p.to_path_buf()).unwrap_or(initial_path);
        let secondary_pane = FolderPane::new(
            PaneId::Secondary,
            secondary_path,
            on_activated.clone(),
            on_path_changed.clone(),
            on_selection_changed.clone(),
            Some(on_close_split),
        );
        secondary_pane.widget().set_visible(false); // Oculto inicialmente

        // Paned para dividir entre panel primario y secundario (Split View)
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
        right_pane.append(&split_paned);
        right_pane.append(path_bar.widget());
        right_pane.append(status_bar.widget());

        hpaned.set_start_child(Some(&left_pane));
        hpaned.set_end_child(Some(&right_pane));

        window.set_child(Some(&hpaned));

        let branches_window = Rc::new(Self {
            window,
            sidebar,
            path_bar,
            status_bar,
            primary_pane,
            secondary_pane,
            split_paned,
            active_pane_id,
            split_view_active,
            split_btn,
            back_btn,
            forward_btn,
            title_label,
            btn_icon_view,
            btn_list_view,
            btn_col_view,
            btn_gallery_view,
            quick_look,
            search_toggle_btn,
        });

        *win_weak_holder.borrow_mut() = Some(branches_window.clone());

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
            if btn.is_active() {
                bw_v1.active_pane().set_view_mode("icons");
            }
        });

        let bw_v2 = branches_window.clone();
        branches_window.btn_list_view.connect_toggled(move |btn| {
            if btn.is_active() {
                bw_v2.active_pane().set_view_mode("list");
            }
        });

        let bw_v3 = branches_window.clone();
        branches_window.btn_col_view.connect_toggled(move |btn| {
            if btn.is_active() {
                bw_v3.active_pane().set_view_mode("columns");
            }
        });

        let bw_v4 = branches_window.clone();
        branches_window.btn_gallery_view.connect_toggled(move |btn| {
            if btn.is_active() {
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

        // Conectar campo de búsqueda
        let bw_search = branches_window.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            bw_search.active_pane().set_search_query(query);
        });

        // Conectar acciones del menú
        branches_window.setup_actions();

        // Conectar atajos de teclado
        branches_window.setup_keybindings();

        // Sincronizar UI con el panel inicial
        let curr = branches_window.primary_pane.current_path();
        branches_window.sync_active_ui(&curr);

        branches_window
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn active_pane(&self) -> Rc<FolderPane> {
        if *self.active_pane_id.borrow() == PaneId::Secondary {
            self.secondary_pane.clone()
        } else {
            self.primary_pane.clone()
        }
    }

    pub fn set_active_pane(&self, id: PaneId) {
        *self.active_pane_id.borrow_mut() = id;
        self.primary_pane.set_is_active(id == PaneId::Primary);
        self.secondary_pane.set_is_active(id == PaneId::Secondary);

        let active = self.active_pane();
        let curr = active.current_path();
        self.sync_active_ui(&curr);
    }

    pub fn toggle_split_view(&self) {
        let current = *self.split_view_active.borrow();
        self.set_split_view(!current);
    }

    pub fn toggle_quick_look(&self) {
        let item = self.active_pane().get_selected_item();
        self.quick_look.toggle(&self.window, item);
    }

    pub fn set_split_view(&self, active: bool) {
        *self.split_view_active.borrow_mut() = active;

        self.secondary_pane.widget().set_visible(active);
        self.primary_pane.set_split_mode_active(active);
        self.secondary_pane.set_split_mode_active(active);

        if active {
            self.split_btn.add_css_class("active");
            self.split_paned.set_position(430);
            self.secondary_pane.reload();
        } else {
            self.split_btn.remove_css_class("active");
            self.set_active_pane(PaneId::Primary);
        }
    }

    pub fn sync_active_ui(&self, path: &PathBuf) {
        let active = self.active_pane();

        self.back_btn.set_sensitive(active.can_go_back());
        self.forward_btn.set_sensitive(active.can_go_forward());

        let folder_name = if path == &PathBuf::from("/") {
            "Macintosh HD".to_string()
        } else {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Branches".to_string())
        };
        self.title_label.set_label(&folder_name);

        self.path_bar.update_path(path);
        self.sidebar.highlight_path(path);

        let free = get_free_space(path);
        self.status_bar.update(active.items_count(), 0, &free);
    }

    fn setup_actions(&self) {
        let actions = gio::SimpleActionGroup::new();

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
        let current_path = self.active_pane().current_path();
        let mut target = current_path.join("Nueva carpeta");
        let mut counter = 2;
        while target.exists() {
            target = current_path.join(format!("Nueva carpeta {}", counter));
            counter += 1;
        }

        if let Err(e) = std::fs::create_dir(&target) {
            tracing::error!("Error al crear carpeta: {}", e);
        } else {
            self.active_pane().reload();
        }
    }

    fn setup_keybindings(&self) {
        let key_controller = EventControllerKey::new();
        let bw = self.clone_ref();

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            let is_alt = state.contains(gdk::ModifierType::ALT_MASK);
            let is_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);

            if keyval == gdk::Key::Escape && bw.search_toggle_btn.is_active() {
                bw.search_toggle_btn.set_active(false);
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

            if keyval == gdk::Key::space {
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
            if (is_alt && keyval == gdk::Key::Up) || keyval == gdk::Key::BackSpace {
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

            if is_ctrl && (keyval == gdk::Key::d || keyval == gdk::Key::D) {
                bw.active_pane().duplicate_selected();
                return glib::Propagation::Stop;
            }
            if keyval == gdk::Key::Delete {
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
            primary_pane: self.primary_pane.clone(),
            secondary_pane: self.secondary_pane.clone(),
            split_paned: self.split_paned.clone(),
            active_pane_id: self.active_pane_id.clone(),
            split_view_active: self.split_view_active.clone(),
            split_btn: self.split_btn.clone(),
            back_btn: self.back_btn.clone(),
            forward_btn: self.forward_btn.clone(),
            title_label: self.title_label.clone(),
            btn_icon_view: self.btn_icon_view.clone(),
            btn_list_view: self.btn_list_view.clone(),
            btn_col_view: self.btn_col_view.clone(),
            btn_gallery_view: self.btn_gallery_view.clone(),
            quick_look: self.quick_look.clone(),
            search_toggle_btn: self.search_toggle_btn.clone(),
        })
    }
}
