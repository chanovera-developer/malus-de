use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, MenuButton, Orientation, Popover, Scale, Separator,
};
use malus_de_common::config::{DisplaysConfig, MenuBarConfig};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use crate::services::audio::MprisService;
use crate::services::brightness::BrightnessService;

pub struct MenuBarExtraWidgets {
    pub battery: MenuButton,
    pub focus: MenuButton,
    pub screen_mirroring: MenuButton,
    pub display: MenuButton,
    pub now_playing: MenuButton,
    pub fast_user: MenuButton,
    pub time_machine: MenuButton,
    pub kbd_brightness: MenuButton,
    pub timer: MenuButton,
    pub weather: MenuButton,
    refresh_fn: Rc<dyn Fn()>,
}

impl MenuBarExtraWidgets {
    pub fn refresh(&self, cfg: &MenuBarConfig) {
        let _ = cfg;
        (self.refresh_fn)();
    }

    pub fn append_to(&self, tray: &GtkBox) {
        tray.append(&self.timer);
        tray.append(&self.now_playing);
        tray.append(&self.focus);
        tray.append(&self.screen_mirroring);
        tray.append(&self.display);
        tray.append(&self.kbd_brightness);
        tray.append(&self.time_machine);
        tray.append(&self.weather);
        tray.append(&self.battery);
        tray.append(&self.fast_user);
    }
}

fn make_extra_menu_button(tooltip: &str, width: i32) -> (MenuButton, Popover, GtkBox) {
    let menu_btn = MenuButton::builder()
        .css_classes(["network-pill", "topbar-btn"])
        .tooltip_text(tooltip)
        .has_frame(false)
        .always_show_arrow(false)
        .build();

    let popover = Popover::builder()
        .css_classes(["control-center-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(width)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(8)
        .build();

    popover.set_child(Some(&pop_box));
    menu_btn.set_popover(Some(&popover));
    (menu_btn, popover, pop_box)
}

fn launch_settings_panel(panel: &str) {
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("malus-de-settings"));
    let mut cmd = Command::new(exe);
    malus_de_common::config::set_cursor_environment(&mut cmd);
    let _ = cmd
        .args(["--panel", panel])
        .env("WAYLAND_DISPLAY", wayland_display)
        .env("GDK_BACKEND", "wayland")
        .env("GSK_RENDERER", "cairo")
        .spawn();
}

pub fn read_system_battery() -> Option<(u8, String)> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("BAT") {
            continue;
        }
        let path = entry.path();
        let cap_raw = std::fs::read_to_string(path.join("capacity")).ok()?;
        let cap = cap_raw.trim().parse::<u8>().ok()?.clamp(0, 100);
        let status = std::fs::read_to_string(path.join("status"))
            .unwrap_or_default()
            .trim()
            .to_string();
        return Some((cap, status));
    }
    None
}

pub fn battery_icon_name(pct: u8, charging: bool) -> &'static str {
    let bucket = ((pct as f64 / 10.0).round() as u8 * 10).clamp(0, 100);
    match (bucket, charging) {
        (0..=5, false) => "battery-level-0-symbolic",
        (0..=5, true) => "battery-level-0-charging-symbolic",
        (6..=15, false) => "battery-level-10-symbolic",
        (6..=15, true) => "battery-level-10-charging-symbolic",
        (16..=25, false) => "battery-level-20-symbolic",
        (16..=25, true) => "battery-level-20-charging-symbolic",
        (26..=35, false) => "battery-level-30-symbolic",
        (26..=35, true) => "battery-level-30-charging-symbolic",
        (36..=45, false) => "battery-level-40-symbolic",
        (36..=45, true) => "battery-level-40-charging-symbolic",
        (46..=55, false) => "battery-level-50-symbolic",
        (46..=55, true) => "battery-level-50-charging-symbolic",
        (56..=65, false) => "battery-level-60-symbolic",
        (56..=65, true) => "battery-level-60-charging-symbolic",
        (66..=75, false) => "battery-level-70-symbolic",
        (66..=75, true) => "battery-level-70-charging-symbolic",
        (76..=85, false) => "battery-level-80-symbolic",
        (76..=85, true) => "battery-level-80-charging-symbolic",
        (86..=95, false) => "battery-level-90-symbolic",
        (86..=95, true) => "battery-level-90-charging-symbolic",
        (_, false) => "battery-level-100-symbolic",
        (_, true) => "battery-level-100-charging-symbolic",
    }
}

