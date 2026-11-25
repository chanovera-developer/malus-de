use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, Image, Label, Orientation,
    PasswordEntry, PolicyType, Popover, ScrolledWindow, Spinner, StringList, Window,
};
use malus_de_common::config::{BiometricsConfig, LockScreenConfig};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const BIOMETRICS_PANEL_ID: &str = "biometrics";
pub const BIOMETRICS_PANEL_TITLE: &str = "Biometría y contraseña";

const ENROLL_FINGERS: &[(&str, &str)] = &[
    ("Índice derecho", "right-index-finger"),
    ("Índice izquierdo", "left-index-finger"),
    ("Pulgar derecho", "right-thumb"),
    ("Pulgar izquierdo", "left-thumb"),
    ("Medio derecho", "right-middle-finger"),
    ("Medio izquierdo", "left-middle-finger"),
    ("Anular derecho", "right-ring-finger"),
    ("Anular izquierdo", "left-ring-finger"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct FingerprintSnapshot {
    device_available: bool,
    status_note: Option<String>,
    enrolled_fingers: Vec<String>,
    pam_fingerprint_enabled: Option<bool>,
}

fn current_username() -> String {
    std::env::var("USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "usuario".to_string())
}

fn finger_label_es(raw: &str) -> String {
    match raw.trim() {
        "right-index-finger" => "Índice derecho".to_string(),
        "left-index-finger" => "Índice izquierdo".to_string(),
        "right-thumb" => "Pulgar derecho".to_string(),
        "left-thumb" => "Pulgar izquierdo".to_string(),
        "right-middle-finger" => "Medio derecho".to_string(),
        "left-middle-finger" => "Medio izquierdo".to_string(),
        "right-ring-finger" => "Anular derecho".to_string(),
        "left-ring-finger" => "Anular izquierdo".to_string(),
        "right-little-finger" => "Meñique derecho".to_string(),
        "left-little-finger" => "Meñique izquierdo".to_string(),
        other => other.to_string(),
    }
}

fn parse_fprintd_list_output(stdout: &str, stderr: &str) -> (bool, Option<String>, Vec<String>) {
    let combined = format!("{stdout}\n{stderr}").to_lowercase();
    if combined.contains("no devices available") || combined.contains("failed to get devices") {
        return (
            false,
            Some("No se detectó lector de huellas compatible en este equipo.".to_string()),
            Vec::new(),
        );
    }
    if combined.contains("failed to connect") || combined.contains("no existe el fichero") {
        return (
            false,
            Some("Servicio fprintd no disponible en esta sesión.".to_string()),
            Vec::new(),
        );
    }

    let mut fingers = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let f = rest.trim();
            if !f.is_empty() {
                fingers.push(f.to_string());
            }
        }
    }
    (true, None, fingers)
}

fn probe_pam_fingerprint_enabled() -> Option<bool> {
    let out = Command::new("authselect").arg("current").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.contains("with-fingerprint"))
}

fn probe_fingerprint_snapshot() -> FingerprintSnapshot {
    let user = current_username();
    let pam_enabled = probe_pam_fingerprint_enabled();
    match Command::new("fprintd-list").arg(&user).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let (device_available, status_note, enrolled_fingers) =
                parse_fprintd_list_output(&stdout, &stderr);
            FingerprintSnapshot {
                device_available,
                status_note,
                enrolled_fingers,
                pam_fingerprint_enabled: pam_enabled,
            }
        }
        Err(_) => FingerprintSnapshot {
            device_available: false,
            status_note: Some("fprintd no está instalado en el sistema.".to_string()),
            enrolled_fingers: Vec::new(),
            pam_fingerprint_enabled: pam_enabled,
        },
    }
}

