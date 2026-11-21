use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, MenuButton, Orientation, PolicyType, Popover,
    Scale, ScrolledWindow, Separator,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use tracing::info;

use crate::services::audio::AudioService;
use crate::services::bluetooth::BluetoothService;
use crate::services::network::{NetworkService, WifiApInfo};

thread_local! {
    static TRAY_VISIBILITY_REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

pub fn refresh_tray_visibility() {
    TRAY_VISIBILITY_REFRESH.with(|cell| {
        if let Some(ref f) = *cell.borrow() {
            f();
        }
    });
}

pub fn create_status_tray_widget() -> GtkBox {
    let tray = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();

    // 0. Indicador y selector rápido de Teclado (LAT / ES / US)
    let kb_widget = create_keyboard_tray_button();
    tray.append(&kb_widget);

    // 1. Malus Link / AirDrop Menu Extra
    let link_widget = create_link_tray_button();
    tray.append(&link_widget);

    // 2. Bluetooth Menu Extra (Popover con dispositivos y switch)
    let bt_widget = create_bluetooth_tray_button();
    tray.append(&bt_widget);

    // 3. Wi-Fi / Red Menu Extra (Popover con redes y switch)
    let wifi_widget = create_wifi_tray_button();
    tray.append(&wifi_widget);

    // 4. Sonido / Volumen Menu Extra (Popover con slider y salida de audio)
    let vol_widget = create_volume_tray_button();
    tray.append(&vol_widget);

    // 5. Spotlight Search (Lupa macOS)
    let spotlight_btn = create_spotlight_button();
    tray.append(&spotlight_btn);

    // 6. Módulos adicionales configurables desde Barra de menús (Batería, Enfoque, Ahora suena, Pantalla, Usuario, Clima, Temporizador...)
    let extras = super::menubar_extras::create_menubar_extra_widgets();
    extras.append_to(&tray);

    // 7. Centro de Control macOS Unificado (Esquina derecha junto al Reloj)
    let cc_widget = super::control_center::create_control_center_widget();
    tray.append(&cc_widget);

    let apply_vis: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = malus_de_common::config::MenuBarConfig::load();
        kb_widget.set_visible(cfg.input_source);
        link_widget.set_visible(cfg.airdrop);
        bt_widget.set_visible(cfg.bluetooth);
        wifi_widget.set_visible(cfg.wifi);
        vol_widget.set_visible(cfg.sound);
        spotlight_btn.set_visible(cfg.spotlight);
        super::power_profile::refresh_power_profile_visibility();
        super::auto_tiling::refresh_auto_tiling_visibility();
        extras.refresh(&cfg);
    });
    apply_vis();
    let apply_poll = apply_vis.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        apply_poll();
        glib::ControlFlow::Continue
    });
    TRAY_VISIBILITY_REFRESH.with(|cell| {
        *cell.borrow_mut() = Some(apply_vis);
    });

    tray
}

fn create_keyboard_tray_button() -> Button {
    let input_cfg = malus_de_common::config::InputConfig::load();
    let btn = Button::builder()
        .css_classes(["network-pill", "topbar-btn"])
        .label(&format!("⌨ {}", input_cfg.layout_short_badge()))
        .tooltip_text(&format!(
            "Teclado: {} (Clic para alternar LATAM / ES / US)",
            input_cfg.layout_display_name()
        ))
        .has_frame(false)
        .build();

    let btn_click = btn.clone();
    btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::InputConfig::load();
        let next_layout = match cfg.keyboard_layout.as_str() {
            "latam" => "es",
            "es" => "us",
            _ => "latam",
        };
        cfg.keyboard_layout = next_layout.to_string();
        cfg.keyboard_variant = String::new();
        let _ = cfg.save();
        let _ = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::SetKeyboardLayout {
                layout: next_layout.to_string(),
                variant: String::new(),
            },
        );
        btn_click.set_label(&format!("⌨ {}", cfg.layout_short_badge()));
        btn_click.set_tooltip_text(Some(&format!(
            "Teclado: {} (Clic para alternar LATAM / ES / US)",
            cfg.layout_display_name()
        )));
    });

    let btn_timer = btn.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let cfg = malus_de_common::config::InputConfig::load();
        let expected = format!("⌨ {}", cfg.layout_short_badge());
        if btn_timer.label().as_deref() != Some(expected.as_str()) {
            btn_timer.set_label(&expected);
            btn_timer.set_tooltip_text(Some(&format!(
                "Teclado: {} (Clic para alternar LATAM / ES / US)",
                cfg.layout_display_name()
            )));
        }
        glib::ControlFlow::Continue
    });

    btn
}

// ==========================================
// 1. Wi-Fi Menu Extra
// ==========================================

