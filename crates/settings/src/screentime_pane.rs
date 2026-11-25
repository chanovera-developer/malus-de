use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, Entry, Image, Label, MenuButton, Orientation,
    PolicyType, Popover, Scale, ScrolledWindow, SearchEntry, Stack, Window,
};
use malus_de_common::apps::{index_applications, resolve_icon_file_path, DesktopAppEntry};
use malus_de_common::config::ScreenTimeConfig;
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_button,
    make_squircle_colored_badge, wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const SCREENTIME_PANEL_ID: &str = "screentime";
pub const SCREENTIME_APPS_SUBPANEL_ID: &str = "screentime-apps";
pub const SCREENTIME_ALWAYS_SUBPANEL_ID: &str = "screentime-always";
pub const SCREENTIME_SCHEDULE_SUBPANEL_ID: &str = "screentime-schedule";
pub const SCREENTIME_LIMITS_SUBPANEL_ID: &str = "screentime-limits";
pub const SCREENTIME_CONTENT_SUBPANEL_ID: &str = "screentime-content";
pub const SCREENTIME_SAFETY_SUBPANEL_ID: &str = "screentime-safety";
pub const SCREENTIME_PASSCODE_SUBPANEL_ID: &str = "screentime-passcode";
pub const SCREENTIME_PANEL_TITLE: &str = "Tiempo en pantalla";

const INDIGO_TOP: (f64, f64, f64) = (0.37, 0.36, 0.90);
const INDIGO_BOT: (f64, f64, f64) = (0.29, 0.27, 0.78);
const BLUE_TOP: (f64, f64, f64) = (0.14, 0.54, 1.00);
const BLUE_BOT: (f64, f64, f64) = (0.04, 0.42, 0.90);
const GREEN_TOP: (f64, f64, f64) = (0.22, 0.80, 0.36);
const GREEN_BOT: (f64, f64, f64) = (0.14, 0.65, 0.26);
const ORANGE_TOP: (f64, f64, f64) = (1.00, 0.62, 0.08);
const ORANGE_BOT: (f64, f64, f64) = (0.90, 0.45, 0.02);
const RED_TOP: (f64, f64, f64) = (1.00, 0.31, 0.27);
const RED_BOT: (f64, f64, f64) = (0.89, 0.16, 0.14);
const PINK_TOP: (f64, f64, f64) = (0.98, 0.22, 0.42);
const PINK_BOT: (f64, f64, f64) = (0.86, 0.12, 0.30);
const GRAY_TOP: (f64, f64, f64) = (0.56, 0.57, 0.60);
const GRAY_BOT: (f64, f64, f64) = (0.43, 0.44, 0.48);

pub fn is_screentime_subpanel(id: &str) -> bool {
    matches!(
        id,
        SCREENTIME_PANEL_ID
            | SCREENTIME_APPS_SUBPANEL_ID
            | SCREENTIME_ALWAYS_SUBPANEL_ID
            | SCREENTIME_SCHEDULE_SUBPANEL_ID
            | SCREENTIME_LIMITS_SUBPANEL_ID
            | SCREENTIME_CONTENT_SUBPANEL_ID
            | SCREENTIME_SAFETY_SUBPANEL_ID
            | SCREENTIME_PASSCODE_SUBPANEL_ID
    )
}

pub fn screentime_subpanel_title(id: &str) -> Option<&'static str> {
    match id {
        SCREENTIME_PANEL_ID => Some(SCREENTIME_PANEL_TITLE),
        SCREENTIME_APPS_SUBPANEL_ID => Some("Apps y sitios"),
        SCREENTIME_ALWAYS_SUBPANEL_ID => Some("Permitir siempre"),
        SCREENTIME_SCHEDULE_SUBPANEL_ID => Some("Horario de Tiempo en pantalla"),
        SCREENTIME_LIMITS_SUBPANEL_ID => Some("Límites de tiempo"),
        SCREENTIME_CONTENT_SUBPANEL_ID => Some("Contenido y privacidad"),
        SCREENTIME_SAFETY_SUBPANEL_ID => Some("Seguridad en la comunicación"),
        SCREENTIME_PASSCODE_SUBPANEL_ID => Some("Código y administración"),
        _ => None,
    }
}

/// Cargador bajo demanda para «Tiempo en pantalla» y todas sus subvistas.
pub fn create_screentime_pane_loader(
    window: &gtk4::ApplicationWindow,
    stack: &Stack,
    navigate: Rc<dyn Fn(&str)>,
) -> Rc<dyn Fn(&str)> {
    let win_c = window.clone();
    let stack_c = stack.clone();
    let cfg = Rc::new(RefCell::new({
        let mut c = ScreenTimeConfig::load();
        c.sync_today_from_uptime();
        let _ = c.save();
        c
    }));

    let refresh_main: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_apps: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_always: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_schedule: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_limits: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_passcode: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    Rc::new(move |target: &str| {
        {
            let mut c = ScreenTimeConfig::load();
            c.sync_today_from_uptime();
            *cfg.borrow_mut() = c;
        }
        match target {
            SCREENTIME_PANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_PANEL_ID).is_none() {
                    let (pane, rf) = build_screentime_main_pane(&cfg, &navigate);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_PANEL_ID));
                    *refresh_main.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_main.borrow().as_ref() {
                    rf();
                }
            }
            SCREENTIME_APPS_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_APPS_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_apps_and_sites_pane(&cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_APPS_SUBPANEL_ID));
                    *refresh_apps.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_apps.borrow().as_ref() {
                    rf();
                }
            }
            SCREENTIME_ALWAYS_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_ALWAYS_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_always_allowed_pane(&cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_ALWAYS_SUBPANEL_ID));
                    *refresh_always.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_always.borrow().as_ref() {
                    rf();
                }
            }
            SCREENTIME_SCHEDULE_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_SCHEDULE_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_schedule_pane(&cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_SCHEDULE_SUBPANEL_ID));
                    *refresh_schedule.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_schedule.borrow().as_ref() {
                    rf();
                }
            }
            SCREENTIME_LIMITS_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_LIMITS_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_limits_pane(&cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_LIMITS_SUBPANEL_ID));
                    *refresh_limits.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_limits.borrow().as_ref() {
                    rf();
                }
            }
            SCREENTIME_CONTENT_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_CONTENT_SUBPANEL_ID).is_none() {
                    let pane = build_content_privacy_pane(&cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_CONTENT_SUBPANEL_ID));
                }
            }
            SCREENTIME_SAFETY_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_SAFETY_SUBPANEL_ID).is_none() {
                    let pane = build_comm_safety_pane(&win_c, &cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_SAFETY_SUBPANEL_ID));
                }
            }
            SCREENTIME_PASSCODE_SUBPANEL_ID => {
                if stack_c.child_by_name(SCREENTIME_PASSCODE_SUBPANEL_ID).is_none() {
                    let (pane, rf) = build_passcode_pane(&win_c, &cfg);
                    crate::configure_right_pane_widget(pane.upcast_ref());
                    stack_c.add_named(&pane, Some(SCREENTIME_PASSCODE_SUBPANEL_ID));
                    *refresh_passcode.borrow_mut() = Some(rf);
                }
                if let Some(rf) = refresh_passcode.borrow().as_ref() {
                    rf();
                }
            }
            _ => {}
        }
    })
}

