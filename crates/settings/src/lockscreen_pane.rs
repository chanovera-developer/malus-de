use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, CheckButton, DropDown, Entry, Label, Orientation,
    PolicyType, ScrolledWindow, StringList, Window,
};
use malus_de_common::config::{LockScreenConfig, LoginWindowMode};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const LOCKSCREEN_PANEL_ID: &str = "lockscreen";
pub const LOCKSCREEN_PANEL_TITLE: &str = "Pantalla bloqueada";

const DISPLAY_OFF_OPTIONS: &[(&str, u32)] = &[
    ("Durante 1 minuto", 60),
    ("Durante 2 minutos", 120),
    ("Durante 3 minutos", 180),
    ("Durante 5 minutos", 300),
    ("Durante 10 minutos", 600),
    ("Durante 20 minutos", 1200),
    ("Durante 30 minutos", 1800),
    ("Durante 1 hora", 3600),
    ("Durante 2 horas", 7200),
    ("Durante 3 horas", 10800),
    ("Nunca", 0),
];

const REQUIRE_PASSWORD_OPTIONS: &[(&str, i32)] = &[
    ("Inmediatamente", 0),
    ("Después de 5 segundos", 5),
    ("Después de 1 minuto", 60),
    ("Después de 5 minutos", 300),
    ("Después de 15 minutos", 900),
    ("Después de 1 hora", 3600),
    ("Después de 4 horas", 14400),
    ("Después de 8 horas", 28800),
    ("Nunca", -1),
];

fn display_off_index_for_secs(secs: u32) -> u32 {
    DISPLAY_OFF_OPTIONS
        .iter()
        .position(|(_, s)| *s == secs)
        .unwrap_or(1) as u32
}

fn require_password_index_for_secs(secs: i32) -> u32 {
    REQUIRE_PASSWORD_OPTIONS
        .iter()
        .position(|(_, s)| *s == secs)
        .unwrap_or(3) as u32
}

fn read_gsetting_bool(schema: &str, key: &str, fallback: bool) -> bool {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return fallback;
    };
    if source.lookup(schema, true).is_none() {
        return fallback;
    }
    gtk4::gio::Settings::new(schema).boolean(key)
}

fn write_gsetting_bool(schema: &str, key: &str, value: bool) {
    let Some(source) = gtk4::gio::SettingsSchemaSource::default() else {
        return;
    };
    if source.lookup(schema, true).is_none() {
        return;
    }
    let _ = gtk4::gio::Settings::new(schema).set_boolean(key, value);
}

fn make_dropdown_row(
    title: &str,
    options: &[&str],
    selected: u32,
    pending: bool,
) -> (GtkBox, DropDown) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();

    let left = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .valign(Align::Center)
        .build();

    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["wifi-row-title"])
        .wrap(true)
        .xalign(0.0)
        .build();
    left.append(&title_lbl);

    if pending {
        let badge_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .halign(Align::Start)
            .build();
        badge_row.append(&make_pending_badge());
        left.append(&badge_row);
    }

    let model = StringList::new(options);
    let dd = DropDown::builder()
        .model(&model)
        .selected(selected.min(options.len().saturating_sub(1) as u32))
        .valign(Align::Center)
        .css_classes(["wifi-compact-dropdown"])
        .build();

    row.append(&left);
    row.append(&dd);
    (row, dd)
}

