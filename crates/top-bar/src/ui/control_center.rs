use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, MenuButton, Orientation, PolicyType, Popover,
    Scale, ScrolledWindow, Separator, Spinner, Stack, StackTransitionType, Switch,
};
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use tracing::info;

use crate::services::audio::{AudioService, MprisService};
use crate::services::bluetooth::{BluetoothService, BtDeviceInfo};
use crate::services::brightness::BrightnessService;
use crate::services::network::{NetworkService, WifiApInfo};

pub fn create_control_center_widget() -> MenuButton {
    let menu_btn = MenuButton::builder()
        .css_classes(["control-center-toggle", "topbar-btn"])
        .tooltip_text("Centro de Control")
        .has_frame(false)
        .always_show_arrow(false)
        .build();

    let cc_icon = Image::builder()
        .icon_name("control-center-symbolic")
        .pixel_size(16)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    menu_btn.set_child(Some(&cc_icon));

    let popover = Popover::builder()
        .css_classes(["control-center-popover"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::End)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let stack = Stack::builder()
        .transition_type(StackTransitionType::SlideLeftRight)
        .transition_duration(220)
        .interpolate_size(true)
        .width_request(330)
        .build();

    // ==========================================
    // Vista Principal ("main")
    // ==========================================
    let main_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .spacing(10)
        .build();

    // 1. Matriz 2x2 de Módulos Superiores
    let grid_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .build();

    let row1 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let row2 = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    // --- Tile 1: Wi-Fi ---
    let (wifi_tile, wifi_power_btn, wifi_nav_btn, wifi_icon, wifi_sub) =
        create_cc_nav_tile("network-wireless-symbolic", "Wi-Fi", "Buscando...");
    row1.append(&wifi_tile);

    // --- Tile 2: Bluetooth ---
    let (bt_tile, bt_power_btn, bt_nav_btn, bt_icon, bt_sub) =
        create_cc_nav_tile("bluetooth-active-symbolic", "Bluetooth", "Verificando...");
    row1.append(&bt_tile);

    // --- Tile 3: AirDrop ---
    let (airdrop_tile, airdrop_power_btn, airdrop_nav_btn, airdrop_icon, airdrop_sub) =
        create_cc_nav_tile("network-wireless-hotspot-symbolic", "AirDrop", "Todos (10 min)");
    row2.append(&airdrop_tile);

    // --- Tile 4: Modo Oscuro ---
    let (dark_btn, dark_icon, dark_sub) =
        create_cc_tile("weather-clear-night-symbolic", "Modo Oscuro", "Activado");
    row2.append(&dark_btn);

    grid_box.append(&row1);
    grid_box.append(&row2);
    main_box.append(&grid_box);

    // 2. Tarjeta Deslizadora de Sonido (PipeWire / WirePlumber)
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
    let vol_icon = Image::from_icon_name("audio-volume-high-symbolic");
    mute_btn.set_child(Some(&vol_icon));

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
    main_box.append(&sound_card);

    // 2.5. Tarjeta Deslizadora de Pantalla con Brillo, Escalado, Disposición y Enlace Físico 1:1 (TrueSize Link)
    let display_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(8)
        .build();

    let display_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let disp_icon_btn = Button::builder()
        .css_classes(["cc-mute-btn"])
        .has_frame(false)
        .tooltip_text("Pantalla y Brillo")
        .build();
    let disp_icon = Image::from_icon_name("display-brightness-symbolic");
    disp_icon_btn.set_child(Some(&disp_icon));

    let disp_title = Label::builder()
        .label("Pantalla")
        .css_classes(["cc-tile-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let current_cfg = malus_de_common::config::DisplaysConfig::load();
    let disp_pct = Label::builder()
        .label(&format!("{}%", current_cfg.brightness.round() as i32))
        .css_classes(["cc-tile-subtitle"])
        .build();

    let disp_arrange_nav_btn = Button::builder()
        .css_classes(["cc-pill-btn", "cc-pill-btn-active"])
        .has_frame(false)
        .tooltip_text("Configurar disposición de pantallas, escalas individuales y enlace 1:1")
        .build();
    let disp_arrange_nav_lbl = Label::builder()
        .label("📐 Disposición y Escalas ›")
        .css_classes(["cc-tile-subtitle"])
        .build();
    disp_arrange_nav_btn.set_child(Some(&disp_arrange_nav_lbl));

    display_header.append(&disp_icon_btn);
    display_header.append(&disp_title);
    display_header.append(&disp_pct);
    display_header.append(&disp_arrange_nav_btn);

    // Deslizador de Brillo de Pantalla
    let brightness_scale = Scale::with_range(Orientation::Horizontal, 5.0, 100.0, 1.0);
    brightness_scale.set_value(current_cfg.brightness.clamp(5.0, 100.0));
    brightness_scale.set_draw_value(false);

    // Deslizador rápido de Escala Maestro / Monitor Primario (100% a 250%)
    let scale_row_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let scale_row_lbl = Label::builder()
        .label("Escala Global / Primaria")
        .css_classes(["cc-tile-subtitle"])
        .hexpand(true)
        .xalign(0.0)
        .margin_start(4)
        .build();
    let scale_row_val = Label::builder()
        .label(&format!("{:.1}%", current_cfg.master_scale * 100.0))
        .css_classes(["cc-tile-subtitle"])
        .build();
    scale_row_header.append(&scale_row_lbl);
    scale_row_header.append(&scale_row_val);

    let master_scale_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 5.0);
    master_scale_slider.set_value((current_cfg.master_scale * 100.0).clamp(100.0, 250.0));
    master_scale_slider.set_draw_value(false);

    // Fila de Acciones Rápidas (Modo Noche + TrueSize 1:1)
    let pills_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let night_shift_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Alternar Modo Noche (temperatura de color cálida)")
        .build();
    let night_shift_lbl = Label::builder()
        .label("🌙 Modo Noche")
        .css_classes(["cc-tile-subtitle"])
        .build();
    night_shift_btn.set_child(Some(&night_shift_lbl));

    let link_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Enlazar resolución física 1:1 pixel a pixel entre ambas pantallas (163.2 PPI ↔ 183.6 PPI)")
        .build();
    if current_cfg.linked_physical_1to1 {
        link_btn.add_css_class("cc-pill-btn-active");
    }
    let link_icon_label = Label::builder()
        .label(if current_cfg.linked_physical_1to1 { "🔗 TrueSize 1:1" } else { "🔓 Escala Libre" })
        .css_classes(["cc-tile-subtitle"])
        .build();
    link_btn.set_child(Some(&link_icon_label));

    pills_box.append(&night_shift_btn);
    pills_box.append(&link_btn);

    // Subtítulo con escala física y resolución lógica de ambos monitores
    let monitors_info = Label::builder()
        .label(&format_monitors_summary(&current_cfg))
        .css_classes(["cc-tile-subtitle"])
        .xalign(0.0)
        .margin_start(4)
        .wrap(true)
        .build();

    let is_updating_master_scale = Rc::new(RefCell::new(false));

    // Conectar el deslizador de escala rápida
    let is_upd_ms_c = is_updating_master_scale.clone();
    let scale_row_val_c = scale_row_val.clone();
    let monitors_info_ms_c = monitors_info.clone();
    master_scale_slider.connect_value_changed(move |slider| {
        if *is_upd_ms_c.borrow() {
            return;
        }
        let pct = (slider.value() / 5.0).round() * 5.0;
        let new_scale = (pct / 100.0).clamp(0.5, 3.0);
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        let primary_idx = cfg.outputs.iter().position(|o| o.primary).unwrap_or(0);
        cfg.set_output_scale(primary_idx, new_scale);
        let _ = cfg.save();
        let _ = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
        );
        scale_row_val_c.set_label(&format!("{:.1}%", cfg.master_scale * 100.0));
        monitors_info_ms_c.set_label(&format_monitors_summary(&cfg));
    });

    // Conectar el botón de enlace 1:1 (TrueSize Link)
    let link_btn_clone = link_btn.clone();
    let link_label_clone = link_icon_label.clone();
    let monitors_info_clone = monitors_info.clone();
    let scale_row_val_link = scale_row_val.clone();
    let master_slider_link = master_scale_slider.clone();
    let is_upd_ms_link = is_updating_master_scale.clone();
    link_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.linked_physical_1to1 = !cfg.linked_physical_1to1;
        if cfg.linked_physical_1to1 {
            cfg.apply_1to1_link();
            link_btn_clone.add_css_class("cc-pill-btn-active");
            link_label_clone.set_label("🔗 TrueSize 1:1");
        } else {
            cfg.recalculate_layout();
            link_btn_clone.remove_css_class("cc-pill-btn-active");
            link_label_clone.set_label("🔓 Escala Libre");
        }
        let _ = cfg.save();
        let _ = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
        );
        *is_upd_ms_link.borrow_mut() = true;
        master_slider_link.set_value((cfg.master_scale * 100.0).clamp(100.0, 250.0));
        *is_upd_ms_link.borrow_mut() = false;
        scale_row_val_link.set_label(&format!("{:.1}%", cfg.master_scale * 100.0));
        monitors_info_clone.set_label(&format_monitors_summary(&cfg));
    });

    // Conectar el botón de Modo Noche (Night Shift)
    let ns_btn_c = night_shift_btn.clone();
    let ns_lbl_c = night_shift_lbl.clone();
    night_shift_btn.connect_clicked(move |_| {
        let btn = ns_btn_c.clone();
        let lbl = ns_lbl_c.clone();
        glib::MainContext::default().spawn_local(async move {
            let current = BrightnessService::get_night_shift().await;
            BrightnessService::set_night_shift(!current);
            refresh_night_shift_ui_async(&btn, &lbl).await;
        });
    });

    // Conectar el deslizador de brillo
    let is_updating_brightness = Rc::new(RefCell::new(false));
    let is_upd_b_clone = is_updating_brightness.clone();
    let disp_pct_clone = disp_pct.clone();
    let disp_icon_clone = disp_icon.clone();
    brightness_scale.connect_value_changed(move |scale| {
        if *is_upd_b_clone.borrow() {
            return;
        }
        let val = scale.value();
        disp_pct_clone.set_label(&format!("{}%", val.round() as i32));
        if val < 35.0 {
            disp_icon_clone.set_icon_name(Some("display-brightness-low-symbolic"));
        } else if val < 70.0 {
            disp_icon_clone.set_icon_name(Some("display-brightness-medium-symbolic"));
        } else {
            disp_icon_clone.set_icon_name(Some("display-brightness-symbolic"));
        }
        BrightnessService::set_brightness(val);
    });

    display_card.append(&display_header);
    display_card.append(&brightness_scale);
    display_card.append(&scale_row_header);
    display_card.append(&master_scale_slider);
    display_card.append(&pills_box);
    display_card.append(&monitors_info);
    main_box.append(&display_card);

    // 2.6. Tarjeta de Distribución de Teclado (Soporte Español Latinoamericano 'latam' en vivo)
    let (keyboard_card, refresh_keyboard_card) = create_keyboard_card();
    main_box.append(&keyboard_card);

    // 3. Tarjeta Multimedia "En Reproducción" (MPRIS D-Bus)
    let (media_card, refresh_media) = create_mpris_media_card();
    main_box.append(&media_card);

    // 4. Fila de Accesos Rápidos Inferiores
    let bottom_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let spotlight_quick = Button::builder()
        .css_classes(["cc-quick-action-btn"])
        .label("🔍 Spotlight (Cmd + Espacio)")
        .hexpand(true)
        .has_frame(false)
        .build();

    let settings_quick = Button::builder()
        .css_classes(["cc-quick-action-btn"])
        .label("⚙️ Ajustes")
        .has_frame(false)
        .build();

    bottom_box.append(&spotlight_quick);
    bottom_box.append(&settings_quick);
    main_box.append(&bottom_box);

    stack.add_named(&main_box, Some("main"));

    // ==========================================
    // Subvistas de Navegación ("wifi", "bluetooth", "airdrop", "displays")
    // ==========================================
    let (wifi_subview, refresh_wifi) =
        create_wifi_subview(&stack, &popover, &wifi_power_btn, &wifi_icon, &wifi_sub);
    stack.add_named(&wifi_subview, Some("wifi"));

    let (bt_subview, refresh_bt) =
        create_bt_subview(&stack, &popover, &bt_power_btn, &bt_icon, &bt_sub);
    stack.add_named(&bt_subview, Some("bluetooth"));

    let (airdrop_subview, refresh_airdrop) = create_airdrop_subview(
        &stack,
        &popover,
        &airdrop_power_btn,
        &airdrop_icon,
        &airdrop_sub,
    );
    stack.add_named(&airdrop_subview, Some("airdrop"));

    let (displays_subview, refresh_displays_subview) = create_displays_subview(
        &stack,
        &monitors_info,
        &link_btn,
        &link_icon_label,
        &master_scale_slider,
        &scale_row_val,
        &is_updating_master_scale,
    );
    stack.add_named(&displays_subview, Some("displays"));

    popover.set_child(Some(&stack));
    menu_btn.set_popover(Some(&popover));

    // ==========================================
    // Conexiones de Navegación e Interactividad
    // ==========================================

    // Navegación Disposición y Escalas de Pantallas
    let stack_disp = stack.clone();
    let ref_disp = refresh_displays_subview.clone();
    disp_arrange_nav_btn.connect_clicked(move |_| {
        ref_disp();
        stack_disp.set_visible_child_name("displays");
    });

    // Navegación Wi-Fi
    let stack_wifi = stack.clone();
    let ref_wifi = refresh_wifi.clone();
    wifi_nav_btn.connect_clicked(move |_| {
        stack_wifi.set_visible_child_name("wifi");
        ref_wifi();
    });

    // Navegación Bluetooth
    let stack_bt = stack.clone();
    let ref_bt = refresh_bt.clone();
    bt_nav_btn.connect_clicked(move |_| {
        stack_bt.set_visible_child_name("bluetooth");
        ref_bt();
    });

    // Navegación AirDrop
    let stack_ad = stack.clone();
    let ref_ad = refresh_airdrop.clone();
    airdrop_nav_btn.connect_clicked(move |_| {
        stack_ad.set_visible_child_name("airdrop");
        ref_ad();
    });

    // Wi-Fi Power Toggle directo (botón circular) con D-Bus
    let wifi_sub_c = wifi_sub.clone();
    let wifi_icon_c = wifi_icon.clone();
    let wifi_btn_c = wifi_power_btn.clone();
    wifi_power_btn.connect_clicked(move |_| {
        let btn = wifi_btn_c.clone();
        let ico = wifi_icon_c.clone();
        let sub = wifi_sub_c.clone();
        glib::MainContext::default().spawn_local(async move {
            let current = NetworkService::is_wifi_enabled().await;
            info!("Cambiando estado de Wi-Fi a: {}", !current);
            let _ = NetworkService::set_wifi_enabled(!current).await;
            update_wifi_ui_async(&btn, &ico, &sub).await;
        });
    });

    // Bluetooth Power Toggle directo (botón circular) con D-Bus
    let bt_sub_c = bt_sub.clone();
    let bt_icon_c = bt_icon.clone();
    let bt_btn_c = bt_power_btn.clone();
    bt_power_btn.connect_clicked(move |_| {
        let btn = bt_btn_c.clone();
        let ico = bt_icon_c.clone();
        let sub = bt_sub_c.clone();
        glib::MainContext::default().spawn_local(async move {
            let current = BluetoothService::is_powered().await;
            info!("Cambiando estado de Bluetooth a: {}", !current);
            let _ = BluetoothService::set_powered(!current).await;
            update_bluetooth_ui_async(&btn, &ico, &sub).await;
        });
    });

    // AirDrop Power Toggle directo (botón circular) con D-Bus
    let ad_sub_c = airdrop_sub.clone();
    let ad_ico_c = airdrop_icon.clone();
    let ad_btn_c = airdrop_power_btn.clone();
    airdrop_power_btn.connect_clicked(move |_| {
        let is_active = ad_btn_c.has_css_class("cc-tile-active");
        let new_mode = if is_active { "off" } else { "everyone" };
        info!("Cambiando estado de AirDrop a: {}", new_mode);
        let btn = ad_btn_c.clone();
        let ico = ad_ico_c.clone();
        let sub = ad_sub_c.clone();
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus(new_mode).await;
            update_airdrop_ui_mode(&btn, &ico, &sub, new_mode);
        });
    });

    // Modo Oscuro Toggle
    let dark_sub_c = dark_sub.clone();
    let dark_btn_c = dark_btn.clone();
    let dark_icon_c = dark_icon.clone();
    let popover_dark = popover.clone();
    dark_btn.connect_clicked(move |_| {
        let is_dark = is_dark_mode_enabled();
        apply_system_appearance_mode(!is_dark);
        crate::ui::theme::update_all_popovers_theme(!is_dark);
        update_dark_mode_ui(&dark_btn_c, &dark_icon_c, &dark_sub_c, Some(&popover_dark));
    });

    // Preferencias del Sistema (Ajustes nativos de Malus DE)
    let popover_c = popover.clone();
    settings_quick.connect_clicked(move |_| {
        popover_c.popdown();
        info!("Lanzando Ajustes del Sistema (malus-de-settings) desde Centro de Control...");
        let wayland_display =
            std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
        let mut command = Command::new(&exe);
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("GDK_BACKEND", "wayland")
            .env("GSK_RENDERER", "cairo")
            .spawn();
    });

    // Spotlight Quick Launch
    let popover_c2 = popover.clone();
    spotlight_quick.connect_clicked(move |_| {
        popover_c2.popdown();
        let wayland_display =
            std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
        info!("Invocando Spotlight desde Centro de Control...");
        let mut command = Command::new("malus-de-spotlight");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command
            .env("WAYLAND_DISPLAY", wayland_display)
            .env("GDK_BACKEND", "wayland")
            .env("GSK_RENDERER", "cairo")
            .spawn();
    });

    // Control de Volumen con debouncing y AudioService
    let is_updating_volume = Rc::new(RefCell::new(false));
    let is_upd_clone = is_updating_volume.clone();
    let pct_clone = sound_pct.clone();
    let vol_icon_clone = vol_icon.clone();

    volume_scale.connect_value_changed(move |scale| {
        if *is_upd_clone.borrow() {
            return;
        }
        let val = scale.value();
        pct_clone.set_label(&format!("{}%", val.round() as i32));

        if val < 1.0 {
            vol_icon_clone.set_icon_name(Some("audio-volume-muted-symbolic"));
        } else if val < 40.0 {
            vol_icon_clone.set_icon_name(Some("audio-volume-low-symbolic"));
        } else if val < 75.0 {
            vol_icon_clone.set_icon_name(Some("audio-volume-medium-symbolic"));
        } else {
            vol_icon_clone.set_icon_name(Some("audio-volume-high-symbolic"));
        }

        AudioService::set_volume(val);
    });

    // Botón Silenciar
    let scale_c = volume_scale.clone();
    let is_upd_mute = is_updating_volume.clone();
    let pct_c = sound_pct.clone();
    let icon_c = vol_icon.clone();
    mute_btn.connect_clicked(move |_| {
        AudioService::toggle_mute();
        let scale = scale_c.clone();
        let pct = pct_c.clone();
        let icon = icon_c.clone();
        let lock = is_upd_mute.clone();
        glib::MainContext::default().spawn_local(async move {
            refresh_volume_ui_async(&scale, &pct, &icon, &lock).await;
        });
    });

    // Actualización de estado al abrir el Popover (reseteando vista a "main")
    let stack_pop = stack.clone();
    let w_btn = wifi_power_btn.clone();
    let w_ico = wifi_icon.clone();
    let w_sub = wifi_sub.clone();
    let b_btn = bt_power_btn.clone();
    let b_ico = bt_icon.clone();
    let b_sub = bt_sub.clone();
    let a_btn = airdrop_power_btn.clone();
    let a_ico = airdrop_icon.clone();
    let a_sub = airdrop_sub.clone();
    let d_btn = dark_btn.clone();
    let d_ico = dark_icon.clone();
    let d_sub = dark_sub.clone();
    let v_scale = volume_scale.clone();
    let v_pct = sound_pct.clone();
    let v_ico = vol_icon.clone();
    let v_lock = is_updating_volume.clone();
    let ref_media_c = refresh_media.clone();
    let ref_kb_c = refresh_keyboard_card.clone();
    let ref_disp_c = refresh_displays_subview.clone();

    let br_scale = brightness_scale.clone();
    let br_pct = disp_pct.clone();
    let br_ico = disp_icon.clone();
    let br_lock = is_updating_brightness.clone();
    let ns_btn = night_shift_btn.clone();
    let ns_lbl = night_shift_lbl.clone();

    popover.connect_show(move |pop| {
        stack_pop.set_visible_child_name("main");
        update_dark_mode_ui(&d_btn, &d_ico, &d_sub, Some(pop));
        ref_media_c();
        ref_kb_c();
        ref_disp_c();

        let wb = w_btn.clone();
        let wi = w_ico.clone();
        let ws = w_sub.clone();
        let bb = b_btn.clone();
        let bi = b_ico.clone();
        let bs = b_sub.clone();
        let ab = a_btn.clone();
        let ai = a_ico.clone();
        let asub = a_sub.clone();
        let vs = v_scale.clone();
        let vp = v_pct.clone();
        let vi = v_ico.clone();
        let vl = v_lock.clone();
        let brs = br_scale.clone();
        let brp = br_pct.clone();
        let bri = br_ico.clone();
        let brl = br_lock.clone();
        let nsb = ns_btn.clone();
        let nsl = ns_lbl.clone();

        glib::MainContext::default().spawn_local(async move {
            update_wifi_ui_async(&wb, &wi, &ws).await;
            update_bluetooth_ui_async(&bb, &bi, &bs).await;
            refresh_volume_ui_async(&vs, &vp, &vi, &vl).await;
            refresh_brightness_ui_async(&brs, &brp, &bri, &brl).await;
            refresh_night_shift_ui_async(&nsb, &nsl).await;
            let mode = get_airdrop_mode_dbus().await;
            update_airdrop_ui_mode(&ab, &ai, &asub, &mode);
        });
    });

    // Inicializar estado una vez al crearse
    update_dark_mode_ui(&dark_btn, &dark_icon, &dark_sub, Some(&popover));
    let wb_init = wifi_power_btn.clone();
    let wi_init = wifi_icon.clone();
    let ws_init = wifi_sub.clone();
    let bb_init = bt_power_btn.clone();
    let bi_init = bt_icon.clone();
    let bs_init = bt_sub.clone();
    let ab_init = airdrop_power_btn.clone();
    let ai_init = airdrop_icon.clone();
    let as_init = airdrop_sub.clone();
    let vs_init = volume_scale.clone();
    let vp_init = sound_pct.clone();
    let vi_init = vol_icon.clone();
    let vl_init = is_updating_volume.clone();
    let brs_init = brightness_scale.clone();
    let brp_init = disp_pct.clone();
    let bri_init = disp_icon.clone();
    let brl_init = is_updating_brightness.clone();
    let nsb_init = night_shift_btn.clone();
    let nsl_init = night_shift_lbl.clone();

    glib::MainContext::default().spawn_local(async move {
        update_wifi_ui_async(&wb_init, &wi_init, &ws_init).await;
        update_bluetooth_ui_async(&bb_init, &bi_init, &bs_init).await;
        refresh_volume_ui_async(&vs_init, &vp_init, &vi_init, &vl_init).await;
        refresh_brightness_ui_async(&brs_init, &brp_init, &bri_init, &brl_init).await;
        refresh_night_shift_ui_async(&nsb_init, &nsl_init).await;
        let mode = get_airdrop_mode_dbus().await;
        update_airdrop_ui_mode(&ab_init, &ai_init, &as_init, &mode);
    });

    menu_btn
}

