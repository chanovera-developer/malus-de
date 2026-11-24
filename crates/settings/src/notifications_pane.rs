use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, MenuButton, Orientation, PolicyType, Popover,
    ScrolledWindow, Stack,
};
use malus_de_common::apps::{index_applications, resolve_icon_file_path, DesktopAppEntry};
use malus_de_common::config::{
    AppNotificationRule, NotificationAlertStyle, NotificationPreviewMode, NotificationsConfig,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::RefCell;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_colored_badge,
    wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const NOTIFICATIONS_PANEL_ID: &str = "notifications";
pub const NOTIFICATIONS_APP_SUBPANEL_ID: &str = "notifications-app";
pub const NOTIFICATIONS_PANEL_TITLE: &str = "Notificaciones";

const RED_TOP: (f64, f64, f64) = (1.0, 0.31, 0.27);
const RED_BOT: (f64, f64, f64) = (0.89, 0.16, 0.14);

const PREVIEW_OPTIONS: [(&str, NotificationPreviewMode); 3] = [
    ("Siempre", NotificationPreviewMode::Always),
    ("Si desbloqueaste", NotificationPreviewMode::WhenUnlocked),
    ("Nunca", NotificationPreviewMode::Never),
];

const APP_PREVIEW_OPTIONS: [(&str, Option<NotificationPreviewMode>); 4] = [
    ("Por omisión", None),
    ("Siempre", Some(NotificationPreviewMode::Always)),
    ("Si desbloqueaste", Some(NotificationPreviewMode::WhenUnlocked)),
    ("Nunca", Some(NotificationPreviewMode::Never)),
];

const STYLE_OPTIONS: [(&str, NotificationAlertStyle); 3] = [
    ("Ninguno", NotificationAlertStyle::None),
    ("Tiras", NotificationAlertStyle::Banners),
    ("Alertas", NotificationAlertStyle::Alerts),
];

pub fn is_notifications_subpanel(id: &str) -> bool {
    matches!(id, NOTIFICATIONS_PANEL_ID | NOTIFICATIONS_APP_SUBPANEL_ID)
}

fn preview_label(mode: NotificationPreviewMode) -> &'static str {
    PREVIEW_OPTIONS
        .iter()
        .find(|(_, m)| *m == mode)
        .map(|(l, _)| *l)
        .unwrap_or("Si desbloqueaste")
}

fn app_preview_label(mode: Option<NotificationPreviewMode>) -> &'static str {
    APP_PREVIEW_OPTIONS
        .iter()
        .find(|(_, m)| *m == mode)
        .map(|(l, _)| *l)
        .unwrap_or("Por omisión")
}

fn style_label(style: NotificationAlertStyle) -> &'static str {
    STYLE_OPTIONS
        .iter()
        .find(|(_, s)| *s == style)
        .map(|(l, _)| *l)
        .unwrap_or("Tiras")
}

#[derive(Clone)]
struct SelectedApp {
    name: String,
    icon: String,
}