fn create_wifi_tray_button() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["network-pill", "topbar-btn"])
        .tooltip_text("Wi-Fi")
        .has_frame(false)
        .always_show_arrow(false)
        .build();

    let icon = Image::from_icon_name("network-wireless-symbolic");
    menu_btn.set_child(Some(&icon));

    let popover = Popover::builder()
        .css_classes(["control-center-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(270)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    // Header: Título + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let title_lbl = Label::builder()
        .label("Wi-Fi")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let wifi_switch = Switch::builder().valign(Align::Center).build();

    header.append(&title_lbl);
    header.append(&*wifi_switch);
    popover_box.append(&header);

    // Scrolled list de redes
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(160)
        .max_content_height(230)
        .propagate_natural_height(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    scrolled.set_child(Some(&content_box));
    popover_box.append(&scrolled);

    // Footer: Separador y Preferencias de Wi-Fi
    let sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Preferencias de Wi-Fi...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let settings_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
        let mut command = Command::new(settings_bin);
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.args(["--panel", "wifi"]).spawn();
    });

    popover_box.append(&sep);
    popover_box.append(&pref_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    // Conectar Switch
    let is_updating_switch = Rc::new(RefCell::new(false));
    let is_upd_sw = is_updating_switch.clone();
    let cb_for_sw = content_box.clone();
    let sw_for_sw = wifi_switch.clone();
    let po_for_sw = popover.clone();
    let icon_sw = icon.clone();

    wifi_switch.connect_state_set(move |_sw, state| {
        if *is_upd_sw.borrow() {
            return glib::Propagation::Proceed;
        }
        info!("Tray Wi-Fi switch cambiado a: {}", state);

        let cb_inner = cb_for_sw.clone();
        let sw_inner = sw_for_sw.clone();
        let upd_inner = is_upd_sw.clone();
        let po_inner = po_for_sw.clone();
        let ico_inner = icon_sw.clone();

        glib::MainContext::default().spawn_local(async move {
            let _ = NetworkService::set_wifi_enabled(state).await;
            update_wifi_tray_icon(&ico_inner).await;
            refresh_wifi_tray_list(&cb_inner, &sw_inner, &upd_inner, &po_inner, &ico_inner).await;
        });

        glib::Propagation::Proceed
    });

    // Actualizar al abrir el Popover
    let cb_pop = content_box.clone();
    let sw_pop = wifi_switch.clone();
    let upd_pop = is_updating_switch.clone();
    let po_pop = popover.clone();
    let ico_pop = icon.clone();

    popover.connect_show(move |_| {
        let cb = cb_pop.clone();
        let sw = sw_pop.clone();
        let upd = upd_pop.clone();
        let po = po_pop.clone();
        let ico = ico_pop.clone();

        glib::MainContext::default().spawn_local(async move {
            update_wifi_tray_icon(&ico).await;
            refresh_wifi_tray_list(&cb, &sw, &upd, &po, &ico).await;
        });
    });

    // Actualización inicial del icono
    let ico_init = icon.clone();
    glib::MainContext::default().spawn_local(async move {
        update_wifi_tray_icon(&ico_init).await;
    });

    menu_btn
}

async fn refresh_wifi_tray_list(
    content_box: &GtkBox,
    wifi_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    popover: &Popover,
    tray_icon: &Image,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    let (enabled, networks) = NetworkService::scan_networks().await;

    *is_upd_sw.borrow_mut() = true;
    wifi_switch.set_state(enabled);
    *is_upd_sw.borrow_mut() = false;

    if !enabled {
        let disabled_lbl = Label::builder()
            .label("El Wi-Fi está desactivado")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(30)
            .margin_bottom(30)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    // Red conectada actualmente
    if let Some(active) = networks.iter().find(|n| n.in_use) {
        let active_header = Label::builder()
            .label("RED ACTUAL")
            .css_classes(["cc-section-label"])
            .xalign(0.0)
            .margin_start(4)
            .margin_top(2)
            .build();
        content_box.append(&active_header);

        let active_card = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["cc-list-row", "cc-list-row-active"])
            .spacing(8)
            .build();

        let sig_icon = Image::builder()
            .icon_name(get_wifi_signal_icon(active.signal))
            .pixel_size(16)
            .build();

        let ssid_lbl = Label::builder()
            .label(&active.ssid)
            .css_classes(["cc-item-title"])
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let check_icon = Image::builder()
            .icon_name("object-select-symbolic")
            .css_classes(["cc-item-check"])
            .pixel_size(14)
            .build();

        active_card.append(&sig_icon);
        active_card.append(&ssid_lbl);
        active_card.append(&check_icon);
        content_box.append(&active_card);
    }

    // Otras redes disponibles
    let other_networks: Vec<&WifiApInfo> = networks.iter().filter(|n| !n.in_use).collect();
    if other_networks.is_empty() {
        let empty_lbl = Label::builder()
            .label("No se detectaron otras redes")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .build();
        content_box.append(&empty_lbl);
    } else {
        let others_lbl = Label::builder()
            .label("OTRAS REDES")
            .css_classes(["cc-section-label"])
            .xalign(0.0)
            .margin_start(4)
            .margin_top(6)
            .build();
        content_box.append(&others_lbl);

        for net in other_networks {
            let row_btn = Button::builder()
                .css_classes(["cc-list-row"])
                .has_frame(false)
                .build();

            let row_content = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .build();

            let sig_icon = Image::builder()
                .icon_name(get_wifi_signal_icon(net.signal))
                .pixel_size(16)
                .build();

            let ssid_lbl = Label::builder()
                .label(&net.ssid)
                .css_classes(["cc-item-title"])
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();

            row_content.append(&sig_icon);
            row_content.append(&ssid_lbl);

            if net.is_encrypted {
                let lock_icon = Image::builder()
                    .icon_name("channel-secure-symbolic")
                    .css_classes(["cc-item-lock"])
                    .pixel_size(12)
                    .build();
                row_content.append(&lock_icon);
            }

            row_btn.set_child(Some(&row_content));

            let target = net.ssid.clone();
            let cb_c = content_box.clone();
            let sw_c = wifi_switch.clone();
            let upd_c = is_upd_sw.clone();
            let po_c = popover.clone();
            let ico_c = tray_icon.clone();

            row_btn.connect_clicked(move |_| {
                info!("Conectando a red Wi-Fi desde menú de tray: {}", target);
                let t = target.clone();
                let cb = cb_c.clone();
                let sw = sw_c.clone();
                let upd = upd_c.clone();
                let po = po_c.clone();
                let ico = ico_c.clone();

                glib::MainContext::default().spawn_local(async move {
                    let _ = NetworkService::connect_to_ssid(&t).await;
                    update_wifi_tray_icon(&ico).await;
                    refresh_wifi_tray_list(&cb, &sw, &upd, &po, &ico).await;
                });
            });

            content_box.append(&row_btn);
        }
    }
}

async fn update_wifi_tray_icon(icon: &Image) {
    let enabled = NetworkService::is_wifi_enabled().await;
    if !enabled {
        icon.set_icon_name(Some("network-wireless-offline-symbolic"));
        return;
    }

    if let Some(_) = NetworkService::get_active_wifi_ssid().await {
        icon.set_icon_name(Some("network-wireless-signal-excellent-symbolic"));
    } else {
        icon.set_icon_name(Some("network-wireless-acquiring-symbolic"));
    }
}

fn get_wifi_signal_icon(signal: u8) -> &'static str {
    if signal >= 80 {
        "network-wireless-signal-excellent-symbolic"
    } else if signal >= 55 {
        "network-wireless-signal-good-symbolic"
    } else if signal >= 30 {
        "network-wireless-signal-ok-symbolic"
    } else if signal >= 10 {
        "network-wireless-signal-weak-symbolic"
    } else {
        "network-wireless-signal-none-symbolic"
    }
}

