use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, Image, Label, Orientation,
    PolicyType, Popover, ScrolledWindow, StringList, Window,
};
use malus_de_common::config::{InternetAccountEntry, InternetAccountsConfig};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, make_squircle_colored_badge, wrap_squircle_card,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const ACCOUNTS_PANEL_ID: &str = "accounts";
pub const ACCOUNTS_PANEL_TITLE: &str = "Cuentas de Internet";

const BLUE_TOP: (f64, f64, f64) = (0.14, 0.54, 1.00);
const BLUE_BOT: (f64, f64, f64) = (0.04, 0.42, 0.90);
const CYAN_TOP: (f64, f64, f64) = (0.24, 0.72, 0.98);
const CYAN_BOT: (f64, f64, f64) = (0.10, 0.54, 0.88);
const ORANGE_TOP: (f64, f64, f64) = (1.00, 0.62, 0.08);
const ORANGE_BOT: (f64, f64, f64) = (0.90, 0.45, 0.02);
const INDIGO_TOP: (f64, f64, f64) = (0.37, 0.36, 0.90);
const INDIGO_BOT: (f64, f64, f64) = (0.29, 0.27, 0.78);
const GRAY_TOP: (f64, f64, f64) = (0.56, 0.57, 0.60);
const GRAY_BOT: (f64, f64, f64) = (0.43, 0.44, 0.48);