/// Devuelve un cargador bajo demanda para «Notificaciones» y la vista de detalle por app
/// (`notifications-app`), junto con un proveedor del título activo de cabecera.
pub fn create_notifications_pane_loader(
    stack: &Stack,
    navigate: Rc<dyn Fn(&str)>,
) -> (Rc<dyn Fn(&str)>, Rc<dyn Fn(&str) -> Option<String>>) {
    let stack_c = stack.clone();
    let cfg = Rc::new(RefCell::new(NotificationsConfig::load()));
    let selected_app: Rc<RefCell<Option<SelectedApp>>> = Rc::new(RefCell::new(None));
    let refresh_main: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuild_app_detail: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let title_fn: Rc<dyn Fn(&str) -> Option<String>> = {
        let sel = selected_app.clone();
        Rc::new(move |id: &str| match id {
            NOTIFICATIONS_PANEL_ID => Some(NOTIFICATIONS_PANEL_TITLE.to_string()),
            NOTIFICATIONS_APP_SUBPANEL_ID => Some(
                sel.borrow()
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| NOTIFICATIONS_PANEL_TITLE.to_string()),
            ),
            _ => None,
        })
    };

    let loader: Rc<dyn Fn(&str)> = Rc::new(move |target: &str| match target {
        NOTIFICATIONS_PANEL_ID => {
            if stack_c.child_by_name(NOTIFICATIONS_PANEL_ID).is_none() {
                let (pane, rf) =
                    build_notifications_main_pane(&cfg, &selected_app, &navigate);
                crate::configure_right_pane_widget(pane.upcast_ref());
                stack_c.add_named(&pane, Some(NOTIFICATIONS_PANEL_ID));
                *refresh_main.borrow_mut() = Some(rf);
            }
            if let Some(rf) = refresh_main.borrow().as_ref() {
                rf();
            }
        }
        NOTIFICATIONS_APP_SUBPANEL_ID => {
            if stack_c.child_by_name(NOTIFICATIONS_APP_SUBPANEL_ID).is_none() {
                let (pane, rebuild) = build_app_detail_pane(&cfg, &selected_app);
                crate::configure_right_pane_widget(pane.upcast_ref());
                stack_c.add_named(&pane, Some(NOTIFICATIONS_APP_SUBPANEL_ID));
                *rebuild_app_detail.borrow_mut() = Some(rebuild);
            }
            if let Some(rb) = rebuild_app_detail.borrow().as_ref() {
                rb();
            }
        }
        _ => {}
    });

    (loader, title_fn)
}

