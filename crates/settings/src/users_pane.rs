use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, CheckButton, DrawingArea, DropDown, Entry,
    Image, Label, Orientation, PasswordEntry, PolicyType, Popover, ScrolledWindow, Spinner,
    StringList, Window,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const USERS_PANEL_ID: &str = "users";
pub const USERS_PANEL_TITLE: &str = "Usuarios y grupos";

const GDM_CUSTOM_CONF: &str = "/etc/gdm/custom.conf";

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemUser {
    username: String,
    full_name: String,
    uid: u32,
    gid: u32,
    home: String,
    shell: String,
    is_admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemGroup {
    name: String,
    gid: u32,
    members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsersSnapshot {
    users: Vec<SystemUser>,
    groups: Vec<SystemGroup>,
    autologin_user: Option<String>,
    luks_active: bool,
}

fn current_username() -> String {
    std::env::var("USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "usuario".to_string())
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn suggest_username(full_name: &str) -> String {
    let mut slug = String::new();
    for ch in full_name.trim().chars() {
        let lower = match ch {
            'á' | 'Á' => 'a',
            'é' | 'É' => 'e',
            'í' | 'Í' => 'i',
            'ó' | 'Ó' => 'o',
            'ú' | 'Ú' | 'ü' | 'Ü' => 'u',
            'ñ' | 'Ñ' => 'n',
            c => c.to_ascii_lowercase(),
        };
        if lower.is_ascii_alphanumeric() {
            slug.push(lower);
        } else if (lower.is_whitespace() || lower == '-' || lower == '_') && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("u{trimmed}")
    } else {
        trimmed.chars().take(28).collect()
    }
}

fn is_valid_linux_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn parse_passwd_and_group(
    passwd_raw: &str,
    group_raw: &str,
) -> (Vec<SystemUser>, Vec<SystemGroup>) {
    let mut admin_users: HashSet<String> = HashSet::new();
    let mut all_groups: Vec<SystemGroup> = Vec::new();
    let mut user_primary_gids: HashMap<String, u32> = HashMap::new();

    for line in group_raw.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let gname = parts[0].trim();
        let Ok(gid) = parts[2].trim().parse::<u32>() else {
            continue;
        };
        let members: Vec<String> = parts[3]
            .split(',')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .collect();
        if gname == "wheel" || gname == "sudo" {
            for m in &members {
                admin_users.insert(m.clone());
            }
        }
        if (1000..65534).contains(&gid) {
            all_groups.push(SystemGroup {
                name: gname.to_string(),
                gid,
                members,
            });
        }
    }

    let mut users = Vec::new();
    for line in passwd_raw.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 7 {
            continue;
        }
        let username = parts[0].trim();
        let Ok(uid) = parts[2].trim().parse::<u32>() else {
            continue;
        };
        let Ok(gid) = parts[3].trim().parse::<u32>() else {
            continue;
        };
        let gecos_full = parts[4].split(',').next().unwrap_or("").trim();
        let home = parts[5].trim();
        let shell = parts[6].trim();

        if !(1000..65534).contains(&uid)
            || username == "nobody"
            || shell.ends_with("/nologin")
            || shell.ends_with("/false")
        {
            continue;
        }
        let full_name = if gecos_full.is_empty() {
            capitalize_first(username)
        } else {
            gecos_full.to_string()
        };
        user_primary_gids.insert(username.to_string(), gid);
        users.push(SystemUser {
            username: username.to_string(),
            full_name,
            uid,
            gid,
            home: home.to_string(),
            shell: shell.to_string(),
            is_admin: admin_users.contains(username),
        });
    }

    // Mostrar solo grupos personalizados/compartidos (excluir el grupo privado de cada usuario)
    let custom_groups: Vec<SystemGroup> = all_groups
        .into_iter()
        .filter(|g| user_primary_gids.get(&g.name).copied() != Some(g.gid))
        .collect();

    users.sort_by(|a, b| a.uid.cmp(&b.uid));
    (users, custom_groups)
}

fn parse_gdm_autologin(content: &str) -> Option<String> {
    let mut in_daemon = false;
    let mut enabled = false;
    let mut user: Option<String> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_daemon = line.eq_ignore_ascii_case("[daemon]");
            continue;
        }
        if !in_daemon {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "AutomaticLoginEnable" => {
                    enabled = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
                }
                "AutomaticLogin" => {
                    let u = v.trim();
                    if !u.is_empty() {
                        user = Some(u.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    if enabled {
        user
    } else {
        None
    }
}

fn update_gdm_autologin_content(content: &str, autologin_user: Option<&str>) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_daemon = false;
    let mut seen_daemon = false;
    let mut wrote_block = false;

    let write_entries = |lines: &mut Vec<String>, target: Option<&str>| {
        if let Some(u) = target {
            lines.push("AutomaticLoginEnable=True".to_string());
            lines.push(format!("AutomaticLogin={u}"));
        } else {
            lines.push("AutomaticLoginEnable=False".to_string());
        }
    };

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_daemon && !wrote_block {
                write_entries(&mut out_lines, autologin_user);
                wrote_block = true;
            }
            in_daemon = trimmed.eq_ignore_ascii_case("[daemon]");
            if in_daemon {
                seen_daemon = true;
            }
            out_lines.push(raw_line.to_string());
            continue;
        }
        if in_daemon {
            if let Some((k, _)) = trimmed.split_once('=') {
                let key = k.trim();
                if key == "AutomaticLoginEnable" || key == "AutomaticLogin" {
                    continue;
                }
            }
        }
        out_lines.push(raw_line.to_string());
    }

    if in_daemon && !wrote_block {
        write_entries(&mut out_lines, autologin_user);
    } else if !seen_daemon {
        if !out_lines.is_empty() {
            out_lines.push(String::new());
        }
        out_lines.push("[daemon]".to_string());
        write_entries(&mut out_lines, autologin_user);
    }

    let mut result = out_lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn probe_luks_active() -> bool {
    Command::new("lsblk")
        .args(["-o", "TYPE", "-rn"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim() == "crypt")
        })
}

fn probe_users_snapshot() -> UsersSnapshot {
    let passwd_raw = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let group_raw = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let (users, groups) = parse_passwd_and_group(&passwd_raw, &group_raw);
    let gdm_raw = std::fs::read_to_string(GDM_CUSTOM_CONF).unwrap_or_default();
    let autologin_user = parse_gdm_autologin(&gdm_raw);
    let luks_active = probe_luks_active();
    UsersSnapshot {
        users,
        groups,
        autologin_user,
        luks_active,
    }
}

fn find_user_avatar_path(username: &str, home: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("/var/lib/AccountsService/icons").join(username),
        PathBuf::from(home).join(".face"),
        PathBuf::from(home).join(".face.icon"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn make_user_avatar_widget(username: &str, full_name: &str, home: &str, size: i32) -> DrawingArea {
    let pixbuf = find_user_avatar_path(username, home).and_then(|p| {
        let loaded = gdk_pixbuf::Pixbuf::from_file(&p).ok()?;
        let pb = loaded.apply_embedded_orientation().unwrap_or(loaded);
        let side = pb.width().min(pb.height()).max(1);
        let cx = (pb.width() - side) / 2;
        let cy = (pb.height() - side) / 2;
        let cropped = pb.new_subpixbuf(cx, cy, side, side);
        cropped.scale_simple(size * 2, size * 2, gdk_pixbuf::InterpType::Bilinear)
    });
    let initial = full_name
        .chars()
        .next()
        .or_else(|| username.chars().next())
        .unwrap_or('U')
        .to_uppercase()
        .to_string();

    let area = DrawingArea::builder()
        .width_request(size)
        .height_request(size)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    area.set_draw_func(move |_, cr, w, h| {
        let w = w as f64;
        let h = h as f64;
        let cx = w * 0.5;
        let cy = h * 0.5;
        let r = (w.min(h) * 0.5) - 0.5;
        if r <= 1.0 {
            return;
        }
        if cr.save().is_ok() {
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            cr.clip();
            if let Some(ref pb) = pixbuf {
                let scale = (r * 2.0) / (pb.width().max(1) as f64);
                cr.translate(cx - r, cy - r);
                cr.scale(scale, scale);
                cr.set_source_pixbuf(pb, 0.0, 0.0);
                let _ = cr.paint();
            } else {
                let grad = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
                grad.add_color_stop_rgb(0.0, 0.58, 0.62, 0.69);
                grad.add_color_stop_rgb(1.0, 0.43, 0.47, 0.54);
                let _ = cr.set_source(&grad);
                let _ = cr.paint();

                cr.set_source_rgba(1.0, 1.0, 1.0, 0.94);
                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Bold,
                );
                cr.set_font_size(r * 0.92);
                if let Ok(ext) = cr.text_extents(&initial) {
                    cr.move_to(cx - ext.width() * 0.5 - ext.x_bearing(), cy + ext.height() * 0.5);
                    let _ = cr.show_text(&initial);
                }
            }
            let _ = cr.restore();
        }
    });
    area
}

fn make_info_circle_button(tooltip: &str) -> Button {
    Button::builder()
        .icon_name("dialog-information-symbolic")
        .css_classes(["circular", "flat"])
        .valign(Align::Center)
        .tooltip_text(tooltip)
        .build()
}

fn show_user_detail_modal(
    parent: &ApplicationWindow,
    user: SystemUser,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(430)
        .css_classes(["macos-settings-window"])
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let is_self = user.username == current_username();
    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    let hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(Align::Center)
        .build();
    hdr.append(&make_user_avatar_widget(
        &user.username,
        &user.full_name,
        &user.home,
        46,
    ));
    let hdr_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    hdr_col.append(
        &Label::builder()
            .label(&user.full_name)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr_col.append(
        &Label::builder()
            .label(&format!(
                "{} · UID {} · {}",
                user.username,
                user.uid,
                if user.is_admin {
                    "Administración"
                } else {
                    "Estándar"
                }
            ))
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    hdr.append(&hdr_col);
    root.append(&hdr);

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    // Fila permisos de administrador
    let admin_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    admin_row.append(
        &Label::builder()
            .label("Permitir que este usuario administre esta computadora")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let admin_sw = Switch::builder()
        .valign(Align::Center)
        .active(user.is_admin)
        .sensitive(!is_self)
        .build();
    admin_row.append(&*admin_sw);
    card.append(&admin_row);

    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Info carpeta personal y shell
    let info_row = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["wifi-pref-row"])
        .build();
    info_row.append(
        &Label::builder()
            .label(&format!("Carpeta personal: {}", user.home))
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    info_row.append(
        &Label::builder()
            .label(&format!("Shell de inicio: {}", user.shell))
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    card.append(&info_row);

    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Cambiar contraseña inline
    let pwd_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();
    pwd_box.append(
        &Label::builder()
            .label("Restablecer contraseña")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let pwd_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let new_pwd = PasswordEntry::builder()
        .placeholder_text("Nueva contraseña (mín. 4 caracteres)")
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    let (set_pwd_wrap, set_pwd_btn) = make_squircle_button("Cambiar", false, None);
    pwd_row.append(&new_pwd);
    pwd_row.append(&set_pwd_wrap);
    pwd_box.append(&pwd_row);
    card.append(&pwd_box);

    root.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let feedback_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    root.append(&feedback_lbl);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .build();

    if !is_self {
        let (del_wrap, del_btn) = make_squircle_button("Eliminar usuario...", false, None);
        let uname_del = user.username.clone();
        let fb_del = feedback_lbl.clone();
        let dlg_del = dialog.clone();
        let cb_del = on_changed.clone();
        del_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let u = uname_del.clone();
            let fb = fb_del.clone();
            let dlg = dlg_del.clone();
            let cb = cb_del.clone();
            let btn_c = btn.clone();
            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || {
                    let out = Command::new("pkexec")
                        .args(["userdel", "-r", &u])
                        .output()
                        .map_err(|e| format!("Error al lanzar userdel: {e}"))?;
                    if out.status.success() {
                        Ok(())
                    } else {
                        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        Err(if e.is_empty() {
                            "No se pudo eliminar el usuario (cancelado o denegado).".to_string()
                        } else {
                            e
                        })
                    }
                })
                .await;
                btn_c.set_sensitive(true);
                match res {
                    Ok(Ok(())) => {
                        cb();
                        crate::close_attached_modal(&dlg);
                    }
                    Ok(Err(msg)) => {
                        fb.set_label(&msg);
                        fb.set_visible(true);
                    }
                    Err(e) => {
                        fb.set_label(&format!("Error interno: {e:?}"));
                        fb.set_visible(true);
                    }
                }
            });
        });
        actions.append(&del_wrap);
    }

    let spacer = GtkBox::builder().hexpand(true).build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&spacer);
    actions.append(&ok_wrap);
    root.append(&actions);

    dialog.set_child(Some(&root));

    // Toggle administrador
    {
        let uname = user.username.clone();
        let fb = feedback_lbl.clone();
        let cb = on_changed.clone();
        let revert_guard = Rc::new(Cell::new(false));
        admin_sw.connect_state_set(move |sw, active| {
            if revert_guard.get() {
                return glib::Propagation::Proceed;
            }
            let u = uname.clone();
            let fb_c = fb.clone();
            let cb_c = cb.clone();
            let sw_c = sw.clone();
            let rg = revert_guard.clone();
            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || {
                    let out = if active {
                        Command::new("pkexec")
                            .args(["usermod", "-aG", "wheel", &u])
                            .output()
                    } else {
                        Command::new("pkexec")
                            .args(["gpasswd", "-d", &u, "wheel"])
                            .output()
                    }
                    .map_err(|e| format!("Error al actualizar grupo wheel: {e}"))?;
                    if out.status.success() {
                        Ok(())
                    } else {
                        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        Err(if e.is_empty() {
                            "Operación cancelada o denegada.".to_string()
                        } else {
                            e
                        })
                    }
                })
                .await;
                match res {
                    Ok(Ok(())) => {
                        fb_c.set_label("Permisos actualizados correctamente.");
                        fb_c.set_visible(true);
                        cb_c();
                    }
                    Ok(Err(msg)) => {
                        rg.set(true);
                        sw_c.set_active(!active);
                        rg.set(false);
                        fb_c.set_label(&msg);
                        fb_c.set_visible(true);
                    }
                    Err(_) => {}
                }
            });
            glib::Propagation::Proceed
        });
    }

    // Cambiar contraseña del usuario
    {
        let uname = user.username.clone();
        let pwd_c = new_pwd;
        let fb = feedback_lbl;
        set_pwd_btn.connect_clicked(move |btn| {
            let p = pwd_c.text().to_string();
            if p.len() < 4 || p.contains('\n') || p.contains('\r') {
                fb.set_label("Introduce una contraseña válida de al menos 4 caracteres.");
                fb.set_visible(true);
                return;
            }
            btn.set_sensitive(false);
            let u = uname.clone();
            let fb_c = fb.clone();
            let pwd_r = pwd_c.clone();
            let btn_c = btn.clone();
            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || -> Result<(), String> {
                    let mut child = Command::new("pkexec")
                        .arg("chpasswd")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|e| format!("Error al lanzar chpasswd: {e}"))?;
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin
                            .write_all(format!("{u}:{p}\n").as_bytes())
                            .map_err(|e| format!("Error al escribir contraseña: {e}"))?;
                    }
                    let out = child
                        .wait_with_output()
                        .map_err(|e| format!("Error al esperar chpasswd: {e}"))?;
                    if out.status.success() {
                        Ok(())
                    } else {
                        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        Err(if e.is_empty() {
                            "Operación cancelada o denegada.".to_string()
                        } else {
                            e
                        })
                    }
                })
                .await;
                btn_c.set_sensitive(true);
                match res {
                    Ok(Ok(())) => {
                        pwd_r.set_text("");
                        fb_c.set_label("Contraseña actualizada correctamente.");
                        fb_c.set_visible(true);
                    }
                    Ok(Err(msg)) => {
                        fb_c.set_label(&msg);
                        fb_c.set_visible(true);
                    }
                    Err(e) => {
                        fb_c.set_label(&format!("Error interno: {e:?}"));
                        fb_c.set_visible(true);
                    }
                }
            });
        });
    }

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

