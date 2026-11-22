use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DropTarget, GestureClick, Image, Label, ListBox, ListBoxRow,
    Orientation, PolicyType, Popover, ScrolledWindow,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

thread_local! {
    static SIDEBAR_INSTANCES: RefCell<Vec<Weak<SidebarInner>>> = const { RefCell::new(Vec::new()) };
}

pub fn refresh_all_sidebars() {
    SIDEBAR_INSTANCES.with(|instances| {
        let mut list = instances.borrow_mut();
        list.retain(|weak| {
            if let Some(inner) = weak.upgrade() {
                inner.sync_default_rows_visibility();
                inner.rebuild_bookmarks();
                inner.rebuild_devices();
                inner.rebuild_tags();
                true
            } else {
                false
            }
        });
    });
}

struct SidebarInner {
    list_box: ListBox,
    default_removable_rows: Vec<(PathBuf, ListBoxRow)>,
    bookmarks_header: ListBoxRow,
    bookmark_rows: RefCell<Vec<ListBoxRow>>,
    devices_anchor: ListBoxRow,
    device_rows: RefCell<Vec<ListBoxRow>>,
    tags_header: ListBoxRow,
    tag_rows: RefCell<Vec<ListBoxRow>>,
    volume_monitor: gio::VolumeMonitor,
    signal_ids: RefCell<Vec<glib::SignalHandlerId>>,
    on_navigate: Rc<dyn Fn(PathBuf)>,
    on_tag_selected: Rc<dyn Fn(String)>,
    on_files_dropped: Rc<dyn Fn()>,
}

pub struct Sidebar {
    container: ScrolledWindow,
    list_box: ListBox,
    #[allow(dead_code)]
    inner: Rc<SidebarInner>,
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
    pub fn new<
        F: Fn(PathBuf) + 'static,
        R: Fn() + 'static,
        A: Fn() + 'static,
        T: Fn(String) + 'static,
        D: Fn() + 'static,
        C: Fn() -> PathBuf + 'static,
    >(
        on_navigate: F,
        on_recents: R,
        on_applications: A,
        on_tag_selected: T,
        on_files_dropped: D,
        get_current_folder: C,
    ) -> Self {
        let on_navigate: Rc<dyn Fn(PathBuf)> = Rc::new(on_navigate);
        let on_recents = Rc::new(on_recents);
        let on_applications = Rc::new(on_applications);
        let on_tag_selected = Rc::new(on_tag_selected);
        let on_files_dropped: Rc<dyn Fn()> = Rc::new(on_files_dropped);
        let get_current_folder: Rc<dyn Fn() -> PathBuf> = Rc::new(get_current_folder);

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .activate_on_single_click(true)
            .css_classes(["branches-sidebar"])
            .hexpand(true)
            .vexpand(false)
            .build();

        let es = malus_de_common::is_spanish_locale();

        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        let shared_path = resolve_user_dir(&home, "XDG_PUBLICSHARE_DIR", "Público", "Public");

        // 0. Accesos superiores sin encabezado (Recientes y Compartidos, estilo Finder)
        let recents_row = create_virtual_row(
            if es { "Recientes" } else { "Recents" },
            "document-open-recent-symbolic",
            "target_recents",
        );
        attach_virtual_row_handlers(&recents_row, on_recents.clone(), "@recents");
        list_box.append(&recents_row);

        let mut default_removable_rows: Vec<(PathBuf, ListBoxRow)> = Vec::new();

        let shared_title = if es { "Compartidos" } else { "Shared" };
        let shared_row = create_sidebar_row(
            shared_title,
            "folder-publicshare-symbolic",
            shared_path.clone(),
            true,
        );
        attach_folder_navigation_handlers(
            &shared_row,
            &on_navigate,
            shared_path.clone(),
            shared_title.to_string(),
            "folder-publicshare-symbolic",
            SidebarRemovalKind::DefaultEntry,
        );
        attach_folder_drop_target(&shared_row, shared_path.clone(), &on_files_dropped);
        list_box.append(&shared_row);
        default_removable_rows.push((shared_path, shared_row));

        // 1. Sección Favoritos
        let favorites_header =
            add_section_title(&list_box, if es { "Favoritos" } else { "Favorites" });
        attach_bookmark_drop_target(&favorites_header);
        let apps_row = create_virtual_row(
            if es { "Aplicaciones" } else { "Applications" },
            "system-software-install-symbolic",
            "target_applications",
        );
        attach_virtual_row_handlers(&apps_row, on_applications.clone(), "@applications");
        list_box.append(&apps_row);

        let desktop_path = resolve_user_dir(&home, "XDG_DESKTOP_DIR", "Escritorio", "Desktop");
        let docs_path = resolve_user_dir(&home, "XDG_DOCUMENTS_DIR", "Documentos", "Documents");
        let downloads_path = resolve_user_dir(&home, "XDG_DOWNLOAD_DIR", "Descargas", "Downloads");
        let pictures_path = resolve_user_dir(&home, "XDG_PICTURES_DIR", "Imágenes", "Pictures");
        let music_path = resolve_user_dir(&home, "XDG_MUSIC_DIR", "Música", "Music");
        let videos_path = resolve_user_dir(&home, "XDG_VIDEOS_DIR", "Vídeos", "Videos");
        let projects_path = resolve_user_dir(&home, "XDG_PROJECTS_DIR", "Proyectos", "Projects");
        let trash_path = home.join(".local/share/Trash/files");

        let favorites = [
            SidebarEntry {
                title: if es {
                    "Escritorio".to_string()
                } else {
                    "Desktop".to_string()
                },
                icon_name: "user-desktop-symbolic",
                path: desktop_path,
            },
            SidebarEntry {
                title: if es {
                    "Documentos".to_string()
                } else {
                    "Documents".to_string()
                },
                icon_name: "folder-documents-symbolic",
                path: docs_path,
            },
            SidebarEntry {
                title: if es {
                    "Descargas".to_string()
                } else {
                    "Downloads".to_string()
                },
                icon_name: "folder-download-symbolic",
                path: downloads_path,
            },
            SidebarEntry {
                title: if es {
                    "Imágenes".to_string()
                } else {
                    "Pictures".to_string()
                },
                icon_name: "folder-pictures-symbolic",
                path: pictures_path,
            },
            SidebarEntry {
                title: if es {
                    "Música".to_string()
                } else {
                    "Music".to_string()
                },
                icon_name: "folder-music-symbolic",
                path: music_path,
            },
            SidebarEntry {
                title: if es {
                    "Vídeos".to_string()
                } else {
                    "Videos".to_string()
                },
                icon_name: "folder-videos-symbolic",
                path: videos_path,
            },
            SidebarEntry {
                title: if es {
                    "Proyectos".to_string()
                } else {
                    "Projects".to_string()
                },
                icon_name: "folder-projects-symbolic",
                path: projects_path,
            },
        ];

        for entry in favorites {
            let row = create_sidebar_row(&entry.title, entry.icon_name, entry.path.clone(), true);
            attach_folder_navigation_handlers(
                &row,
                &on_navigate,
                entry.path.clone(),
                entry.title.clone(),
                entry.icon_name,
                SidebarRemovalKind::DefaultEntry,
            );
            attach_folder_drop_target(&row, entry.path.clone(), &on_files_dropped);
            list_box.append(&row);
            default_removable_rows.push((entry.path, row));
        }

        // 2. Sección Marcadores (personalizados por el usuario)
        let bookmarks_header = add_bookmarks_section_header(
            &list_box,
            if es { "Marcadores" } else { "Bookmarks" },
            &get_current_folder,
        );
        attach_bookmark_drop_target(&bookmarks_header);

        // 3. Sección Ubicaciones (estilo macOS Finder: usuario, equipo, red, papelera)
        let locations_header =
            add_section_title(&list_box, if es { "Ubicaciones" } else { "Locations" });
        attach_bookmark_drop_target(&locations_header);
        let home_label = home
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                if es {
                    "Inicio".to_string()
                } else {
                    "Home".to_string()
                }
            });
        let locations = [
            SidebarEntry {
                title: home_label,
                icon_name: "user-home-symbolic",
                path: home.clone(),
            },
            SidebarEntry {
                title: if es {
                    "Este Equipo".to_string()
                } else {
                    "This Computer".to_string()
                },
                icon_name: "drive-harddisk-symbolic",
                path: PathBuf::from("/"),
            },
            SidebarEntry {
                title: if es {
                    "Red".to_string()
                } else {
                    "Network".to_string()
                },
                icon_name: "network-workgroup-symbolic",
                path: PathBuf::from("/mnt"),
            },
            SidebarEntry {
                title: if es {
                    "Papelera".to_string()
                } else {
                    "Trash".to_string()
                },
                icon_name: "user-trash-symbolic",
                path: trash_path,
            },
        ];

        let mut devices_anchor: Option<ListBoxRow> = None;
        for entry in locations {
            let is_network_entry = entry.icon_name == "network-workgroup-symbolic";
            let is_trash_entry = entry.path.ends_with("Trash/files");
            let row = create_sidebar_row(&entry.title, entry.icon_name, entry.path.clone(), false);
            if is_network_entry {
                unsafe {
                    row.set_data("target_network", true);
                }
                let click = GestureClick::new();
                click.set_button(gdk::BUTTON_PRIMARY);
                let lb_ref = list_box.clone();
                let nav_cb = on_navigate.clone();
                click.connect_released(move |_, _, _, _| {
                    let win = lb_ref.root().and_downcast::<gtk4::Window>();
                    let cb = nav_cb.clone();
                    crate::network::open_network_location(win.as_ref(), move |p| cb(p));
                });
                row.add_controller(click);
                attach_network_context_menu(&row, es, &on_navigate);
            } else if is_trash_entry {
                attach_primary_and_middle_folder_nav(&row, &on_navigate, entry.path.clone());
                attach_folder_drop_target(&row, entry.path, &on_files_dropped);
                attach_trash_context_menu(&row, es);
            } else {
                let removable = if entry.path == home {
                    SidebarRemovalKind::DefaultEntry
                } else {
                    SidebarRemovalKind::None
                };
                attach_folder_navigation_handlers(
                    &row,
                    &on_navigate,
                    entry.path.clone(),
                    entry.title.clone(),
                    entry.icon_name,
                    removable,
                );
                if entry.path != Path::new("/") {
                    attach_folder_drop_target(&row, entry.path.clone(), &on_files_dropped);
                }
                if removable == SidebarRemovalKind::DefaultEntry {
                    default_removable_rows.push((entry.path, row.clone()));
                }
            }
            list_box.append(&row);
            devices_anchor = Some(row);
        }
        let devices_anchor = devices_anchor.expect("locations no vacío");

        // 4. Sección Etiquetas (Tags)
        let tags_header = add_section_title(&list_box, if es { "Etiquetas" } else { "Tags" });
        attach_bookmark_drop_target(&tags_header);
        list_box.append(&tags_header);

        // Manejador centralizado de activación de filas del ListBox
        let nav_for_lb = on_navigate.clone();
        let recents_for_lb = on_recents.clone();
        let apps_for_lb = on_applications.clone();
        let tag_for_lb = on_tag_selected.clone();
        let lb_for_net = list_box.clone();
        list_box.connect_row_activated(move |_lb, row| {
            if unsafe { row.data::<bool>("target_recents") }.is_some() {
                recents_for_lb();
            } else if unsafe { row.data::<bool>("target_applications") }.is_some() {
                apps_for_lb();
            } else if unsafe { row.data::<bool>("target_network") }.is_some() {
                let win = lb_for_net.root().and_downcast::<gtk4::Window>();
                let cb = nav_for_lb.clone();
                crate::network::open_network_location(win.as_ref(), move |p| cb(p));
            } else if let Some(path_obj) = unsafe { row.data::<PathBuf>("target_path") } {
                let p = unsafe { path_obj.as_ref() }.clone();
                if p.is_dir() {
                    nav_for_lb(p);
                } else {
                    let _ = crate::fs_backend::open_file(&p);
                }
            } else if let Some(tag_obj) = unsafe { row.data::<String>("target_tag") } {
                let t = unsafe { tag_obj.as_ref() }.clone();
                tag_for_lb(t);
            }
        });

        let sidebar_content = GtkBox::new(Orientation::Vertical, 0);
        sidebar_content.set_vexpand(true);
        sidebar_content.set_hexpand(true);
        sidebar_content.append(&list_box);

        // Zona vacía exclusiva debajo de las filas: recibe carpetas o archivos arrastrados
        // para añadirlos como marcadores sin iluminar el resto del sidebar.
        let empty_drop_zone = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .vexpand(true)
            .hexpand(true)
            .css_classes(["branches-sidebar-empty-drop"])
            .build();
        empty_drop_zone.set_size_request(-1, 64);
        attach_bookmark_drop_target(&empty_drop_zone);
        sidebar_content.append(&empty_drop_zone);

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .css_classes(["branches-sidebar-scroll"])
            .hexpand(true)
            .vexpand(true)
            .child(&sidebar_content)
            .build();

        // Clic derecho en el área libre del sidebar permite añadir marcadores sin afectar drag & drop
        let bg_right_click = GestureClick::new();
        bg_right_click.set_button(gdk::BUTTON_SECONDARY);
        let empty_zone_menu = empty_drop_zone.clone();
        let get_cur_bg = get_current_folder.clone();
        bg_right_click.connect_pressed(move |g, _n, x, y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            show_add_bookmark_popover(&empty_zone_menu, Some((x, y)), &get_cur_bg);
        });
        empty_drop_zone.add_controller(bg_right_click);

        let inner = Rc::new(SidebarInner {
            list_box: list_box.clone(),
            default_removable_rows,
            bookmarks_header,
            bookmark_rows: RefCell::new(Vec::new()),
            devices_anchor,
            device_rows: RefCell::new(Vec::new()),
            tags_header,
            tag_rows: RefCell::new(Vec::new()),
            volume_monitor: gio::VolumeMonitor::get(),
            signal_ids: RefCell::new(Vec::new()),
            on_navigate,
            on_tag_selected,
            on_files_dropped,
        });
        inner.sync_default_rows_visibility();
        inner.rebuild_bookmarks();
        inner.rebuild_devices();
        inner.rebuild_tags();
        inner.connect_volume_signals();

        SIDEBAR_INSTANCES.with(|instances| {
            instances.borrow_mut().push(Rc::downgrade(&inner));
        });

        Self {
            container,
            list_box,
            inner,
        }
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

    pub fn highlight_virtual(&self, data_key: &str) {
        let mut child = self.list_box.first_child();
        while let Some(widget) = child {
            if let Some(row) = widget.downcast_ref::<ListBoxRow>() {
                if unsafe { row.data::<bool>(data_key) }.is_some() {
                    self.list_box.select_row(Some(row));
                    return;
                }
            }
            child = widget.next_sibling();
        }
        self.list_box.unselect_all();
    }

    /// Si la fila seleccionada en el sidebar es un marcador, activa su edición inline y devuelve `true`.
    pub fn rename_selected_bookmark(&self) -> bool {
        let Some(row) = self.list_box.selected_row() else {
            return false;
        };
        if unsafe { row.data::<bool>("is_bookmark") }.is_none() {
            return false;
        }
        let Some(path_obj) = (unsafe { row.data::<PathBuf>("target_path") }) else {
            return false;
        };
        let folder_path = unsafe { path_obj.as_ref() }.clone();
        let current_title = row
            .child()
            .and_then(|c| c.downcast::<GtkBox>().ok())
            .and_then(|hbox| {
                let mut child = hbox.first_child();
                while let Some(w) = child {
                    if let Ok(lbl) = w.clone().downcast::<Label>() {
                        return Some(lbl.text().to_string());
                    }
                    child = w.next_sibling();
                }
                None
            })
            .unwrap_or_default();
        if current_title.is_empty() {
            return false;
        }
        show_rename_bookmark_popover(&row, folder_path, current_title, None);
        true
    }
}

