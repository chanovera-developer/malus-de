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
    static VOLUME_TRAY_REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static KEYBOARD_CONFIG_MONITOR: RefCell<Option<gtk4::gio::FileMonitor>> = const { RefCell::new(None) };
    static KEYBOARD_LABEL_REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static SYSTEM_DBUS_CONN: RefCell<Option<gtk4::gio::DBusConnection>> = const { RefCell::new(None) };
    static SYSTEM_DBUS_SUBS: RefCell<Vec<gtk4::gio::SignalSubscriptionId>> = const { RefCell::new(Vec::new()) };
}

fn system_dbus_connection() -> Option<gtk4::gio::DBusConnection> {
    SYSTEM_DBUS_CONN.with(|cell| {
        if let Some(conn) = cell.borrow().as_ref() {
            return Some(conn.clone());
        }
        let conn = gtk4::gio::bus_get_sync(gtk4::gio::BusType::System, gtk4::gio::Cancellable::NONE)
            .ok()?;
        *cell.borrow_mut() = Some(conn.clone());
        Some(conn)
    })
}

fn dbus_props_changed_matches(
    params: &glib::Variant,
    expected_iface: &str,
    keys: &[&str],
) -> bool {
    let Some(iface) = params.try_child_get::<String>(0).ok().flatten() else {
        return false;
    };
    if iface != expected_iface {
        return false;
    }
    let Some(dict) = params.try_child_get::<glib::VariantDict>(1).ok().flatten() else {
        return false;
    };
    keys.iter().any(|k| dict.contains(k))
}

pub fn refresh_keyboard_layout_badge() {
    KEYBOARD_LABEL_REFRESH.with(|cell| {
        if let Some(ref f) = *cell.borrow() {
            f();
        }
    });
}

pub fn refresh_tray_visibility() {
    TRAY_VISIBILITY_REFRESH.with(|cell| {
        if let Some(ref f) = *cell.borrow() {
            f();
        }
    });
    refresh_volume_tray();
}