// ==========================================
// Constructores de Tarjetas y Subvistas
// ==========================================

fn create_cc_tile(icon_name: &str, title: &str, subtitle: &str) -> (Button, Image, Label) {
    let btn = Button::builder()
        .css_classes(["cc-tile"])
        .has_frame(false)
        .build();

    let box_content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();

    let icon_box = GtkBox::builder()
        .css_classes(["cc-icon-circle"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    let icon = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    icon_box.append(&icon);

    let text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .spacing(1)
        .build();

    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["cc-tile-title"])
        .xalign(0.0)
        .build();

    let sub_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["cc-tile-subtitle"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(11)
        .build();

    text_box.append(&title_lbl);
    text_box.append(&sub_lbl);

    box_content.append(&icon_box);
    box_content.append(&text_box);
    btn.set_child(Some(&box_content));

    (btn, icon, sub_lbl)
}

fn create_cc_nav_tile(
    icon_name: &str,
    title: &str,
    subtitle: &str,
) -> (GtkBox, Button, Button, Image, Label) {
    let tile_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-tile", "cc-nav-tile"])
        .spacing(6)
        .build();

    let power_btn = Button::builder()
        .css_classes(["cc-tile-power-btn", "cc-icon-circle"])
        .has_frame(false)
        .valign(Align::Center)
        .halign(Align::Start)
        .tooltip_text("Activar / Desactivar")
        .build();

    let icon = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    power_btn.set_child(Some(&icon));

    let nav_btn = Button::builder()
        .css_classes(["cc-tile-nav-btn"])
        .has_frame(false)
        .hexpand(true)
        .valign(Align::Fill)
        .tooltip_text("Ver opciones")
        .build();

    let nav_content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .hexpand(true)
        .build();

    let text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .hexpand(true)
        .spacing(1)
        .build();

    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["cc-tile-title"])
        .xalign(0.0)
        .build();

    let sub_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["cc-tile-subtitle"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(11)
        .build();

    text_box.append(&title_lbl);
    text_box.append(&sub_lbl);

    let chevron = Image::builder()
        .icon_name("go-next-symbolic")
        .css_classes(["cc-chevron"])
        .pixel_size(12)
        .valign(Align::Center)
        .halign(Align::End)
        .build();

    nav_content.append(&text_box);
    nav_content.append(&chevron);
    nav_btn.set_child(Some(&nav_content));

    tile_box.append(&power_btn);
    tile_box.append(&nav_btn);

    (tile_box, power_btn, nav_btn, icon, sub_lbl)
}

// ==========================================
// Tarjeta Multimedia (MPRIS D-Bus)
// ==========================================

fn create_mpris_media_card() -> (GtkBox, Rc<dyn Fn()>) {
    let card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-media-card"])
        .spacing(10)
        .build();

    let art_box = GtkBox::builder()
        .css_classes(["cc-media-art"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let art_icon = Image::builder()
        .icon_name("audio-x-generic-symbolic")
        .pixel_size(24)
        .build();
    art_box.append(&art_icon);

    let info_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .hexpand(true)
        .spacing(2)
        .build();

    let title_lbl = Label::builder()
        .label("Sin reproducción activa")
        .css_classes(["cc-media-title"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(18)
        .build();

    let artist_lbl = Label::builder()
        .label("Multimedia")
        .css_classes(["cc-media-subtitle"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(18)
        .build();

    info_box.append(&title_lbl);
    info_box.append(&artist_lbl);

    let controls_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(Align::Center)
        .spacing(4)
        .build();

    let prev_btn = Button::builder()
        .css_classes(["cc-media-btn"])
        .has_frame(false)
        .tooltip_text("Pista anterior")
        .build();
    let prev_icon = Image::from_icon_name("media-skip-backward-symbolic");
    prev_btn.set_child(Some(&prev_icon));

    let play_btn = Button::builder()
        .css_classes(["cc-media-btn", "cc-media-play-btn"])
        .has_frame(false)
        .tooltip_text("Reproducir / Pausar")
        .build();
    let play_icon = Image::from_icon_name("media-playback-start-symbolic");
    play_btn.set_child(Some(&play_icon));

    let next_btn = Button::builder()
        .css_classes(["cc-media-btn"])
        .has_frame(false)
        .tooltip_text("Siguiente pista")
        .build();
    let next_icon = Image::from_icon_name("media-skip-forward-symbolic");
    next_btn.set_child(Some(&next_icon));

    controls_box.append(&prev_btn);
    controls_box.append(&play_btn);
    controls_box.append(&next_btn);

    card.append(&art_box);
    card.append(&info_box);
    card.append(&controls_box);

    let current_bus = Rc::new(RefCell::new(String::new()));
    let t_c = title_lbl.clone();
    let a_c = artist_lbl.clone();
    let pi_c = play_icon.clone();
    let bus_c = current_bus.clone();

    let refresh = Rc::new(move || {
        let t = t_c.clone();
        let a = a_c.clone();
        let pi = pi_c.clone();
        let bus = bus_c.clone();

        glib::MainContext::default().spawn_local(async move {
            if let Some(media) = MprisService::get_current_media().await {
                *bus.borrow_mut() = media.player_bus.clone();
                t.set_label(&media.title);
                a.set_label(&format!("{} • {}", media.artist, media.player_name));
                if media.is_playing {
                    pi.set_icon_name(Some("media-playback-pause-symbolic"));
                } else {
                    pi.set_icon_name(Some("media-playback-start-symbolic"));
                }
            } else {
                *bus.borrow_mut() = String::new();
                t.set_label("Sin reproducción activa");
                a.set_label("Multimedia");
                pi.set_icon_name(Some("media-playback-start-symbolic"));
            }
        });
    });

    let b_prev = current_bus.clone();
    let r_prev = refresh.clone();
    prev_btn.connect_clicked(move |_| {
        let bus = b_prev.borrow().clone();
        if !bus.is_empty() {
            let r = r_prev.clone();
            glib::MainContext::default().spawn_local(async move {
                MprisService::previous(&bus).await;
                glib::timeout_future(std::time::Duration::from_millis(150)).await;
                r();
            });
        }
    });

    let b_play = current_bus.clone();
    let r_play = refresh.clone();
    play_btn.connect_clicked(move |_| {
        let bus = b_play.borrow().clone();
        if !bus.is_empty() {
            let r = r_play.clone();
            glib::MainContext::default().spawn_local(async move {
                MprisService::play_pause(&bus).await;
                glib::timeout_future(std::time::Duration::from_millis(150)).await;
                r();
            });
        }
    });

    let b_next = current_bus.clone();
    let r_next = refresh.clone();
    next_btn.connect_clicked(move |_| {
        let bus = b_next.borrow().clone();
        if !bus.is_empty() {
            let r = r_next.clone();
            glib::MainContext::default().spawn_local(async move {
                MprisService::next(&bus).await;
                glib::timeout_future(std::time::Duration::from_millis(150)).await;
                r();
            });
        }
    });

    refresh();
    (card, refresh)
}

// ==========================================
// Subvista AirDrop (Link D-Bus)
// ==========================================

fn create_airdrop_subview(
    stack: &Stack,
    popover: &Popover,
    main_ad_btn: &Button,
    main_ad_icon: &Image,
    main_ad_sub: &Label,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    // Header: Back btn + Title
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    let title_lbl = Label::builder()
        .label("AirDrop")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    header.append(&back_btn);
    header.append(&title_lbl);
    view_box.append(&header);

    let desc_lbl = Label::builder()
        .label("AirDrop te permite compartir archivos al instante con dispositivos Apple o Linux cercanos.")
        .css_classes(["cc-airdrop-desc"])
        .wrap(true)
        .xalign(0.0)
        .build();
    view_box.append(&desc_lbl);

    // Opciones de visibilidad
    let (row_off, check_off) =
        create_airdrop_option_row("No recibir", "Nadie puede descubrir este equipo");
    let (row_contacts, check_contacts) =
        create_airdrop_option_row("Solo contactos", "Solo dispositivos conocidos pueden enviarte");
    let (row_everyone, check_everyone) =
        create_airdrop_option_row("Todos durante 10 min", "Visible temporalmente para todos");

    view_box.append(&row_off);
    view_box.append(&row_contacts);
    view_box.append(&row_everyone);

    // Botón de prueba de simulación
    let sim_btn = Button::builder()
        .css_classes(["cc-airdrop-sim-btn"])
        .label("📲 Simular recepción (iPhone de Cristian)")
        .has_frame(false)
        .tooltip_text("Prueba la notificación nativa de archivo entrante por AirDrop")
        .build();

    sim_btn.connect_clicked(move |_| {
        glib::MainContext::default().spawn_local(async move {
            simulate_airdrop_receive_dbus("iPhone de Cristian", "presentacion_malus_de.pdf").await;
        });
    });
    view_box.append(&sim_btn);

    // Footer: Separador y acceso a Descargas
    let footer_sep = Separator::new(Orientation::Horizontal);
    let downloads_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Abrir carpeta Descargas...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    downloads_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let mut command = Command::new("nautilus");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.arg(glib::home_dir().join("Descargas")).spawn();
    });

    view_box.append(&footer_sep);
    view_box.append(&downloads_btn);

    // Manejador de cambio de opción
    let ro_c = row_off.clone();
    let rc_c = row_contacts.clone();
    let re_c = row_everyone.clone();
    let cho_c = check_off.clone();
    let chc_c = check_contacts.clone();
    let che_c = check_everyone.clone();
    let mb_c = main_ad_btn.clone();
    let mi_c = main_ad_icon.clone();
    let ms_c = main_ad_sub.clone();

    let update_selection = Rc::new(move |mode: &str| {
        ro_c.remove_css_class("cc-list-row-active");
        rc_c.remove_css_class("cc-list-row-active");
        re_c.remove_css_class("cc-list-row-active");

        cho_c.set_visible(false);
        chc_c.set_visible(false);
        che_c.set_visible(false);

        match mode {
            "contacts" => {
                rc_c.add_css_class("cc-list-row-active");
                chc_c.set_visible(true);
            }
            "everyone" => {
                re_c.add_css_class("cc-list-row-active");
                che_c.set_visible(true);
            }
            _ => {
                ro_c.add_css_class("cc-list-row-active");
                cho_c.set_visible(true);
            }
        }
        update_airdrop_ui_mode(&mb_c, &mi_c, &ms_c, mode);
    });

    // Conectar clics en filas
    let sel_off = update_selection.clone();
    row_off.connect_clicked(move |_| {
        sel_off("off");
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus("off").await;
        });
    });

    let sel_cont = update_selection.clone();
    row_contacts.connect_clicked(move |_| {
        sel_cont("contacts");
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus("contacts").await;
        });
    });

    let sel_every = update_selection.clone();
    row_everyone.connect_clicked(move |_| {
        sel_every("everyone");
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus("everyone").await;
        });
    });

    // Callback de refresco al navegar a la subvista
    let sel_refresh = update_selection.clone();
    let refresh_closure: Rc<dyn Fn()> = Rc::new(move || {
        let sel = sel_refresh.clone();
        glib::MainContext::default().spawn_local(async move {
            let mode = get_airdrop_mode_dbus().await;
            sel(&mode);
        });
    });

    (view_box, refresh_closure)
}