fn show_add_user_modal(parent: &ApplicationWindow, on_created: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(430)
        .css_classes(["macos-settings-window"])
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    root.append(
        &Label::builder()
            .label("Agregar usuario")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );

    let role_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    role_row.append(
        &Label::builder()
            .label("Nueva cuenta")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let role_model = StringList::new(&["Estándar", "Administración"]);
    let role_dd = DropDown::builder()
        .model(&role_model)
        .selected(0)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    role_row.append(&role_dd);
    root.append(&role_row);

    let full_name_entry = Entry::builder()
        .placeholder_text("Nombre completo (ej. Ada Lovelace)")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    let username_entry = Entry::builder()
        .placeholder_text("Nombre de la cuenta (ej. ada)")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    let pwd_entry = PasswordEntry::builder()
        .placeholder_text("Contraseña")
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    let confirm_entry = PasswordEntry::builder()
        .placeholder_text("Verificar contraseña")
        .show_peek_icon(true)
        .hexpand(true)
        .build();

    let manual_username = Rc::new(Cell::new(false));
    {
        let u_ent = username_entry.clone();
        let man = manual_username.clone();
        full_name_entry.connect_changed(move |ent| {
            if !man.get() {
                u_ent.set_text(&suggest_username(&ent.text()));
            }
        });
    }
    {
        let man = manual_username;
        let full_c = full_name_entry.clone();
        username_entry.connect_changed(move |ent| {
            if ent.has_focus() {
                man.set(!ent.text().is_empty() && ent.text() != suggest_username(&full_c.text()));
            }
        });
    }

    let fb_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    let spinner = Spinner::builder()
        .spinning(false)
        .visible(false)
        .valign(Align::Center)
        .build();

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(6)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (create_wrap, create_btn) = make_squircle_button("Crear usuario", true, None);
    actions.append(&spinner);
    actions.append(&cancel_wrap);
    actions.append(&create_wrap);

    root.append(&full_name_entry);
    root.append(&username_entry);
    root.append(&pwd_entry);
    root.append(&confirm_entry);
    root.append(&fb_lbl);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_cancel));

    {
        let fn_c = full_name_entry.clone();
        let un_c = username_entry;
        let p1_c = pwd_entry;
        let p2_c = confirm_entry;
        let dd_c = role_dd;
        let fb_c = fb_lbl;
        let sp_c = spinner;
        let dlg_c = dialog.clone();
        let cb = on_created;
        create_btn.connect_clicked(move |btn| {
            let full_name = fn_c.text().trim().to_string();
            let username = un_c.text().trim().to_string();
            let p1 = p1_c.text().to_string();
            let p2 = p2_c.text().to_string();
            let is_admin = dd_c.selected() == 1;

            if full_name.is_empty() {
                fb_c.set_label("Introduce un nombre completo.");
                fb_c.set_visible(true);
                return;
            }
            if !is_valid_linux_name(&username) {
                fb_c.set_label(
                    "El nombre de la cuenta debe empezar con letra minúscula o '_' y usar solo letras minúsculas, números, '_' o '-'.",
                );
                fb_c.set_visible(true);
                return;
            }
            if p1.len() < 4 || p1.contains('\n') || p1.contains('\r') {
                fb_c.set_label("La contraseña debe tener al menos 4 caracteres.");
                fb_c.set_visible(true);
                return;
            }
            if p1 != p2 {
                fb_c.set_label("Las contraseñas no coinciden.");
                fb_c.set_visible(true);
                return;
            }

            fb_c.set_visible(false);
            sp_c.set_visible(true);
            sp_c.set_spinning(true);
            btn.set_sensitive(false);

            let btn_r = btn.clone();
            let sp_r = sp_c.clone();
            let fb_r = fb_c.clone();
            let dlg_r = dlg_c.clone();
            let cb_r = cb.clone();

            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || -> Result<(), String> {
                    let mut args = vec!["useradd", "-m", "-c", &full_name];
                    if is_admin {
                        args.push("-G");
                        args.push("wheel");
                    }
                    args.push(&username);
                    let out = Command::new("pkexec")
                        .args(&args)
                        .output()
                        .map_err(|e| format!("No se pudo lanzar useradd: {e}"))?;
                    if !out.status.success() {
                        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        return Err(if e.is_empty() {
                            "Operación cancelada o denegada.".to_string()
                        } else {
                            e
                        });
                    }

                    let mut child = Command::new("pkexec")
                        .arg("chpasswd")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|e| format!("Usuario creado, pero falló chpasswd: {e}"))?;
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(format!("{username}:{p1}\n").as_bytes());
                    }
                    let ch_out = child
                        .wait_with_output()
                        .map_err(|e| format!("Error al establecer contraseña: {e}"))?;
                    if !ch_out.status.success() {
                        let e = String::from_utf8_lossy(&ch_out.stderr).trim().to_string();
                        return Err(format!("Usuario creado, pero no se fijó contraseña: {e}"));
                    }
                    Ok(())
                })
                .await;

                sp_r.set_spinning(false);
                sp_r.set_visible(false);
                btn_r.set_sensitive(true);

                match res {
                    Ok(Ok(())) => {
                        cb_r();
                        crate::close_attached_modal(&dlg_r);
                    }
                    Ok(Err(msg)) => {
                        fb_r.set_label(&msg);
                        fb_r.set_visible(true);
                    }
                    Err(e) => {
                        fb_r.set_label(&format!("Error interno: {e:?}"));
                        fb_r.set_visible(true);
                    }
                }
            });
        });
    }

    crate::present_attached_modal(&dialog, parent);
    full_name_entry.grab_focus();
}