fn show_lock_message_modal(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<LockScreenConfig>>,
    on_saved: Rc<dyn Fn()>,
) {
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

    let title = Label::builder()
        .label("Mensaje de la pantalla bloqueada")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let desc = Label::builder()
        .label(
            "Este mensaje aparecerá en la parte inferior de la pantalla bloqueada de Malus DE \
             (por ejemplo, información de contacto en caso de extravío).",
        )
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    let entry = Entry::builder()
        .placeholder_text("Ej. Si encuentras este equipo, llama al...")
        .text(&cfg.borrow().lock_message)
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(6)
        .build();

    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&cancel_wrap);
    actions.append(&ok_wrap);

    root.append(&title);
    root.append(&desc);
    root.append(&entry);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_cancel);
    });

    let save_action = {
        let cfg_c = cfg.clone();
        let entry_c = entry.clone();
        let dlg_ok = dialog.clone();
        let cb = on_saved.clone();
        Rc::new(move || {
            {
                let mut c = cfg_c.borrow_mut();
                c.lock_message = entry_c.text().trim().to_string();
                let _ = c.save();
            }
            cb();
            crate::close_attached_modal(&dlg_ok);
        })
    };

    let sa_btn = save_action.clone();
    ok_btn.connect_clicked(move |_| {
        sa_btn();
    });
    let sa_ent = save_action;
    entry.connect_activate(move |_| {
        sa_ent();
    });

    crate::present_attached_modal(&dialog, parent);
    entry.grab_focus();
}

fn show_accessibility_options_modal(
    parent: &ApplicationWindow,
    navigate: Rc<dyn Fn(&str)>,
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

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    let title = Label::builder()
        .label("Opciones de Accesibilidad")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();
    let desc = Label::builder()
        .label("Ajustes rápidos de accesibilidad activos en la sesión y en la pantalla bloqueada.")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();

    let card_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let make_opt_row = |label: &str, active: bool| -> (GtkBox, Switch) {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        let lbl = Label::builder()
            .label(label)
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build();
        let sw = Switch::builder()
            .valign(Align::Center)
            .active(active)
            .build();
        row.append(&lbl);
        row.append(&*sw);
        (row, sw)
    };

    let (sr_row, sr_sw) = make_opt_row(
        "Lector de pantalla (Orca)",
        read_gsetting_bool(
            "org.gnome.desktop.a11y.applications",
            "screen-reader-enabled",
            false,
        ),
    );
    sr_sw.connect_state_set(|_, active| {
        write_gsetting_bool(
            "org.gnome.desktop.a11y.applications",
            "screen-reader-enabled",
            active,
        );
        glib::Propagation::Proceed
    });

    let (hc_row, hc_sw) = make_opt_row(
        "Aumentar contraste",
        read_gsetting_bool("org.gnome.desktop.a11y.interface", "high-contrast", false),
    );
    hc_sw.connect_state_set(|_, active| {
        write_gsetting_bool("org.gnome.desktop.a11y.interface", "high-contrast", active);
        glib::Propagation::Proceed
    });

    let (rm_row, rm_sw) = make_opt_row(
        "Reducir movimiento",
        !read_gsetting_bool("org.gnome.desktop.interface", "enable-animations", true),
    );
    rm_sw.connect_state_set(|_, active| {
        write_gsetting_bool("org.gnome.desktop.interface", "enable-animations", !active);
        glib::Propagation::Proceed
    });

    card_inner.append(&sr_row);
    card_inner.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card_inner.append(&hc_row);
    card_inner.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card_inner.append(&rm_row);
    let card_wrap = wrap_squircle_card(&card_inner, WIFI_CARD_SQUIRCLE_RADIUS);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .build();

    let (open_wrap, open_btn) = make_squircle_button("Abrir Accesibilidad...", false, None);
    let spacer = GtkBox::builder().hexpand(true).build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&open_wrap);
    actions.append(&spacer);
    actions.append(&ok_wrap);

    root.append(&title);
    root.append(&desc);
    root.append(&card_wrap);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dlg_open = dialog.clone();
    open_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_open);
        navigate("accessibility");
    });

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_ok);
    });

    crate::present_attached_modal(&dialog, parent);
}