fn create_airdrop_option_row(title: &str, subtitle: &str) -> (Button, Image) {
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

async fn get_airdrop_mode_dbus() -> String {
    if let Ok(connection) = zbus::Connection::session().await {
        if let Ok(reply) = connection
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
    "everyone".to_string()
}

async fn set_airdrop_mode_dbus(mode: &str) {
    if let Ok(connection) = zbus::Connection::session().await {
        let _ = connection
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "SetAirDropMode",
                &(mode,),
            )
            .await;
    }
}

async fn simulate_airdrop_receive_dbus(sender: &str, file: &str) {
    if let Ok(connection) = zbus::Connection::session().await {
        let res = connection
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "SimulateAirDropReceive",
                &(sender, file),
            )
            .await;
        if res.is_ok() {
            return;
        }
    }

    // Fallback directo a notify-send
    let summary = format!("AirDrop de {}", sender);
    let body = format!("Compartió el archivo '{}'\nGuardado en ~/Descargas", file);
    let _ = Command::new("notify-send")
        .args([
            "--app-name=AirDrop",
            "--icon=network-wireless-hotspot-symbolic",
            &summary,
            &body,
        ])
        .spawn();
}

fn update_airdrop_ui_mode(btn: &Button, icon: &Image, sub: &Label, mode: &str) {
    match mode {
        "contacts" => {
            btn.add_css_class("cc-tile-active");
            if let Some(icon_box) = icon.parent() {
                icon_box.add_css_class("cc-icon-circle-active");
            }
            sub.set_label("Solo contactos");
        }
        "everyone" => {
            btn.add_css_class("cc-tile-active");
            if let Some(icon_box) = icon.parent() {
                icon_box.add_css_class("cc-icon-circle-active");
            }
            sub.set_label("Todos (10 min)");
        }
        _ => {
            btn.remove_css_class("cc-tile-active");
            if let Some(icon_box) = icon.parent() {
                icon_box.remove_css_class("cc-icon-circle-active");
            }
            sub.set_label("Desactivado");
        }
    }
}