// ==========================================
// 2. Bluetooth Menu Extra
// ==========================================

fn create_bluetooth_tray_button() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["bluetooth-pill", "topbar-btn"])
        .tooltip_text("Bluetooth")
        .has_frame(false)
        .always_show_arrow(false)
        .build();

    let icon = Image::from_icon_name("bluetooth-active-symbolic");
    menu_btn.set_child(Some(&icon));

    let popover = Popover::builder()
        .css_classes(["control-center-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(270)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    // Header: Título + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let title_lbl = Label::builder()
        .label("Bluetooth")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let bt_switch = Switch::builder().valign(Align::Center).build();

    header.append(&title_lbl);
    header.append(&*bt_switch);
    popover_box.append(&header);

    // Scrolled list de dispositivos
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(160)
        .max_content_height(230)
        .propagate_natural_height(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    scrolled.set_child(Some(&content_box));
    popover_box.append(&scrolled);

    // Footer: Separador y Preferencias de Bluetooth
    let sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Preferencias de Bluetooth...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let settings_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
        let mut command = Command::new(settings_bin);
        malus_de_common::config::set_cursor_environment(&mut command);
        if let Err(err) = command.args(["--panel", "bluetooth"]).spawn() {
            tracing::error!("No se pudieron abrir los ajustes de Bluetooth de Malus DE: {err}");
        }
    });

    popover_box.append(&sep);
    popover_box.append(&pref_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    // Conectar Switch
    let is_updating_switch = Rc::new(RefCell::new(false));
    let is_upd_sw = is_updating_switch.clone();
    let cb_for_sw = content_box.clone();
    let sw_for_sw = bt_switch.clone();
    let po_for_sw = popover.clone();
    let icon_sw = icon.clone();

    bt_switch.connect_state_set(move |_sw, state| {
        if *is_upd_sw.borrow() {
            return glib::Propagation::Proceed;
        }
        info!("Tray Bluetooth switch cambiado a: {}", state);

        let cb_inner = cb_for_sw.clone();
        let sw_inner = sw_for_sw.clone();
        let upd_inner = is_upd_sw.clone();
        let po_inner = po_for_sw.clone();
        let ico_inner = icon_sw.clone();

        glib::MainContext::default().spawn_local(async move {
            let _ = BluetoothService::set_powered(state).await;
            update_bluetooth_tray_icon(&ico_inner).await;
            refresh_bt_tray_list(&cb_inner, &sw_inner, &upd_inner, &po_inner, &ico_inner).await;
        });

        glib::Propagation::Proceed
    });

    // Actualizar al abrir Popover
    let cb_pop = content_box.clone();
    let sw_pop = bt_switch.clone();
    let upd_pop = is_updating_switch.clone();
    let po_pop = popover.clone();
    let ico_pop = icon.clone();

    popover.connect_show(move |_| {
        let cb = cb_pop.clone();
        let sw = sw_pop.clone();
        let upd = upd_pop.clone();
        let po = po_pop.clone();
        let ico = ico_pop.clone();

        glib::MainContext::default().spawn_local(async move {
            update_bluetooth_tray_icon(&ico).await;
            refresh_bt_tray_list(&cb, &sw, &upd, &po, &ico).await;
        });
    });

    // Actualización inicial del icono
    let ico_init = icon.clone();
    glib::MainContext::default().spawn_local(async move {
        update_bluetooth_tray_icon(&ico_init).await;
    });

    menu_btn
}

async fn refresh_bt_tray_list(
    content_box: &GtkBox,
    bt_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    popover: &Popover,
    tray_icon: &Image,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    let powered = BluetoothService::is_powered().await;

    *is_upd_sw.borrow_mut() = true;
    bt_switch.set_state(powered);
    *is_upd_sw.borrow_mut() = false;

    if !powered {
        let disabled_lbl = Label::builder()
            .label("El Bluetooth está desactivado")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(30)
            .margin_bottom(30)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    let devices = BluetoothService::get_paired_devices().await;

    if devices.is_empty() {
        let empty_lbl = Label::builder()
            .label("No hay dispositivos vinculados")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(20)
            .margin_bottom(20)
            .build();
        content_box.append(&empty_lbl);
        return;
    }

    let dev_lbl = Label::builder()
        .label("DISPOSITIVOS")
        .css_classes(["cc-section-label"])
        .xalign(0.0)
        .margin_start(4)
        .margin_top(2)
        .build();
    content_box.append(&dev_lbl);

    for dev in devices {
        let row_btn = Button::builder()
            .css_classes(["cc-list-row"])
            .has_frame(false)
            .build();

        if dev.connected {
            row_btn.add_css_class("cc-list-row-active");
        }

        let row_content = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();

        let dev_icon = Image::builder()
            .icon_name(&dev.icon_name)
            .pixel_size(16)
            .build();

        let text_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .hexpand(true)
            .spacing(1)
            .build();

        let name_lbl = Label::builder()
            .label(&dev.name)
            .css_classes(["cc-item-title"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let sub_text = if dev.connected {
            if let Some(bat) = dev.battery {
                format!("Conectado • {}% batería", bat)
            } else {
                "Conectado".to_string()
            }
        } else {
            "No conectado".to_string()
        };

        let status_lbl = Label::builder()
            .label(&sub_text)
            .css_classes(["cc-item-subtitle"])
            .xalign(0.0)
            .build();

        text_box.append(&name_lbl);
        text_box.append(&status_lbl);

        row_content.append(&dev_icon);
        row_content.append(&text_box);

        if dev.connected {
            let check_icon = Image::builder()
                .icon_name("object-select-symbolic")
                .css_classes(["cc-item-check"])
                .pixel_size(14)
                .build();
            row_content.append(&check_icon);
        }

        row_btn.set_child(Some(&row_content));

        let path = dev.path.clone();
        let is_connected = dev.connected;
        let cb_c = content_box.clone();
        let sw_c = bt_switch.clone();
        let upd_c = is_upd_sw.clone();
        let po_c = popover.clone();
        let ico_c = tray_icon.clone();

        row_btn.connect_clicked(move |_| {
            let p = path.clone();
            let cb = cb_c.clone();
            let sw = sw_c.clone();
            let upd = upd_c.clone();
            let po = po_c.clone();
            let ico = ico_c.clone();

            glib::MainContext::default().spawn_local(async move {
                if is_connected {
                    BluetoothService::disconnect_device(&p).await;
                } else {
                    BluetoothService::connect_device(&p).await;
                }
                update_bluetooth_tray_icon(&ico).await;
                refresh_bt_tray_list(&cb, &sw, &upd, &po, &ico).await;
            });
        });

        content_box.append(&row_btn);
    }
}

async fn update_bluetooth_tray_icon(icon: &Image) {
    let powered = BluetoothService::is_powered().await;
    if powered {
        icon.set_icon_name(Some("bluetooth-active-symbolic"));
    } else {
        icon.set_icon_name(Some("bluetooth-disabled-symbolic"));
    }
}

// ==========================================
// 3. Sonido / Volumen Menu Extra
// ==========================================

fn create_volume_tray_button() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["volume-pill", "topbar-btn"])
        .tooltip_text("Sonido")
        .has_frame(false)
        .always_show_arrow(false)
        .build();

    let icon = Image::from_icon_name("audio-volume-high-symbolic");
    menu_btn.set_child(Some(&icon));

    let popover = Popover::builder()
        .css_classes(["control-center-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(270)
        .spacing(10)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    // Slider card idéntico al Centro de Control
    let sound_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let sound_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let mute_btn = Button::builder()
        .css_classes(["cc-mute-btn"])
        .has_frame(false)
        .tooltip_text("Silenciar / Reactivar Audio")
        .build();
    let vol_inner_icon = Image::from_icon_name("audio-volume-high-symbolic");
    mute_btn.set_child(Some(&vol_inner_icon));

    let sound_title = Label::builder()
        .label("Sonido")
        .css_classes(["cc-tile-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let sound_pct = Label::builder()
        .label("75%")
        .css_classes(["cc-tile-subtitle"])
        .build();

    sound_header.append(&mute_btn);
    sound_header.append(&sound_title);
    sound_header.append(&sound_pct);

    let volume_scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    volume_scale.set_value(75.0);
    volume_scale.set_draw_value(false);

    sound_card.append(&sound_header);
    sound_card.append(&volume_scale);
    popover_box.append(&sound_card);

    // Sección Dispositivo de Salida
    let output_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    let output_lbl = Label::builder()
        .label("SALIDA")
        .css_classes(["cc-section-label"])
        .xalign(0.0)
        .margin_start(4)
        .build();
    output_section.append(&output_lbl);

    let dev_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-list-row", "cc-list-row-active"])
        .spacing(8)
        .build();

    let dev_icon = Image::builder()
        .icon_name("audio-speakers-symbolic")
        .pixel_size(16)
        .build();

    let dev_name_lbl = Label::builder()
        .label("Altavoces")
        .css_classes(["cc-item-title"])
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    let dev_check = Image::builder()
        .icon_name("object-select-symbolic")
        .css_classes(["cc-item-check"])
        .pixel_size(14)
        .build();

    dev_card.append(&dev_icon);
    dev_card.append(&dev_name_lbl);
    dev_card.append(&dev_check);
    output_section.append(&dev_card);
    popover_box.append(&output_section);

    // Footer: Separador y Preferencias de Sonido
    let sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Preferencias de Sonido...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let mut command = Command::new("gnome-control-center");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.arg("sound").spawn();
    });

    popover_box.append(&sep);
    popover_box.append(&pref_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    // Lógica del Slider de volumen
    let is_updating_vol = Rc::new(RefCell::new(false));
    let is_upd_scale = is_updating_vol.clone();
    let pct_scale = sound_pct.clone();
    let inner_ico_scale = vol_inner_icon.clone();
    let tray_ico_scale = icon.clone();

    volume_scale.connect_value_changed(move |scale| {
        if *is_upd_scale.borrow() {
            return;
        }
        let val = scale.value();
        pct_scale.set_label(&format!("{}%", val.round() as i32));

        let icon_name = if val < 1.0 {
            "audio-volume-muted-symbolic"
        } else if val < 35.0 {
            "audio-volume-low-symbolic"
        } else if val < 70.0 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        };

        inner_ico_scale.set_icon_name(Some(icon_name));
        tray_ico_scale.set_icon_name(Some(icon_name));

        AudioService::set_volume(val);
    });

    // Botón Silenciar
    let is_upd_mute = is_updating_vol.clone();
    let scale_mute = volume_scale.clone();
    let pct_mute = sound_pct.clone();
    let inner_ico_mute = vol_inner_icon.clone();
    let tray_ico_mute = icon.clone();

    mute_btn.connect_clicked(move |_| {
        AudioService::toggle_mute();
        let sc = scale_mute.clone();
        let pc = pct_mute.clone();
        let ii = inner_ico_mute.clone();
        let ti = tray_ico_mute.clone();
        let lk = is_upd_mute.clone();

        glib::MainContext::default().spawn_local(async move {
            refresh_volume_tray_ui(&sc, &pc, &ii, &ti, &lk).await;
        });
    });

    // Actualizar al abrir Popover
    let sc_pop = volume_scale.clone();
    let pc_pop = sound_pct.clone();
    let ii_pop = vol_inner_icon.clone();
    let ti_pop = icon.clone();
    let lk_pop = is_updating_vol.clone();
    let dn_pop = dev_name_lbl.clone();
    let di_pop = dev_icon.clone();

    popover.connect_show(move |_| {
        let sc = sc_pop.clone();
        let pc = pc_pop.clone();
        let ii = ii_pop.clone();
        let ti = ti_pop.clone();
        let lk = lk_pop.clone();
        let dn = dn_pop.clone();
        let di = di_pop.clone();

        glib::MainContext::default().spawn_local(async move {
            refresh_volume_tray_ui(&sc, &pc, &ii, &ti, &lk).await;
            let sink_name = AudioService::get_active_sink_name().await;
            dn.set_label(&sink_name);
            let s_lower = sink_name.to_lowercase();
            if s_lower.contains("space one")
                || s_lower.contains("auricular")
                || s_lower.contains("headphone")
                || s_lower.contains("headset")
                || s_lower.contains("buds")
            {
                di.set_icon_name(Some("audio-headset-symbolic"));
            } else {
                di.set_icon_name(Some("audio-speakers-symbolic"));
            }
        });
    });

    // Actualización inicial
    let sc_init = volume_scale.clone();
    let pc_init = sound_pct.clone();
    let ii_init = vol_inner_icon.clone();
    let ti_init = icon.clone();
    let lk_init = is_updating_vol.clone();
    glib::MainContext::default().spawn_local(async move {
        refresh_volume_tray_ui(&sc_init, &pc_init, &ii_init, &ti_init, &lk_init).await;
    });

    menu_btn
}

async fn refresh_volume_tray_ui(
    scale: &Scale,
    pct_lbl: &Label,
    inner_icon: &Image,
    tray_icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    let (vol, is_muted) = AudioService::get_volume().await;

    *is_updating.borrow_mut() = true;
    scale.set_value(vol);
    *is_updating.borrow_mut() = false;

    if is_muted {
        pct_lbl.set_label("Silenciado");
        inner_icon.set_icon_name(Some("audio-volume-muted-symbolic"));
        tray_icon.set_icon_name(Some("audio-volume-muted-symbolic"));
    } else {
        pct_lbl.set_label(&format!("{}%", vol.round() as i32));
        let icon_name = if vol < 1.0 {
            "audio-volume-muted-symbolic"
        } else if vol < 35.0 {
            "audio-volume-low-symbolic"
        } else if vol < 70.0 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        };
        inner_icon.set_icon_name(Some(icon_name));
        tray_icon.set_icon_name(Some(icon_name));
    }
}

// ==========================================
// 4. Malus Link / AirDrop Menu Extra
// ==========================================

fn create_link_tray_button() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["link-pill", "topbar-btn"])
        .tooltip_text("AirDrop y Enlace Móvil")
        .has_frame(false)
        .always_show_arrow(false)
        .build();

    let icon = Image::from_icon_name("network-wireless-hotspot-symbolic");
    menu_btn.set_child(Some(&icon));

    let popover = Popover::builder()
        .css_classes(["control-center-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(270)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let title_lbl = Label::builder()
        .label("AirDrop")
        .css_classes(["cc-subview-title"])
        .xalign(0.0)
        .build();
    popover_box.append(&title_lbl);

    let desc_lbl = Label::builder()
        .label(
            "Comparte archivos vía AirDrop nativo (AWDL/owly) con MacBook, iPhone o iPad cercanos.",
        )
        .css_classes(["cc-airdrop-desc"])
        .wrap(true)
        .xalign(0.0)
        .build();
    popover_box.append(&desc_lbl);

    let status_lbl = Label::builder()
        .label("Consultando estado del motor AirDrop...")
        .css_classes(["cc-item-subtitle"])
        .wrap(true)
        .xalign(0.0)
        .build();
    popover_box.append(&status_lbl);

    // Opciones
    let (row_off, check_off) =
        create_link_option_row("No recibir", "Desactiva AWDL y mantiene tu Wi-Fi normal");
    let (row_contacts, check_contacts) =
        create_link_option_row("Solo contactos", "Requiere Apple ID (inactivo en Linux)");
    let (row_everyone, check_everyone) =
        create_link_option_row("Todos durante 10 min", "Activa awdl0 y recibe en Descargas");

    popover_box.append(&row_off);
    popover_box.append(&row_contacts);
    popover_box.append(&row_everyone);

    // Dispositivos cercanos detectados
    let peers_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .visible(false)
        .build();
    popover_box.append(&peers_box);

    // Acciones reales de escaneo y envío
    let actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let scan_btn = Button::builder()
        .css_classes(["cc-airdrop-sim-btn"])
        .label("🔍 Buscar")
        .has_frame(false)
        .tooltip_text(
            "Buscar dispositivos Apple cercanos en awdl0 (requiere activar 'Todos durante 10 min')",
        )
        .build();

    let send_btn = Button::builder()
        .css_classes(["cc-airdrop-sim-btn"])
        .label("📤 Enviar archivo")
        .has_frame(false)
        .tooltip_text("Seleccionar un archivo y enviarlo al primer dispositivo AirDrop cercano")
        .build();

    actions_row.append(&scan_btn);
    actions_row.append(&send_btn);
    popover_box.append(&actions_row);

    // Footer
    let sep = Separator::new(Orientation::Horizontal);
    let down_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Abrir carpeta Descargas...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    down_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let dl_dir = resolve_airdrop_downloads_dir();
        let mut command = Command::new("malus-de-branches");
        malus_de_common::config::set_cursor_environment(&mut command);
        if command.arg(&dl_dir).spawn().is_err() {
            let mut fallback = Command::new("xdg-open");
            let _ = fallback.arg(&dl_dir).spawn();
        }
    });

    popover_box.append(&sep);
    popover_box.append(&down_btn);

    // Manejador de selección
    let ro = row_off.clone();
    let rc = row_contacts.clone();
    let re = row_everyone.clone();
    let co = check_off.clone();
    let cc = check_contacts.clone();
    let ce = check_everyone.clone();

    let update_ui_mode = Rc::new(move |mode: &str| {
        ro.remove_css_class("cc-list-row-active");
        rc.remove_css_class("cc-list-row-active");
        re.remove_css_class("cc-list-row-active");
        co.set_visible(false);
        cc.set_visible(false);
        ce.set_visible(false);

        match mode {
            "contacts" => {
                rc.add_css_class("cc-list-row-active");
                cc.set_visible(true);
            }
            "everyone" => {
                re.add_css_class("cc-list-row-active");
                ce.set_visible(true);
            }
            _ => {
                ro.add_css_class("cc-list-row-active");
                co.set_visible(true);
            }
        }
    });

    let pop_send = popover.clone();
    send_btn.connect_clicked(move |_| {
        pop_send.popdown();
        pick_and_send_airdrop_file("auto".to_string());
    });

    let status_scan = status_lbl.clone();
    let peers_scan = peers_box.clone();
    let pop_scan = popover.clone();
    scan_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("⏳ Buscando...");
        status_scan.set_label("Buscando MacBook / iPhone cercanos vía AWDL...");
        let btn_c = btn.clone();
        let st_c = status_scan.clone();
        let pb_c = peers_scan.clone();
        let pop_peer = pop_scan.clone();
        glib::MainContext::default().spawn_local(async move {
            let peers = scan_airdrop_peers_dbus(6).await;
            btn_c.set_sensitive(true);
            btn_c.set_label("🔍 Buscar");
            populate_airdrop_peers_list(&pb_c, &peers, &pop_peer);
            if peers.is_empty() {
                st_c.set_label("Sin dispositivos encontrados. Asegúrate de tener 'Todos durante 10 min' activo aquí y en tu Mac.");
            } else {
                st_c.set_label(&format!("{} dispositivo(s) AirDrop encontrado(s). Haz clic para enviar un archivo.", peers.len()));
            }
        });
    });

    let refresh_status: Rc<dyn Fn()> = {
        let u_mode = update_ui_mode.clone();
        let st_lbl = status_lbl.clone();
        let pb = peers_box.clone();
        let pop_r = popover.clone();
        Rc::new(move || {
            let u = u_mode.clone();
            let s = st_lbl.clone();
            let p = pb.clone();
            let pop_inner = pop_r.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Some(v) = get_airdrop_status_json_dbus().await {
                    let mode = v["mode"].as_str().unwrap_or("off");
                    u(mode);
                    s.set_label(&format_airdrop_status_text(&v));
                    let peers: Vec<(String, String, String)> = v["peers"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|item| {
                                    (
                                        item["id"].as_str().unwrap_or("").to_string(),
                                        item["name"].as_str().unwrap_or("Apple").to_string(),
                                        item["address"].as_str().unwrap_or("").to_string(),
                                    )
                                })
                                .filter(|(id, _, _)| !id.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    populate_airdrop_peers_list(&p, &peers, &pop_inner);
                } else {
                    let mode = get_link_mode_dbus().await;
                    u(&mode);
                    s.set_label("malus-de-link no disponible en D-Bus.");
                }
            });
        })
    };

    let u_off = update_ui_mode.clone();
    let ref_off = refresh_status.clone();
    row_off.connect_clicked(move |_| {
        u_off("off");
        set_link_mode_dbus_and_refresh("off", ref_off.clone());
    });

    let u_cont = update_ui_mode.clone();
    let ref_cont = refresh_status.clone();
    row_contacts.connect_clicked(move |_| {
        u_cont("contacts");
        set_link_mode_dbus_and_refresh("contacts", ref_cont.clone());
    });

    let u_every = update_ui_mode.clone();
    let ref_every = refresh_status.clone();
    let st_every = status_lbl.clone();
    row_everyone.connect_clicked(move |_| {
        u_every("everyone");
        st_every.set_label(
            "Levantando radio AWDL (awdl0)... En Intel AX210 el Wi-Fi se pausa temporalmente.",
        );
        set_link_mode_dbus_and_refresh("everyone", ref_every.clone());
    });

    let ref_pop = refresh_status.clone();
    popover.connect_show(move |_| {
        ref_pop();
    });

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    menu_btn
}