fn show_add_group_modal(
    parent: &ApplicationWindow,
    users: Vec<SystemUser>,
    on_created: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(400)
        .css_classes(["macos-settings-window"])
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    root.append(
        &Label::builder()
            .label("Agregar grupo")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    let name_entry = Entry::builder()
        .placeholder_text("Nombre del grupo (ej. diseño -> diseno)")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    root.append(&name_entry);

    let mut member_checks: Vec<(String, CheckButton)> = Vec::new();
    if !users.is_empty() {
        root.append(
            &Label::builder()
                .label("Miembros iniciales")
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .build(),
        );
        let list_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        for u in &users {
            let chk = CheckButton::with_label(&format!("{} ({})", u.full_name, u.username));
            list_box.append(&chk);
            member_checks.push((u.username.clone(), chk));
        }
        root.append(&list_box);
    }

    let fb_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    root.append(&fb_lbl);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (create_wrap, create_btn) = make_squircle_button("Crear grupo", true, None);
    actions.append(&cancel_wrap);
    actions.append(&create_wrap);
    root.append(&actions);

    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_cancel));

    {
        let name_c = name_entry.clone();
        let fb_c = fb_lbl;
        let dlg_c = dialog.clone();
        let cb = on_created;
        create_btn.connect_clicked(move |btn| {
            let gname = name_c.text().trim().to_string();
            if !is_valid_linux_name(&gname) {
                fb_c.set_label("Usa solo letras minúsculas, números, '_' o '-' empezando por letra o '_'.");
                fb_c.set_visible(true);
                return;
            }
            let selected_members: Vec<String> = member_checks
                .iter()
                .filter(|(_, chk)| chk.is_active())
                .map(|(u, _)| u.clone())
                .collect();

            btn.set_sensitive(false);
            let btn_r = btn.clone();
            let fb_r = fb_c.clone();
            let dlg_r = dlg_c.clone();
            let cb_r = cb.clone();

            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || -> Result<(), String> {
                    let out = Command::new("pkexec")
                        .args(["groupadd", &gname])
                        .output()
                        .map_err(|e| format!("Error al lanzar groupadd: {e}"))?;
                    if !out.status.success() {
                        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        return Err(if e.is_empty() {
                            "Operación cancelada o denegada.".to_string()
                        } else {
                            e
                        });
                    }
                    for m in selected_members {
                        let _ = Command::new("pkexec")
                            .args(["usermod", "-aG", &gname, &m])
                            .output();
                    }
                    Ok(())
                })
                .await;
                btn_r.set_sensitive(true);
                match res {
                    Ok(Ok(())) => {
                        cb_r();
                        crate::close_attached_modal(&dlg_r);
                    }
                    Ok(Err(msg)) => {
                        fb_r.set_label(&msg);
                        fb_r.set_visible(true);
                    }
                    Err(e) => {
                        fb_r.set_label(&format!("Error interno: {e:?}"));
                        fb_r.set_visible(true);
                    }
                }
            });
        });
    }

    crate::present_attached_modal(&dialog, parent);
    name_entry.grab_focus();
}

