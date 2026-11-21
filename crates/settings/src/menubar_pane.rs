use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, CheckButton, DrawingArea, Image, Label,
    MenuButton, Orientation, PolicyType, Popover, ScrolledWindow,
};
use malus_de_common::config::MenuBarConfig;
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_button,
    make_squircle_colored_badge, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

const RECENT_OPTIONS: [(u32, &str); 7] = [
    (0, "Ninguno"),
    (5, "5"),
    (10, "10"),
    (15, "15"),
    (20, "20"),
    (30, "30"),
    (50, "50"),
];

const AUTO_HIDE_OPTIONS: [&str; 4] = [
    "Sólo en pantalla completa",
    "Siempre",
    "Sólo en el escritorio",
    "Nunca",
];

const VISIBILITY_MODE_OPTIONS: [&str; 2] =
    ["Mostrar siempre", "Mostrar cuando se haya activado"];

const FOCUS_MODE_OPTIONS: [&str; 3] = [
    "Mostrar siempre",
    "Mostrar cuando se haya activado",
    "No mostrar",
];

const USER_SWITCH_MODE_OPTIONS: [&str; 3] = ["Nombre completo", "Nombre de la cuenta", "Ícono"];

#[derive(Clone)]
struct MenuBarControlSpec {
    label: &'static str,
    icon_name: &'static str,
    top_rgb: (f64, f64, f64),
    bot_rgb: (f64, f64, f64),
    has_checkbox: bool,
    get_checked: fn(&MenuBarConfig) -> bool,
    set_checked: fn(&mut MenuBarConfig, bool),
    extra: ControlRowExtra,
}

#[derive(Clone, Copy)]
enum ControlRowExtra {
    None,
    ClockOptions,
    BatteryOptions,
    ModeSelector {
        options: &'static [&'static str],
        get_mode: fn(&MenuBarConfig) -> usize,
        set_mode: fn(&mut MenuBarConfig, usize),
        dim_when_unchecked: bool,
    },
}