impl Drop for SidebarInner {
    fn drop(&mut self) {
        for id in self.signal_ids.borrow_mut().drain(..) {
            self.volume_monitor.disconnect(id);
        }
    }
}

/// Puntos de montaje del sistema que nunca deben mostrarse como "disco externo".
fn is_system_mount_path(path: &Path) -> bool {
    const SYSTEM: [&str; 13] = [
        "/",
        "/boot",
        "/boot/efi",
        "/efi",
        "/home",
        "/var",
        "/usr",
        "/tmp",
        "/proc",
        "/sys",
        "/dev",
        "/nix",
        "/snap",
    ];
    if SYSTEM.iter().any(|s| path == Path::new(s)) {
        return true;
    }
    // /run/* excepto /run/media/...
    path.starts_with("/run") && !path.starts_with("/run/media")
}

/// Montajes típicos de medios extraíbles (udisks / automontaje).
fn is_external_mount_path(path: &Path) -> bool {
    !is_system_mount_path(path)
        && (path.starts_with("/run/media") || path.starts_with("/media"))
        && path != Path::new("/run/media")
        && path != Path::new("/media")
}

fn eject_or_unmount(mount: &gio::Mount, parent: Option<gtk4::Window>) {
    let op = gtk4::MountOperation::new(parent.as_ref());
    let name = mount.name();
    let report = move |res: Result<(), glib::Error>| {
        if let Err(e) = res {
            tracing::warn!("No se pudo expulsar {}: {}", name, e);
        }
    };
    if mount.can_eject() {
        mount.eject_with_operation(
            gio::MountUnmountFlags::NONE,
            Some(&op),
            gio::Cancellable::NONE,
            report,
        );
    } else {
        mount.unmount_with_operation(
            gio::MountUnmountFlags::NONE,
            Some(&op),
            gio::Cancellable::NONE,
            report,
        );
    }
}

fn eject_volume(volume: &gio::Volume, parent: Option<gtk4::Window>) {
    let op = gtk4::MountOperation::new(parent.as_ref());
    volume.eject_with_operation(
        gio::MountUnmountFlags::NONE,
        Some(&op),
        gio::Cancellable::NONE,
        |res| {
            if let Err(e) = res {
                tracing::warn!("No se pudo expulsar el volumen: {}", e);
            }
        },
    );
}

fn add_eject_button(row: &ListBoxRow, on_click: impl Fn() + 'static) {
    let es = malus_de_common::is_spanish_locale();
    let btn = Button::builder()
        .icon_name("media-eject-symbolic")
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text(if es { "Expulsar" } else { "Eject" })
        .css_classes(["branches-sidebar-eject-btn"])
        .build();
    btn.connect_clicked(move |_| on_click());
    if let Some(hbox) = row.child().and_then(|c| c.downcast::<GtkBox>().ok()) {
        hbox.append(&btn);
    }
}

