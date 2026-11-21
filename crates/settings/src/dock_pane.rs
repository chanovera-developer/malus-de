use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, ApplicationWindow, Box as GtkBox, CheckButton, Image, Label, MenuButton,
    Orientation, Popover, PolicyType, Scale, ScrolledWindow,
};
use malus_de_common::apps::index_applications;
use malus_de_common::config::{
    get_default_web_browser_desktop, set_default_web_browser_desktop,
    titlebar_double_click_gtk_action, AppSwitcherStyle, DesktopDockConfig, DesktopIconsConfig,
    DockConfig, DockPosition, HotCornerAction, InputConfig,
};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_button, wrap_squircle_card,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

const POSITION_OPTIONS: [&str; 3] = ["Abajo", "Izquierda", "Derecha"];
const MINIMIZE_OPTIONS: [&str; 2] = ["Efecto Aladino", "Efecto de escala"];
const DBLCLICK_OPTIONS: [&str; 3] = ["Zoom", "Minimizar", "No hacer nada"];
const CLICK_WALLPAPER_OPTIONS: [&str; 2] = ["Siempre", "Sólo en el organizador visual"];
const APP_WINDOWS_OPTIONS: [&str; 2] = ["Todas a la vez", "Una a la vez"];
const DIM_WIDGETS_OPTIONS: [&str; 3] = ["Automáticamente", "Siempre", "Nunca"];
const PREFER_TABS_OPTIONS: [&str; 3] = ["Nunca", "Al abrir documentos", "En pantalla completa"];
const SWITCHER_OPTIONS: [&str; 2] = [
    "Tarjetas en cascada (Predeterminado)",
    "Barra clásica de iconos",
];

const HOT_CORNER_LABELS: [(HotCornerAction, &str); 5] = [
    (HotCornerAction::None, "—"),
    (HotCornerAction::MissionControl, "Mission Control"),
    (HotCornerAction::NotificationCenter, "Centro de notificaciones"),
    (HotCornerAction::ShowDesktop, "Escritorio"),
    (HotCornerAction::LockScreen, "Bloquear pantalla"),
];

fn save_dock_and_reload(cfg: &DockConfig) {
    let _ = cfg.save();
    let _ = IpcClient::send_command(IpcMessage::ReloadDockConfig);
}

fn save_desktop_dock_and_reload(cfg: &DesktopDockConfig) {
    let _ = cfg.save();
    let _ = IpcClient::send_command(IpcMessage::ReloadDesktopDockConfig);
}

fn discover_installed_browsers() -> Vec<(String, String, String)> {
    let mut seen = HashSet::new();
    let mut browsers = Vec::new();
    for app in index_applications() {
        let desktop_id = app
            .desktop_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if desktop_id.is_empty() {
            continue;
        }
        let cats = app.categories.to_lowercase();
        let id_low = desktop_id.to_lowercase();
        if cats.contains("webbrowser")
            || id_low.contains("firefox")
            || id_low.contains("chrome")
            || id_low.contains("chromium")
            || id_low.contains("brave")
            || id_low.contains("epiphany")
            || id_low.contains("zen")
            || id_low.contains("edge")
            || id_low.contains("opera")
            || id_low.contains("vivaldi")
        {
            if seen.insert(desktop_id.clone()) {
                browsers.push((app.name, desktop_id, app.icon));
            }
        }
    }
    if browsers.is_empty() {
        browsers.push((
            "Firefox".to_string(),
            "firefox.desktop".to_string(),
            "firefox".to_string(),
        ));
    }
    browsers.sort_by(|a, b| a.0.cmp(&b.0));
    browsers
}

