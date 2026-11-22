use crate::fs_backend::{format_size, FileItem};
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, CheckButton, DropDown, Grid, Label,
    Orientation, PolicyType, ScrolledWindow, Separator, StringList, Window,
};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirStats {
    pub total_bytes: u64,
    pub item_count: usize,
}

pub fn compute_dir_stats_cancellable(path: &Path, cancel: &AtomicBool) -> Option<DirStats> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut stats = DirStats::default();
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Some(stats);
    };
    if !meta.is_dir() {
        stats.total_bytes = meta.len();
        stats.item_count = 1;
        return Some(stats);
    }

    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let p = entry.path();
            if let Ok(m) = fs::symlink_metadata(&p) {
                stats.item_count += 1;
                if m.is_dir() && !m.file_type().is_symlink() {
                    stack.push(p);
                } else {
                    stats.total_bytes = stats.total_bytes.saturating_add(m.len());
                }
            }
        }
    }
    Some(stats)
}

pub fn show_get_info_window(
    parent: &ApplicationWindow,
    path: PathBuf,
    on_tags_changed: Rc<dyn Fn()>,
) {
    show_get_info_impl(Some(parent), None, path, on_tags_changed);
}

pub fn show_get_info_standalone(app: &gtk4::Application, path: PathBuf) {
    show_get_info_impl(None, Some(app), path, Rc::new(|| {}));
}