impl SidebarInner {
    fn sync_default_rows_visibility(&self) {
        let hidden = crate::db::open_db()
            .map(|conn| crate::db::get_hidden_sidebar_paths(&conn))
            .unwrap_or_default();
        for (path, row) in &self.default_removable_rows {
            row.set_visible(!hidden.contains(path));
        }
    }

    fn connect_volume_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let rebuild: Rc<dyn Fn()> = Rc::new(move || {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.rebuild_devices();
                }
            });
        });
        let m = &self.volume_monitor;
        let mut ids = Vec::new();
        let r = rebuild.clone();
        ids.push(m.connect_mount_added(move |_, _| r()));
        let r = rebuild.clone();
        ids.push(m.connect_mount_removed(move |_, _| r()));
        let r = rebuild.clone();
        ids.push(m.connect_mount_changed(move |_, _| r()));
        let r = rebuild.clone();
        ids.push(m.connect_volume_added(move |_, _| r()));
        let r = rebuild.clone();
        ids.push(m.connect_volume_removed(move |_, _| r()));
        let r = rebuild.clone();
        ids.push(m.connect_volume_changed(move |_, _| r()));
        let r = rebuild.clone();
        ids.push(m.connect_drive_connected(move |_, _| r()));
        let r = rebuild;
        ids.push(m.connect_drive_disconnected(move |_, _| r()));
        *self.signal_ids.borrow_mut() = ids;
    }

    fn rebuild_devices(&self) {
        let selected_path = self.list_box.selected_row().and_then(|r| {
            unsafe { r.data::<PathBuf>("target_path") }.map(|p| unsafe { p.as_ref() }.clone())
        });

        for old in self.device_rows.borrow_mut().drain(..) {
            self.list_box.remove(&old);
        }
        let base_idx = self.devices_anchor.index();
        if base_idx < 0 {
            return;
        }

        let mut rows: Vec<ListBoxRow> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        // 1. Montajes activos (unidades extraíbles y recursos de red GVfs)
        for mount in self.volume_monitor.mounts() {
            if mount.is_shadowed() {
                continue;
            }
            let is_network = crate::network::is_network_gio_mount(&mount);
            let Some(root) = mount
                .root()
                .path()
                .or_else(|| mount.default_location().path())
            else {
                continue;
            };
            if !is_network && is_system_mount_path(&root) {
                continue;
            }
            let removable = mount.drive().is_some_and(|d| d.is_removable());
            if !(is_network || is_external_mount_path(&root) || removable) {
                continue;
            }
            if !seen.insert(root.clone()) {
                continue;
            }
            let icon = if is_network {
                "folder-remote-symbolic"
            } else if removable {
                "drive-removable-media-symbolic"
            } else {
                "drive-harddisk-symbolic"
            };
            let mount_name = mount.name().to_string();
            let row = create_sidebar_row(&mount_name, icon, root.clone(), false);
            attach_folder_navigation_handlers(
                &row,
                &self.on_navigate,
                root.clone(),
                mount_name,
                icon,
                SidebarRemovalKind::None,
            );
            attach_folder_drop_target(&row, root, &self.on_files_dropped);
            if mount.can_eject() || mount.can_unmount() {
                let mount = mount.clone();
                let list_box = self.list_box.clone();
                add_eject_button(&row, move || {
                    eject_or_unmount(&mount, list_box.root().and_downcast::<gtk4::Window>());
                });
            }
            rows.push(row);
        }

        // 2. Volúmenes sin montar (se montan al hacer clic)
        for volume in self.volume_monitor.volumes() {
            if volume.get_mount().is_some() || !volume.can_mount() {
                continue;
            }
            if !(volume.can_eject() || volume.drive().is_some_and(|d| d.is_removable())) {
                continue;
            }
            let row = create_sidebar_row(
                volume.name().as_str(),
                "drive-removable-media-symbolic",
                PathBuf::new(),
                false,
            );
            row.add_css_class("branches-sidebar-row-unmounted");
            let mount_action: Rc<dyn Fn()> = {
                let volume = volume.clone();
                let on_navigate = self.on_navigate.clone();
                let list_box = self.list_box.clone();
                Rc::new(move || {
                    let op = gtk4::MountOperation::new(
                        list_box.root().and_downcast::<gtk4::Window>().as_ref(),
                    );
                    let vol = volume.clone();
                    let on_navigate = on_navigate.clone();
                    volume.mount(
                        gio::MountMountFlags::NONE,
                        Some(&op),
                        gio::Cancellable::NONE,
                        move |res| match res {
                            Ok(()) => {
                                if let Some(path) = vol.get_mount().and_then(|m| m.root().path()) {
                                    on_navigate(path);
                                }
                            }
                            Err(e) => tracing::warn!("No se pudo montar el volumen: {}", e),
                        },
                    );
                })
            };
            let click = GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            let ma = mount_action.clone();
            click.connect_released(move |_, _, _, _| ma());
            row.add_controller(click);
            row.connect_activate(move |_| mount_action());
            if volume.can_eject() {
                let volume = volume.clone();
                let list_box = self.list_box.clone();
                add_eject_button(&row, move || {
                    eject_volume(&volume, list_box.root().and_downcast::<gtk4::Window>());
                });
            }
            rows.push(row);
        }

        // 3. Respaldo: directorios de /run/media/$USER/* y /media/*
        let mut scan_dirs = vec![PathBuf::from("/media")];
        if let Ok(user) = std::env::var("USER") {
            scan_dirs.push(PathBuf::from("/run/media").join(user));
        }
        for dir in scan_dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_dir() || !seen.insert(p.clone()) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let row =
                    create_sidebar_row(&name, "drive-removable-media-symbolic", p.clone(), false);
                attach_folder_navigation_handlers(
                    &row,
                    &self.on_navigate,
                    p.clone(),
                    name,
                    "drive-removable-media-symbolic",
                    SidebarRemovalKind::None,
                );
                attach_folder_drop_target(&row, p, &self.on_files_dropped);
                rows.push(row);
            }
        }

        for (i, row) in rows.iter().enumerate() {
            self.list_box.insert(row, base_idx + 1 + i as i32);
        }
        if let Some(sel) = selected_path {
            for row in &rows {
                if let Some(p) = unsafe { row.data::<PathBuf>("target_path") } {
                    if unsafe { p.as_ref() } == &sel {
                        self.list_box.select_row(Some(row));
                    }
                }
            }
        }
        *self.device_rows.borrow_mut() = rows;
    }

    fn rebuild_bookmarks(&self) {
        for old_row in self.bookmark_rows.borrow_mut().drain(..) {
            self.list_box.remove(&old_row);
        }

        let base_idx = self.bookmarks_header.index();
        if base_idx < 0 {
            return;
        }

        let bookmarks = crate::db::open_db()
            .map(|conn| crate::db::get_bookmarks(&conn))
            .unwrap_or_default();

        self.bookmarks_header.set_visible(!bookmarks.is_empty());

        let mut rows = Vec::with_capacity(bookmarks.len());
        for (offset, entry) in bookmarks.into_iter().enumerate() {
            let is_dir = entry.folder_path.is_dir();
            let icon_name = if is_dir {
                "folder-symbolic"
            } else {
                "text-x-generic-symbolic"
            };
            let row = create_sidebar_row(&entry.title, icon_name, entry.folder_path.clone(), true);
            unsafe {
                row.set_data("is_bookmark", true);
            }
            attach_bookmark_activation_handlers(
                &row,
                &self.on_navigate,
                entry.folder_path.clone(),
                entry.title.clone(),
            );
            if is_dir {
                attach_folder_drop_target(&row, entry.folder_path.clone(), &self.on_files_dropped);
            }
            attach_bookmark_context_menu(&row, entry.folder_path, entry.title, &self.on_navigate);
            self.list_box.insert(&row, base_idx + 1 + offset as i32);
            rows.push(row);
        }
        *self.bookmark_rows.borrow_mut() = rows;
    }

    fn rebuild_tags(&self) {
        for old_row in self.tag_rows.borrow_mut().drain(..) {
            self.list_box.remove(&old_row);
        }

        let base_idx = self.tags_header.index();
        if base_idx < 0 {
            return;
        }

        let es = malus_de_common::is_spanish_locale();
        let tags = crate::db::open_db()
            .map(|conn| crate::db::get_all_sidebar_tags(&conn, es))
            .unwrap_or_else(|_| {
                crate::db::TAG_CATALOG
                    .iter()
                    .map(|def| crate::db::TagMetadata {
                        tag_id: def.id.to_string(),
                        label: if es { def.label_es } else { def.label_en }.to_string(),
                        color_id: def.id.to_string(),
                        hidden_in_sidebar: false,
                    })
                    .collect()
            });

        self.tags_header.set_visible(!tags.is_empty());

        let mut rows = Vec::with_capacity(tags.len());
        for (offset, meta) in tags.into_iter().enumerate() {
            let row = create_tag_row(&meta.tag_id, &meta.color_id, &meta.label);
            let tag_cb = self.on_tag_selected.clone();
            let tag_id = meta.tag_id.clone();

            let click = GestureClick::new();
            let cb_c = tag_cb.clone();
            let t_c = tag_id.clone();
            click.connect_released(move |_, _, _, _| {
                cb_c(t_c.clone());
            });
            row.add_controller(click);

            let middle_click = GestureClick::new();
            middle_click.set_button(gdk::BUTTON_MIDDLE);
            let row_mid = row.downgrade();
            let t_mid = tag_id.clone();
            middle_click.connect_pressed(move |g, _n, _x, _y| {
                g.set_state(gtk4::EventSequenceState::Claimed);
                if let Some(r) = row_mid.upgrade() {
                    open_target_in_new_tab(&r, &format!("@tag:{}", t_mid));
                }
            });
            row.add_controller(middle_click);

            let t_a = tag_id.clone();
            row.connect_activate(move |_| {
                tag_cb(t_a.clone());
            });

            let color_hex = crate::db::tag_color_hex(&meta.color_id).to_string();
            attach_tag_drop_target(&row, tag_id, color_hex, &self.on_files_dropped);
            attach_tag_context_menu(
                &row,
                meta.tag_id,
                meta.label,
                meta.color_id,
                &self.on_files_dropped,
            );
            self.list_box.insert(&row, base_idx + 1 + offset as i32);
            rows.push(row);
        }
        *self.tag_rows.borrow_mut() = rows;
    }
}

