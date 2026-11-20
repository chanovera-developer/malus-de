use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, ApplicationWindow, Box as GtkBox, Button, DrawingArea, Image, Label,
    MenuButton, Orientation, Overlay, PolicyType, Popover, Scale, ScrolledWindow, Switch,
};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use crate::wifi_pane::{
    detail_kv_row, make_help_circle_button, make_popover_action_row, make_squircle_button,
    wrap_squircle_card, wrap_squircle_round_control, RoundControlStyle, WIFI_CARD_SQUIRCLE_RADIUS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LowPowerSchedule {
    Never,
    Always,
    BatteryOnly,
    PowerAdapterOnly,
}

impl LowPowerSchedule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Nunca",
            Self::Always => "Siempre",
            Self::BatteryOnly => "Sólo al usar la batería",
            Self::PowerAdapterOnly => "Sólo con el adaptador de corriente",
        }
    }

    pub fn from_index(idx: usize) -> Self {
        match idx {
            1 => Self::Always,
            2 => Self::BatteryOnly,
            3 => Self::PowerAdapterOnly,
            _ => Self::Never,
        }
    }

    pub fn to_index(self) -> usize {
        match self {
            Self::Never => 0,
            Self::Always => 1,
            Self::BatteryOnly => 2,
            Self::PowerAdapterOnly => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WakeForNetworkPolicy {
    Never,
    PowerAdapterOnly,
    Always,
}

impl WakeForNetworkPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Nunca",
            Self::PowerAdapterOnly => "Sólo con el adaptador de corriente",
            Self::Always => "Siempre",
        }
    }

    pub fn from_index(idx: usize) -> Self {
        match idx {
            1 => Self::PowerAdapterOnly,
            2 => Self::Always,
            _ => Self::Never,
        }
    }

    pub fn to_index(self) -> usize {
        match self {
            Self::Never => 0,
            Self::PowerAdapterOnly => 1,
            Self::Always => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatterySettingsConfig {
    pub low_power: LowPowerSchedule,
    pub charge_limit_pct: u8,
    pub optimized_charging: bool,
    pub dim_display_on_battery: bool,
    pub prevent_sleep_on_ac_display_off: bool,
    pub wake_for_network: WakeForNetworkPolicy,
    pub optimize_video_streaming_on_battery: bool,
}

impl Default for BatterySettingsConfig {
    fn default() -> Self {
        Self {
            low_power: LowPowerSchedule::Never,
            charge_limit_pct: 100,
            optimized_charging: true,
            dim_display_on_battery: true,
            prevent_sleep_on_ac_display_off: false,
            wake_for_network: WakeForNetworkPolicy::PowerAdapterOnly,
            optimize_video_streaming_on_battery: false,
        }
    }
}

impl BatterySettingsConfig {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/battery-settings.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(parsed) = toml::from_str::<Self>(&raw) {
                return parsed;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(raw) = toml::to_string_pretty(self) {
            let _ = fs::write(path, raw);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatteryHistorySample {
    pub ts_secs: i64,
    pub level_pct: u8,
    pub charging: bool,
    pub screen_on_mins: f64,
}

#[derive(Debug, Clone)]
pub struct HourlySlot {
    pub level_pct: Option<u8>,
    pub charging: bool,
}

#[derive(Debug, Clone)]
pub struct DailySlot {
    pub weekday_letter: String,
    pub date_caption: Option<String>,
    pub avg_energy_pct: Option<u8>,
    pub screen_on_hours: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct BatteryHardwareSnapshot {
    pub has_battery: bool,
    pub sysfs_path: Option<String>,
    pub level_pct: u8,
    pub status: String,
    pub ac_online: bool,
    pub condition_label: String,
    pub max_capacity_pct: u8,
    pub cycle_count: Option<u32>,
    pub design_wh: Option<f64>,
    pub full_wh: Option<f64>,
    pub manufacturer: String,
    pub model_name: String,
    pub charge_limit_writable: bool,
    pub sysfs_charge_limit: Option<u8>,
    pub tuned_profile: String,
    pub samples: Vec<BatteryHistorySample>,
}

/// Callback para actualizar la línea de subtítulo ("Nivel de batería: N%") en la cabecera de Ajustes.
pub type BatteryHeaderCallback = Rc<dyn Fn(&str, &str)>;

/// Crea el panel de Configuración de Batería basado en `assets/screenshots/Bateria00.png`..`Bateria05.png`.
pub fn create_battery_pane(
    parent_window: &ApplicationWindow,
    on_header_update: BatteryHeaderCallback,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let cfg = Rc::new(RefCell::new(BatterySettingsConfig::load()));
    let latest_snap = Rc::new(RefCell::new(BatteryHardwareSnapshot::default()));
    let reload_lock = Rc::new(Cell::new(false));
    let active_tab = Rc::new(Cell::new(0u8)); // 0 = Últimas 24 horas, 1 = Últimos 10 días

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

    let status_banner = Label::builder()
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    pane.append(&status_banner);

    // =========================================================================
    // Tarjeta 1: "Ahorrar batería" + selector transparente estilo macOS (Bateria01)
    // =========================================================================
    let low_power_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .build();
    let low_power_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    low_power_row.append(
        &Label::builder()
            .label("Ahorrar batería")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );

    let low_power_options = [
        LowPowerSchedule::Never.label(),
        LowPowerSchedule::Always.label(),
        LowPowerSchedule::BatteryOnly.label(),
        LowPowerSchedule::PowerAdapterOnly.label(),
    ];
    let initial_lp_idx = cfg.borrow().low_power.to_index();
    let trigger_reload: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let (lp_btn, lp_sync_fn) = make_selector_menu_button(&low_power_options, initial_lp_idx, {
        let cfg_c = cfg.clone();
        let snap_c = latest_snap.clone();
        let ban_c = status_banner.clone();
        move |idx| {
            let sched = LowPowerSchedule::from_index(idx);
            {
                let mut c = cfg_c.borrow_mut();
                c.low_power = sched;
                c.save();
            }
            let ac_online = snap_c.borrow().ac_online;
            let has_bat = snap_c.borrow().has_battery;
            apply_low_power_policy_async(sched, has_bat, ac_online, &ban_c);
        }
    });
    low_power_row.append(&lp_btn);
    low_power_card.append(&low_power_row);
    pane.append(&wrap_squircle_card(
        &low_power_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // =========================================================================
    // Tarjeta 2: "Condición de la batería" + "Recarga" (Bateria00 / 02 / 03)
    // =========================================================================
    let info_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let cond_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    cond_row.append(
        &Label::builder()
            .label("Condición de la batería")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let cond_status_lbl = Label::builder()
        .label("Normal")
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let (cond_info_wrap, cond_info_btn) = make_circle_info_button("Detalles de condición de batería");
    cond_row.append(&cond_status_lbl);
    cond_row.append(&cond_info_wrap);
    info_card.append(&cond_row);

    info_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let charge_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    charge_row.append(
        &Label::builder()
            .label("Recarga")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let (charge_info_wrap, charge_info_btn) = make_circle_info_button("Límite de recarga y recarga optimizada");
    charge_row.append(&charge_info_wrap);
    info_card.append(&charge_row);

    pane.append(&wrap_squircle_card(&info_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // =========================================================================
    // Selector segmentado [Últimas 24 horas | Últimos 10 días] + Tarjeta de Gráficas
    // =========================================================================
    let chart_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .css_classes(["bat-chart-card"])
        .build();

    let seg_track = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .homogeneous(true)
        .spacing(2)
        .css_classes(["bat-segmented-track"])
        .build();
    let tab_24h = Button::builder()
        .label("Últimas 24 horas")
        .has_frame(false)
        .css_classes(["bat-seg-tab", "bat-seg-tab-active"])
        .build();
    let tab_10d = Button::builder()
        .label("Últimos 10 días")
        .has_frame(false)
        .css_classes(["bat-seg-tab"])
        .build();
    seg_track.append(&tab_24h);
    seg_track.append(&tab_10d);
    chart_card.append(&seg_track);

    let last_charge_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .margin_top(2)
        .build();
    let last_charge_title = Label::builder()
        .label("Última recarga registrada")
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .build();
    let last_charge_sub = Label::builder()
        .label("Recopilando muestras locales de batería")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();
    last_charge_box.append(&last_charge_title);
    last_charge_box.append(&last_charge_sub);
    chart_card.append(&last_charge_box);

    let level_chart_title = Label::builder()
        .label("Nivel de batería")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .margin_top(2)
        .build();
    chart_card.append(&level_chart_title);

    let level_canvas = DrawingArea::new();
    level_canvas.set_content_height(132);
    level_canvas.set_hexpand(true);
    chart_card.append(&level_canvas);

    let screen_chart_title = Label::builder()
        .label("Uso con pantalla encendida")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .margin_top(4)
        .build();
    chart_card.append(&screen_chart_title);

    let screen_canvas = DrawingArea::new();
    screen_canvas.set_content_height(132);
    screen_canvas.set_hexpand(true);
    chart_card.append(&screen_canvas);

    pane.append(&wrap_squircle_card(&chart_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // =========================================================================
    // Pie inferior: [Opciones...] [?] (Bateria00 / Bateria05)
    // =========================================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (options_wrap, options_btn) = make_squircle_button("Opciones...", false, None);
    footer.append(&options_wrap);

    let (help_wrap, help_btn) = make_help_circle_button("Ayuda sobre Batería");
    help_btn.set_popover(Some(&build_battery_help_popover()));
    footer.append(&help_wrap);
    pane.append(&footer);

    scrolled.set_child(Some(&pane));

    // =========================================================================
    // Dibujo Cairo de gráficas (Últimas 24 horas y Últimos 10 días)
    // =========================================================================
    {
        let tab_c = active_tab.clone();
        let snap_c = latest_snap.clone();
        level_canvas.set_draw_func(move |_, cr, w, h| {
            let snap = snap_c.borrow();
            if tab_c.get() == 0 {
                draw_battery_level_24h(cr, w as f64, h as f64, &snap.samples);
            } else {
                draw_energy_usage_10d(cr, w as f64, h as f64, &snap.samples);
            }
        });
    }
    {
        let tab_c = active_tab.clone();
        let snap_c = latest_snap.clone();
        screen_canvas.set_draw_func(move |_, cr, w, h| {
            let snap = snap_c.borrow();
            if tab_c.get() == 0 {
                draw_screen_on_24h(cr, w as f64, h as f64, &snap.samples);
            } else {
                draw_screen_on_10d(cr, w as f64, h as f64, &snap.samples);
            }
        });
    }

    // Cambio de pestañas [Últimas 24 horas | Últimos 10 días]
    {
        let tab_c = active_tab.clone();
        let t24 = tab_24h.clone();
        let t10 = tab_10d.clone();
        let lbl_t = level_chart_title.clone();
        let lc = level_canvas.clone();
        let sc = screen_canvas.clone();
        tab_24h.connect_clicked(move |_| {
            tab_c.set(0);
            t24.add_css_class("bat-seg-tab-active");
            t10.remove_css_class("bat-seg-tab-active");
            lbl_t.set_label("Nivel de batería");
            lc.queue_draw();
            sc.queue_draw();
        });
    }
    {
        let tab_c = active_tab.clone();
        let t24 = tab_24h.clone();
        let t10 = tab_10d.clone();
        let lbl_t = level_chart_title.clone();
        let lc = level_canvas.clone();
        let sc = screen_canvas.clone();
        tab_10d.connect_clicked(move |_| {
            tab_c.set(1);
            t10.add_css_class("bat-seg-tab-active");
            t24.remove_css_class("bat-seg-tab-active");
            lbl_t.set_label("Uso de energía");
            lc.queue_draw();
            sc.queue_draw();
        });
    }

    // =========================================================================
    // Recarga asíncrona de estado de hardware y muestras
    // =========================================================================
    let run_reload: Rc<dyn Fn()> = {
        let lock_c = reload_lock.clone();
        let snap_c = latest_snap.clone();
        let cfg_c = cfg.clone();
        let lp_sync = lp_sync_fn.clone();
        let cond_lbl = cond_status_lbl.clone();
        let lc_title = last_charge_title.clone();
        let lc_sub = last_charge_sub.clone();
        let lvl_draw = level_canvas.clone();
        let scr_draw = screen_canvas.clone();
        let hdr_cb = on_header_update.clone();

        Rc::new(move || {
            if lock_c.replace(true) {
                return;
            }
            let (tx, rx) = async_channel::bounded::<BatteryHardwareSnapshot>(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(collect_battery_snapshot());
            });

            let lock_d = lock_c.clone();
            let snap_d = snap_c.clone();
            let cfg_d = cfg_c.clone();
            let lp_d = lp_sync.clone();
            let cond_d = cond_lbl.clone();
            let lct_d = lc_title.clone();
            let lcs_d = lc_sub.clone();
            let lvl_d = lvl_draw.clone();
            let scr_d = scr_draw.clone();
            let hdr_d = hdr_cb.clone();

            glib::spawn_future_local(async move {
                if let Ok(snap) = rx.recv().await {
                    *snap_d.borrow_mut() = snap.clone();

                    cond_d.set_label(&snap.condition_label);
                    lp_d(cfg_d.borrow().low_power.to_index());

                    let (hdr_icon, hdr_text) = format_battery_header_status(&snap);
                    hdr_d(&hdr_icon, &hdr_text);

                    let (charge_t, charge_s) = format_last_charge_summary(&snap.samples, &snap);
                    lct_d.set_label(&charge_t);
                    lcs_d.set_label(&charge_s);

                    lvl_d.queue_draw();
                    scr_d.queue_draw();
                }
                lock_d.set(false);
            });
        })
    };
    *trigger_reload.borrow_mut() = Some(run_reload.clone());

    // Acciones modales (Bateria02, Bateria03, Bateria05)
    {
        let win_c = parent_window.clone();
        let snap_c = latest_snap.clone();
        cond_info_btn.connect_clicked(move |_| {
            show_battery_condition_dialog(&win_c, &snap_c.borrow());
        });
    }
    {
        let win_c = parent_window.clone();
        let cfg_c = cfg.clone();
        let snap_c = latest_snap.clone();
        let ban_c = status_banner.clone();
        let rel_c = run_reload.clone();
        charge_info_btn.connect_clicked(move |_| {
            show_battery_charging_dialog(&win_c, &cfg_c, &snap_c.borrow(), &ban_c, &rel_c);
        });
    }
    {
        let win_c = parent_window.clone();
        let cfg_c = cfg.clone();
        options_btn.connect_clicked(move |_| {
            show_battery_options_dialog(&win_c, &cfg_c);
        });
    }

    // Refresco periódico discreto cada 30s mientras la vista siga mapeada
    {
        let pane_w = pane.clone();
        let rel_t = run_reload.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(30), move || {
            if pane_w.is_mapped() {
                rel_t();
            }
            glib::ControlFlow::Continue
        });
    }

    run_reload();
    (scrolled, run_reload)
}

fn make_circle_info_button(tooltip: &str) -> (Overlay, Button) {
    let info_lbl = Label::builder()
        .label("i")
        .css_classes(["bt-info-glyph"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let info_btn = Button::builder()
        .child(&info_lbl)
        .css_classes(["wifi-more-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text(tooltip)
        .build();
    let wrap = wrap_squircle_round_control(&info_btn, 20, RoundControlStyle::CircleOutline);
    (wrap, info_btn)
}

fn make_selector_menu_button<F: Fn(usize) + 'static>(
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
    let arrows = Image::builder()
        .icon_name("pan-down-symbolic")
        .pixel_size(12)
        .build();
    inner.append(&cur_lbl);
    inner.append(&arrows);

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

    let rebuild_popover: Rc<dyn Fn()> = {
        let pop_c = pop.clone();
        let sel_c = selected.clone();
        let lbl_c = cur_lbl.clone();
        let opts_c = opts.clone();
        let cb_c = cb.clone();
        let self_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let self_slot_c = self_slot.clone();

        let func: Rc<dyn Fn()> = Rc::new(move || {
            let menu_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();
            let cur = sel_c.get();
            for (idx, opt) in opts_c.iter().enumerate() {
                let (wrap, item_btn) =
                    make_popover_action_row(opt, true, (idx == cur).then_some("✓"));
                let p_click = pop_c.clone();
                let s_click = sel_c.clone();
                let l_click = lbl_c.clone();
                let opt_str = opt.clone();
                let cb_click = cb_c.clone();
                let reb_click = self_slot_c.clone();
                item_btn.connect_clicked(move |_| {
                    p_click.popdown();
                    s_click.set(idx);
                    l_click.set_label(&opt_str);
                    if let Some(f) = reb_click.borrow().as_ref() {
                        f();
                    }
                    cb_click(idx);
                });
                menu_box.append(&wrap);
            }
            pop_c.set_child(Some(&menu_box));
        });
        *self_slot.borrow_mut() = Some(func.clone());
        func
    };
    rebuild_popover();

    let sync_fn: Rc<dyn Fn(usize)> = {
        let sel_s = selected;
        let lbl_s = cur_lbl;
        let opts_s = opts;
        let reb_s = rebuild_popover;
        Rc::new(move |idx: usize| {
            let clamped = idx.min(opts_s.len().saturating_sub(1));
            if sel_s.get() != clamped {
                sel_s.set(clamped);
                if let Some(txt) = opts_s.get(clamped) {
                    lbl_s.set_label(txt);
                }
                reb_s();
            }
        })
    };

    (btn, sync_fn)
}

fn build_battery_help_popover() -> Popover {
    let pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    let box_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .width_request(280)
        .build();
    box_inner.append(
        &Label::builder()
            .label("Batería y Energía")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    box_inner.append(
        &Label::builder()
            .label("• Lee carga, salud, ciclos y límite de carga desde sysfs (/sys/class/power_supply) y UPower.\n• Registra historial local cada vez que se consulta la energía y sincroniza «Ahorrar batería» con el perfil de energía de Malus DE / tuned-adm (ver docs/decisiones/0010-bateria-pendiente.md).")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pop.set_child(Some(&box_inner));
    pop
}

/// Modal "Condición de la batería" (`Bateria02.png`).
fn show_battery_condition_dialog(parent: &ApplicationWindow, snap: &BatteryHardwareSnapshot) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Condición de la batería")
        .default_width(470)
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

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    // Sección 1: Estado de la batería
    let r1 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .build();
    let r1_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    r1_text.append(
        &Label::builder()
            .label("Estado de la batería")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    r1_text.append(
        &Label::builder()
            .label("Las baterías recargables, como componentes de consumo, se vuelven menos eficaces conforme pasa el tiempo.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    r1.append(&r1_text);
    r1.append(
        &Label::builder()
            .label(&snap.condition_label)
            .css_classes(["card-section-sub"])
            .valign(Align::Start)
            .build(),
    );
    card.append(&r1);

    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Sección 2: Capacidad máxima
    let r2 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .build();
    let r2_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    r2_text.append(
        &Label::builder()
            .label("Capacidad máxima")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    r2_text.append(
        &Label::builder()
            .label("Esta es una medición de la capacidad de la batería relativa a cuando era nueva. Una menor capacidad podría disminuir el tiempo de uso entre cada recarga.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    r2.append(&r2_text);
    let max_cap_txt = if snap.has_battery {
        format!("{}%", snap.max_capacity_pct)
    } else {
        "—".to_string()
    };
    r2.append(
        &Label::builder()
            .label(&max_cap_txt)
            .css_classes(["card-section-sub"])
            .valign(Align::Start)
            .build(),
    );
    card.append(&r2);

    // Detalles técnicos desplegables al pulsar "Obtén detalles..."
    let extra_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .visible(false)
        .build();
    extra_box.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    extra_box.append(&detail_kv_row(
        "Dispositivo sysfs",
        snap.sysfs_path.as_deref().unwrap_or("No detectado"),
    ));
    if !snap.manufacturer.is_empty() || !snap.model_name.is_empty() {
        extra_box.append(&detail_kv_row(
            "Fabricante / Modelo",
            &format!("{} {}", snap.manufacturer, snap.model_name).trim().to_string(),
        ));
    }
    if let Some(cycles) = snap.cycle_count {
        extra_box.append(&detail_kv_row("Conteo de ciclos", &cycles.to_string()));
    }
    if let (Some(full), Some(design)) = (snap.full_wh, snap.design_wh) {
        extra_box.append(&detail_kv_row(
            "Capacidad energética",
            &format!("{full:.1} Wh / {design:.1} Wh diseño"),
        ));
    }
    card.append(&extra_box);

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let (det_wrap, det_btn) = make_squircle_button("Obtén detalles...", false, None);
    let extra_c = extra_box.clone();
    det_btn.connect_clicked(move |_| {
        extra_c.set_visible(!extra_c.is_visible());
    });
    footer.append(&det_wrap);
    footer.append(&GtkBox::builder().hexpand(true).build());

    let (ok_wrap, ok_btn) = make_squircle_button("Listo", true, None);
    let d_c = dialog.clone();
    ok_btn.connect_clicked(move |_| d_c.close());
    footer.append(&ok_wrap);

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

/// Modal "Recarga" (`Bateria03.png`) con límite 80-85-90-95-100% y "Recarga optimizada".
fn show_battery_charging_dialog(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<BatterySettingsConfig>>,
    snap: &BatteryHardwareSnapshot,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Recarga")
        .default_width(500)
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

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    let initial_limit = snap
        .sysfs_charge_limit
        .unwrap_or_else(|| cfg.borrow().charge_limit_pct)
        .clamp(80, 100);

    // 1. Límite de recarga
    let lim_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .build();
    let lim_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    lim_text.append(
        &Label::builder()
            .label("Límite de recarga")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    let lim_sub = Label::builder()
        .label(&format!(
            "Tu equipo se recargará hasta el límite del {initial_limit}%."
        ))
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .build();
    lim_text.append(&lim_sub);

    let adj = Adjustment::new(initial_limit as f64, 80.0, 100.0, 5.0, 5.0, 0.0);
    let slider = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&adj)
        .round_digits(0)
        .draw_value(false)
        .width_request(210)
        .css_classes(["bat-limit-slider"])
        .valign(Align::Center)
        .build();
    for pct in [80, 85, 90, 95, 100] {
        slider.add_mark(pct as f64, gtk4::PositionType::Bottom, Some(&format!("{pct}%")));
    }

    let lim_sub_c = lim_sub.clone();
    let selected_limit = Rc::new(Cell::new(snap_charge_limit_stop(initial_limit as i32)));
    let sel_lim_c = selected_limit.clone();
    slider.connect_value_changed(move |sc| {
        let snapped = snap_charge_limit_stop(sc.value().round() as i32);
        if (sc.value() - snapped as f64).abs() > 0.1 {
            sc.set_value(snapped as f64);
        }
        sel_lim_c.set(snapped);
        lim_sub_c.set_label(&format!(
            "Tu equipo se recargará hasta el límite del {snapped}%."
        ));
    });

    lim_row.append(&lim_text);
    lim_row.append(&slider);
    card.append(&lim_row);

    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 2. Recarga optimizada
    let opt_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .build();
    let opt_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    opt_text.append(
        &Label::builder()
            .label("Recarga optimizada")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    opt_text.append(
        &Label::builder()
            .label("Tu equipo aprende tu rutina diaria de recarga para reducir el deterioro de la batería al recargarla a más del 80% hasta que vayas a usarla sin conexión a la corriente.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let opt_sw = Switch::builder()
        .active(cfg.borrow().optimized_charging)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    opt_row.append(&opt_text);
    opt_row.append(&opt_sw);
    card.append(&opt_row);

    // Nota de detalles Linux (oculta salvo "Obtén detalles...")
    let hw_note = Label::builder()
        .label(if snap.charge_limit_writable {
            "Hardware compatible detectado: al pulsar Listo se aplicará `charge_control_end_threshold` en sysfs mediante pkexec."
        } else {
            "Anotación Linux: este kernel/equipo no expone `charge_control_end_threshold` modificable en sysfs; el límite y Recarga optimizada se guardan en battery-settings.toml (ver docs/decisiones/0010-bateria-pendiente.md)."
        })
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    card.append(&hw_note);

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let (det_wrap, det_btn) = make_squircle_button("Obtén detalles...", false, None);
    let note_c = hw_note.clone();
    det_btn.connect_clicked(move |_| note_c.set_visible(!note_c.is_visible()));
    footer.append(&det_wrap);
    footer.append(&GtkBox::builder().hexpand(true).build());

    let (ok_wrap, ok_btn) = make_squircle_button("Listo", true, None);
    let d_ok = dialog.clone();
    let cfg_ok = cfg.clone();
    let sysfs_path = snap.sysfs_path.clone();
    let writable = snap.charge_limit_writable;
    let ban_ok = banner.clone();
    let rel_ok = trigger_reload.clone();
    ok_btn.connect_clicked(move |_| {
        let lim = selected_limit.get();
        let opt = opt_sw.is_active();
        {
            let mut c = cfg_ok.borrow_mut();
            c.charge_limit_pct = lim;
            c.optimized_charging = opt;
            c.save();
        }
        d_ok.close();

        if writable {
            if let Some(bat_dir) = sysfs_path.clone() {
                let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
                std::thread::spawn(move || {
                    let res = write_charge_control_threshold(&bat_dir, lim);
                    let _ = tx.send_blocking(res);
                });
                let b = ban_ok.clone();
                let r = rel_ok.clone();
                glib::spawn_future_local(async move {
                    if let Ok(Err(err)) = rx.recv().await {
                        b.set_label(&format!("No se pudo fijar límite de carga: {err}"));
                        b.set_visible(true);
                    } else {
                        b.set_visible(false);
                    }
                    r();
                });
            }
        }
    });
    footer.append(&ok_wrap);

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

/// Modal "Opciones" de Batería (`Bateria05.png`).
fn show_battery_options_dialog(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<BatterySettingsConfig>>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Opciones de Batería")
        .default_width(530)
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

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let cur = cfg.borrow().clone();

    let (dim_row, dim_sw) = build_option_switch(
        "Atenuar ligeramente la pantalla al usar la batería",
        None,
        cur.dim_display_on_battery,
    );
    card.append(&dim_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (sleep_row, sleep_sw) = build_option_switch(
        "Impedir entrar en reposo automáticamente cuando se apaga la pantalla y se usa el adaptador de corriente",
        None,
        cur.prevent_sleep_on_ac_display_off,
    );
    card.append(&sleep_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Fila 3: Wake for network access
    let wake_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let wake_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    wake_text.append(
        &Label::builder()
            .label("Activar la computadora para permitir el acceso a la red")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    wake_text.append(
        &Label::builder()
            .label("Mientras está en reposo, tu equipo puede recibir tráfico entrante de red —como mensajería y sincronización en segundo plano— para mantener tus apps actualizadas.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let wake_sel = Rc::new(Cell::new(cur.wake_for_network));
    let wake_opts = [
        WakeForNetworkPolicy::Never.label(),
        WakeForNetworkPolicy::PowerAdapterOnly.label(),
        WakeForNetworkPolicy::Always.label(),
    ];
    let (wake_btn, _) = make_selector_menu_button(&wake_opts, cur.wake_for_network.to_index(), {
        let ws = wake_sel.clone();
        move |idx| ws.set(WakeForNetworkPolicy::from_index(idx))
    });
    wake_row.append(&wake_text);
    wake_row.append(&wake_btn);
    card.append(&wake_row);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (hdr_row, hdr_sw) = build_option_switch(
        "Optimizar transmisión de vídeo al usar la batería",
        Some("Para mejorar la duración de la batería, tu equipo transmitirá vídeo en rango dinámico estándar (SDR) en lugar de en alto rango dinámico (HDR)."),
        cur.optimize_video_streaming_on_battery,
    );
    card.append(&hdr_row);

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("Listo", true, None);
    let d_ok = dialog.clone();
    let cfg_ok = cfg.clone();
    ok_btn.connect_clicked(move |_| {
        // TODO(malus-bateria): Conectar brillo en batería, systemd-inhibit, WoL (ethtool) y preferencia SDR/HDR al entorno.
        {
            let mut c = cfg_ok.borrow_mut();
            c.dim_display_on_battery = dim_sw.is_active();
            c.prevent_sleep_on_ac_display_off = sleep_sw.is_active();
            c.wake_for_network = wake_sel.get();
            c.optimize_video_streaming_on_battery = hdr_sw.is_active();
            c.save();
        }
        d_ok.close();
    });
    footer.append(&ok_wrap);
    vbox.append(&footer);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn build_option_switch(title: &str, subtitle: Option<&str>, active: bool) -> (GtkBox, Switch) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
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
        .active(active)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    row.append(&col);
    row.append(&sw);
    (row, sw)
}

// ============================================================================
// Renderizado de gráficas Cairo (Bateria00 / Bateria04)
// ============================================================================

fn draw_chart_grid(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    y_labels: &[&str],
    v_divs: usize,
) -> (f64, f64, f64, f64) {
    let dark = libadwaita::StyleManager::default().is_dark();
    let pad_l = 8.0;
    let pad_r = 44.0;
    let pad_t = 8.0;
    let pad_b = 30.0;
    let gw = (w - pad_l - pad_r).max(40.0);
    let gh = (h - pad_t - pad_b).max(40.0);

    // Marco sutil redondeado del gráfico
    cr.set_source_rgba(
        if dark { 1.0 } else { 0.0 },
        if dark { 1.0 } else { 0.0 },
        if dark { 1.0 } else { 0.0 },
        if dark { 0.03 } else { 0.025 },
    );
    cr.rectangle(pad_l, pad_t, gw, gh);
    let _ = cr.fill();

    // Líneas horizontales + etiquetas derecha (100% / 50% / 0% o 60 min / 30 min / 0 min)
    let h_steps = y_labels.len().saturating_sub(1).max(1);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Normal,
    );
    cr.set_font_size(10.0);
    for (idx, lbl) in y_labels.iter().enumerate() {
        let y = pad_t + gh * (idx as f64 / h_steps as f64);
        cr.set_source_rgba(
            if dark { 1.0 } else { 0.0 },
            if dark { 1.0 } else { 0.0 },
            if dark { 1.0 } else { 0.0 },
            if idx == h_steps { 0.14 } else { 0.08 },
        );
        cr.set_line_width(1.0);
        cr.move_to(pad_l, y);
        cr.line_to(pad_l + gw, y);
        let _ = cr.stroke();

        cr.set_source_rgba(
            if dark { 0.85 } else { 0.35 },
            if dark { 0.85 } else { 0.35 },
            if dark { 0.88 } else { 0.38 },
            0.82,
        );
        cr.move_to(pad_l + gw + 6.0, y + 3.5);
        let _ = cr.show_text(lbl);
    }

    // Divisiones verticales punteadas
    cr.set_dash(&[2.0, 2.5], 0.0);
    for i in 0..=v_divs {
        let x = pad_l + gw * (i as f64 / v_divs.max(1) as f64);
        cr.set_source_rgba(
            if dark { 1.0 } else { 0.0 },
            if dark { 1.0 } else { 0.0 },
            if dark { 1.0 } else { 0.0 },
            0.09,
        );
        cr.move_to(x, pad_t);
        cr.line_to(x, pad_t + gh);
        let _ = cr.stroke();
    }
    cr.set_dash(&[], 0.0);

    (pad_l, pad_t, gw, gh)
}

fn draw_rounded_bar(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
) {
    if w <= 0.5 || h <= 0.5 {
        return;
    }
    let r = radius.min(w * 0.5).min(h * 0.5);
    cr.new_sub_path();
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.arc(x + w - r, y + r, r, 1.5 * std::f64::consts::PI, 0.0);
    cr.line_to(x + w, y + h);
    cr.line_to(x, y + h);
    cr.close_path();
    let _ = cr.fill();
}

fn draw_x_axis_labels(
    cr: &gtk4::cairo::Context,
    pad_l: f64,
    base_y: f64,
    gw: f64,
    labels: &[(usize, &str)],
    total_cols: usize,
    date_notes: &[(usize, String)],
) {
    let dark = libadwaita::StyleManager::default().is_dark();
    cr.set_font_size(10.0);
    cr.set_source_rgba(
        if dark { 0.85 } else { 0.35 },
        if dark { 0.85 } else { 0.35 },
        if dark { 0.88 } else { 0.38 },
        0.85,
    );
    for &(col, txt) in labels {
        let x = pad_l + gw * (col as f64 / total_cols.max(1) as f64) + 2.0;
        cr.move_to(x, base_y + 13.0);
        let _ = cr.show_text(txt);
    }
    for (col, txt) in date_notes {
        let x = pad_l + gw * (*col as f64 / total_cols.max(1) as f64) + 2.0;
        cr.move_to(x, base_y + 25.0);
        let _ = cr.show_text(txt);
    }
}

fn draw_battery_level_24h(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    samples: &[BatteryHistorySample],
) {
    let (pad_l, pad_t, gw, gh) = draw_chart_grid(cr, w, h, &["100%", "50%", "0%"], 8);
    let now_ts = now_unix_secs();
    let slots = aggregate_24h_battery_slots(samples, now_ts);
    let col_w = gw / 48.0;
    let bar_w = (col_w * 0.62).max(2.5);

    for (i, slot) in slots.iter().enumerate() {
        let x = pad_l + i as f64 * col_w + (col_w - bar_w) * 0.5;
        if let Some(pct) = slot.level_pct {
            let bh = (gh * (pct as f64 / 100.0)).max(2.0);
            let y = pad_t + gh - bh;
            if slot.charging {
                cr.set_source_rgba(0.20, 0.78, 0.35, 0.20);
                cr.move_to(x + bar_w * 0.5, pad_t);
                cr.line_to(x + bar_w * 0.5, pad_t + gh);
                let _ = cr.stroke();
            }
            cr.set_source_rgba(0.28, 0.80, 0.38, 0.92);
            draw_rounded_bar(cr, x, y, bar_w, bh, 2.0);
        }
    }

    let today_label = format_short_day_month(now_ts);
    draw_x_axis_labels(
        cr,
        pad_l,
        pad_t + gh,
        gw,
        &[
            (0, "12 am"),
            (1, "3"),
            (2, "6"),
            (3, "9"),
            (4, "12 pm"),
            (5, "3"),
            (6, "6"),
            (7, "9"),
        ],
        8,
        &[(0, today_label)],
    );
}

fn draw_screen_on_24h(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    samples: &[BatteryHistorySample],
) {
    let (pad_l, pad_t, gw, gh) = draw_chart_grid(cr, w, h, &["60 min", "30 min", "0 min"], 8);
    let now_ts = now_unix_secs();
    let mins_by_hour = aggregate_24h_screen_mins(samples, now_ts);
    let col_w = gw / 24.0;
    let bar_w = (col_w * 0.60).clamp(4.0, 14.0);

    for (i, &mins) in mins_by_hour.iter().enumerate() {
        if mins > 0.1 {
            let frac = (mins / 60.0).clamp(0.03, 1.0);
            let bh = gh * frac;
            let x = pad_l + i as f64 * col_w + (col_w - bar_w) * 0.5;
            let y = pad_t + gh - bh;
            cr.set_source_rgba(0.08, 0.52, 0.99, 0.94);
            draw_rounded_bar(cr, x, y, bar_w, bh, 3.0);
        }
    }

    let today_label = format_short_day_month(now_ts);
    draw_x_axis_labels(
        cr,
        pad_l,
        pad_t + gh,
        gw,
        &[
            (0, "12 am"),
            (1, "3"),
            (2, "6"),
            (3, "9"),
            (4, "12 pm"),
            (5, "3"),
            (6, "6"),
            (7, "9"),
        ],
        8,
        &[(0, today_label)],
    );
}

fn draw_energy_usage_10d(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    samples: &[BatteryHistorySample],
) {
    let (pad_l, pad_t, gw, gh) = draw_chart_grid(cr, w, h, &["100%", "50%", "0%"], 10);
    let now_ts = now_unix_secs();
    let days = aggregate_10d_slots(samples, now_ts);
    let col_w = gw / 10.0;
    let bar_w = (col_w * 0.62).clamp(8.0, 26.0);

    for (i, d) in days.iter().enumerate() {
        if let Some(pct) = d.avg_energy_pct {
            let bh = (gh * (pct as f64 / 100.0)).max(3.0);
            let x = pad_l + i as f64 * col_w + (col_w - bar_w) * 0.5;
            let y = pad_t + gh - bh;
            cr.set_source_rgba(0.28, 0.80, 0.38, 0.92);
            draw_rounded_bar(cr, x, y, bar_w, bh, 4.0);
        }
    }

    let x_labels: Vec<(usize, &str)> = days
        .iter()
        .enumerate()
        .map(|(i, d)| (i, d.weekday_letter.as_str()))
        .collect();
    let date_notes: Vec<(usize, String)> = days
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.date_caption.clone().map(|c| (i, c)))
        .collect();
    draw_x_axis_labels(cr, pad_l, pad_t + gh, gw, &x_labels, 10, &date_notes);
}

fn draw_screen_on_10d(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    samples: &[BatteryHistorySample],
) {
    let (pad_l, pad_t, gw, gh) =
        draw_chart_grid(cr, w, h, &["4 h", "3 h", "2 h", "1 h", "0 min"], 10);
    let now_ts = now_unix_secs();
    let days = aggregate_10d_slots(samples, now_ts);
    let col_w = gw / 10.0;
    let bar_w = (col_w * 0.62).clamp(8.0, 26.0);

    for (i, d) in days.iter().enumerate() {
        if let Some(hours) = d.screen_on_hours {
            if hours > 0.02 {
                let bh = gh * (hours / 4.0).clamp(0.03, 1.0);
                let x = pad_l + i as f64 * col_w + (col_w - bar_w) * 0.5;
                let y = pad_t + gh - bh;
                cr.set_source_rgba(0.08, 0.52, 0.99, 0.94);
                draw_rounded_bar(cr, x, y, bar_w, bh, 4.0);
            }
        }
    }

    let x_labels: Vec<(usize, &str)> = days
        .iter()
        .enumerate()
        .map(|(i, d)| (i, d.weekday_letter.as_str()))
        .collect();
    let date_notes: Vec<(usize, String)> = days
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.date_caption.clone().map(|c| (i, c)))
        .collect();
    draw_x_axis_labels(cr, pad_l, pad_t + gh, gw, &x_labels, 10, &date_notes);
}

// ============================================================================
// Backend Linux: sysfs / upower / tuned-adm + registro local y parseos testables
// ============================================================================

pub(crate) fn snap_charge_limit_stop(raw: i32) -> u8 {
    let stops = [80i32, 85, 90, 95, 100];
    *stops
        .iter()
        .min_by_key(|&&s| (s - raw).abs())
        .unwrap_or(&100) as u8
}

fn history_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/malus-de/battery-history.csv")
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn format_short_day_month(ts: i64) -> String {
    let Ok(dt) = glib::DateTime::from_unix_local(ts) else {
        return String::new();
    };
    let month = match dt.month() {
        1 => "ene",
        2 => "feb",
        3 => "mar",
        4 => "abr",
        5 => "may",
        6 => "jun",
        7 => "jul",
        8 => "ago",
        9 => "sep",
        10 => "oct",
        11 => "nov",
        _ => "dic",
    };
    format!("{} {month}", dt.day_of_month())
}

fn weekday_initial_es(ts: i64) -> String {
    let Ok(dt) = glib::DateTime::from_unix_local(ts) else {
        return "-".to_string();
    };
    match dt.day_of_week() {
        1 => "L",
        2 => "M",
        3 => "X",
        4 => "J",
        5 => "V",
        6 => "S",
        _ => "D",
    }
    .to_string()
}

pub(crate) fn format_battery_header_status(snap: &BatteryHardwareSnapshot) -> (String, String) {
    if !snap.has_battery {
        return (
            "ac-adapter-symbolic".to_string(),
            "Corriente alterna (sin batería física)".to_string(),
        );
    }
    let rounded = (snap.level_pct / 10).min(10) * 10;
    let charging = snap.status.eq_ignore_ascii_case("Charging")
        || snap.status.eq_ignore_ascii_case("Full");
    let icon = if snap.status.eq_ignore_ascii_case("Charging") {
        format!("battery-level-{rounded}-charging-symbolic")
    } else if charging && snap.ac_online {
        format!("battery-level-{rounded}-plugged-in-symbolic")
    } else {
        format!("battery-level-{rounded}-symbolic")
    };
    (icon, format!("Nivel de batería: {}%", snap.level_pct))
}

fn format_last_charge_summary(
    samples: &[BatteryHistorySample],
    snap: &BatteryHardwareSnapshot,
) -> (String, String) {
    if let Some(last_chg) = samples.iter().rev().find(|s| s.charging) {
        let time_str = glib::DateTime::from_unix_local(last_chg.ts_secs)
            .ok()
            .and_then(|dt| dt.format("%d %b, %I:%M %P").ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Reciente".to_string());
        (
            format!("Última recarga al {}%", last_chg.level_pct),
            time_str,
        )
    } else if snap.has_battery {
        (
            format!("Nivel actual: {}% ({})", snap.level_pct, snap.status),
            "Muestras históricas acumulándose localmente".to_string(),
        )
    } else {
        (
            "Equipo sin batería física detectada".to_string(),
            "Alimentación directa por corriente alterna".to_string(),
        )
    }
}

fn read_sysfs_string(dir: &Path, file: &str) -> Option<String> {
    fs::read_to_string(dir.join(file))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_sysfs_f64(dir: &Path, file: &str) -> Option<f64> {
    read_sysfs_string(dir, file)?.parse::<f64>().ok()
}

pub(crate) fn compute_battery_max_capacity(full: Option<f64>, design: Option<f64>) -> Option<u8> {
    let (f, d) = (full?, design?);
    if d <= 0.0 || f <= 0.0 {
        return None;
    }
    Some(((f / d) * 100.0).round().clamp(1.0, 100.0) as u8)
}

fn collect_battery_snapshot() -> BatteryHardwareSnapshot {
    let mut snap = BatteryHardwareSnapshot {
        condition_label: "Normal".to_string(),
        max_capacity_pct: 100,
        status: "Sin batería".to_string(),
        ..Default::default()
    };

    let mut ac_online = false;
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            let ptype = read_sysfs_string(&dir, "type").unwrap_or_default();
            if ptype == "Mains" || dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("AC")) {
                if read_sysfs_string(&dir, "online").as_deref() == Some("1") {
                    ac_online = true;
                }
            } else if ptype == "Battery" && !snap.has_battery {
                snap.has_battery = true;
                snap.sysfs_path = Some(dir.to_string_lossy().to_string());
                snap.level_pct = read_sysfs_f64(&dir, "capacity")
                    .map(|v| v.round().clamp(0.0, 100.0) as u8)
                    .unwrap_or(0);
                snap.status = read_sysfs_string(&dir, "status").unwrap_or_else(|| "Unknown".into());
                snap.manufacturer = read_sysfs_string(&dir, "manufacturer").unwrap_or_default();
                snap.model_name = read_sysfs_string(&dir, "model_name").unwrap_or_default();
                snap.cycle_count = read_sysfs_string(&dir, "cycle_count").and_then(|s| s.parse().ok());

                let (full_val, design_val) = if let (Some(ef), Some(ed)) = (
                    read_sysfs_f64(&dir, "energy_full"),
                    read_sysfs_f64(&dir, "energy_full_design"),
                ) {
                    snap.full_wh = Some(ef / 1_000_000.0);
                    snap.design_wh = Some(ed / 1_000_000.0);
                    (Some(ef), Some(ed))
                } else {
                    (
                        read_sysfs_f64(&dir, "charge_full"),
                        read_sysfs_f64(&dir, "charge_full_design"),
                    )
                };

                snap.max_capacity_pct = compute_battery_max_capacity(full_val, design_val).unwrap_or(100);
                let health_raw = read_sysfs_string(&dir, "health").unwrap_or_else(|| "Good".into());
                snap.condition_label = if health_raw.eq_ignore_ascii_case("Dead")
                    || snap.max_capacity_pct < 75
                {
                    "Se recomienda revisión".to_string()
                } else {
                    "Normal".to_string()
                };

                let lim_path = dir.join("charge_control_end_threshold");
                if lim_path.exists() {
                    snap.sysfs_charge_limit =
                        read_sysfs_string(&dir, "charge_control_end_threshold").and_then(|s| s.parse().ok());
                    snap.charge_limit_writable = true;
                }
            }
        }
    }
    snap.ac_online = ac_online;

    if !snap.has_battery {
        if let Ok(out) = Command::new("upower").args(["-i", "/org/freedesktop/UPower/devices/DisplayDevice"]).output() {
            if out.status.success() {
                let txt = String::from_utf8_lossy(&out.stdout);
                parse_upower_fallback(&txt, &mut snap);
            }
        }
    }

    if !snap.has_battery {
        snap.condition_label = "Sin batería".to_string();
    }

    if let Ok(out) = Command::new("tuned-adm").arg("active").output() {
        if out.status.success() {
            snap.tuned_profile = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }

    // Registrar muestra viva si hay batería física y cargar historial persistido
    let now_ts = now_unix_secs();
    let mut samples = load_battery_history();
    if snap.has_battery {
        let should_append = samples
            .last()
            .is_none_or(|last| now_ts - last.ts_secs >= 300 || last.level_pct != snap.level_pct);
        if should_append {
            let new_sample = BatteryHistorySample {
                ts_secs: now_ts,
                level_pct: snap.level_pct,
                charging: snap.status.eq_ignore_ascii_case("Charging"),
                screen_on_mins: 5.0,
            };
            append_battery_history(&new_sample);
            samples.push(new_sample);
        }
    }
    snap.samples = samples;
    snap
}

pub(crate) fn parse_upower_fallback(txt: &str, snap: &mut BatteryHardwareSnapshot) {
    for line in txt.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("percentage:") {
            if let Ok(pct) = rest.trim().trim_end_matches('%').parse::<f64>() {
                snap.has_battery = true;
                snap.level_pct = pct.round().clamp(0.0, 100.0) as u8;
                snap.condition_label = "Normal".to_string();
            }
        } else if let Some(rest) = t.strip_prefix("state:") {
            let s = rest.trim();
            if s == "charging" {
                snap.status = "Charging".to_string();
                snap.ac_online = true;
            } else if s == "discharging" {
                snap.status = "Discharging".to_string();
            } else if s == "fully-charged" {
                snap.status = "Full".to_string();
                snap.ac_online = true;
            }
        }
    }
}

fn parse_history_csv_line(line: &str) -> Option<BatteryHistorySample> {
    let mut parts = line.trim().split(',');
    let ts_secs = parts.next()?.parse::<i64>().ok()?;
    let level_pct = parts.next()?.parse::<u8>().ok()?;
    let charging = parts.next()?.parse::<u8>().ok()? != 0;
    let screen_on_mins = parts.next()?.parse::<f64>().ok()?;
    Some(BatteryHistorySample {
        ts_secs,
        level_pct,
        charging,
        screen_on_mins,
    })
}

fn load_battery_history() -> Vec<BatteryHistorySample> {
    let path = history_file_path();
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let cutoff = now_unix_secs() - 10 * 86_400;
    raw.lines()
        .filter_map(parse_history_csv_line)
        .filter(|s| s.ts_secs >= cutoff)
        .collect()
}

fn append_battery_history(sample: &BatteryHistorySample) {
    let path = history_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            f,
            "{},{},{},{:.2}",
            sample.ts_secs,
            sample.level_pct,
            if sample.charging { 1 } else { 0 },
            sample.screen_on_mins
        );
    }
}

pub(crate) fn aggregate_24h_battery_slots(
    samples: &[BatteryHistorySample],
    now_ts: i64,
) -> Vec<HourlySlot> {
    let start_ts = now_ts - 86_400;
    let mut buckets: Vec<Vec<&BatteryHistorySample>> = vec![Vec::new(); 48];
    for s in samples {
        if s.ts_secs >= start_ts && s.ts_secs <= now_ts {
            let idx = (((s.ts_secs - start_ts) / 1800) as usize).min(47);
            buckets[idx].push(s);
        }
    }
    buckets
        .into_iter()
        .map(|b| {
            if b.is_empty() {
                HourlySlot {
                    level_pct: None,
                    charging: false,
                }
            } else {
                let sum: u32 = b.iter().map(|s| s.level_pct as u32).sum();
                let charging = b.iter().any(|s| s.charging);
                HourlySlot {
                    level_pct: Some((sum / b.len() as u32) as u8),
                    charging,
                }
            }
        })
        .collect()
}

pub(crate) fn aggregate_24h_screen_mins(samples: &[BatteryHistorySample], now_ts: i64) -> Vec<f64> {
    let start_ts = now_ts - 86_400;
    let mut mins = vec![0.0; 24];
    for s in samples {
        if s.ts_secs >= start_ts && s.ts_secs <= now_ts {
            let idx = (((s.ts_secs - start_ts) / 3600) as usize).min(23);
            mins[idx] = (mins[idx] + s.screen_on_mins).min(60.0);
        }
    }
    mins
}

pub(crate) fn aggregate_10d_slots(samples: &[BatteryHistorySample], now_ts: i64) -> Vec<DailySlot> {
    let start_day_ts = now_ts - 9 * 86_400;
    let mut buckets: Vec<Vec<&BatteryHistorySample>> = vec![Vec::new(); 10];
    for s in samples {
        if s.ts_secs >= start_day_ts - 43_200 && s.ts_secs <= now_ts {
            let day_idx = ((s.ts_secs - (start_day_ts - 43_200)) / 86_400) as usize;
            if day_idx < 10 {
                buckets[day_idx].push(s);
            }
        }
    }
    (0..10)
        .map(|i| {
            let day_ts = start_day_ts + i as i64 * 86_400;
            let b = &buckets[i];
            let (avg_energy_pct, screen_on_hours) = if b.is_empty() {
                (None, None)
            } else {
                let sum_pct: u32 = b.iter().map(|s| s.level_pct as u32).sum();
                let sum_mins: f64 = b.iter().map(|s| s.screen_on_mins).sum();
                (
                    Some((sum_pct / b.len() as u32) as u8),
                    Some(sum_mins / 60.0),
                )
            };
            DailySlot {
                weekday_letter: weekday_initial_es(day_ts),
                date_caption: if i == 0 || i == 5 {
                    Some(format_short_day_month(day_ts))
                } else {
                    None
                },
                avg_energy_pct,
                screen_on_hours,
            }
        })
        .collect()
}

fn apply_low_power_policy_async(
    sched: LowPowerSchedule,
    has_battery: bool,
    ac_online: bool,
    banner: &Label,
) {
    let should_save_power = match sched {
        LowPowerSchedule::Never => false,
        LowPowerSchedule::Always => true,
        LowPowerSchedule::BatteryOnly => has_battery && !ac_online,
        LowPowerSchedule::PowerAdapterOnly => ac_online,
    };

    let tuned_profile = if should_save_power {
        "powersave"
    } else if has_battery {
        "balanced-battery"
    } else {
        "balanced"
    };

    let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
    std::thread::spawn(move || {
        let out = Command::new("tuned-adm")
            .args(["profile", tuned_profile])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let _ = tx.send_blocking(Ok(()));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                let _ = tx.send_blocking(Err(err));
            }
            Err(e) => {
                let _ = tx.send_blocking(Err(e.to_string()));
            }
        }
    });

    let ban_c = banner.clone();
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(())) => ban_c.set_visible(false),
            Ok(Err(_)) => {
                // La preferencia queda guardada en MalusConfig y battery-settings.toml aunque tuned no esté activo en el entorno.
                ban_c.set_visible(false);
            }
            Err(_) => {}
        }
    });
}

fn write_charge_control_threshold(bat_dir: &str, limit: u8) -> Result<(), String> {
    let clean_dir: String = bat_dir
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
        .collect();
    let script = format!("echo {limit} > '{clean_dir}/charge_control_end_threshold'");
    let out = Command::new("pkexec")
        .args(["sh", "-c", &script])
        .output()
        .map_err(|e| format!("pkexec: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            "Operación cancelada o sin soporte de kernel.".to_string()
        } else {
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_battery_health_and_charge_stops_cleanly() {
        assert_eq!(compute_battery_max_capacity(Some(48_500_000.0), Some(50_000_000.0)), Some(97));
        assert_eq!(compute_battery_max_capacity(Some(50.0), Some(50.0)), Some(100));
        assert_eq!(compute_battery_max_capacity(None, Some(50.0)), None);

        assert_eq!(snap_charge_limit_stop(79), 80);
        assert_eq!(snap_charge_limit_stop(83), 85);
        assert_eq!(snap_charge_limit_stop(93), 95);
        assert_eq!(snap_charge_limit_stop(99), 100);
    }

    #[test]
    fn aggregates_24h_and_10d_battery_samples_cleanly() {
        let now = 1_795_136_000;
        let samples = vec![
            BatteryHistorySample {
                ts_secs: now - 3600,
                level_pct: 64,
                charging: true,
                screen_on_mins: 18.0,
            },
            BatteryHistorySample {
                ts_secs: now - 1800,
                level_pct: 53,
                charging: false,
                screen_on_mins: 12.0,
            },
        ];

        let slots_24 = aggregate_24h_battery_slots(&samples, now);
        assert_eq!(slots_24.len(), 48);
        assert!(slots_24.iter().any(|s| s.level_pct == Some(64) && s.charging));
        assert!(slots_24.iter().any(|s| s.level_pct == Some(53)));

        let mins = aggregate_24h_screen_mins(&samples, now);
        assert_eq!(mins.len(), 24);
        assert!(mins.iter().any(|&m| m >= 12.0));

        let slots_10 = aggregate_10d_slots(&samples, now);
        assert_eq!(slots_10.len(), 10);
        assert_eq!(slots_10[9].avg_energy_pct, Some(58));
    }

    #[test]
    fn parses_upower_fallback_fields() {
        let raw = "  battery\n    state:               charging\n    percentage:          53%\n";
        let mut snap = BatteryHardwareSnapshot::default();
        parse_upower_fallback(raw, &mut snap);
        assert!(snap.has_battery);
        assert_eq!(snap.level_pct, 53);
        assert_eq!(snap.status, "Charging");
        assert!(snap.ac_online);
    }
}