// ==========================================
// Subvista Wi-Fi (NetworkManager D-Bus)
// ==========================================

fn create_wifi_subview(
    stack: &Stack,
    popover: &Popover,
    main_wifi_btn: &Button,
    main_wifi_icon: &Image,
    main_wifi_sub: &Label,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    // Header: Back btn + Title + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    let title_lbl = Label::builder()
        .label("Wi-Fi")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let wifi_switch = Switch::builder().valign(Align::Center).build();

    header.append(&back_btn);
    header.append(&title_lbl);
    header.append(&wifi_switch);
    view_box.append(&header);

    // Contenedor con Scroll para redes
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(180)
        .max_content_height(250)
        .propagate_natural_height(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    scrolled.set_child(Some(&content_box));
    view_box.append(&scrolled);

    // Footer: Separador y acceso a Configuración del Sistema
    let footer_sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Preferencias de Wi-Fi...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let mut command = Command::new("gnome-control-center");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.arg("wifi").spawn();
    });

    view_box.append(&footer_sep);
    view_box.append(&pref_btn);

    // Switch connect state
    let is_updating_switch = Rc::new(RefCell::new(false));
    let is_upd_sw = is_updating_switch.clone();
    let m_btn = main_wifi_btn.clone();
    let m_ico = main_wifi_icon.clone();
    let m_sub = main_wifi_sub.clone();
    let cb_for_sw = content_box.clone();
    let sw_for_sw = wifi_switch.clone();
    let st_for_sw = stack.clone();
    let po_for_sw = popover.clone();

    wifi_switch.connect_state_set(move |_sw, state| {
        if *is_upd_sw.borrow() {
            return glib::Propagation::Proceed;
        }
        info!("Switch Wi-Fi cambiado a: {}", state);

        let mb_inner = m_btn.clone();
        let mi_inner = m_ico.clone();
        let ms_inner = m_sub.clone();
        let cb_inner = cb_for_sw.clone();
        let sw_inner = sw_for_sw.clone();
        let upd_inner = is_upd_sw.clone();
        let st_inner = st_for_sw.clone();
        let po_inner = po_for_sw.clone();

        glib::MainContext::default().spawn_local(async move {
            let _ = NetworkService::set_wifi_enabled(state).await;
            update_wifi_ui_async(&mb_inner, &mi_inner, &ms_inner).await;
            trigger_wifi_scan(
                &cb_inner,
                &sw_inner,
                &upd_inner,
                &mb_inner,
                &mi_inner,
                &ms_inner,
                &st_inner,
                &po_inner,
            );
        });

        glib::Propagation::Proceed
    });

    let cb_clone = content_box.clone();
    let sw_clone = wifi_switch.clone();
    let is_upd_clone = is_updating_switch.clone();
    let mb_clone = main_wifi_btn.clone();
    let mi_clone = main_wifi_icon.clone();
    let ms_clone = main_wifi_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    let refresh_closure: Rc<dyn Fn()> = Rc::new(move || {
        trigger_wifi_scan(
            &cb_clone,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });

    (view_box, refresh_closure)
}