fn can_bookmark_path(path: &Path) -> bool {
    path != Path::new("/")
        && !path.ends_with("Trash/files")
        && !path.parent().is_some_and(|p| p.ends_with("Trash/files"))
        && (path.is_dir() || path.is_file())
}

fn add_section_title(list_box: &ListBox, title: &str) -> ListBoxRow {
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
    row
}

fn show_add_bookmark_popover(
    anchor: &impl IsA<gtk4::Widget>,
    pointing_to: Option<(f64, f64)>,
    get_current_folder: &Rc<dyn Fn() -> PathBuf>,
) {
    let es = malus_de_common::is_spanish_locale();
    let current = get_current_folder();
    let pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["malus-popover-menu"])
        .build();
    pop.set_parent(anchor);
    if let Some((x, y)) = pointing_to {
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        pop.set_pointing_to(Some(&rect));
    }
    pop.connect_closed(|p| p.unparent());

    let vbox = GtkBox::new(Orientation::Vertical, 2);

    if can_bookmark_path(&current) {
        let folder_name = current
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| current.to_string_lossy().into_owned());
        let cur_lbl = if es {
            format!("Añadir carpeta actual («{}»)", folder_name)
        } else {
            format!("Bookmark Current Folder (“{}”)", folder_name)
        };
        let btn_cur = make_sidebar_menu_item(&cur_lbl);
        let pop_c = pop.downgrade();
        let cur_p = current;
        btn_cur.connect_clicked(move |_| {
            if let Some(p) = pop_c.upgrade() {
                p.popdown();
            }
            if let Ok(conn) = crate::db::open_db() {
                if !crate::db::unhide_sidebar_path(&conn, &cur_p).unwrap_or(false) {
                    let _ = crate::db::add_bookmark(&conn, &cur_p, None);
                }
                refresh_all_sidebars();
            }
        });
        vbox.append(&btn_cur);
    }

    let btn_pick = make_sidebar_menu_item(if es {
        "Elegir carpeta…"
    } else {
        "Choose Folder…"
    });
    let pop_p = pop.downgrade();
    let parent_for_dialog = anchor.clone().upcast::<gtk4::Widget>();
    btn_pick.connect_clicked(move |_| {
        if let Some(p) = pop_p.upgrade() {
            p.popdown();
        }
        let dialog = gtk4::FileDialog::builder()
            .title(if es {
                "Seleccionar carpeta para marcador"
            } else {
                "Select Folder to Bookmark"
            })
            .modal(true)
            .build();
        let win = parent_for_dialog
            .root()
            .and_then(|r| r.downcast::<gtk4::Window>().ok());
        dialog.select_folder(win.as_ref(), gio::Cancellable::NONE, |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    if can_bookmark_path(&path) {
                        if let Ok(conn) = crate::db::open_db() {
                            if !crate::db::unhide_sidebar_path(&conn, &path).unwrap_or(false) {
                                let _ = crate::db::add_bookmark(&conn, &path, None);
                            }
                            refresh_all_sidebars();
                        }
                    }
                }
            }
        });
    });
    vbox.append(&btn_pick);

    let has_hidden = crate::db::open_db()
        .map(|conn| {
            !crate::db::get_hidden_sidebar_paths(&conn).is_empty()
                || !crate::db::get_hidden_sidebar_tags(&conn).is_empty()
        })
        .unwrap_or(false);
    if has_hidden {
        vbox.append(&make_sidebar_menu_separator());
        let btn_restore = make_sidebar_menu_item(if es {
            "Restaurar barra lateral predeterminada"
        } else {
            "Restore Default Sidebar"
        });
        let pop_r = pop.downgrade();
        btn_restore.connect_clicked(move |_| {
            if let Some(p) = pop_r.upgrade() {
                p.popdown();
            }
            if let Ok(conn) = crate::db::open_db() {
                let _ = crate::db::restore_all_hidden_sidebar_paths(&conn);
                let _ = crate::db::restore_all_hidden_sidebar_tags(&conn);
                refresh_all_sidebars();
            }
        });
        vbox.append(&btn_restore);
    }

    pop.set_child(Some(&vbox));
    pop.popup();
}

fn add_bookmarks_section_header(
    list_box: &ListBox,
    title: &str,
    get_current_folder: &Rc<dyn Fn() -> PathBuf>,
) -> ListBoxRow {
    let es = malus_de_common::is_spanish_locale();
    let header_box = GtkBox::new(Orientation::Horizontal, 4);
    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .hexpand(true)
        .css_classes(["branches-sidebar-section-title"])
        .build();

    let add_btn = Button::builder()
        .icon_name("list-add-symbolic")
        .has_frame(false)
        .tooltip_text(if es {
            "Añadir marcador"
        } else {
            "Add Bookmark"
        })
        .css_classes(["branches-sidebar-add-btn"])
        .valign(Align::Center)
        .build();

    let get_cur = get_current_folder.clone();
    let anchor_btn = add_btn.clone();
    add_btn.connect_clicked(move |_| {
        show_add_bookmark_popover(&anchor_btn, None, &get_cur);
    });

    header_box.append(&label);
    header_box.append(&add_btn);

    let row = ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .child(&header_box)
        .build();

    list_box.append(&row);
    row
}

fn make_sidebar_menu_item(label: &str) -> Button {
    let btn = Button::builder()
        .label(label)
        .has_frame(false)
        .css_classes(["popover-menu-item"])
        .build();
    if let Some(lbl) = btn.child().and_then(|c| c.downcast::<Label>().ok()) {
        lbl.set_xalign(0.0);
    }
    crate::app_window::attach_squircle_menu_hover(&btn);
    btn
}

fn open_target_in_new_tab(anchor: &impl IsA<gtk4::Widget>, target: &str) {
    let _ = anchor.activate_action("win.open_in_new_tab", Some(&target.to_variant()));
}

fn attach_middle_click_new_tab(row: &ListBoxRow, target: String) {
    let mid_click = GestureClick::new();
    mid_click.set_button(gdk::BUTTON_MIDDLE);
    mid_click.connect_pressed(|g, _n, _x, _y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
    });
    let row_mid = row.clone();
    mid_click.connect_released(move |g, _n, _x, _y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        open_target_in_new_tab(&row_mid, &target);
    });
    row.add_controller(mid_click);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarRemovalKind {
    None,
    DefaultEntry,
    Bookmark,
}

fn make_sidebar_menu_separator() -> gtk4::Separator {
    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.set_margin_top(3);
    sep.set_margin_bottom(3);
    sep.set_margin_start(6);
    sep.set_margin_end(6);
    sep
}

fn encode_file_uri_for_dock(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut encoded = String::with_capacity(7 + raw.len());
    encoded.push_str("file://");
    for byte in raw.as_bytes() {
        let b = *byte;
        let is_unreserved = b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
}

fn dock_icon_for_sidebar_folder(sidebar_icon: &str) -> &'static str {
    match sidebar_icon {
        "user-desktop-symbolic" => "user-desktop",
        "folder-documents-symbolic" => "folder-documents",
        "folder-download-symbolic" => "folder-download",
        "folder-pictures-symbolic" => "folder-pictures",
        "folder-music-symbolic" => "folder-music",
        "folder-videos-symbolic" => "folder-videos",
        "folder-publicshare-symbolic" => "folder-publicshare",
        "user-home-symbolic" => "user-home",
        "drive-harddisk-symbolic" => "drive-harddisk",
        "drive-removable-media-symbolic" => "drive-removable-media",
        "folder-remote-symbolic" => "folder-remote",
        _ => "folder",
    }
}

fn folder_dock_app_config(
    path: &Path,
    title: &str,
    sidebar_icon: &str,
) -> malus_de_common::config::DockAppConfig {
    let fallback_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let name = if title.trim().is_empty() {
        fallback_name
    } else {
        title.trim().to_string()
    };
    malus_de_common::config::DockAppConfig {
        name,
        icon: dock_icon_for_sidebar_folder(sidebar_icon).to_string(),
        command: format!("malus-de-branches {}", encode_file_uri_for_dock(path)),
        matchers: Vec::new(),
        icon_appearance: malus_de_common::config::DockIconAppearance::Auto,
    }
}

fn is_folder_pinned_in_dock(
    cfg: &malus_de_common::config::DockConfig,
    app: &malus_de_common::config::DockAppConfig,
) -> bool {
    cfg.apps
        .iter()
        .any(|existing| !existing.command.is_empty() && existing.command == app.command)
}