fn control_specs() -> Vec<MenuBarControlSpec> {
    vec![
        MenuBarControlSpec {
            label: "Reloj",
            icon_name: "preferences-system-time-symbolic",
            top_rgb: (0.64, 0.65, 0.68),
            bot_rgb: (0.48, 0.49, 0.52),
            has_checkbox: false,
            get_checked: |_| true,
            set_checked: |_, _| {},
            extra: ControlRowExtra::ClockOptions,
        },
        MenuBarControlSpec {
            label: "Spotlight",
            icon_name: "system-search-symbolic",
            top_rgb: (0.20, 0.56, 0.99),
            bot_rgb: (0.04, 0.42, 0.92),
            has_checkbox: true,
            get_checked: |c| c.spotlight,
            set_checked: |c, v| c.spotlight = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Wi-Fi",
            icon_name: "network-wireless-symbolic",
            top_rgb: (0.16, 0.56, 0.99),
            bot_rgb: (0.03, 0.44, 0.94),
            has_checkbox: true,
            get_checked: |c| c.wifi,
            set_checked: |c, v| c.wifi = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Bluetooth",
            icon_name: "bluetooth-active-symbolic",
            top_rgb: (0.15, 0.52, 0.98),
            bot_rgb: (0.04, 0.39, 0.88),
            has_checkbox: true,
            get_checked: |c| c.bluetooth,
            set_checked: |c, v| c.bluetooth = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Batería",
            icon_name: "battery-level-100-symbolic",
            top_rgb: (0.31, 0.82, 0.41),
            bot_rgb: (0.18, 0.69, 0.29),
            has_checkbox: true,
            get_checked: |c| c.battery,
            set_checked: |c, v| c.battery = v,
            extra: ControlRowExtra::BatteryOptions,
        },
        MenuBarControlSpec {
            label: "AirDrop",
            icon_name: "emblem-shared-symbolic",
            top_rgb: (0.35, 0.72, 0.99),
            bot_rgb: (0.17, 0.56, 0.95),
            has_checkbox: true,
            get_checked: |c| c.airdrop,
            set_checked: |c, v| c.airdrop = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Enfoque",
            icon_name: "weather-clear-night-symbolic",
            top_rgb: (0.43, 0.41, 0.94),
            bot_rgb: (0.32, 0.30, 0.82),
            has_checkbox: true,
            get_checked: |c| c.focus,
            set_checked: |c, v| c.focus = v,
            extra: ControlRowExtra::ModeSelector {
                options: &FOCUS_MODE_OPTIONS,
                get_mode: |c| c.focus_mode as usize,
                set_mode: |c, v| c.focus_mode = v as u8,
                dim_when_unchecked: false,
            },
        },
        MenuBarControlSpec {
            label: "Duplicar pantalla",
            icon_name: "video-display-symbolic",
            top_rgb: (0.28, 0.74, 0.98),
            bot_rgb: (0.11, 0.55, 0.90),
            has_checkbox: true,
            get_checked: |c| c.screen_mirroring,
            set_checked: |c, v| c.screen_mirroring = v,
            extra: ControlRowExtra::ModeSelector {
                options: &VISIBILITY_MODE_OPTIONS,
                get_mode: |c| c.screen_mirroring_mode as usize,
                set_mode: |c, v| c.screen_mirroring_mode = v as u8,
                dim_when_unchecked: false,
            },
        },
        MenuBarControlSpec {
            label: "Pantalla",
            icon_name: "display-brightness-symbolic",
            top_rgb: (0.20, 0.56, 0.99),
            bot_rgb: (0.04, 0.43, 0.91),
            has_checkbox: true,
            get_checked: |c| c.display,
            set_checked: |c, v| c.display = v,
            extra: ControlRowExtra::ModeSelector {
                options: &VISIBILITY_MODE_OPTIONS,
                get_mode: |c| c.display_mode as usize,
                set_mode: |c, v| c.display_mode = v as u8,
                dim_when_unchecked: false,
            },
        },
        MenuBarControlSpec {
            label: "Sonido",
            icon_name: "audio-volume-high-symbolic",
            top_rgb: (0.98, 0.30, 0.35),
            bot_rgb: (0.86, 0.16, 0.22),
            has_checkbox: true,
            get_checked: |c| c.sound,
            set_checked: |c, v| c.sound = v,
            extra: ControlRowExtra::ModeSelector {
                options: &VISIBILITY_MODE_OPTIONS,
                get_mode: |c| c.sound_mode as usize,
                set_mode: |c, v| c.sound_mode = v as u8,
                dim_when_unchecked: false,
            },
        },
        MenuBarControlSpec {
            label: "Ahora suena",
            icon_name: "multimedia-player-symbolic",
            top_rgb: (0.98, 0.26, 0.44),
            bot_rgb: (0.86, 0.14, 0.32),
            has_checkbox: true,
            get_checked: |c| c.now_playing,
            set_checked: |c, v| c.now_playing = v,
            extra: ControlRowExtra::ModeSelector {
                options: &VISIBILITY_MODE_OPTIONS,
                get_mode: |c| c.now_playing_mode as usize,
                set_mode: |c, v| c.now_playing_mode = v as u8,
                dim_when_unchecked: false,
            },
        },
        MenuBarControlSpec {
            label: "Cambio rápido de usuario",
            icon_name: "avatar-default-symbolic",
            top_rgb: (0.64, 0.65, 0.68),
            bot_rgb: (0.48, 0.49, 0.52),
            has_checkbox: true,
            get_checked: |c| c.fast_user_switching,
            set_checked: |c, v| c.fast_user_switching = v,
            extra: ControlRowExtra::ModeSelector {
                options: &USER_SWITCH_MODE_OPTIONS,
                get_mode: |c| c.fast_user_switching_mode as usize,
                set_mode: |c, v| c.fast_user_switching_mode = v as u8,
                dim_when_unchecked: true,
            },
        },
        MenuBarControlSpec {
            label: "Entrada de texto",
            icon_name: "input-keyboard-symbolic",
            top_rgb: (0.64, 0.65, 0.68),
            bot_rgb: (0.48, 0.49, 0.52),
            has_checkbox: true,
            get_checked: |c| c.input_source,
            set_checked: |c, v| c.input_source = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Time Machine",
            icon_name: "document-open-recent-symbolic",
            top_rgb: (0.24, 0.78, 0.72),
            bot_rgb: (0.12, 0.62, 0.57),
            has_checkbox: true,
            get_checked: |c| c.time_machine,
            set_checked: |c, v| c.time_machine = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Brillo del teclado",
            icon_name: "keyboard-brightness-symbolic",
            top_rgb: (0.64, 0.65, 0.68),
            bot_rgb: (0.48, 0.49, 0.52),
            has_checkbox: true,
            get_checked: |c| c.keyboard_brightness,
            set_checked: |c, v| c.keyboard_brightness = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Temporizador",
            icon_name: "alarm-symbolic",
            top_rgb: (0.98, 0.62, 0.16),
            bot_rgb: (0.89, 0.47, 0.05),
            has_checkbox: true,
            get_checked: |c| c.timer,
            set_checked: |c, v| c.timer = v,
            extra: ControlRowExtra::ModeSelector {
                options: &VISIBILITY_MODE_OPTIONS,
                get_mode: |c| c.timer_mode as usize,
                set_mode: |c, v| c.timer_mode = v as u8,
                dim_when_unchecked: false,
            },
        },
        MenuBarControlSpec {
            label: "Clima",
            icon_name: "weather-few-clouds-symbolic",
            top_rgb: (0.31, 0.71, 0.99),
            bot_rgb: (0.16, 0.53, 0.94),
            has_checkbox: true,
            get_checked: |c| c.weather,
            set_checked: |c, v| c.weather = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Perfil de rendimiento",
            icon_name: "power-profile-balanced-symbolic",
            top_rgb: (0.28, 0.76, 0.52),
            bot_rgb: (0.16, 0.62, 0.39),
            has_checkbox: true,
            get_checked: |c| c.power_profile,
            set_checked: |c, v| c.power_profile = v,
            extra: ControlRowExtra::None,
        },
        MenuBarControlSpec {
            label: "Ordenamiento de ventanas",
            icon_name: "view-grid-symbolic",
            top_rgb: (0.36, 0.52, 0.96),
            bot_rgb: (0.22, 0.37, 0.85),
            has_checkbox: true,
            get_checked: |c| c.window_tiling,
            set_checked: |c, v| c.window_tiling = v,
            extra: ControlRowExtra::None,
        },
    ]
}

