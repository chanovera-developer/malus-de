use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Image, Label, ListBox, ListBoxRow, Orientation, PolicyType, ScrolledWindow};
use std::path::PathBuf;
use std::rc::Rc;

pub struct Sidebar {
    container: ScrolledWindow,
    list_box: ListBox,
}

struct SidebarEntry {
    title: &'static str,
    icon_name: &'static str,
    path: PathBuf,
}

impl Sidebar {
    pub fn new<F: Fn(PathBuf) + 'static, T: Fn(String) + 'static>(
        on_navigate: F,
        on_tag_selected: T,
    ) -> Self {
        let on_navigate = Rc::new(on_navigate);
        let on_tag_selected = Rc::new(on_tag_selected);

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["branches-sidebar"])
            .hexpand(true)
            .vexpand(true)
            .build();

        // 1. Sección Favoritos
        add_section_title(&list_box, "Favoritos");

        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
        let favorites = [
            SidebarEntry {
                title: "AirDrop",
                icon_name: "network-wireless-symbolic",
                path: home.clone(), // En fase posterior: malus-de-link
            },
            SidebarEntry {
                title: "Recientes",
                icon_name: "document-open-recent-symbolic",
                path: home.clone(),
            },
            SidebarEntry {
                title: "Aplicaciones",
                icon_name: "system-software-install-symbolic",
                path: PathBuf::from("/usr/share/applications"),
            },
            SidebarEntry {
                title: "Inicio",
                icon_name: "user-home-symbolic",
                path: home.clone(),
            },
            SidebarEntry {
                title: "Escritorio",
                icon_name: "user-desktop-symbolic",
                path: home.join("Desktop"),
            },
            SidebarEntry {
                title: "Documentos",
                icon_name: "folder-documents-symbolic",
                path: home.join("Documents"),
            },
            SidebarEntry {
                title: "Descargas",
                icon_name: "folder-download-symbolic",
                path: home.join("Downloads"),
            },
            SidebarEntry {
                title: "Imágenes",
                icon_name: "folder-pictures-symbolic",
                path: home.join("Pictures"),
            },
            SidebarEntry {
                title: "Música",
                icon_name: "folder-music-symbolic",
                path: home.join("Music"),
            },
            SidebarEntry {
                title: "Vídeos",
                icon_name: "folder-videos-symbolic",
                path: home.join("Videos"),
            },
            SidebarEntry {
                title: "Papelera",
                icon_name: "user-trash-symbolic",
                path: home.join(".local/share/Trash/files"),
            },
        ];

        for entry in favorites {
            let row = create_sidebar_row(entry.title, entry.icon_name, entry.path.clone());
            let cb = on_navigate.clone();
            let p = entry.path;
            
            // Conectar activación de la fila
            row.connect_activate(move |_| {
                cb(p.clone());
            });

            list_box.append(&row);
        }

        // 2. Sección Ubicaciones
        add_section_title(&list_box, "Ubicaciones");
        let locations = [
            SidebarEntry {
                title: "Este Equipo",
                icon_name: "drive-harddisk-symbolic",
                path: PathBuf::from("/"),
            },
            SidebarEntry {
                title: "Red",
                icon_name: "network-workgroup-symbolic",
                path: PathBuf::from("/mnt"),
            },
        ];

        for entry in locations {
            let row = create_sidebar_row(entry.title, entry.icon_name, entry.path.clone());
            let cb = on_navigate.clone();
            let p = entry.path;

            row.connect_activate(move |_| {
                cb(p.clone());
            });

            list_box.append(&row);
        }

        // 3. Sección Etiquetas (Tags)
        add_section_title(&list_box, "Etiquetas");
        let tags = [
            ("Rojo", "#ff3b30"),
            ("Naranja", "#ff9500"),
            ("Amarillo", "#ffcc00"),
            ("Verde", "#34c759"),
            ("Azul", "#007aff"),
            ("Púrpura", "#af52de"),
            ("Gris", "#8e8e93"),
        ];

        for (name, color_hex) in tags {
            let row = create_tag_row(name, color_hex);
            let tag_cb = on_tag_selected.clone();
            let tag_name = name.to_string();
            row.connect_activate(move |_| {
                tag_cb(tag_name.clone());
            });
            list_box.append(&row);
        }

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .css_classes(["branches-sidebar-scroll"])
            .hexpand(true)
            .vexpand(true)
            .child(&list_box)
            .build();

        Self { container, list_box }
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.container
    }

    pub fn highlight_path(&self, current: &PathBuf) {
        // Recorrer las filas y marcar seleccionada la que coincida
        let mut child = self.list_box.first_child();
        while let Some(widget) = child {
            if let Some(row) = widget.downcast_ref::<ListBoxRow>() {
                if let Some(path_obj) = unsafe { row.data::<PathBuf>("target_path") } {
                    if unsafe { path_obj.as_ref() } == current {
                        self.list_box.select_row(Some(row));
                        return;
                    }
                }
            }
            child = widget.next_sibling();
        }
        self.list_box.unselect_all();
    }
}

fn add_section_title(list_box: &ListBox, title: &str) {
    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .css_classes(["branches-sidebar-section-title"])
        .build();

    let row = ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .child(&label)
        .build();

    list_box.append(&row);
}

fn create_sidebar_row(title: &str, icon_name: &str, path: PathBuf) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

    let icon = Image::from_icon_name(icon_name);
    icon.set_pixel_size(16);
    icon.add_css_class("branches-sidebar-icon");

    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .hexpand(true)
        .build();

    hbox.append(&icon);
    hbox.append(&label);

    let row = ListBoxRow::builder()
        .child(&hbox)
        .css_classes(["branches-sidebar-row"])
        .build();

    unsafe {
        row.set_data("target_path", path);
    }

    row
}

fn create_tag_row(title: &str, color_hex: &str) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

    let dot = Image::builder()
        .css_classes(["branches-tag-dot"])
        .build();
    
    // Aplicar estilo de color al punto
    let provider = gtk4::CssProvider::new();
    let css = format!(".branches-tag-dot-{} {{ background-color: {}; min-width: 10px; min-height: 10px; border-radius: 5px; }}", title, color_hex);
    provider.load_from_string(&css);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    dot.add_css_class(&format!("branches-tag-dot-{}", title));

    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .hexpand(true)
        .build();

    hbox.append(&dot);
    hbox.append(&label);

    ListBoxRow::builder()
        .child(&hbox)
        .css_classes(["branches-sidebar-row"])
        .build()
}