fn set_folder_pinned_in_dock(app: malus_de_common::config::DockAppConfig, pin: bool) {
    use malus_de_common::config::DockConfig;
    use malus_de_common::ipc::{send_ipc_message, IpcMessage};

    let ipc_msg = if pin {
        IpcMessage::PinAppToDock {
            name: app.name.clone(),
            icon: app.icon.clone(),
            command: app.command.clone(),
            matchers: app.matchers.clone(),
            insert_at_dock_x: None,
        }
    } else {
        IpcMessage::UnpinAppFromDock {
            name: app.name.clone(),
        }
    };

    if send_ipc_message(&ipc_msg).is_err() {
        let mut cfg = DockConfig::load_or_create_default();
        let changed = if pin {
            cfg.pin_app(app)
        } else {
            cfg.unpin_app(&app.name)
        };
        if changed {
            let _ = cfg.save();
        }
    }
}

fn attach_primary_and_middle_folder_nav(
    row: &ListBoxRow,
    on_navigate: &Rc<dyn Fn(PathBuf)>,
    path: PathBuf,
) {
    let cb = on_navigate.clone();
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let cb_c = cb.clone();
    let p_c = path.clone();
    click.connect_released(move |_, _, _, _| {
        cb_c(p_c.clone());
    });
    row.add_controller(click);

    attach_middle_click_new_tab(row, path.to_string_lossy().to_string());

    row.connect_activate(move |_| {
        cb(path.clone());
    });
}

fn attach_folder_context_menu(
    row: &ListBoxRow,
    on_navigate: &Rc<dyn Fn(PathBuf)>,
    path: PathBuf,
    title: String,
    sidebar_icon: &'static str,
    removal_kind: SidebarRemovalKind,
) {
    let right_click = GestureClick::new();
    right_click.set_button(gdk::BUTTON_SECONDARY);
    let row_ref = row.downgrade();
    let nav_menu = on_navigate.clone();
    let p_menu = path;
    right_click.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        let Some(row_widget) = row_ref.upgrade() else {
            return;
        };
        let es = malus_de_common::is_spanish_locale();
        let pop = Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        pop.set_parent(&row_widget);
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        pop.set_pointing_to(Some(&rect));
        pop.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);

        // 1. Abrir en una pestaña nueva
        let new_tab_btn = make_sidebar_menu_item(if es {
            "Abrir en una pestaña nueva"
        } else {
            "Open in New Tab"
        });
        {
            let p_tab = pop.downgrade();
            let r_anchor = row_widget.downgrade();
            let target_str = p_menu.to_string_lossy().to_string();
            new_tab_btn.connect_clicked(move |_| {
                if let Some(p) = p_tab.upgrade() {
                    p.popdown();
                }
                if let Some(r) = r_anchor.upgrade() {
                    open_target_in_new_tab(&r, &target_str);
                }
            });
        }
        vbox.append(&new_tab_btn);

        // 2. Mostrar en la carpeta contenedora
        if let Some(parent) = p_menu.parent() {
            let parent_path = parent.to_path_buf();
            let enclosing_btn = make_sidebar_menu_item(if es {
                "Mostrar en la carpeta contenedora"
            } else {
                "Show in Enclosing Folder"
            });
            let p_enc = pop.downgrade();
            let nav = nav_menu.clone();
            enclosing_btn.connect_clicked(move |_| {
                if let Some(p) = p_enc.upgrade() {
                    p.popdown();
                }
                nav(parent_path.clone());
            });
            vbox.append(&enclosing_btn);
        }

        // 3. Eliminar de la barra lateral (separado por divisores estilo Finder)
        if removal_kind != SidebarRemovalKind::None {
            vbox.append(&make_sidebar_menu_separator());
            let remove_btn = make_sidebar_menu_item(if es {
                "Eliminar de la barra lateral"
            } else {
                "Remove from Sidebar"
            });
            let p_rem = pop.downgrade();
            let target = p_menu.clone();
            remove_btn.connect_clicked(move |_| {
                if let Some(p) = p_rem.upgrade() {
                    p.popdown();
                }
                if let Ok(conn) = crate::db::open_db() {
                    let changed = match removal_kind {
                        SidebarRemovalKind::Bookmark => {
                            crate::db::remove_bookmark(&conn, &target).unwrap_or(false)
                        }
                        SidebarRemovalKind::DefaultEntry => {
                            crate::db::hide_sidebar_path(&conn, &target).unwrap_or(false)
                        }
                        SidebarRemovalKind::None => false,
                    };
                    if changed {
                        refresh_all_sidebars();
                    }
                }
            });
            vbox.append(&remove_btn);
        }

        vbox.append(&make_sidebar_menu_separator());

        // 4. Obtener información
        let info_btn = make_sidebar_menu_item(if es {
            "Obtener información"
        } else {
            "Get Info"
        });
        {
            let p_info = pop.downgrade();
            let r_anchor = row_widget.downgrade();
            let target_str = p_menu.to_string_lossy().to_string();
            info_btn.connect_clicked(move |_| {
                if let Some(p) = p_info.upgrade() {
                    p.popdown();
                }
                if let Some(r) = r_anchor.upgrade() {
                    let _ = r.activate_action("win.get_info_item", Some(&target_str.to_variant()));
                }
            });
        }
        vbox.append(&info_btn);

        // 5. Agregar al Dock / Quitar del Dock
        let dock_app = folder_dock_app_config(&p_menu, &title, sidebar_icon);
        let dock_cfg = malus_de_common::config::DockConfig::load_or_create_default();
        let currently_pinned = is_folder_pinned_in_dock(&dock_cfg, &dock_app);
        let dock_label = if currently_pinned {
            if es {
                "Quitar del Dock"
            } else {
                "Remove from Dock"
            }
        } else if es {
            "Agregar al Dock"
        } else {
            "Add to Dock"
        };
        let dock_btn = make_sidebar_menu_item(dock_label);
        {
            let p_dock = pop.downgrade();
            dock_btn.connect_clicked(move |_| {
                if let Some(p) = p_dock.upgrade() {
                    p.popdown();
                }
                set_folder_pinned_in_dock(dock_app.clone(), !currently_pinned);
            });
        }
        vbox.append(&dock_btn);

        pop.set_child(Some(&vbox));
        pop.popup();
    });
    row.add_controller(right_click);
}

fn attach_folder_navigation_handlers(
    row: &ListBoxRow,
    on_navigate: &Rc<dyn Fn(PathBuf)>,
    path: PathBuf,
    title: String,
    sidebar_icon: &'static str,
    removal_kind: SidebarRemovalKind,
) {
    attach_primary_and_middle_folder_nav(row, on_navigate, path.clone());
    attach_folder_context_menu(row, on_navigate, path, title, sidebar_icon, removal_kind);
}

fn attach_bookmark_activation_handlers(
    row: &ListBoxRow,
    on_navigate: &Rc<dyn Fn(PathBuf)>,
    path: PathBuf,
    title: String,
) {
    use std::cell::Cell;

    let cb = on_navigate.clone();
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let row_press = row.clone();
    let was_selected_on_press = Rc::new(Cell::new(false));
    let pending_slow_rename: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));

    {
        let was_sel = was_selected_on_press.clone();
        let pending = pending_slow_rename.clone();
        click.connect_pressed(move |_, n_press, _, _| {
            if let Some(source_id) = pending.take() {
                source_id.remove();
            }
            if n_press == 1 {
                was_sel.set(row_press.is_selected());
            } else {
                was_sel.set(false);
            }
        });
    }

    let cb_c = cb.clone();
    let p_c = path.clone();
    let row_rel = row.clone();
    let was_sel_rel = was_selected_on_press;
    let pending_rel = pending_slow_rename;
    click.connect_released(move |_, n_press, _, _| {
        if n_press == 1 {
            row_rel.grab_focus();
            if was_sel_rel.get() {
                let r_ren = row_rel.clone();
                let p_ren = p_c.clone();
                let t_ren = title.clone();
                let slot = pending_rel.clone();
                let id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(450),
                    move || {
                        slot.set(None);
                        if !r_ren.is_selected() {
                            return;
                        }
                        show_rename_bookmark_popover(&r_ren, p_ren, t_ren, None);
                    },
                );
                pending_rel.set(Some(id));
            } else if p_c.is_dir() {
                cb_c(p_c.clone());
            } else {
                let _ = crate::fs_backend::open_file(&p_c);
            }
        } else if n_press >= 2 {
            if p_c.is_dir() {
                cb_c(p_c.clone());
            } else {
                let _ = crate::fs_backend::open_file(&p_c);
            }
        }
    });
    row.add_controller(click);
    if path.is_dir() {
        attach_middle_click_new_tab(row, path.to_string_lossy().to_string());
    }
    row.connect_activate(move |_| {
        if path.is_dir() {
            cb(path.clone());
        } else {
            let _ = crate::fs_backend::open_file(&path);
        }
    });
}

fn attach_folder_drop_target(row: &ListBoxRow, dest_dir: PathBuf, on_files_dropped: &Rc<dyn Fn()>) {
    let drop_target = DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    let r_enter = row.clone();
    drop_target.connect_enter(move |_, _, _| {
        r_enter.add_css_class("folder-drop-hover");
        gdk::DragAction::MOVE
    });
    drop_target.connect_motion(move |_, _, _| gdk::DragAction::MOVE);
    let r_leave = row.clone();
    drop_target.connect_leave(move |_| {
        r_leave.remove_css_class("folder-drop-hover");
    });

    let r_drop = row.clone();
    let reload_cb = on_files_dropped.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        r_drop.remove_css_class("folder-drop-hover");
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            if crate::operations::drop_files_into_folder(&file_list.files(), &dest_dir) {
                refresh_all_sidebars();
                reload_cb();
                return true;
            }
        }
        false
    });
    row.add_controller(drop_target);
}