fn save_menubar_and_reload(cfg: &MenuBarConfig) {
    let _ = cfg.save();
    let _ = IpcClient::send_command(IpcMessage::ReloadMenuBarConfig);
}

pub fn create_menubar_pane(
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

    let cfg = Rc::new(RefCell::new(MenuBarConfig::load()));
    let syncing = Rc::new(Cell::new(false));
    let sync_hooks: Rc<RefCell<Vec<Box<dyn Fn(&MenuBarConfig)>>>> =
        Rc::new(RefCell::new(Vec::new()));

    // ------------------------------------------------------------------------
    // Tarjeta 1: Comportamiento general de la barra de menús
    // ------------------------------------------------------------------------
    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let (auto_hide_row, set_auto_hide_idx) = build_dropdown_row(
        "Ocultar y mostrar la barra de menús automáticamente",
        &AUTO_HIDE_OPTIONS,
        cfg.borrow().auto_hide_mode as usize,
        {
            let cfg_c = cfg.clone();
            let sync_c = syncing.clone();
            move |idx| {
                if sync_c.get() {
                    return;
                }
                let mut b = cfg_c.borrow_mut();
                b.auto_hide_mode = idx as u8;
                save_menubar_and_reload(&b);
            }
        },
    );
    sync_hooks.borrow_mut().push(Box::new(move |c| {
        set_auto_hide_idx(c.auto_hide_mode as usize);
    }));
    top_card.append(&auto_hide_row);
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (bg_row, bg_sw) = build_switch_row(
        "Mostrar fondo de la barra de menús",
        cfg.borrow().show_background,
        {
            let cfg_c = cfg.clone();
            let sync_c = syncing.clone();
            move |active| {
                if sync_c.get() {
                    return;
                }
                let mut b = cfg_c.borrow_mut();
                b.show_background = active;
                save_menubar_and_reload(&b);
            }
        },
    );
    sync_hooks.borrow_mut().push(Box::new(move |c| {
        bg_sw.set_active(c.show_background);
    }));
    top_card.append(&bg_row);
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let recent_labels: Vec<&str> = RECENT_OPTIONS.iter().map(|(_, lbl)| *lbl).collect();
    let initial_recent_idx = recent_idx_for_count(cfg.borrow().recent_items_count);
    let (recent_row, set_recent_idx) = build_dropdown_row(
        "Documentos, apps y servidores recientes",
        &recent_labels,
        initial_recent_idx,
        {
            let cfg_c = cfg.clone();
            let sync_c = syncing.clone();
            move |idx| {
                if sync_c.get() {
                    return;
                }
                let mut b = cfg_c.borrow_mut();
                b.recent_items_count = RECENT_OPTIONS.get(idx).map(|(n, _)| *n).unwrap_or(10);
                save_menubar_and_reload(&b);
            }
        },
    );
    sync_hooks.borrow_mut().push(Box::new(move |c| {
        set_recent_idx(recent_idx_for_count(c.recent_items_count));
    }));
    top_card.append(&recent_row);

    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ------------------------------------------------------------------------
    // Tarjeta 2: Sugerencias en galería de controles
    // ------------------------------------------------------------------------
    let gallery_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let (gallery_row, gallery_sw) = build_switch_row(
        "Mostrar sugerencias en la galería de controles",
        cfg.borrow().show_gallery_suggestions,
        {
            let cfg_c = cfg.clone();
            let sync_c = syncing.clone();
            move |active| {
                if sync_c.get() {
                    return;
                }
                let mut b = cfg_c.borrow_mut();
                b.show_gallery_suggestions = active;
                save_menubar_and_reload(&b);
            }
        },
    );
    sync_hooks.borrow_mut().push(Box::new(move |c| {
        gallery_sw.set_active(c.show_gallery_suggestions);
    }));
    gallery_card.append(&gallery_row);
    pane.append(&wrap_squircle_card(&gallery_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ------------------------------------------------------------------------
    // Sección: Controles de la barra de menús
    // ------------------------------------------------------------------------
    pane.append(
        &Label::builder()
            .label("Controles de la barra de menús")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    // Banner descriptivo con miniatura del Centro de control y botón Agregar controles...
    let banner_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let banner_left = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    banner_left.append(
        &Label::builder()
            .label("Los controles del sistema y de las apps se pueden configurar para que aparezcan tanto en el centro de control como en la barra de menús.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let (add_btn_wrap, add_btn) = make_squircle_button("Agregar controles...", false, None);
    add_btn_wrap.set_halign(Align::Start);
    banner_left.append(&add_btn_wrap);

    let preview_area = DrawingArea::new();
    preview_area.set_content_width(148);
    preview_area.set_content_height(78);
    preview_area.set_halign(Align::End);
    preview_area.set_valign(Align::Center);
    preview_area.set_draw_func(|_, cr, w, h| {
        draw_control_center_illustration(cr, w as f64, h as f64);
    });

    banner_card.append(&banner_left);
    banner_card.append(&preview_area);
    pane.append(&wrap_squircle_card(&banner_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Lista principal de controles de la barra de menús
    let controls_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let specs = control_specs();
    for (idx, spec) in specs.iter().enumerate() {
        if idx > 0 {
            controls_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        build_control_row(
            &controls_card,
            spec,
            parent_window,
            &navigate,
            &cfg,
            &syncing,
            &mut sync_hooks.borrow_mut(),
        );
    }
    pane.append(&wrap_squircle_card(&controls_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Pie inferior: [Restablecer el centro de control...] [?]
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (reset_wrap, reset_btn) =
        make_squircle_button("Restablecer el centro de control...", false, None);
    let (help_wrap, _) = make_help_circle_button(
        "Elige qué controles aparecen en la barra de menús y cuándo mostrar módulos como Enfoque, Sonido o Ahora suena.",
    );
    footer.append(&reset_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    let sync_all: Rc<dyn Fn(MenuBarConfig)> = Rc::new({
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        let hooks_c = sync_hooks;
        move |next_cfg: MenuBarConfig| {
            *cfg_c.borrow_mut() = next_cfg;
            let snapshot = cfg_c.borrow().clone();
            sync_c.set(true);
            for hook in hooks_c.borrow().iter() {
                hook(&snapshot);
            }
            sync_c.set(false);
        }
    });

    {
        let parent_c = parent_window.clone();
        let cfg_c = cfg.clone();
        let sync_c = sync_all.clone();
        add_btn.connect_clicked(move |_| {
            open_add_controls_dialog(&parent_c, &cfg_c, &sync_c);
        });
    }

    {
        let sync_c = sync_all.clone();
        reset_btn.connect_clicked(move |_| {
            let defaults = MenuBarConfig::default();
            save_menubar_and_reload(&defaults);
            sync_c(defaults);
        });
    }

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        sync_all(MenuBarConfig::load());
    });

    scrolled.set_child(Some(&pane));
    (scrolled, refresh)
}

fn recent_idx_for_count(count: u32) -> usize {
    RECENT_OPTIONS
        .iter()
        .position(|(n, _)| *n == count)
        .unwrap_or(2)
}

fn build_switch_row<F: Fn(bool) + 'static>(
    title: &str,
    initial: bool,
    on_toggle: F,
) -> (GtkBox, Switch) {
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
    let sw = Switch::builder()
        .active(initial)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(&*sw);
    (row, sw)
}

fn build_compact_dropdown<F: Fn(usize) + 'static>(
    options: &[&str],
    initial_idx: usize,
    on_select: F,
) -> (MenuButton, Rc<dyn Fn(usize)>) {
    let selected = Rc::new(Cell::new(initial_idx.min(options.len().saturating_sub(1))));
    let opts: Vec<String> = options.iter().map(|s| s.to_string()).collect();
    let cb = Rc::new(on_select);

    let inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
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
        let opts_c = opts.clone();
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
                let os = opt.clone();
                let cbc = cb_c.clone();
                let rfc = self_ref.clone();
                b.connect_clicked(move |_| {
                    pc.popdown();
                    sc.set(idx);
                    lc.set_label(&os);
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
        let opts_c = opts;
        let reb_c = reb_fn;
        Rc::new(move |idx: usize| {
            let clamped = idx.min(opts_c.len().saturating_sub(1));
            sel_c.set(clamped);
            if let Some(txt) = opts_c.get(clamped) {
                lbl_c.set_label(txt);
            }
            reb_c();
        })
    };

    (btn, set_selected)
}

fn build_dropdown_row<F: Fn(usize) + 'static>(
    title: &str,
    options: &[&str],
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
    let (btn, set_selected) = build_compact_dropdown(options, initial_idx, on_select);
    row.append(&btn);
    (row, set_selected)
}

fn build_control_row(
    parent_card: &GtkBox,
    spec: &MenuBarControlSpec,
    parent_window: &ApplicationWindow,
    navigate: &Rc<dyn Fn(&str)>,
    cfg: &Rc<RefCell<MenuBarConfig>>,
    syncing: &Rc<Cell<bool>>,
    sync_hooks: &mut Vec<Box<dyn Fn(&MenuBarConfig)>>,
) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();

    let initial_checked = (spec.get_checked)(&cfg.borrow());
    let mut mode_widget_for_dim: Option<MenuButton> = None;

    if spec.has_checkbox {
        let chk = CheckButton::builder()
            .active(initial_checked)
            .css_classes(["menubar-check"])
            .valign(Align::Center)
            .build();
        let set_checked_fn = spec.set_checked;
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        row.append(&chk);

        let badge = make_squircle_colored_badge(
            26,
            spec.icon_name,
            14,
            13.0,
            spec.top_rgb,
            spec.bot_rgb,
        );
        badge.set_valign(Align::Center);
        row.append(&badge);

        row.append(
            &Label::builder()
                .label(spec.label)
                .css_classes(["wifi-row-title"])
                .wrap(true)
                .xalign(0.0)
                .hexpand(true)
                .valign(Align::Center)
                .build(),
        );

        match spec.extra {
            ControlRowExtra::None => {}
            ControlRowExtra::BatteryOptions => {
                let (opt_wrap, opt_btn) = make_squircle_button("Opciones de batería...", false, None);
                let pw = parent_window.clone();
                let cc = cfg.clone();
                let nav_c = navigate.clone();
                opt_btn.connect_clicked(move |_| {
                    open_battery_options_dialog(&pw, &cc, &nav_c);
                });
                row.append(&opt_wrap);
            }
            ControlRowExtra::ModeSelector {
                options,
                get_mode,
                set_mode,
                dim_when_unchecked,
            } => {
                let (btn, set_idx) = build_compact_dropdown(options, get_mode(&cfg.borrow()), {
                    let cfg_m = cfg.clone();
                    let sync_m = syncing.clone();
                    move |idx| {
                        if sync_m.get() {
                            return;
                        }
                        let mut b = cfg_m.borrow_mut();
                        set_mode(&mut b, idx);
                        save_menubar_and_reload(&b);
                    }
                });
                if dim_when_unchecked {
                    btn.set_sensitive(initial_checked);
                    mode_widget_for_dim = Some(btn.clone());
                }
                row.append(&btn);
                sync_hooks.push(Box::new(move |c| {
                    set_idx(get_mode(c));
                }));
            }
            ControlRowExtra::ClockOptions => {}
        }

        let get_checked_fn = spec.get_checked;
        let dim_btn = mode_widget_for_dim.clone();
        chk.connect_toggled(move |cb| {
            let active = cb.is_active();
            if let Some(ref b) = dim_btn {
                b.set_sensitive(active);
            }
            if sync_c.get() {
                return;
            }
            let mut b = cfg_c.borrow_mut();
            set_checked_fn(&mut b, active);
            save_menubar_and_reload(&b);
        });

        let dim_btn_sync = mode_widget_for_dim;
        sync_hooks.push(Box::new(move |c| {
            let v = get_checked_fn(c);
            chk.set_active(v);
            if let Some(ref b) = dim_btn_sync {
                b.set_sensitive(v);
            }
        }));
    } else {
        // Reloj: no tiene casilla de verificación, alinea con espaciador de 15 px
        let spacer = GtkBox::builder()
            .width_request(15)
            .height_request(15)
            .valign(Align::Center)
            .build();
        row.append(&spacer);

        let badge = make_squircle_colored_badge(
            26,
            spec.icon_name,
            14,
            13.0,
            spec.top_rgb,
            spec.bot_rgb,
        );
        badge.set_valign(Align::Center);
        row.append(&badge);

        row.append(
            &Label::builder()
                .label(spec.label)
                .css_classes(["wifi-row-title"])
                .wrap(true)
                .xalign(0.0)
                .hexpand(true)
                .valign(Align::Center)
                .build(),
        );

        let (opt_wrap, opt_btn) = make_squircle_button("Opciones de reloj...", false, None);
        let pw = parent_window.clone();
        let cc = cfg.clone();
        opt_btn.connect_clicked(move |_| {
            open_clock_options_dialog(&pw, &cc);
        });
        row.append(&opt_wrap);
    }

    parent_card.append(&row);
}

fn draw_control_center_illustration(cr: &gtk4::cairo::Context, w: f64, h: f64) {
    let dark = libadwaita::StyleManager::default().is_dark();
    let _ = cr.save();

    let (bar_r, bar_g, bar_b, bar_a) = if dark {
        (1.0, 1.0, 1.0, 0.14)
    } else {
        (0.0, 0.0, 0.0, 0.09)
    };
    let (card_r, card_g, card_b, card_a) = if dark {
        (1.0, 1.0, 1.0, 0.12)
    } else {
        (1.0, 1.0, 1.0, 0.72)
    };
    let (pill_r, pill_g, pill_b, pill_a) = if dark {
        (1.0, 1.0, 1.0, 0.18)
    } else {
        (0.0, 0.0, 0.0, 0.07)
    };

    // Barra superior miniatura
    cr.save().ok();
    crate::squircle_path(cr, w, 12.0, 5.0);
    cr.set_source_rgba(bar_r, bar_g, bar_b, bar_a);
    let _ = cr.fill();
    cr.restore().ok();

    for i in 0..6 {
        let cx = w - 10.0 - (i as f64) * 11.0;
        cr.arc(cx, 6.0, 2.2, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(
            if dark { 1.0 } else { 0.35 },
            if dark { 1.0 } else { 0.37 },
            if dark { 1.0 } else { 0.42 },
            0.55,
        );
        let _ = cr.fill();
    }

    // Panel flotante tipo Centro de control debajo de la barra
    let cc_x = 12.0;
    let cc_y = 16.0;
    let cc_w = w - 16.0;
    let cc_h = h - 18.0;

    cr.save().ok();
    cr.translate(cc_x, cc_y);
    crate::squircle_path(cr, cc_w, cc_h, 10.0);
    cr.set_source_rgba(card_r, card_g, card_b, card_a);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(
        if dark { 1.0 } else { 0.0 },
        if dark { 1.0 } else { 0.0 },
        if dark { 1.0 } else { 0.0 },
        if dark { 0.14 } else { 0.08 },
    );
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.restore().ok();

    // Bloques internos del mini Centro de control (2 filas x 2 columnas)
    let tile_rects = [
        (cc_x + 7.0, cc_y + 7.0, 52.0, 22.0),
        (cc_x + 65.0, cc_y + 7.0, 58.0, 22.0),
        (cc_x + 7.0, cc_y + 34.0, 34.0, 20.0),
        (cc_x + 46.0, cc_y + 34.0, 77.0, 20.0),
    ];
    for (idx, (x, y, tw, th)) in tile_rects.iter().enumerate() {
        cr.save().ok();
        cr.translate(*x, *y);
        crate::squircle_path(cr, *tw, *th, 6.0);
        cr.set_source_rgba(pill_r, pill_g, pill_b, pill_a);
        let _ = cr.fill();
        cr.restore().ok();

        cr.arc(x + 9.0, y + th * 0.5, 4.5, 0.0, std::f64::consts::TAU);
        let (ar, ag, ab) = if idx == 0 {
            crate::accent_color()
        } else {
            (0.55, 0.56, 0.60)
        };
        cr.set_source_rgba(ar, ag, ab, if idx == 0 { 0.85 } else { 0.45 });
        let _ = cr.fill();
    }

    let _ = cr.restore();
}

fn open_clock_options_dialog(parent: &ApplicationWindow, cfg: &Rc<RefCell<MenuBarConfig>>) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Opciones de reloj")
        .default_width(430)
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

    let (date_row, _) = build_dropdown_row(
        "Mostrar fecha",
        &["Al haber espacio", "Siempre", "Nunca"],
        cfg.borrow().clock_show_date as usize,
        {
            let cc = cfg.clone();
            move |idx| {
                let mut b = cc.borrow_mut();
                b.clock_show_date = idx as u8;
                save_menubar_and_reload(&b);
            }
        },
    );
    card.append(&date_row);

    let switches: [(&str, fn(&MenuBarConfig) -> bool, fn(&mut MenuBarConfig, bool)); 5] = [
        (
            "Mostrar el día de la semana",
            |c| c.clock_show_day_of_week,
            |c, v| c.clock_show_day_of_week = v,
        ),
        (
            "Usar un reloj de 24 horas",
            |c| c.clock_24h,
            |c, v| c.clock_24h = v,
        ),
        (
            "Mostrar AM/PM",
            |c| c.clock_show_am_pm,
            |c, v| c.clock_show_am_pm = v,
        ),
        (
            "Mostrar fecha y hora con segundos",
            |c| c.clock_show_seconds,
            |c, v| c.clock_show_seconds = v,
        ),
        (
            "Parpadear los separadores de hora",
            |c| c.clock_flash_separators,
            |c, v| c.clock_flash_separators = v,
        ),
    ];

    for (title, getter, setter) in switches {
        card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        let initial = getter(&cfg.borrow());
        let cc = cfg.clone();
        let (row, _) = build_switch_row(title, initial, move |active| {
            let mut b = cc.borrow_mut();
            setter(&mut b, active);
            save_menubar_and_reload(&b);
        });
        card.append(&row);
    }

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    ok_wrap.set_halign(Align::End);
    let dlg_c = dialog.clone();
    ok_btn.connect_clicked(move |_| dlg_c.close());
    vbox.append(&ok_wrap);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn open_battery_options_dialog(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<MenuBarConfig>>,
    navigate: &Rc<dyn Fn(&str)>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Opciones de batería")
        .default_width(420)
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
    let (pct_row, _) = build_switch_row(
        "Mostrar porcentaje en la barra de menús",
        cfg.borrow().battery_show_percentage,
        {
            let cc = cfg.clone();
            move |active| {
                let mut b = cc.borrow_mut();
                b.battery_show_percentage = active;
                save_menubar_and_reload(&b);
            }
        },
    );
    card.append(&pct_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let (cc_row, _) = build_switch_row(
        "Mostrar en el centro de control",
        cfg.borrow().battery_show_in_control_center,
        {
            let cc = cfg.clone();
            move |active| {
                let mut b = cc.borrow_mut();
                b.battery_show_in_control_center = active;
                save_menubar_and_reload(&b);
            }
        },
    );
    card.append(&cc_row);
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let btn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (go_wrap, go_btn) = make_squircle_button("Ir a Batería...", false, None);
    let dlg_nav = dialog.clone();
    let nav_c = navigate.clone();
    go_btn.connect_clicked(move |_| {
        dlg_nav.close();
        nav_c("battery");
    });
    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| dlg_ok.close());
    btn_row.append(&go_wrap);
    btn_row.append(&ok_wrap);
    vbox.append(&btn_row);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn open_add_controls_dialog(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<MenuBarConfig>>,
    sync_all: &Rc<dyn Fn(MenuBarConfig)>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Agregar controles")
        .default_width(450)
        .default_height(480)
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
            .label("Selecciona los controles que deseas mostrar rápidamente en la barra de menús.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .build();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let mut first = true;
    for spec in control_specs().into_iter().filter(|s| s.has_checkbox) {
        if !first {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        first = false;
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wifi-pref-row"])
            .build();
        let badge = make_squircle_colored_badge(
            24,
            spec.icon_name,
            13,
            12.0,
            spec.top_rgb,
            spec.bot_rgb,
        );
        badge.set_valign(Align::Center);
        row.append(&badge);
        row.append(
            &Label::builder()
                .label(spec.label)
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .hexpand(true)
                .valign(Align::Center)
                .build(),
        );
        let sw = Switch::builder()
            .active((spec.get_checked)(&cfg.borrow()))
            .css_classes(["macos-switch"])
            .valign(Align::Center)
            .build();
        let cc = cfg.clone();
        let sa = sync_all.clone();
        let setter = spec.set_checked;
        sw.connect_state_set(move |_, active| {
            let mut next = cc.borrow().clone();
            setter(&mut next, active);
            save_menubar_and_reload(&next);
            sa(next);
            glib::Propagation::Proceed
        });
        row.append(&*sw);
        card.append(&row);
    }

    scroll.set_child(Some(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS)));
    vbox.append(&scroll);

    let (ok_wrap, ok_btn) = make_squircle_button("Listo", true, None);
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
    fn control_specs_match_menubar_layout_order() {
        let specs = control_specs();
        let labels: Vec<&str> = specs.iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![
                "Reloj",
                "Spotlight",
                "Wi-Fi",
                "Bluetooth",
                "Batería",
                "AirDrop",
                "Enfoque",
                "Duplicar pantalla",
                "Pantalla",
                "Sonido",
                "Ahora suena",
                "Cambio rápido de usuario",
                "Entrada de texto",
                "Time Machine",
                "Brillo del teclado",
                "Temporizador",
                "Clima",
                "Perfil de rendimiento",
                "Ordenamiento de ventanas",
            ]
        );
        assert!(!specs[0].has_checkbox);
        assert!(specs[1..].iter().all(|s| s.has_checkbox));
    }
}