fn show_group_detail_modal(
    parent: &ApplicationWindow,
    group: SystemGroup,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(390)
        .css_classes(["macos-settings-window"])
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    root.append(
        &Label::builder()
            .label(&format!("Grupo «{}»", group.name))
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    let members_txt = if group.members.is_empty() {
        "Sin miembros adicionales".to_string()
    } else {
        format!("Miembros: {}", group.members.join(", "))
    };
    root.append(
        &Label::builder()
            .label(&format!("GID {} · {}", group.gid, members_txt))
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let fb_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    root.append(&fb_lbl);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .build();
    let (del_wrap, del_btn) = make_squircle_button("Eliminar grupo", false, None);
    let spacer = GtkBox::builder().hexpand(true).build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&del_wrap);
    actions.append(&spacer);
    actions.append(&ok_wrap);
    root.append(&actions);
    dialog.set_child(Some(&root));

    {
        let gname = group.name;
        let fb = fb_lbl;
        let dlg = dialog.clone();
        let cb = on_changed;
        del_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let g = gname.clone();
            let fb_c = fb.clone();
            let dlg_c = dlg.clone();
            let cb_c = cb.clone();
            let btn_c = btn.clone();
            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || {
                    let out = Command::new("pkexec")
                        .args(["groupdel", &g])
                        .output()
                        .map_err(|e| format!("Error al lanzar groupdel: {e}"))?;
                    if out.status.success() {
                        Ok(())
                    } else {
                        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        Err(if e.is_empty() {
                            "Operación cancelada o denegada.".to_string()
                        } else {
                            e
                        })
                    }
                })
                .await;
                btn_c.set_sensitive(true);
                match res {
                    Ok(Ok(())) => {
                        cb_c();
                        crate::close_attached_modal(&dlg_c);
                    }
                    Ok(Err(msg)) => {
                        fb_c.set_label(&msg);
                        fb_c.set_visible(true);
                    }
                    Err(e) => {
                        fb_c.set_label(&format!("Error interno: {e:?}"));
                        fb_c.set_visible(true);
                    }
                }
            });
        });
    }

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