fn show_change_password_modal(parent: &ApplicationWindow, on_updated: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(420)
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
            .label("Cambiar contraseña")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    root.append(
        &Label::builder()
            .label(format!(
                "Actualiza la contraseña de inicio de sesión de «{}». Se solicitará autorización del sistema.",
                current_username()
            ))
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let new_pwd = PasswordEntry::builder()
        .placeholder_text("Nueva contraseña")
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    let confirm_pwd = PasswordEntry::builder()
        .placeholder_text("Verificar nueva contraseña")
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    let lock_cfg = LockScreenConfig::load();
    let hint_entry = Entry::builder()
        .placeholder_text("Pista de contraseña (opcional)")
        .text(&lock_cfg.password_hint)
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();

    let feedback_lbl = Label::builder()
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
    let (save_wrap, save_btn) = make_squircle_button("Cambiar contraseña", true, None);
    actions.append(&spinner);
    actions.append(&cancel_wrap);
    actions.append(&save_wrap);

    root.append(&new_pwd);
    root.append(&confirm_pwd);
    root.append(&hint_entry);
    root.append(&feedback_lbl);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_cancel);
    });

    let submit_action = {
        let new_c = new_pwd.clone();
        let conf_c = confirm_pwd.clone();
        let hint_c = hint_entry.clone();
        let fb_c = feedback_lbl.clone();
        let spin_c = spinner.clone();
        let btn_c = save_btn.clone();
        let dlg_c = dialog.clone();
        let cb = on_updated;
        Rc::new(move || {
            let p1 = new_c.text().to_string();
            let p2 = conf_c.text().to_string();
            let hint_val = hint_c.text().trim().to_string();

            if p1.len() < 4 {
                fb_c.set_label("La nueva contraseña debe tener al menos 4 caracteres.");
                fb_c.set_visible(true);
                return;
            }
            if p1 != p2 {
                fb_c.set_label("Las contraseñas no coinciden.");
                fb_c.set_visible(true);
                return;
            }
            if p1.contains('\n') || p1.contains('\r') {
                fb_c.set_label("La contraseña no puede contener saltos de línea.");
                fb_c.set_visible(true);
                return;
            }

            fb_c.set_visible(false);
            spin_c.set_visible(true);
            spin_c.set_spinning(true);
            btn_c.set_sensitive(false);

            let user = current_username();
            let fb_res = fb_c.clone();
            let spin_res = spin_c.clone();
            let btn_res = btn_c.clone();
            let dlg_res = dlg_c.clone();
            let cb_res = cb.clone();

            glib::MainContext::default().spawn_local(async move {
                let result = gtk4::gio::spawn_blocking(move || -> Result<(), String> {
                    let mut child = Command::new("pkexec")
                        .arg("chpasswd")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|e| format!("No se pudo lanzar pkexec chpasswd: {e}"))?;
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin
                            .write_all(format!("{user}:{p1}\n").as_bytes())
                            .map_err(|e| format!("Error al enviar datos: {e}"))?;
                    }
                    let output = child
                        .wait_with_output()
                        .map_err(|e| format!("Error al esperar chpasswd: {e}"))?;
                    if output.status.success() {
                        Ok(())
                    } else {
                        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        Err(if err.is_empty() {
                            "Operación cancelada o denegada.".to_string()
                        } else {
                            err
                        })
                    }
                })
                .await;

                spin_res.set_spinning(false);
                spin_res.set_visible(false);
                btn_res.set_sensitive(true);

                match result {
                    Ok(Ok(())) => {
                        let mut lcfg = LockScreenConfig::load();
                        lcfg.password_hint = hint_val;
                        let _ = lcfg.save();
                        cb_res();
                        crate::close_attached_modal(&dlg_res);
                    }
                    Ok(Err(msg)) => {
                        fb_res.set_label(&msg);
                        fb_res.set_visible(true);
                    }
                    Err(e) => {
                        fb_res.set_label(&format!("Error interno: {e:?}"));
                        fb_res.set_visible(true);
                    }
                }
            });
        })
    };

    let s_btn = submit_action.clone();
    save_btn.connect_clicked(move |_| s_btn());
    confirm_pwd.connect_activate(move |_| submit_action());

    crate::present_attached_modal(&dialog, parent);
    new_pwd.grab_focus();
}