pub fn create_dock_pane(
    parent_window: &ApplicationWindow,
    navigate: Rc<dyn Fn(&str)>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
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

    let dock_cfg = Rc::new(RefCell::new(DockConfig::load_or_create_default()));
    let dd_cfg = Rc::new(RefCell::new(DesktopDockConfig::load()));
    let syncing = Rc::new(Cell::new(false));
    let dock_hooks: Rc<RefCell<Vec<Box<dyn Fn(&DockConfig)>>>> =
        Rc::new(RefCell::new(Vec::new()));
    let dd_hooks: Rc<RefCell<Vec<Box<dyn Fn(&DesktopDockConfig)>>>> =
        Rc::new(RefCell::new(Vec::new()));

    // ========================================================================
    // Sección 1: Dock
    // ========================================================================
    pane.append(
        &Label::builder()
            .label("Dock")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );

    let dock_card1 = GtkBox::builder().orientation(Orientation::Vertical).build();

    // Fila doble de deslizadores: Tamaño | Ampliación
    let sliders_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(22)
        .homogeneous(true)
        .css_classes(["wifi-pref-row"])
        .build();

    // Izquierda: Tamaño
    let size_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    size_col.append(
        &Label::builder()
            .label("Tamaño")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let size_adj = Adjustment::new(
        dock_cfg.borrow().icon_size.clamp(36, 96) as f64,
        36.0,
        96.0,
        2.0,
        4.0,
        0.0,
    );
    let size_slider = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&size_adj)
        .draw_value(false)
        .css_classes(["bat-limit-slider"])
        .hexpand(true)
        .build();
    size_slider.add_mark(36.0, gtk4::PositionType::Bottom, Some("Chico"));
    size_slider.add_mark(96.0, gtk4::PositionType::Bottom, Some("Grande"));
    {
        let dc = dock_cfg.clone();
        let sc = syncing.clone();
        size_slider.connect_value_changed(move |s| {
            if sc.get() {
                return;
            }
            let val = s.value().round().clamp(36.0, 96.0) as u32;
            let mut b = dc.borrow_mut();
            let extra = b.magnification_max_size.saturating_sub(b.icon_size);
            b.icon_size = val;
            if b.magnification_enabled {
                b.magnification_max_size = (val + extra.max(8)).clamp(val, 128);
            }
            save_dock_and_reload(&b);
        });
    }
    {
        let sld = size_slider.clone();
        dock_hooks.borrow_mut().push(Box::new(move |c| {
            sld.set_value(c.icon_size.clamp(36, 96) as f64);
        }));
    }
    size_col.append(&size_slider);

    // Derecha: Ampliación
    let mag_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    mag_col.append(
        &Label::builder()
            .label("Ampliación")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let initial_mag_extra = if dock_cfg.borrow().magnification_enabled {
        dock_cfg
            .borrow()
            .magnification_max_size
            .saturating_sub(dock_cfg.borrow().icon_size)
            .clamp(4, 48) as f64
    } else {
        0.0
    };
    let mag_adj = Adjustment::new(initial_mag_extra, 0.0, 48.0, 2.0, 4.0, 0.0);
    let mag_slider = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&mag_adj)
        .draw_value(false)
        .css_classes(["bat-limit-slider"])
        .hexpand(true)
        .build();
    mag_slider.add_mark(0.0, gtk4::PositionType::Bottom, Some("Desactivada"));
    mag_slider.add_mark(16.0, gtk4::PositionType::Bottom, Some("Chica"));
    mag_slider.add_mark(48.0, gtk4::PositionType::Bottom, Some("Grande"));
    {
        let dc = dock_cfg.clone();
        let sc = syncing.clone();
        mag_slider.connect_value_changed(move |s| {
            if sc.get() {
                return;
            }
            let extra = s.value().round().clamp(0.0, 48.0) as u32;
            let mut b = dc.borrow_mut();
            if extra == 0 {
                b.magnification_enabled = false;
            } else {
                b.magnification_enabled = true;
                b.magnification_max_size = (b.icon_size + extra).clamp(b.icon_size, 140);
            }
            save_dock_and_reload(&b);
        });
    }
    {
        let sld = mag_slider.clone();
        dock_hooks.borrow_mut().push(Box::new(move |c| {
            let v = if c.magnification_enabled {
                c.magnification_max_size
                    .saturating_sub(c.icon_size)
                    .clamp(4, 48) as f64
            } else {
                0.0
            };
            sld.set_value(v);
        }));
    }
    mag_col.append(&mag_slider);

    sliders_row.append(&size_col);
    sliders_row.append(&mag_col);
    dock_card1.append(&sliders_row);
    dock_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Posición del Dock
    let pos_idx = position_to_idx(dock_cfg.borrow().position);
    let (pos_row, set_pos) = build_dropdown_row(
        "Posición del Dock en la pantalla",
        None,
        &POSITION_OPTIONS,
        pos_idx,
        {
            let dc = dock_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = dc.borrow_mut();
                b.position = match idx {
                    1 => DockPosition::Left,
                    2 => DockPosition::Right,
                    _ => DockPosition::Bottom,
                };
                save_dock_and_reload(&b);
            }
        },
    );
    dock_hooks
        .borrow_mut()
        .push(Box::new(move |c| set_pos(position_to_idx(c.position))));
    dock_card1.append(&pos_row);
    dock_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Animación al minimizar
    let min_anim_idx = if dock_cfg.borrow().genie_effect { 0 } else { 1 };
    let (min_anim_row, set_min_anim) = build_dropdown_row(
        "Animación al minimizar ventanas",
        None,
        &MINIMIZE_OPTIONS,
        min_anim_idx,
        {
            let dc = dock_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = dc.borrow_mut();
                b.genie_effect = idx == 0;
                save_dock_and_reload(&b);
            }
        },
    );
    dock_hooks.borrow_mut().push(Box::new(move |c| {
        set_min_anim(if c.genie_effect { 0 } else { 1 });
    }));
    dock_card1.append(&min_anim_row);
    dock_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Doble clic en barra de título
    let (dbl_row, set_dbl) = build_dropdown_row(
        "Acción al hacer doble clic en barra de título de ventana",
        None,
        &DBLCLICK_OPTIONS,
        dock_cfg.borrow().titlebar_double_click_action as usize,
        {
            let dc = dock_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = dc.borrow_mut();
                b.titlebar_double_click_action = idx as u8;
                save_dock_and_reload(&b);
                let gtk_action = titlebar_double_click_gtk_action(b.titlebar_double_click_action);
                let wm = gtk4::gio::Settings::new("org.gnome.desktop.wm.preferences");
                let _ = wm.set_string("action-double-click-titlebar", gtk_action);
                gtk4::gio::Settings::sync();
                if let Some(gtk_settings) = gtk4::Settings::default() {
                    gtk_settings.set_property("gtk-titlebar-double-click", gtk_action);
                }
            }
        },
    );
    dock_hooks.borrow_mut().push(Box::new(move |c| {
        set_dbl(c.titlebar_double_click_action as usize);
    }));
    dock_card1.append(&dbl_row);
    dock_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Selector de aplicaciones (Alt+Tab)
    let sw_idx = match dock_cfg.borrow().app_switcher_style {
        AppSwitcherStyle::Deck => 0,
        AppSwitcherStyle::Classic => 1,
    };
    let (switcher_row, set_switcher) = build_dropdown_row(
        "Selector de aplicaciones (Alt+Tab)",
        None,
        &SWITCHER_OPTIONS,
        sw_idx,
        {
            let dc = dock_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = dc.borrow_mut();
                b.app_switcher_style = if idx == 1 {
                    AppSwitcherStyle::Classic
                } else {
                    AppSwitcherStyle::Deck
                };
                save_dock_and_reload(&b);
            }
        },
    );
    dock_hooks.borrow_mut().push(Box::new(move |c| {
        set_switcher(match c.app_switcher_style {
            AppSwitcherStyle::Deck => 0,
            AppSwitcherStyle::Classic => 1,
        });
    }));
    dock_card1.append(&switcher_row);
    dock_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Minimizar en el icono de la aplicación
    let (min_icon_row, min_icon_sw) = build_switch_row(
        "Minimizar ventanas en el ícono de la aplicación",
        None,
        dock_cfg.borrow().minimize_to_app_icon,
        {
            let dc = dock_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = dc.borrow_mut();
                b.minimize_to_app_icon = active;
                save_dock_and_reload(&b);
            }
        },
    );
    dock_hooks.borrow_mut().push(Box::new(move |c| {
        min_icon_sw.set_active(c.minimize_to_app_icon);
    }));
    dock_card1.append(&min_icon_row);

    pane.append(&wrap_squircle_card(&dock_card1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Dock Card 2: 4 toggles
    let dock_card2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let dock_switches: [(
        &str,
        fn(&DockConfig) -> bool,
        fn(&mut DockConfig, bool),
    ); 4] = [
        (
            "Ocultar y mostrar el Dock automáticamente",
            |c| c.auto_hide,
            |c, v| c.auto_hide = v,
        ),
        (
            "Animar apertura de aplicaciones",
            |c| c.animate_opening,
            |c, v| c.animate_opening = v,
        ),
        (
            "Mostrar indicadores en las aplicaciones abiertas",
            |c| c.show_indicators,
            |c, v| c.show_indicators = v,
        ),
        (
            "Mostrar apps recientes y sugeridas en el Dock",
            |c| c.show_recent_apps,
            |c, v| c.show_recent_apps = v,
        ),
    ];
    for (idx, (title, getter, setter)) in dock_switches.into_iter().enumerate() {
        if idx > 0 {
            dock_card2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let initial = getter(&dock_cfg.borrow());
        let (row, sw) = build_switch_row(title, None, initial, {
            let dc = dock_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = dc.borrow_mut();
                setter(&mut b, active);
                save_dock_and_reload(&b);
            }
        });
        dock_hooks.borrow_mut().push(Box::new(move |c| {
            sw.set_active(getter(c));
        }));
        dock_card2.append(&row);
    }
    pane.append(&wrap_squircle_card(&dock_card2, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // Sección 2: Escritorio y Organizador visual
    // ========================================================================
    pane.append(
        &Label::builder()
            .label("Escritorio y Organizador visual")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let desk_card1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (show_items_row, chk_items_desk, chk_items_sm) = build_dual_checkbox_row(
        "Mostrar elementos",
        "En el escritorio",
        DesktopIconsConfig::load().show_icons,
        "En el organizador visual",
        dd_cfg.borrow().show_items_in_stage_manager,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut desk = DesktopIconsConfig::load();
                desk.show_icons = active;
                let _ = desk.save();
                let mut b = ddc.borrow_mut();
                b.show_items_on_desktop = active;
                save_desktop_dock_and_reload(&b);
            }
        },
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.show_items_in_stage_manager = active;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        chk_items_desk.set_active(DesktopIconsConfig::load().show_icons);
        chk_items_sm.set_active(c.show_items_in_stage_manager);
    }));
    desk_card1.append(&show_items_row);
    desk_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (click_wp_row, set_click_wp) = build_dropdown_row(
        "Hacer clic en el fondo de pantalla para mostrar el escritorio",
        Some("Haz clic en el fondo de pantalla para apartar las ventanas y mostrar los elementos y widgets del escritorio."),
        &CLICK_WALLPAPER_OPTIONS,
        dd_cfg.borrow().click_wallpaper_shows_desktop as usize,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.click_wallpaper_shows_desktop = idx as u8;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        set_click_wp(c.click_wallpaper_shows_desktop as usize);
    }));
    desk_card1.append(&click_wp_row);
    pane.append(&wrap_squircle_card(&desk_card1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Organizador visual Card 2
    let desk_card2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (sm_row, sm_sw) = build_switch_row(
        "Organizador visual",
        Some("El organizador visual acomoda tus ventanas recientes en una sola franja para reducir el desorden y ofrecer un acceso más rápido."),
        dd_cfg.borrow().stage_manager_enabled,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.stage_manager_enabled = active;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks
        .borrow_mut()
        .push(Box::new(move |c| sm_sw.set_active(c.stage_manager_enabled)));
    desk_card2.append(&sm_row);
    desk_card2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (sm_rec_row, sm_rec_sw) = build_switch_row(
        "Mostrar apps recientes en el organizador visual",
        None,
        dd_cfg.borrow().stage_manager_recent_apps,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.stage_manager_recent_apps = active;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        sm_rec_sw.set_active(c.stage_manager_recent_apps);
    }));
    desk_card2.append(&sm_rec_row);
    desk_card2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (sm_win_row, set_sm_win) = build_dropdown_row(
        "Mostrar las ventanas de una app",
        None,
        &APP_WINDOWS_OPTIONS,
        dd_cfg.borrow().stage_manager_app_windows as usize,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.stage_manager_app_windows = idx as u8;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        set_sm_win(c.stage_manager_app_windows as usize);
    }));
    desk_card2.append(&sm_win_row);
    pane.append(&wrap_squircle_card(&desk_card2, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // Sección 3: Widgets & Navegador predeterminado
    // ========================================================================
    pane.append(
        &Label::builder()
            .label("Widgets")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let widgets_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (w_show_row, chk_w_desk, chk_w_sm) = build_dual_checkbox_row(
        "Mostrar widgets",
        "En el escritorio",
        dd_cfg.borrow().widgets_on_desktop,
        "En el organizador visual",
        dd_cfg.borrow().widgets_in_stage_manager,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.widgets_on_desktop = active;
                save_desktop_dock_and_reload(&b);
            }
        },
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.widgets_in_stage_manager = active;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        chk_w_desk.set_active(c.widgets_on_desktop);
        chk_w_sm.set_active(c.widgets_in_stage_manager);
    }));
    widgets_card.append(&w_show_row);
    widgets_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (dim_row, set_dim) = build_dropdown_row(
        "Atenuar widgets en el escritorio",
        None,
        &DIM_WIDGETS_OPTIONS,
        dd_cfg.borrow().dim_widgets_on_desktop as usize,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.dim_widgets_on_desktop = idx as u8;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        set_dim(c.dim_widgets_on_desktop as usize);
    }));
    widgets_card.append(&dim_row);
    pane.append(&wrap_squircle_card(&widgets_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Navegador predeterminado
    let browser_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let browsers = discover_installed_browsers();
    let browser_names: Vec<&str> = browsers.iter().map(|(n, _, _)| n.as_str()).collect();
    let active_browser_id = get_default_web_browser_desktop().unwrap_or_default();
    let initial_browser_idx = browsers
        .iter()
        .position(|(_, id, _)| id.eq_ignore_ascii_case(&active_browser_id))
        .unwrap_or(0);
    let browser_ids: Vec<String> = browsers.iter().map(|(_, id, _)| id.clone()).collect();
    let browser_icons: Vec<String> = browsers.iter().map(|(_, _, ic)| ic.clone()).collect();
    let (browser_row, _set_browser) = build_browser_selector_row(
        "Navegador predeterminado",
        &browser_names,
        &browser_icons,
        initial_browser_idx,
        move |idx| {
            if let Some(desktop_id) = browser_ids.get(idx) {
                let id_c = desktop_id.clone();
                std::thread::spawn(move || {
                    let _ = set_default_web_browser_desktop(&id_c);
                });
            }
        },
    );
    browser_card.append(&browser_row);
    pane.append(&wrap_squircle_card(&browser_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // Sección 4: Ventanas
    // ========================================================================
    pane.append(
        &Label::builder()
            .label("Ventanas")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let win_card1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (tabs_row, set_tabs) = build_dropdown_row(
        "Utilizar pestañas al abrir documentos",
        None,
        &PREFER_TABS_OPTIONS,
        dd_cfg.borrow().prefer_tabs_mode as usize,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |idx| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.prefer_tabs_mode = idx as u8;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        set_tabs(c.prefer_tabs_mode as usize);
    }));
    win_card1.append(&tabs_row);
    win_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (ask_row, ask_sw) = build_switch_row(
        "Preguntar si se guardan los cambios al cerrar los documentos",
        None,
        dd_cfg.borrow().ask_to_keep_changes,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.ask_to_keep_changes = active;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks
        .borrow_mut()
        .push(Box::new(move |c| ask_sw.set_active(c.ask_to_keep_changes)));
    win_card1.append(&ask_row);
    win_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (close_q_row, close_q_sw) = build_switch_row(
        "Cerrar ventanas al salir de una app",
        Some("Si se activa, las ventanas y documentos abiertos no se restaurarán cuando se vuelva a abrir una app."),
        dd_cfg.borrow().close_windows_on_quit,
        {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                b.close_windows_on_quit = active;
                save_desktop_dock_and_reload(&b);
            }
        },
    );
    dd_hooks.borrow_mut().push(Box::new(move |c| {
        close_q_sw.set_active(c.close_windows_on_quit);
    }));
    win_card1.append(&close_q_row);
    pane.append(&wrap_squircle_card(&win_card1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Ventanas Card 2: Mosaico / Snap Layouts
    let win_card2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let tile_switches: [(
        &str,
        fn(&DesktopDockConfig) -> bool,
        fn(&mut DesktopDockConfig, bool),
    ); 4] = [
        (
            "Arrastrar ventanas al borde izquierdo o derecho de la pantalla para disponerlas en mosaico",
            |c| c.drag_to_edges_tile,
            |c, v| c.drag_to_edges_tile = v,
        ),
        (
            "Arrastrar ventanas a la barra de menús para llenar la pantalla",
            |c| c.drag_to_top_maximize,
            |c, v| c.drag_to_top_maximize = v,
        ),
        (
            "Mantener presionada la tecla ⌥ al arrastrar las ventanas para disponerlas en mosaico",
            |c| c.option_drag_tile,
            |c, v| c.option_drag_tile = v,
        ),
        (
            "Usar márgenes para ventanas en mosaico",
            |c| c.tiled_windows_have_margins,
            |c, v| c.tiled_windows_have_margins = v,
        ),
    ];
    for (idx, (title, getter, setter)) in tile_switches.into_iter().enumerate() {
        if idx > 0 {
            win_card2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let initial = getter(&dd_cfg.borrow());
        let (row, sw) = build_switch_row(title, None, initial, {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                setter(&mut b, active);
                save_desktop_dock_and_reload(&b);
            }
        });
        dd_hooks.borrow_mut().push(Box::new(move |c| {
            sw.set_active(getter(c));
        }));
        win_card2.append(&row);
    }
    pane.append(&wrap_squircle_card(&win_card2, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // Sección 5: Mission Control
    // ========================================================================
    let mc_hdr = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(6)
        .build();
    mc_hdr.append(
        &Label::builder()
            .label("Mission Control")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    mc_hdr.append(
        &Label::builder()
            .label("Mission Control muestra una visualización unificada de todas las ventanas abiertas y las miniaturas de las apps que están en pantalla completa.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pane.append(&mc_hdr);

    let mc_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let mc_switches: [(
        &str,
        fn(&DesktopDockConfig) -> bool,
        fn(&mut DesktopDockConfig, bool),
    ); 5] = [
        (
            "Reordenar Spaces automáticamente en función del uso más reciente",
            |c| c.mc_auto_rearrange_spaces,
            |c, v| c.mc_auto_rearrange_spaces = v,
        ),
        (
            "Al cambiar de aplicación, ir a un Space con ventanas abiertas de esa aplicación",
            |c| c.mc_switch_to_space_with_app,
            |c, v| c.mc_switch_to_space_with_app = v,
        ),
        (
            "Agrupar ventanas por aplicación",
            |c| c.mc_group_windows_by_app,
            |c, v| c.mc_group_windows_by_app = v,
        ),
        (
            "Las pantallas tienen Spaces separados",
            |c| c.mc_displays_have_separate_spaces,
            |c, v| c.mc_displays_have_separate_spaces = v,
        ),
        (
            "Arrastrar ventanas a la parte superior de la pantalla para abrir Mission Control",
            |c| c.mc_drag_to_top_for_expose,
            |c, v| c.mc_drag_to_top_for_expose = v,
        ),
    ];
    for (idx, (title, getter, setter)) in mc_switches.into_iter().enumerate() {
        if idx > 0 {
            mc_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let initial = getter(&dd_cfg.borrow());
        let (row, sw) = build_switch_row(title, None, initial, {
            let ddc = dd_cfg.clone();
            let sc = syncing.clone();
            move |active| {
                if sc.get() {
                    return;
                }
                let mut b = ddc.borrow_mut();
                setter(&mut b, active);
                save_desktop_dock_and_reload(&b);
            }
        });
        dd_hooks.borrow_mut().push(Box::new(move |c| {
            sw.set_active(getter(c));
        }));
        mc_card.append(&row);
    }
    pane.append(&wrap_squircle_card(&mc_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ========================================================================
    // Pie inferior: [Atajos...] [Esquinas activas...] [?]
    // ========================================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (shortcuts_wrap, shortcuts_btn) = make_squircle_button("Atajos...", false, None);
    let (hot_corners_wrap, hot_corners_btn) =
        make_squircle_button("Esquinas activas...", false, None);
    let (help_wrap, _) = make_help_circle_button(
        "Configura el tamaño, posición y comportamiento del Dock, la vista de ventanas en mosaico y las esquinas activas.",
    );
    footer.append(&shortcuts_wrap);
    footer.append(&hot_corners_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    {
        let pw = parent_window.clone();
        let nav = navigate;
        shortcuts_btn.connect_clicked(move |_| {
            open_shortcuts_dialog(&pw, &nav);
        });
    }
    {
        let pw = parent_window.clone();
        hot_corners_btn.connect_clicked(move |_| {
            open_hot_corners_dialog(&pw);
        });
    }

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let next_dock = DockConfig::load_or_create_default();
        let next_dd = DesktopDockConfig::load();
        *dock_cfg.borrow_mut() = next_dock.clone();
        *dd_cfg.borrow_mut() = next_dd.clone();
        syncing.set(true);
        for h in dock_hooks.borrow().iter() {
            h(&next_dock);
        }
        for h in dd_hooks.borrow().iter() {
            h(&next_dd);
        }
        syncing.set(false);
    });

    scrolled.set_child(Some(&pane));
    (scrolled, refresh)
}

fn position_to_idx(pos: DockPosition) -> usize {
    match pos {
        DockPosition::Bottom => 0,
        DockPosition::Left => 1,
        DockPosition::Right => 2,
    }
}

fn build_switch_row<F: Fn(bool) + 'static>(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
    on_toggle: F,
) -> (GtkBox, Switch) {
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
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
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
    let sw = Switch::builder()
        .active(initial)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(&col);
    row.append(&*sw);
    (row, sw)
}

fn build_dual_checkbox_row<F1: Fn(bool) + 'static, F2: Fn(bool) + 'static>(
    title: &str,
    opt1: &str,
    init1: bool,
    opt2: &str,
    init2: bool,
    on_toggle1: F1,
    on_toggle2: F2,
) -> (GtkBox, CheckButton, CheckButton) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let checks_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(Align::Center)
        .build();

    let make_chk = |lbl: &str, active: bool| -> CheckButton {
        let chk = CheckButton::builder()
            .active(active)
            .css_classes(["menubar-check"])
            .valign(Align::Center)
            .build();
        let inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();
        inner.append(
            &Label::builder()
                .label(lbl)
                .css_classes(["wifi-row-title"])
                .build(),
        );
        chk.set_child(Some(&inner));
        chk
    };
    let chk1 = make_chk(opt1, init1);
    let chk2 = make_chk(opt2, init2);
    chk1.connect_toggled(move |c| on_toggle1(c.is_active()));
    chk2.connect_toggled(move |c| on_toggle2(c.is_active()));
    checks_box.append(&chk1);
    checks_box.append(&chk2);
    row.append(&checks_box);
    (row, chk1, chk2)
}

fn build_compact_dropdown<F: Fn(usize) + 'static>(
    options: &[&str],
    icons: Option<&[String]>,
    initial_idx: usize,
    on_select: F,
) -> (MenuButton, Rc<dyn Fn(usize)>) {
    let selected = Rc::new(Cell::new(initial_idx.min(options.len().saturating_sub(1))));
    let opts: Vec<String> = options.iter().map(|s| s.to_string()).collect();
    let icon_list: Vec<String> = icons.map(|s| s.to_vec()).unwrap_or_default();
    let cb = Rc::new(on_select);

    let inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let leading_img = if let Some(ic) = icon_list.get(selected.get()) {
        let img = Image::builder().icon_name(ic).pixel_size(14).build();
        inner.append(&img);
        Some(img)
    } else {
        None
    };
    let cur_lbl = Label::builder()
        .label(opts.get(selected.get()).map(String::as_str).unwrap_or(""))
        .css_classes(["card-section-sub"])
        .build();
    inner.append(&cur_lbl);
    inner.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .build(),
    );

    let btn = MenuButton::builder()
        .child(&inner)
        .has_frame(false)
        .always_show_arrow(false)
        .css_classes(["fw-rule-action-btn"])
        .valign(Align::Center)
        .build();
    let pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    btn.set_popover(Some(&pop));

    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let reb_fn: Rc<dyn Fn()> = {
        let pop_c = pop;
        let sel_c = selected.clone();
        let lbl_c = cur_lbl.clone();
        let img_c = leading_img.clone();
        let opts_c = opts.clone();
        let icons_c = icon_list.clone();
        let cb_c = cb;
        let self_ref = rebuild.clone();
        Rc::new(move || {
            let mbox = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();
            let cur = sel_c.get();
            for (idx, opt) in opts_c.iter().enumerate() {
                let (w, b) = make_popover_action_row(opt, true, (idx == cur).then_some("✓"));
                let pc = pop_c.clone();
                let sc = sel_c.clone();
                let lc = lbl_c.clone();
                let ic = img_c.clone();
                let ic_name = icons_c.get(idx).cloned();
                let os = opt.clone();
                let cbc = cb_c.clone();
                let rfc = self_ref.clone();
                b.connect_clicked(move |_| {
                    pc.popdown();
                    sc.set(idx);
                    lc.set_label(&os);
                    if let (Some(ref img), Some(ref name)) = (&ic, &ic_name) {
                        img.set_icon_name(Some(name));
                    }
                    if let Some(f) = rfc.borrow().as_ref() {
                        f();
                    }
                    cbc(idx);
                });
                mbox.append(&w);
            }
            pop_c.set_child(Some(&mbox));
        })
    };
    *rebuild.borrow_mut() = Some(reb_fn.clone());
    reb_fn();

    let set_selected: Rc<dyn Fn(usize)> = {
        let sel_c = selected;
        let lbl_c = cur_lbl;
        let img_c = leading_img;
        let opts_c = opts;
        let icons_c = icon_list;
        let reb_c = reb_fn;
        Rc::new(move |idx: usize| {
            let clamped = idx.min(opts_c.len().saturating_sub(1));
            sel_c.set(clamped);
            if let Some(txt) = opts_c.get(clamped) {
                lbl_c.set_label(txt);
            }
            if let (Some(ref img), Some(name)) = (&img_c, icons_c.get(clamped)) {
                img.set_icon_name(Some(name));
            }
            reb_c();
        })
    };

    (btn, set_selected)
}

fn build_dropdown_row<F: Fn(usize) + 'static>(
    title: &str,
    subtitle: Option<&str>,
    options: &[&str],
    initial_idx: usize,
    on_select: F,
) -> (GtkBox, Rc<dyn Fn(usize)>) {
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
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
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
    let (btn, set_selected) = build_compact_dropdown(options, None, initial_idx, on_select);
    row.append(&col);
    row.append(&btn);
    (row, set_selected)
}

fn build_browser_selector_row<F: Fn(usize) + 'static>(
    title: &str,
    options: &[&str],
    icons: &[String],
    initial_idx: usize,
    on_select: F,
) -> (GtkBox, Rc<dyn Fn(usize)>) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let (btn, set_selected) = build_compact_dropdown(options, Some(icons), initial_idx, on_select);
    row.append(&btn);
    (row, set_selected)
}

fn open_shortcuts_dialog(parent: &ApplicationWindow, navigate: &Rc<dyn Fn(&str)>) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Atajos de Escritorio y Mission Control")
        .default_width(440)
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let entries = [
        ("Mission Control", "Ctrl + ↑"),
        ("Ventanas de aplicación", "Ctrl + ↓"),
        ("Cambiar de espacio", "Ctrl + ← / →"),
        ("Spotlight", "⌘ + Espacio"),
        ("Ordenamiento en mosaico", "⌘ + Y"),
        ("Bloquear pantalla", "Ctrl + ⌘ + Q"),
    ];
    for (idx, (label, keys)) in entries.iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let r = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        r.append(
            &Label::builder()
                .label(*label)
                .css_classes(["wifi-row-title"])
                .hexpand(true)
                .xalign(0.0)
                .build(),
        );
        r.append(
            &Label::builder()
                .label(*keys)
                .css_classes(["card-section-sub"])
                .xalign(1.0)
                .build(),
        );
        card.append(&r);
    }
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let btn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (kb_wrap, kb_btn) = make_squircle_button("Ir a Teclado...", false, None);
    let dlg_nav = dialog.clone();
    let nav_c = navigate.clone();
    kb_btn.connect_clicked(move |_| {
        dlg_nav.close();
        nav_c("keyboard");
    });
    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| dlg_ok.close());
    btn_row.append(&kb_wrap);
    btn_row.append(&ok_wrap);
    vbox.append(&btn_row);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn hot_corner_to_idx(action: HotCornerAction) -> usize {
    HOT_CORNER_LABELS
        .iter()
        .position(|(a, _)| *a == action)
        .unwrap_or(0)
}

fn open_hot_corners_dialog(parent: &ApplicationWindow) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Esquinas activas")
        .default_width(460)
        .build();
    crate::attach_modal_scrim(&dialog, parent);

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    vbox.append(
        &Label::builder()
            .label("Asigna una acción al mover el cursor a cada esquina de la pantalla.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let input_cfg = Rc::new(RefCell::new(InputConfig::load()));
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let corner_opts: Vec<&str> = HOT_CORNER_LABELS.iter().map(|(_, l)| *l).collect();

    let corners: [(
        &str,
        fn(&InputConfig) -> HotCornerAction,
        fn(&mut InputConfig, HotCornerAction),
    ); 4] = [
        (
            "Esquina superior izquierda",
            |c| c.hot_corners.top_left,
            |c, a| c.hot_corners.top_left = a,
        ),
        (
            "Esquina superior derecha",
            |c| c.hot_corners.top_right,
            |c, a| c.hot_corners.top_right = a,
        ),
        (
            "Esquina inferior izquierda",
            |c| c.hot_corners.bottom_left,
            |c, a| c.hot_corners.bottom_left = a,
        ),
        (
            "Esquina inferior derecha",
            |c| c.hot_corners.bottom_right,
            |c, a| c.hot_corners.bottom_right = a,
        ),
    ];

    for (idx, (label, getter, setter)) in corners.into_iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let initial = hot_corner_to_idx(getter(&input_cfg.borrow()));
        let ic = input_cfg.clone();
        let (row, _) = build_dropdown_row(label, None, &corner_opts, initial, move |sel| {
            let action = HOT_CORNER_LABELS
                .get(sel)
                .map(|(a, _)| *a)
                .unwrap_or(HotCornerAction::None);
            let mut b = ic.borrow_mut();
            setter(&mut b, action);
            b.hot_corners.enabled = b.hot_corners.top_left != HotCornerAction::None
                || b.hot_corners.top_right != HotCornerAction::None
                || b.hot_corners.bottom_left != HotCornerAction::None
                || b.hot_corners.bottom_right != HotCornerAction::None;
            let _ = b.save();
            let _ = IpcClient::send_command(IpcMessage::ReloadInputConfig);
        });
        card.append(&row);
    }

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    ok_wrap.set_halign(Align::End);
    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| dlg_ok.close());
    vbox.append(&ok_wrap);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_corner_mapping_roundtrips_all_actions() {
        for (action, _) in HOT_CORNER_LABELS {
            let idx = hot_corner_to_idx(action);
            assert_eq!(HOT_CORNER_LABELS[idx].0, action);
        }
    }
}
