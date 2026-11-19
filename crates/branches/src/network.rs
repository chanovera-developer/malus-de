use gtk4::gio;
use gtk4::gio::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Entry, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow,
    Spinner, Window,
};
use malus_de_common::is_spanish_locale;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const SUPPORTED_SCHEMES: [&str; 8] = ["smb", "sftp", "nfs", "dav", "davs", "ftp", "ftps", "afp"];
const MAX_RECENT_SERVERS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkMountInfo {
    pub name: String,
    pub uri: String,
    pub root_path: Option<PathBuf>,
}

/// Comprueba si una URI usa un esquema de red soportado por GVfs.
pub fn is_network_uri(uri: &str) -> bool {
    let lower = uri.trim().to_ascii_lowercase();
    let Some((scheme, _)) = lower.split_once("://") else {
        return false;
    };
    SUPPORTED_SCHEMES.contains(&scheme) || scheme == "ssh"
}

/// Valida y normaliza una URI de servidor (`smb://`, `sftp://`, `nfs://`, `dav://`, etc.).
pub fn normalize_network_uri(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(if is_spanish_locale() {
            "Introduce una dirección de servidor (ej. smb://servidor/compartido).".to_string()
        } else {
            "Enter a server address (e.g. smb://server/share).".to_string()
        });
    }

    let Some((raw_scheme, rest)) = trimmed.split_once("://") else {
        return Err(if is_spanish_locale() {
            "La dirección debe incluir el protocolo (smb://, sftp://, nfs://, dav://).".to_string()
        } else {
            "Address must include a protocol scheme (smb://, sftp://, nfs://, dav://)."
                .to_string()
        });
    };

    let lower_scheme = raw_scheme.to_ascii_lowercase();
    let canonical_scheme = if lower_scheme == "ssh" {
        "sftp".to_string()
    } else if SUPPORTED_SCHEMES.contains(&lower_scheme.as_str()) {
        lower_scheme
    } else {
        return Err(if is_spanish_locale() {
            format!(
                "Protocolo «{}» no soportado. Usa smb://, sftp://, nfs://, dav:// o ftp://.",
                raw_scheme
            )
        } else {
            format!(
                "Unsupported protocol \"{}\". Use smb://, sftp://, nfs://, dav://, or ftp://.",
                raw_scheme
            )
        });
    };

    let rest_trimmed = rest.trim();
    let authority = rest_trimmed.split('/').next().unwrap_or("");
    let host_part = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim();
    if host_part.is_empty() {
        return Err(if is_spanish_locale() {
            "Falta el nombre de host o IP del servidor.".to_string()
        } else {
            "Server hostname or IP address is missing.".to_string()
        });
    }

    Ok(format!("{}://{}", canonical_scheme, rest_trimmed))
}

/// Indica si una ruta pertenece a un punto de montaje GVfs (`/run/user/<uid>/gvfs/<recurso>`).
pub fn is_gvfs_mount_path(path: &Path) -> bool {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    // Esperamos: ["run", "user", "<uid>", "gvfs", "<mount_name>", ...]
    parts.len() >= 5 && parts[0] == "run" && parts[1] == "user" && parts[3] == "gvfs"
}

/// Devuelve el directorio raíz FUSE donde GVfs expone los montajes de usuario (`$XDG_RUNTIME_DIR/gvfs`).
pub fn gvfs_mounts_root() -> Option<PathBuf> {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let candidate = PathBuf::from(runtime_dir).join("gvfs");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

/// Indica si un `gio::Mount` representa un recurso de red remoto.
pub fn is_network_gio_mount(mount: &gio::Mount) -> bool {
    let root_uri = mount.root().uri();
    if is_network_uri(root_uri.as_str()) {
        return true;
    }
    let default_uri = mount.default_location().uri();
    if is_network_uri(default_uri.as_str()) {
        return true;
    }
    mount
        .root()
        .path()
        .is_some_and(|p| is_gvfs_mount_path(&p))
}

/// Lista los montajes de red GVfs actualmente activos en la sesión.
pub fn list_active_network_mounts() -> Vec<NetworkMountInfo> {
    let monitor = gio::VolumeMonitor::get();
    let mut list = Vec::new();
    for mount in monitor.mounts() {
        if mount.is_shadowed() || !is_network_gio_mount(&mount) {
            continue;
        }
        let root_file = mount.root();
        let default_file = mount.default_location();
        let uri = root_file.uri().to_string();
        let root_path = root_file.path().or_else(|| default_file.path());
        list.push(NetworkMountInfo {
            name: mount.name().to_string(),
            uri,
            root_path,
        });
    }
    list
}

fn recent_servers_file_path() -> Option<PathBuf> {
    Some(glib::user_config_dir().join("malus-de").join("branches-servers.txt"))
}

pub fn load_recent_servers_from_file(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && is_network_uri(line))
        .take(MAX_RECENT_SERVERS)
        .map(ToString::to_string)
        .collect()
}