fn show_enroll_fingerprint_modal(parent: &ApplicationWindow, on_done: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(420)
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

    let icon = Image::builder()
        .icon_name("auth-fingerprint-symbolic")
        .pixel_size(40)
        .halign(Align::Center)
        .build();
    root.append(&icon);

    root.append(
        &Label::builder()
            .label("Agregar huella digital")
            .css_classes(["card-section-title"])
            .halign(Align::Center)
            .build(),
    );

    let status_lbl = Label::builder()
        .label("Selecciona el dedo y presiona «Iniciar captura» para registrar tu huella mediante fprintd.")
        .css_classes(["card-section-sub"])
        .justify(gtk4::Justification::Center)
        .wrap(true)
        .build();
    root.append(&status_lbl);

    let labels: Vec<&str> = ENROLL_FINGERS.iter().map(|(l, _)| *l).collect();
    let finger_model = StringList::new(&labels);
    let finger_dd = DropDown::builder()
        .model(&finger_model)
        .selected(0)
        .css_classes(["wifi-compact-dropdown"])
        .halign(Align::Center)
        .build();
    root.append(&finger_dd);

    let spinner = Spinner::builder()
        .spinning(false)
        .visible(false)
        .halign(Align::Center)
        .build();
    root.append(&spinner);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (close_wrap, close_btn) = make_squircle_button("Cerrar", false, None);
    let (start_wrap, start_btn) = make_squircle_button("Iniciar captura", true, None);
    actions.append(&close_wrap);
    actions.append(&start_wrap);
    root.append(&actions);

    dialog.set_child(Some(&root));

    let dlg_close = dialog.clone();
    close_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_close);
    });

    {
        let status_c = status_lbl.clone();
        let spin_c = spinner.clone();
        let start_c = start_btn.clone();
        let dd_c = finger_dd.clone();
        let dlg_c = dialog.clone();
        let cb = on_done;
        start_btn.connect_clicked(move |_| {
            let finger_id = ENROLL_FINGERS
                .get(dd_c.selected() as usize)
                .map(|(_, id)| *id)
                .unwrap_or("right-index-finger")
                .to_string();
            let user = current_username();
            status_c.set_label(
                "Coloca y levanta repetidamente el dedo sobre el sensor hasta completar la captura...",
            );
            spin_c.set_visible(true);
            spin_c.set_spinning(true);
            start_c.set_sensitive(false);
            dd_c.set_sensitive(false);

            let st_res = status_c.clone();
            let sp_res = spin_c.clone();
            let sb_res = start_c.clone();
            let dd_res = dd_c.clone();
            let dlg_res = dlg_c.clone();
            let cb_res = cb.clone();

            glib::MainContext::default().spawn_local(async move {
                let res = gtk4::gio::spawn_blocking(move || -> Result<(), String> {
                    let out = Command::new("fprintd-enroll")
                        .args(["-f", &finger_id, &user])
                        .output()
                        .map_err(|e| format!("No se pudo ejecutar fprintd-enroll: {e}"))?;
                    if out.status.success() {
                        Ok(())
                    } else {
                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        let msg = if err.is_empty() {
                            String::from_utf8_lossy(&out.stdout).trim().to_string()
                        } else {
                            err
                        };
                        Err(if msg.is_empty() {
                            "No se pudo completar el registro de huella.".to_string()
                        } else {
                            msg
                        })
                    }
                })
                .await;

                sp_res.set_spinning(false);
                sp_res.set_visible(false);
                sb_res.set_sensitive(true);
                dd_res.set_sensitive(true);

                match res {
                    Ok(Ok(())) => {
                        cb_res();
                        crate::close_attached_modal(&dlg_res);
                    }
                    Ok(Err(err)) => {
                        st_res.set_label(&err);
                    }
                    Err(e) => {
                        st_res.set_label(&format!("Error interno: {e:?}"));
                    }
                }
            });
        });
    }

    crate::present_attached_modal(&dialog, parent);
}