fn resolve_airdrop_downloads_dir() -> std::path::PathBuf {
    let home = glib::home_dir();
    let es = home.join("Descargas");
    if es.exists() {
        es
    } else {
        home.join("Downloads")
    }
}

fn format_airdrop_status_text(v: &serde_json::Value) -> String {
    let radio_state = v["radio_state"].as_str().unwrap_or("off");
    let rem = v["remaining_seconds"].as_u64().unwrap_or(0);
    let station = v["station"].as_str().unwrap_or("Wi-Fi");
    let rmode = v["radio_mode"].as_str().unwrap_or("");
    let hw_ready = v["hardware_ready"].as_bool().unwrap_or(false);
    if let Some(err) = v["last_error"].as_str() {
        if !err.trim().is_empty() {
            return format!("⚠️ {}", err);
        }
    }
    match radio_state {
        "starting" => format!("⏳ Iniciando awdl0 en {}...", station),
        "active" => {
            let name = v["device_name"].as_str().unwrap_or("Malus DE");
            let time_part = if rem > 0 {
                format!(" · {}:{:02}", rem / 60, rem % 60)
            } else {
                String::new()
            };
            format!("🟢 Activo como '{}' en awdl0{}", name, time_part)
        }
        "error" => "❌ Error activando radio AWDL.".to_string(),
        _ => {
            if hw_ready {
                if rmode == "exclusive" {
                    format!(
                        "Listo ({} · modo exclusivo: pausa Wi-Fi al activar)",
                        station
                    )
                } else {
                    format!("Listo ({} · modo compartido)", station)
                }
            } else {
                "⚠️ Motor owly/hardware AWDL no listo.".to_string()
            }
        }
    }
}

