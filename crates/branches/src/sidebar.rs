use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, GestureClick, Image, Label, ListBox, ListBoxRow, Orientation, PolicyType, ScrolledWindow};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Sidebar {
    container: ScrolledWindow,
    list_box: ListBox,
}

struct SidebarEntry {
    title: String,
    icon_name: &'static str,
    path: PathBuf,
}

fn resolve_user_dir(home: &Path, xdg_key: &str, spanish_name: &str, english_name: &str) -> PathBuf {
    let config_file = home.join(".config/user-dirs.dirs");
    if let Ok(content) = std::fs::read_to_string(&config_file) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(xdg_key) && trimmed.contains('=') {
                if let Some((_, raw_val)) = trimmed.split_once('=') {
                    let val = raw_val.trim().trim_matches('"');
                    let expanded = val.replace("$HOME", &home.to_string_lossy());
                    let p = PathBuf::from(expanded);
                    return p;
                }
            }
        }
    }

    let es_dir = home.join(spanish_name);
    if es_dir.exists() {
        return es_dir;
    }
    let en_dir = home.join(english_name);
    if en_dir.exists() {
        return en_dir;
    }

    if malus_de_common::is_spanish_locale() {
        es_dir
    } else {
        en_dir
    }
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
            .activate_on_single_click(true)
            .css_classes(["branches-sidebar"])
            .hexpand(true)
            .vexpand(true)
            .build();

        let es = malus_de_common::is_spanish_locale();

        // 1. Sección Favoritos
        add_section_title(&list_box, if es { "Favoritos" } else { "Favorites" });

        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
        let desktop_path = resolve_user_dir(&home, "XDG_DESKTOP_DIR", "Escritorio", "Desktop");
        let docs_path = resolve_user_dir(&home, "XDG_DOCUMENTS_DIR", "Documentos", "Documents");
        let downloads_path = resolve_user_dir(&home, "XDG_DOWNLOAD_DIR", "Descargas", "Downloads");
        let pictures_path = resolve_user_dir(&home, "XDG_PICTURES_DIR", "Imágenes", "Pictures");
        let music_path = resolve_user_dir(&home, "XDG_MUSIC_DIR", "Música", "Music");
        let videos_path = resolve_user_dir(&home, "XDG_VIDEOS_DIR", "Vídeos", "Videos");
        let projects_path = resolve_user_dir(&home, "XDG_PROJECTS_DIR", "Proyectos", "Projects");
        let trash_path = home.join(".local/share/Trash/files");
        let apps_path = PathBuf::from("/usr/share/applications");

        let favorites = [
            SidebarEntry {
                title: if es { "Aplicaciones".to_string() } else { "Applications".to_string() },
                icon_name: "system-software-install-symbolic",
                path: apps_path,
            },
            SidebarEntry {
                title: if es { "Inicio".to_string() } else { "Home".to_string() },
                icon_name: "user-home-symbolic",
                path: home.clone(),
            },
            SidebarEntry {
                title: if es { "Escritorio".to_string() } else { "Desktop".to_string() },
                icon_name: "user-desktop-symbolic",
                path: desktop_path,
            },
            SidebarEntry {
                title: if es { "Documentos".to_string() } else { "Documents".to_string() },
                icon_name: "folder-documents-symbolic",
                path: docs_path,
            },
            SidebarEntry {
                title: if es { "Descargas".to_string() } else { "Downloads".to_string() },
                icon_name: "folder-download-symbolic",
                path: downloads_path,
            },
            SidebarEntry {
                title: if es { "Imágenes".to_string() } else { "Pictures".to_string() },
                icon_name: "folder-pictures-symbolic",
                path: pictures_path,
            },
            SidebarEntry {
                title: if es { "Música".to_string() } else { "Music".to_string() },
                icon_name: "folder-music-symbolic",
                path: music_path,
            },
            SidebarEntry {
                title: if es { "Vídeos".to_string() } else { "Videos".to_string() },
                icon_name: "folder-videos-symbolic",
                path: videos_path,
            },
            SidebarEntry {
                title: if es { "Proyectos".to_string() } else { "Projects".to_string() },
                icon_name: "folder-projects-symbolic",
                path: projects_path,
            },
            SidebarEntry {
                title: if es { "Papelera".to_string() } else { "Trash".to_string() },
                icon_name: "user-trash-symbolic",
                path: trash_path,
            },
        ];

        for entry in favorites {
            let row = create_sidebar_row(&entry.title, entry.icon_name, entry.path.clone());
            let cb = on_navigate.clone();
            let p = entry.path;

            // Controlador de clic directo de ratón
            let click = GestureClick::new();
            let cb_c = cb.clone();
            let p_c = p.clone();
            click.connect_released(move |_, _, _, _| {
                cb_c(p_c.clone());
            });
            row.add_controller(click);

            // Controlador de teclado (Enter)
            row.connect_activate(move |_| {
                cb(p.clone());
            });

            list_box.append(&row);
        }

        // 2. Sección Ubicaciones
        add_section_title(&list_box, if es { "Ubicaciones" } else { "Locations" });
        let locations = [
            SidebarEntry {
                title: if es { "Este Equipo".to_string() } else { "This Computer".to_string() },
                icon_name: "drive-harddisk-symbolic",
                path: PathBuf::from("/"),
            },
            SidebarEntry {
                title: if es { "Red".to_string() } else { "Network".to_string() },
                icon_name: "network-workgroup-symbolic",
                path: PathBuf::from("/mnt"),
            },
        ];

        for entry in locations {
            let row = create_sidebar_row(&entry.title, entry.icon_name, entry.path.clone());
            let cb = on_navigate.clone();
            let p = entry.path;

            let click = GestureClick::new();
            let cb_c = cb.clone();
            let p_c = p.clone();
            click.connect_released(move |_, _, _, _| {
                cb_c(p_c.clone());
            });
            row.add_controller(click);

            row.connect_activate(move |_| {
                cb(p.clone());
            });

            list_box.append(&row);
        }

        // 3. Sección Etiquetas (Tags)
        add_section_title(&list_box, if es { "Etiquetas" } else { "Tags" });
        let tags = [
            (if es { "Rojo" } else { "Red" }, "#ff3b30"),
            (if es { "Naranja" } else { "Orange" }, "#ff9500"),
            (if es { "Amarillo" } else { "Yellow" }, "#ffcc00"),
            (if es { "Verde" } else { "Green" }, "#34c759"),
            (if es { "Azul" } else { "Blue" }, "#007aff"),
            (if es { "Púrpura" } else { "Purple" }, "#af52de"),
            (if es { "Gris" } else { "Gray" }, "#8e8e93"),
        ];

        for (name, color_hex) in tags {
            let row = create_tag_row(name, color_hex);
            let tag_cb = on_tag_selected.clone();
            let tag_name = name.to_string();

            let click = GestureClick::new();
            let cb_c = tag_cb.clone();
            let t_c = tag_name.clone();
            click.connect_released(move |_, _, _, _| {
                cb_c(t_c.clone());
            });
            row.add_controller(click);

            let t_a = tag_name.clone();
            row.connect_activate(move |_| {
                tag_cb(t_a.clone());
            });
            list_box.append(&row);
        }

        // Manejador centralizado de activación de filas del ListBox
        let nav_for_lb = on_navigate.clone();
        let tag_for_lb = on_tag_selected.clone();
        list_box.connect_row_activated(move |_lb, row| {
            if let Some(path_obj) = unsafe { row.data::<PathBuf>("target_path") } {
                let p = unsafe { path_obj.as_ref() }.clone();
                nav_for_lb(p);
            } else if let Some(tag_obj) = unsafe { row.data::<String>("target_tag") } {
                let t = unsafe { tag_obj.as_ref() }.clone();
                tag_for_lb(t);
            }
        });

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
        .activatable(true)
        .selectable(true)
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

    let row = ListBoxRow::builder()
        .child(&hbox)
        .activatable(true)
        .selectable(true)
        .css_classes(["branches-sidebar-row"])
        .build();

    unsafe {
        row.set_data("target_tag", title.to_string());
    }

    row
}