pub fn refresh_volume_tray() {
    VOLUME_TRAY_REFRESH.with(|cell| {
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

    let btn_sync = btn.clone();
    let refresh_kb: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = malus_de_common::config::InputConfig::load();
        let expected = format!("⌨ {}", cfg.layout_short_badge());
        if btn_sync.label().as_deref() != Some(expected.as_str()) {
            btn_sync.set_label(&expected);
            btn_sync.set_tooltip_text(Some(&format!(
                "Teclado: {} (Clic para alternar LATAM / ES / US)",
                cfg.layout_display_name()
            )));
        }
    });
    KEYBOARD_LABEL_REFRESH.with(|cell| {
        *cell.borrow_mut() = Some(refresh_kb.clone());
    });
    let input_file = gtk4::gio::File::for_path(malus_de_common::config::InputConfig::config_path());
    if let Ok(monitor) = input_file.monitor_file(
        gtk4::gio::FileMonitorFlags::NONE,
        gtk4::gio::Cancellable::NONE,
    ) {
        let refresh_mon = refresh_kb.clone();
        monitor.connect_changed(move |_, _, _, _| {
            refresh_mon();
        });
        KEYBOARD_CONFIG_MONITOR.with(|cell| {
            *cell.borrow_mut() = Some(monitor);
        });
    }

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
        .css_classes(["control-center-popover", "sound-menu-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(292)
        .spacing(7)
        .build();

    // Header: Título + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .margin_start(6)
        .margin_end(6)
        .build();

    let title_lbl = Label::builder()
        .label("Wi-Fi")
        .css_classes(["sound-menu-title"])
        .hexpand(true)
        .xalign(0.0)
        .valign(Align::Center)
        .build();

    let wifi_switch = Switch::builder().valign(Align::Center).build();

    header.append(&title_lbl);
    header.append(&*wifi_switch);
    popover_box.append(&header);

    let sep_top = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    popover_box.append(&sep_top);

    // Lista ajustada dinámicamente al contenido (sin altura mínima vacía ni padding extra)
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .max_content_height(260)
        .propagate_natural_height(true)
        .vexpand(false)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .vexpand(false)
        .build();
    scrolled.set_child(Some(&content_box));
    popover_box.append(&scrolled);

    // Footer: Separador y Preferencias de Wi-Fi
    let sep = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    let pref_btn = Button::builder()
        .css_classes(["sound-menu-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();
    pref_btn.set_child(Some(
        &Label::builder()
            .label("Preferencias de Wi-Fi...")
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .build(),
    ));
    crate::ui::system_menu::attach_tray_squircle_button(&pref_btn, false, false, 4, 6);

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

    // Sincronización en tiempo real con NetworkManager (ej. si se alterna Wi-Fi desde Ajustes o Centro de Control)
    if let Some(bus) = system_dbus_connection() {
        let cb_sig = content_box.clone();
        let sw_sig = wifi_switch.clone();
        let upd_sig = is_updating_switch.clone();
        let po_sig = popover.clone();
        let ico_sig = icon.clone();
        let pending = Rc::new(RefCell::new(false));
        let sub_id = bus.signal_subscribe(
            Some("org.freedesktop.NetworkManager"),
            Some("org.freedesktop.DBus.Properties"),
            Some("PropertiesChanged"),
            Some("/org/freedesktop/NetworkManager"),
            None,
            gtk4::gio::DBusSignalFlags::NONE,
            move |_conn, _sender, _path, _iface, _signal, params| {
                if !dbus_props_changed_matches(
                    params,
                    "org.freedesktop.NetworkManager",
                    &["WirelessEnabled", "PrimaryConnection", "State", "Connectivity"],
                ) {
                    return;
                }
                if *pending.borrow() {
                    return;
                }
                *pending.borrow_mut() = true;
                let pend = pending.clone();
                let cb = cb_sig.clone();
                let sw = sw_sig.clone();
                let upd = upd_sig.clone();
                let po = po_sig.clone();
                let ico = ico_sig.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(160), move || {
                    *pend.borrow_mut() = false;
                    glib::MainContext::default().spawn_local(async move {
                        update_wifi_tray_icon(&ico).await;
                        if po.is_visible() {
                            refresh_wifi_tray_list(&cb, &sw, &upd, &po, &ico).await;
                        }
                    });
                });
            },
        );
        SYSTEM_DBUS_SUBS.with(|subs| subs.borrow_mut().push(sub_id));
    }

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
            .label("Wi-Fi desactivado")
            .css_classes(["sound-menu-section"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(6)
            .margin_end(6)
            .margin_top(2)
            .margin_bottom(2)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    // Red conectada actualmente
    if let Some(active) = networks.iter().find(|n| n.in_use) {
        let active_header = Label::builder()
            .label("Red actual")
            .css_classes(["sound-menu-section"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(6)
            .margin_end(6)
            .build();
        content_box.append(&active_header);

        let active_card = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .halign(Align::Fill)
            .valign(Align::Center)
            .hexpand(true)
            .vexpand(false)
            .spacing(8)
            .margin_top(2)
            .margin_bottom(2)
            .margin_start(6)
            .margin_end(6)
            .build();

        let circle = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .width_request(24)
            .height_request(24)
            .halign(Align::Start)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .css_classes(["sound-menu-device-circle", "sound-menu-device-circle-active"])
            .build();
        let sig_icon = Image::builder()
            .icon_name(get_wifi_signal_icon(active.signal))
            .pixel_size(13)
            .width_request(24)
            .height_request(24)
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        circle.append(&sig_icon);

        let ssid_lbl = Label::builder()
            .label(&active.ssid)
            .css_classes(["sound-menu-device-name"])
            .hexpand(true)
            .halign(Align::Start)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let check_icon = Image::builder()
            .icon_name("object-select-symbolic")
            .css_classes(["cc-item-check"])
            .pixel_size(13)
            .hexpand(false)
            .halign(Align::End)
            .build();

        active_card.append(&circle);
        active_card.append(&ssid_lbl);
        active_card.append(&check_icon);
        content_box.append(&active_card);
    }

    // Otras redes disponibles
    let other_networks: Vec<&WifiApInfo> = networks.iter().filter(|n| !n.in_use).collect();
    if other_networks.is_empty() {
        let empty_lbl = Label::builder()
            .label("No se detectaron otras redes")
            .css_classes(["sound-menu-section"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(6)
            .margin_end(6)
            .margin_top(2)
            .build();
        content_box.append(&empty_lbl);
    } else {
        let others_lbl = Label::builder()
            .label("Otras redes")
            .css_classes(["sound-menu-section"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .build();
        content_box.append(&others_lbl);

        let list_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .vexpand(false)
            .build();

        for net in other_networks {
            let row_btn = Button::builder()
                .css_classes(["sound-menu-device-row"])
                .has_frame(false)
                .halign(Align::Fill)
                .hexpand(true)
                .build();

            let row_content = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .halign(Align::Fill)
                .valign(Align::Center)
                .hexpand(true)
                .vexpand(false)
                .spacing(8)
                .build();

            let circle = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .width_request(24)
                .height_request(24)
                .halign(Align::Start)
                .valign(Align::Center)
                .hexpand(false)
                .vexpand(false)
                .css_classes(["sound-menu-device-circle"])
                .build();
            let sig_icon = Image::builder()
                .icon_name(get_wifi_signal_icon(net.signal))
                .pixel_size(13)
                .width_request(24)
                .height_request(24)
                .halign(Align::Center)
                .valign(Align::Center)
                .hexpand(false)
                .vexpand(false)
                .build();
            circle.append(&sig_icon);

            let ssid_lbl = Label::builder()
                .label(&net.ssid)
                .css_classes(["sound-menu-device-name"])
                .hexpand(true)
                .halign(Align::Start)
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();

            row_content.append(&circle);
            row_content.append(&ssid_lbl);

            if net.is_encrypted {
                let lock_icon = Image::builder()
                    .icon_name("channel-secure-symbolic")
                    .css_classes(["cc-item-lock"])
                    .pixel_size(12)
                    .hexpand(false)
                    .halign(Align::End)
                    .build();
                row_content.append(&lock_icon);
            }

            row_btn.set_child(Some(&row_content));
            crate::ui::system_menu::attach_tray_squircle_button(&row_btn, false, false, 2, 6);

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

            list_box.append(&row_btn);
        }

        content_box.append(&list_box);
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
        .css_classes(["control-center-popover", "sound-menu-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(292)
        .spacing(7)
        .build();

    // Header: Título + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .margin_start(6)
        .margin_end(6)
        .build();

    let title_lbl = Label::builder()
        .label("Bluetooth")
        .css_classes(["sound-menu-title"])
        .hexpand(true)
        .xalign(0.0)
        .valign(Align::Center)
        .build();

    let bt_switch = Switch::builder().valign(Align::Center).build();

    header.append(&title_lbl);
    header.append(&*bt_switch);
    popover_box.append(&header);

    let sep_top = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    popover_box.append(&sep_top);

    // Lista ajustada dinámicamente al contenido (sin altura mínima vacía)
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .max_content_height(260)
        .propagate_natural_height(true)
        .vexpand(false)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .vexpand(false)
        .build();
    scrolled.set_child(Some(&content_box));
    popover_box.append(&scrolled);

    // Footer: Separador y Configuración de Bluetooth...
    let sep = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    let pref_btn = Button::builder()
        .css_classes(["sound-menu-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();
    pref_btn.set_child(Some(
        &Label::builder()
            .label("Configuración de Bluetooth...")
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .build(),
    ));
    crate::ui::system_menu::attach_tray_squircle_button(&pref_btn, false, false, 4, 6);

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

    // Sincronización en tiempo real con BlueZ (ej. si se alterna Bluetooth o un dispositivo desde Ajustes o Centro de Control)
    if let Some(bus) = system_dbus_connection() {
        let cb_sig = content_box.clone();
        let sw_sig = bt_switch.clone();
        let upd_sig = is_updating_switch.clone();
        let po_sig = popover.clone();
        let ico_sig = icon.clone();
        let pending = Rc::new(RefCell::new(false));
        let sub_id = bus.signal_subscribe(
            Some("org.bluez"),
            Some("org.freedesktop.DBus.Properties"),
            Some("PropertiesChanged"),
            None,
            None,
            gtk4::gio::DBusSignalFlags::NONE,
            move |_conn, _sender, _path, _iface, _signal, params| {
                let adapter_changed =
                    dbus_props_changed_matches(params, "org.bluez.Adapter1", &["Powered"]);
                let device_changed =
                    dbus_props_changed_matches(params, "org.bluez.Device1", &["Connected"]);
                if !adapter_changed && !device_changed {
                    return;
                }
                if *pending.borrow() {
                    return;
                }
                *pending.borrow_mut() = true;
                let pend = pending.clone();
                let cb = cb_sig.clone();
                let sw = sw_sig.clone();
                let upd = upd_sig.clone();
                let po = po_sig.clone();
                let ico = ico_sig.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(160), move || {
                    *pend.borrow_mut() = false;
                    glib::MainContext::default().spawn_local(async move {
                        update_bluetooth_tray_icon(&ico).await;
                        if po.is_visible() {
                            refresh_bt_tray_list(&cb, &sw, &upd, &po, &ico).await;
                        }
                    });
                });
            },
        );
        SYSTEM_DBUS_SUBS.with(|subs| subs.borrow_mut().push(sub_id));
    }

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
            .label("Bluetooth desactivado")
            .css_classes(["sound-menu-section"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(6)
            .margin_end(6)
            .margin_top(2)
            .margin_bottom(2)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    let devices = BluetoothService::get_paired_devices().await;

    if devices.is_empty() {
        let empty_lbl = Label::builder()
            .label("No hay dispositivos vinculados")
            .css_classes(["sound-menu-section"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(6)
            .margin_end(6)
            .margin_top(2)
            .margin_bottom(2)
            .build();
        content_box.append(&empty_lbl);
        return;
    }

    let dev_lbl = Label::builder()
        .label("Dispositivos")
        .css_classes(["sound-menu-section"])
        .xalign(0.0)
        .halign(Align::Start)
        .margin_start(6)
        .margin_end(6)
        .build();
    content_box.append(&dev_lbl);

    let list_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .vexpand(false)
        .build();

    for dev in devices {
        let row_btn = Button::builder()
            .css_classes(["sound-menu-device-row"])
            .has_frame(false)
            .halign(Align::Fill)
            .hexpand(true)
            .build();

        let row_content = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::Fill)
            .valign(Align::Center)
            .hexpand(true)
            .vexpand(false)
            .build();

        let circle = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .width_request(24)
            .height_request(24)
            .halign(Align::Start)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .css_classes(if dev.connected {
                ["sound-menu-device-circle", "sound-menu-device-circle-active"].as_slice()
            } else {
                ["sound-menu-device-circle"].as_slice()
            })
            .build();
        let dev_icon = Image::builder()
            .icon_name(&dev.icon_name)
            .pixel_size(13)
            .width_request(24)
            .height_request(24)
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        circle.append(&dev_icon);

        let name_lbl = Label::builder()
            .label(&dev.name)
            .css_classes(["sound-menu-device-name"])
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        row_content.append(&circle);
        row_content.append(&name_lbl);

        if dev.connected {
            if let Some(bat) = dev.battery {
                let bat_lbl = Label::builder()
                    .label(format!("{}%", bat))
                    .css_classes(["sound-menu-section"])
                    .xalign(1.0)
                    .halign(Align::End)
                    .hexpand(false)
                    .valign(Align::Center)
                    .build();
                row_content.append(&bat_lbl);
            }
        }

        row_btn.set_child(Some(&row_content));
        crate::ui::system_menu::attach_tray_squircle_button(&row_btn, false, false, 2, 6);

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

        list_box.append(&row_btn);
    }

    content_box.append(&list_box);
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
        .css_classes(["control-center-popover", "sound-menu-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let popover_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .width_request(292)
        .spacing(7)
        .build();

    // Cabecera "Sonido" + slider estilo macOS Tahoe (sin tarjeta anidada)
    let sound_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    let sound_title = Label::builder()
        .label("Sonido")
        .css_classes(["sound-menu-title"])
        .xalign(0.0)
        .build();
    sound_section.append(&sound_title);

    let slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();

    let mute_btn = Button::builder()
        .css_classes(["sound-menu-vol-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Silenciar / Reactivar audio")
        .build();
    let vol_low_icon = Image::builder()
        .icon_name("audio-volume-low-symbolic")
        .pixel_size(14)
        .build();
    mute_btn.set_child(Some(&vol_low_icon));

    let volume_scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    volume_scale.set_value(75.0);
    volume_scale.set_draw_value(false);
    volume_scale.set_hexpand(true);
    volume_scale.set_valign(Align::Center);
    volume_scale.set_css_classes(&["sound-menu-scale"]);

    let max_vol_btn = Button::builder()
        .css_classes(["sound-menu-vol-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Subir volumen")
        .build();
    let vol_high_icon = Image::builder()
        .icon_name("audio-volume-high-symbolic")
        .pixel_size(15)
        .build();
    max_vol_btn.set_child(Some(&vol_high_icon));

    slider_row.append(&mute_btn);
    slider_row.append(&volume_scale);
    slider_row.append(&max_vol_btn);
    sound_section.append(&slider_row);
    popover_box.append(&sound_section);

    let sep_top = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    popover_box.append(&sep_top);

    // Sección "Salida" con lista real de dispositivos PipeWire y círculo azul activo
    let output_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(5)
        .build();

    let output_lbl = Label::builder()
        .label("Salida")
        .css_classes(["sound-menu-section"])
        .xalign(0.0)
        .margin_start(6)
        .margin_end(6)
        .build();
    output_section.append(&output_lbl);

    let devices_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();
    output_section.append(&devices_box);
    popover_box.append(&output_section);

    // Sección "Aplicaciones" (Mezclador por aplicación PipeWire, visible solo cuando hay flujos activos)
    let apps_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(5)
        .visible(false)
        .build();
    let apps_sep = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    let apps_lbl = Label::builder()
        .label("Aplicaciones")
        .css_classes(["sound-menu-section"])
        .xalign(0.0)
        .margin_start(6)
        .margin_end(6)
        .build();
    let apps_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .margin_start(6)
        .margin_end(6)
        .build();
    apps_section.append(&apps_sep);
    apps_section.append(&apps_lbl);
    apps_section.append(&apps_box);
    popover_box.append(&apps_section);

    let sep_bottom = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["sound-menu-sep"])
        .margin_start(6)
        .margin_end(6)
        .build();
    popover_box.append(&sep_bottom);

    // Footer: "Configuración de Sonido..." -> abre Malus DE Settings (--panel sound)
    let pref_btn = Button::builder()
        .css_classes(["sound-menu-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();
    pref_btn.set_child(Some(
        &Label::builder()
            .label("Configuración de Sonido...")
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .build(),
    ));
    crate::ui::system_menu::attach_tray_squircle_button(&pref_btn, false, false, 4, 6);

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
        if let Err(err) = command.args(["--panel", "sound"]).spawn() {
            tracing::error!("No se pudieron abrir los ajustes de Sonido de Malus DE: {err}");
        }
    });
    popover_box.append(&pref_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    // Lógica del Slider de volumen
    let is_updating_vol = Rc::new(RefCell::new(false));
    let is_upd_scale = is_updating_vol.clone();
    let low_ico_scale = vol_low_icon.clone();
    let tray_ico_scale = icon.clone();
    let last_tray_vol_feedback = Rc::new(RefCell::new(
        std::time::Instant::now() - std::time::Duration::from_secs(1),
    ));

    volume_scale.connect_value_changed(move |scale| {
        if *is_upd_scale.borrow() {
            return;
        }
        let val = scale.value();
        let tray_icon_name = if val < 1.0 {
            "audio-volume-muted-symbolic"
        } else if val < 35.0 {
            "audio-volume-low-symbolic"
        } else if val < 70.0 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        };
        low_ico_scale.set_icon_name(Some(if val < 1.0 {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-low-symbolic"
        }));
        tray_ico_scale.set_icon_name(Some(tray_icon_name));

        AudioService::set_volume(val);
        let now = std::time::Instant::now();
        if now.duration_since(*last_tray_vol_feedback.borrow())
            >= std::time::Duration::from_millis(180)
        {
            *last_tray_vol_feedback.borrow_mut() = now;
            AudioService::play_volume_feedback_if_enabled();
        }
    });

    // Botón izquierdo: silenciar / reactivar
    {
        let sc = volume_scale.clone();
        let li = vol_low_icon.clone();
        let ti = icon.clone();
        let lk = is_updating_vol.clone();
        mute_btn.connect_clicked(move |_| {
            AudioService::toggle_mute();
            let sc_c = sc.clone();
            let li_c = li.clone();
            let ti_c = ti.clone();
            let lk_c = lk.clone();
            glib::MainContext::default().spawn_local(async move {
                refresh_volume_tray_ui(&sc_c, &li_c, &ti_c, &lk_c).await;
            });
        });
    }

    // Botón derecho: subir volumen (+10%)
    {
        let sc = volume_scale.clone();
        max_vol_btn.connect_clicked(move |_| {
            let next = (sc.value() + 10.0).min(100.0);
            sc.set_value(next);
        });
    }

    let last_tray_mixer_write = Rc::new(RefCell::new(
        std::time::Instant::now() - std::time::Duration::from_secs(5),
    ));
    let refresh_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_devices_and_vol: Rc<dyn Fn()> = {
        let sc = volume_scale.clone();
        let li = vol_low_icon.clone();
        let ti = icon.clone();
        let lk = is_updating_vol.clone();
        let dev_box = devices_box.clone();
        let app_sec = apps_section.clone();
        let app_box = apps_box.clone();
        let lw = last_tray_mixer_write.clone();
        let rf_h = refresh_holder.clone();
        Rc::new(move || {
            let sc_c = sc.clone();
            let li_c = li.clone();
            let ti_c = ti.clone();
            let lk_c = lk.clone();
            let dev_box_c = dev_box.clone();
            let app_sec_c = app_sec.clone();
            let app_box_c = app_box.clone();
            let lw_c = lw.clone();
            let rf_c = rf_h.clone();
            glib::MainContext::default().spawn_local(async move {
                refresh_volume_tray_ui(&sc_c, &li_c, &ti_c, &lk_c).await;
                let devices = AudioService::list_output_devices().await;
                populate_sound_output_rows(&dev_box_c, &devices, &sc_c, &li_c, &ti_c, &lk_c);
                let streams = AudioService::list_app_streams().await;
                while let Some(child) = app_box_c.first_child() {
                    app_box_c.remove(&child);
                }
                if streams.is_empty() {
                    app_sec_c.set_visible(false);
                } else {
                    let on_routed: Rc<dyn Fn()> = {
                        let rf_inner = rf_c.clone();
                        Rc::new(move || {
                            if let Some(rf) = rf_inner.borrow().as_ref() {
                                rf();
                            }
                        })
                    };
                    for stream in &streams {
                        app_box_c.append(&super::control_center::build_per_app_mixer_row(
                            stream,
                            &devices,
                            &lw_c,
                            on_routed.clone(),
                        ));
                    }
                    app_sec_c.set_visible(true);
                }
            });
        })
    };
    *refresh_holder.borrow_mut() = Some(refresh_devices_and_vol.clone());

    // Actualizar al abrir Popover + suscripción reactiva en vivo (`pactl subscribe`) mientras permanezca abierto
    let tray_sound_watcher: Rc<RefCell<Option<crate::services::audio::PactlWatcher>>> =
        Rc::new(RefCell::new(None));
    {
        let rf = refresh_devices_and_vol.clone();
        let sw_c = tray_sound_watcher.clone();
        let lw_c = last_tray_mixer_write.clone();
        let pop_weak = popover.downgrade();
        popover.connect_show(move |_| {
            rf();
            if sw_c.borrow().is_some() {
                return;
            }
            let (tx, rx) = async_channel::bounded::<crate::services::audio::PactlEventKind>(32);
            let watcher = crate::services::audio::PactlWatcher::start(tx);
            *sw_c.borrow_mut() = Some(watcher);
            let rf_loop = rf.clone();
            let lw_loop = lw_c.clone();
            let pop_w = pop_weak.clone();
            let pending_debounce = Rc::new(RefCell::new(false));
            glib::MainContext::default().spawn_local(async move {
                while let Ok(kind) = rx.recv().await {
                    if !pop_w.upgrade().is_some_and(|p| p.is_visible()) {
                        continue;
                    }
                    if matches!(kind, crate::services::audio::PactlEventKind::Change)
                        && std::time::Instant::now().duration_since(*lw_loop.borrow())
                            < std::time::Duration::from_millis(500)
                    {
                        continue;
                    }
                    if *pending_debounce.borrow() {
                        continue;
                    }
                    *pending_debounce.borrow_mut() = true;
                    let pd = pending_debounce.clone();
                    let rf_fire = rf_loop.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(180),
                        move || {
                            *pd.borrow_mut() = false;
                            rf_fire();
                        },
                    );
                }
            });
        });
    }
    {
        let sw_close = tray_sound_watcher.clone();
        popover.connect_closed(move |_| {
            *sw_close.borrow_mut() = None;
        });
    }

    // Actualización inicial y registro para refresco en vivo (teclas multimedia / IPC)
    let sc_init = volume_scale.clone();
    let li_init = vol_low_icon.clone();
    let ti_init = icon.clone();
    let lk_init = is_updating_vol.clone();
    let trigger_vol_refresh: Rc<dyn Fn()> = Rc::new(move || {
        let sc = sc_init.clone();
        let li = li_init.clone();
        let ti = ti_init.clone();
        let lk = lk_init.clone();
        glib::MainContext::default().spawn_local(async move {
            refresh_volume_tray_ui(&sc, &li, &ti, &lk).await;
        });
    });
    trigger_vol_refresh();
    VOLUME_TRAY_REFRESH.with(|cell| {
        *cell.borrow_mut() = Some(trigger_vol_refresh);
    });

    menu_btn
}

fn populate_sound_output_rows(
    devices_box: &GtkBox,
    devices: &[crate::services::audio::AudioOutputDevice],
    scale: &Scale,
    low_icon: &Image,
    tray_icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    while let Some(child) = devices_box.first_child() {
        devices_box.remove(&child);
    }

    for dev in devices {
        let row_btn = Button::builder()
            .css_classes(["sound-menu-device-row"])
            .has_frame(false)
            .halign(Align::Fill)
            .hexpand(true)
            .build();

        let row_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::Fill)
            .valign(Align::Center)
            .hexpand(true)
            .vexpand(false)
            .build();

        let circle = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .width_request(24)
            .height_request(24)
            .halign(Align::Start)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .css_classes(if dev.is_default {
                ["sound-menu-device-circle", "sound-menu-device-circle-active"].as_slice()
            } else {
                ["sound-menu-device-circle"].as_slice()
            })
            .build();
        let dev_img = Image::builder()
            .icon_name(dev.icon_name)
            .pixel_size(13)
            .width_request(24)
            .height_request(24)
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        circle.append(&dev_img);

        let name_lbl = Label::builder()
            .label(&dev.description)
            .css_classes(["sound-menu-device-name"])
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        row_box.append(&circle);
        row_box.append(&name_lbl);
        row_btn.set_child(Some(&row_box));
        crate::ui::system_menu::attach_tray_squircle_button(&row_btn, false, false, 2, 6);

        let sink_name = dev.name.clone();
        let box_c = devices_box.clone();
        let sc_c = scale.clone();
        let li_c = low_icon.clone();
        let ti_c = tray_icon.clone();
        let lk_c = is_updating.clone();
        row_btn.connect_clicked(move |_| {
            let sink_target = sink_name.clone();
            let box_r = box_c.clone();
            let sc_r = sc_c.clone();
            let li_r = li_c.clone();
            let ti_r = ti_c.clone();
            let lk_r = lk_c.clone();
            glib::MainContext::default().spawn_local(async move {
                AudioService::set_default_sink(sink_target).await;
                refresh_volume_tray_ui(&sc_r, &li_r, &ti_r, &lk_r).await;
                let updated = AudioService::list_output_devices().await;
                populate_sound_output_rows(&box_r, &updated, &sc_r, &li_r, &ti_r, &lk_r);
            });
        });

        devices_box.append(&row_btn);
    }
}

async fn refresh_volume_tray_ui(
    scale: &Scale,
    low_icon: &Image,
    tray_icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    let (vol, is_muted) = AudioService::get_volume().await;

    *is_updating.borrow_mut() = true;
    scale.set_value(if is_muted { 0.0 } else { vol });
    *is_updating.borrow_mut() = false;

    if is_muted || vol < 1.0 {
        low_icon.set_icon_name(Some("audio-volume-muted-symbolic"));
        tray_icon.set_icon_name(Some("audio-volume-muted-symbolic"));
    } else {
        low_icon.set_icon_name(Some("audio-volume-low-symbolic"));
        let icon_name = if vol < 35.0 {
            "audio-volume-low-symbolic"
        } else if vol < 70.0 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        };
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
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();
    down_btn.set_child(Some(
        &Label::builder()
            .label("Abrir carpeta Descargas...")
            .xalign(0.0)
            .hexpand(true)
            .build(),
    ));

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
        .tooltip_text("Búsqueda de Canopy (Alt + Espacio / Super + K)")
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