fn attach_bookmark_drop_target(widget: &impl IsA<gtk4::Widget>) {
    let drop_target = DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    let w_enter = widget.clone();
    drop_target.connect_enter(move |_, _, _| {
        w_enter.add_css_class("branches-sidebar-section-drop-hover");
        gdk::DragAction::COPY
    });
    drop_target.connect_motion(move |_, _, _| gdk::DragAction::COPY);
    let w_leave = widget.clone();
    drop_target.connect_leave(move |_| {
        w_leave.remove_css_class("branches-sidebar-section-drop-hover");
    });
    let w_drop = widget.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        w_drop.remove_css_class("branches-sidebar-section-drop-hover");
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let mut added_any = false;
            if let Ok(conn) = crate::db::open_db() {
                for file in file_list.files() {
                    if let Some(path) = file.path() {
                        if can_bookmark_path(&path)
                            && (crate::db::unhide_sidebar_path(&conn, &path).unwrap_or(false)
                                || crate::db::add_bookmark(&conn, &path, None).unwrap_or(false))
                        {
                            added_any = true;
                        }
                    }
                }
            }
            if added_any {
                refresh_all_sidebars();
                return true;
            }
        }
        false
    });
    widget.add_controller(drop_target);
}

fn attach_tag_drop_target(
    row: &ListBoxRow,
    tag_name: String,
    color_hex: String,
    on_files_dropped: &Rc<dyn Fn()>,
) {
    let drop_target = DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    let r_enter = row.clone();
    drop_target.connect_enter(move |_, _, _| {
        r_enter.add_css_class("folder-drop-hover");
        gdk::DragAction::COPY
    });
    drop_target.connect_motion(move |_, _, _| gdk::DragAction::COPY);
    let r_leave = row.clone();
    drop_target.connect_leave(move |_| {
        r_leave.remove_css_class("folder-drop-hover");
    });
    let r_drop = row.clone();
    let reload_cb = on_files_dropped.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        r_drop.remove_css_class("folder-drop-hover");
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let mut tagged = false;
            if let Ok(conn) = crate::db::open_db() {
                for file in file_list.files() {
                    if let Some(path) = file.path() {
                        if crate::db::set_file_tag(&conn, &path, &tag_name, &color_hex).is_ok() {
                            tagged = true;
                        }
                    }
                }
            }
            if tagged {
                reload_cb();
                return true;
            }
        }
        false
    });
    row.add_controller(drop_target);
}

fn show_rename_bookmark_popover(
    anchor: &impl IsA<gtk4::Widget>,
    folder_path: PathBuf,
    current_title: String,
    _pointing_to: Option<(f64, f64)>,
) {
    use std::cell::Cell;

    if anchor.root().is_none() {
        return;
    }

    let anchor_widget = anchor.as_ref();
    let Some(hbox) = anchor_widget
        .downcast_ref::<ListBoxRow>()
        .and_then(|r| r.child())
        .and_then(|c| c.downcast::<GtkBox>().ok())
    else {
        return;
    };

    let mut label_opt: Option<Label> = None;
    let mut child = hbox.first_child();
    while let Some(w) = child {
        if let Ok(lbl) = w.clone().downcast::<Label>() {
            label_opt = Some(lbl);
            break;
        }
        child = w.next_sibling();
    }
    let Some(label) = label_opt else {
        return;
    };
    if !label.is_visible() {
        return;
    }

    let label_w = label.width().max(24);
    let label_h = label.height().max(16);

    let entry = gtk4::Text::builder()
        .text(&current_title)
        .hexpand(true)
        .halign(Align::Fill)
        .valign(Align::Center)
        .width_chars(1)
        .max_width_chars(1)
        .css_classes(["branches-inline-rename-entry", "branches-inline-rename-row"])
        .build();
    entry.set_size_request(label_w, label_h);

    label.set_visible(false);
    hbox.insert_child_after(&entry, Some(&label));

    let finished = Rc::new(Cell::new(false));
    let finish_edit: Rc<dyn Fn(bool)> = {
        let finished = finished.clone();
        let entry_ref = entry.clone();
        let label_ref = label.clone();
        let hbox_ref = hbox.clone();
        let initial = current_title.clone();
        Rc::new(move |commit: bool| {
            if finished.replace(true) {
                return;
            }
            let new_title = entry_ref.text().to_string();
            let trimmed = new_title.trim();

            if entry_ref.parent().is_some() {
                hbox_ref.remove(&entry_ref);
            }
            label_ref.set_visible(true);
            if let Some(row) = label_ref.ancestor(ListBoxRow::static_type()) {
                row.grab_focus();
            }

            if !commit || trimmed.is_empty() || trimmed == initial {
                return;
            }
            label_ref.set_label(trimmed);
            if let Ok(conn) = crate::db::open_db() {
                if crate::db::rename_bookmark(&conn, &folder_path, trimmed).unwrap_or(false) {
                    refresh_all_sidebars();
                }
            }
        })
    };

    {
        let finish = finish_edit.clone();
        entry.connect_activate(move |_| {
            finish(true);
        });
    }

    {
        let key_ctrl = gtk4::EventControllerKey::new();
        let finish = finish_edit.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                finish(false);
                return glib::Propagation::Stop;
            }
            if matches!(
                key,
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
            ) {
                finish(true);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }

    {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        let finish = finish_edit;
        focus_ctrl.connect_leave(move |_| {
            let f = finish.clone();
            glib::idle_add_local_once(move || {
                f(true);
            });
        });
        entry.add_controller(focus_ctrl);
    }

    {
        let click_guard = GestureClick::new();
        click_guard.set_button(0);
        click_guard.connect_pressed(|g, _n, _x, _y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        entry.add_controller(click_guard);
    }

    let entry_focus = entry;
    glib::idle_add_local_once(move || {
        entry_focus.grab_focus();
        entry_focus.select_region(0, -1);
    });
}

fn attach_bookmark_context_menu(
    row: &ListBoxRow,
    folder_path: PathBuf,
    current_title: String,
    on_navigate: &Rc<dyn Fn(PathBuf)>,
) {
    if folder_path.is_dir() {
        attach_folder_context_menu(
            row,
            on_navigate,
            folder_path,
            current_title,
            "folder-symbolic",
            SidebarRemovalKind::Bookmark,
        );
        return;
    }

    let right_click = GestureClick::new();
    right_click.set_button(gdk::BUTTON_SECONDARY);
    let row_ref = row.downgrade();
    let nav_cb = on_navigate.clone();
    right_click.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        let Some(row_widget) = row_ref.upgrade() else {
            return;
        };
        let es = malus_de_common::is_spanish_locale();
        let pop = Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        pop.set_parent(&row_widget);
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        pop.set_pointing_to(Some(&rect));
        pop.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);

        let open_btn = make_sidebar_menu_item(if es { "Abrir" } else { "Open" });
        {
            let p_open = pop.downgrade();
            let target = folder_path.clone();
            open_btn.connect_clicked(move |_| {
                if let Some(p) = p_open.upgrade() {
                    p.popdown();
                }
                let _ = crate::fs_backend::open_file(&target);
            });
        }
        vbox.append(&open_btn);

        if let Some(parent) = folder_path.parent() {
            let parent_path = parent.to_path_buf();
            let enclosing_btn = make_sidebar_menu_item(if es {
                "Mostrar en la carpeta contenedora"
            } else {
                "Show in Enclosing Folder"
            });
            let p_enc = pop.downgrade();
            let nav = nav_cb.clone();
            enclosing_btn.connect_clicked(move |_| {
                if let Some(p) = p_enc.upgrade() {
                    p.popdown();
                }
                nav(parent_path.clone());
            });
            vbox.append(&enclosing_btn);
        }

        vbox.append(&make_sidebar_menu_separator());

        let remove_btn = make_sidebar_menu_item(if es {
            "Eliminar de la barra lateral"
        } else {
            "Remove from Sidebar"
        });
        {
            let p_rem = pop.downgrade();
            let target = folder_path.clone();
            remove_btn.connect_clicked(move |_| {
                if let Some(p) = p_rem.upgrade() {
                    p.popdown();
                }
                if let Ok(conn) = crate::db::open_db() {
                    if crate::db::remove_bookmark(&conn, &target).unwrap_or(false) {
                        refresh_all_sidebars();
                    }
                }
            });
        }
        vbox.append(&remove_btn);

        vbox.append(&make_sidebar_menu_separator());

        let info_btn = make_sidebar_menu_item(if es {
            "Obtener información"
        } else {
            "Get Info"
        });
        {
            let p_info = pop.downgrade();
            let r_anchor = row_widget.downgrade();
            let target_str = folder_path.to_string_lossy().to_string();
            info_btn.connect_clicked(move |_| {
                if let Some(p) = p_info.upgrade() {
                    p.popdown();
                }
                if let Some(r) = r_anchor.upgrade() {
                    let _ = r.activate_action("win.get_info_item", Some(&target_str.to_variant()));
                }
            });
        }
        vbox.append(&info_btn);

        pop.set_child(Some(&vbox));
        pop.popup();
    });
    row.add_controller(right_click);
}

