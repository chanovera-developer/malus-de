use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, MenuButton, Orientation, Popover, Separator};
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{info, warn};

use crate::services::dbusmenu::{DbusMenuCategory, DbusMenuItem, DbusMenuService};

#[derive(Clone)]
pub struct GlobalMenuHandles {
    pub container: GtkBox,
    pub app_title_label: Label,
    pub hide_btn: Button,
    pub quit_btn: Button,
    pub menus_container: GtkBox,
    pub dbus_menu_service: Arc<DbusMenuService>,
    pub current_active: Rc<RefCell<(Option<String>, Option<String>, Option<u64>)>>,
}

#[derive(Clone, Copy)]
pub enum MenuAction {
    HideActiveWindow,
    CloseActiveWindow,
    Command(&'static [&'static str]),
    None,
}

pub struct MenuItemDef {
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub action: MenuAction,
    pub is_separator: bool,
}

impl MenuItemDef {
    pub const fn item(label: &'static str, shortcut: Option<&'static str>, action: MenuAction) -> Self {
        Self {
            label,
            shortcut,
            action,
            is_separator: false,
        }
    }

    pub const fn separator() -> Self {
        Self {
            label: "",
            shortcut: None,
            action: MenuAction::None,
            is_separator: true,
        }
    }
}

pub struct MenuCategoryDef {
    pub title: &'static str,
    pub items: &'static [MenuItemDef],
}

// =========================================================================
// Definiciones auténticas de Menú Global para aplicaciones comunes y macOS
// =========================================================================