fn make_scrolled_pane() -> (ScrolledWindow, GtkBox) {
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
    (scrolled, pane)
}

fn make_banner_card(
    icon_name: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    subtitle: &str,
) -> GtkBox {
    let card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    card.append(&make_squircle_colored_badge(
        34,
        icon_name,
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    col.append(
        &Label::builder()
            .label(subtitle)
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    card.append(&col);
    card
}

fn make_nav_row(
    icon_name: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    subtitle: Option<&str>,
    pending: bool,
    on_click: Option<Rc<dyn Fn()>>,
) -> (Button, Option<Label>) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    row.append(&make_squircle_colored_badge(
        28,
        icon_name,
        15,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
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
    let sub_lbl = subtitle.map(|s| {
        let l = Label::builder()
            .label(s)
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        col.append(&l);
        l
    });
    row.append(&col);
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
    if let Some(cb) = on_click {
        btn.connect_clicked(move |_| cb());
    } else {
        btn.set_sensitive(false);
    }
    (btn, sub_lbl)
}

fn make_help_footer(tooltip: &str) -> GtkBox {
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (help_wrap, _) = make_help_circle_button(tooltip);
    footer.append(&help_wrap);
    footer
}

fn build_screentime_main_pane(
    cfg: &Rc<RefCell<ScreenTimeConfig>>,
    navigate: &Rc<dyn Fn(&str)>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    // 1. Tarjeta superior (Banner)
    let banner = make_banner_card(
        "preferences-system-time-symbolic",
        INDIGO_TOP,
        INDIGO_BOT,
        SCREENTIME_PANEL_TITLE,
        "Obtén información sobre el tiempo que pasas ante la pantalla y establece los límites que decidas. Los adultos podrán, además, establecer controles parentales en los dispositivos de menores.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Tarjeta «Promedio diario» con gráfica semanal real (Cairo)
    let chart_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .css_classes(["wifi-card-compact"])
        .build();
    let chart_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    chart_hdr.append(
        &Label::builder()
            .label("Promedio diario")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    chart_hdr.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(13)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    chart_card.append(&chart_hdr);

    let avg_lbl = Label::builder()
        .label(&ScreenTimeConfig::format_minutes_es(
            cfg.borrow().average_daily_minutes(),
        ))
        .xalign(0.0)
        .css_classes(["card-section-title"])
        .margin_bottom(4)
        .build();
    chart_card.append(&avg_lbl);

    let chart_area = DrawingArea::builder()
        .content_height(132)
        .hexpand(true)
        .build();
    {
        let cfg_draw = cfg.clone();
        chart_area.set_draw_func(move |_, cr, w, h| {
            draw_weekly_screentime_chart(cr, w as f64, h as f64, &cfg_draw.borrow());
        });
    }
    crate::register_accent_draw_area(&chart_area);
    chart_card.append(&chart_area);
    pane.append(&wrap_squircle_card(&chart_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 3. Grupo 1: Apps y sitios, Contactos permitidos, Permitir siempre
    let group1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (apps_btn, apps_sub) = make_nav_row(
        "view-app-grid-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Apps y sitios",
        Some("18+ y Acceso completo a Internet"),
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_APPS_SUBPANEL_ID))
        }),
    );
    group1.append(&apps_btn);
    group1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (contacts_btn, _) = make_nav_row(
        "system-users-symbolic",
        GREEN_TOP,
        GREEN_BOT,
        "Contactos permitidos",
        Some("Todos"),
        true,
        None,
    );
    group1.append(&contacts_btn);
    group1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (always_btn, always_sub) = make_nav_row(
        "emblem-ok-symbolic",
        GREEN_TOP,
        GREEN_BOT,
        "Permitir siempre",
        Some("0 apps"),
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_ALWAYS_SUBPANEL_ID))
        }),
    );
    group1.append(&always_btn);
    pane.append(&wrap_squircle_card(&group1, WIFI_CARD_SQUIRCLE_RADIUS));

    // 4. Grupo 2: Horario de Tiempo en pantalla, Límites de tiempo
    let group2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (sched_btn, sched_sub) = make_nav_row(
        "x-office-calendar-symbolic",
        INDIGO_TOP,
        INDIGO_BOT,
        "Horario de Tiempo en pantalla",
        Some("Tiempo disponible para usar el dispositivo"),
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_SCHEDULE_SUBPANEL_ID))
        }),
    );
    group2.append(&sched_btn);
    group2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (limits_btn, limits_sub) = make_nav_row(
        "preferences-system-time-symbolic",
        ORANGE_TOP,
        ORANGE_BOT,
        "Límites de tiempo",
        Some("Tiempo permitido para las categorías de contenido"),
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_LIMITS_SUBPANEL_ID))
        }),
    );
    group2.append(&limits_btn);
    pane.append(&wrap_squircle_card(&group2, WIFI_CARD_SQUIRCLE_RADIUS));

    // 5. Grupo 3: Contenido y privacidad, Seguridad en la comunicación
    let group3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (content_btn, _) = make_nav_row(
        "action-unavailable-symbolic",
        RED_TOP,
        RED_BOT,
        "Contenido y privacidad",
        Some("Administrar contenido, funciones y privacidad"),
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_CONTENT_SUBPANEL_ID))
        }),
    );
    group3.append(&content_btn);
    group3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (safety_btn, _) = make_nav_row(
        "user-available-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Seguridad en la comunicación",
        Some("Protege contra contenido delicado."),
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_SAFETY_SUBPANEL_ID))
        }),
    );
    group3.append(&safety_btn);
    pane.append(&wrap_squircle_card(&group3, WIFI_CARD_SQUIRCLE_RADIUS));

    // 6. Grupo 4: Código y administración
    let group4 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (pass_btn, pass_sub) = make_nav_row(
        "changes-prevent-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Código y administración",
        None,
        false,
        Some({
            let nav = navigate.clone();
            Rc::new(move || nav(SCREENTIME_PASSCODE_SUBPANEL_ID))
        }),
    );
    let _ = pass_sub;
    group4.append(&pass_btn);
    pane.append(&wrap_squircle_card(&group4, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(&make_help_footer("Ayuda sobre Tiempo en pantalla"));

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let avg_c = avg_lbl.clone();
        let chart_c = chart_area.clone();
        let apps_sub_c = apps_sub.clone();
        let always_sub_c = always_sub.clone();
        let sched_sub_c = sched_sub.clone();
        let limits_sub_c = limits_sub.clone();
        Rc::new(move || {
            let conf = c.borrow();
            avg_c.set_label(&ScreenTimeConfig::format_minutes_es(
                conf.average_daily_minutes(),
            ));
            chart_c.queue_draw();
            if let Some(ref l) = apps_sub_c {
                if conf.blocked_apps.is_empty() {
                    l.set_label("18+ y Acceso completo a Internet");
                } else {
                    l.set_label(&format!(
                        "{} app(s) bloqueada(s)",
                        conf.blocked_apps.len()
                    ));
                }
            }
            if let Some(ref l) = always_sub_c {
                l.set_label(&format!("{} apps", conf.always_allowed_apps.len()));
            }
            if let Some(ref l) = sched_sub_c {
                if conf.schedule_enabled {
                    l.set_label(&format!(
                        "{:02}:{:02} – {:02}:{:02}",
                        conf.schedule_start_hour,
                        conf.schedule_start_minute,
                        conf.schedule_end_hour,
                        conf.schedule_end_minute
                    ));
                } else {
                    l.set_label("Tiempo disponible para usar el dispositivo");
                }
            }
            if let Some(ref l) = limits_sub_c {
                if conf.all_limit_hours > 0 {
                    l.set_label(&format!("Todo · {}", conf.limit_summary_label()));
                } else {
                    l.set_label("Tiempo permitido para las categorías de contenido");
                }
            }
        })
    };

    refresh();
    (scrolled, refresh)
}