fn populate_airdrop_peers_list(
    peers_box: &GtkBox,
    peers: &[(String, String, String)],
    popover: &Popover,
) {
    while let Some(child) = peers_box.first_child() {
        peers_box.remove(&child);
    }
    if peers.is_empty() {
        peers_box.set_visible(false);
        return;
    }
    peers_box.set_visible(true);
    let hdr = Label::builder()
        .label("Dispositivos cercanos (clic para enviar):")
        .css_classes(["cc-item-subtitle"])
        .xalign(0.0)
        .build();
    peers_box.append(&hdr);

    for (id, name, addr) in peers {
        let subtitle = if addr.is_empty() {
            "Enviar archivo vía AirDrop".to_string()
        } else {
            format!("AWDL · {}", addr)
        };
        let (row, _) = create_link_option_row(&format!("💻 {}", name), &subtitle);
        let target_id = id.clone();
        let pop_c = popover.clone();
        row.connect_clicked(move |_| {
            pop_c.popdown();
            pick_and_send_airdrop_file(target_id.clone());
        });
        peers_box.append(&row);
    }
}

fn pick_and_send_airdrop_file(target: String) {
    let dialog = gtk4::FileDialog::builder()
        .title("Enviar archivo por AirDrop")
        .build();
    dialog.open(
        None::<&gtk4::Window>,
        None::<&gtk4::gio::Cancellable>,
        move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().into_owned();
                    let target_c = target.clone();
                    glib::MainContext::default().spawn_local(async move {
                        if let Ok(conn) = zbus::Connection::session().await {
                            let _ = conn
                                .call_method(
                                    Some("org.malus_de.Link"),
                                    "/org/malus_de/Link",
                                    Some("org.malus_de.Link"),
                                    "SendAirDropFile",
                                    &(target_c, path_str),
                                )
                                .await;
                        }
                    });
                }
            }
        },
    );
}