fn trigger_wifi_scan(
    content_box: &GtkBox,
    wifi_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Popover,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    let loading_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .halign(Align::Center)
        .valign(Align::Center)
        .margin_top(30)
        .margin_bottom(30)
        .build();

    let spinner = Spinner::builder().spinning(true).build();
    let lbl = Label::builder()
        .label("Buscando redes Wi-Fi...")
        .css_classes(["cc-empty-label"])
        .build();
    loading_box.append(&spinner);
    loading_box.append(&lbl);
    content_box.append(&loading_box);

    let cb_clone = content_box.clone();
    let sw_clone = wifi_switch.clone();
    let is_upd_clone = is_upd_sw.clone();
    let mb_clone = m_btn.clone();
    let mi_clone = m_ico.clone();
    let ms_clone = m_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    glib::MainContext::default().spawn_local(async move {
        let (enabled, networks) = NetworkService::scan_networks().await;

        *is_upd_clone.borrow_mut() = true;
        sw_clone.set_state(enabled);
        *is_upd_clone.borrow_mut() = false;

        update_wifi_ui_async(&mb_clone, &mi_clone, &ms_clone).await;

        populate_wifi_list(
            &cb_clone,
            enabled,
            networks,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });
}

fn populate_wifi_list(
    content_box: &GtkBox,
    enabled: bool,
    networks: Vec<WifiApInfo>,
    wifi_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Popover,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    if !enabled {
        let disabled_lbl = Label::builder()
            .label("El Wi-Fi está desactivado")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(40)
            .margin_bottom(40)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    // Red conectada actualmente (si existe)
    if let Some(active) = networks.iter().find(|n| n.in_use) {
        let active_header = Label::builder()
            .label("RED ACTUAL")
            .css_classes(["cc-section-label"])
            .xalign(0.0)
            .margin_start(4)
            .margin_top(4)
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

    // Cabecera de Otras Redes con botón de refresco
    let header_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-section-header-box"])
        .spacing(6)
        .build();

    let others_lbl = Label::builder()
        .label("OTRAS REDES")
        .css_classes(["cc-section-label"])
        .hexpand(true)
        .xalign(0.0)
        .margin_start(4)
        .build();

    let refresh_btn = Button::builder()
        .css_classes(["cc-refresh-btn"])
        .has_frame(false)
        .tooltip_text("Buscar redes Wi-Fi")
        .build();
    let ref_icon = Image::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_child(Some(&ref_icon));

    let cb_ref = content_box.clone();
    let sw_ref = wifi_switch.clone();
    let upd_ref = is_upd_sw.clone();
    let mb_ref = m_btn.clone();
    let mi_ref = m_ico.clone();
    let ms_ref = m_sub.clone();
    let st_ref = stack.clone();
    let po_ref = popover.clone();
    refresh_btn.connect_clicked(move |_| {
        trigger_wifi_scan(
            &cb_ref,
            &sw_ref,
            &upd_ref,
            &mb_ref,
            &mi_ref,
            &ms_ref,
            &st_ref,
            &po_ref,
        );
    });

    header_box.append(&others_lbl);
    header_box.append(&refresh_btn);
    content_box.append(&header_box);

    let other_networks: Vec<&WifiApInfo> = networks.iter().filter(|n| !n.in_use).collect();
    if other_networks.is_empty() {
        let empty_lbl = Label::builder()
            .label("No se detectaron otras redes")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .build();
        content_box.append(&empty_lbl);
    } else {
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

            let target_ssid = net.ssid.clone();
            let cb_c = content_box.clone();
            let sw_c = wifi_switch.clone();
            let upd_c = is_upd_sw.clone();
            let mb_c = m_btn.clone();
            let mi_c = m_ico.clone();
            let ms_c = m_sub.clone();
            let st_c = stack.clone();
            let po_c = popover.clone();

            row_btn.connect_clicked(move |_| {
                info!("Conectando a red Wi-Fi: {}", target_ssid);
                let target = target_ssid.clone();
                let cb_inner = cb_c.clone();
                let sw_inner = sw_c.clone();
                let upd_inner = upd_c.clone();
                let mb_inner = mb_c.clone();
                let mi_inner = mi_c.clone();
                let ms_inner = ms_c.clone();
                let st_inner = st_c.clone();
                let po_inner = po_c.clone();

                glib::MainContext::default().spawn_local(async move {
                    let _ = NetworkService::connect_to_ssid(&target).await;
                    trigger_wifi_scan(
                        &cb_inner,
                        &sw_inner,
                        &upd_inner,
                        &mb_inner,
                        &mi_inner,
                        &ms_inner,
                        &st_inner,
                        &po_inner,
                    );
                });
            });

            content_box.append(&row_btn);
        }
    }
}

// ==========================================
// Subvista Bluetooth (BlueZ D-Bus)
// ==========================================

fn create_bt_subview(
    stack: &Stack,
    popover: &Popover,
    main_bt_btn: &Button,
    main_bt_icon: &Image,
    main_bt_sub: &Label,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    // Header: Back btn + Title + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    let title_lbl = Label::builder()
        .label("Bluetooth")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let bt_switch = Switch::builder().valign(Align::Center).build();

    header.append(&back_btn);
    header.append(&title_lbl);
    header.append(&bt_switch);
    view_box.append(&header);

    // Contenedor con Scroll para dispositivos
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(180)
        .max_content_height(250)
        .propagate_natural_height(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    scrolled.set_child(Some(&content_box));
    view_box.append(&scrolled);

    // Footer: Separador y acceso a Configuración del Sistema
    let footer_sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Preferencias de Bluetooth...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.popdown();
        let mut command = Command::new("gnome-control-center");
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.arg("bluetooth").spawn();
    });

    view_box.append(&footer_sep);
    view_box.append(&pref_btn);

    // Switch connect state
    let is_updating_switch = Rc::new(RefCell::new(false));
    let is_upd_sw = is_updating_switch.clone();
    let m_btn = main_bt_btn.clone();
    let m_ico = main_bt_icon.clone();
    let m_sub = main_bt_sub.clone();
    let cb_for_sw = content_box.clone();
    let sw_for_sw = bt_switch.clone();
    let st_for_sw = stack.clone();
    let po_for_sw = popover.clone();

    bt_switch.connect_state_set(move |_sw, state| {
        if *is_upd_sw.borrow() {
            return glib::Propagation::Proceed;
        }
        info!("Switch Bluetooth cambiado a: {}", state);

        let mb_inner = m_btn.clone();
        let mi_inner = m_ico.clone();
        let ms_inner = m_sub.clone();
        let cb_inner = cb_for_sw.clone();
        let sw_inner = sw_for_sw.clone();
        let upd_inner = is_upd_sw.clone();
        let st_inner = st_for_sw.clone();
        let po_inner = po_for_sw.clone();

        glib::MainContext::default().spawn_local(async move {
            let _ = BluetoothService::set_powered(state).await;
            update_bluetooth_ui_async(&mb_inner, &mi_inner, &ms_inner).await;
            trigger_bt_scan(
                &cb_inner,
                &sw_inner,
                &upd_inner,
                &mb_inner,
                &mi_inner,
                &ms_inner,
                &st_inner,
                &po_inner,
            );
        });

        glib::Propagation::Proceed
    });

    let cb_clone = content_box.clone();
    let sw_clone = bt_switch.clone();
    let is_upd_clone = is_updating_switch.clone();
    let mb_clone = main_bt_btn.clone();
    let mi_clone = main_bt_icon.clone();
    let ms_clone = main_bt_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    let refresh_closure: Rc<dyn Fn()> = Rc::new(move || {
        trigger_bt_scan(
            &cb_clone,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });

    (view_box, refresh_closure)
}

fn trigger_bt_scan(
    content_box: &GtkBox,
    bt_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Popover,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    let loading_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .halign(Align::Center)
        .valign(Align::Center)
        .margin_top(30)
        .margin_bottom(30)
        .build();

    let spinner = Spinner::builder().spinning(true).build();
    let lbl = Label::builder()
        .label("Consultando dispositivos Bluetooth...")
        .css_classes(["cc-empty-label"])
        .build();
    loading_box.append(&spinner);
    loading_box.append(&lbl);
    content_box.append(&loading_box);

    let cb_clone = content_box.clone();
    let sw_clone = bt_switch.clone();
    let is_upd_clone = is_upd_sw.clone();
    let mb_clone = m_btn.clone();
    let mi_clone = m_ico.clone();
    let ms_clone = m_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    glib::MainContext::default().spawn_local(async move {
        let powered = BluetoothService::is_powered().await;
        let devices = BluetoothService::get_paired_devices().await;

        *is_upd_clone.borrow_mut() = true;
        sw_clone.set_state(powered);
        *is_upd_clone.borrow_mut() = false;

        update_bluetooth_ui_async(&mb_clone, &mi_clone, &ms_clone).await;

        populate_bt_list(
            &cb_clone,
            powered,
            devices,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });
}

fn populate_bt_list(
    content_box: &GtkBox,
    enabled: bool,
    devices: Vec<BtDeviceInfo>,
    bt_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Popover,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    if !enabled {
        let disabled_lbl = Label::builder()
            .label("El Bluetooth está desactivado")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(40)
            .margin_bottom(40)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    let header_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-section-header-box"])
        .spacing(6)
        .build();

    let dev_lbl = Label::builder()
        .label("DISPOSITIVOS VINCULADOS")
        .css_classes(["cc-section-label"])
        .hexpand(true)
        .xalign(0.0)
        .margin_start(4)
        .build();

    let refresh_btn = Button::builder()
        .css_classes(["cc-refresh-btn"])
        .has_frame(false)
        .tooltip_text("Actualizar dispositivos")
        .build();
    let ref_icon = Image::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_child(Some(&ref_icon));

    let cb_ref = content_box.clone();
    let sw_ref = bt_switch.clone();
    let upd_ref = is_upd_sw.clone();
    let mb_ref = m_btn.clone();
    let mi_ref = m_ico.clone();
    let ms_ref = m_sub.clone();
    let st_ref = stack.clone();
    let po_ref = popover.clone();
    refresh_btn.connect_clicked(move |_| {
        trigger_bt_scan(
            &cb_ref,
            &sw_ref,
            &upd_ref,
            &mb_ref,
            &mi_ref,
            &ms_ref,
            &st_ref,
            &po_ref,
        );
    });

    header_box.append(&dev_lbl);
    header_box.append(&refresh_btn);
    content_box.append(&header_box);

    if devices.is_empty() {
        let empty_lbl = Label::builder()
            .label("No hay dispositivos vinculados")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .build();
        content_box.append(&empty_lbl);
    } else {
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
                .spacing(10)
                .build();

            let dev_icon = Image::builder()
                .icon_name(&dev.icon_name)
                .pixel_size(18)
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
            let mb_c = m_btn.clone();
            let mi_c = m_ico.clone();
            let ms_c = m_sub.clone();
            let st_c = stack.clone();
            let po_c = popover.clone();

            row_btn.connect_clicked(move |_| {
                let p = path.clone();
                let cb_inner = cb_c.clone();
                let sw_inner = sw_c.clone();
                let upd_inner = upd_c.clone();
                let mb_inner = mb_c.clone();
                let mi_inner = mi_c.clone();
                let ms_inner = ms_c.clone();
                let st_inner = st_c.clone();
                let po_inner = po_c.clone();

                glib::MainContext::default().spawn_local(async move {
                    if is_connected {
                        BluetoothService::disconnect_device(&p).await;
                    } else {
                        BluetoothService::connect_device(&p).await;
                    }
                    trigger_bt_scan(
                        &cb_inner,
                        &sw_inner,
                        &upd_inner,
                        &mb_inner,
                        &mi_inner,
                        &ms_inner,
                        &st_inner,
                        &po_inner,
                    );
                });
            });

            content_box.append(&row_btn);
        }
    }
}

// ==========================================
// Utilidades Asíncronas de UI
// ==========================================

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

async fn update_wifi_ui_async(btn: &Button, icon: &Image, sub: &Label) {
    let enabled = NetworkService::is_wifi_enabled().await;
    if enabled {
        btn.add_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.add_css_class("cc-icon-circle-active");
        }
        if let Some(ssid) = NetworkService::get_active_wifi_ssid().await {
            sub.set_label(&ssid);
        } else {
            sub.set_label("Activado");
        }
    } else {
        btn.remove_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.remove_css_class("cc-icon-circle-active");
        }
        sub.set_label("Desactivado");
    }
}

async fn update_bluetooth_ui_async(btn: &Button, icon: &Image, sub: &Label) {
    let enabled = BluetoothService::is_powered().await;
    if enabled {
        btn.add_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.add_css_class("cc-icon-circle-active");
        }
        if let Some(dev_name) = BluetoothService::get_active_device_name().await {
            sub.set_label(&dev_name);
        } else {
            sub.set_label("Activado");
        }
    } else {
        btn.remove_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.remove_css_class("cc-icon-circle-active");
        }
        sub.set_label("Desactivado");
    }
}