/// Construye el panel «Pantalla bloqueada» estilo macOS Tahoe.
pub fn create_lockscreen_pane(
    window: &ApplicationWindow,
    navigate: Rc<dyn Fn(&str)>,
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

    let cfg = Rc::new(RefCell::new(LockScreenConfig::load()));
    let syncing = Rc::new(Cell::new(false));

    // ========================================================
    // Tarjeta 1: Temporizadores de apagado de pantalla y contraseña
    // ========================================================
    let display_labels: Vec<&str> = DISPLAY_OFF_OPTIONS.iter().map(|(l, _)| *l).collect();
    let pwd_labels: Vec<&str> = REQUIRE_PASSWORD_OPTIONS.iter().map(|(l, _)| *l).collect();

    let card1_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let (bat_row, bat_dd) = make_dropdown_row(
        "Apagar pantalla cuando se use la batería y no haya actividad",
        &display_labels,
        display_off_index_for_secs(cfg.borrow().battery_display_off_secs),
        true,
    );
    let (ac_row, ac_dd) = make_dropdown_row(
        "Apagar pantalla cuando se use adaptador de corriente y no haya actividad",
        &display_labels,
        display_off_index_for_secs(cfg.borrow().ac_display_off_secs),
        true,
    );
    let (req_pwd_row, req_pwd_dd) = make_dropdown_row(
        "Solicitar contraseña cuando se apague la pantalla o se muestre el protector",
        &pwd_labels,
        require_password_index_for_secs(cfg.borrow().require_password_after_secs),
        true,
    );

    card1_inner.append(&bat_row);
    card1_inner.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card1_inner.append(&ac_row);
    card1_inner.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card1_inner.append(&req_pwd_row);
    pane.append(&wrap_squircle_card(&card1_inner, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 2: Pistas de contraseña y mensaje en pantalla bloqueada
    // ========================================================
    let card2_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    // Fila 2.1: Mostrar pistas de contraseña
    let hints_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let hints_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    hints_left.append(
        &Label::builder()
            .label("Mostrar pistas de contraseña")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    hints_left.append(&make_pending_badge());
    let hints_sw = Switch::builder()
        .valign(Align::Center)
        .active(cfg.borrow().show_password_hints)
        .build();
    hints_row.append(&hints_left);
    hints_row.append(&*hints_sw);

    // Fila 2.2: Mostrar mensaje en pantalla bloqueada + Establecer... (100% real en session-lock)
    let msg_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let msg_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let msg_title = Label::builder()
        .label("Mostrar mensaje en pantalla bloqueada")
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .build();
    let msg_preview = Label::builder()
        .css_classes(["card-section-sub"])
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .xalign(0.0)
        .visible(false)
        .build();
    msg_col.append(&msg_title);
    msg_col.append(&msg_preview);

    let msg_sw = Switch::builder()
        .valign(Align::Center)
        .active(cfg.borrow().show_lock_message)
        .build();
    let (set_msg_wrap, set_msg_btn) = make_squircle_button("Establecer...", false, None);
    set_msg_btn.set_sensitive(cfg.borrow().show_lock_message);

    msg_row.append(&msg_col);
    msg_row.append(&*msg_sw);
    msg_row.append(&set_msg_wrap);

    card2_inner.append(&hints_row);
    card2_inner.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card2_inner.append(&msg_row);
    pane.append(&wrap_squircle_card(&card2_inner, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Tarjeta 3: Actividades en vivo
    // ========================================================
    let card3_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let live_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let live_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    live_left.append(
        &Label::builder()
            .label("Mostrar actividades en vivo con dispositivo bloqueado")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    live_left.append(&make_pending_badge());
    let live_sw = Switch::builder()
        .valign(Align::Center)
        .active(cfg.borrow().show_live_activities)
        .build();
    live_row.append(&live_left);
    live_row.append(&*live_sw);
    card3_inner.append(&live_row);
    pane.append(&wrap_squircle_card(&card3_inner, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Sección: Al cambiar de usuario
    // ========================================================
    let switch_heading = Label::builder()
        .label("Al cambiar de usuario")
        .css_classes(["wifi-section-heading"])
        .xalign(0.0)
        .margin_top(4)
        .build();
    pane.append(&switch_heading);

    let card4_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    // Fila 4.1: La ventana de inicio de sesión muestra (radio buttons)
    let login_mode_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .css_classes(["wifi-pref-row"])
        .build();
    let login_mode_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    login_mode_header.append(
        &Label::builder()
            .label("La ventana de inicio de sesión muestra")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    login_mode_header.append(&make_pending_badge());
    login_mode_box.append(&login_mode_header);

    let rb_user_list = CheckButton::with_label("Lista de usuarios");
    let rb_name_pwd = CheckButton::with_label("Nombre y contraseña");
    rb_name_pwd.set_group(Some(&rb_user_list));
    rb_user_list.add_css_class("macos-radio");
    rb_name_pwd.add_css_class("macos-radio");
    match cfg.borrow().login_window_mode {
        LoginWindowMode::UserList => rb_user_list.set_active(true),
        LoginWindowMode::NameAndPassword => rb_name_pwd.set_active(true),
    }
    login_mode_box.append(&rb_user_list);
    login_mode_box.append(&rb_name_pwd);

    // Fila 4.2: Mostrar los botones Reposo, Reiniciar y Apagar (100% real en session-lock)
    let power_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let power_lbl = Label::builder()
        .label("Mostrar los botones Reposo, Reiniciar y Apagar")
        .css_classes(["wifi-row-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let power_sw = Switch::builder()
        .valign(Align::Center)
        .active(cfg.borrow().show_power_buttons)
        .build();
    power_row.append(&power_lbl);
    power_row.append(&*power_sw);

    card4_inner.append(&login_mode_box);
    card4_inner.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card4_inner.append(&power_row);
    pane.append(&wrap_squircle_card(&card4_inner, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================
    // Pie inferior: Opciones de Accesibilidad... y ?
    // ========================================================
    let footer_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (a11y_wrap, a11y_btn) =
        make_squircle_button("Opciones de Accesibilidad...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Pantalla bloqueada");

    let help_popover = gtk4::Popover::builder()
        .has_arrow(true)
        .autohide(true)
        .build();
    let help_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .width_request(290)
        .build();
    help_box.append(
        &Label::builder()
            .label("Pantalla bloqueada")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "El mensaje en pantalla bloqueada, los botones Reposo/Reiniciar/Apagar y las \
                 opciones rápidas de Accesibilidad se aplican en vivo en la pantalla de bloqueo \
                 de Malus DE. Las preferencias marcadas como «por implementar» se guardan en \
                 lockscreen.toml para futuras integraciones del compositor y gestor de sesiones.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    help_popover.set_child(Some(&help_box));
    help_btn.set_popover(Some(&help_popover));

    footer_row.append(&a11y_wrap);
    footer_row.append(&help_wrap);
    pane.append(&footer_row);

    // ========================================================
    // Helper de actualización de vista previa de mensaje
    // ========================================================
    let update_msg_preview: Rc<dyn Fn()> = {
        let cfg_c = cfg.clone();
        let preview_c = msg_preview.clone();
        let set_btn_c = set_msg_btn.clone();
        Rc::new(move || {
            let c = cfg_c.borrow();
            set_btn_c.set_sensitive(c.show_lock_message);
            if let Some(msg) = c.effective_lock_message() {
                preview_c.set_label(&format!("«{msg}»"));
                preview_c.set_visible(true);
            } else {
                preview_c.set_visible(false);
            }
        })
    };
    update_msg_preview();

    // ========================================================
    // Señales interactivas
    // ========================================================
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        bat_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            if let Some((_, secs)) = DISPLAY_OFF_OPTIONS.get(dd.selected() as usize) {
                let mut c = cfg_c.borrow_mut();
                c.battery_display_off_secs = *secs;
                let _ = c.save();
            }
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        ac_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            if let Some((_, secs)) = DISPLAY_OFF_OPTIONS.get(dd.selected() as usize) {
                let mut c = cfg_c.borrow_mut();
                c.ac_display_off_secs = *secs;
                let _ = c.save();
            }
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        req_pwd_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            if let Some((_, secs)) = REQUIRE_PASSWORD_OPTIONS.get(dd.selected() as usize) {
                let mut c = cfg_c.borrow_mut();
                c.require_password_after_secs = *secs;
                let _ = c.save();
            }
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        hints_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.show_password_hints = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        let upd_c = update_msg_preview.clone();
        let win_c = window.clone();
        msg_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let should_prompt = {
                    let mut c = cfg_c.borrow_mut();
                    c.show_lock_message = active;
                    let _ = c.save();
                    active && c.lock_message.trim().is_empty()
                };
                upd_c();
                if should_prompt {
                    show_lock_message_modal(&win_c, &cfg_c, upd_c.clone());
                }
            }
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let upd_c = update_msg_preview.clone();
        let win_c = window.clone();
        set_msg_btn.connect_clicked(move |_| {
            show_lock_message_modal(&win_c, &cfg_c, upd_c.clone());
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        live_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.show_live_activities = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        rb_user_list.connect_toggled(move |rb| {
            if sync_c.get() || !rb.is_active() {
                return;
            }
            let mut c = cfg_c.borrow_mut();
            c.login_window_mode = LoginWindowMode::UserList;
            let _ = c.save();
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        rb_name_pwd.connect_toggled(move |rb| {
            if sync_c.get() || !rb.is_active() {
                return;
            }
            let mut c = cfg_c.borrow_mut();
            c.login_window_mode = LoginWindowMode::NameAndPassword;
            let _ = c.save();
        });
    }
    {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        power_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.show_power_buttons = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let win_c = window.clone();
        let nav_c = navigate.clone();
        a11y_btn.connect_clicked(move |_| {
            show_accessibility_options_modal(&win_c, nav_c.clone());
        });
    }

    // Refresco desde disco al volver a entrar al panel
    let refresh: Rc<dyn Fn()> = {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        let bat_c = bat_dd;
        let ac_c = ac_dd;
        let req_c = req_pwd_dd;
        let hints_c = hints_sw;
        let msg_c = msg_sw;
        let live_c = live_sw;
        let rb_ul_c = rb_user_list;
        let rb_np_c = rb_name_pwd;
        let pow_c = power_sw;
        let upd_c = update_msg_preview;
        Rc::new(move || {
            let loaded = LockScreenConfig::load();
            *cfg_c.borrow_mut() = loaded.clone();
            sync_c.set(true);
            bat_c.set_selected(display_off_index_for_secs(loaded.battery_display_off_secs));
            ac_c.set_selected(display_off_index_for_secs(loaded.ac_display_off_secs));
            req_c.set_selected(require_password_index_for_secs(
                loaded.require_password_after_secs,
            ));
            hints_c.set_active(loaded.show_password_hints);
            msg_c.set_active(loaded.show_lock_message);
            live_c.set_active(loaded.show_live_activities);
            match loaded.login_window_mode {
                LoginWindowMode::UserList => rb_ul_c.set_active(true),
                LoginWindowMode::NameAndPassword => rb_np_c.set_active(true),
            }
            pow_c.set_active(loaded.show_power_buttons);
            sync_c.set(false);
            upd_c();
        })
    };

    scrolled.set_child(Some(&pane));
    (scrolled, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockscreen_option_mappings_match_defaults_and_bounds() {
        assert_eq!(display_off_index_for_secs(120), 1);
        assert_eq!(display_off_index_for_secs(600), 4);
        assert_eq!(
            display_off_index_for_secs(0),
            (DISPLAY_OFF_OPTIONS.len() - 1) as u32
        );
        assert_eq!(require_password_index_for_secs(0), 0);
        assert_eq!(require_password_index_for_secs(300), 3);
        assert_eq!(
            require_password_index_for_secs(-1),
            (REQUIRE_PASSWORD_OPTIONS.len() - 1) as u32
        );
    }
}