fn draw_weekly_screentime_chart(
    cr: &gtk4::cairo::Context,
    width: f64,
    height: f64,
    cfg: &ScreenTimeConfig,
) {
    if width < 60.0 || height < 40.0 {
        return;
    }
    let is_dark = libadwaita::StyleManager::default().is_dark();
    let (ar, ag, ab) = crate::accent_color();
    let bars = cfg.last_7_days_bars();
    let max_mins = bars
        .iter()
        .map(|(_, m)| *m)
        .max()
        .unwrap_or(0)
        .max(240);
    let scale_max = ((max_mins as f64 / 60.0).ceil().max(4.0)) * 60.0;
    let avg_mins = cfg.average_daily_minutes() as f64;

    let pad_left = 8.0;
    let pad_right = 46.0;
    let pad_top = 12.0;
    let pad_bottom = 22.0;
    let plot_w = (width - pad_left - pad_right).max(20.0);
    let plot_h = (height - pad_top - pad_bottom).max(20.0);

    let line_rgba = if is_dark {
        (1.0, 1.0, 1.0, 0.10)
    } else {
        (0.0, 0.0, 0.0, 0.08)
    };
    let text_rgba = if is_dark {
        (1.0, 1.0, 1.0, 0.48)
    } else {
        (0.0, 0.0, 0.0, 0.45)
    };

    // Líneas guía horizontales (tope, mitad, 0 s)
    let top_h = (scale_max / 60.0).round() as u32;
    let mid_h = top_h / 2;
    for (frac, label) in [
        (0.0, format!("{top_h} h")),
        (0.5, format!("{mid_h} h")),
        (1.0, "0 s".to_string()),
    ] {
        let y = pad_top + plot_h * frac;
        cr.set_source_rgba(line_rgba.0, line_rgba.1, line_rgba.2, line_rgba.3);
        cr.set_line_width(1.0);
        cr.move_to(pad_left, y);
        cr.line_to(pad_left + plot_w, y);
        let _ = cr.stroke();

        cr.set_source_rgba(text_rgba.0, text_rgba.1, text_rgba.2, text_rgba.3);
        cr.set_font_size(10.5);
        cr.move_to(pad_left + plot_w + 8.0, y + 3.5);
        let _ = cr.show_text(&label);
    }

    // Línea del promedio diario ("prom.")
    if avg_mins > 0.0 {
        let avg_y = pad_top + plot_h * (1.0 - (avg_mins / scale_max).clamp(0.0, 1.0));
        cr.set_source_rgba(line_rgba.0, line_rgba.1, line_rgba.2, line_rgba.3 * 1.6);
        cr.set_line_width(1.0);
        cr.move_to(pad_left, avg_y);
        cr.line_to(pad_left + plot_w, avg_y);
        let _ = cr.stroke();

        cr.set_source_rgba(text_rgba.0, text_rgba.1, text_rgba.2, text_rgba.3);
        cr.set_font_size(10.0);
        cr.move_to(pad_left + plot_w + 8.0, avg_y + 3.0);
        let _ = cr.show_text("prom.");
    }

    // Barras semanales + etiquetas de día
    let count = bars.len().max(1) as f64;
    let slot_w = plot_w / count;
    let bar_w = (slot_w * 0.42).clamp(12.0, 26.0);

    for (idx, (day_letter, mins)) in bars.iter().enumerate() {
        let cx = pad_left + slot_w * (idx as f64 + 0.5);
        if *mins > 0 {
            let bh = (plot_h * (*mins as f64 / scale_max)).clamp(5.0, plot_h);
            let bx = cx - bar_w * 0.5;
            let by = pad_top + plot_h - bh;
            let r = 4.5_f64.min(bar_w * 0.5).min(bh * 0.5);
            cr.new_sub_path();
            cr.arc(bx + bar_w - r, by + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.line_to(bx + bar_w, pad_top + plot_h);
            cr.line_to(bx, pad_top + plot_h);
            cr.arc(bx + r, by + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
            let alpha = if idx == bars.len() - 1 { 0.96 } else { 0.78 };
            cr.set_source_rgba(ar, ag, ab, alpha);
            let _ = cr.fill();
        }

        // Etiqueta de día debajo
        cr.set_source_rgba(
            text_rgba.0,
            text_rgba.1,
            text_rgba.2,
            if idx == bars.len() - 1 { 0.85 } else { text_rgba.3 },
        );
        cr.set_font_size(11.0);
        cr.move_to(cx - 3.5, height - 4.0);
        let _ = cr.show_text(day_letter);
    }
}

fn build_apps_and_sites_pane(cfg: &Rc<RefCell<ScreenTimeConfig>>) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    // 1. Tarjeta superior + Restricciones
    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    top_card.append(&make_banner_card(
        "view-app-grid-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Apps y sitios",
        "Limita el acceso a algunas apps y sitios web. También puedes administrar los permisos de tiendas y navegación web.",
    ));
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let restr_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    restr_row.append(
        &Label::builder()
            .label("Restricciones")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    restr_row.append(&make_pending_badge());
    restr_row.append(
        &Label::builder()
            .label("18+ y Acceso completo a Internet")
            .css_classes(["card-section-sub"])
            .valign(Align::Center)
            .build(),
    );
    top_card.append(&restr_row);
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Selector segmentado "Permitido" | "Bloqueado" (REAL)
    let show_blocked = Rc::new(Cell::new(false));
    let seg_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .homogeneous(true)
        .build();
    let (allowed_wrap, allowed_btn) = make_squircle_button("Permitido", true, None);
    let (blocked_wrap, blocked_btn) = make_squircle_button("Bloqueado", false, None);
    allowed_wrap.set_hexpand(true);
    blocked_wrap.set_hexpand(true);
    seg_box.append(&allowed_wrap);
    seg_box.append(&blocked_wrap);
    pane.append(&seg_box);

    // 3. Barra de búsqueda real + badge explicando que el bloqueo forzoso en lanzadores está pendiente
    let tools_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let search = SearchEntry::builder()
        .placeholder_text("Buscar")
        .hexpand(true)
        .build();
    tools_row.append(&search);
    tools_row.append(&make_pending_badge());
    pane.append(&tools_row);

    // 4. Lista real de apps instaladas con botón Permitir / Bloquear
    let list_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    pane.append(&wrap_squircle_card(&list_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let installed = Rc::new(load_sorted_unique_apps());
    let query_text: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    let rebuild_list: Rc<dyn Fn()> = {
        let card = list_card.clone();
        let cfg_r = cfg.clone();
        let apps_r = installed.clone();
        let blk_r = show_blocked.clone();
        let q_r = query_text.clone();
        Rc::new(move || {
            while let Some(ch) = card.first_child() {
                card.remove(&ch);
            }
            let blocked_only = blk_r.get();
            let q = q_r.borrow().to_lowercase();
            let blocked_set = cfg_r.borrow().blocked_apps.clone();

            let filtered: Vec<&DesktopAppEntry> = apps_r
                .iter()
                .filter(|app| {
                    let is_blk = blocked_set
                        .iter()
                        .any(|b| b.eq_ignore_ascii_case(&app.name));
                    let tab_ok = if blocked_only { is_blk } else { !is_blk };
                    let q_ok = q.is_empty() || app.name.to_lowercase().contains(&q);
                    tab_ok && q_ok
                })
                .collect();

            if filtered.is_empty() {
                let empty = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(["wifi-pref-row"])
                    .build();
                empty.append(
                    &Label::builder()
                        .label(if blocked_only {
                            "No hay apps bloqueadas."
                        } else {
                            "No se encontraron aplicaciones."
                        })
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .build(),
                );
                card.append(&empty);
                return;
            }

            for (idx, app) in filtered.into_iter().enumerate() {
                if idx > 0 {
                    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .css_classes(["wifi-pref-row"])
                    .build();
                row.append(&make_app_icon_image(&app.icon, 24));
                row.append(
                    &Label::builder()
                        .label(&app.name)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .valign(Align::Center)
                        .ellipsize(gtk4::pango::EllipsizeMode::End)
                        .build(),
                );
                let action_lbl = if blocked_only { "Permitir" } else { "Bloquear" };
                let (act_wrap, act_btn) = make_squircle_button(action_lbl, false, None);
                let c_act = cfg_r.clone();
                let name_act = app.name.clone();
                let card_act = card.clone();
                let row_act = row.clone();
                act_btn.connect_clicked(move |_| {
                    {
                        let mut guard = c_act.borrow_mut();
                        if blocked_only {
                            guard
                                .blocked_apps
                                .retain(|x| !x.eq_ignore_ascii_case(&name_act));
                        } else if !guard
                            .blocked_apps
                            .iter()
                            .any(|x| x.eq_ignore_ascii_case(&name_act))
                        {
                            guard.blocked_apps.push(name_act.clone());
                            guard.blocked_apps.sort_by_key(|a| a.to_ascii_lowercase());
                        }
                        let _ = guard.save();
                    }
                    if let Some(prev) = row_act.prev_sibling() {
                        card_act.remove(&prev);
                    }
                    card_act.remove(&row_act);
                });
                row.append(&act_wrap);
                card.append(&row);
            }
        })
    };

    {
        let blk = show_blocked.clone();
        let rf = rebuild_list.clone();
        allowed_btn.connect_clicked(move |_| {
            blk.set(false);
            rf();
        });
    }
    {
        let blk = show_blocked.clone();
        let rf = rebuild_list.clone();
        blocked_btn.connect_clicked(move |_| {
            blk.set(true);
            rf();
        });
    }
    {
        let q = query_text.clone();
        let rf = rebuild_list.clone();
        search.connect_search_changed(move |entry| {
            *q.borrow_mut() = entry.text().to_string();
            rf();
        });
    }

    rebuild_list();
    (scrolled, rebuild_list)
}

fn build_always_allowed_pane(cfg: &Rc<RefCell<ScreenTimeConfig>>) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let banner = make_banner_card(
        "emblem-ok-symbolic",
        GREEN_TOP,
        GREEN_BOT,
        "Permitir siempre",
        "Estas apps y personas se permitirán siempre, incluso durante la hora de dormir y cuando el dispositivo esté en pausa.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    let count_lbl = Label::builder()
        .label("0 apps")
        .css_classes(["wifi-section-title"])
        .xalign(0.0)
        .margin_top(4)
        .build();
    pane.append(&count_lbl);

    let apps_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    pane.append(&wrap_squircle_card(&apps_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let add_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (add_wrap, add_btn) = make_squircle_button("Agregar apps...", false, None);
    let pop = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .position(gtk4::PositionType::Bottom)
        .css_classes(["malus-popover-menu"])
        .build();
    pop.set_parent(&add_btn);
    let pop_unroot = pop.clone();
    add_btn.connect_unrealize(move |_| {
        pop_unroot.unparent();
    });
    add_row.append(&add_wrap);
    pane.append(&add_row);

    // Sección Personas (Por implementar)
    pane.append(
        &Label::builder()
            .label("Personas")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );
    let people_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let people_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    people_row.append(
        &Label::builder()
            .label("Contactos específicos...")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    people_row.append(&make_pending_badge());
    people_row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    people_card.append(&people_row);
    pane.append(&wrap_squircle_card(&people_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&make_help_footer("Ayuda sobre Permitir siempre"));

    let refresh: Rc<dyn Fn()> = {
        let card = apps_card.clone();
        let cnt = count_lbl.clone();
        let c = cfg.clone();
        Rc::new(move || {
            while let Some(ch) = card.first_child() {
                card.remove(&ch);
            }
            let list = c.borrow().always_allowed_apps.clone();
            cnt.set_label(&format!("{} apps", list.len()));
            if list.is_empty() {
                let empty = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(["wifi-pref-row"])
                    .build();
                empty.append(
                    &Label::builder()
                        .label("Ninguna app agregada todavía.")
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .build(),
                );
                card.append(&empty);
                return;
            }
            let installed = load_sorted_unique_apps();
            for (idx, name) in list.iter().enumerate() {
                if idx > 0 {
                    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .css_classes(["wifi-pref-row"])
                    .build();
                let icon_str = installed
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(name))
                    .map(|a| a.icon.as_str())
                    .unwrap_or("application-x-executable");
                row.append(&make_app_icon_image(icon_str, 24));
                row.append(
                    &Label::builder()
                        .label(name)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .valign(Align::Center)
                        .build(),
                );
                let rm = Button::builder()
                    .icon_name("window-close-symbolic")
                    .has_frame(false)
                    .css_classes(["flat"])
                    .tooltip_text("Quitar")
                    .valign(Align::Center)
                    .build();
                let c_rm = c.clone();
                let n_rm = name.clone();
                let card_rm = card.clone();
                let row_rm = row.clone();
                let cnt_rm = cnt.clone();
                rm.connect_clicked(move |_| {
                    let remaining = {
                        let mut guard = c_rm.borrow_mut();
                        guard
                            .always_allowed_apps
                            .retain(|x| !x.eq_ignore_ascii_case(&n_rm));
                        let _ = guard.save();
                        guard.always_allowed_apps.len()
                    };
                    cnt_rm.set_label(&format!("{remaining} apps"));
                    if let Some(prev) = row_rm.prev_sibling() {
                        card_rm.remove(&prev);
                    }
                    card_rm.remove(&row_rm);
                });
                row.append(&rm);
                card.append(&row);
            }
        })
    };

    {
        let pop_open = pop.clone();
        let c_pop = cfg.clone();
        let rf = refresh.clone();
        add_btn.connect_clicked(move |_| {
            let scroll = ScrolledWindow::builder()
                .hscrollbar_policy(PolicyType::Never)
                .vscrollbar_policy(PolicyType::Automatic)
                .max_content_height(280)
                .propagate_natural_height(true)
                .width_request(230)
                .build();
            let box_list = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .css_classes(["wifi-popover-box"])
                .build();
            let current = c_pop.borrow().always_allowed_apps.clone();
            let mut count = 0usize;
            for app in load_sorted_unique_apps() {
                if current.iter().any(|a| a.eq_ignore_ascii_case(&app.name)) {
                    continue;
                }
                let (w, b) = make_popover_action_row(&app.name, false, None);
                let c_sel = c_pop.clone();
                let name_sel = app.name.clone();
                let p_sel = pop_open.clone();
                let rf_sel = rf.clone();
                b.connect_clicked(move |_| {
                    {
                        let mut guard = c_sel.borrow_mut();
                        if !guard
                            .always_allowed_apps
                            .iter()
                            .any(|x| x.eq_ignore_ascii_case(&name_sel))
                        {
                            guard.always_allowed_apps.push(name_sel.clone());
                            guard
                                .always_allowed_apps
                                .sort_by_key(|a| a.to_ascii_lowercase());
                        }
                        let _ = guard.save();
                    }
                    p_sel.popdown();
                    rf_sel();
                });
                box_list.append(&w);
                count += 1;
            }
            if count == 0 {
                box_list.append(
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
            scroll.set_child(Some(&box_list));
            pop_open.set_child(Some(&scroll));
            pop_open.popup();
        });
    }

    refresh();
    (scrolled, refresh)
}

fn build_schedule_pane(cfg: &Rc<RefCell<ScreenTimeConfig>>) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    top_card.append(&make_banner_card(
        "x-office-calendar-symbolic",
        INDIGO_TOP,
        INDIGO_BOT,
        "Horario de Tiempo en pantalla",
        "Establece las categorías de límites de tiempo que están disponibles en diferentes momentos del día para controlar tu acceso a las apps y los sitios web. Los contactos y apps siempre permitidos seguirán accesibles.",
    ));
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let toggle_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    toggle_row.append(
        &Label::builder()
            .label("Horario")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    toggle_row.append(&make_pending_badge());
    let sched_sw = Switch::new(cfg.borrow().schedule_enabled);
    sched_sw.set_valign(Align::Center);
    toggle_row.append(&*sched_sw);
    top_card.append(&toggle_row);
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Se permite todos los días")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    let sched_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let range_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let range_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    range_col.append(
        &Label::builder()
            .label("Todo el día")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let start_lbl = Label::builder()
        .label(&format!(
            "{:02}:{:02}",
            cfg.borrow().schedule_start_hour,
            cfg.borrow().schedule_start_minute
        ))
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let end_lbl = Label::builder()
        .label(&format!(
            "{:02}:{:02}",
            cfg.borrow().schedule_end_hour,
            cfg.borrow().schedule_end_minute
        ))
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let times_inline = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    times_inline.append(&build_hour_picker(&start_lbl, {
        let c = cfg.clone();
        let lbl = start_lbl.clone();
        move |h, m| {
            {
                let mut g = c.borrow_mut();
                g.schedule_start_hour = h;
                g.schedule_start_minute = m;
                let _ = g.save();
            }
            lbl.set_label(&format!("{h:02}:{m:02}"));
        }
    }));
    times_inline.append(
        &Label::builder()
            .label("–")
            .css_classes(["card-section-sub"])
            .build(),
    );
    times_inline.append(&build_hour_picker(&end_lbl, {
        let c = cfg.clone();
        let lbl = end_lbl.clone();
        move |h, m| {
            {
                let mut g = c.borrow_mut();
                g.schedule_end_hour = h;
                g.schedule_end_minute = m;
                let _ = g.save();
            }
            lbl.set_label(&format!("{h:02}:{m:02}"));
        }
    }));
    range_col.append(&times_inline);
    range_row.append(&range_col);
    range_row.append(&make_squircle_colored_badge(
        26,
        "view-app-grid-symbolic",
        14,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        BLUE_TOP,
        BLUE_BOT,
    ));
    sched_card.append(&range_row);

    let actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    actions_row.append(&make_pending_badge());
    let (edit_days_wrap, edit_days_btn) = make_squircle_button("Editar días...", false, None);
    let (add_int_wrap, add_int_btn) = make_squircle_button("Agregar intervalo", false, None);
    edit_days_btn.set_sensitive(false);
    add_int_btn.set_sensitive(false);
    actions_row.append(&edit_days_wrap);
    actions_row.append(&add_int_wrap);
    sched_card.append(&actions_row);
    sched_card.set_sensitive(cfg.borrow().schedule_enabled);
    pane.append(&wrap_squircle_card(&sched_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (new_sched_wrap, new_sched_btn) = make_squircle_button("Agregar nuevo horario...", false, None);
    new_sched_btn.set_sensitive(false);
    let (help_wrap, _) = make_help_circle_button("Ayuda sobre Horario de Tiempo en pantalla");
    footer.append(&new_sched_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    {
        let c = cfg.clone();
        let sc = sched_card.clone();
        sched_sw.connect_state_set(move |_, active| {
            c.borrow_mut().schedule_enabled = active;
            let _ = c.borrow().save();
            sc.set_sensitive(active);
            glib::Propagation::Proceed
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let sw_c = sched_sw.clone();
        let sc = sched_card.clone();
        Rc::new(move || {
            let enabled = c.borrow().schedule_enabled;
            sw_c.set_active(enabled);
            sc.set_sensitive(enabled);
        })
    };
    (scrolled, refresh)
}

fn build_limits_pane(cfg: &Rc<RefCell<ScreenTimeConfig>>) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    // 1. Tarjeta superior + Categorías
    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    top_card.append(&make_banner_card(
        "preferences-system-time-symbolic",
        ORANGE_TOP,
        ORANGE_BOT,
        "Límites de tiempo",
        "Establece por cuánto tiempo puedes usar ciertas categorías de apps y sitios web.",
    ));
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let cat_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    cat_row.append(
        &Label::builder()
            .label("Categorías")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    cat_row.append(&make_pending_badge());
    cat_row.append(&make_squircle_colored_badge(
        26,
        "view-app-grid-symbolic",
        14,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        BLUE_TOP,
        BLUE_BOT,
    ));
    cat_row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    top_card.append(&cat_row);
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Sección "Límites de tiempo" con fila expandible "Todo" + botones -/+ + deslizador (capturas #1 y #4)
    pane.append(
        &Label::builder()
            .label("Límites de tiempo")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    let limit_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let header_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();

    let expanded = Rc::new(Cell::new(cfg.borrow().all_limit_hours > 0));
    let expand_btn = Button::builder()
        .icon_name(if expanded.get() {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        })
        .has_frame(false)
        .css_classes(["flat"])
        .valign(Align::Center)
        .build();
    header_row.append(&expand_btn);

    header_row.append(&make_squircle_colored_badge(
        28,
        "view-app-grid-symbolic",
        15,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        BLUE_TOP,
        BLUE_BOT,
    ));

    let todo_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let todo_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    todo_title_row.append(
        &Label::builder()
            .label("Todo")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    todo_title_row.append(&make_pending_badge());
    todo_col.append(&todo_title_row);

    let limit_lbl = Label::builder()
        .label(&cfg.borrow().limit_summary_label())
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    todo_col.append(&limit_lbl);
    header_row.append(&todo_col);

    let stepper = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .valign(Align::Center)
        .build();
    let (minus_wrap, minus_btn) = make_squircle_button("−", false, None);
    let (plus_wrap, plus_btn) = make_squircle_button("+", false, None);
    stepper.append(&minus_wrap);
    stepper.append(&plus_wrap);
    header_row.append(&stepper);
    limit_card.append(&header_row);

    let slider_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-pref-row"])
        .visible(expanded.get())
        .build();
    let slider = Scale::with_range(Orientation::Horizontal, 0.0, 23.0, 1.0);
    slider.set_hexpand(true);
    slider.set_draw_value(false);
    slider.set_value(f64::from(cfg.borrow().all_limit_hours));
    for h in [0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 23.0] {
        slider.add_mark(h, gtk4::PositionType::Bottom, None);
    }
    slider_box.append(&slider);
    limit_card.append(&slider_box);
    pane.append(&wrap_squircle_card(&limit_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("De forma predeterminada, las apps nuevas usan el límite de tiempo correspondiente a su categoría. Las categorías sin límite de tiempo están disponibles excepto durante las horas restringidas.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(4)
            .margin_end(4)
            .build(),
    );

    // 3. Tarjeta "Ajustar al horario de Tiempo en pantalla"
    let adj_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let adj_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    adj_row.append(
        &Label::builder()
            .label("Ajustar al horario de Tiempo en pantalla")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    adj_row.append(&make_pending_badge());
    let adj_sw = Switch::new(cfg.borrow().adjust_to_schedule);
    adj_sw.set_valign(Align::Center);
    {
        let c = cfg.clone();
        adj_sw.connect_state_set(move |_, active| {
            c.borrow_mut().adjust_to_schedule = active;
            let _ = c.borrow().save();
            glib::Propagation::Proceed
        });
    }
    adj_row.append(&*adj_sw);
    adj_card.append(&adj_row);
    pane.append(&wrap_squircle_card(&adj_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Personaliza los límites de tiempo diarios desde tu horario de Tiempo en pantalla.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(4)
            .build(),
    );
    pane.append(&make_help_footer("Ayuda sobre Límites de tiempo"));

    let updating = Rc::new(Cell::new(false));
    let apply_hours: Rc<dyn Fn(u8)> = {
        let c = cfg.clone();
        let lbl = limit_lbl.clone();
        let sl = slider.clone();
        let upd = updating.clone();
        Rc::new(move |hours: u8| {
            let clamped = hours.min(23);
            {
                let mut g = c.borrow_mut();
                g.all_limit_hours = clamped;
                let _ = g.save();
                lbl.set_label(&g.limit_summary_label());
            }
            upd.set(true);
            sl.set_value(f64::from(clamped));
            upd.set(false);
        })
    };

    {
        let exp = expanded.clone();
        let sbox = slider_box.clone();
        let ebtn = expand_btn.clone();
        expand_btn.connect_clicked(move |_| {
            let next = !exp.get();
            exp.set(next);
            sbox.set_visible(next);
            ebtn.set_icon_name(if next {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
        });
    }
    {
        let c = cfg.clone();
        let app_h = apply_hours.clone();
        minus_btn.connect_clicked(move |_| {
            let cur = c.borrow().all_limit_hours;
            app_h(cur.saturating_sub(1));
        });
    }
    {
        let c = cfg.clone();
        let app_h = apply_hours.clone();
        let exp = expanded.clone();
        let sbox = slider_box.clone();
        let ebtn = expand_btn.clone();
        plus_btn.connect_clicked(move |_| {
            let cur = c.borrow().all_limit_hours;
            app_h((cur + 1).min(23));
            if !exp.get() {
                exp.set(true);
                sbox.set_visible(true);
                ebtn.set_icon_name("pan-down-symbolic");
            }
        });
    }
    {
        let app_h = apply_hours.clone();
        let upd = updating.clone();
        slider.connect_value_changed(move |s| {
            if upd.get() {
                return;
            }
            app_h(s.value().round().clamp(0.0, 23.0) as u8);
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let app_h = apply_hours.clone();
        let sw_c = adj_sw.clone();
        Rc::new(move || {
            let h = c.borrow().all_limit_hours;
            app_h(h);
            sw_c.set_active(c.borrow().adjust_to_schedule);
        })
    };

    (scrolled, refresh)
}

fn build_content_privacy_pane(cfg: &Rc<RefCell<ScreenTimeConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    top_card.append(&make_banner_card(
        "action-unavailable-symbolic",
        RED_TOP,
        RED_BOT,
        "Contenido y privacidad",
        "Administra el contenido, las funciones y la privacidad en tu dispositivo, incluyendo el bloqueo de contenido inapropiado.",
    ));
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let all_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    all_row.append(
        &Label::builder()
            .label("Todas las restricciones")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    all_row.append(&make_pending_badge());
    let all_sw = Switch::new(cfg.borrow().content_restrictions_enabled);
    all_sw.set_valign(Align::Center);
    all_row.append(&*all_sw);
    top_card.append(&all_row);
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let sections_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .sensitive(cfg.borrow().content_restrictions_enabled)
        .build();

    // Restricciones de funciones
    sections_box.append(
        &Label::builder()
            .label("Restricciones de funciones")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );
    let func_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    for (idx, (icon, top, bot, title, value)) in [
        ("audio-input-microphone-symbolic", GRAY_TOP, GRAY_BOT, "Pip", "Permitido"),
        ("video-display-symbolic", BLUE_TOP, BLUE_BOT, "Distancia de la pantalla", "No"),
        ("view-paged-symbolic", GRAY_TOP, GRAY_BOT, "Otras funciones", ""),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            func_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        func_card.append(&make_static_restriction_row(icon, top, bot, title, value));
    }
    sections_box.append(&wrap_squircle_card(&func_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Restricciones de contenido
    sections_box.append(
        &Label::builder()
            .label("Restricciones de contenido")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );
    let content_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    for (idx, (icon, top, bot, title, value)) in [
        ("accessories-dictionary-symbolic", ORANGE_TOP, ORANGE_BOT, "Libros", "Contenido explícito"),
        ("video-x-generic-symbolic", GRAY_TOP, GRAY_BOT, "Películas", "Permitir todo"),
        ("tv-symbolic", GRAY_TOP, GRAY_BOT, "Programas de TV", "Permitir todo"),
        ("audio-x-generic-symbolic", PINK_TOP, PINK_BOT, "Música, Podcasts y Noticias", "Contenido explícito"),
        ("avatar-default-symbolic", PINK_TOP, PINK_BOT, "Perfiles de Música", "Permitido"),
        ("multimedia-player-symbolic", PINK_TOP, PINK_BOT, "Videos musicales", "Permitido"),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            content_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        content_card.append(&make_static_restriction_row(icon, top, bot, title, value));
    }
    sections_box.append(&wrap_squircle_card(&content_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Restricciones de privacidad
    sections_box.append(
        &Label::builder()
            .label("Restricciones de privacidad")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );
    let priv_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let priv_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    priv_row.append(
        &Label::builder()
            .label("Permitir cambiar la configuración")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    priv_row.append(&make_pending_badge());
    priv_card.append(&priv_row);
    sections_box.append(&wrap_squircle_card(&priv_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(&sections_box);
    pane.append(&make_help_footer("Ayuda sobre Contenido y privacidad"));

    {
        let c = cfg.clone();
        let sb = sections_box.clone();
        all_sw.connect_state_set(move |_, active| {
            c.borrow_mut().content_restrictions_enabled = active;
            let _ = c.borrow().save();
            sb.set_sensitive(active);
            glib::Propagation::Proceed
        });
    }

    scrolled
}

fn make_static_restriction_row(
    icon: &str,
    top: (f64, f64, f64),
    bot: (f64, f64, f64),
    title: &str,
    value: &str,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(&make_squircle_colored_badge(
        26,
        icon,
        14,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        top,
        bot,
    ));
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
    if !value.is_empty() {
        row.append(
            &Label::builder()
                .label(value)
                .css_classes(["card-section-sub"])
                .valign(Align::Center)
                .build(),
        );
    }
    row
}

fn build_comm_safety_pane(
    window: &gtk4::ApplicationWindow,
    cfg: &Rc<RefCell<ScreenTimeConfig>>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    top_card.append(&make_banner_card(
        "user-available-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Seguridad en la comunicación",
        "Detecta fotos y videos de desnudos antes de que se envíen o se muestren en el dispositivo del menor.",
    ));
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let sw_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    sw_row.append(
        &Label::builder()
            .label("Seguridad en la comunicación")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    sw_row.append(&make_pending_badge());
    let safety_sw = Switch::new(cfg.borrow().communication_safety_enabled);
    safety_sw.set_valign(Align::Center);
    {
        let c = cfg.clone();
        let win_c = window.clone();
        let sw_clone = safety_sw.clone();
        safety_sw.connect_state_set(move |_, active| {
            if active && !c.borrow().communication_safety_enabled {
                show_comm_safety_modal(&win_c, &c, &sw_clone);
                return glib::Propagation::Stop;
            }
            c.borrow_mut().communication_safety_enabled = active;
            let _ = c.borrow().save();
            glib::Propagation::Proceed
        });
    }
    sw_row.append(&*safety_sw);
    top_card.append(&sw_row);
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Esta configuración ofrece orientación y recursos adecuados según la edad para tomar decisiones conscientes. El análisis local no comparte fotos ni videos.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(4)
            .build(),
    );

    // Recursos para cuidar de los menores
    let res_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let res_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    res_row.append(
        &Label::builder()
            .label("Recursos para cuidar de los menores")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    res_row.append(&make_pending_badge());
    res_card.append(&res_row);
    pane.append(&wrap_squircle_card(&res_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Análisis y mejoras
    pane.append(
        &Label::builder()
            .label("Análisis y mejoras")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );
    let imp_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let imp_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    imp_row.append(
        &Label::builder()
            .label("Mejorar la seguridad en la comunicación")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    imp_row.append(&make_pending_badge());
    let imp_sw = Switch::new(cfg.borrow().improve_communication_safety);
    imp_sw.set_valign(Align::Center);
    {
        let c = cfg.clone();
        imp_sw.connect_state_set(move |_, active| {
            c.borrow_mut().improve_communication_safety = active;
            let _ = c.borrow().save();
            glib::Propagation::Proceed
        });
    }
    imp_row.append(&*imp_sw);
    imp_card.append(&imp_row);
    pane.append(&wrap_squircle_card(&imp_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(&make_help_footer("Ayuda sobre Seguridad en la comunicación"));
    scrolled
}

fn show_comm_safety_modal(
    parent: &gtk4::ApplicationWindow,
    cfg: &Rc<RefCell<ScreenTimeConfig>>,
    sw: &Switch,
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
        .margin_top(24)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build();

    let badge = make_squircle_colored_badge(
        48,
        "user-available-symbolic",
        26,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        BLUE_TOP,
        BLUE_BOT,
    );
    badge.set_halign(Align::Start);
    root.append(&badge);

    root.append(
        &Label::builder()
            .label("Protección de fotos y videos delicados")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    root.append(
        &Label::builder()
            .label("A través de recursos que brindan orientación, esta función ayuda a proteger a los menores evitando que compartan fotos o videos delicados.\n\nEsta función está marcada como «Por implementar» para el análisis multimedia local.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::End)
        .margin_top(10)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Ahora no", false, None);
    let (ok_wrap, ok_btn) = make_squircle_button("Continuar", true, None);
    {
        let dlg = dialog.clone();
        let sw_c = sw.clone();
        cancel_btn.connect_clicked(move |_| {
            sw_c.set_active(false);
            crate::close_attached_modal(&dlg);
        });
    }
    {
        let dlg = dialog.clone();
        let c = cfg.clone();
        let sw_c = sw.clone();
        ok_btn.connect_clicked(move |_| {
            c.borrow_mut().communication_safety_enabled = true;
            let _ = c.borrow().save();
            sw_c.set_active(true);
            crate::close_attached_modal(&dlg);
        });
    }
    actions.append(&cancel_wrap);
    actions.append(&ok_wrap);
    root.append(&actions);
    dialog.set_child(Some(&root));
    crate::present_attached_modal(&dialog, parent);
}

fn build_passcode_pane(
    window: &gtk4::ApplicationWindow,
    cfg: &Rc<RefCell<ScreenTimeConfig>>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    let banner = make_banner_card(
        "changes-prevent-symbolic",
        BLUE_TOP,
        BLUE_BOT,
        "Código y administración",
        "Administra la configuración del código de Tiempo en pantalla o deja de administrar tu dispositivo.",
    );
    pane.append(&wrap_squircle_card(&banner, WIFI_CARD_SQUIRCLE_RADIUS));

    // Compartir entre dispositivos (Por implementar)
    let share_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let share_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    share_row.append(
        &Label::builder()
            .label("Compartir entre dispositivos")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    share_row.append(&make_pending_badge());
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

    pane.append(
        &Label::builder()
            .label("La configuración de Tiempo en pantalla se sincroniza entre tus dispositivos. Si quieres ajustar una configuración específica para este dispositivo, desactiva esta opción.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(4)
            .build(),
    );

    // Actividad en apps y sitios (REAL)
    let act_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let act_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    act_row.append(
        &Label::builder()
            .label("Actividad en apps y sitios")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let act_sw = Switch::new(cfg.borrow().activity_tracking);
    act_sw.set_valign(Align::Center);
    {
        let c = cfg.clone();
        act_sw.connect_state_set(move |_, active| {
            {
                let mut g = c.borrow_mut();
                g.activity_tracking = active;
                if active {
                    g.sync_today_from_uptime();
                }
                let _ = g.save();
            }
            glib::Propagation::Proceed
        });
    }
    act_row.append(&*act_sw);
    act_card.append(&act_row);
    pane.append(&wrap_squircle_card(&act_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Si se desactiva esta opción, se desactivan los reportes en tiempo real, la hora de dormir, la pausa de los dispositivos, los límites de tiempo y los elementos siempre permitidos.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(4)
            .build(),
    );

    // Bloquear configuración de Tiempo en pantalla (REAL: código PIN de 4 dígitos)
    let lock_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let lock_inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    lock_inner.append(
        &Label::builder()
            .label("Bloquear configuración de Tiempo en pantalla")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let pin_status_lbl = Label::builder()
        .label(if cfg.borrow().has_passcode() {
            "Activado"
        } else {
            "Desactivado"
        })
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    lock_inner.append(&pin_status_lbl);
    lock_inner.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let lock_btn = Button::builder()
        .child(&lock_inner)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    {
        let win_c = window.clone();
        let c = cfg.clone();
        let lbl = pin_status_lbl.clone();
        lock_btn.connect_clicked(move |_| {
            show_passcode_modal(&win_c, &c, &lbl);
        });
    }
    lock_card.append(&lock_btn);
    pane.append(&wrap_squircle_card(&lock_card, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Usa un código de 4 dígitos para proteger la configuración de Tiempo en pantalla.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(4)
            .build(),
    );
    pane.append(&make_help_footer("Ayuda sobre Código y administración"));

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let s_sw = share_sw.clone();
        let a_sw = act_sw.clone();
        let p_lbl = pin_status_lbl.clone();
        Rc::new(move || {
            let conf = c.borrow();
            s_sw.set_active(conf.share_across_devices);
            a_sw.set_active(conf.activity_tracking);
            p_lbl.set_label(if conf.has_passcode() {
                "Activado"
            } else {
                "Desactivado"
            });
        })
    };

    (scrolled, refresh)
}

fn show_passcode_modal(
    parent: &gtk4::ApplicationWindow,
    cfg: &Rc<RefCell<ScreenTimeConfig>>,
    status_lbl: &Label,
) {
    let has_pin = cfg.borrow().has_passcode();
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(360)
        .css_classes(["macos-settings-window"])
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(20)
        .margin_bottom(18)
        .margin_start(22)
        .margin_end(22)
        .build();

    root.append(
        &Label::builder()
            .label(if has_pin {
                "Administrar código de Tiempo en pantalla"
            } else {
                "Establecer código de 4 dígitos"
            })
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );

    let pin_entry = Entry::builder()
        .placeholder_text(if has_pin {
            "Ingresa el código actual (4 dígitos)"
        } else {
            "Nuevo código de 4 dígitos"
        })
        .max_length(4)
        .visibility(false)
        .build();
    root.append(&pin_entry);

    let error_lbl = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .visible(false)
        .build();
    root.append(&error_lbl);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(6)
        .build();
    let (cancel_w, cancel_b) = make_squircle_button("Cancelar", false, None);
    let (save_w, save_b) = make_squircle_button(
        if has_pin { "Desactivar código" } else { "Guardar" },
        true,
        None,
    );
    {
        let dlg = dialog.clone();
        cancel_b.connect_clicked(move |_| crate::close_attached_modal(&dlg));
    }
    {
        let dlg = dialog.clone();
        let c = cfg.clone();
        let entry = pin_entry.clone();
        let err = error_lbl.clone();
        let st = status_lbl.clone();
        save_b.connect_clicked(move |_| {
            let val = entry.text().to_string();
            if has_pin {
                if c.borrow().verify_passcode(&val) {
                    c.borrow_mut().clear_passcode();
                    let _ = c.borrow().save();
                    st.set_label("Desactivado");
                    crate::close_attached_modal(&dlg);
                } else {
                    err.set_label("Código incorrecto.");
                    err.set_visible(true);
                }
            } else if c.borrow_mut().set_passcode(&val) {
                let _ = c.borrow().save();
                st.set_label("Activado");
                crate::close_attached_modal(&dlg);
            } else {
                err.set_label("El código debe tener exactamente 4 dígitos numéricos.");
                err.set_visible(true);
            }
        });
    }
    actions.append(&cancel_w);
    actions.append(&save_w);
    root.append(&actions);
    dialog.set_child(Some(&root));
    crate::present_attached_modal(&dialog, parent);
}

fn build_hour_picker<F>(value_lbl: &Label, on_select: F) -> MenuButton
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
    apps
}

fn make_app_icon_image(icon_name: &str, size: i32) -> Image {
    if !icon_name.is_empty() && std::path::Path::new(icon_name).is_file() {
        return Image::builder()
            .file(icon_name)
            .pixel_size(size)
            .valign(Align::Center)
            .build();
    }
    let img = Image::builder()
        .icon_name(if icon_name.is_empty() {
            "application-x-executable"
        } else {
            icon_name
        })
        .pixel_size(size)
        .valign(Align::Center)
        .build();
    if let Some(display) = gtk4::gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        if !icon_name.is_empty() && !theme.has_icon(icon_name) {
            if let Some(path) = resolve_icon_file_path(icon_name) {
                img.set_from_file(Some(path));
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screentime_subpanels_and_titles_resolve() {
        assert!(is_screentime_subpanel(SCREENTIME_PANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_APPS_SUBPANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_ALWAYS_SUBPANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_SCHEDULE_SUBPANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_LIMITS_SUBPANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_CONTENT_SUBPANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_SAFETY_SUBPANEL_ID));
        assert!(is_screentime_subpanel(SCREENTIME_PASSCODE_SUBPANEL_ID));
        assert!(!is_screentime_subpanel("focus"));
        assert_eq!(
            screentime_subpanel_title(SCREENTIME_LIMITS_SUBPANEL_ID),
            Some("Límites de tiempo")
        );
    }
}