fn is_dark_mode_enabled() -> bool {
    crate::ui::theme::is_dark_mode_enabled()
}

pub fn apply_system_appearance_mode(to_dark: bool) {
    let target = if to_dark { "prefer-dark" } else { "prefer-light" };
    info!("🎨 Aplicando modo de apariencia de Malus DE: {}", target);

    match malus_de_common::ipc::IpcClient::prepare_theme_transition() {
        Ok(true) => info!("✨ Fotograma anterior guardado para la transición de tema"),
        Ok(false) => tracing::warn!(
            "No se preparó la transición de tema; se aplicará el cambio sin animación"
        ),
        Err(error) => tracing::warn!(
            "No se pudo preparar la transición de tema: {}; se aplicará el cambio sin animación",
            error
        ),
    }

    // 1. Sincronizar las preferencias aisladas de GSettings, GTK y CSS de Malus DE.
    malus_de_common::config::sync_system_appearance(to_dark);

    // 2. Actualizar AdwStyleManager en el proceso actual.
    libadwaita::StyleManager::default().set_color_scheme(
        if to_dark {
            libadwaita::ColorScheme::ForceDark
        } else {
            libadwaita::ColorScheme::ForceLight
        }
    );

    // 3. Notificar a otros módulos mediante IPC
    let _ = malus_de_common::ipc::IpcClient::send_command(
        malus_de_common::ipc::IpcMessage::ThemeChanged {
            theme_name: "macOS".to_string(),
            is_dark: to_dark,
        }
    );
}

fn update_dark_mode_ui(btn: &Button, icon: &Image, sub: &Label, popover: Option<&Popover>) {
    let is_dark = is_dark_mode_enabled();
    if is_dark {
        btn.add_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.add_css_class("cc-icon-circle-active");
        }
        sub.set_label("Activado");
        if let Some(pop) = popover {
            pop.remove_css_class("light-popover");
        }
    } else {
        btn.remove_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.remove_css_class("cc-icon-circle-active");
        }
        sub.set_label("Desactivado");
        if let Some(pop) = popover {
            pop.add_css_class("light-popover");
        }
    }
}

async fn refresh_volume_ui_async(
    scale: &Scale,
    pct_lbl: &Label,
    icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    let (vol_val, is_muted) = AudioService::get_volume().await;

    *is_updating.borrow_mut() = true;
    scale.set_value(vol_val);
    *is_updating.borrow_mut() = false;

    if is_muted {
        pct_lbl.set_label("Silenciado");
        icon.set_icon_name(Some("audio-volume-muted-symbolic"));
    } else {
        pct_lbl.set_label(&format!("{}%", vol_val.round() as i32));
        if vol_val < 1.0 {
            icon.set_icon_name(Some("audio-volume-muted-symbolic"));
        } else if vol_val < 40.0 {
            icon.set_icon_name(Some("audio-volume-low-symbolic"));
        } else if vol_val < 75.0 {
            icon.set_icon_name(Some("audio-volume-medium-symbolic"));
        } else {
            icon.set_icon_name(Some("audio-volume-high-symbolic"));
        }
    }
}

async fn refresh_brightness_ui_async(
    scale: &Scale,
    pct_lbl: &Label,
    icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    let b = BrightnessService::get_brightness().await;

    *is_updating.borrow_mut() = true;
    scale.set_value(b);
    *is_updating.borrow_mut() = false;

    pct_lbl.set_label(&format!("{}%", b.round() as i32));
    if b < 35.0 {
        icon.set_icon_name(Some("display-brightness-low-symbolic"));
    } else if b < 70.0 {
        icon.set_icon_name(Some("display-brightness-medium-symbolic"));
    } else {
        icon.set_icon_name(Some("display-brightness-symbolic"));
    }
}

async fn refresh_night_shift_ui_async(btn: &Button, lbl: &Label) {
    let enabled = BrightnessService::get_night_shift().await;
    if enabled {
        btn.add_css_class("cc-pill-btn-active");
        lbl.set_label("🌙 Modo Noche: Activo");
    } else {
        btn.remove_css_class("cc-pill-btn-active");
        lbl.set_label("🌙 Modo Noche: Inactivo");
    }
}

fn format_monitors_summary(cfg: &malus_de_common::config::DisplaysConfig) -> String {
    if cfg.outputs.is_empty() {
        return format!("Escala global: {:.1}%", cfg.master_scale * 100.0);
    }
    let mut sorted: Vec<_> = cfg.outputs.iter().collect();
    sorted.sort_by_key(|o| o.x);
    let parts: Vec<String> = sorted
        .iter()
        .map(|o| {
            let (lw, lh) = o.logical_size();
            let short_name = if o.connector.contains("DP") || o.effective_diagonal() >= 26.5 {
                "LG 27\""
            } else {
                "Mi 24\""
            };
            format!(
                "{}{} ({}): {:.1}% · {}×{} @({}, {})",
                if o.primary { "★ " } else { "" },
                short_name,
                o.connector,
                o.scale * 100.0,
                lw,
                lh,
                o.x,
                o.y
            )
        })
        .collect();
    parts.join("  |  ")
}

// ==========================================
// Tarjeta de Teclado / Idioma (Español Latinoamericano 'latam')
// ==========================================