fn notifications_gsettings() -> Option<gtk4::gio::Settings> {
    let source = gtk4::gio::SettingsSchemaSource::default()?;
    source.lookup("org.gnome.desktop.notifications", true)?;
    Some(gtk4::gio::Settings::new("org.gnome.desktop.notifications"))
}

fn is_focus_active() -> bool {
    let focus_cfg = malus_de_common::config::FocusConfig::load();
    if focus_cfg.is_active_now() {
        return true;
    }
    if let Some(settings) = notifications_gsettings() {
        return !settings.boolean("show-banners");
    }
    false
}

fn active_focus_status_text() -> String {
    let focus_cfg = malus_de_common::config::FocusConfig::load();
    if let Some(mode) = focus_cfg.active_mode_now() {
        return format!("{} activado", mode.name);
    }
    if is_focus_active() {
        "No molestar activado".to_string()
    } else {
        "Enfoque desactivado".to_string()
    }
}

fn set_focus_active(active: bool) {
    let mut focus_cfg = malus_de_common::config::FocusConfig::load();
    focus_cfg.active_focus_id = if active {
        Some("do-not-disturb".to_string())
    } else {
        None
    };
    let _ = focus_cfg.save();

    if let Some(settings) = notifications_gsettings() {
        let _ = settings.set_boolean("show-banners", !active);
    }
}

fn resolve_user_names() -> (String, String) {
    let acct = std::env::var("USER").unwrap_or_else(|_| "usuario".to_string());
    if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let mut parts = line.split(':');
            if parts.next() == Some(acct.as_str()) {
                let gecos = parts.nth(3).unwrap_or("").split(',').next().unwrap_or("").trim();
                if !gecos.is_empty() {
                    return (gecos.to_string(), acct);
                }
            }
        }
    }
    let mut chars = acct.chars();
    let cap = match chars.next() {
        Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
        None => acct.clone(),
    };
    (cap, acct)
}

fn find_kbd_backlight_dir() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/leds").ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.contains("kbd_backlight") {
            return Some(e.path());
        }
    }
    None
}