/// Construye el panel «Usuarios y grupos» estilo macOS Tahoe.
pub fn create_users_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let snapshot_state: Rc<RefCell<UsersSnapshot>> = Rc::new(RefCell::new(UsersSnapshot {
        users: Vec::new(),
        groups: Vec::new(),
        autologin_user: None,
        luks_active: false,
    }));
    let autologin_choices: Rc<RefCell<Vec<Option<String>>>> = Rc::new(RefCell::new(vec![None]));
    let syncing = Rc::new(Cell::new(false));

    // ========================================================
    // Tarjeta 1: Lista de usuarios, grupos personalizados y Usuario invitado
    // ========================================================
    let users_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    pane.append(&wrap_squircle_card(&users_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Botones "Agregar grupo..." y "Agregar usuario..."
    let add_bar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (add_grp_wrap, add_grp_btn) = make_squircle_button("Agregar grupo...", false, None);
    let (add_usr_wrap, add_usr_btn) = make_squircle_button("Agregar usuario...", false, None);
    add_bar.append(&add_grp_wrap);
    add_bar.append(&add_usr_wrap);
    pane.append(&add_bar);

    // ========================================================
    // Tarjeta 2: Inicio de sesión automático y Servidor de cuentas de red
    // ========================================================
    let login_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let auto_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let auto_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    auto_col.append(
        &Label::builder()
            .label("Iniciar sesión automáticamente como")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let auto_sub = Label::builder()
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    auto_col.append(&auto_sub);

    let auto_model = StringList::new(&["Desactivado"]);
    let auto_dd = DropDown::builder()
        .model(&auto_model)
        .selected(0)
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    auto_row.append(&auto_col);
    auto_row.append(&auto_dd);

    // Fila: Servidor de cuentas de red (por implementar)
    let net_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let net_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    net_left.append(
        &Label::builder()
            .label("Servidor de cuentas de red")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    net_left.append(&make_pending_badge());
    let (edit_net_wrap, edit_net_btn) = make_squircle_button("Editar...", false, None);
    edit_net_btn.set_sensitive(false);
    net_row.append(&net_left);
    net_row.append(&edit_net_wrap);

    login_card.append(&auto_row);
    login_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    login_card.append(&net_row);
    pane.append(&wrap_squircle_card(&login_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Pie inferior: botón de ayuda ?
    // ========================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Usuarios y grupos");
    let help_popover = Popover::builder().has_arrow(true).autohide(true).build();
    let help_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .width_request(300)
        .build();
    help_box.append(
        &Label::builder()
            .label("Usuarios y grupos")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "Lee y administra cuentas locales reales (/etc/passwd, /etc/group) con useradd, \
                 usermod, userdel, groupadd y chpasswd, e inicio de sesión automático vía GDM \
                 (/etc/gdm/custom.conf). «Usuario invitado» y «Servidor de cuentas de red» quedan \
                 marcados como por implementar.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    help_popover.set_child(Some(&help_box));
    help_btn.set_popover(Some(&help_popover));
    footer.append(&help_wrap);
    pane.append(&footer);

    // ========================================================
    // Refresco vivo desde /etc/passwd, /etc/group y /etc/gdm/custom.conf
    // ========================================================
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let refresh: Rc<dyn Fn()> = {
        let win_c = window.clone();
        let card_c = users_card.clone();
        let snap_state = snapshot_state.clone();
        let choices_c = autologin_choices.clone();
        let dd_c = auto_dd.clone();
        let sub_c = auto_sub.clone();
        let sync_c = syncing.clone();
        let self_slot = refresh_slot.clone();

        Rc::new(move || {
            let win_r = win_c.clone();
            let card_r = card_c.clone();
            let snap_r = snap_state.clone();
            let choices_r = choices_c.clone();
            let dd_r = dd_c.clone();
            let sub_r = sub_c.clone();
            let sync_r = sync_c.clone();
            let slot_r = self_slot.clone();

            glib::MainContext::default().spawn_local(async move {
                let Ok(snap) = gtk4::gio::spawn_blocking(probe_users_snapshot).await else {
                    return;
                };
                *snap_r.borrow_mut() = snap.clone();

                while let Some(child) = card_r.first_child() {
                    card_r.remove(&child);
                }

                let mut added_any = false;
                for user in &snap.users {
                    if added_any {
                        card_r.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                    }
                    added_any = true;

                    let row = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(12)
                        .css_classes(["wifi-pref-row"])
                        .build();
                    row.append(&make_user_avatar_widget(
                        &user.username,
                        &user.full_name,
                        &user.home,
                        34,
                    ));

                    let col = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(1)
                        .hexpand(true)
                        .valign(Align::Center)
                        .build();
                    col.append(
                        &Label::builder()
                            .label(&user.full_name)
                            .css_classes(["wifi-row-title"])
                            .xalign(0.0)
                            .build(),
                    );
                    col.append(
                        &Label::builder()
                            .label(if user.is_admin {
                                "Administración"
                            } else {
                                "Estándar"
                            })
                            .css_classes(["card-section-sub"])
                            .xalign(0.0)
                            .build(),
                    );
                    row.append(&col);

                    let info_btn = make_info_circle_button("Detalles del usuario");
                    let u_clone = user.clone();
                    let w_clone = win_r.clone();
                    let sl_clone = slot_r.clone();
                    info_btn.connect_clicked(move |_| {
                        let sl = sl_clone.clone();
                        show_user_detail_modal(
                            &w_clone,
                            u_clone.clone(),
                            Rc::new(move || {
                                if let Some(rf) = sl.borrow().as_ref() {
                                    rf();
                                }
                            }),
                        );
                    });
                    row.append(&info_btn);
                    card_r.append(&row);
                }

                // Grupos personalizados locales
                for grp in &snap.groups {
                    if added_any {
                        card_r.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                    }
                    added_any = true;

                    let row = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(12)
                        .css_classes(["wifi-pref-row"])
                        .build();
                    let grp_icon = Image::builder()
                        .icon_name("system-users-symbolic")
                        .pixel_size(24)
                        .width_request(34)
                        .height_request(34)
                        .valign(Align::Center)
                        .build();
                    row.append(&grp_icon);

                    let col = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(1)
                        .hexpand(true)
                        .valign(Align::Center)
                        .build();
                    col.append(
                        &Label::builder()
                            .label(&grp.name)
                            .css_classes(["wifi-row-title"])
                            .xalign(0.0)
                            .build(),
                    );
                    let m_sub = match grp.members.len() {
                        0 => "Grupo · Sin miembros".to_string(),
                        1 => "Grupo · 1 miembro".to_string(),
                        n => format!("Grupo · {n} miembros"),
                    };
                    col.append(
                        &Label::builder()
                            .label(&m_sub)
                            .css_classes(["card-section-sub"])
                            .xalign(0.0)
                            .build(),
                    );
                    row.append(&col);

                    let info_btn = make_info_circle_button("Detalles del grupo");
                    let g_clone = grp.clone();
                    let w_clone = win_r.clone();
                    let sl_clone = slot_r.clone();
                    info_btn.connect_clicked(move |_| {
                        let sl = sl_clone.clone();
                        show_group_detail_modal(
                            &w_clone,
                            g_clone.clone(),
                            Rc::new(move || {
                                if let Some(rf) = sl.borrow().as_ref() {
                                    rf();
                                }
                            }),
                        );
                    });
                    row.append(&info_btn);
                    card_r.append(&row);
                }

                // Fila final: Usuario invitado (por implementar)
                if added_any {
                    card_r.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let guest_row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .css_classes(["wifi-pref-row"])
                    .build();
                let guest_avatar = Image::builder()
                    .icon_name("avatar-default-symbolic")
                    .pixel_size(28)
                    .width_request(34)
                    .height_request(34)
                    .valign(Align::Center)
                    .build();
                guest_row.append(&guest_avatar);

                let guest_col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build();
                let guest_title_row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(8)
                    .build();
                guest_title_row.append(
                    &Label::builder()
                        .label("Usuario invitado")
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .build(),
                );
                guest_title_row.append(&make_pending_badge());
                guest_col.append(&guest_title_row);
                guest_col.append(
                    &Label::builder()
                        .label("Desactivado")
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .build(),
                );
                guest_row.append(&guest_col);
                let guest_info = make_info_circle_button("Sesión de invitado (por implementar)");
                guest_info.set_sensitive(false);
                guest_row.append(&guest_info);
                card_r.append(&guest_row);

                // Actualizar dropdown de inicio de sesión automático
                let mut labels: Vec<String> = vec!["Desactivado".to_string()];
                let mut map: Vec<Option<String>> = vec![None];
                let mut selected_idx = 0u32;
                for (i, u) in snap.users.iter().enumerate() {
                    labels.push(u.full_name.clone());
                    map.push(Some(u.username.clone()));
                    if snap.autologin_user.as_deref() == Some(u.username.as_str()) {
                        selected_idx = (i + 1) as u32;
                    }
                }
                *choices_r.borrow_mut() = map;
                let str_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                sync_r.set(true);
                dd_r.set_model(Some(&StringList::new(&str_refs)));
                dd_r.set_selected(selected_idx);
                sync_r.set(false);

                if snap.luks_active {
                    sub_r.set_label(
                        "El cifrado de disco FileVault (LUKS) está activado: el sistema seguirá \
                         pidiendo la clave de descifrado al arrancar.",
                    );
                    sub_r.set_visible(true);
                } else {
                    sub_r.set_visible(false);
                }
            });
        })
    };
    *refresh_slot.borrow_mut() = Some(refresh.clone());

    // ========================================================
    // Señales interactivas
    // ========================================================
    {
        let win_c = window.clone();
        let snap_c = snapshot_state.clone();
        let rf_c = refresh.clone();
        add_grp_btn.connect_clicked(move |_| {
            let users = snap_c.borrow().users.clone();
            show_add_group_modal(&win_c, users, rf_c.clone());
        });
    }
    {
        let win_c = window.clone();
        let rf_c = refresh.clone();
        add_usr_btn.connect_clicked(move |_| {
            show_add_user_modal(&win_c, rf_c.clone());
        });
    }
    {
        let choices_c = autologin_choices;
        let sync_c = syncing;
        let rf_c = refresh.clone();
        auto_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let target_user = choices_c
                .borrow()
                .get(dd.selected() as usize)
                .cloned()
                .flatten();
            let rf_r = rf_c.clone();
            glib::MainContext::default().spawn_local(async move {
                let _ = gtk4::gio::spawn_blocking(move || {
                    let current = std::fs::read_to_string(GDM_CUSTOM_CONF).unwrap_or_default();
                    let updated = update_gdm_autologin_content(&current, target_user.as_deref());
                    if let Ok(mut child) = Command::new("pkexec")
                        .args(["tee", GDM_CUSTOM_CONF])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                    {
                        if let Some(mut stdin) = child.stdin.take() {
                            let _ = stdin.write_all(updated.as_bytes());
                        }
                        let _ = child.wait();
                    }
                })
                .await;
                rf_r();
            });
        });
    }

    refresh();
    scrolled.set_child(Some(&pane));
    (scrolled, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_passwd_groups_and_gdm_autologin_cleanly() {
        let passwd = "\
root:x:0:0:root:/root:/bin/bash
cristian:x:1000:1000:Cristian Vera:/home/cristian:/bin/bash
ada:x:1001:1001:Ada:/home/ada:/bin/bash
nobody:x:65534:65534:Kernel Overflow User:/:/sbin/nologin
";
        let group = "\
wheel:x:10:cristian
cristian:x:1000:
ada:x:1001:
developers:x:1005:cristian,ada
";
        let (users, groups) = parse_passwd_and_group(passwd, group);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].username, "cristian");
        assert!(users[0].is_admin);
        assert_eq!(users[1].username, "ada");
        assert!(!users[1].is_admin);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "developers");
        assert_eq!(groups[0].members, vec!["cristian", "ada"]);

        let gdm = "# GDM\n[daemon]\n\n[security]\n";
        assert_eq!(parse_gdm_autologin(gdm), None);
        let updated = update_gdm_autologin_content(gdm, Some("cristian"));
        assert_eq!(parse_gdm_autologin(&updated).as_deref(), Some("cristian"));
        let disabled = update_gdm_autologin_content(&updated, None);
        assert_eq!(parse_gdm_autologin(&disabled), None);

        assert_eq!(suggest_username("Ada Lovelace"), "ada_lovelace");
        assert!(is_valid_linux_name("ada_lovelace"));
        assert!(!is_valid_linux_name("123bad"));
    }
}