fn create_keyboard_card() -> (GtkBox, Rc<dyn Fn()>) {
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let icon_lbl = Label::builder()
        .label("⌨️ Teclado")
        .css_classes(["cc-tile-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let input_cfg = malus_de_common::config::InputConfig::load();
    let active_lbl = Label::builder()
        .label(input_cfg.layout_display_name())
        .css_classes(["cc-tile-subtitle"])
        .build();

    header.append(&icon_lbl);
    header.append(&active_lbl);

    let pills_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let btn_latam = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Español (Latinoamérica) — tecla ñ, acentos y símbolos latam")
        .build();
    btn_latam.set_child(Some(
        &Label::builder()
            .label("🇲🇽 Latam (ñ)")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    let btn_es = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Español (España / ISO)")
        .build();
    btn_es.set_child(Some(
        &Label::builder()
            .label("🇪🇸 España")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    let btn_us = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Inglés (EE. UU. / ANSI)")
        .build();
    btn_us.set_child(Some(
        &Label::builder()
            .label("🇺🇸 Inglés US")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    pills_row.append(&btn_latam);
    pills_row.append(&btn_es);
    pills_row.append(&btn_us);

    card.append(&header);
    card.append(&pills_row);

    let b_latam = btn_latam.clone();
    let b_es = btn_es.clone();
    let b_us = btn_us.clone();
    let a_lbl = active_lbl.clone();

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = malus_de_common::config::InputConfig::load();
        a_lbl.set_label(cfg.layout_display_name());
        b_latam.remove_css_class("cc-pill-btn-active");
        b_es.remove_css_class("cc-pill-btn-active");
        b_us.remove_css_class("cc-pill-btn-active");
        match cfg.keyboard_layout.as_str() {
            "latam" => b_latam.add_css_class("cc-pill-btn-active"),
            "es" => b_es.add_css_class("cc-pill-btn-active"),
            "us" => b_us.add_css_class("cc-pill-btn-active"),
            _ => b_latam.add_css_class("cc-pill-btn-active"),
        }
    });

    refresh();

    let apply_layout = |layout_code: &'static str, refresh_fn: Rc<dyn Fn()>| {
        move |_: &Button| {
            let mut cfg = malus_de_common::config::InputConfig::load();
            cfg.keyboard_layout = layout_code.to_string();
            cfg.keyboard_variant = String::new();
            let _ = cfg.save();
            let _ = malus_de_common::ipc::IpcClient::send_command(
                malus_de_common::ipc::IpcMessage::SetKeyboardLayout {
                    layout: layout_code.to_string(),
                    variant: String::new(),
                },
            );
            refresh_fn();
        }
    };

    btn_latam.connect_clicked(apply_layout("latam", refresh.clone()));
    btn_es.connect_clicked(apply_layout("es", refresh.clone()));
    btn_us.connect_clicked(apply_layout("us", refresh.clone()));

    (card, refresh)
}

// ==========================================
// Subvista 4: Disposición de Pantallas, Escalado Individual y Enlace 1:1 Pixel a Pixel
// ==========================================

fn create_displays_subview(
    stack: &Stack,
    main_monitors_info: &Label,
    main_link_btn: &Button,
    main_link_lbl: &Label,
    main_master_slider: &Scale,
    main_master_val_lbl: &Label,
    is_updating_master_scale: &Rc<RefCell<bool>>,
) -> (GtkBox, Rc<dyn Fn()>) {
    let subview = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    // 1. Header con botón de retorno
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver al Centro de Control")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let title_lbl = Label::builder()
        .label("📐 Disposición y Escalas Dual 4K")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    header.append(&back_btn);
    header.append(&title_lbl);
    subview.append(&header);

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    // 2. Contenedor con scroll para todos los controles de disposición y escalado
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(360)
        .max_content_height(460)
        .propagate_natural_height(true)
        .build();

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .build();

    // --- TARJETA A: DISPOSICIÓN FÍSICA DE PANTALLAS (ARRANGEMENT) ---
    let arrange_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let arrange_title = Label::builder()
        .label("🖥 Disposición de Pantallas (Orden y Alineación)")
        .css_classes(["cc-tile-title"])
        .xalign(0.0)
        .build();
    arrange_card.append(&arrange_title);

    // Representación visual lado a lado de Monitor Izquierdo y Monitor Derecho
    let visual_monitors_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .homogeneous(true)
        .build();

    let left_mon_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-pill-btn", "cc-pill-btn-active"])
        .spacing(2)
        .build();
    let left_mon_title = Label::builder()
        .label("⬅ Izquierda")
        .css_classes(["cc-tile-title"])
        .build();
    let left_mon_sub = Label::builder()
        .label("DP-1 · LG 27\"")
        .css_classes(["cc-tile-subtitle"])
        .wrap(true)
        .build();
    left_mon_box.append(&left_mon_title);
    left_mon_box.append(&left_mon_sub);

    let right_mon_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-pill-btn"])
        .spacing(2)
        .build();
    let right_mon_title = Label::builder()
        .label("➡ Derecha")
        .css_classes(["cc-tile-title"])
        .build();
    let right_mon_sub = Label::builder()
        .label("HDMI-A-1 · Mi 24\"")
        .css_classes(["cc-tile-subtitle"])
        .wrap(true)
        .build();
    right_mon_box.append(&right_mon_title);
    right_mon_box.append(&right_mon_sub);

    visual_monitors_row.append(&left_mon_box);
    visual_monitors_row.append(&right_mon_box);
    arrange_card.append(&visual_monitors_row);

    // Botones para Invertir Orden (Izquierda <-> Derecha) y Cambiar Principal
    let order_actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let swap_order_btn = Button::builder()
        .css_classes(["cc-pill-btn", "cc-pill-btn-active"])
        .has_frame(false)
        .tooltip_text("Intercambiar qué pantalla está a la izquierda y cuál a la derecha")
        .build();
    swap_order_btn.set_child(Some(
        &Label::builder()
            .label("⇄ Invertir Izq / Der")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    let toggle_primary_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Cambiar cuál monitor es el monitor principal (★)")
        .build();
    toggle_primary_btn.set_child(Some(
        &Label::builder()
            .label("★ Alternar Principal")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    order_actions_row.append(&swap_order_btn);
    order_actions_row.append(&toggle_primary_btn);
    arrange_card.append(&order_actions_row);

    // Alineación Vertical (Top, Center, Bottom, Desk y=519)
    let align_lbl = Label::builder()
        .label("Alineación vertical entre monitores:")
        .css_classes(["cc-tile-subtitle"])
        .xalign(0.0)
        .build();
    arrange_card.append(&align_lbl);

    let align_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .homogeneous(true)
        .build();

    let align_top_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("⬆ Arriba")
        .has_frame(false)
        .tooltip_text("Alinear bordes superiores (y = 0)")
        .build();
    let align_center_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("⬌ Centro")
        .has_frame(false)
        .tooltip_text("Centrar verticalmente ambas pantallas")
        .build();
    let align_bottom_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("⬇ Abajo")
        .has_frame(false)
        .tooltip_text("Alinear bordes inferiores")
        .build();
    let align_desk_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("🖥 Escritorio")
        .has_frame(false)
        .tooltip_text("Alineación física de escritorio (Mi 24\" a y = +519 px)")
        .build();

    align_row.append(&align_top_btn);
    align_row.append(&align_center_btn);
    align_row.append(&align_bottom_btn);
    align_row.append(&align_desk_btn);
    arrange_card.append(&align_row);

    // Ajuste fino de altura vertical (offset Y de Monitor 2)
    let fine_y_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();
    let nudge_up_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("🔼 Subir M2 (-50px)")
        .has_frame(false)
        .tooltip_text("Subir posición vertical del segundo monitor en 50 píxeles")
        .build();
    let nudge_down_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("🔽 Bajar M2 (+50px)")
        .has_frame(false)
        .tooltip_text("Bajar posición vertical del segundo monitor en 50 píxeles")
        .build();
    fine_y_row.append(&nudge_up_btn);
    fine_y_row.append(&nudge_down_btn);
    arrange_card.append(&fine_y_row);

    content.append(&arrange_card);

    // --- TARJETA B: BOTÓN DE ENLACE FÍSICO 1:1 PIXEL A PIXEL (TRUESIZE LINK) ---
    let link_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let sub_link_btn = Button::builder()
        .css_classes(["cc-pill-btn", "cc-pill-btn-active"])
        .has_frame(false)
        .tooltip_text("Enlaza ambas pantallas por densidad física (PPI) para que las ventanas midan exactamente los mismos milímetros al cruzar de un monitor a otro")
        .build();
    let sub_link_lbl = Label::builder()
        .label("🔗 Enlazar Resolución 1:1 Pixel a Pixel (Activo)")
        .css_classes(["cc-tile-title"])
        .build();
    sub_link_btn.set_child(Some(&sub_link_lbl));

    let link_presets_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let link_p1_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("Ajuste 1: 171% · 150%")
        .tooltip_text("Ajuste 1: LG 24\" a 171% y Mi 27\" a 150% (Texto nítido / Óptimo)")
        .has_frame(false)
        .build();

    let link_p2_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .label("Ajuste 2: 152.5% · 133.3%")
        .tooltip_text("Ajuste 2: LG 24\" a 152.5% y Mi 27\" a 133.33% (Más espacio)")
        .has_frame(false)
        .build();

    link_presets_row.append(&link_p1_btn);
    link_presets_row.append(&link_p2_btn);

    let link_desc_lbl = Label::builder()
        .label("Relación física: 163.2 PPI (27\") ↔ 183.6 PPI (24\") · Factor 1.125x")
        .css_classes(["cc-tile-subtitle"])
        .xalign(0.0)
        .wrap(true)
        .build();

    link_card.append(&sub_link_btn);
    link_card.append(&link_presets_row);
    link_card.append(&link_desc_lbl);
    content.append(&link_card);

    // --- TARJETA C: ESCALADO DE MONITOR 1 (DP-1 · LG Ultra HD 24") ---
    let mon0_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let mon0_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let mon0_title = Label::builder()
        .label("Monitor 1: DP-1 (LG 24\" 4K)")
        .css_classes(["cc-tile-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let mon0_val_lbl = Label::builder()
        .label("171.0% · 2246×1263")
        .css_classes(["cc-tile-subtitle"])
        .build();
    mon0_header.append(&mon0_title);
    mon0_header.append(&mon0_val_lbl);

    let mon0_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 5.0);
    mon0_slider.set_draw_value(false);

    let mon0_presets = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .homogeneous(true)
        .build();
    for &(pct_label, _) in &[
        ("100%", 1.0),
        ("125%", 1.25),
        ("152.5%", 1.525),
        ("171%", 1.71),
        ("200%", 2.0),
    ] {
        let p_btn = Button::builder()
            .css_classes(["cc-pill-btn"])
            .label(pct_label)
            .has_frame(false)
            .build();
        mon0_presets.append(&p_btn);
    }

    mon0_card.append(&mon0_header);
    mon0_card.append(&mon0_slider);
    mon0_card.append(&mon0_presets);
    content.append(&mon0_card);

    // --- TARJETA D: ESCALADO DE MONITOR 2 (HDMI-A-1 · Mi Monitor 27") ---
    let mon1_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let mon1_header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let mon1_title = Label::builder()
        .label("Monitor 2: HDMI-A-1 (Mi 27\" 4K)")
        .css_classes(["cc-tile-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let mon1_val_lbl = Label::builder()
        .label("150.0% · 2560×1440")
        .css_classes(["cc-tile-subtitle"])
        .build();
    mon1_header.append(&mon1_title);
    mon1_header.append(&mon1_val_lbl);

    let mon1_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 5.0);
    mon1_slider.set_draw_value(false);

    let mon1_presets = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .homogeneous(true)
        .build();
    for &(pct_label, _) in &[
        ("100%", 1.0),
        ("125%", 1.25),
        ("133.3%", 1.333333),
        ("150%", 1.5),
        ("200%", 2.0),
    ] {
        let p_btn = Button::builder()
            .css_classes(["cc-pill-btn"])
            .label(pct_label)
            .has_frame(false)
            .build();
        mon1_presets.append(&p_btn);
    }

    mon1_card.append(&mon1_header);
    mon1_card.append(&mon1_slider);
    mon1_card.append(&mon1_presets);
    content.append(&mon1_card);

    scrolled.set_child(Some(&content));
    subview.append(&scrolled);

    // Estado de bloqueo para evitar recursión al sincronizar sliders
    let is_syncing = Rc::new(RefCell::new(false));

    let l_title = left_mon_title.clone();
    let l_sub = left_mon_sub.clone();
    let r_title = right_mon_title.clone();
    let r_sub = right_mon_sub.clone();
    let s_link_btn = sub_link_btn.clone();
    let s_link_lbl = sub_link_lbl.clone();
    let l_p1_btn = link_p1_btn.clone();
    let l_p2_btn = link_p2_btn.clone();
    let m0_val = mon0_val_lbl.clone();
    let m0_sld = mon0_slider.clone();
    let m1_val = mon1_val_lbl.clone();
    let m1_sld = mon1_slider.clone();
    let main_info_c = main_monitors_info.clone();
    let main_l_btn_c = main_link_btn.clone();
    let main_l_lbl_c = main_link_lbl.clone();
    let main_ms_sld_c = main_master_slider.clone();
    let main_ms_val_c = main_master_val_lbl.clone();
    let is_upd_ms_c = is_updating_master_scale.clone();
    let sync_lock = is_syncing.clone();

    let refresh_ui: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = malus_de_common::config::DisplaysConfig::load();
        *sync_lock.borrow_mut() = true;

        // Actualizar disposición visual (Izquierda vs Derecha)
        let mut sorted: Vec<_> = cfg.outputs.iter().enumerate().collect();
        sorted.sort_by_key(|(_, o)| o.x);

        if let Some((_, left_o)) = sorted.first() {
            let (lw, lh) = left_o.logical_size();
            l_title.set_label(&format!(
                "⬅ Izq: {}{}",
                left_o.connector,
                if left_o.primary { " ★" } else { "" }
            ));
            l_sub.set_label(&format!(
                "{}\n{}×{} @({}, {}) · {:.1}%",
                left_o.name,
                lw,
                lh,
                left_o.x,
                left_o.y,
                left_o.scale * 100.0
            ));
        }
        if let Some((_, right_o)) = sorted.get(1) {
            let (rw, rh) = right_o.logical_size();
            r_title.set_label(&format!(
                "➡ Der: {}{}",
                right_o.connector,
                if right_o.primary { " ★" } else { "" }
            ));
            r_sub.set_label(&format!(
                "{}\n{}×{} @({}, {}) · {:.1}%",
                right_o.name,
                rw,
                rh,
                right_o.x,
                right_o.y,
                right_o.scale * 100.0
            ));
        }

        // Actualizar estado del botón de enlace 1:1 en subvista y tarjeta principal
        if cfg.linked_physical_1to1 {
            s_link_btn.add_css_class("cc-pill-btn-active");
            s_link_lbl.set_label("🔗 Enlace 1:1 Pixel a Pixel: ACTIVO");
            main_l_btn_c.add_css_class("cc-pill-btn-active");
            main_l_lbl_c.set_label("🔗 TrueSize 1:1");
        } else {
            s_link_btn.remove_css_class("cc-pill-btn-active");
            s_link_lbl.set_label("🔓 Escalas Independientes (Enlace 1:1 Inactivo)");
            main_l_btn_c.remove_css_class("cc-pill-btn-active");
            main_l_lbl_c.set_label("🔓 Escala Libre");
        }

        // Actualizar estado de los botones de Ajuste 1 y Ajuste 2
        let curr_preset = cfg.current_linked_preset();
        if curr_preset == Some(1) {
            l_p1_btn.add_css_class("cc-pill-btn-active");
            l_p2_btn.remove_css_class("cc-pill-btn-active");
        } else if curr_preset == Some(2) {
            l_p2_btn.add_css_class("cc-pill-btn-active");
            l_p1_btn.remove_css_class("cc-pill-btn-active");
        } else {
            l_p1_btn.remove_css_class("cc-pill-btn-active");
            l_p2_btn.remove_css_class("cc-pill-btn-active");
        }

        // Actualizar Monitor 0 (DP-1)
        if let Some(o0) = cfg.outputs.first() {
            let (w0, h0) = o0.logical_size();
            m0_val.set_label(&format!("{:.1}% · {}×{}", o0.scale * 100.0, w0, h0));
            m0_sld.set_value((o0.scale * 100.0).clamp(100.0, 250.0));
        }

        // Actualizar Monitor 1 (HDMI-A-1)
        if let Some(o1) = cfg.outputs.get(1) {
            let (w1, h1) = o1.logical_size();
            m1_val.set_label(&format!("{:.1}% · {}×{}", o1.scale * 100.0, w1, h1));
            m1_sld.set_value((o1.scale * 100.0).clamp(100.0, 250.0));
        }

        // Actualizar tarjeta principal
        *is_upd_ms_c.borrow_mut() = true;
        main_ms_sld_c.set_value((cfg.master_scale * 100.0).clamp(100.0, 250.0));
        *is_upd_ms_c.borrow_mut() = false;
        main_ms_val_c.set_label(&format!("{:.1}%", cfg.master_scale * 100.0));
        main_info_c.set_label(&format_monitors_summary(&cfg));

        *sync_lock.borrow_mut() = false;
    });

    refresh_ui();

    // Helper para guardar cambios, notificar al compositor en vivo y refrescar la UI
    let apply_and_notify = |cfg: malus_de_common::config::DisplaysConfig,
                            ref_fn: &Rc<dyn Fn()>| {
        let _ = cfg.save();
        let _ = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
        );
        ref_fn();
    };

    // Conectar Invertir Orden (Izquierda <-> Derecha)
    let ref_swap = refresh_ui.clone();
    swap_order_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.swap_horizontal_order();
        apply_and_notify(cfg, &ref_swap);
    });

    // Conectar Alternar Monitor Principal
    let ref_prim = refresh_ui.clone();
    toggle_primary_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        let current_primary = cfg.outputs.iter().position(|o| o.primary).unwrap_or(0);
        let next_primary = if cfg.outputs.len() > 1 {
            (current_primary + 1) % cfg.outputs.len()
        } else {
            0
        };
        cfg.set_primary_output(next_primary);
        apply_and_notify(cfg, &ref_prim);
    });

    // Conectar botones de Alineación Vertical
    for (btn, mode) in [
        (&align_top_btn, "top"),
        (&align_center_btn, "center"),
        (&align_bottom_btn, "bottom"),
        (&align_desk_btn, "desk"),
    ] {
        let ref_align = refresh_ui.clone();
        let mode_str = mode.to_string();
        btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.set_vertical_alignment(&mode_str);
            apply_and_notify(cfg, &ref_align);
        });
    }

    // Conectar ajuste fino vertical (-50px / +50px)
    let ref_up = refresh_ui.clone();
    nudge_up_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        let target_idx = if cfg.outputs.len() > 1 { 1 } else { 0 };
        cfg.adjust_vertical_offset(target_idx, -50);
        apply_and_notify(cfg, &ref_up);
    });

    let ref_down = refresh_ui.clone();
    nudge_down_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        let target_idx = if cfg.outputs.len() > 1 { 1 } else { 0 };
        cfg.adjust_vertical_offset(target_idx, 50);
        apply_and_notify(cfg, &ref_down);
    });

    // Conectar botones de Ajustes Predefinidos 1 y 2 en la subvista
    let ref_lp1 = refresh_ui.clone();
    link_p1_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.apply_linked_preset(1);
        apply_and_notify(cfg, &ref_lp1);
    });

    let ref_lp2 = refresh_ui.clone();
    link_p2_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.apply_linked_preset(2);
        apply_and_notify(cfg, &ref_lp2);
    });

    // Conectar botón de Enlace Físico 1:1 en la subvista
    let ref_link = refresh_ui.clone();
    sub_link_btn.connect_clicked(move |_| {
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.linked_physical_1to1 = !cfg.linked_physical_1to1;
        if cfg.linked_physical_1to1 {
            if cfg.current_linked_preset().is_none() {
                cfg.apply_linked_preset(1);
            } else {
                cfg.apply_1to1_link();
            }
        } else {
            cfg.recalculate_layout();
        }
        apply_and_notify(cfg, &ref_link);
    });

    // Conectar Slider de Monitor 0 (DP-1)
    let sync_m0 = is_syncing.clone();
    let ref_m0 = refresh_ui.clone();
    mon0_slider.connect_value_changed(move |sld| {
        if *sync_m0.borrow() {
            return;
        }
        let pct = (sld.value() / 2.5).round() * 2.5;
        let new_scale = (pct / 100.0).clamp(0.5, 3.0);
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.set_output_scale_independent(0, new_scale);
        apply_and_notify(cfg, &ref_m0);
    });

    // Conectar Presets de Monitor 0 (DP-1)
    let preset_scales_m0 = [1.0, 1.25, 1.525, 1.71, 2.0];
    let mut child_m0 = mon0_presets.first_child();
    let mut idx_m0 = 0;
    while let Some(w) = child_m0 {
        if let Ok(btn) = w.clone().downcast::<Button>() {
            let target_scale = preset_scales_m0.get(idx_m0).copied().unwrap_or(1.71);
            let ref_p0 = refresh_ui.clone();
            btn.connect_clicked(move |_| {
                let mut cfg = malus_de_common::config::DisplaysConfig::load();
                cfg.set_output_scale_independent(0, target_scale);
                apply_and_notify(cfg, &ref_p0);
            });
            idx_m0 += 1;
        }
        child_m0 = w.next_sibling();
    }

    // Conectar Slider de Monitor 1 (HDMI-A-1)
    let sync_m1 = is_syncing.clone();
    let ref_m1 = refresh_ui.clone();
    mon1_slider.connect_value_changed(move |sld| {
        if *sync_m1.borrow() {
            return;
        }
        let pct = (sld.value() / 2.5).round() * 2.5;
        let new_scale = (pct / 100.0).clamp(0.5, 3.0);
        let mut cfg = malus_de_common::config::DisplaysConfig::load();
        cfg.set_output_scale_independent(1, new_scale);
        apply_and_notify(cfg, &ref_m1);
    });

    // Conectar Presets de Monitor 1 (HDMI-A-1)
    let preset_scales_m1 = [1.0, 1.25, 1.333333, 1.5, 2.0];
    let mut child_m1 = mon1_presets.first_child();
    let mut idx_m1 = 0;
    while let Some(w) = child_m1 {
        if let Ok(btn) = w.clone().downcast::<Button>() {
            let target_scale = preset_scales_m1.get(idx_m1).copied().unwrap_or(1.6875);
            let ref_p1 = refresh_ui.clone();
            btn.connect_clicked(move |_| {
                let mut cfg = malus_de_common::config::DisplaysConfig::load();
                cfg.set_output_scale_independent(1, target_scale);
                apply_and_notify(cfg, &ref_p1);
            });
            idx_m1 += 1;
        }
        child_m1 = w.next_sibling();
    }

    (subview, refresh_ui)
}