const PROVIDERS: &[(&str, &str, &str)] = &[
    ("google", "Google", "goa-account-google"),
    ("microsoft", "Microsoft Exchange / 365", "goa-account-ms365"),
    ("nextcloud", "Nextcloud / ownCloud", "goa-account-owncloud"),
    ("imap", "Correo (IMAP / SMTP)", "mail-unread-symbolic"),
    ("dav", "CalDAV / CardDAV", "x-office-calendar-symbolic"),
    ("fedora", "Cuenta de Fedora", "goa-account-fedora"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoaAccountInfo {
    provider: String,
    identity: String,
    services: Vec<&'static str>,
}

fn goa_accounts_conf_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/goa-1.0/accounts.conf")
}

fn parse_goa_accounts_conf(raw: &str) -> Vec<GoaAccountInfo> {
    let mut accounts = Vec::new();
    let mut in_account = false;
    let mut provider = String::new();
    let mut identity = String::new();
    let mut mail = false;
    let mut contacts = false;
    let mut calendar = false;
    let mut files = false;

    let flush = |accounts: &mut Vec<GoaAccountInfo>,
                 in_acc: bool,
                 prov: &mut String,
                 ident: &mut String,
                 mail: &mut bool,
                 contacts: &mut bool,
                 calendar: &mut bool,
                 files: &mut bool| {
        if in_acc && !prov.is_empty() {
            let mut services = Vec::new();
            if *mail {
                services.push("Correo");
            }
            if *contacts {
                services.push("Contactos");
            }
            if *calendar {
                services.push("Calendarios");
            }
            if *files {
                services.push("Archivos");
            }
            accounts.push(GoaAccountInfo {
                provider: std::mem::take(prov),
                identity: std::mem::take(ident),
                services,
            });
        } else {
            prov.clear();
            ident.clear();
        }
        *mail = false;
        *contacts = false;
        *calendar = false;
        *files = false;
    };

    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            flush(
                &mut accounts,
                in_account,
                &mut provider,
                &mut identity,
                &mut mail,
                &mut contacts,
                &mut calendar,
                &mut files,
            );
            in_account = line.starts_with("[Account ");
            continue;
        }
        if !in_account {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let val = v.trim();
            match k.trim() {
                "Provider" => provider = val.to_string(),
                "PresentationIdentity" | "Identity" => {
                    if identity.is_empty() || k.trim() == "PresentationIdentity" {
                        identity = val.to_string();
                    }
                }
                "MailEnabled" => mail = val.eq_ignore_ascii_case("true"),
                "ContactsEnabled" => contacts = val.eq_ignore_ascii_case("true"),
                "CalendarEnabled" => calendar = val.eq_ignore_ascii_case("true"),
                "FilesEnabled" => files = val.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
    }
    flush(
        &mut accounts,
        in_account,
        &mut provider,
        &mut identity,
        &mut mail,
        &mut contacts,
        &mut calendar,
        &mut files,
    );
    accounts
}

fn load_goa_accounts() -> Vec<GoaAccountInfo> {
    std::fs::read_to_string(goa_accounts_conf_path())
        .map(|raw| parse_goa_accounts_conf(&raw))
        .unwrap_or_default()
}

fn provider_display_name(provider: &str) -> &str {
    match provider {
        "google" => "Google",
        "microsoft" | "exchange" | "ms_graph" => "Microsoft 365 / Exchange",
        "nextcloud" | "owncloud" => "Nextcloud / ownCloud",
        "imap" | "imap_smtp" => "Correo (IMAP / SMTP)",
        "dav" | "webdav" => "CalDAV / CardDAV",
        "fedora" => "Fedora",
        "icloud" => "iCloud",
        other => other,
    }
}

fn make_provider_badge(provider: &str) -> gtk4::Widget {
    let icon_theme_has = |name: &str| {
        gtk4::gdk::Display::default()
            .map(|d| gtk4::IconTheme::for_display(&d).has_icon(name))
            .unwrap_or(false)
    };

    let goa_icon = match provider {
        "google" => Some("goa-account-google"),
        "microsoft" | "ms_graph" => Some("goa-account-ms365"),
        "exchange" => Some("goa-account-exchange"),
        "nextcloud" | "owncloud" => Some("goa-account-owncloud"),
        "fedora" => Some("goa-account-fedora"),
        _ => None,
    };

    if let Some(icon_name) = goa_icon {
        if icon_theme_has(icon_name) {
            return Image::builder()
                .icon_name(icon_name)
                .pixel_size(30)
                .width_request(34)
                .height_request(34)
                .valign(Align::Center)
                .build()
                .upcast();
        }
    }

    let (sym, top, bot) = match provider {
        "icloud" => ("weather-overcast-symbolic", CYAN_TOP, CYAN_BOT),
        "google" => ("goa-panel-symbolic", BLUE_TOP, BLUE_BOT),
        "microsoft" | "exchange" | "ms_graph" => ("mail-unread-symbolic", BLUE_TOP, BLUE_BOT),
        "nextcloud" | "owncloud" => ("folder-remote-symbolic", CYAN_TOP, CYAN_BOT),
        "imap" | "imap_smtp" => ("mail-unread-symbolic", INDIGO_TOP, INDIGO_BOT),
        "dav" | "webdav" => ("x-office-calendar-symbolic", ORANGE_TOP, ORANGE_BOT),
        "fedora" => ("system-users-symbolic", BLUE_TOP, BLUE_BOT),
        _ => ("goa-panel-symbolic", GRAY_TOP, GRAY_BOT),
    };
    make_squircle_colored_badge(32, sym, 17, 8.5, top, bot).upcast()
}

fn show_add_account_modal(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<InternetAccountsConfig>>,
    on_saved: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(440)
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
            .label("Agregar cuenta de Internet")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    root.append(
        &Label::builder()
            .label(
                "Registra una cuenta en Malus DE o abre el asistente de Cuentas en línea (GOA) \
                 para iniciar sesión mediante OAuth2.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let prov_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    prov_row.append(
        &Label::builder()
            .label("Proveedor")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let prov_labels: Vec<&str> = PROVIDERS.iter().map(|(_, l, _)| *l).collect();
    let prov_dd = DropDown::builder()
        .model(&StringList::new(&prov_labels))
        .selected(0)
        .css_classes(["wifi-compact-dropdown"])
        .build();
    prov_row.append(&prov_dd);
    root.append(&prov_row);

    let email_entry = Entry::builder()
        .placeholder_text("Correo electrónico o usuario (ej. nombre@dominio.com)")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    let server_entry = Entry::builder()
        .placeholder_text("Servidor (opcional, ej. https://cloud.midominio.com)")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();

    let fb_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();

    root.append(&email_entry);
    root.append(&server_entry);
    root.append(&fb_lbl);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .build();

    let has_goa_ui = std::path::Path::new("/usr/bin/gnome-control-center").is_file();
    if has_goa_ui {
        let (goa_wrap, goa_btn) = make_squircle_button("Asistente GOA...", false, None);
        let dlg_goa = dialog.clone();
        goa_btn.connect_clicked(move |_| {
            let _ = Command::new("gnome-control-center")
                .arg("online-accounts")
                .spawn();
            crate::close_attached_modal(&dlg_goa);
        });
        actions.append(&goa_wrap);
    }

    let spacer = GtkBox::builder().hexpand(true).build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (add_wrap, add_btn) = make_squircle_button("Agregar", true, None);
    actions.append(&spacer);
    actions.append(&cancel_wrap);
    actions.append(&add_wrap);
    root.append(&actions);

    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_cancel));

    let save_action = {
        let cfg_c = cfg.clone();
        let dd_c = prov_dd;
        let email_c = email_entry.clone();
        let srv_c = server_entry;
        let fb_c = fb_lbl;
        let dlg_c = dialog.clone();
        let cb = on_saved;
        Rc::new(move || {
            let user = email_c.text().trim().to_string();
            if user.is_empty() {
                fb_c.set_label("Introduce un correo electrónico o nombre de usuario.");
                fb_c.set_visible(true);
                return;
            }
            let (prov_id, prov_name, _) = PROVIDERS
                .get(dd_c.selected() as usize)
                .copied()
                .unwrap_or(("other", "Cuenta", "goa-panel-symbolic"));
            let is_cloud = matches!(prov_id, "google" | "nextcloud" | "microsoft");
            let entry = InternetAccountEntry {
                provider: prov_id.to_string(),
                display_name: prov_name.to_string(),
                email_or_user: user,
                server_url: srv_c.text().trim().to_string(),
                enabled: true,
                sync_mail: prov_id != "dav",
                sync_contacts: true,
                sync_calendars: true,
                sync_notes: prov_id == "google" || prov_id == "nextcloud",
                sync_files: is_cloud,
                ..InternetAccountEntry::default()
            };
            {
                let mut c = cfg_c.borrow_mut();
                c.add_account(entry);
                let _ = c.save();
            }
            cb();
            crate::close_attached_modal(&dlg_c);
        })
    };

    let sa_btn = save_action.clone();
    add_btn.connect_clicked(move |_| sa_btn());
    email_entry.connect_activate(move |_| save_action());

    crate::present_attached_modal(&dialog, parent);
    email_entry.grab_focus();
}

fn show_account_detail_modal(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<InternetAccountsConfig>>,
    account_id: String,
    on_changed: Rc<dyn Fn()>,
) {
    let Some(entry) = cfg
        .borrow()
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
    else {
        return;
    };

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
    hdr.append(&make_provider_badge(&entry.provider));
    let hdr_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    hdr_col.append(
        &Label::builder()
            .label(&entry.display_name)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr_col.append(
        &Label::builder()
            .label(&entry.email_or_user)
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    hdr.append(&hdr_col);
    root.append(&hdr);

    if !entry.server_url.is_empty() {
        root.append(
            &Label::builder()
                .label(&format!("Servidor: {}", entry.server_url))
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .build(),
        );
    }

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let make_svc_row = |title: &str, pending: bool, active: bool| -> (GtkBox, Switch) {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
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

    let (en_row, en_sw) = make_svc_row("Activar esta cuenta", false, entry.enabled);
    let (mail_row, mail_sw) = make_svc_row("Correo", true, entry.sync_mail);
    let (cont_row, cont_sw) = make_svc_row("Contactos", true, entry.sync_contacts);
    let (cal_row, cal_sw) = make_svc_row("Calendarios", true, entry.sync_calendars);
    let (notes_row, notes_sw) = make_svc_row("Notas", true, entry.sync_notes);
    let (files_row, files_sw) = make_svc_row("Archivos", true, entry.sync_files);

    let rows = [en_row, mail_row, cont_row, cal_row, notes_row, files_row];
    for (idx, r) in rows.into_iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        card.append(&r);
    }
    root.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    macro_rules! bind_field {
        ($sw:expr, $field:ident) => {{
            let cfg_c = cfg.clone();
            let id_c = account_id.clone();
            let cb_c = on_changed.clone();
            $sw.connect_state_set(move |_, active| {
                if let Some(acc) = cfg_c.borrow_mut().accounts.iter_mut().find(|a| a.id == id_c) {
                    acc.$field = active;
                }
                let _ = cfg_c.borrow().save();
                cb_c();
                glib::Propagation::Proceed
            });
        }};
    }
    bind_field!(en_sw, enabled);
    bind_field!(mail_sw, sync_mail);
    bind_field!(cont_sw, sync_contacts);
    bind_field!(cal_sw, sync_calendars);
    bind_field!(notes_sw, sync_notes);
    bind_field!(files_sw, sync_files);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .build();
    let (del_wrap, del_btn) = make_squircle_button("Eliminar cuenta", false, None);
    let spacer = GtkBox::builder().hexpand(true).build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&del_wrap);
    actions.append(&spacer);
    actions.append(&ok_wrap);
    root.append(&actions);

    dialog.set_child(Some(&root));

    {
        let cfg_c = cfg.clone();
        let id_c = account_id;
        let dlg_c = dialog.clone();
        let cb_c = on_changed;
        del_btn.connect_clicked(move |_| {
            {
                let mut c = cfg_c.borrow_mut();
                c.remove_account(&id_c);
                let _ = c.save();
            }
            cb_c();
            crate::close_attached_modal(&dlg_c);
        });
    }

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

/// Construye el panel «Cuentas de Internet» estilo macOS Tahoe.
pub fn create_accounts_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
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

    let cfg = Rc::new(RefCell::new(InternetAccountsConfig::load()));

    // Tarjeta principal de cuentas
    let accounts_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    pane.append(&wrap_squircle_card(
        &accounts_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Fila de acciones inferior: Agregar cuenta... y ?
    let actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();

    let (add_wrap, add_btn) = make_squircle_button("Agregar cuenta...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Cuentas de Internet");

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
            .label("Cuentas de Internet")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "Detecta automáticamente cuentas configuradas en GNOME Online Accounts \
                 (~/.config/goa-1.0/accounts.conf) y permite administrar cuentas locales de \
                 Malus DE en internet-accounts.toml. La sincronización nativa con iCloud y los \
                 demonios de correo/calendario aparecen marcados como «por implementar».",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    help_popover.set_child(Some(&help_box));
    help_btn.set_popover(Some(&help_popover));

    actions_row.append(&add_wrap);
    actions_row.append(&help_wrap);
    pane.append(&actions_row);

    // Refresco de la lista de cuentas
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh: Rc<dyn Fn()> = {
        let win_c = window.clone();
        let cfg_c = cfg.clone();
        let card_c = accounts_card.clone();
        let slot_c = refresh_slot.clone();

        Rc::new(move || {
            *cfg_c.borrow_mut() = InternetAccountsConfig::load();
            let goa_accounts = load_goa_accounts();

            while let Some(child) = card_c.first_child() {
                card_c.remove(&child);
            }

            // 1. Fila iCloud de referencia (por implementar, igual a la captura)
            let icloud_btn = Button::builder()
                .css_classes(["wifi-nav-row"])
                .has_frame(false)
                .hexpand(true)
                .sensitive(false)
                .build();
            let icloud_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .valign(Align::Center)
                .build();
            icloud_row.append(&make_provider_badge("icloud"));
            let ic_col = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .hexpand(true)
                .valign(Align::Center)
                .build();
            let ic_hdr = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .build();
            ic_hdr.append(
                &Label::builder()
                    .label("iCloud")
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .build(),
            );
            ic_hdr.append(&make_pending_badge());
            ic_col.append(&ic_hdr);
            ic_col.append(
                &Label::builder()
                    .label("iCloud Drive, Contactos, Calendarios, Recordatorios, Notas, Fotos y Llavero")
                    .css_classes(["card-section-sub"])
                    .wrap(true)
                    .xalign(0.0)
                    .build(),
            );
            icloud_row.append(&ic_col);
            icloud_row.append(
                &Image::builder()
                    .icon_name("go-next-symbolic")
                    .pixel_size(13)
                    .css_classes(["wifi-chevron"])
                    .valign(Align::Center)
                    .build(),
            );
            icloud_btn.set_child(Some(&icloud_row));
            card_c.append(&icloud_btn);

            // 2. Cuentas reales detectadas en GNOME Online Accounts (GOA)
            for acc in goa_accounts {
                card_c.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                let btn = Button::builder()
                    .css_classes(["wifi-nav-row"])
                    .has_frame(false)
                    .hexpand(true)
                    .build();
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .valign(Align::Center)
                    .build();
                row.append(&make_provider_badge(&acc.provider));
                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build();
                col.append(
                    &Label::builder()
                        .label(&format!(
                            "{} · {}",
                            provider_display_name(&acc.provider),
                            acc.identity
                        ))
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .build(),
                );
                let sub = if acc.services.is_empty() {
                    "Cuenta en línea (GOA)".to_string()
                } else {
                    acc.services.join(", ")
                };
                col.append(
                    &Label::builder()
                        .label(&sub)
                        .css_classes(["card-section-sub"])
                        .wrap(true)
                        .xalign(0.0)
                        .build(),
                );
                row.append(&col);
                row.append(
                    &Image::builder()
                        .icon_name("go-next-symbolic")
                        .pixel_size(13)
                        .css_classes(["wifi-chevron"])
                        .valign(Align::Center)
                        .build(),
                );
                btn.set_child(Some(&row));
                btn.connect_clicked(|_| {
                    let _ = Command::new("gnome-control-center")
                        .arg("online-accounts")
                        .spawn();
                });
                card_c.append(&btn);
            }

            // 3. Cuentas guardadas en Malus DE (internet-accounts.toml)
            let local_accounts = cfg_c.borrow().accounts.clone();
            for acc in local_accounts {
                card_c.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                let btn = Button::builder()
                    .css_classes(["wifi-nav-row"])
                    .has_frame(false)
                    .hexpand(true)
                    .build();
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .valign(Align::Center)
                    .build();
                row.append(&make_provider_badge(&acc.provider));
                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build();
                col.append(
                    &Label::builder()
                        .label(&format!("{} · {}", acc.display_name, acc.email_or_user))
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .build(),
                );
                col.append(
                    &Label::builder()
                        .label(&acc.enabled_services_summary())
                        .css_classes(["card-section-sub"])
                        .wrap(true)
                        .xalign(0.0)
                        .build(),
                );
                row.append(&col);
                row.append(
                    &Image::builder()
                        .icon_name("go-next-symbolic")
                        .pixel_size(13)
                        .css_classes(["wifi-chevron"])
                        .valign(Align::Center)
                        .build(),
                );
                btn.set_child(Some(&row));

                let win_d = win_c.clone();
                let cfg_d = cfg_c.clone();
                let id_d = acc.id.clone();
                let sl_d = slot_c.clone();
                btn.connect_clicked(move |_| {
                    let sl = sl_d.clone();
                    show_account_detail_modal(
                        &win_d,
                        &cfg_d,
                        id_d.clone(),
                        Rc::new(move || {
                            if let Some(rf) = sl.borrow().as_ref() {
                                rf();
                            }
                        }),
                    );
                });
                card_c.append(&btn);
            }
        })
    };
    *refresh_slot.borrow_mut() = Some(refresh.clone());

    {
        let win_c = window.clone();
        let cfg_c = cfg;
        let rf_c = refresh.clone();
        add_btn.connect_clicked(move |_| {
            show_add_account_modal(&win_c, &cfg_c, rf_c.clone());
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
    fn parses_gnome_online_accounts_conf_cleanly() {
        let sample = "\
[Account account_1700000000_0]
Provider=google
Identity=cristian@gmail.com
PresentationIdentity=cristian@gmail.com
MailEnabled=true
CalendarEnabled=true
ContactsEnabled=true
FilesEnabled=false

[Account account_1700000001_1]
Provider=owncloud
PresentationIdentity=cristian@cloud.local
FilesEnabled=true
";
        let parsed = parse_goa_accounts_conf(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].provider, "google");
        assert_eq!(parsed[0].identity, "cristian@gmail.com");
        assert_eq!(parsed[0].services, vec!["Correo", "Contactos", "Calendarios"]);
        assert_eq!(parsed[1].provider, "owncloud");
        assert_eq!(parsed[1].services, vec!["Archivos"]);
    }
}