fn create_link_option_row(title: &str, subtitle: &str) -> (Button, Image) {
    let btn = Button::builder()
        .css_classes(["cc-list-row"])
        .has_frame(false)
        .build();

    let content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .spacing(1)
        .build();

    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["cc-item-title"])
        .xalign(0.0)
        .build();

    let sub_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["cc-item-subtitle"])
        .xalign(0.0)
        .build();

    text_box.append(&title_lbl);
    text_box.append(&sub_lbl);

    let check_icon = Image::builder()
        .icon_name("object-select-symbolic")
        .css_classes(["cc-item-check"])
        .pixel_size(14)
        .visible(false)
        .build();

    content.append(&text_box);
    content.append(&check_icon);
    btn.set_child(Some(&content));

    (btn, check_icon)
}

fn set_link_mode_dbus_and_refresh(mode: &'static str, refresh: Rc<dyn Fn()>) {
    glib::MainContext::default().spawn_local(async move {
        if let Ok(conn) = zbus::Connection::session().await {
            let _ = conn
                .call_method(
                    Some("org.malus_de.Link"),
                    "/org/malus_de/Link",
                    Some("org.malus_de.Link"),
                    "SetAirDropMode",
                    &(mode,),
                )
                .await;
        }
        refresh();
        if mode == "everyone" {
            let ref_later = refresh.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                ref_later();
            });
        }
    });
}