// --- 1. Firefox ---
const FIREFOX_MENUS: &[MenuCategoryDef] = &[
    MenuCategoryDef {
        title: "Archivo",
        items: &[
            MenuItemDef::item("Nueva pestaña", Some("⌘T"), MenuAction::Command(&["firefox", "--new-tab", "about:blank"])),
            MenuItemDef::item("Nueva ventana", Some("⌘N"), MenuAction::Command(&["firefox", "--new-window"])),
            MenuItemDef::item("Nueva ventana privada", Some("⇧⌘P"), MenuAction::Command(&["firefox", "--private-window"])),
            MenuItemDef::item("Abrir archivo...", Some("⌘O"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Cerrar pestaña", Some("⌘W"), MenuAction::None),
            MenuItemDef::item("Cerrar ventana", Some("⇧⌘W"), MenuAction::CloseActiveWindow),
            MenuItemDef::item("Guardar página como...", Some("⌘S"), MenuAction::None),
            MenuItemDef::item("Imprimir...", Some("⌘P"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Edición",
        items: &[
            MenuItemDef::item("Deshacer", Some("⌘Z"), MenuAction::None),
            MenuItemDef::item("Rehacer", Some("⇧⌘Z"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Cortar", Some("⌘X"), MenuAction::None),
            MenuItemDef::item("Copiar", Some("⌘C"), MenuAction::None),
            MenuItemDef::item("Pegar", Some("⌘V"), MenuAction::None),
            MenuItemDef::item("Seleccionar todo", Some("⌘A"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Buscar en la página...", Some("⌘F"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ver",
        items: &[
            MenuItemDef::item("Acercar (+)", Some("⌘+"), MenuAction::None),
            MenuItemDef::item("Alejar (-)", Some("⌘-"), MenuAction::None),
            MenuItemDef::item("Tamaño real", Some("⌘0"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Pantalla completa", Some("F11"), MenuAction::None),
            MenuItemDef::item("Barra lateral de marcadores", Some("⌘B"), MenuAction::None),
            MenuItemDef::item("Barra lateral de historial", Some("⌘H"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Historial",
        items: &[
            MenuItemDef::item("Atrás", Some("⌘["), MenuAction::None),
            MenuItemDef::item("Adelante", Some("⌘]"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Pestañas cerradas recientemente", Some("⇧⌘T"), MenuAction::None),
            MenuItemDef::item("Mostrar todo el historial", Some("⇧⌘H"), MenuAction::None),
            MenuItemDef::item("Limpiar el historial reciente...", Some("⇧⌘Supr"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Marcadores",
        items: &[
            MenuItemDef::item("Añadir a marcadores", Some("⌘D"), MenuAction::None),
            MenuItemDef::item("Mostrar todos los marcadores", Some("⌥⌘B"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Herramientas",
        items: &[
            MenuItemDef::item("Descargas", Some("⌘J"), MenuAction::None),
            MenuItemDef::item("Complementos y temas", Some("⇧⌘A"), MenuAction::Command(&["firefox", "about:addons"])),
            MenuItemDef::item("Herramientas para desarrolladores", Some("F12"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ventana",
        items: &[
            MenuItemDef::item("Minimizar", Some("⌘M"), MenuAction::HideActiveWindow),
            MenuItemDef::item("Zoom", None, MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Siguiente pestaña", Some("⌃⇥"), MenuAction::None),
            MenuItemDef::item("Pestaña anterior", Some("⌃⇧⇥"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ayuda",
        items: &[
            MenuItemDef::item("Ayuda de Firefox", Some("F1"), MenuAction::Command(&["firefox", "https://support.mozilla.org"])),
            MenuItemDef::item("Información para solucionar problemas", None, MenuAction::Command(&["firefox", "about:support"])),
            MenuItemDef::separator(),
            MenuItemDef::item("Acerca de Firefox", None, MenuAction::Command(&["firefox", "about:about"])),
        ],
    },
];

// --- 2. Archivos / Nautilus ---
const NAUTILUS_MENUS: &[MenuCategoryDef] = &[
    MenuCategoryDef {
        title: "Archivo",
        items: &[
            MenuItemDef::item("Nueva ventana", Some("⌘N"), MenuAction::Command(&["nautilus", "--new-window"])),
            MenuItemDef::item("Nueva pestaña", Some("⌘T"), MenuAction::None),
            MenuItemDef::item("Nueva carpeta", Some("⇧⌘N"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Propiedades", Some("⌘I"), MenuAction::None),
            MenuItemDef::item("Cerrar ventana", Some("⌘W"), MenuAction::CloseActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Edición",
        items: &[
            MenuItemDef::item("Cortar", Some("⌘X"), MenuAction::None),
            MenuItemDef::item("Copiar", Some("⌘C"), MenuAction::None),
            MenuItemDef::item("Pegar", Some("⌘V"), MenuAction::None),
            MenuItemDef::item("Seleccionar todo", Some("⌘A"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ver",
        items: &[
            MenuItemDef::item("Iconos en cuadrícula", Some("⌘1"), MenuAction::None),
            MenuItemDef::item("Lista detallada", Some("⌘2"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Mostrar archivos ocultos", Some("⌘H"), MenuAction::None),
            MenuItemDef::item("Recargar", Some("⌘R"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ir",
        items: &[
            MenuItemDef::item("Carpeta personal", Some("⌥⌘H"), MenuAction::Command(&["nautilus"])),
            MenuItemDef::item("Descargas", Some("⌥⌘L"), MenuAction::Command(&["nautilus", "Downloads"])),
            MenuItemDef::item("Documentos", Some("⌥⌘D"), MenuAction::Command(&["nautilus", "Documents"])),
            MenuItemDef::item("Papelera", Some("⌥⌘T"), MenuAction::Command(&["nautilus", "trash:///"])),
        ],
    },
    MenuCategoryDef {
        title: "Ventana",
        items: &[
            MenuItemDef::item("Minimizar", Some("⌘M"), MenuAction::HideActiveWindow),
            MenuItemDef::item("Cerrar", Some("⌘W"), MenuAction::CloseActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Ayuda",
        items: &[
            MenuItemDef::item("Ayuda de Archivos", Some("F1"), MenuAction::None),
            MenuItemDef::item("Acerca de Archivos", None, MenuAction::None),
        ],
    },
];

// --- 3. Terminal / Ptyxis ---
const TERMINAL_MENUS: &[MenuCategoryDef] = &[
    MenuCategoryDef {
        title: "Terminal",
        items: &[
            MenuItemDef::item("Nueva ventana", Some("⇧⌘N"), MenuAction::Command(&["ptyxis", "--new-window"])),
            MenuItemDef::item("Nueva pestaña", Some("⇧⌘T"), MenuAction::Command(&["ptyxis", "--tab"])),
            MenuItemDef::separator(),
            MenuItemDef::item("Cerrar pestaña", Some("⇧⌘W"), MenuAction::None),
            MenuItemDef::item("Cerrar ventana", Some("⇧⌘Q"), MenuAction::CloseActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Editar",
        items: &[
            MenuItemDef::item("Copiar", Some("⇧⌘C"), MenuAction::None),
            MenuItemDef::item("Pegar", Some("⇧⌘V"), MenuAction::None),
            MenuItemDef::item("Seleccionar todo", Some("⇧⌘A"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Buscar...", Some("⇧⌘F"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ver",
        items: &[
            MenuItemDef::item("Acercar (+)", Some("⌘+"), MenuAction::None),
            MenuItemDef::item("Alejar (-)", Some("⌘-"), MenuAction::None),
            MenuItemDef::item("Tamaño normal", Some("⌘0"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Pantalla completa", Some("F11"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ventana",
        items: &[
            MenuItemDef::item("Minimizar", Some("⌘M"), MenuAction::HideActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Ayuda",
        items: &[
            MenuItemDef::item("Atajos de teclado", Some("F1"), MenuAction::None),
            MenuItemDef::item("Acerca de Terminal", None, MenuAction::None),
        ],
    },
];

// --- 4. Editor de Texto ---
const TEXT_EDITOR_MENUS: &[MenuCategoryDef] = &[
    MenuCategoryDef {
        title: "Archivo",
        items: &[
            MenuItemDef::item("Nuevo documento", Some("⌘N"), MenuAction::Command(&["gnome-text-editor"])),
            MenuItemDef::item("Abrir...", Some("⌘O"), MenuAction::None),
            MenuItemDef::item("Guardar", Some("⌘S"), MenuAction::None),
            MenuItemDef::item("Guardar como...", Some("⇧⌘S"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Cerrar", Some("⌘W"), MenuAction::CloseActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Editar",
        items: &[
            MenuItemDef::item("Deshacer", Some("⌘Z"), MenuAction::None),
            MenuItemDef::item("Rehacer", Some("⇧⌘Z"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Cortar", Some("⌘X"), MenuAction::None),
            MenuItemDef::item("Copiar", Some("⌘C"), MenuAction::None),
            MenuItemDef::item("Pegar", Some("⌘V"), MenuAction::None),
            MenuItemDef::item("Seleccionar todo", Some("⌘A"), MenuAction::None),
            MenuItemDef::separator(),
            MenuItemDef::item("Buscar y reemplazar", Some("⌘F"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ver",
        items: &[
            MenuItemDef::item("Números de línea", None, MenuAction::None),
            MenuItemDef::item("Ajuste de línea", None, MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ventana",
        items: &[
            MenuItemDef::item("Minimizar", Some("⌘M"), MenuAction::HideActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Ayuda",
        items: &[
            MenuItemDef::item("Acerca de Editor de Texto", None, MenuAction::None),
        ],
    },
];

// --- 5. Calculadora ---
const CALCULATOR_MENUS: &[MenuCategoryDef] = &[
    MenuCategoryDef {
        title: "Modo",
        items: &[
            MenuItemDef::item("Modo Básico", Some("⌘1"), MenuAction::None),
            MenuItemDef::item("Modo Avanzado", Some("⌘2"), MenuAction::None),
            MenuItemDef::item("Modo Financiero", Some("⌘3"), MenuAction::None),
            MenuItemDef::item("Modo Programador", Some("⌘4"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Editar",
        items: &[
            MenuItemDef::item("Copiar resultado", Some("⌘C"), MenuAction::None),
            MenuItemDef::item("Pegar valor", Some("⌘V"), MenuAction::None),
            MenuItemDef::item("Deshacer", Some("⌘Z"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ver",
        items: &[
            MenuItemDef::item("Historial de cálculos", Some("⌘H"), MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ventana",
        items: &[
            MenuItemDef::item("Minimizar", Some("⌘M"), MenuAction::HideActiveWindow),
            MenuItemDef::item("Cerrar", Some("⌘W"), MenuAction::CloseActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Ayuda",
        items: &[
            MenuItemDef::item("Acerca de Calculadora", None, MenuAction::None),
        ],
    },
];

// --- 6. Menú General por defecto (Malus DE) ---
const DEFAULT_MENUS: &[MenuCategoryDef] = &[
    MenuCategoryDef {
        title: "Archivo",
        items: &[
            MenuItemDef::item("Búsqueda Spotlight", Some("⌘Espacio"), MenuAction::Command(&["malus-de-spotlight"])),
            MenuItemDef::item("Bloquear pantalla", Some("⌃⌘L"), MenuAction::Command(&["malus-de-session-lock"])),
        ],
    },
    MenuCategoryDef {
        title: "Edición",
        items: &[
            MenuItemDef::item("Preferencias del Sistema...", Some("⌘,"), MenuAction::Command(&["gnome-control-center"])),
        ],
    },
    MenuCategoryDef {
        title: "Ver",
        items: &[
            MenuItemDef::item("Alternar Modo Oscuro", None, MenuAction::None),
        ],
    },
    MenuCategoryDef {
        title: "Ventana",
        items: &[
            MenuItemDef::item("Minimizar ventana activa", Some("⌘M"), MenuAction::HideActiveWindow),
            MenuItemDef::item("Cerrar ventana activa", Some("⌘W"), MenuAction::CloseActiveWindow),
        ],
    },
    MenuCategoryDef {
        title: "Ayuda",
        items: &[
            MenuItemDef::item("Guía de Malus DE", None, MenuAction::None),
            MenuItemDef::item("Atajos de teclado", None, MenuAction::None),
        ],
    },
];

// =========================================================================
// Construcción de la interfaz de Menú Global
// =========================================================================

pub fn create_global_menu_widget(dbus_menu_service: Arc<DbusMenuService>) -> GlobalMenuHandles {
    let container = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .build();

    // 1. Menú de la Aplicación activa (ej. "Firefox" o "Malus DE")
    let app_title_btn = MenuButton::builder()
        .css_classes(["app-title-btn", "topbar-btn"])
        .has_frame(false)
        .always_show_arrow(false)
        .build();
    let app_title_label = Label::builder()
        .label("Malus DE")
        .css_classes(["active-app-name"])
        .build();
    app_title_btn.set_child(Some(&app_title_label));

    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(gtk4::Align::Start)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let hide_btn = create_menu_row("Ocultar Malus DE", Some("⌘H"));
    let popover_hide = popover.clone();
    hide_btn.connect_clicked(move |_| {
        popover_hide.popdown();
        let _ = malus_de_common::ipc::send_ipc_message(&malus_de_common::ipc::IpcMessage::HideActiveWindow);
    });

    let quit_btn = create_menu_row("Salir de Malus DE", Some("⌘Q"));
    let popover_quit = popover.clone();
    quit_btn.connect_clicked(move |_| {
        popover_quit.popdown();
        let _ = malus_de_common::ipc::send_ipc_message(&malus_de_common::ipc::IpcMessage::CloseActiveWindow);
    });

    content.append(&hide_btn);
    content.append(&create_divider());
    content.append(&quit_btn);
    popover.set_child(Some(&content));
    app_title_btn.set_popover(Some(&popover));

    container.append(&app_title_btn);

    // 2. Contenedor dinámico para los menús de la aplicación
    let menus_container = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .build();
    container.append(&menus_container);

    let current_active = Rc::new(RefCell::new((None, None, None)));

    let handles = GlobalMenuHandles {
        container,
        app_title_label,
        hide_btn,
        quit_btn,
        menus_container,
        dbus_menu_service,
        current_active,
    };

    // Inicializar con el menú por defecto
    update_global_menu(&handles, None, None, None);

    handles
}

pub fn update_global_menu(
    handles: &GlobalMenuHandles,
    app_id: Option<&str>,
    title: Option<&str>,
    window_id: Option<u64>,
) {
    // Actualizar estado activo en handles
    *handles.current_active.borrow_mut() = (
        app_id.map(String::from),
        title.map(String::from),
        window_id,
    );

    // Intentar buscar menú D-Bus registrado dinámico
    if let Some((service, path)) = handles.dbus_menu_service.find_menu(window_id, app_id) {
        info!("🔍 Menú D-Bus encontrado para {:?} (id={:?}): {}:{}", app_id, window_id, service, path);
        let h_clone = handles.clone();
        let srv = service.clone();
        let pth = path.clone();
        let a_id = app_id.map(String::from);
        let t_id = title.map(String::from);

        glib::MainContext::default().spawn_local(async move {
            match DbusMenuService::fetch_layout(&srv, &pth).await {
                Ok(categories) if !categories.is_empty() => {
                    if !is_current_menu(&h_clone, &a_id, &t_id, window_id, &srv, &pth) {
                        return;
                    }
                    info!("✨ Menú dinámico D-Bus renderizado para {} ({} categorías)", srv, categories.len());
                    render_dbus_categories(&h_clone, categories, &srv, &pth);
                }
                Err(err) => {
                    warn!("No se pudo obtener el menú D-Bus de {srv}:{pth}: {err}");
                    if is_current_menu(&h_clone, &a_id, &t_id, window_id, &srv, &pth) {
                        clear_app_menus(&h_clone);
                    }
                }
                Ok(_) => {
                    if is_current_menu(&h_clone, &a_id, &t_id, window_id, &srv, &pth) {
                        clear_app_menus(&h_clone);
                    }
                }
            }
        });
        return;
    }

    if app_id.is_none() && title.is_none() {
        render_static_categories(handles, None, None);
    } else {
        clear_app_menus(handles);
    }
}

fn is_current_menu(
    handles: &GlobalMenuHandles,
    app_id: &Option<String>,
    title: &Option<String>,
    window_id: Option<u64>,
    service: &str,
    path: &str,
) -> bool {
    let active = handles.current_active.borrow();
    if active.0.as_ref() != app_id.as_ref()
        || active.1.as_ref() != title.as_ref()
        || active.2 != window_id
    {
        return false;
    }

    handles
        .dbus_menu_service
        .find_menu(window_id, app_id.as_deref())
        .is_some_and(|menu| menu.0 == service && menu.1 == path)
}

fn clear_app_menus(handles: &GlobalMenuHandles) {
    while let Some(child) = handles.menus_container.first_child() {
        handles.menus_container.remove(&child);
    }
}

fn render_dbus_categories(
    handles: &GlobalMenuHandles,
    categories: Vec<DbusMenuCategory>,
    service: &str,
    path: &str,
) {
    // Limpiar menús previos
    while let Some(child) = handles.menus_container.first_child() {
        handles.menus_container.remove(&child);
    }

    for cat in categories {
        let label = Label::builder()
            .label(&cat.title)
            .css_classes(["global-menu-item"])
            .build();

        let menu_btn = MenuButton::builder()
            .css_classes(["topbar-btn"])
            .has_frame(false)
            .always_show_arrow(false)
            .child(&label)
            .build();

        let popover = Popover::builder()
            .css_classes(["malus-popover-menu"])
            .has_arrow(false)
            .position(gtk4::PositionType::Bottom)
            .halign(gtk4::Align::Start)
            .build();
        popover.set_offset(0, 4);
        crate::ui::theme::register_popover(&popover);

        let menu_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();

        for item in &cat.items {
            append_dbus_menu_item(&menu_box, item, service, path, &popover);
        }

        let cat_id = cat.id;
        let srv = service.to_string();
        let pth = path.to_string();
        popover.connect_show(move |_| {
            let srv_c = srv.clone();
            let pth_c = pth.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(err) = DbusMenuService::about_to_show(&srv_c, &pth_c, cat_id).await {
                    warn!("No se pudo actualizar la categoría D-Bus {cat_id}: {err}");
                }
            });
        });

        popover.set_child(Some(&menu_box));
        menu_btn.set_popover(Some(&popover));
        handles.menus_container.append(&menu_btn);
    }
}

fn append_dbus_menu_item(
    menu_box: &GtkBox,
    item: &DbusMenuItem,
    service: &str,
    path: &str,
    parent_popover: &Popover,
) {
    if item.is_separator {
        menu_box.append(&create_divider());
        return;
    }

    if !item.children.is_empty() {
        // Elemento con submenú en cascada (macOS style)
        let row_btn = Button::builder()
            .css_classes(["popover-menu-item"])
            .has_frame(false)
            .sensitive(item.enabled)
            .build();

        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .build();

        let label = Label::builder()
            .label(&item.label)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["menu-row-label"])
            .build();
        row.append(&label);

        let chevron = Label::builder()
            .label("›")
            .css_classes(["menu-shortcut-hint"])
            .build();
        row.append(&chevron);

        row_btn.set_child(Some(&row));

        let sub_popover = Popover::builder()
            .css_classes(["malus-popover-menu"])
            .has_arrow(false)
            .position(gtk4::PositionType::Right)
            .build();
        sub_popover.set_parent(&row_btn);
        crate::ui::theme::register_popover(&sub_popover);

        let sub_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();

        for child in &item.children {
            append_dbus_menu_item(&sub_box, child, service, path, parent_popover);
        }

        sub_popover.set_child(Some(&sub_box));

        let sp_clone = sub_popover.clone();
        let srv = service.to_string();
        let pth = path.to_string();
        let item_id = item.id;

        row_btn.connect_clicked(move |_| {
            let srv_c = srv.clone();
            let pth_c = pth.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(err) = DbusMenuService::about_to_show(&srv_c, &pth_c, item_id).await {
                    warn!("No se pudo actualizar el submenú D-Bus {item_id}: {err}");
                }
            });
            sp_clone.popup();
        });

        menu_box.append(&row_btn);
        return;
    }

    // Elemento regular de menú
    let btn = create_menu_row(&item.label, item.shortcut.as_deref());
    btn.set_sensitive(item.enabled);

    let pop_c = parent_popover.clone();
    let srv = service.to_string();
    let pth = path.to_string();
    let item_id = item.id;

    btn.connect_clicked(move |_| {
        pop_c.popdown();
        let srv_c = srv.clone();
        let pth_c = pth.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Err(err) = DbusMenuService::trigger_event(&srv_c, &pth_c, item_id).await {
                warn!("No se pudo activar el elemento de menú D-Bus {item_id}: {err}");
            }
        });
    });

    menu_box.append(&btn);
}

fn render_static_categories(
    handles: &GlobalMenuHandles,
    app_id: Option<&str>,
    title: Option<&str>,
) {
    // Determinar conjunto de menús según la aplicación activa
    let categories: &[MenuCategoryDef] = if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.contains("firefox") || lower.contains("mozilla") {
            FIREFOX_MENUS
        } else if lower.contains("nautilus") || lower.contains("archivos") {
            NAUTILUS_MENUS
        } else if lower.contains("ptyxis") || lower.contains("terminal") {
            TERMINAL_MENUS
        } else if lower.contains("texteditor") || lower.contains("text-editor") || lower.contains("gedit") {
            TEXT_EDITOR_MENUS
        } else if lower.contains("calculator") || lower.contains("calculadora") {
            CALCULATOR_MENUS
        } else {
            DEFAULT_MENUS
        }
    } else if let Some(t) = title {
        let lower = t.to_lowercase();
        if lower.contains("firefox") {
            FIREFOX_MENUS
        } else if lower.contains("archivos") || lower.contains("nautilus") {
            NAUTILUS_MENUS
        } else if lower.contains("terminal") || lower.contains("ptyxis") {
            TERMINAL_MENUS
        } else if lower.contains("editor") {
            TEXT_EDITOR_MENUS
        } else if lower.contains("calculadora") {
            CALCULATOR_MENUS
        } else {
            DEFAULT_MENUS
        }
    } else {
        DEFAULT_MENUS
    };

    // Limpiar menús previos
    while let Some(child) = handles.menus_container.first_child() {
        handles.menus_container.remove(&child);
    }

    // Construir menús dinámicos correspondientes a la app activa
    for cat in categories {
        let label = Label::builder()
            .label(cat.title)
            .css_classes(["global-menu-item"])
            .build();

        let menu_btn = MenuButton::builder()
            .css_classes(["topbar-btn"])
            .has_frame(false)
            .always_show_arrow(false)
            .child(&label)
            .build();

        let popover = Popover::builder()
            .css_classes(["malus-popover-menu"])
            .has_arrow(false)
            .position(gtk4::PositionType::Bottom)
            .halign(gtk4::Align::Start)
            .build();
        popover.set_offset(0, 4);
        crate::ui::theme::register_popover(&popover);

        let menu_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();

        for item in cat.items {
            if item.is_separator {
                menu_box.append(&create_divider());
                continue;
            }

            let btn = create_menu_row(item.label, item.shortcut);
            let pop_clone = popover.clone();
            let action = item.action;

            btn.connect_clicked(move |_| {
                pop_clone.popdown();
                match action {
                    MenuAction::HideActiveWindow => {
                        let _ = malus_de_common::ipc::send_ipc_message(&malus_de_common::ipc::IpcMessage::HideActiveWindow);
                    }
                    MenuAction::CloseActiveWindow => {
                        let _ = malus_de_common::ipc::send_ipc_message(&malus_de_common::ipc::IpcMessage::CloseActiveWindow);
                    }
                    MenuAction::Command(args) => {
                        if !args.is_empty() {
                            info!("Ejecutando acción de menú global: {:?}", args);
                            let mut command = Command::new(args[0]);
                            malus_de_common::config::set_cursor_environment(&mut command);
                            if let Ok(mut child) = command.args(&args[1..]).spawn() {
                                std::thread::spawn(move || {
                                    let _ = child.wait();
                                });
                            }
                        }
                    }
                    MenuAction::None => {}
                }
            });

            menu_box.append(&btn);
        }

        popover.set_child(Some(&menu_box));
        menu_btn.set_popover(Some(&popover));
        handles.menus_container.append(&menu_btn);
    }
}

pub fn create_menu_row(label_text: &str, shortcut: Option<&str>) -> Button {
    let btn = Button::builder()
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(16)
        .build();

    let label = Label::builder()
        .label(label_text)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["menu-row-label"])
        .build();
    row.append(&label);

    if let Some(sc) = shortcut {
        let sc_label = Label::builder()
            .label(sc)
            .xalign(1.0)
            .halign(gtk4::Align::End)
            .css_classes(["menu-shortcut-hint"])
            .build();
        row.append(&sc_label);
    }

    btn.set_child(Some(&row));
    btn
}

fn create_divider() -> Separator {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("menu-separator");
    sep
}
