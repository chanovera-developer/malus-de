use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Entry, Image, Label, MenuButton, Orientation, PolicyType,
    Popover, ScrolledWindow, Stack,
};
use malus_de_common::apps::{index_applications, resolve_icon_file_path, DesktopAppEntry};
use malus_de_common::config::FocusConfig;
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::RefCell;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_button,
    make_squircle_colored_badge, wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const FOCUS_PANEL_ID: &str = "focus";
pub const FOCUS_DETAIL_SUBPANEL_ID: &str = "focus-detail";
pub const FOCUS_STATUS_SUBPANEL_ID: &str = "focus-status";
pub const FOCUS_PANEL_TITLE: &str = "Enfoque";

const COLOR_PRESETS: [(&str, &str, (f64, f64, f64), (f64, f64, f64)); 6] = [
    ("indigo", "Índigo", (0.37, 0.36, 0.90), (0.29, 0.27, 0.78)),
    ("purple", "Púrpura", (0.85, 0.28, 0.94), (0.64, 0.14, 0.75)),
    ("blue", "Azul", (0.14, 0.54, 1.00), (0.04, 0.42, 0.90)),
    ("green", "Verde", (0.20, 0.78, 0.35), (0.13, 0.64, 0.25)),
    ("orange", "Naranja", (1.00, 0.62, 0.04), (0.88, 0.46, 0.02)),
    ("teal", "Turquesa", (0.19, 0.69, 0.78), (0.10, 0.54, 0.65)),
];

const QUICK_PRESETS: [(&str, &str, &str); 5] = [
    ("Trabajo", "applications-office-symbolic", "blue"),
    ("Personal", "avatar-default-symbolic", "purple"),
    ("Sueño", "weather-clear-night-symbolic", "teal"),
    ("Lectura", "accessories-dictionary-symbolic", "orange"),
    ("Juegos", "input-gaming-symbolic", "green"),
];

pub fn is_focus_subpanel(id: &str) -> bool {
    matches!(
        id,
        FOCUS_PANEL_ID | FOCUS_DETAIL_SUBPANEL_ID | FOCUS_STATUS_SUBPANEL_ID
    )
}

fn palette_for_color(color: &str) -> ((f64, f64, f64), (f64, f64, f64)) {
    COLOR_PRESETS
        .iter()
        .find(|(id, _, _, _)| *id == color)
        .map(|(_, _, top, bot)| (*top, *bot))
        .unwrap_or((COLOR_PRESETS[0].2, COLOR_PRESETS[0].3))
}

fn sync_gsettings_dnd(active: bool) {
    if let Some(source) = gtk4::gio::SettingsSchemaSource::default() {
        if source
            .lookup("org.gnome.desktop.notifications", true)
            .is_some()
        {
            let settings = gtk4::gio::Settings::new("org.gnome.desktop.notifications");
            let _ = settings.set_boolean("show-banners", !active);
        }
    }
}

/// Devuelve un cargador bajo demanda para «Enfoque», el detalle de cada enfoque (`focus-detail`)
/// y la subvista «Estado de enfoque» (`focus-status`), junto con su proveedor de título.
pub fn create_focus_pane_loader(
    stack: &Stack,
    navigate: Rc<dyn Fn(&str)>,
) -> (Rc<dyn Fn(&str)>, Rc<dyn Fn(&str) -> Option<String>>) {
    let stack_c = stack.clone();
    let cfg = Rc::new(RefCell::new(FocusConfig::load()));
    let selected_mode_id: Rc<RefCell<String>> =
        Rc::new(RefCell::new("do-not-disturb".to_string()));
    let refresh_main: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuild_detail: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_status: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let title_fn: Rc<dyn Fn(&str) -> Option<String>> = {
        let cfg_t = cfg.clone();
        let sel_t = selected_mode_id.clone();
        Rc::new(move |id: &str| match id {
            FOCUS_PANEL_ID => Some(FOCUS_PANEL_TITLE.to_string()),
            FOCUS_STATUS_SUBPANEL_ID => Some("Estado de enfoque".to_string()),
            FOCUS_DETAIL_SUBPANEL_ID => {
                let target_id = sel_t.borrow().clone();
                let title = cfg_t
                    .borrow()
                    .mode_by_id(&target_id)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| FOCUS_PANEL_TITLE.to_string());
                Some(title)
            }
            _ => None,
        })
    };

    let loader: Rc<dyn Fn(&str)> = Rc::new(move |target: &str| {
        *cfg.borrow_mut() = FocusConfig::load();
        match target {
            FOCUS_PANEL_ID => {
                if stack_c.child_by_name(FOCUS_PANEL_ID).is_none() {
                    let (pane, rf) = build_focus_main_pane(&cfg, &selected_mode_id, &navigate);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(FOCUS_PANEL_ID));
                    *refresh_main.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_main.borrow().as_ref() {
                    rf();
                }
            }
            FOCUS_DETAIL_SUBPANEL_ID => {
                if stack_c.child_by_name(FOCUS_DETAIL_SUBPANEL_ID).is_none() {
                    let (pane, rb) =
                        build_focus_detail_pane(&cfg, &selected_mode_id, &navigate);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(FOCUS_DETAIL_SUBPANEL_ID));
                    *rebuild_detail.borrow_mut() = Some(rb);
                }
                if let Some(rb) = rebuild_detail.borrow().as_ref() {
                    rb();
                }
            }
            FOCUS_STATUS_SUBPANEL_ID => {
                if stack_c.child_by_name(FOCUS_STATUS_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_focus_status_pane(&cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(FOCUS_STATUS_SUBPANEL_ID));
                    *refresh_status.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_status.borrow().as_ref() {
                    rf();
                }
            }
            _ => {}
        }
    });

    (loader, title_fn)
}