fn build_notifications_main_pane(
    cfg: &Rc<RefCell<NotificationsConfig>>,
    selected_app: &Rc<RefCell<Option<SelectedApp>>>,
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

    // 1. Tarjeta superior (campana roja + Notificaciones + descripción)
    let banner_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    banner_card.append(&make_squircle_colored_badge(
        34,
        "preferences-system-notifications-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        RED_TOP,
        RED_BOT,
    ));
    let banner_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    banner_col.append(
        &Label::builder()
            .label(NOTIFICATIONS_PANEL_TITLE)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    banner_col.append(
        &Label::builder()
            .label("Personaliza cuándo y cómo se mostrarán las notificaciones, si reproducirán un sonido y qué apps las pueden enviar.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    banner_card.append(&banner_col);
    pane.append(&wrap_squircle_card(&banner_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Cabecera "Centro de notificaciones"
    let center_hdr = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .build();
    center_hdr.append(
        &Label::builder()
            .label("Centro de notificaciones")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    center_hdr.append(
        &Label::builder()
            .label("El centro de notificaciones muestra tus notificaciones en la esquina superior derecha de la pantalla y puedes mostrarlo u ocultarlo haciendo clic en el reloj en la barra de menús.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pane.append(&center_hdr);

    // 3. Tarjeta de opciones de "Centro de notificaciones"
    let center_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    // 3.1 Previsualizar (REAL, conectado a malus-de-notifications)
    let preview_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    preview_row.append(
        &Label::builder()
            .label("Previsualizar")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let preview_lbl = Label::builder()
        .label(preview_label(cfg.borrow().preview_mode))
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let preview_menu = build_inline_popover_menu(&preview_lbl, &PREVIEW_OPTIONS, {
        let c = cfg.clone();
        let lbl = preview_lbl.clone();
        move |label, mode| {
            c.borrow_mut().preview_mode = mode;
            let _ = c.borrow().save();
            lbl.set_label(label);
        }
    });
    preview_row.append(&preview_menu);
    center_card.append(&preview_row);
    center_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 3.2 Subtítulo "Mostrar notificaciones:" + filas indentadas (por implementar)
    let show_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-pref-row"])
        .build();
    show_hdr.append(
        &Label::builder()
            .label("Mostrar notificaciones:")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    center_card.append(&show_hdr);

    center_card.append(&make_indented_pending_switch_row(
        "cuando la pantalla esté en reposo",
        false,
    ));
    center_card.append(&make_indented_hairline());
    center_card.append(&make_indented_pending_switch_row(
        "cuando la pantalla está bloqueada",
        true,
    ));
    center_card.append(&make_indented_hairline());

    let share_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .margin_start(18)
        .build();
    share_row.append(
        &Label::builder()
            .label("al duplicar o compartir la pantalla")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    share_row.append(&make_pending_badge());
    share_row.append(
        &Label::builder()
            .label("Notificaciones desactivadas")
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build(),
    );
    center_card.append(&share_row);
    center_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 3.3 Resumir notificaciones (por implementar)
    let sum_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    sum_row.append(
        &Label::builder()
            .label("Resumir notificaciones")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    sum_row.append(&make_pending_badge());
    sum_row.append(
        &Label::builder()
            .label("No")
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build(),
    );
    sum_row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    center_card.append(&sum_row);
    pane.append(&wrap_squircle_card(&center_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 4. Sección "Notificaciones de apps" (REAL: lista de apps instaladas + subvista editable)
    pane.append(
        &Label::builder()
            .label("Notificaciones de apps")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let apps_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let installed = load_sorted_unique_apps();
    let summary_labels: Rc<RefCell<Vec<(String, Label)>>> = Rc::new(RefCell::new(Vec::new()));

    if installed.is_empty() {
        let empty = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["wifi-pref-row"])
            .build();
        empty.append(
            &Label::builder()
                .label("No se encontraron aplicaciones instaladas.")
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .build(),
        );
        apps_card.append(&empty);
    } else {
        for (idx, app) in installed.iter().enumerate() {
            if idx > 0 {
                apps_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
            }
            let row_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .build();

            let icon_img = make_app_icon_image(&app.icon, 28);
            row_box.append(&icon_img);

            let text_col = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .hexpand(true)
                .valign(Align::Center)
                .build();
            text_col.append(
                &Label::builder()
                    .label(&app.name)
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .build(),
            );
            let sum_lbl = Label::builder()
                .label(&cfg.borrow().rule_for_app(&app.name).summary_label())
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            text_col.append(&sum_lbl);
            summary_labels
                .borrow_mut()
                .push((app.name.clone(), sum_lbl));

            row_box.append(&text_col);
            row_box.append(
                &Image::builder()
                    .icon_name("go-next-symbolic")
                    .pixel_size(14)
                    .css_classes(["net-row-chevron"])
                    .valign(Align::Center)
                    .build(),
            );

            let btn = Button::builder()
                .child(&row_box)
                .css_classes(["net-service-row"])
                .has_frame(false)
                .hexpand(true)
                .build();
            let sel_c = selected_app.clone();
            let nav_c = navigate.clone();
            let app_name = app.name.clone();
            let app_icon = app.icon.clone();
            btn.connect_clicked(move |_| {
                *sel_c.borrow_mut() = Some(SelectedApp {
                    name: app_name.clone(),
                    icon: app_icon.clone(),
                });
                nav_c(NOTIFICATIONS_APP_SUBPANEL_ID);
            });
            apps_card.append(&btn);
        }
    }
    pane.append(&wrap_squircle_card(&apps_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 5. Pie inferior ("?")
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (help_wrap, _) = make_help_circle_button("Ayuda sobre Notificaciones");
    footer.append(&help_wrap);
    pane.append(&footer);

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let p_lbl = preview_lbl;
        let sums = summary_labels;
        Rc::new(move || {
            let loaded = NotificationsConfig::load();
            *c.borrow_mut() = loaded.clone();
            p_lbl.set_label(preview_label(loaded.preview_mode));
            for (name, lbl) in sums.borrow().iter() {
                lbl.set_label(&loaded.rule_for_app(name).summary_label());
            }
        })
    };

    (scrolled, refresh)
}

fn build_app_detail_pane(
    cfg: &Rc<RefCell<NotificationsConfig>>,
    selected_app: &Rc<RefCell<Option<SelectedApp>>>,
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

    let rebuild: Rc<dyn Fn()> = {
        let pane_c = pane.clone();
        let cfg_c = cfg.clone();
        let sel_c = selected_app.clone();
        Rc::new(move || {
            while let Some(child) = pane_c.first_child() {
                pane_c.remove(&child);
            }
            let Some(app) = sel_c.borrow().clone() else {
                return;
            };
            let current_rule = cfg_c.borrow().rule_for_app(&app.name);

            // Cabecera de la app
            let hdr_card = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-card-compact", "wifi-header-section"])
                .build();
            hdr_card.append(&make_app_icon_image(&app.icon, 36));
            let hdr_col = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .hexpand(true)
                .valign(Align::Center)
                .build();
            hdr_col.append(
                &Label::builder()
                    .label(&app.name)
                    .css_classes(["card-section-title"])
                    .xalign(0.0)
                    .build(),
            );
            hdr_col.append(
                &Label::builder()
                    .label("Configura si esta aplicación puede mostrar tiras, alertas y sonidos en Malus DE.")
                    .css_classes(["card-section-sub"])
                    .wrap(true)
                    .xalign(0.0)
                    .build(),
            );
            hdr_card.append(&hdr_col);
            pane_c.append(&wrap_squircle_card(&hdr_card, WIFI_CARD_SQUIRCLE_RADIUS));

            // Tarjeta "Permitir notificaciones"
            let allow_card = GtkBox::builder().orientation(Orientation::Vertical).build();
            let allow_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-pref-row"])
                .build();
            allow_row.append(
                &Label::builder()
                    .label("Permitir notificaciones")
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build(),
            );
            let allow_sw = Switch::builder()
                .active(current_rule.enabled)
                .css_classes(["macos-switch"])
                .valign(Align::Center)
                .build();
            allow_row.append(&*allow_sw);
            allow_card.append(&allow_row);
            pane_c.append(&wrap_squircle_card(&allow_card, WIFI_CARD_SQUIRCLE_RADIUS));

            // Tarjeta de opciones activas de la app
            let opts_card = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .sensitive(current_rule.enabled)
                .build();

            // Estilo de alerta (Ninguno / Tiras / Alertas)
            let style_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-pref-row"])
                .build();
            style_row.append(
                &Label::builder()
                    .label("Estilo de alerta")
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build(),
            );
            let style_lbl = Label::builder()
                .label(style_label(current_rule.style))
                .css_classes(["card-section-sub"])
                .valign(Align::Center)
                .build();
            let style_menu = build_inline_popover_menu(&style_lbl, &STYLE_OPTIONS, {
                let c = cfg_c.clone();
                let app_name = app.name.clone();
                let lbl = style_lbl.clone();
                move |label, style| {
                    let mut guard = c.borrow_mut();
                    let mut rule = guard.rule_for_app(&app_name);
                    rule.style = style;
                    guard.set_rule_for_app(&app_name, rule);
                    let _ = guard.save();
                    lbl.set_label(label);
                }
            });
            style_row.append(&style_menu);
            opts_card.append(&style_row);
            opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

            // Sonido
            let sound_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-pref-row"])
                .build();
            sound_row.append(
                &Label::builder()
                    .label("Reproducir sonido de notificación")
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build(),
            );
            let sound_sw = Switch::builder()
                .active(current_rule.sound)
                .css_classes(["macos-switch"])
                .valign(Align::Center)
                .build();
            {
                let c = cfg_c.clone();
                let app_name = app.name.clone();
                sound_sw.connect_state_set(move |_, active| {
                    let mut guard = c.borrow_mut();
                    let mut rule = guard.rule_for_app(&app_name);
                    rule.sound = active;
                    guard.set_rule_for_app(&app_name, rule);
                    let _ = guard.save();
                    glib::Propagation::Proceed
                });
            }
            sound_row.append(&*sound_sw);
            opts_card.append(&sound_row);
            opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

            // Previsualizaciones por app
            let prev_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .css_classes(["wifi-pref-row"])
                .build();
            prev_row.append(
                &Label::builder()
                    .label("Previsualizar")
                    .css_classes(["wifi-row-title"])
                    .xalign(0.0)
                    .hexpand(true)
                    .valign(Align::Center)
                    .build(),
            );
            let prev_lbl = Label::builder()
                .label(app_preview_label(current_rule.preview_override))
                .css_classes(["card-section-sub"])
                .valign(Align::Center)
                .build();
            let prev_menu = build_inline_popover_menu(&prev_lbl, &APP_PREVIEW_OPTIONS, {
                let c = cfg_c.clone();
                let app_name = app.name.clone();
                let lbl = prev_lbl.clone();
                move |label, ovr| {
                    let mut guard = c.borrow_mut();
                    let mut rule = guard.rule_for_app(&app_name);
                    rule.preview_override = ovr;
                    guard.set_rule_for_app(&app_name, rule);
                    let _ = guard.save();
                    lbl.set_label(label);
                }
            });
            prev_row.append(&prev_menu);
            opts_card.append(&prev_row);
            opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

            // Contadores en el ícono de la app (por implementar)
            opts_card.append(&make_pending_switch_row("Contadores en el ícono de la app"));
            opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

            // Notificaciones urgentes / críticas (por implementar)
            opts_card.append(&make_pending_switch_row("Notificaciones urgentes"));

            let opts_card_c = opts_card.clone();
            {
                let c = cfg_c.clone();
                let app_name = app.name.clone();
                allow_sw.connect_state_set(move |_, active| {
                    let mut guard = c.borrow_mut();
                    let mut rule: AppNotificationRule = guard.rule_for_app(&app_name);
                    rule.enabled = active;
                    guard.set_rule_for_app(&app_name, rule);
                    let _ = guard.save();
                    opts_card_c.set_sensitive(active);
                    glib::Propagation::Proceed
                });
            }

            pane_c.append(&wrap_squircle_card(&opts_card, WIFI_CARD_SQUIRCLE_RADIUS));
        })
    };

    (scrolled, rebuild)
}

fn load_sorted_unique_apps() -> Vec<DesktopAppEntry> {
    let mut apps = index_applications();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

fn make_app_icon_image(icon_name: &str, size: i32) -> Image {
    let img = if !icon_name.is_empty() {
        if let Some(path) = resolve_icon_file_path(icon_name) {
            Image::from_file(path)
        } else {
            Image::from_icon_name(icon_name)
        }
    } else {
        Image::from_icon_name("application-x-executable-symbolic")
    };
    img.set_pixel_size(size);
    img.set_valign(Align::Center);
    img
}

fn make_indented_hairline() -> GtkBox {
    GtkBox::builder()
        .css_classes(["wifi-hairline-sep"])
        .margin_start(18)
        .build()
}

fn make_indented_pending_switch_row(title: &str, visual_on: bool) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .margin_start(18)
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    row.append(&make_pending_badge());
    let sw = Switch::builder()
        .active(visual_on)
        .sensitive(false)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    row.append(&*sw);
    row
}

fn make_pending_switch_row(title: &str) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    row.append(&make_pending_badge());
    let sw = Switch::builder()
        .active(false)
        .sensitive(false)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    row.append(&*sw);
    row
}

fn build_inline_popover_menu<T: Copy + 'static>(
    value_label: &Label,
    options: &[(&'static str, T)],
    on_select: impl Fn(&'static str, T) + 'static,
) -> MenuButton {
    let btn_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    btn_box.append(value_label);
    btn_box.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let menu_btn = MenuButton::builder()
        .child(&btn_box)
        .has_frame(false)
        .css_classes(["macos-inline-selector-btn"])
        .valign(Align::Center)
        .build();

    let popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .css_classes(["macos-popover"])
        .build();
    let pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["wifi-popover-box"])
        .build();

    let cb = Rc::new(on_select);
    for &(label, val) in options {
        let (item_wrap, item_btn) = make_popover_action_row(label, false, None);
        let cb_c = cb.clone();
        let pop_c = popover.clone();
        item_btn.connect_clicked(move |_| {
            cb_c(label, val);
            pop_c.popdown();
        });
        pop_box.append(&item_wrap);
    }
    popover.set_child(Some(&pop_box));
    menu_btn.set_popover(Some(&popover));
    menu_btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_subpanel_ids_and_labels_are_consistent() {
        assert!(is_notifications_subpanel("notifications"));
        assert!(is_notifications_subpanel("notifications-app"));
        assert!(!is_notifications_subpanel("sound"));
        assert_eq!(preview_label(NotificationPreviewMode::Always), "Siempre");
        assert_eq!(
            preview_label(NotificationPreviewMode::WhenUnlocked),
            "Si desbloqueaste"
        );
        assert_eq!(preview_label(NotificationPreviewMode::Never), "Nunca");
        assert_eq!(style_label(NotificationAlertStyle::None), "Ninguno");
        assert_eq!(style_label(NotificationAlertStyle::Banners), "Tiras");
        assert_eq!(style_label(NotificationAlertStyle::Alerts), "Alertas");
    }
}