fn show_get_info_impl(
    parent: Option<&ApplicationWindow>,
    app: Option<&gtk4::Application>,
    path: PathBuf,
    on_tags_changed: Rc<dyn Fn()>,
) {
    let es = malus_de_common::is_spanish_locale();
    let Some(item) = FileItem::from_path(&path) else {
        return;
    };

    let mut builder = Window::builder()
        .default_width(360)
        .default_height(520)
        .title(if es {
            format!("Información de {}", item.name)
        } else {
            format!("{} Info", item.name)
        })
        .css_classes(["branches-info-window"]);
    if let Some(p) = parent {
        builder = builder.transient_for(p).destroy_with_parent(true);
    }
    if let Some(a) = app {
        builder = builder.application(a);
    }
    let win = builder.build();

    let outer = GtkBox::new(Orientation::Vertical, 12);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);
    outer.set_margin_start(16);
    outer.set_margin_end(16);

    // 1. Cabecera (Icono + Nombre + Tamaño)
    let header = GtkBox::new(Orientation::Horizontal, 12);
    let icon = crate::views::create_item_image(&item, 48, Some("branches-info-thumb"));
    icon.set_size_request(48, 48);
    icon.set_valign(Align::Center);
    header.append(&icon);

    let title_col = GtkBox::new(Orientation::Vertical, 2);
    title_col.set_hexpand(true);
    title_col.set_valign(Align::Center);
    let name_lbl = Label::builder()
        .label(&item.name)
        .halign(Align::Start)
        .wrap(true)
        .css_classes(["title-3"])
        .build();
    let size_lbl = Label::builder()
        .label(if item.is_dir {
            if es {
                "Calculando tamaño…"
            } else {
                "Calculating size…"
            }
        } else {
            &item.size_str
        })
        .halign(Align::Start)
        .css_classes(["dim-label"])
        .build();
    title_col.append(&name_lbl);
    title_col.append(&size_lbl);
    header.append(&title_col);
    outer.append(&header);

    // Cálculo asíncrono de tamaño para carpetas
    let cancel_stats = Arc::new(AtomicBool::new(false));
    if item.is_dir {
        let cancel_worker = cancel_stats.clone();
        let dir_path = path.clone();
        let (tx, rx) = mpsc::channel::<DirStats>();
        std::thread::spawn(move || {
            if let Some(stats) = compute_dir_stats_cancellable(&dir_path, &cancel_worker) {
                let _ = tx.send(stats);
            }
        });
        let size_lbl_ref = size_lbl.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(stats) => {
                    let txt = if es {
                        format!(
                            "{} ({} elementos)",
                            format_size(stats.total_bytes),
                            stats.item_count
                        )
                    } else {
                        format!(
                            "{} ({} items)",
                            format_size(stats.total_bytes),
                            stats.item_count
                        )
                    };
                    size_lbl_ref.set_label(&txt);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    let cancel_on_close = cancel_stats;
    win.connect_close_request(move |_| {
        cancel_on_close.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });

    outer.append(&Separator::new(Orientation::Horizontal));

    // 2. General
    let general_title = Label::builder()
        .label(if es { "General" } else { "General" })
        .halign(Align::Start)
        .css_classes(["heading"])
        .build();
    outer.append(&general_title);

    let grid = Grid::builder().row_spacing(6).column_spacing(12).build();
    let mut row = 0;
    let add_row = |g: &Grid, r: &mut i32, key: &str, val: &str| {
        let k = Label::builder()
            .label(key)
            .halign(Align::End)
            .css_classes(["dim-label"])
            .build();
        let v = Label::builder()
            .label(val)
            .halign(Align::Start)
            .hexpand(true)
            .selectable(true)
            .wrap(true)
            .build();
        g.attach(&k, 0, *r, 1, 1);
        g.attach(&v, 1, *r, 1, 1);
        *r += 1;
    };

    add_row(
        &grid,
        &mut row,
        if es { "Clase:" } else { "Kind:" },
        &item.kind_str,
    );
    let where_str = path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    add_row(
        &grid,
        &mut row,
        if es { "Ubicación:" } else { "Where:" },
        &where_str,
    );
    add_row(
        &grid,
        &mut row,
        if es { "Modificado:" } else { "Modified:" },
        &item.modified_str,
    );
    if item.is_symlink {
        if let Ok(target) = fs::read_link(&path) {
            add_row(
                &grid,
                &mut row,
                if es { "Original:" } else { "Original:" },
                &target.to_string_lossy(),
            );
        }
    }
    outer.append(&grid);

    // 3. Etiquetas
    outer.append(&Separator::new(Orientation::Horizontal));
    let tags_title = Label::builder()
        .label(if es { "Etiquetas" } else { "Tags" })
        .halign(Align::Start)
        .css_classes(["heading"])
        .build();
    outer.append(&tags_title);

    let tags_row = GtkBox::new(Orientation::Horizontal, 6);
    let tags_status = Label::builder()
        .halign(Align::Start)
        .css_classes(["dim-label"])
        .build();
    let refresh_tags_status = {
        let path_ref = path.clone();
        let lbl_ref = tags_status.clone();
        Rc::new(move || {
            let names: Vec<String> = crate::db::open_db()
                .map(|c| {
                    crate::db::get_file_tags(&c, &path_ref)
                        .into_iter()
                        .map(|(id, _)| crate::db::tag_display_label(&id, es))
                        .collect()
                })
                .unwrap_or_default();
            if names.is_empty() {
                lbl_ref.set_label(if es { "Sin etiquetas" } else { "No tags" });
            } else {
                lbl_ref.set_label(&names.join(", "));
            }
        })
    };
    refresh_tags_status();

    for def in crate::db::TAG_CATALOG.iter() {
        let btn = Button::builder()
            .has_frame(false)
            .tooltip_text(if es { def.label_es } else { def.label_en })
            .css_classes([
                "branches-context-tag-btn",
                &format!("branches-tag-color-{}", def.id),
            ])
            .build();
        let p_tag = path.clone();
        let tid = def.id.to_string();
        let ref_lbl = refresh_tags_status.clone();
        let cb_changed = on_tags_changed.clone();
        btn.connect_clicked(move |_| {
            if let Ok(conn) = crate::db::open_db() {
                let _ = crate::db::toggle_file_tag(&conn, &p_tag, &tid);
            }
            ref_lbl();
            cb_changed();
        });
        tags_row.append(&btn);
    }
    let clear_btn = Button::builder()
        .icon_name("edit-clear-symbolic")
        .has_frame(false)
        .tooltip_text(if es { "Quitar etiquetas" } else { "Clear Tags" })
        .build();
    {
        let p_clr = path.clone();
        let ref_lbl = refresh_tags_status.clone();
        let cb_changed = on_tags_changed;
        clear_btn.connect_clicked(move |_| {
            if let Ok(conn) = crate::db::open_db() {
                let _ = crate::db::clear_file_tags(&conn, &p_clr);
            }
            ref_lbl();
            cb_changed();
        });
    }
    tags_row.append(&clear_btn);
    outer.append(&tags_row);
    outer.append(&tags_status);

    // 4. Abrir con… (para archivos)
    if !item.is_dir {
        let (ctype, _) = gio::content_type_guess(Some(&path), &[]);
        let apps = gio::AppInfo::all_for_type(&ctype);
        if !apps.is_empty() {
            outer.append(&Separator::new(Orientation::Horizontal));
            let open_with_title = Label::builder()
                .label(if es { "Abrir con:" } else { "Open with:" })
                .halign(Align::Start)
                .css_classes(["heading"])
                .build();
            outer.append(&open_with_title);

            let default_app = gio::AppInfo::default_for_type(&ctype, false);
            let names: Vec<String> = apps.iter().map(|a| a.display_name().to_string()).collect();
            let model = StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
            let dropdown = DropDown::new(Some(model), None::<gtk4::Expression>);
            if let Some(ref def) = default_app {
                if let Some(pos) = apps.iter().position(|a| a.equal(def)) {
                    dropdown.set_selected(pos as u32);
                }
            }

            let apps_rc = Rc::new(apps);
            let change_all_btn = Button::builder()
                .label(if es {
                    "Cambiar todo…"
                } else {
                    "Change All…"
                })
                .halign(Align::End)
                .build();
            let status_app = Label::builder()
                .halign(Align::Start)
                .css_classes(["dim-label"])
                .build();
            let dd_ref = dropdown.clone();
            let apps_clone = apps_rc.clone();
            let ctype_str = ctype.to_string();
            let status_ref = status_app.clone();
            change_all_btn.connect_clicked(move |_| {
                let idx = dd_ref.selected() as usize;
                if let Some(app_info) = apps_clone.get(idx) {
                    if app_info.set_as_default_for_type(&ctype_str).is_ok() {
                        status_ref.set_label(if es {
                            "Aplicación predeterminada actualizada"
                        } else {
                            "Default application updated"
                        });
                    }
                }
            });

            let ow_row = GtkBox::new(Orientation::Horizontal, 8);
            dropdown.set_hexpand(true);
            ow_row.append(&dropdown);
            ow_row.append(&change_all_btn);
            outer.append(&ow_row);
            outer.append(&status_app);
        }
    }

    // 5. Permisos POSIX
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.file_type().is_symlink() {
                outer.append(&Separator::new(Orientation::Horizontal));
                let perm_title = Label::builder()
                    .label(if es {
                        "Compartir y permisos"
                    } else {
                        "Sharing & Permissions"
                    })
                    .halign(Align::Start)
                    .css_classes(["heading"])
                    .build();
                outer.append(&perm_title);

                let mode_cell = Rc::new(RefCell::new(meta.permissions().mode()));
                let mode_lbl = Label::builder()
                    .label(&format!(
                        "{}: {:04o}",
                        if es { "Modo octal" } else { "Octal mode" },
                        meta.permissions().mode() & 0o7777
                    ))
                    .halign(Align::Start)
                    .css_classes(["dim-label"])
                    .build();

                let pgrid = Grid::builder().row_spacing(4).column_spacing(10).build();
                let col_headers = [
                    if es { "Lectura" } else { "Read" },
                    if es { "Escritura" } else { "Write" },
                    if es { "Ejecución" } else { "Exec" },
                ];
                for (c, h) in col_headers.iter().enumerate() {
                    let lbl = Label::builder()
                        .label(*h)
                        .css_classes(["dim-label"])
                        .build();
                    pgrid.attach(&lbl, (c + 1) as i32, 0, 1, 1);
                }

                let groups: [(&str, [u32; 3]); 3] = [
                    (
                        if es { "Propietario" } else { "Owner" },
                        [0o400, 0o200, 0o100],
                    ),
                    (if es { "Grupo" } else { "Group" }, [0o040, 0o020, 0o010]),
                    (if es { "Otros" } else { "Everyone" }, [0o004, 0o002, 0o001]),
                ];
                let perm_err_lbl = Label::builder()
                    .halign(Align::Start)
                    .css_classes(["error"])
                    .build();

                for (r_idx, (gname, bits)) in groups.iter().enumerate() {
                    let g_lbl = Label::builder().label(*gname).halign(Align::Start).build();
                    pgrid.attach(&g_lbl, 0, (r_idx + 1) as i32, 1, 1);
                    for (c_idx, &bit) in bits.iter().enumerate() {
                        let chk = CheckButton::new();
                        chk.set_active((*mode_cell.borrow() & bit) != 0);
                        chk.set_halign(Align::Center);
                        let p_perm = path.clone();
                        let mc = mode_cell.clone();
                        let ml = mode_lbl.clone();
                        let pel = perm_err_lbl.clone();
                        chk.connect_toggled(move |cb| {
                            let mut cur = *mc.borrow();
                            if cb.is_active() {
                                cur |= bit;
                            } else {
                                cur &= !bit;
                            }
                            let perms = fs::Permissions::from_mode(cur);
                            match fs::set_permissions(&p_perm, perms) {
                                Ok(()) => {
                                    *mc.borrow_mut() = cur;
                                    ml.set_label(&format!(
                                        "{}: {:04o}",
                                        if es { "Modo octal" } else { "Octal mode" },
                                        cur & 0o7777
                                    ));
                                    pel.set_label("");
                                }
                                Err(e) => {
                                    pel.set_label(&format!("chmod: {}", e));
                                }
                            }
                        });
                        pgrid.attach(&chk, (c_idx + 1) as i32, (r_idx + 1) as i32, 1, 1);
                    }
                }
                outer.append(&pgrid);
                outer.append(&mode_lbl);
                outer.append(&perm_err_lbl);
            }
        }
    }

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&outer)
        .build();
    win.set_child(Some(&scrolled));
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_stats_computes_recursive_size_and_respects_cancel() {
        let root = std::env::temp_dir().join("malus_branches_info_stats_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.bin"), vec![1u8; 100]).unwrap();
        fs::write(root.join("sub/b.bin"), vec![2u8; 250]).unwrap();

        let cancel = AtomicBool::new(false);
        let stats = compute_dir_stats_cancellable(&root, &cancel).unwrap();
        assert_eq!(stats.total_bytes, 350);
        assert_eq!(stats.item_count, 3); // sub + a.bin + b.bin

        cancel.store(true, Ordering::Relaxed);
        assert!(compute_dir_stats_cancellable(&root, &cancel).is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