fn build_focus_main_pane(
    cfg: &Rc<RefCell<FocusConfig>>,
    selected_mode_id: &Rc<RefCell<String>>,
    navigate: &Rc<dyn Fn(&str)>,
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
    scrolled.set_child(Some(&pane));

    // 1. Tarjeta superior: lista de modos de Enfoque (No molestar, Menos interrupciones + personalizados)
    let modes_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    pane.append(&wrap_squircle_card(&modes_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Botón derecho "Agregar enfoque..." (idéntico a la captura de macOS Tahoe)
    let add_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (add_wrap, add_btn) = make_squircle_button("Agregar enfoque...", false, None);
    let add_popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .position(gtk4::PositionType::Bottom)
        .css_classes(["malus-popover-menu"])
        .build();
    add_popover.set_parent(&add_btn);
    let add_pop_unroot = add_popover.clone();
    add_btn.connect_unrealize(move |_| {
        add_pop_unroot.unparent();
    });
    add_row.append(&add_wrap);
    pane.append(&add_row);

    // 3. Tarjeta "Compartir entre dispositivos" (Por implementar)
    let share_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let share_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let share_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let share_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    share_title_row.append(
        &Label::builder()
            .label("Compartir entre dispositivos")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    share_title_row.append(&make_pending_badge());
    share_col.append(&share_title_row);
    share_col.append(
        &Label::builder()
            .label("El modo de enfoque se comparte entre todos tus dispositivos, por lo que activarlo en uno lo activará en todos.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    share_row.append(&share_col);
    let share_sw = Switch::new(cfg.borrow().share_across_devices);
    share_sw.set_valign(Align::Center);
    {
        let c = cfg.clone();
        share_sw.connect_state_set(move |_, active| {
            c.borrow_mut().share_across_devices = active;
            let _ = c.borrow().save();
            glib::Propagation::Proceed
        });
    }
    share_row.append(&*share_sw);
    share_card.append(&share_row);
    pane.append(&wrap_squircle_card(&share_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 4. Tarjeta "Estado de enfoque" (Por implementar, con navegación a subvista)
    let status_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let status_inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let status_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let status_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    status_title_row.append(
        &Label::builder()
            .label("Estado de enfoque")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    status_title_row.append(&make_pending_badge());
    status_col.append(&status_title_row);
    status_col.append(
        &Label::builder()
            .label("Cuando autorizas a una app, esta puede compartir que silenciaste las notificaciones cuando usas un enfoque.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    status_inner.append(&status_col);
    let status_val_lbl = Label::builder()
        .label(if cfg.borrow().share_focus_status {
            "Activado"
        } else {
            "Desactivado"
        })
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    status_inner.append(&status_val_lbl);
    status_inner.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let status_btn = Button::builder()
        .child(&status_inner)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    {
        let nav = navigate.clone();
        status_btn.connect_clicked(move |_| {
            nav(FOCUS_STATUS_SUBPANEL_ID);
        });
    }
    status_card.append(&status_btn);
    pane.append(&wrap_squircle_card(&status_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 5. Botón de ayuda (?) en la esquina inferior derecha
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (help_wrap, _) = make_help_circle_button("Ayuda sobre Enfoque");
    footer.append(&help_wrap);
    pane.append(&footer);

    // Refresco reactivo de la lista de enfoques y etiquetas de estado
    let rebuild_modes: Rc<dyn Fn()> = {
        let card = modes_card.clone();
        let cfg_r = cfg.clone();
        let sel_r = selected_mode_id.clone();
        let nav_r = navigate.clone();
        let share_sw_r = share_sw.clone();
        let status_lbl_r = status_val_lbl.clone();
        Rc::new(move || {
            while let Some(child) = card.first_child() {
                card.remove(&child);
            }
            let conf = cfg_r.borrow().clone();
            share_sw_r.set_active(conf.share_across_devices);
            status_lbl_r.set_label(if conf.share_focus_status {
                "Activado"
            } else {
                "Desactivado"
            });

            let active_now_id = conf.active_mode_now().map(|m| m.id.clone());

            for (idx, mode) in conf.modes.iter().enumerate() {
                if idx > 0 {
                    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .build();

                let (top, bot) = palette_for_color(&mode.color);
                row.append(&make_squircle_colored_badge(
                    28,
                    &mode.icon,
                    15,
                    WIFI_BADGE_SQUIRCLE_RADIUS,
                    top,
                    bot,
                ));

                let text_col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(1)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build();
                text_col.append(
                    &Label::builder()
                        .label(&mode.name)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .build(),
                );
                if mode.schedule.enabled {
                    text_col.append(
                        &Label::builder()
                            .label(&format!("Horario: {}", mode.schedule.summary_label()))
                            .css_classes(["card-section-sub"])
                            .xalign(0.0)
                            .build(),
                    );
                }
                row.append(&text_col);

                if active_now_id.as_deref() == Some(mode.id.as_str()) {
                    row.append(
                        &Label::builder()
                            .label("Activado")
                            .css_classes(["card-section-sub"])
                            .valign(Align::Center)
                            .build(),
                    );
                }

                row.append(
                    &Image::builder()
                        .icon_name("go-next-symbolic")
                        .pixel_size(14)
                        .css_classes(["net-row-chevron"])
                        .valign(Align::Center)
                        .build(),
                );

                let btn = Button::builder()
                    .child(&row)
                    .css_classes(["net-service-row"])
                    .has_frame(false)
                    .hexpand(true)
                    .build();
                let mode_id = mode.id.clone();
                let sel_c = sel_r.clone();
                let nav_c = nav_r.clone();
                btn.connect_clicked(move |_| {
                    *sel_c.borrow_mut() = mode_id.clone();
                    nav_c(FOCUS_DETAIL_SUBPANEL_ID);
                });
                card.append(&btn);
            }
        })
    };

    // Popover de "Agregar enfoque..." (presets rápidos + nombre personalizado)
    {
        let pop_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .css_classes(["wifi-popover-box"])
            .width_request(240)
            .build();
        pop_box.append(
            &Label::builder()
                .label("Nuevo enfoque")
                .css_classes(["wifi-section-title"])
                .xalign(0.0)
                .margin_start(6)
                .margin_top(2)
                .build(),
        );

        let name_entry = Entry::builder()
            .placeholder_text("Nombre personalizado...")
            .hexpand(true)
            .margin_start(4)
            .margin_end(4)
            .build();
        pop_box.append(&name_entry);

        let (create_wrap, create_btn) = make_squircle_button("Crear enfoque", true, None);
        create_wrap.set_halign(Align::End);
        create_wrap.set_margin_end(4);
        create_wrap.set_margin_top(2);
        {
            let entry_c = name_entry.clone();
            let cfg_c = cfg.clone();
            let sel_c = selected_mode_id.clone();
            let nav_c = navigate.clone();
            let pop_c = add_popover.clone();
            let rf_c = rebuild_modes.clone();
            create_btn.connect_clicked(move |_| {
                let text = entry_c.text().trim().to_string();
                if text.is_empty() {
                    return;
                }
                let new_id = {
                    let mut guard = cfg_c.borrow_mut();
                    let id = guard.add_custom_mode(&text, "weather-clear-night-symbolic", "blue");
                    let _ = guard.save();
                    id
                };
                entry_c.set_text("");
                pop_c.popdown();
                rf_c();
                *sel_c.borrow_mut() = new_id;
                nav_c(FOCUS_DETAIL_SUBPANEL_ID);
            });
        }
        pop_box.append(&create_wrap);

        pop_box.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        pop_box.append(
            &Label::builder()
                .label("Sugerencias")
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .margin_start(6)
                .margin_top(2)
                .build(),
        );

        for &(preset_name, preset_icon, preset_color) in &QUICK_PRESETS {
            let (item_wrap, item_btn) = make_popover_action_row(preset_name, false, None);
            let cfg_c = cfg.clone();
            let sel_c = selected_mode_id.clone();
            let nav_c = navigate.clone();
            let pop_c = add_popover.clone();
            let rf_c = rebuild_modes.clone();
            item_btn.connect_clicked(move |_| {
                let new_id = {
                    let mut guard = cfg_c.borrow_mut();
                    let id = guard.add_custom_mode(preset_name, preset_icon, preset_color);
                    let _ = guard.save();
                    id
                };
                pop_c.popdown();
                rf_c();
                *sel_c.borrow_mut() = new_id;
                nav_c(FOCUS_DETAIL_SUBPANEL_ID);
            });
            pop_box.append(&item_wrap);
        }

        add_popover.set_child(Some(&pop_box));
        let pop_open = add_popover.clone();
        add_btn.connect_clicked(move |_| {
            pop_open.popup();
        });
    }

    rebuild_modes();
    (scrolled, rebuild_modes)
}

fn build_focus_detail_pane(
    cfg: &Rc<RefCell<FocusConfig>>,
    selected_mode_id: &Rc<RefCell<String>>,
    navigate: &Rc<dyn Fn(&str)>,
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
    scrolled.set_child(Some(&pane));

    let pane_c = pane.clone();
    let cfg_c = cfg.clone();
    let sel_c = selected_mode_id.clone();
    let nav_c = navigate.clone();

    let rebuild: Rc<dyn Fn()> = Rc::new(move || {
        while let Some(child) = pane_c.first_child() {
            pane_c.remove(&child);
        }
        let mode_id = sel_c.borrow().clone();
        let (mode, is_manual_active) = {
            let guard = cfg_c.borrow();
            let Some(m) = guard.mode_by_id(&mode_id).cloned() else {
                return;
            };
            let manual = guard.active_focus_id.as_deref() == Some(mode_id.as_str());
            (m, manual)
        };

        // 1. Tarjeta de cabecera + activación real del modo de Enfoque
        let hdr_card = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-card-compact", "wifi-header-section"])
            .build();
        let (top, bot) = palette_for_color(&mode.color);
        hdr_card.append(&make_squircle_colored_badge(
            34,
            &mode.icon,
            18,
            WIFI_BADGE_SQUIRCLE_RADIUS,
            top,
            bot,
        ));
        let hdr_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        hdr_col.append(
            &Label::builder()
                .label(&mode.name)
                .css_classes(["card-section-title"])
                .xalign(0.0)
                .build(),
        );
        let subtitle_text = if mode.id == "do-not-disturb" {
            "Silencia las alertas y notificaciones mientras este enfoque esté activo."
        } else if mode.id == "reduce-interruptions" {
            "Permite solo las notificaciones urgentes y las apps que autorices."
        } else {
            "Silencia notificaciones no permitidas mientras este enfoque esté activo."
        };
        hdr_col.append(
            &Label::builder()
                .label(subtitle_text)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        hdr_card.append(&hdr_col);

        let active_sw = Switch::new(is_manual_active);
        active_sw.set_valign(Align::Center);
        {
            let c = cfg_c.clone();
            let id_c = mode.id.clone();
            active_sw.connect_state_set(move |_, on| {
                {
                    let mut guard = c.borrow_mut();
                    guard.active_focus_id = if on { Some(id_c.clone()) } else { None };
                    let _ = guard.save();
                }
                sync_gsettings_dnd(c.borrow().is_active_now());
                glib::Propagation::Proceed
            });
        }
        hdr_card.append(&*active_sw);
        pane_c.append(&wrap_squircle_card(&hdr_card, WIFI_CARD_SQUIRCLE_RADIUS));

        // 2. Sección "Permitir notificaciones"
        pane_c.append(
            &Label::builder()
                .label("Permitir notificaciones")
                .css_classes(["wifi-section-title"])
                .xalign(0.0)
                .margin_top(4)
                .build(),
        );
        let allow_card = GtkBox::builder().orientation(Orientation::Vertical).build();

        // 2a. Personas permitidas (Por implementar)
        let people_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wifi-pref-row"])
            .build();
        people_row.append(
            &Label::builder()
                .label("Personas permitidas")
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .hexpand(true)
                .valign(Align::Center)
                .build(),
        );
        people_row.append(&make_pending_badge());
        people_row.append(
            &Label::builder()
                .label("Ninguna")
                .css_classes(["card-section-sub"])
                .valign(Align::Center)
                .build(),
        );
        allow_card.append(&people_row);
        allow_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

        // 2b. Apps permitidas (REAL)
        let apps_hdr_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wifi-pref-row"])
            .build();
        let apps_hdr_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(1)
            .hexpand(true)
            .build();
        apps_hdr_col.append(
            &Label::builder()
                .label("Apps permitidas")
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        apps_hdr_col.append(
            &Label::builder()
                .label("Las notificaciones de estas apps se mostrarán aunque el enfoque esté activado.")
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        apps_hdr_row.append(&apps_hdr_col);

        let allowed_list_box = GtkBox::builder().orientation(Orientation::Vertical).build();
        let refresh_allowed_apps: Rc<dyn Fn()> = {
            let list_b = allowed_list_box.clone();
            let c = cfg_c.clone();
            let id_c = mode.id.clone();
            Rc::new(move || {
                while let Some(ch) = list_b.first_child() {
                    list_b.remove(&ch);
                }
                let apps = c
                    .borrow()
                    .mode_by_id(&id_c)
                    .map(|m| m.allowed_apps.clone())
                    .unwrap_or_default();
                if apps.is_empty() {
                    let empty = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .css_classes(["wifi-pref-row"])
                        .margin_start(14)
                        .build();
                    empty.append(
                        &Label::builder()
                            .label("Ninguna app permitida todavía.")
                            .css_classes(["card-section-sub"])
                            .xalign(0.0)
                            .build(),
                    );
                    list_b.append(&empty);
                    return;
                }
                for app_name in apps {
                    list_b.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                    let r = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(10)
                        .css_classes(["wifi-pref-row"])
                        .margin_start(14)
                        .build();
                    r.append(
                        &Label::builder()
                            .label(&app_name)
                            .css_classes(["wifi-row-title"])
                            .xalign(0.0)
                            .hexpand(true)
                            .valign(Align::Center)
                            .build(),
                    );
                    let rm_btn = Button::builder()
                        .icon_name("window-close-symbolic")
                        .tooltip_text("Quitar app")
                        .has_frame(false)
                        .css_classes(["flat"])
                        .valign(Align::Center)
                        .build();
                    let c_rm = c.clone();
                    let id_rm = id_c.clone();
                    let app_rm = app_name.clone();
                    let list_rm = list_b.clone();
                    let r_rm = r.clone();
                    rm_btn.connect_clicked(move |_| {
                        {
                            let mut guard = c_rm.borrow_mut();
                            if let Some(m) = guard.mode_by_id_mut(&id_rm) {
                                m.allowed_apps.retain(|x| !x.eq_ignore_ascii_case(&app_rm));
                            }
                            let _ = guard.save();
                        }
                        if let Some(prev) = r_rm.prev_sibling() {
                            list_rm.remove(&prev);
                        }
                        list_rm.remove(&r_rm);
                    });
                    r.append(&rm_btn);
                    list_b.append(&r);
                }
            })
        };

        let (add_app_wrap, add_app_btn) = make_squircle_button("Agregar app...", false, None);
        let apps_pop = Popover::builder()
            .has_arrow(false)
            .autohide(true)
            .position(gtk4::PositionType::Bottom)
            .css_classes(["malus-popover-menu"])
            .build();
        apps_pop.set_parent(&add_app_btn);
        let apps_pop_unroot = apps_pop.clone();
        add_app_btn.connect_unrealize(move |_| {
            apps_pop_unroot.unparent();
        });
        {
            let pop_open = apps_pop.clone();
            let c_pop = cfg_c.clone();
            let id_pop = mode.id.clone();
            let rf_apps = refresh_allowed_apps.clone();
            add_app_btn.connect_clicked(move |_| {
                let scroll = ScrolledWindow::builder()
                    .hscrollbar_policy(PolicyType::Never)
                    .vscrollbar_policy(PolicyType::Automatic)
                    .max_content_height(280)
                    .propagate_natural_height(true)
                    .width_request(230)
                    .build();
                let list = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .css_classes(["wifi-popover-box"])
                    .build();
                let already = c_pop
                    .borrow()
                    .mode_by_id(&id_pop)
                    .map(|m| m.allowed_apps.clone())
                    .unwrap_or_default();
                let installed = load_sorted_unique_apps();
                let mut added_count = 0usize;
                for app in installed {
                    if already.iter().any(|a| a.eq_ignore_ascii_case(&app.name)) {
                        continue;
                    }
                    let (row_w, row_b) = make_popover_action_row(&app.name, false, None);
                    let c_sel = c_pop.clone();
                    let id_sel = id_pop.clone();
                    let name_sel = app.name.clone();
                    let pop_sel = pop_open.clone();
                    let rf_sel = rf_apps.clone();
                    row_b.connect_clicked(move |_| {
                        {
                            let mut guard = c_sel.borrow_mut();
                            if let Some(m) = guard.mode_by_id_mut(&id_sel) {
                                if !m
                                    .allowed_apps
                                    .iter()
                                    .any(|a| a.eq_ignore_ascii_case(&name_sel))
                                {
                                    m.allowed_apps.push(name_sel.clone());
                                    m.allowed_apps.sort_by_key(|a| a.to_ascii_lowercase());
                                }
                            }
                            let _ = guard.save();
                        }
                        pop_sel.popdown();
                        rf_sel();
                    });
                    list.append(&row_w);
                    added_count += 1;
                }
                if added_count == 0 {
                    list.append(
                        &Label::builder()
                            .label("Todas las apps ya están permitidas")
                            .css_classes(["card-section-sub"])
                            .margin_top(6)
                            .margin_bottom(6)
                            .margin_start(8)
                            .margin_end(8)
                            .build(),
                    );
                }
                scroll.set_child(Some(&list));
                pop_open.set_child(Some(&scroll));
                pop_open.popup();
            });
        }
        apps_hdr_row.append(&add_app_wrap);
        allow_card.append(&apps_hdr_row);
        allow_card.append(&allowed_list_box);
        refresh_allowed_apps();
        pane_c.append(&wrap_squircle_card(&allow_card, WIFI_CARD_SQUIRCLE_RADIUS));

        // 3. Sección "Opciones"
        pane_c.append(
            &Label::builder()
                .label("Opciones")
                .css_classes(["wifi-section-title"])
                .xalign(0.0)
                .margin_top(4)
                .build(),
        );
        let opts_card = GtkBox::builder().orientation(Orientation::Vertical).build();

        // 3a. Permitir notificaciones urgentes (REAL)
        opts_card.append(&make_focus_switch_row(
            "Permitir notificaciones urgentes",
            Some("Las notificaciones marcadas como críticas se mostrarán inmediatamente."),
            mode.allow_urgent,
            false,
            {
                let c = cfg_c.clone();
                let id_c = mode.id.clone();
                move |v| {
                    let mut guard = c.borrow_mut();
                    if let Some(m) = guard.mode_by_id_mut(&id_c) {
                        m.allow_urgent = v;
                    }
                    let _ = guard.save();
                }
            },
        ));
        opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

        // 3b. Silenciar globos de notificación en el Dock (REAL)
        opts_card.append(&make_focus_switch_row(
            "Ocultar globos de notificación en el Dock",
            Some("Evita incrementar el contador rojo en los íconos de apps silenciadas."),
            mode.silence_badges,
            false,
            {
                let c = cfg_c.clone();
                let id_c = mode.id.clone();
                move |v| {
                    let mut guard = c.borrow_mut();
                    if let Some(m) = guard.mode_by_id_mut(&id_c) {
                        m.silence_badges = v;
                    }
                    let _ = guard.save();
                }
            },
        ));
        opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

        // 3c. Atenuar pantalla bloqueada (Por implementar)
        opts_card.append(&make_focus_switch_row(
            "Atenuar pantalla bloqueada",
            None,
            mode.dim_lock_screen,
            true,
            {
                let c = cfg_c.clone();
                let id_c = mode.id.clone();
                move |v| {
                    let mut guard = c.borrow_mut();
                    if let Some(m) = guard.mode_by_id_mut(&id_c) {
                        m.dim_lock_screen = v;
                    }
                    let _ = guard.save();
                }
            },
        ));
        pane_c.append(&wrap_squircle_card(&opts_card, WIFI_CARD_SQUIRCLE_RADIUS));

        // 4. Sección "Establecer horario" (REAL)
        pane_c.append(
            &Label::builder()
                .label("Establecer horario")
                .css_classes(["wifi-section-title"])
                .xalign(0.0)
                .margin_top(4)
                .build(),
        );
        let sched_card = GtkBox::builder().orientation(Orientation::Vertical).build();
        sched_card.append(&make_focus_switch_row(
            "Activar según horario",
            Some("Activa automáticamente este enfoque dentro de la franja horaria indicada."),
            mode.schedule.enabled,
            false,
            {
                let c = cfg_c.clone();
                let id_c = mode.id.clone();
                move |v| {
                    {
                        let mut guard = c.borrow_mut();
                        if let Some(m) = guard.mode_by_id_mut(&id_c) {
                            m.schedule.enabled = v;
                        }
                        let _ = guard.save();
                    }
                    sync_gsettings_dnd(c.borrow().is_active_now());
                }
            },
        ));
        sched_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

        let times_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        times_row.append(
            &Label::builder()
                .label("Desde")
                .css_classes(["wifi-row-title"])
                .valign(Align::Center)
                .build(),
        );
        let start_lbl = Label::builder()
            .label(&format!(
                "{:02}:{:02}",
                mode.schedule.start_hour, mode.schedule.start_minute
            ))
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build();
        times_row.append(&build_hour_menu_button(&start_lbl, {
            let c = cfg_c.clone();
            let id_c = mode.id.clone();
            let lbl = start_lbl.clone();
            move |h, m| {
                {
                    let mut guard = c.borrow_mut();
                    if let Some(mode_mut) = guard.mode_by_id_mut(&id_c) {
                        mode_mut.schedule.start_hour = h;
                        mode_mut.schedule.start_minute = m;
                    }
                    let _ = guard.save();
                }
                lbl.set_label(&format!("{h:02}:{m:02}"));
                sync_gsettings_dnd(c.borrow().is_active_now());
            }
        }));

        times_row.append(
            &Label::builder()
                .label("Hasta")
                .css_classes(["wifi-row-title"])
                .margin_start(16)
                .valign(Align::Center)
                .build(),
        );
        let end_lbl = Label::builder()
            .label(&format!(
                "{:02}:{:02}",
                mode.schedule.end_hour, mode.schedule.end_minute
            ))
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build();
        times_row.append(&build_hour_menu_button(&end_lbl, {
            let c = cfg_c.clone();
            let id_c = mode.id.clone();
            let lbl = end_lbl.clone();
            move |h, m| {
                {
                    let mut guard = c.borrow_mut();
                    if let Some(mode_mut) = guard.mode_by_id_mut(&id_c) {
                        mode_mut.schedule.end_hour = h;
                        mode_mut.schedule.end_minute = m;
                    }
                    let _ = guard.save();
                }
                lbl.set_label(&format!("{h:02}:{m:02}"));
                sync_gsettings_dnd(c.borrow().is_active_now());
            }
        }));
        sched_card.append(&times_row);
        pane_c.append(&wrap_squircle_card(&sched_card, WIFI_CARD_SQUIRCLE_RADIUS));

        // 5. Filtros de enfoque (Por implementar)
        let filter_card = GtkBox::builder().orientation(Orientation::Vertical).build();
        let filter_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wifi-pref-row"])
            .build();
        let filter_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        let filter_title = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        filter_title.append(
            &Label::builder()
                .label("Filtros de enfoque")
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        filter_title.append(&make_pending_badge());
        filter_col.append(&filter_title);
        filter_col.append(
            &Label::builder()
                .label("Personaliza cómo se comportan tus apps y el sistema al activar este enfoque.")
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        filter_row.append(&filter_col);
        filter_card.append(&filter_row);
        pane_c.append(&wrap_squircle_card(&filter_card, WIFI_CARD_SQUIRCLE_RADIUS));

        // 6. Eliminar enfoque (solo para enfoques personalizados)
        if !mode.builtin {
            let del_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .halign(Align::End)
                .margin_top(4)
                .build();
            let (del_wrap, del_btn) = make_squircle_button("Eliminar enfoque", false, None);
            let c_del = cfg_c.clone();
            let id_del = mode.id.clone();
            let nav_del = nav_c.clone();
            del_btn.connect_clicked(move |_| {
                {
                    let mut guard = c_del.borrow_mut();
                    guard.remove_custom_mode(&id_del);
                    let _ = guard.save();
                }
                sync_gsettings_dnd(c_del.borrow().is_active_now());
                nav_del(FOCUS_PANEL_ID);
            });
            del_row.append(&del_wrap);
            pane_c.append(&del_row);
        }
    });

    (scrolled, rebuild)
}

fn build_focus_status_pane(cfg: &Rc<RefCell<FocusConfig>>) -> (ScrolledWindow, Rc<dyn Fn()>) {
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
    scrolled.set_child(Some(&pane));

    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    let title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    title_row.append(
        &Label::builder()
            .label("Compartir estado de enfoque")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    title_row.append(&make_pending_badge());
    col.append(&title_row);
    col.append(
        &Label::builder()
            .label("Permite a las apps autorizadas mostrar a otras personas que tienes las notificaciones silenciadas.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    row.append(&col);
    let sw = Switch::new(cfg.borrow().share_focus_status);
    sw.set_valign(Align::Center);
    {
        let c = cfg.clone();
        sw.connect_state_set(move |_, active| {
            c.borrow_mut().share_focus_status = active;
            let _ = c.borrow().save();
            glib::Propagation::Proceed
        });
    }
    row.append(&*sw);
    card.append(&row);
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let sw_c = sw.clone();
        Rc::new(move || {
            sw_c.set_active(c.borrow().share_focus_status);
        })
    };
    (scrolled, refresh)
}

fn make_focus_switch_row<F>(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
    pending: bool,
    on_toggle: F,
) -> GtkBox
where
    F: Fn(bool) + 'static,
{
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    title_row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    if pending {
        title_row.append(&make_pending_badge());
    }
    col.append(&title_row);
    if let Some(sub) = subtitle {
        col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    row.append(&col);
    let sw = Switch::new(initial);
    sw.set_valign(Align::Center);
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(&*sw);
    row
}

fn build_hour_menu_button<F>(value_lbl: &Label, on_select: F) -> MenuButton
where
    F: Fn(u8, u8) + 'static,
{
    let content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .valign(Align::Center)
        .build();
    content.append(value_lbl);
    content.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(11)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );

    let popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .position(gtk4::PositionType::Bottom)
        .css_classes(["malus-popover-menu"])
        .build();
    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .max_content_height(240)
        .propagate_natural_height(true)
        .width_request(110)
        .build();
    let list = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["wifi-popover-box"])
        .build();

    let cb = Rc::new(on_select);
    for hour in 0u8..24 {
        for minute in [0u8, 30u8] {
            let text = format!("{hour:02}:{minute:02}");
            let (wrap, btn) = make_popover_action_row(&text, false, None);
            let cb_c = cb.clone();
            let pop_c = popover.clone();
            btn.connect_clicked(move |_| {
                cb_c(hour, minute);
                pop_c.popdown();
            });
            list.append(&wrap);
        }
    }
    scroll.set_child(Some(&list));
    popover.set_child(Some(&scroll));

    MenuButton::builder()
        .child(&content)
        .popover(&popover)
        .css_classes(["flat"])
        .valign(Align::Center)
        .build()
}

fn load_sorted_unique_apps() -> Vec<DesktopAppEntry> {
    let mut apps = index_applications();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps.retain(|a| !a.name.trim().is_empty());
    let _ = resolve_icon_file_path;
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_subpanel_detection_and_palettes_work() {
        assert!(is_focus_subpanel(FOCUS_PANEL_ID));
        assert!(is_focus_subpanel(FOCUS_DETAIL_SUBPANEL_ID));
        assert!(is_focus_subpanel(FOCUS_STATUS_SUBPANEL_ID));
        assert!(!is_focus_subpanel("notifications"));
        assert_eq!(
            palette_for_color("indigo"),
            (COLOR_PRESETS[0].2, COLOR_PRESETS[0].3)
        );
        assert_eq!(
            palette_for_color("unknown"),
            (COLOR_PRESETS[0].2, COLOR_PRESETS[0].3)
        );
    }

    #[test]
    fn focus_mode_config_defaults_match_tahoe_modes() {
        let modes = malus_de_common::config::FocusModeConfig::default_modes();
        assert_eq!(modes.len(), 2);
        assert_eq!(modes[0].name, "No molestar");
        assert_eq!(modes[1].name, "Menos interrupciones");
    }
}