fn attach_virtual_row_handlers(row: &ListBoxRow, cb: Rc<dyn Fn()>, new_tab_target: &'static str) {
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let cb_c = cb.clone();
    click.connect_released(move |_, _, _, _| {
        cb_c();
    });
    row.add_controller(click);

    attach_middle_click_new_tab(row, new_tab_target.to_string());

    let right_click = GestureClick::new();
    right_click.set_button(gdk::BUTTON_SECONDARY);
    let row_ref = row.clone();
    let cb_menu = cb.clone();
    right_click.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        let es = malus_de_common::is_spanish_locale();
        let pop = Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        pop.set_parent(&row_ref);
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        pop.set_pointing_to(Some(&rect));
        pop.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);

        let open_btn = make_sidebar_menu_item(if es { "Abrir" } else { "Open" });
        {
            let p_open = pop.clone();
            let cb_open = cb_menu.clone();
            open_btn.connect_clicked(move |_| {
                p_open.popdown();
                cb_open();
            });
        }
        vbox.append(&open_btn);

        let new_tab_btn = make_sidebar_menu_item(if es {
            "Abrir en nueva pestaña"
        } else {
            "Open in New Tab"
        });
        {
            let p_tab = pop.clone();
            let r_anchor = row_ref.clone();
            new_tab_btn.connect_clicked(move |_| {
                p_tab.popdown();
                open_target_in_new_tab(&r_anchor, new_tab_target);
            });
        }
        vbox.append(&new_tab_btn);

        pop.set_child(Some(&vbox));
        pop.popup();
    });
    row.add_controller(right_click);

    row.connect_activate(move |_| {
        cb();
    });
}

fn create_virtual_row(title: &str, icon_name: &str, data_key: &'static str) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

    let icon = Image::from_icon_name(icon_name);
    icon.add_css_class("branches-sidebar-icon");
    malus_de_common::sidebar_style::register_sidebar_icon(&icon);

    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["branches-sidebar-label", "malus-sidebar-label"])
        .build();

    hbox.append(&icon);
    hbox.append(&label);

    let row = ListBoxRow::builder()
        .child(&hbox)
        .activatable(true)
        .selectable(true)
        .css_classes(["branches-sidebar-row", "branches-sidebar-row-favorite"])
        .build();
    malus_de_common::sidebar_style::register_sidebar_row(&row);

    unsafe {
        row.set_data(data_key, true);
    }

    row
}

fn create_sidebar_row(
    title: &str,
    icon_name: &str,
    path: PathBuf,
    is_favorite: bool,
) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

    let icon = Image::from_icon_name(icon_name);
    icon.add_css_class("branches-sidebar-icon");
    malus_de_common::sidebar_style::register_sidebar_icon(&icon);

    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["branches-sidebar-label", "malus-sidebar-label"])
        .build();

    hbox.append(&icon);
    hbox.append(&label);

    let row = ListBoxRow::builder()
        .child(&hbox)
        .activatable(true)
        .selectable(true)
        .css_classes([
            "branches-sidebar-row",
            if is_favorite {
                "branches-sidebar-row-favorite"
            } else {
                "branches-sidebar-row-location"
            },
        ])
        .build();
    malus_de_common::sidebar_style::register_sidebar_row(&row);

    unsafe {
        row.set_data("target_path", path);
    }

    row
}

fn attach_network_context_menu(row: &ListBoxRow, es: bool, on_navigate: &Rc<dyn Fn(PathBuf)>) {
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    let row_ref = row.clone();
    let nav_ref = on_navigate.clone();
    click.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        let popover = gtk4::Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        popover.set_parent(&row_ref);
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        popover.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);

        let connect_btn = make_sidebar_menu_item(if es {
            "Conectar al servidor…"
        } else {
            "Connect to Server…"
        });
        let anchor = row_ref.clone();
        let pop = popover.clone();
        connect_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = anchor.activate_action("win.connect_server", None);
        });
        vbox.append(&connect_btn);

        if let Some(gvfs_root) = crate::network::gvfs_mounts_root() {
            let gvfs_btn = make_sidebar_menu_item(if es {
                "Abrir carpeta de montajes GVfs"
            } else {
                "Open GVfs Mounts Folder"
            });
            let pop2 = popover.clone();
            let nav_c = nav_ref.clone();
            gvfs_btn.connect_clicked(move |_| {
                pop2.popdown();
                nav_c(gvfs_root.clone());
            });
            vbox.append(&gvfs_btn);
        }

        popover.set_child(Some(&vbox));
        popover.popup();
    });
    row.add_controller(click);
}

fn attach_trash_context_menu(row: &ListBoxRow, es: bool) {
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    let row_ref = row.clone();
    click.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        let popover = gtk4::Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        popover.set_parent(&row_ref);
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        popover.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);
        let empty_btn = make_sidebar_menu_item(if es {
            "Vaciar Papelera…"
        } else {
            "Empty Trash…"
        });
        let anchor = row_ref.clone();
        let pop = popover.clone();
        empty_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = anchor.activate_action("win.empty_trash", None);
        });
        vbox.append(&empty_btn);
        popover.set_child(Some(&vbox));
        popover.popup();
    });
    row.add_controller(click);
}

fn make_tag_color_menu_item(color_id: &str, label: &str) -> Button {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(2);
    hbox.set_margin_end(4);

    let dot = GtkBox::new(Orientation::Horizontal, 0);
    dot.set_size_request(12, 12);
    dot.set_valign(Align::Center);
    dot.add_css_class("branches-tag-dot");
    dot.add_css_class(&format!("branches-tag-color-{}", color_id));

    let text = Label::builder()
        .label(label)
        .halign(Align::Start)
        .hexpand(true)
        .build();

    hbox.append(&dot);
    hbox.append(&text);

    Button::builder()
        .child(&hbox)
        .has_frame(false)
        .css_classes(["flat", "malus-menu-item"])
        .build()
}

fn show_delete_tag_dialog(
    parent_win: Option<&gtk4::Window>,
    tag_id: &str,
    tag_label: &str,
    on_changed: &Rc<dyn Fn()>,
) {
    let es = malus_de_common::is_spanish_locale();
    let mut builder = gtk4::Window::builder()
        .modal(true)
        .resizable(false)
        .destroy_with_parent(true)
        .default_width(360)
        .title(if es {
            format!("¿Eliminar la etiqueta “{}”?", tag_label)
        } else {
            format!("Delete Tag “{}”?", tag_label)
        });
    if let Some(w) = parent_win {
        builder = builder.transient_for(w);
    }
    let dialog = builder.build();

    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(18);
    vbox.set_margin_end(18);

    let body = Label::builder()
        .label(if es {
            format!(
                "Esta acción quitará la etiqueta «{}» de todos los archivos y la eliminará de la barra lateral.",
                tag_label
            )
        } else {
            format!(
                "This will remove the “{}” tag from all files and delete it from the sidebar.",
                tag_label
            )
        })
        .wrap(true)
        .xalign(0.0)
        .build();
    vbox.append(&body);

    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let cancel_btn = Button::with_label(if es { "Cancelar" } else { "Cancel" });
    let delete_btn = Button::builder()
        .label(if es { "Eliminar" } else { "Delete" })
        .css_classes(["destructive-action"])
        .build();
    btns.append(&cancel_btn);
    btns.append(&delete_btn);
    vbox.append(&btns);
    dialog.set_child(Some(&vbox));

    let d_cancel = dialog.downgrade();
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = d_cancel.upgrade() {
            d.close();
        }
    });

    let d_ok = dialog.downgrade();
    let tid = tag_id.to_string();
    let cb = on_changed.clone();
    delete_btn.connect_clicked(move |_| {
        if let Some(d) = d_ok.upgrade() {
            d.close();
        }
        if let Ok(conn) = crate::db::open_db() {
            let _ = crate::db::delete_tag_globally(&conn, &tid);
            refresh_all_sidebars();
            cb();
        }
    });

    dialog.present();
}

fn show_rename_tag_dialog(
    parent_win: Option<&gtk4::Window>,
    tag_id: &str,
    current_label: &str,
    on_changed: &Rc<dyn Fn()>,
) {
    let es = malus_de_common::is_spanish_locale();
    let mut builder = gtk4::Window::builder()
        .modal(true)
        .resizable(false)
        .destroy_with_parent(true)
        .default_width(340)
        .title(if es {
            "Renombrar etiqueta"
        } else {
            "Rename Tag"
        });
    if let Some(w) = parent_win {
        builder = builder.transient_for(w);
    }
    let dialog = builder.build();

    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(18);
    vbox.set_margin_end(18);

    let entry = gtk4::Entry::builder().text(current_label).build();
    entry.select_region(0, -1);
    vbox.append(&entry);

    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let cancel_btn = Button::with_label(if es { "Cancelar" } else { "Cancel" });
    let save_btn = Button::builder()
        .label(if es { "Renombrar" } else { "Rename" })
        .css_classes(["suggested-action"])
        .build();
    btns.append(&cancel_btn);
    btns.append(&save_btn);
    vbox.append(&btns);
    dialog.set_child(Some(&vbox));

    let d_cancel = dialog.downgrade();
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = d_cancel.upgrade() {
            d.close();
        }
    });

    let apply_rename = {
        let d_ok = dialog.downgrade();
        let entry_c = entry.downgrade();
        let tid = tag_id.to_string();
        let cb = on_changed.clone();
        Rc::new(move || {
            let Some(e) = entry_c.upgrade() else {
                return;
            };
            let new_name = e.text().to_string();
            if new_name.trim().is_empty() {
                return;
            }
            if let Some(d) = d_ok.upgrade() {
                d.close();
            }
            if let Ok(conn) = crate::db::open_db() {
                if crate::db::rename_tag(&conn, &tid, &new_name).unwrap_or(false) {
                    refresh_all_sidebars();
                    cb();
                }
            }
        })
    };

    let apply_btn = apply_rename.clone();
    save_btn.connect_clicked(move |_| apply_btn());
    entry.connect_activate(move |_| apply_rename());

    dialog.present();
    entry.grab_focus();
}