/// Construye el panel «Biometría y contraseña» estilo macOS Tahoe.
pub fn create_biometrics_pane(
    window: &ApplicationWindow,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
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

    let cfg = Rc::new(RefCell::new(BiometricsConfig::load()));
    let syncing = Rc::new(Cell::new(false));

    // ========================================================
    // Sección 1: Contraseña
    // ========================================================
    pane.append(
        &Label::builder()
            .label("Contraseña")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .build(),
    );

    let pwd_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let pwd_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let pwd_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    pwd_col.append(
        &Label::builder()
            .label("Se definió una contraseña de inicio de sesión para este usuario.")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let hint_sub = Label::builder()
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .visible(false)
        .build();
    pwd_col.append(&hint_sub);
    let (change_wrap, change_btn) = make_squircle_button("Cambiar...", false, None);
    pwd_row.append(&pwd_col);
    pwd_row.append(&change_wrap);
    pwd_card.append(&pwd_row);
    pane.append(&wrap_squircle_card(&pwd_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Sección 2: Biometría (huellas registradas + Agregar huella)
    // ========================================================
    let bio_hdr = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .margin_top(6)
        .build();
    bio_hdr.append(
        &Label::builder()
            .label("Biometría")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .build(),
    );
    bio_hdr.append(
        &Label::builder()
            .label(
                "La biometría te permite usar tu huella digital (fprintd) para desbloquear tu \
                 equipo y autorizar acciones del sistema.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pane.append(&bio_hdr);

    // Tira horizontal centrada de huellas + botón "+" (igual a la captura)
    let fingers_strip = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(28)
        .halign(Align::Center)
        .margin_top(8)
        .margin_bottom(6)
        .build();
    pane.append(&fingers_strip);

    let device_status_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .halign(Align::Center)
        .justify(gtk4::Justification::Center)
        .wrap(true)
        .visible(false)
        .build();
    pane.append(&device_status_lbl);

    // ========================================================
    // Tarjeta de interruptores de uso biométrico (5 filas)
    // ========================================================
    let toggles_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let make_bio_row = |title: &str, pending: bool, active: bool| -> (GtkBox, Switch) {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        let left = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        left.append(
            &Label::builder()
                .label(title)
                .css_classes(["wifi-row-title"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        if pending {
            left.append(&make_pending_badge());
        }
        let sw = Switch::builder()
            .valign(Align::Center)
            .active(active)
            .build();
        row.append(&left);
        row.append(&*sw);
        (row, sw)
    };

    let (unlock_row, unlock_sw) = make_bio_row(
        "Usar biometría para desbloquear el equipo",
        false,
        cfg.borrow().unlock_with_biometrics,
    );
    let (pay_row, pay_sw) = make_bio_row(
        "Usar biometría para pagos",
        true,
        cfg.borrow().biometrics_for_payments,
    );
    let (store_row, store_sw) = make_bio_row(
        "Usar biometría para compras en tiendas de aplicaciones",
        true,
        cfg.borrow().biometrics_for_store_purchases,
    );
    let (autofill_row, autofill_sw) = make_bio_row(
        "Usar biometría para autorrellenar contraseñas",
        true,
        cfg.borrow().biometrics_for_autofill,
    );
    let (switch_user_row, switch_user_sw) = make_bio_row(
        "Usar biometría para el cambio rápido de usuario",
        true,
        cfg.borrow().biometrics_for_fast_user_switch,
    );

    let rows = [unlock_row, pay_row, store_row, autofill_row, switch_user_row];
    for (idx, r) in rows.into_iter().enumerate() {
        if idx > 0 {
            toggles_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        toggles_card.append(&r);
    }
    pane.append(&wrap_squircle_card(&toggles_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Pie inferior: botón de ayuda ?
    // ========================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Biometría y contraseña");
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
            .label("Biometría y contraseña")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "El cambio de contraseña usa chpasswd con autorización del sistema; el registro \
                 y gestión de huellas usa fprintd y la opción de desbloqueo sincroniza el perfil \
                 PAM with-fingerprint mediante authselect.",
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
    // Refresco asíncrono de huellas + estado PAM + pista
    // ========================================================
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let refresh: Rc<dyn Fn()> = {
        let win_c = window.clone();
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        let strip_c = fingers_strip.clone();
        let dev_lbl_c = device_status_lbl.clone();
        let hint_lbl_c = hint_sub.clone();
        let unlock_c = unlock_sw.clone();
        let pay_c = pay_sw.clone();
        let store_c = store_sw.clone();
        let auto_c = autofill_sw.clone();
        let fsu_c = switch_user_sw.clone();
        let self_slot = refresh_slot.clone();

        Rc::new(move || {
            let loaded = BiometricsConfig::load();
            *cfg_c.borrow_mut() = loaded.clone();

            let lcfg = LockScreenConfig::load();
            if !lcfg.password_hint.trim().is_empty() {
                hint_lbl_c.set_label(&format!("Pista configurada: {}", lcfg.password_hint.trim()));
                hint_lbl_c.set_visible(true);
            } else {
                hint_lbl_c.set_visible(false);
            }

            sync_c.set(true);
            unlock_c.set_active(loaded.unlock_with_biometrics);
            pay_c.set_active(loaded.biometrics_for_payments);
            store_c.set_active(loaded.biometrics_for_store_purchases);
            auto_c.set_active(loaded.biometrics_for_autofill);
            fsu_c.set_active(loaded.biometrics_for_fast_user_switch);
            sync_c.set(false);

            let win_r = win_c.clone();
            let strip_r = strip_c.clone();
            let dev_r = dev_lbl_c.clone();
            let unlock_r = unlock_c.clone();
            let sync_r = sync_c.clone();
            let slot_r = self_slot.clone();

            glib::MainContext::default().spawn_local(async move {
                let Ok(snap) = gtk4::gio::spawn_blocking(probe_fingerprint_snapshot).await else {
                    return;
                };

                if let Some(pam_on) = snap.pam_fingerprint_enabled {
                    sync_r.set(true);
                    unlock_r.set_active(pam_on);
                    sync_r.set(false);
                }

                while let Some(child) = strip_r.first_child() {
                    strip_r.remove(&child);
                }

                // Huellas ya registradas
                for finger in &snap.enrolled_fingers {
                    let col = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .halign(Align::Center)
                        .spacing(6)
                        .build();

                    let fp_btn = Button::builder()
                        .icon_name("auth-fingerprint-symbolic")
                        .width_request(44)
                        .height_request(44)
                        .halign(Align::Center)
                        .css_classes(["circular"])
                        .tooltip_text("Clic para eliminar esta huella")
                        .build();

                    let pop = Popover::builder().has_arrow(true).autohide(true).build();
                    let pbox = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(6)
                        .margin_top(8)
                        .margin_bottom(8)
                        .margin_start(10)
                        .margin_end(10)
                        .build();
                    pbox.append(
                        &Label::builder()
                            .label(&finger_label_es(finger))
                            .css_classes(["card-section-title"])
                            .build(),
                    );
                    let (del_wrap, del_btn) = make_squircle_button("Eliminar huella", false, None);
                    pbox.append(&del_wrap);
                    pop.set_child(Some(&pbox));
                    pop.set_parent(&fp_btn);

                    let pop_open = pop.clone();
                    fp_btn.connect_clicked(move |_| pop_open.popup());
                    let pop_unroot = pop.clone();
                    fp_btn.connect_unrealize(move |_| pop_unroot.unparent());

                    let finger_del = finger.clone();
                    let pop_del = pop;
                    let slot_del = slot_r.clone();
                    del_btn.connect_clicked(move |_| {
                        pop_del.popdown();
                        let f_id = finger_del.clone();
                        let user = current_username();
                        let sl = slot_del.clone();
                        glib::MainContext::default().spawn_local(async move {
                            let _ = gtk4::gio::spawn_blocking(move || {
                                let _ = Command::new("fprintd-delete")
                                    .args([&user, "-f", &f_id])
                                    .output();
                            })
                            .await;
                            if let Some(rf) = sl.borrow().as_ref() {
                                rf();
                            }
                        });
                    });

                    col.append(&fp_btn);
                    col.append(
                        &Label::builder()
                            .label(&finger_label_es(finger))
                            .css_classes(["card-section-sub"])
                            .halign(Align::Center)
                            .build(),
                    );
                    strip_r.append(&col);
                }

                // Botón "+ Agregar huella"
                let add_col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .halign(Align::Center)
                    .spacing(6)
                    .build();
                let add_btn = Button::builder()
                    .icon_name("list-add-symbolic")
                    .width_request(44)
                    .height_request(44)
                    .halign(Align::Center)
                    .css_classes(["circular"])
                    .sensitive(snap.device_available)
                    .tooltip_text(if snap.device_available {
                        "Registrar una nueva huella digital"
                    } else {
                        "No se detectó lector de huellas"
                    })
                    .build();

                let win_enroll = win_r.clone();
                let slot_enroll = slot_r.clone();
                add_btn.connect_clicked(move |_| {
                    let sl = slot_enroll.clone();
                    show_enroll_fingerprint_modal(
                        &win_enroll,
                        Rc::new(move || {
                            if let Some(rf) = sl.borrow().as_ref() {
                                rf();
                            }
                        }),
                    );
                });

                add_col.append(&add_btn);
                add_col.append(
                    &Label::builder()
                        .label("Agregar huella")
                        .css_classes(["card-section-sub"])
                        .halign(Align::Center)
                        .build(),
                );
                strip_r.append(&add_col);

                if let Some(note) = snap.status_note {
                    dev_r.set_label(&note);
                    dev_r.set_visible(true);
                } else {
                    dev_r.set_visible(false);
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
        let rf_c = refresh.clone();
        change_btn.connect_clicked(move |_| {
            show_change_password_modal(&win_c, rf_c.clone());
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        unlock_sw.connect_state_set(move |sw, active| {
            if sync_c.get() {
                return glib::Propagation::Proceed;
            }
            {
                let mut c = cfg_c.borrow_mut();
                c.unlock_with_biometrics = active;
                let _ = c.save();
            }
            let sw_c = sw.clone();
            let sync_r = sync_c.clone();
            glib::MainContext::default().spawn_local(async move {
                let subcmd = if active {
                    "enable-feature"
                } else {
                    "disable-feature"
                };
                let ok = gtk4::gio::spawn_blocking(move || {
                    Command::new("pkexec")
                        .args(["authselect", subcmd, "with-fingerprint"])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                })
                .await
                .unwrap_or(false);
                if !ok {
                    if let Some(actual) =
                        gtk4::gio::spawn_blocking(probe_pam_fingerprint_enabled)
                            .await
                            .ok()
                            .flatten()
                    {
                        sync_r.set(true);
                        sw_c.set_active(actual);
                        sync_r.set(false);
                    }
                }
            });
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        pay_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.biometrics_for_payments = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        store_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.biometrics_for_store_purchases = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        autofill_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.biometrics_for_autofill = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing;
        switch_user_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.biometrics_for_fast_user_switch = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
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
    fn fprintd_parser_handles_enrolled_fingers_and_missing_device() {
        let sample = "\
found 1 devices
Device at /net/reactivated/Fprint/Device/0
Using device /net/reactivated/Fprint/Device/0
Fingerprints for user cristian on Goodix Fingerprint Device (press):
 - right-index-finger
 - right-thumb
";
        let (avail, note, fingers) = parse_fprintd_list_output(sample, "");
        assert!(avail);
        assert!(note.is_none());
        assert_eq!(fingers, vec!["right-index-finger", "right-thumb"]);
        assert_eq!(finger_label_es(&fingers[0]), "Índice derecho");
        assert_eq!(finger_label_es(&fingers[1]), "Pulgar derecho");

        let (avail2, note2, fingers2) =
            parse_fprintd_list_output("", "list_devices failed: No devices available");
        assert!(!avail2);
        assert!(note2.is_some());
        assert!(fingers2.is_empty());
    }
}