pub fn create_menubar_extra_widgets() -> MenuBarExtraWidgets {
    // 1. Batería
    let (bat_btn, bat_pop, bat_box) = make_extra_menu_button("Batería", 240);
    let bat_pill = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .valign(Align::Center)
        .build();
    let bat_pct_lbl = Label::builder().css_classes(["cc-tile-subtitle"]).build();
    let bat_icon = Image::from_icon_name("battery-level-100-symbolic");
    bat_pill.append(&bat_pct_lbl);
    bat_pill.append(&bat_icon);
    bat_btn.set_child(Some(&bat_pill));

    let bat_pop_title = Label::builder()
        .label("Batería")
        .css_classes(["cc-tile-title"])
        .xalign(0.0)
        .build();
    let bat_pop_status = Label::builder()
        .label("Fuente: Adaptador / Batería")
        .css_classes(["cc-tile-subtitle"])
        .xalign(0.0)
        .build();
    let bat_settings_btn = Button::builder()
        .label("Configuración de batería...")
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();
    let bp_c = bat_pop.clone();
    bat_settings_btn.connect_clicked(move |_| {
        bp_c.popdown();
        launch_settings_panel("battery");
    });
    bat_box.append(&bat_pop_title);
    bat_box.append(&bat_pop_status);
    bat_box.append(&Separator::new(Orientation::Horizontal));
    bat_box.append(&bat_settings_btn);

    // 2. Enfoque
    let (focus_btn, focus_pop, focus_box) = make_extra_menu_button("Enfoque (No molestar)", 230);
    let focus_icon = Image::from_icon_name("weather-clear-night-symbolic");
    focus_btn.set_child(Some(&focus_icon));
    let focus_status_lbl = Label::builder()
        .label("Enfoque desactivado")
        .css_classes(["cc-tile-title"])
        .xalign(0.0)
        .build();
    let focus_toggle_btn = Button::builder()
        .label("Activar / Desactivar No molestar")
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();
    let focus_settings_btn = Button::builder()
        .label("Configuración de enfoque...")
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();
    {
        let fp_s = focus_pop.clone();
        focus_settings_btn.connect_clicked(move |_| {
            fp_s.popdown();
            launch_settings_panel("focus");
        });
    }
    focus_box.append(&focus_status_lbl);
    focus_box.append(&Separator::new(Orientation::Horizontal));
    focus_box.append(&focus_toggle_btn);
    focus_box.append(&focus_settings_btn);

    // 3. Duplicar pantalla / Pantallas
    let (mirror_btn, mirror_pop, mirror_box) = make_extra_menu_button("Duplicar pantalla", 240);
    mirror_btn.set_child(Some(&Image::from_icon_name("video-display-symbolic")));
    let mirror_info = Label::builder()
        .label("Pantallas conectadas")
        .css_classes(["cc-tile-subtitle"])
        .wrap(true)
        .xalign(0.0)
        .build();
    let mirror_open_btn = Button::builder()
        .label("Configuración de pantallas...")
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();
    let mp_c = mirror_pop;
    mirror_open_btn.connect_clicked(move |_| {
        mp_c.popdown();
        launch_settings_panel("displays");
    });
    mirror_box.append(&Label::builder().label("Duplicar pantalla").css_classes(["cc-tile-title"]).xalign(0.0).build());
    mirror_box.append(&mirror_info);
    mirror_box.append(&Separator::new(Orientation::Horizontal));
    mirror_box.append(&mirror_open_btn);

    // 4. Pantalla (Brillo)
    let (disp_btn, disp_pop, disp_box) = make_extra_menu_button("Brillo de pantalla", 250);
    disp_btn.set_child(Some(&Image::from_icon_name("display-brightness-symbolic")));
    let disp_pct_lbl = Label::builder().label("80%").css_classes(["cc-tile-subtitle"]).build();
    let disp_hdr = GtkBox::builder().orientation(Orientation::Horizontal).spacing(8).build();
    disp_hdr.append(&Label::builder().label("Pantalla").css_classes(["cc-tile-title"]).hexpand(true).xalign(0.0).build());
    disp_hdr.append(&disp_pct_lbl);
    let disp_scale = Scale::with_range(Orientation::Horizontal, 5.0, 100.0, 1.0);
    disp_scale.set_draw_value(false);
    disp_scale.set_value(DisplaysConfig::load().brightness.clamp(5.0, 100.0));
    let disp_lock = Rc::new(Cell::new(false));
    {
        let dl = disp_lock.clone();
        let dpl = disp_pct_lbl.clone();
        disp_scale.connect_value_changed(move |sc| {
            if dl.get() {
                return;
            }
            let v = sc.value();
            dpl.set_label(&format!("{}%", v.round() as i32));
            BrightnessService::set_brightness(v);
        });
    }
    let disp_open_btn = Button::builder()
        .label("Configuración de pantallas...")
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .build();
    let dp_c = disp_pop.clone();
    disp_open_btn.connect_clicked(move |_| {
        dp_c.popdown();
        launch_settings_panel("displays");
    });
    disp_box.append(&disp_hdr);
    disp_box.append(&disp_scale);
    disp_box.append(&Separator::new(Orientation::Horizontal));
    disp_box.append(&disp_open_btn);
    {
        let dl = disp_lock;
        let ds = disp_scale;
        let dpl = disp_pct_lbl;
        disp_pop.connect_show(move |_| {
            let b = DisplaysConfig::load().brightness.clamp(5.0, 100.0);
            dl.set(true);
            ds.set_value(b);
            dpl.set_label(&format!("{}%", b.round() as i32));
            dl.set(false);
        });
    }

    // 5. Ahora suena (MPRIS)
    let (np_btn, _np_pop, np_box) = make_extra_menu_button("Ahora suena", 250);
    np_btn.set_child(Some(&Image::from_icon_name("multimedia-player-symbolic")));
    let np_title = Label::builder().label("Sin reproducción activa").css_classes(["cc-tile-title"]).xalign(0.0).wrap(true).build();
    let np_artist = Label::builder().label("").css_classes(["cc-tile-subtitle"]).xalign(0.0).wrap(true).build();
    let np_ctrl_row = GtkBox::builder().orientation(Orientation::Horizontal).spacing(6).halign(Align::Center).build();
    let np_prev = Button::builder().icon_name("media-skip-backward-symbolic").has_frame(false).css_classes(["topbar-btn"]).build();
    let np_play = Button::builder().icon_name("media-playback-start-symbolic").has_frame(false).css_classes(["topbar-btn"]).build();
    let np_next = Button::builder().icon_name("media-skip-forward-symbolic").has_frame(false).css_classes(["topbar-btn"]).build();
    let np_bus: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    {
        let b = np_bus.clone();
        np_prev.connect_clicked(move |_| {
            if let Some(bus) = b.borrow().clone() {
                glib::MainContext::default().spawn_local(async move { MprisService::previous(&bus).await; });
            }
        });
    }
    {
        let b = np_bus.clone();
        np_play.connect_clicked(move |_| {
            if let Some(bus) = b.borrow().clone() {
                glib::MainContext::default().spawn_local(async move { MprisService::play_pause(&bus).await; });
            }
        });
    }
    {
        let b = np_bus.clone();
        np_next.connect_clicked(move |_| {
            if let Some(bus) = b.borrow().clone() {
                glib::MainContext::default().spawn_local(async move { MprisService::next(&bus).await; });
            }
        });
    }
    np_ctrl_row.append(&np_prev);
    np_ctrl_row.append(&np_play);
    np_ctrl_row.append(&np_next);
    np_box.append(&np_title);
    np_box.append(&np_artist);
    np_box.append(&np_ctrl_row);

    // 6. Cambio rápido de usuario
    let (user_btn, user_pop, user_box) = make_extra_menu_button("Cambio rápido de usuario", 230);
    let user_row = GtkBox::builder().orientation(Orientation::Horizontal).spacing(5).valign(Align::Center).build();
    user_row.append(&Image::from_icon_name("avatar-default-symbolic"));
    let user_lbl = Label::builder().css_classes(["cc-tile-subtitle"]).build();
    user_row.append(&user_lbl);
    user_btn.set_child(Some(&user_row));
    let (full_name, acct_name) = resolve_user_names();
    let lock_btn = Button::builder().label("Bloquear pantalla").css_classes(["popover-menu-item"]).has_frame(false).build();
    let up_c = user_pop;
    lock_btn.connect_clicked(move |_| {
        up_c.popdown();
        let _ = Command::new("malus-de-session-lock").spawn();
    });
    user_box.append(&Label::builder().label(&full_name).css_classes(["cc-tile-title"]).xalign(0.0).build());
    user_box.append(&Label::builder().label(&format!("Cuenta: {acct_name}")).css_classes(["cc-tile-subtitle"]).xalign(0.0).build());
    user_box.append(&Separator::new(Orientation::Horizontal));
    user_box.append(&lock_btn);

    // 7. Time Machine
    let (tm_btn, tm_pop, tm_box) = make_extra_menu_button("Time Machine", 230);
    tm_btn.set_child(Some(&Image::from_icon_name("document-open-recent-symbolic")));
    let tm_open_btn = Button::builder().label("Abrir configuración de Time Machine...").css_classes(["popover-menu-item"]).has_frame(false).build();
    let tmp_c = tm_pop;
    tm_open_btn.connect_clicked(move |_| {
        tmp_c.popdown();
        launch_settings_panel("general-timemachine");
    });
    tm_box.append(&Label::builder().label("Time Machine").css_classes(["cc-tile-title"]).xalign(0.0).build());
    tm_box.append(&Label::builder().label("Copias de seguridad locales del sistema").css_classes(["cc-tile-subtitle"]).wrap(true).xalign(0.0).build());
    tm_box.append(&Separator::new(Orientation::Horizontal));
    tm_box.append(&tm_open_btn);

    // 8. Brillo del teclado
    let (kbd_btn, _kbd_pop, kbd_box) = make_extra_menu_button("Brillo del teclado", 240);
    kbd_btn.set_child(Some(&Image::from_icon_name("keyboard-brightness-symbolic")));
    let kbd_dir = find_kbd_backlight_dir();
    kbd_box.append(&Label::builder().label("Brillo del teclado").css_classes(["cc-tile-title"]).xalign(0.0).build());
    if let Some(dir) = kbd_dir {
        let max_b = std::fs::read_to_string(dir.join("max_brightness")).ok().and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(100.0).max(1.0);
        let cur_b = std::fs::read_to_string(dir.join("brightness")).ok().and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
        let ks = Scale::with_range(Orientation::Horizontal, 0.0, max_b, 1.0);
        ks.set_draw_value(false);
        ks.set_value(cur_b.clamp(0.0, max_b));
        ks.connect_value_changed(move |sc| {
            let _ = std::fs::write(dir.join("brightness"), format!("{}", sc.value().round() as u32));
        });
        kbd_box.append(&ks);
    } else {
        kbd_box.append(&Label::builder().label("Retroiluminación de teclado no detectada en este equipo.").css_classes(["cc-tile-subtitle"]).wrap(true).xalign(0.0).build());
    }

    // 9. Temporizador
    let (timer_btn, _timer_pop, timer_box) = make_extra_menu_button("Temporizador", 230);
    let timer_pill = GtkBox::builder().orientation(Orientation::Horizontal).spacing(4).valign(Align::Center).build();
    timer_pill.append(&Image::from_icon_name("alarm-symbolic"));
    let timer_lbl = Label::builder().css_classes(["cc-tile-subtitle"]).visible(false).build();
    timer_pill.append(&timer_lbl);
    timer_btn.set_child(Some(&timer_pill));

    let timer_rem = Rc::new(Cell::new(0u32));
    let timer_running = Rc::new(Cell::new(false));
    let timer_source_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let timer_status = Label::builder().label("Sin temporizador activo").css_classes(["cc-tile-title"]).xalign(0.0).build();
    timer_box.append(&timer_status);

    let start_countdown_timer: Rc<dyn Fn()> = {
        let tr = timer_rem.clone();
        let run = timer_running.clone();
        let tl = timer_lbl.clone();
        let ts = timer_status.clone();
        let tb = timer_btn.clone();
        let src_slot = timer_source_id.clone();
        Rc::new(move || {
            if src_slot.borrow().is_some() {
                return;
            }
            let tr = tr.clone();
            let run = run.clone();
            let tl = tl.clone();
            let ts = ts.clone();
            let tb = tb.clone();
            let src_clear = src_slot.clone();
            let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
                if !run.get() {
                    let _ = src_clear.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
                let left = tr.get();
                if left > 0 {
                    let next = left - 1;
                    tr.set(next);
                    let txt = format!("{:02}:{:02}", next / 60, next % 60);
                    tl.set_label(&txt);
                    tl.set_visible(true);
                    ts.set_label(&format!("Restante: {txt}"));
                    glib::ControlFlow::Continue
                } else {
                    run.set(false);
                    tl.set_visible(false);
                    ts.set_label("¡Tiempo finalizado!");
                    let c = MenuBarConfig::load();
                    tb.set_visible(c.timer && c.timer_mode == 0);
                    let _ = src_clear.borrow_mut().take();
                    glib::ControlFlow::Break
                }
            });
            *src_slot.borrow_mut() = Some(id);
        })
    };

    let preset_row = GtkBox::builder().orientation(Orientation::Horizontal).spacing(6).homogeneous(true).build();
    for mins in [1u32, 5, 10, 15] {
        let pb = Button::builder().label(&format!("{mins}m")).css_classes(["cc-pill-btn"]).has_frame(false).build();
        let tr = timer_rem.clone();
        let run = timer_running.clone();
        let tl = timer_lbl.clone();
        let ts = timer_status.clone();
        let start_cb = start_countdown_timer.clone();
        pb.connect_clicked(move |_| {
            tr.set(mins * 60);
            run.set(true);
            let txt = format!("{mins:02}:00");
            tl.set_label(&txt);
            tl.set_visible(true);
            ts.set_label(&format!("Restante: {txt}"));
            start_cb();
        });
        preset_row.append(&pb);
    }
    let stop_btn = Button::builder().label("Detener / Limpiar").css_classes(["popover-menu-item"]).has_frame(false).build();
    {
        let tr = timer_rem.clone();
        let run = timer_running.clone();
        let tl = timer_lbl.clone();
        let ts = timer_status.clone();
        let src_slot = timer_source_id.clone();
        stop_btn.connect_clicked(move |_| {
            tr.set(0);
            run.set(false);
            if let Some(id) = src_slot.borrow_mut().take() {
                id.remove();
            }
            tl.set_visible(false);
            ts.set_label("Sin temporizador activo");
        });
    }
    timer_box.append(&preset_row);
    timer_box.append(&stop_btn);

    // 10. Clima
    let (weather_btn, _w_pop, w_box) = make_extra_menu_button("Clima", 220);
    weather_btn.set_child(Some(&Image::from_icon_name("weather-few-clouds-symbolic")));
    w_box.append(&Label::builder().label("Clima").css_classes(["cc-tile-title"]).xalign(0.0).build());
    w_box.append(&Label::builder().label("Pronóstico local integrado en la barra de menús.").css_classes(["cc-tile-subtitle"]).wrap(true).xalign(0.0).build());

    let np_active = Rc::new(Cell::new(false));

    let refresh_fn: Rc<dyn Fn()> = {
        let bat_btn_c = bat_btn.clone();
        let bat_pct_c = bat_pct_lbl.clone();
        let bat_ico_c = bat_icon.clone();
        let bat_stat_c = bat_pop_status.clone();
        let focus_btn_c = focus_btn.clone();
        let focus_stat_c = focus_status_lbl.clone();
        let mirror_btn_c = mirror_btn.clone();
        let mirror_info_c = mirror_info.clone();
        let disp_btn_c = disp_btn.clone();
        let np_btn_c = np_btn.clone();
        let np_act_c = np_active.clone();
        let np_t_c = np_title.clone();
        let np_a_c = np_artist.clone();
        let np_p_c = np_play.clone();
        let np_bus_c = np_bus;
        let user_btn_c = user_btn.clone();
        let user_lbl_c = user_lbl.clone();
        let full_c = full_name;
        let acct_c = acct_name;
        let tm_btn_c = tm_btn.clone();
        let kbd_btn_c = kbd_btn.clone();
        let timer_btn_c = timer_btn.clone();
        let timer_run_c = timer_running.clone();
        let weather_btn_c = weather_btn.clone();

        Rc::new(move || {
            let cfg = MenuBarConfig::load();
            if let Some((pct, status)) = read_system_battery() {
                let charging = status.eq_ignore_ascii_case("Charging");
                bat_ico_c.set_icon_name(Some(battery_icon_name(pct, charging)));
                bat_pct_c.set_label(&format!("{pct}%"));
                bat_pct_c.set_visible(cfg.battery_show_percentage);
                bat_stat_c.set_label(&format!("{pct}% · {}", if charging { "Cargando" } else { &status }));
                bat_btn_c.set_visible(cfg.battery);
            } else {
                bat_btn_c.set_visible(false);
            }

            let focused = is_focus_active();
            focus_stat_c.set_label(&active_focus_status_text());
            focus_btn_c.set_visible(cfg.focus && (cfg.focus_mode == 0 || (cfg.focus_mode == 1 && focused)));

            let dcfg = DisplaysConfig::load();
            let outs = dcfg.outputs.len().max(1);
            mirror_info_c.set_label(&format!("{outs} pantalla(s) configurada(s)"));
            mirror_btn_c.set_visible(cfg.screen_mirroring && (cfg.screen_mirroring_mode == 0 || outs > 1));
            disp_btn_c.set_visible(cfg.display && (cfg.display_mode == 0 || dcfg.brightness < 99.5));

            np_btn_c.set_visible(cfg.now_playing && (cfg.now_playing_mode == 0 || np_act_c.get()));

            user_btn_c.set_visible(cfg.fast_user_switching);
            match cfg.fast_user_switching_mode {
                0 => {
                    user_lbl_c.set_label(&full_c);
                    user_lbl_c.set_visible(true);
                }
                1 => {
                    user_lbl_c.set_label(&acct_c);
                    user_lbl_c.set_visible(true);
                }
                _ => user_lbl_c.set_visible(false),
            }

            tm_btn_c.set_visible(cfg.time_machine);
            kbd_btn_c.set_visible(cfg.keyboard_brightness);
            timer_btn_c.set_visible(cfg.timer && (cfg.timer_mode == 0 || timer_run_c.get()));
            weather_btn_c.set_visible(cfg.weather);

            if cfg.now_playing {
                let np_b = np_btn_c.clone();
                let np_a = np_act_c.clone();
                let npt = np_t_c.clone();
                let npa = np_a_c.clone();
                let npp = np_p_c.clone();
                let npbus = np_bus_c.clone();
                let np_mode = cfg.now_playing_mode;
                glib::MainContext::default().spawn_local(async move {
                    let media = MprisService::get_current_media().await;
                    if let Some(m) = media {
                        np_a.set(true);
                        npt.set_label(&m.title);
                        npa.set_label(&m.artist);
                        npp.set_icon_name(if m.is_playing {
                            "media-playback-pause-symbolic"
                        } else {
                            "media-playback-start-symbolic"
                        });
                        *npbus.borrow_mut() = Some(m.player_bus);
                    } else {
                        np_a.set(false);
                        npt.set_label("Sin reproducción activa");
                        npa.set_label("");
                        npp.set_icon_name("media-playback-start-symbolic");
                        *npbus.borrow_mut() = None;
                    }
                    np_b.set_visible(np_mode == 0 || np_a.get());
                });
            }
        })
    };

    {
        let rf = refresh_fn.clone();
        let fp = focus_pop;
        focus_toggle_btn.connect_clicked(move |_| {
            set_focus_active(!is_focus_active());
            fp.popdown();
            rf();
        });
    }

    refresh_fn();
    {
        let rf = refresh_fn.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(15), move || {
            rf();
            glib::ControlFlow::Continue
        });
    }

    MenuBarExtraWidgets {
        battery: bat_btn,
        focus: focus_btn,
        screen_mirroring: mirror_btn,
        display: disp_btn,
        now_playing: np_btn,
        fast_user: user_btn,
        time_machine: tm_btn,
        kbd_brightness: kbd_btn,
        timer: timer_btn,
        weather: weather_btn,
        refresh_fn,
    }
}