fn attach_tag_context_menu(
    row: &ListBoxRow,
    tag_id: String,
    tag_label: String,
    _color_id: String,
    on_changed: &Rc<dyn Fn()>,
) {
    let right_click = GestureClick::new();
    right_click.set_button(gdk::BUTTON_SECONDARY);
    let row_ref = row.downgrade();
    let on_changed = on_changed.clone();

    right_click.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        let Some(row_widget) = row_ref.upgrade() else {
            return;
        };
        let es = malus_de_common::is_spanish_locale();
        let pop = Popover::builder()
            .has_arrow(false)
            .css_classes(["malus-popover-menu"])
            .build();
        pop.set_parent(&row_widget);
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        pop.set_pointing_to(Some(&rect));
        pop.connect_closed(|p| p.unparent());

        let vbox = GtkBox::new(Orientation::Vertical, 2);

        // 1. Abrir en una pestaña nueva
        let new_tab_btn = make_sidebar_menu_item(if es {
            "Abrir en una pestaña nueva"
        } else {
            "Open in New Tab"
        });
        {
            let p_tab = pop.downgrade();
            let r_anchor = row_widget.downgrade();
            let spec = format!("@tag:{}", tag_id);
            new_tab_btn.connect_clicked(move |_| {
                if let Some(p) = p_tab.upgrade() {
                    p.popdown();
                }
                if let Some(r) = r_anchor.upgrade() {
                    open_target_in_new_tab(&r, &spec);
                }
            });
        }
        vbox.append(&new_tab_btn);

        vbox.append(&make_sidebar_menu_separator());

        // 2. Eliminar de la barra lateral
        let remove_sidebar_btn = make_sidebar_menu_item(if es {
            "Eliminar de la barra lateral"
        } else {
            "Remove from Sidebar"
        });
        {
            let p_rem = pop.downgrade();
            let tid = tag_id.clone();
            remove_sidebar_btn.connect_clicked(move |_| {
                if let Some(p) = p_rem.upgrade() {
                    p.popdown();
                }
                if let Ok(conn) = crate::db::open_db() {
                    let _ = crate::db::set_tag_hidden_in_sidebar(&conn, &tid, true);
                    refresh_all_sidebars();
                }
            });
        }
        vbox.append(&remove_sidebar_btn);

        // 3. Eliminar etiqueta “<Nombre>”...
        let delete_tag_btn = make_sidebar_menu_item(&if es {
            format!("Eliminar etiqueta “{}”...", tag_label)
        } else {
            format!("Delete Tag “{}”...", tag_label)
        });
        {
            let p_del = pop.downgrade();
            let r_anchor = row_widget.downgrade();
            let tid = tag_id.clone();
            let lbl = tag_label.clone();
            let cb = on_changed.clone();
            delete_tag_btn.connect_clicked(move |_| {
                if let Some(p) = p_del.upgrade() {
                    p.popdown();
                }
                let parent_win = r_anchor
                    .upgrade()
                    .and_then(|r| r.root())
                    .and_downcast::<gtk4::Window>();
                show_delete_tag_dialog(parent_win.as_ref(), &tid, &lbl, &cb);
            });
        }
        vbox.append(&delete_tag_btn);

        vbox.append(&make_sidebar_menu_separator());

        // 4. Renombrar “<Nombre>”
        let rename_btn = make_sidebar_menu_item(&if es {
            format!("Renombrar “{}”", tag_label)
        } else {
            format!("Rename “{}”", tag_label)
        });
        {
            let p_ren = pop.downgrade();
            let r_anchor = row_widget.downgrade();
            let tid = tag_id.clone();
            let lbl = tag_label.clone();
            let cb = on_changed.clone();
            rename_btn.connect_clicked(move |_| {
                if let Some(p) = p_ren.upgrade() {
                    p.popdown();
                }
                let parent_win = r_anchor
                    .upgrade()
                    .and_then(|r| r.root())
                    .and_downcast::<gtk4::Window>();
                show_rename_tag_dialog(parent_win.as_ref(), &tid, &lbl, &cb);
            });
        }
        vbox.append(&rename_btn);

        vbox.append(&make_sidebar_menu_separator());

        // 5. Selector de color de etiqueta (Sin color + 7 colores estándar)
        let no_color_btn =
            make_tag_color_menu_item("none", if es { "Sin color" } else { "No Color" });
        {
            let p_col = pop.downgrade();
            let tid = tag_id.clone();
            let cb = on_changed.clone();
            no_color_btn.connect_clicked(move |_| {
                if let Some(p) = p_col.upgrade() {
                    p.popdown();
                }
                if let Ok(conn) = crate::db::open_db() {
                    let _ = crate::db::set_tag_color(&conn, &tid, "none");
                    refresh_all_sidebars();
                    cb();
                }
            });
        }
        vbox.append(&no_color_btn);

        for def in crate::db::TAG_CATALOG.iter() {
            let color_btn =
                make_tag_color_menu_item(def.id, if es { def.label_es } else { def.label_en });
            let p_col = pop.downgrade();
            let tid = tag_id.clone();
            let cid = def.id.to_string();
            let cb = on_changed.clone();
            color_btn.connect_clicked(move |_| {
                if let Some(p) = p_col.upgrade() {
                    p.popdown();
                }
                if let Ok(conn) = crate::db::open_db() {
                    let _ = crate::db::set_tag_color(&conn, &tid, &cid);
                    refresh_all_sidebars();
                    cb();
                }
            });
            vbox.append(&color_btn);
        }

        pop.set_child(Some(&vbox));
        pop.popup();
    });

    row.add_controller(right_click);
}

fn create_tag_row(tag_id: &str, color_id: &str, title: &str) -> ListBoxRow {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);

    let dot = GtkBox::new(Orientation::Horizontal, 0);
    dot.set_size_request(10, 10);
    dot.set_valign(Align::Center);
    dot.add_css_class("branches-tag-dot");
    dot.add_css_class(&format!("branches-tag-color-{}", color_id));

    let label = Label::builder()
        .label(title)
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["branches-sidebar-label", "malus-sidebar-label"])
        .build();

    hbox.append(&dot);
    hbox.append(&label);

    let row = ListBoxRow::builder()
        .child(&hbox)
        .activatable(true)
        .selectable(true)
        .css_classes(["branches-sidebar-row"])
        .build();
    malus_de_common::sidebar_style::register_sidebar_row(&row);

    unsafe {
        row.set_data("target_tag", tag_id.to_string());
    }

    row
}

#[cfg(test)]
mod device_path_tests {
    use super::*;

    #[test]
    fn system_paths_are_not_external() {
        for p in ["/", "/boot", "/boot/efi", "/home", "/run/user/1000/gvfs"] {
            assert!(!is_external_mount_path(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn media_paths_are_external() {
        assert!(is_external_mount_path(Path::new("/run/media/cristian/USB")));
        assert!(is_external_mount_path(Path::new("/media/disk")));
        assert!(!is_external_mount_path(Path::new("/media")));
    }

    #[test]
    fn folder_dock_app_config_encodes_spaces_and_maps_icons() {
        let cfg = folder_dock_app_config(
            Path::new("/home/cristian/Mis Documentos/Imágenes"),
            "Imágenes",
            "folder-pictures-symbolic",
        );
        assert_eq!(cfg.name, "Imágenes");
        assert_eq!(cfg.icon, "folder-pictures");
        assert_eq!(
            cfg.command,
            "malus-de-branches file:///home/cristian/Mis%20Documentos/Im%C3%A1genes"
        );
        assert!(cfg.matchers.is_empty());
        assert_eq!(dock_icon_for_sidebar_folder("user-desktop-symbolic"), "user-desktop");
        assert_eq!(dock_icon_for_sidebar_folder("unknown-symbolic"), "folder");
    }

    #[test]
    fn hidden_sidebar_defaults_roundtrip_in_sqlite() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE sidebar_hidden_defaults (folder_path TEXT PRIMARY KEY)",
            [],
        )
        .unwrap();
        let desktop = Path::new("/home/cristian/Escritorio");
        assert!(crate::db::hide_sidebar_path(&conn, desktop).unwrap());
        assert!(crate::db::get_hidden_sidebar_paths(&conn).contains(desktop));
        assert!(crate::db::unhide_sidebar_path(&conn, desktop).unwrap());
        assert!(!crate::db::get_hidden_sidebar_paths(&conn).contains(desktop));

        assert!(crate::db::hide_sidebar_path(&conn, desktop).unwrap());
        assert_eq!(crate::db::restore_all_hidden_sidebar_paths(&conn).unwrap(), 1);
        assert!(crate::db::get_hidden_sidebar_paths(&conn).is_empty());
    }
}