pub fn save_recent_server_to_file(path: &Path, uri: &str) {
    let Ok(normalized) = normalize_network_uri(uri) else {
        return;
    };
    let mut items = load_recent_servers_from_file(path);
    items.retain(|existing| existing != &normalized);
    items.insert(0, normalized);
    items.truncate(MAX_RECENT_SERVERS);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, items.join("\n"));
}

fn resolve_mounted_local_path(target_file: &gio::File, target_uri: &str) -> Option<PathBuf> {
    if let Some(local) = target_file.path() {
        if local.exists() {
            return Some(local);
        }
    }
    let prefix = target_uri.trim_end_matches('/');
    for info in list_active_network_mounts() {
        let mount_uri = info.uri.trim_end_matches('/');
        if mount_uri == prefix || prefix.starts_with(mount_uri) || mount_uri.starts_with(prefix) {
            if let Some(p) = info.root_path {
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    gvfs_mounts_root()
}

/// Abre la ruta adecuada para «Red» / «Network»: si existen montajes GVfs activos navega a ellos;
/// de lo contrario abre el diálogo «Conectar al servidor…».
pub fn open_network_location<F: Fn(PathBuf) + 'static>(parent: Option<&Window>, on_navigate: F) {
    let mounts = list_active_network_mounts();
    if let Some(first_path) = mounts.iter().find_map(|m| m.root_path.clone()) {
        if let Some(gvfs_root) = gvfs_mounts_root() {
            on_navigate(gvfs_root);
        } else {
            on_navigate(first_path);
        }
        return;
    }

    if let Ok(entries) = std::fs::read_dir("/mnt") {
        if entries.flatten().next().is_some() {
            on_navigate(PathBuf::from("/mnt"));
            return;
        }
    }

    if let Some(win) = parent {
        open_connect_server_dialog(win, on_navigate);
    } else if let Some(gvfs_root) = gvfs_mounts_root() {
        on_navigate(gvfs_root);
    } else {
        on_navigate(PathBuf::from("/mnt"));
    }
}

/// Muestra el diálogo macOS/Finder «Conectar al servidor…» (`Ctrl/Cmd+K`) y monta por GVfs.
pub fn open_connect_server_dialog<F: Fn(PathBuf) + 'static>(parent: &Window, on_mounted: F) {
    let es = is_spanish_locale();
    let on_mounted: Rc<dyn Fn(PathBuf)> = Rc::new(on_mounted);

    let dialog = Window::builder()
        .title(if es {
            "Conectar al servidor"
        } else {
            "Connect to Server"
        })
        .transient_for(parent)
        .modal(true)
        .default_width(470)
        .resizable(false)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 12);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let heading = Label::builder()
        .label(if es {
            "Dirección del servidor:"
        } else {
            "Server Address:"
        })
        .halign(Align::Start)
        .css_classes(["heading"])
        .build();
    root.append(&heading);

    let uri_entry = Entry::builder()
        .placeholder_text("smb://servidor/compartido  o  sftp://usuario@servidor/ruta")
        .hexpand(true)
        .build();
    root.append(&uri_entry);

    // Accesos rápidos de protocolo
    let scheme_bar = GtkBox::new(Orientation::Horizontal, 6);
    for scheme in ["smb://", "sftp://", "nfs://", "dav://"] {
        let chip = Button::with_label(scheme);
        chip.add_css_class("flat");
        chip.add_css_class("caption");
        let entry_c = uri_entry.clone();
        chip.connect_clicked(move |_| {
            let current = entry_c.text().to_string();
            let stripped = current
                .split_once("://")
                .map(|(_, rest)| rest.to_string())
                .unwrap_or(current);
            entry_c.set_text(&format!("{}{}", scheme, stripped));
            entry_c.set_position(-1);
            entry_c.grab_focus();
        });
        scheme_bar.append(&chip);
    }
    root.append(&scheme_bar);

    let status_row = GtkBox::new(Orientation::Horizontal, 8);
    let spinner = Spinner::new();
    spinner.set_visible(false);
    let status_label = Label::builder()
        .halign(Align::Start)
        .wrap(true)
        .hexpand(true)
        .css_classes(["dim-label", "caption"])
        .build();
    status_row.append(&spinner);
    status_row.append(&status_label);
    root.append(&status_row);

    // Lista de servidores recientes y montajes activos
    let recent_paths = recent_servers_file_path()
        .map(|p| load_recent_servers_from_file(&p))
        .unwrap_or_default();
    let active_mounts = list_active_network_mounts();

    if !active_mounts.is_empty() || !recent_paths.is_empty() {
        let list_title = Label::builder()
            .label(if es {
                "Servidores recientes y montados"
            } else {
                "Recent & Mounted Servers"
            })
            .halign(Align::Start)
            .css_classes(["dim-label", "caption"])
            .build();
        root.append(&list_title);

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();

        for mount in &active_mounts {
            let row = ListBoxRow::new();
            let hbox = GtkBox::new(Orientation::Horizontal, 8);
            hbox.set_margin_top(6);
            hbox.set_margin_bottom(6);
            hbox.set_margin_start(10);
            hbox.set_margin_end(10);
            let badge = Label::builder()
                .label(if es { "Conectado" } else { "Mounted" })
                .css_classes(["success", "caption"])
                .build();
            let lbl = Label::builder()
                .label(&format!("{} ({})", mount.name, mount.uri))
                .halign(Align::Start)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            hbox.append(&lbl);
            hbox.append(&badge);
            row.set_child(Some(&hbox));
            unsafe {
                row.set_data("server_uri", mount.uri.clone());
                if let Some(p) = &mount.root_path {
                    row.set_data("mounted_path", p.clone());
                }
            }
            list_box.append(&row);
        }

        for recent in &recent_paths {
            if active_mounts.iter().any(|m| &m.uri == recent) {
                continue;
            }
            let row = ListBoxRow::new();
            let lbl = Label::builder()
                .label(recent)
                .halign(Align::Start)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(10)
                .margin_end(10)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            row.set_child(Some(&lbl));
            unsafe {
                row.set_data("server_uri", recent.clone());
            }
            list_box.append(&row);
        }

        let entry_c = uri_entry.clone();
        let on_mounted_c = on_mounted.clone();
        let dlg_c = dialog.clone();
        list_box.connect_row_activated(move |_, row| {
            if let Some(mounted_ptr) = unsafe { row.data::<PathBuf>("mounted_path") } {
                let p = unsafe { mounted_ptr.as_ref() }.clone();
                if p.exists() {
                    on_mounted_c(p);
                    dlg_c.close();
                    return;
                }
            }
            if let Some(uri_ptr) = unsafe { row.data::<String>("server_uri") } {
                let uri = unsafe { uri_ptr.as_ref() }.clone();
                entry_c.set_text(&uri);
                entry_c.set_position(-1);
            }
        });

        let scroll = ScrolledWindow::builder()
            .min_content_height(110)
            .max_content_height(170)
            .child(&list_box)
            .build();
        root.append(&scroll);
    }

    let actions_row = GtkBox::new(Orientation::Horizontal, 8);
    actions_row.set_halign(Align::End);
    let cancel_btn = Button::with_label(if es { "Cancelar" } else { "Cancel" });
    let connect_btn = Button::with_label(if es { "Conectar" } else { "Connect" });
    connect_btn.add_css_class("suggested-action");
    actions_row.append(&cancel_btn);
    actions_row.append(&connect_btn);
    root.append(&actions_row);

    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dlg_cancel.close();
    });

    let perform_connect: Rc<dyn Fn()> = Rc::new({
        let uri_entry = uri_entry.clone();
        let status_label = status_label.clone();
        let spinner = spinner.clone();
        let connect_btn = connect_btn.clone();
        let dialog = dialog.clone();
        let on_mounted = on_mounted.clone();
        move || {
            let normalized = match normalize_network_uri(uri_entry.text().as_str()) {
                Ok(uri) => uri,
                Err(msg) => {
                    status_label.set_label(&msg);
                    status_label.remove_css_class("dim-label");
                    status_label.add_css_class("error");
                    return;
                }
            };

            if let Some(cfg_path) = recent_servers_file_path() {
                save_recent_server_to_file(&cfg_path, &normalized);
            }

            status_label.remove_css_class("error");
            status_label.add_css_class("dim-label");
            status_label.set_label(if es {
                "Conectando mediante GVfs…"
            } else {
                "Connecting via GVfs…"
            });
            spinner.set_visible(true);
            spinner.start();
            connect_btn.set_sensitive(false);

            let target_file = gio::File::for_uri(&normalized);
            if let Some(existing_local) = resolve_mounted_local_path(&target_file, &normalized) {
                // Solo saltar directo si ya está montado en el VolumeMonitor
                let already_in_monitor = list_active_network_mounts().iter().any(|m| {
                    let mu = m.uri.trim_end_matches('/');
                    let tu = normalized.trim_end_matches('/');
                    mu == tu || tu.starts_with(mu)
                });
                if already_in_monitor {
                    spinner.stop();
                    spinner.set_visible(false);
                    on_mounted(existing_local);
                    dialog.close();
                    return;
                }
            }

            let mount_op = gtk4::MountOperation::new(Some(&dialog));
            let spinner_c = spinner.clone();
            let status_c = status_label.clone();
            let btn_c = connect_btn.clone();
            let dlg_c = dialog.clone();
            let on_mounted_c = on_mounted.clone();
            let file_c = target_file.clone();
            let uri_c = normalized.clone();

            target_file.mount_enclosing_volume(
                gio::MountMountFlags::NONE,
                Some(&mount_op),
                gio::Cancellable::NONE,
                move |res| {
                    spinner_c.stop();
                    spinner_c.set_visible(false);
                    btn_c.set_sensitive(true);

                    let already_mounted = res
                        .as_ref()
                        .err()
                        .and_then(|e| e.kind::<gio::IOErrorEnum>())
                        == Some(gio::IOErrorEnum::AlreadyMounted);

                    if res.is_ok() || already_mounted {
                        if let Some(local_path) = resolve_mounted_local_path(&file_c, &uri_c) {
                            on_mounted_c(local_path);
                            dlg_c.close();
                        } else {
                            status_c.remove_css_class("dim-label");
                            status_c.add_css_class("error");
                            status_c.set_label(if es {
                                "Conectado, pero no se encontró punto de montaje FUSE local (¿gvfs-fuse activo?)."
                            } else {
                                "Connected, but no local FUSE mount point was found (is gvfs-fuse running?)."
                            });
                        }
                    } else if let Err(err) = res {
                        status_c.remove_css_class("dim-label");
                        status_c.add_css_class("error");
                        status_c.set_label(&err.message());
                    }
                },
            );
        }
    });

    let pc_btn = perform_connect.clone();
    connect_btn.connect_clicked(move |_| pc_btn());

    let pc_entry = perform_connect.clone();
    uri_entry.connect_activate(move |_| pc_entry());

    // Escape cierra el diálogo
    let key_ctrl = gtk4::EventControllerKey::new();
    let dlg_esc = dialog.clone();
    key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gtk4::gdk::Key::Escape {
            dlg_esc.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(key_ctrl);

    dialog.present();
    uri_entry.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_network_uri_accepts_and_canonicalizes_supported_schemes() {
        assert_eq!(
            normalize_network_uri("  SMB://nas.local/media ").unwrap(),
            "smb://nas.local/media"
        );
        assert_eq!(
            normalize_network_uri("ssh://user@192.168.1.10/home/user").unwrap(),
            "sftp://user@192.168.1.10/home/user"
        );
        assert_eq!(
            normalize_network_uri("nfs://fileserver/srv/share").unwrap(),
            "nfs://fileserver/srv/share"
        );
        assert_eq!(
            normalize_network_uri("davs://cloud.example.com/remote.php/dav").unwrap(),
            "davs://cloud.example.com/remote.php/dav"
        );
    }

    #[test]
    fn normalize_network_uri_rejects_invalid_inputs() {
        assert!(normalize_network_uri("").is_err());
        assert!(normalize_network_uri("nas.local/share").is_err());
        assert!(normalize_network_uri("http://example.com").is_err());
        assert!(normalize_network_uri("smb://").is_err());
        assert!(normalize_network_uri("sftp://user@/path").is_err());
    }

    #[test]
    fn gvfs_mount_path_detection_distinguishes_system_and_network_paths() {
        assert!(is_gvfs_mount_path(Path::new(
            "/run/user/1000/gvfs/smb-share:server=nas,share=docs"
        )));
        assert!(!is_gvfs_mount_path(Path::new("/run/user/1000/gvfs")));
        assert!(!is_gvfs_mount_path(Path::new("/run/media/user/USB")));
        assert!(!is_gvfs_mount_path(Path::new("/home/user")));
    }

    #[test]
    fn recent_servers_roundtrip_deduplicates_and_orders_latest_first() {
        let base = std::env::temp_dir().join(format!(
            "malus_branches_net_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cfg = base.join("servers.txt");
        save_recent_server_to_file(&cfg, "smb://nas/one");
        save_recent_server_to_file(&cfg, "ssh://alice@host/two");
        save_recent_server_to_file(&cfg, "SMB://nas/one");
        let loaded = load_recent_servers_from_file(&cfg);
        assert_eq!(
            loaded,
            vec!["smb://nas/one", "sftp://alice@host/two"]
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