async fn get_airdrop_status_json_dbus() -> Option<serde_json::Value> {
    let conn = zbus::Connection::session().await.ok()?;
    let reply = conn
        .call_method(
            Some("org.malus_de.Link"),
            "/org/malus_de/Link",
            Some("org.malus_de.Link"),
            "GetAirDropStatusJson",
            &(),
        )
        .await
        .ok()?;
    let raw: String = reply.body().deserialize().ok()?;
    serde_json::from_str(&raw).ok()
}

async fn scan_airdrop_peers_dbus(seconds: u32) -> Vec<(String, String, String)> {
    if let Ok(conn) = zbus::Connection::session().await {
        if let Ok(reply) = conn
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "ScanAirDropPeers",
                &(seconds,),
            )
            .await
        {
            if let Ok(peers) = reply.body().deserialize::<Vec<(String, String, String)>>() {
                return peers;
            }
        }
    }
    Vec::new()
}

async fn get_link_mode_dbus() -> String {
    if let Ok(conn) = zbus::Connection::session().await {
        if let Ok(reply) = conn
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "GetAirDropMode",
                &(),
            )
            .await
        {
            if let Ok(mode) = reply.body().deserialize::<String>() {
                return mode;
            }
        }
    }
    "off".to_string()
}

// ==========================================
// 5. Spotlight Search Button
// ==========================================

fn create_spotlight_button() -> Button {
    let spotlight_btn = Button::builder()
        .css_classes(["spotlight-pill", "topbar-btn"])
        .tooltip_text("Búsqueda Spotlight (Alt + Espacio / Super + K)")
        .has_frame(false)
        .build();
    let spotlight_icon = Image::from_icon_name("system-search-symbolic");
    spotlight_btn.set_child(Some(&spotlight_icon));

    spotlight_btn.connect_clicked(|_| {
        info!("Alternando Spotlight nativo del compositor desde la barra superior...");
        if let Err(e) = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::ToggleSpotlight,
        ) {
            tracing::error!("Error al enviar ToggleSpotlight al compositor: {:?}", e);
        }
    });

    spotlight_btn
}
