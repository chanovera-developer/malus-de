use crate::wifi_pane::{
    detail_kv_row, make_help_circle_button, make_squircle_badge, make_squircle_button,
    wrap_squircle_card, wrap_squircle_round_control, RoundControlStyle,
    WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS,
};
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, EventControllerMotion, Image, Label,
    Orientation, PolicyType, Popover, ScrolledWindow, Spinner, Switch,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtAdapterInfo {
    pub path: String,
    pub name: String,
    pub alias: String,
    pub address: String,
    pub powered: bool,
    pub discoverable: bool,
    pub discovering: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtDeviceEntry {
    pub path: String,
    pub address: String,
    pub name: String,
    pub icon_name: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub battery: Option<u8>,
    pub rssi: Option<i16>,
    pub device_type_label: String,
}

#[derive(Debug, Clone, Default)]
pub struct BluetoothSnapshot {
    pub bluez_available: bool,
    pub adapter: Option<BtAdapterInfo>,
    pub my_devices: Vec<BtDeviceEntry>,
    pub nearby_devices: Vec<BtDeviceEntry>,
}

/// Construye el panel completo de **Bluetooth** inspirado fielmente en macOS Tahoe (`Bluetooth.png`).
pub fn create_bluetooth_pane(parent_window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
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
        .valign(Align::Start)
        .vexpand(false)
        .build();

    // =========================================================================
    // 1. Tarjeta Superior: Cabecera Bluetooth + Nota de visibilidad local
    // =========================================================================
    let top_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let header_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-header-section"])
        .build();

    let icon_badge = make_squircle_badge(
        34,
        "bluetooth-active-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
    );

    let header_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();

    let bt_title = Label::builder()
        .label("Bluetooth")
        .css_classes(["card-section-title"])
        .xalign(0.0)
        .build();

    let desc_lbl = Label::builder()
        .use_markup(true)
        .label("Conéctate a accesorios que puedes usar para transmitir música, teclear, jugar y otras actividades. <a href=\"details\">Obtén detalles...</a>")
        .css_classes(["card-section-sub", "bt-inline-desc"])
        .wrap(true)
        .xalign(0.0)
        .build();

    header_text.append(&bt_title);
    header_text.append(&desc_lbl);

    let bt_switch = Switch::builder()
        .valign(Align::Start)
        .css_classes(["macos-switch"])
        .build();

    header_row.append(&icon_badge);
    header_row.append(&header_text);
    header_row.append(&bt_switch);
    top_card.append(&header_row);

    let top_sep = GtkBox::builder()
        .css_classes(["wifi-row-separator"])
        .build();
    top_card.append(&top_sep);

    let visibility_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-active-section"])
        .build();
    let visibility_lbl = Label::builder()
        .label("Consultando adaptador Bluetooth...")
        .css_classes(["card-section-sub"])
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .build();
    visibility_row.append(&visibility_lbl);
    top_card.append(&visibility_row);

    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&status_banner);

    // =========================================================================
    // 2. Sección: Mis dispositivos + Botón '?' debajo a la derecha
    // =========================================================================
    let my_section_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let my_title = Label::builder()
        .label("Mis dispositivos")
        .css_classes(["wifi-section-title"])
        .xalign(0.0)
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let my_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();
    my_section_box.append(&my_title);
    my_section_box.append(&wrap_squircle_card(&my_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let help_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .valign(Align::Start)
        .vexpand(false)
        .margin_top(2)
        .build();
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Bluetooth");
    help_row.append(&help_wrap);
    my_section_box.append(&help_row);

    pane.append(&my_section_box);

    // =========================================================================
    // 3. Sección: Dispositivos cercanos + Spinner de búsqueda
    // =========================================================================
    let nearby_section_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .valign(Align::Start)
        .vexpand(false)
        .build();
    let nearby_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();
    let nearby_title = Label::builder()
        .label("Dispositivos cercanos")
        .css_classes(["wifi-section-title"])
        .hexpand(true)
        .xalign(0.0)
        .valign(Align::Center)
        .build();
    let scan_spinner = Spinner::builder()
        .spinning(false)
        .visible(false)
        .valign(Align::Center)
        .build();
    let rescan_btn = Button::builder()
        .icon_name("view-refresh-symbolic")
        .css_classes(["wifi-more-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Buscar dispositivos Bluetooth cercanos")
        .build();
    let rescan_wrap = wrap_squircle_round_control(&rescan_btn, 22, RoundControlStyle::SubtleHover);
    nearby_hdr.append(&nearby_title);
    nearby_hdr.append(&scan_spinner);
    nearby_hdr.append(&rescan_wrap);

    let nearby_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    nearby_section_box.append(&nearby_hdr);
    nearby_section_box.append(&wrap_squircle_card(&nearby_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&nearby_section_box);

    scrolled.set_child(Some(&pane));

    // =========================================================================
    // Estado compartido y refresco reactivo vía BlueZ D-Bus
    // =========================================================================
    let latest_snapshot: Rc<RefCell<Option<BluetoothSnapshot>>> = Rc::new(RefCell::new(None));
    let pending_ops: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let is_syncing_switch = Rc::new(Cell::new(false));
    let discovery_active = Rc::new(Cell::new(false));
    let refresh_gen = Rc::new(Cell::new(0u64));
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>> = Rc::new(RefCell::new(None));
    let rendered_my_devices: Rc<RefCell<Vec<BtDeviceEntry>>> =
        Rc::new(RefCell::new(Vec::new()));
    let rendered_nearby_devices: Rc<RefCell<Vec<BtDeviceEntry>>> =
        Rc::new(RefCell::new(Vec::new()));
    let remembered_my_devices: Rc<RefCell<HashMap<String, BtDeviceEntry>>> =
        Rc::new(RefCell::new(HashMap::new()));

    let win_ref = parent_window.clone();
    let snap_ref = latest_snapshot.clone();
    let ops_ref = pending_ops.clone();
    let sync_sw_ref = is_syncing_switch.clone();
    let gen_ref = refresh_gen.clone();
    let sw_ref = bt_switch.clone();
    let vis_ref = visibility_lbl.clone();
    let my_sec_ref = my_section_box.clone();
    let my_card_ref = my_card.clone();
    let near_sec_ref = nearby_section_box.clone();
    let near_card_ref = nearby_card.clone();
    let spinner_ref = scan_spinner.clone();
    let banner_ref = status_banner.clone();
    let slot_inner = refresh_slot.clone();
    let rendered_my_ref = rendered_my_devices.clone();
    let rendered_nearby_ref = rendered_nearby_devices.clone();
    let remembered_my_ref = remembered_my_devices.clone();

    let run_refresh: Rc<dyn Fn(bool)> = Rc::new(move |start_scan: bool| {
        let my_gen = gen_ref.get().wrapping_add(1);
        gen_ref.set(my_gen);

        let snap_dest = snap_ref.clone();
        let ops_c = ops_ref.clone();
        let sync_sw = sync_sw_ref.clone();
        let gen_check = gen_ref.clone();
        let sw = sw_ref.clone();
        let vis_lbl = vis_ref.clone();
        let my_sec = my_sec_ref.clone();
        let my_c = my_card_ref.clone();
        let near_sec = near_sec_ref.clone();
        let near_c = near_card_ref.clone();
        let spin = spinner_ref.clone();
        let banner = banner_ref.clone();
        let win = win_ref.clone();
        let slot = slot_inner.clone();
        let rendered_my = rendered_my_ref.clone();
        let rendered_nearby = rendered_nearby_ref.clone();
        let remembered_my = remembered_my_ref.clone();

        glib::spawn_future_local(async move {
            if start_scan {
                if let Err(err) = ensure_adapter_discovering(true).await {
                    banner.set_label(&format!("No se pudo iniciar la búsqueda: {err}"));
                    banner.set_visible(true);
                } else {
                    banner.set_visible(false);
                }
            }
            let mut snap = collect_bluetooth_snapshot().await;
            if gen_check.get() != my_gen {
                return;
            }

            // BlueZ puede omitir momentaneamente Paired/Trusted al emitir actualizaciones.
            // Conservamos los dispositivos ya enlazados en memoria para que Ajustes
            // no haga desaparecer filas durante el refresco cada 3 segundos.
            {
                let mut remembered = remembered_my.borrow_mut();
                for dev in &snap.my_devices {
                    remembered.insert(dev.path.clone(), dev.clone());
                }

                let remembered_paths: HashSet<String> =
                    remembered.keys().cloned().collect();
                snap.nearby_devices.retain(|dev| !remembered_paths.contains(&dev.path));
                snap.my_devices = remembered.values().cloned().collect();
                snap.my_devices.sort_by(|a, b| {
                    b.connected
                        .cmp(&a.connected)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            }
            *snap_dest.borrow_mut() = Some(snap.clone());

            let powered = snap.adapter.as_ref().is_some_and(|a| a.powered);
            sync_sw.set(true);
            sw.set_sensitive(snap.adapter.is_some());
            sw.set_active(powered);
            sync_sw.set(false);

            if !snap.bluez_available || snap.adapter.is_none() {
                vis_lbl.set_label("No se detectó ningún adaptador Bluetooth activo en el sistema.");
                my_sec.set_visible(false);
                near_sec.set_visible(false);
                spin.set_spinning(false);
                spin.set_visible(false);
                return;
            }

            let adapter = snap.adapter.as_ref().unwrap();
            if powered {
                vis_lbl.set_label(&format!(
                    "Este equipo estará visible como “{}” mientras la configuración de Bluetooth esté abierta.",
                    adapter.alias
                ));
                my_sec.set_visible(true);
                near_sec.set_visible(true);
                let scanning = adapter.discovering || start_scan;
                spin.set_visible(scanning);
                spin.set_spinning(scanning);
            } else {
                vis_lbl.set_label("Bluetooth está desactivado.");
                my_sec.set_visible(false);
                near_sec.set_visible(false);
                spin.set_spinning(false);
                spin.set_visible(false);
                return;
            }

            let trigger_reload: Rc<dyn Fn(bool)> = {
                let slot_c = slot.clone();
                Rc::new(move |scan| {
                    if let Some(ref f) = *slot_c.borrow() {
                        f(scan);
                    }
                })
            };

            if *rendered_my.borrow() != snap.my_devices {
                populate_bluetooth_devices_card(
                    &my_c,
                    &snap.my_devices,
                    true,
                    &ops_c,
                    &win,
                    &banner,
                    &trigger_reload,
                    &remembered_my,
                );
                *rendered_my.borrow_mut() = snap.my_devices.clone();
            }
            if *rendered_nearby.borrow() != snap.nearby_devices {
                populate_bluetooth_devices_card(
                    &near_c,
                    &snap.nearby_devices,
                    false,
                    &ops_c,
                    &win,
                    &banner,
                    &trigger_reload,
                    &remembered_my,
                );
                *rendered_nearby.borrow_mut() = snap.nearby_devices.clone();
            }
        });
    });

    *refresh_slot.borrow_mut() = Some(run_refresh.clone());

    // Descubrimiento activo mientras el panel esté mapeado; detenerlo limpiamente al salir
    let disc_map = discovery_active.clone();
    let ref_map = run_refresh.clone();
    pane.connect_map(move |_| {
        disc_map.set(true);
        ref_map(true);
    });

    let disc_unmap = discovery_active.clone();
    pane.connect_unmap(move |_| {
        if disc_unmap.replace(false) {
            glib::spawn_future_local(async move {
                let _ = ensure_adapter_discovering(false).await;
            });
        }
    });

    // Switch de encendido/apagado de Bluetooth
    let sync_sw_ev = is_syncing_switch.clone();
    let ref_sw = run_refresh.clone();
    let banner_sw = status_banner.clone();
    let snap_sw = latest_snapshot.clone();
    bt_switch.connect_state_set(move |_, state| {
        if sync_sw_ev.get() {
            return glib::Propagation::Proceed;
        }
        let adapter_path = snap_sw
            .borrow()
            .as_ref()
            .and_then(|s| s.adapter.as_ref().map(|a| a.path.clone()))
            .unwrap_or_else(|| "/org/bluez/hci0".to_string());
        banner_sw.set_label(if state {
            "Activando Bluetooth..."
        } else {
            "Desactivando Bluetooth..."
        });
        banner_sw.set_visible(true);
        let ref_after = ref_sw.clone();
        let b_after = banner_sw.clone();
        glib::spawn_future_local(async move {
            if let Err(err) = set_adapter_powered(&adapter_path, state).await {
                b_after.set_label(&format!("No se pudo cambiar Bluetooth: {err}"));
                b_after.set_visible(true);
            } else {
                b_after.set_visible(false);
            }
            ref_after(state);
        });
        glib::Propagation::Proceed
    });

    // Enlace "Obtén detalles..."
    let win_gd = parent_window.clone();
    let snap_gd = latest_snapshot.clone();
    desc_lbl.connect_activate_link(move |_, uri| {
        if uri == "details" {
            if let Some(ref snap) = *snap_gd.borrow() {
                show_bluetooth_adapter_details_dialog(&win_gd, snap);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });

    // Botón refrescar búsqueda cercana
    let ref_rescan = run_refresh.clone();
    rescan_btn.connect_clicked(move |_| {
        ref_rescan(true);
    });

    // Popover de ayuda '?'
    let help_popover = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    let help_pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .build();
    let help_lbl_pop = Label::builder()
        .label("• Pasa el cursor sobre un dispositivo para conectarte o desconectarte rápidamente.\n• Pulsa ⓘ en «Mis dispositivos» para consultar batería, confianza automática o olvidar el dispositivo.\n• Al mantener este panel abierto, el equipo permanece visible para emparejar nuevos accesorios cercanos.")
        .wrap(true)
        .max_width_chars(38)
        .xalign(0.0)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    help_pop_box.append(&help_lbl_pop);
    help_popover.set_child(Some(&help_pop_box));
    help_btn.set_popover(Some(&help_popover));

    // Refresco periódico mientras el panel esté visible
    let pane_timer = pane.clone();
    let ref_timer = run_refresh.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        if pane_timer.is_mapped() {
            // Consultar el estado de conexión sin reiniciar el escaneo ni
            // reconstruir la lista de dispositivos.
            ref_timer(false);
        }
        glib::ControlFlow::Continue
    });

    let public_refresh: Rc<dyn Fn()> = Rc::new(move || {
        run_refresh(true);
    });

    (scrolled, public_refresh)
}

fn populate_bluetooth_devices_card(
    card: &GtkBox,
    devices: &[BtDeviceEntry],
    is_my_devices: bool,
    pending_ops: &Rc<RefCell<HashSet<String>>>,
    parent_window: &ApplicationWindow,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
    remembered_my_devices: &Rc<RefCell<HashMap<String, BtDeviceEntry>>>,
) {
    while let Some(child) = card.first_child() {
        card.remove(&child);
    }

    if devices.is_empty() {
        let empty = Label::builder()
            .label(if is_my_devices {
                "No hay dispositivos guardados."
            } else {
                "Buscando..."
            })
            .css_classes(["card-section-sub"])
            .xalign(if is_my_devices { 0.0 } else { 0.5 })
            .halign(if is_my_devices {
                Align::Start
            } else {
                Align::Center
            })
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(14)
            .margin_end(14)
            .build();
        card.append(&empty);
        return;
    }

    for (idx, dev) in devices.iter().enumerate() {
        if idx > 0 {
            let sep = GtkBox::builder()
                .css_classes(["wifi-row-separator"])
                .build();
            card.append(&sep);
        }

        let row_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["bt-device-row"])
            .build();
        if is_my_devices && dev.connected {
            row_box.add_css_class("bt-device-row-active");
        }

        let dev_icon = Image::builder()
            .icon_name(&dev.icon_name)
            .pixel_size(24)
            .valign(Align::Center)
            .css_classes(["bt-device-icon"])
            .build();
        row_box.append(&dev_icon);

        let info_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(1)
            .hexpand(true)
            .valign(Align::Center)
            .build();

        let name_lbl = Label::builder()
            .label(&dev.name)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let in_progress = pending_ops.borrow().contains(&dev.path);
        let subtitle_text = if in_progress {
            if dev.connected {
                "Desconectando...".to_string()
            } else {
                "Conectando...".to_string()
            }
        } else if is_my_devices {
            format_my_device_subtitle(dev.connected, dev.battery)
        } else if let Some(rssi) = dev.rssi {
            format!("{} · Señal {} dBm", dev.device_type_label, rssi)
        } else {
            dev.device_type_label.clone()
        };

        let sub_lbl = Label::builder()
            .label(&subtitle_text)
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        info_col.append(&name_lbl);
        info_col.append(&sub_lbl);

        // Hacer clic en el área de texto/icono también permite conectar/desconectar directamente
        let row_click_btn = Button::builder()
            .child(&info_col)
            .has_frame(false)
            .hexpand(true)
            .css_classes(["wifi-row-btn"])
            .build();
        row_box.append(&row_click_btn);

        let right_controls = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .build();

        let action_label = if dev.connected {
            "Desconectar"
        } else {
            "Conectar"
        };
        let (action_wrap, action_btn) = make_squircle_button(action_label, false, None);
        action_btn.set_sensitive(!in_progress);
        action_wrap.set_visible(false);
        right_controls.append(&action_wrap);

        let motion = EventControllerMotion::new();
        let wrap_enter = action_wrap.clone();
        motion.connect_enter(move |_, _, _| {
            wrap_enter.set_visible(true);
        });
        let wrap_leave = action_wrap.clone();
        motion.connect_leave(move |_| {
            wrap_leave.set_visible(false);
        });
        row_box.add_controller(motion);

        let trigger_toggle: Rc<dyn Fn()> = {
            let dev_c = dev.clone();
            let ops_c = pending_ops.clone();
            let ban_c = banner.clone();
            let rel_c = trigger_reload.clone();
            Rc::new(move || {
                if ops_c.borrow().contains(&dev_c.path) {
                    return;
                }
                ops_c.borrow_mut().insert(dev_c.path.clone());
                rel_c(false);

                let d = dev_c.clone();
                let ops_done = ops_c.clone();
                let ban_done = ban_c.clone();
                let rel_done = rel_c.clone();
                glib::spawn_future_local(async move {
                    let res = if d.connected {
                        disconnect_bt_device(&d.path).await
                    } else if d.paired {
                        connect_bt_device(&d.path).await
                    } else {
                        pair_and_connect_bt_device(&d.path).await
                    };
                    ops_done.borrow_mut().remove(&d.path);
                    match res {
                        Ok(()) => ban_done.set_visible(false),
                        Err(err) => {
                            ban_done.set_label(&format!("Bluetooth ({}): {}", d.name, err));
                            ban_done.set_visible(true);
                        }
                    }
                    rel_done(false);
                });
            })
        };

        let t1 = trigger_toggle.clone();
        row_click_btn.connect_clicked(move |_| t1());
        let t2 = trigger_toggle.clone();
        action_btn.connect_clicked(move |_| t2());

        if is_my_devices {
            let info_glyph = Label::builder()
                .label("i")
                .css_classes(["bt-info-glyph"])
                .halign(Align::Center)
                .valign(Align::Center)
                .build();
            let info_btn = Button::builder()
                .child(&info_glyph)
                .css_classes(["wifi-more-btn"])
                .has_frame(false)
                .valign(Align::Center)
                .tooltip_text("Información y opciones del dispositivo")
                .build();
            let info_wrap = wrap_squircle_round_control(&info_btn, 20, RoundControlStyle::CircleOutline);
            right_controls.append(&info_wrap);

            let win_d = parent_window.clone();
            let dev_d = dev.clone();
            let ban_d = banner.clone();
            let rel_d = trigger_reload.clone();
            let remembered_d = remembered_my_devices.clone();
            info_btn.connect_clicked(move |_| {
                show_bt_device_details_dialog(
                    &win_d,
                    &dev_d,
                    &ban_d,
                    &rel_d,
                    &remembered_d,
                );
            });
        }

        row_box.append(&right_controls);
        card.append(&row_box);
    }
}

fn show_bluetooth_adapter_details_dialog(parent: &ApplicationWindow, snap: &BluetoothSnapshot) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Detalles de Bluetooth")
        .default_width(390)
        .build();

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
        .spacing(8)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();

    if let Some(ref adapter) = snap.adapter {
        card.append(&detail_kv_row("Nombre visible", &adapter.alias));
        card.append(&detail_kv_row("Controlador", &adapter.name));
        card.append(&detail_kv_row("Dirección MAC", &adapter.address));
        card.append(&detail_kv_row(
            "Estado",
            if adapter.powered {
                "Activado"
            } else {
                "Desactivado"
            },
        ));
        card.append(&detail_kv_row(
            "Visibilidad",
            if adapter.discoverable {
                "Visible para dispositivos cercanos"
            } else {
                "Oculto"
            },
        ));
        card.append(&detail_kv_row(
            "Dispositivos guardados",
            &snap.my_devices.len().to_string(),
        ));
    } else {
        card.append(&detail_kv_row("Adaptador", "No detectado"));
    }

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    ok_wrap.set_halign(Align::End);
    let d_c = dialog.clone();
    ok_btn.connect_clicked(move |_| d_c.close());
    vbox.append(&ok_wrap);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_bt_device_details_dialog(
    parent: &ApplicationWindow,
    dev: &BtDeviceEntry,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn(bool)>,
    remembered_my_devices: &Rc<RefCell<HashMap<String, BtDeviceEntry>>>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&dev.name)
        .default_width(400)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(20)
        .margin_end(20)
        .build();

    let hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let badge = make_squircle_badge(36, &dev.icon_name, 20, WIFI_BADGE_SQUIRCLE_RADIUS);
    let hdr_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    hdr_text.append(
        &Label::builder()
            .label(&dev.name)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr_text.append(
        &Label::builder()
            .label(&format_my_device_subtitle(dev.connected, dev.battery))
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    hdr.append(&badge);
    hdr.append(&hdr_text);
    vbox.append(&hdr);

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    card.append(&detail_kv_row("Tipo", &dev.device_type_label));
    card.append(&detail_kv_row("Dirección", &dev.address));
    if let Some(pct) = dev.battery {
        card.append(&detail_kv_row("Batería", &format!("{pct}%")));
    }

    let trust_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let trust_lbl = Label::builder()
        .label("Dispositivo de confianza (reconexión automática)")
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let trust_sw = Switch::builder()
        .active(dev.trusted)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    let path_trust = dev.path.clone();
    let rel_trust = trigger_reload.clone();
    trust_sw.connect_state_set(move |_, val| {
        let p = path_trust.clone();
        let r = rel_trust.clone();
        glib::spawn_future_local(async move {
            let _ = set_device_trusted(&p, val).await;
            r(false);
        });
        glib::Propagation::Proceed
    });
    trust_row.append(&trust_lbl);
    trust_row.append(&trust_sw);
    card.append(&trust_row);
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let (forget_wrap, forget_btn) =
        make_squircle_button("Olvidar este dispositivo...", false, None);
    let d_f = dialog.clone();
    let dev_path_f = dev.path.clone();
    let dev_name_f = dev.name.clone();
    let ban_f = banner.clone();
    let rel_f = trigger_reload.clone();
    let remembered_f = remembered_my_devices.clone();
    forget_btn.connect_clicked(move |_| {
        d_f.close();
        let p = dev_path_f.clone();
        let n = dev_name_f.clone();
        let b = ban_f.clone();
        let r = rel_f.clone();
        let remembered_done = remembered_f.clone();
        glib::spawn_future_local(async move {
            if let Err(err) = remove_bt_device(&p).await {
                b.set_label(&format!("No se pudo olvidar {n}: {err}"));
                b.set_visible(true);
            } else {
                remembered_done.borrow_mut().remove(&p);
                b.set_visible(false);
            }
            r(false);
        });
    });
    footer.append(&forget_wrap);

    let spacer = GtkBox::builder().hexpand(true).build();
    footer.append(&spacer);

    let (toggle_wrap, toggle_btn) = make_squircle_button(
        if dev.connected {
            "Desconectar"
        } else {
            "Conectar"
        },
        false,
        None,
    );
    let d_t = dialog.clone();
    let dev_t = dev.clone();
    let ban_t = banner.clone();
    let rel_t = trigger_reload.clone();
    toggle_btn.connect_clicked(move |_| {
        d_t.close();
        let d = dev_t.clone();
        let b = ban_t.clone();
        let r = rel_t.clone();
        glib::spawn_future_local(async move {
            let res = if d.connected {
                disconnect_bt_device(&d.path).await
            } else {
                connect_bt_device(&d.path).await
            };
            if let Err(err) = res {
                b.set_label(&format!("Bluetooth ({}): {}", d.name, err));
                b.set_visible(true);
            } else {
                b.set_visible(false);
            }
            r(false);
        });
    });
    footer.append(&toggle_wrap);

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    let d_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| d_ok.close());
    footer.append(&ok_wrap);

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

// =========================================================================
// Backend BlueZ (org.bluez D-Bus) y utilidades de clasificación testables
// =========================================================================

pub(crate) fn format_my_device_subtitle(connected: bool, battery: Option<u8>) -> String {
    match (connected, battery) {
        (true, Some(pct)) => format!("Conectado · Batería {pct}%"),
        (true, None) => "Conectado".to_string(),
        (false, _) => "Sin conexión".to_string(),
    }
}

pub(crate) fn resolve_bt_icon_and_type(raw_icon: &str, name: &str) -> (String, String) {
    let icon_l = raw_icon.to_lowercase();
    let name_l = name.to_lowercase();
    if icon_l.contains("audio-headset")
        || icon_l.contains("audio-headphones")
        || name_l.contains("airpods")
        || name_l.contains("buds")
        || name_l.contains("headset")
        || name_l.contains("headphones")
        || name_l.contains("wh-")
        || name_l.contains("wf-")
        || name_l.contains("beats")
    {
        ("audio-headset-symbolic".to_string(), "Auriculares".to_string())
    } else if icon_l.contains("audio-card") || icon_l.contains("speaker") || name_l.contains("speaker") {
        ("audio-speakers-symbolic".to_string(), "Altavoz".to_string())
    } else if icon_l.contains("input-keyboard") || name_l.contains("keyboard") || name_l.contains("teclado") {
        ("input-keyboard-symbolic".to_string(), "Teclado".to_string())
    } else if icon_l.contains("input-mouse") || name_l.contains("mouse") || name_l.contains("ratón") || name_l.contains("raton") {
        ("input-mouse-symbolic".to_string(), "Ratón".to_string())
    } else if icon_l.contains("input-gaming") || name_l.contains("controller") || name_l.contains("gamepad") || name_l.contains("dualsense") || name_l.contains("xbox") {
        ("input-gaming-symbolic".to_string(), "Mando".to_string())
    } else if icon_l.contains("phone") || name_l.contains("iphone") || name_l.contains("pixel") || name_l.contains("galaxy") {
        ("phone-symbolic".to_string(), "Teléfono".to_string())
    } else if icon_l.contains("computer") || name_l.contains("macbook") || name_l.contains("laptop") {
        ("computer-symbolic".to_string(), "Ordenador".to_string())
    } else {
        ("bluetooth-active-symbolic".to_string(), "Dispositivo Bluetooth".to_string())
    }
}

pub(crate) fn classify_bluetooth_devices(raw_devices: Vec<BtDeviceEntry>) -> (Vec<BtDeviceEntry>, Vec<BtDeviceEntry>) {
    let mut my_devices = Vec::new();
    let mut nearby_devices = Vec::new();

    for dev in raw_devices {
        if dev.paired || dev.connected {
            my_devices.push(dev);
        } else if dev.rssi.is_some()
            || (!dev.address.trim().is_empty() && dev.name != dev.address)
        {
            nearby_devices.push(dev);
        }
    }

    my_devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    nearby_devices.sort_by(|a, b| {
        b.rssi
            .unwrap_or(i16::MIN)
            .cmp(&a.rssi.unwrap_or(i16::MIN))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    (my_devices, nearby_devices)
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

async fn collect_bluetooth_snapshot() -> BluetoothSnapshot {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(_) => return BluetoothSnapshot::default(),
    };
    let reply = match conn
        .call_method(
            Some("org.bluez"),
            "/",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return BluetoothSnapshot::default(),
    };

    let objects: ManagedObjects = match reply.body().deserialize() {
        Ok(o) => o,
        Err(_) => return BluetoothSnapshot::default(),
    };

    let mut adapter: Option<BtAdapterInfo> = None;
    let mut raw_devices = Vec::new();

    for (path, ifaces) in &objects {
        if let Some(ap) = ifaces.get("org.bluez.Adapter1") {
            let addr = ap.get("Address").and_then(extract_string).unwrap_or_default();
            let name = ap
                .get("Name")
                .and_then(extract_string)
                .unwrap_or_else(|| "Bluetooth".to_string());
            let alias = ap
                .get("Alias")
                .and_then(extract_string)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| name.clone());
            let powered = ap.get("Powered").and_then(extract_bool).unwrap_or(false);
            let discoverable = ap
                .get("Discoverable")
                .and_then(extract_bool)
                .unwrap_or(false);
            let discovering = ap
                .get("Discovering")
                .and_then(extract_bool)
                .unwrap_or(false);
            if adapter.is_none() || powered {
                adapter = Some(BtAdapterInfo {
                    path: path.as_str().to_string(),
                    name,
                    alias,
                    address: addr,
                    powered,
                    discoverable,
                    discovering,
                });
            }
        }
    }

    let adapter_prefix = adapter
        .as_ref()
        .map(|a| format!("{}/", a.path))
        .unwrap_or_else(|| "/org/bluez/".to_string());

    for (path, ifaces) in objects {
        let p_str = path.as_str();
        if !p_str.starts_with(&adapter_prefix) {
            continue;
        }
        if let Some(dp) = ifaces.get("org.bluez.Device1") {
            let address = dp.get("Address").and_then(extract_string).unwrap_or_default();
            let name = dp
                .get("Name")
                .or_else(|| dp.get("Alias"))
                .and_then(extract_string)
                .unwrap_or_else(|| address.clone());
            let paired = dp.get("Paired").and_then(extract_bool).unwrap_or(false);
            let trusted = dp.get("Trusted").and_then(extract_bool).unwrap_or(false);
            let connected = dp.get("Connected").and_then(extract_bool).unwrap_or(false);
            let raw_icon = dp.get("Icon").and_then(extract_string).unwrap_or_default();
            let (icon_name, device_type_label) = resolve_bt_icon_and_type(&raw_icon, &name);
            let battery = ifaces
                .get("org.bluez.Battery1")
                .and_then(|b| b.get("Percentage").and_then(extract_u8));
            let rssi = dp.get("RSSI").and_then(extract_i16);

            raw_devices.push(BtDeviceEntry {
                path: p_str.to_string(),
                address,
                name,
                icon_name,
                paired,
                trusted,
                connected,
                battery,
                rssi,
                device_type_label,
            });
        }
    }

    let (my_devices, nearby_devices) = classify_bluetooth_devices(raw_devices);
    BluetoothSnapshot {
        bluez_available: true,
        adapter,
        my_devices,
        nearby_devices,
    }
}

async fn find_first_adapter(conn: &Connection) -> Option<(String, bool, bool)> {
    let reply = conn
        .call_method(
            Some("org.bluez"),
            "/",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await
        .ok()?;
    let objects: ManagedObjects = reply.body().deserialize().ok()?;
    for (path, ifaces) in objects {
        if let Some(ap) = ifaces.get("org.bluez.Adapter1") {
            let powered = ap.get("Powered").and_then(extract_bool).unwrap_or(false);
            let discovering = ap
                .get("Discovering")
                .and_then(extract_bool)
                .unwrap_or(false);
            return Some((path.as_str().to_string(), powered, discovering));
        }
    }
    None
}

async fn ensure_adapter_discovering(enable: bool) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    let Some((adapter_path, powered, discovering)) = find_first_adapter(&conn).await else {
        return Err("BlueZ no encontró ningún adaptador".to_string());
    };
    if !powered {
        return Err("El adaptador Bluetooth está desactivado".to_string());
    }
    // Discovery (buscar otros dispositivos) no depende de la visibilidad del
    // equipo. Ajustar Discoverable por separado evita que un permiso o una
    // propiedad no admitida impida llamar a StartDiscovery.
    let _ = conn
        .call_method(
            Some("org.bluez"),
            adapter_path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &("org.bluez.Adapter1", "Discoverable", Value::from(enable)),
        )
        .await;
    if enable && !discovering {
        conn
            .call_method(
                Some("org.bluez"),
                adapter_path.as_str(),
                Some("org.bluez.Adapter1"),
                "StartDiscovery",
                &(),
            )
            .await
            .map_err(|e| e.to_string())?;
    } else if !enable && discovering {
        conn
            .call_method(
                Some("org.bluez"),
                adapter_path.as_str(),
                Some("org.bluez.Adapter1"),
                "StopDiscovery",
                &(),
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn set_adapter_powered(adapter_path: &str, powered: bool) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    conn.call_method(
        Some("org.bluez"),
        adapter_path,
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &("org.bluez.Adapter1", "Powered", Value::from(powered)),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn connect_bt_device(path: &str) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    conn.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), "Connect", &())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn disconnect_bt_device(path: &str) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.bluez.Device1"),
        "Disconnect",
        &(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn set_device_trusted(path: &str, trusted: bool) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &("org.bluez.Device1", "Trusted", Value::from(trusted)),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn pair_and_connect_bt_device(path: &str) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    conn.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), "Pair", &())
        .await
        .map_err(|e| e.to_string())?;
    let _ = set_device_trusted(path, true).await;
    connect_bt_device(path).await
}

async fn remove_bt_device(path: &str) -> Result<(), String> {
    let conn = Connection::system().await.map_err(|e| e.to_string())?;
    let adapter_path = path
        .rsplit_once('/')
        .map(|(a, _)| a)
        .unwrap_or("/org/bluez/hci0");
    let obj_path = OwnedObjectPath::try_from(path).map_err(|e| e.to_string())?;
    conn.call_method(
        Some("org.bluez"),
        adapter_path,
        Some("org.bluez.Adapter1"),
        "RemoveDevice",
        &(obj_path,),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_string(val: &OwnedValue) -> Option<String> {
    match &**val {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

fn extract_bool(val: &OwnedValue) -> Option<bool> {
    match &**val {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn extract_u8(val: &OwnedValue) -> Option<u8> {
    match &**val {
        Value::U8(n) => Some(*n),
        _ => None,
    }
}

fn extract_i16(val: &OwnedValue) -> Option<i16> {
    match &**val {
        Value::I16(n) => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paired_and_nearby_bluetooth_devices_cleanly() {
        let raw = vec![
            BtDeviceEntry {
                path: "/org/bluez/hci0/dev_11_22".into(),
                address: "11:22:33:44:55:66".into(),
                name: "Cristian Graciano’s AirPods".into(),
                icon_name: "audio-headset-symbolic".into(),
                paired: true,
                trusted: true,
                connected: false,
                battery: None,
                rssi: None,
                device_type_label: "Auriculares".into(),
            },
            BtDeviceEntry {
                path: "/org/bluez/hci0/dev_AA_BB".into(),
                address: "AA:BB:CC:DD:EE:FF".into(),
                name: "Keychron K3".into(),
                icon_name: "input-keyboard-symbolic".into(),
                paired: true,
                trusted: true,
                connected: true,
                battery: Some(85),
                rssi: None,
                device_type_label: "Teclado".into(),
            },
            BtDeviceEntry {
                path: "/org/bluez/hci0/dev_CC_DD".into(),
                address: "CC:DD:EE:FF:00:11".into(),
                name: "CC:DD:EE:FF:00:11".into(),
                icon_name: "bluetooth-active-symbolic".into(),
                paired: false,
                trusted: false,
                connected: false,
                battery: None,
                rssi: Some(-80),
                device_type_label: "Dispositivo Bluetooth".into(),
            },
            BtDeviceEntry {
                path: "/org/bluez/hci0/dev_EE_FF".into(),
                address: "EE:FF:00:11:22:33".into(),
                name: "Sony WH-1000XM5".into(),
                icon_name: "audio-headset-symbolic".into(),
                paired: false,
                trusted: false,
                connected: false,
                battery: None,
                rssi: Some(-52),
                device_type_label: "Auriculares".into(),
            },
        ];

        let (my, nearby) = classify_bluetooth_devices(raw);
        assert_eq!(my.len(), 2);
        assert_eq!(my[0].name, "Keychron K3");
        assert!(my[0].connected);
        assert_eq!(
            format_my_device_subtitle(my[0].connected, my[0].battery),
            "Conectado · Batería 85%"
        );
        assert_eq!(
            format_my_device_subtitle(my[1].connected, my[1].battery),
            "Sin conexión"
        );

        // Los cercanos pueden aparecer inicialmente solo con su dirección MAC.
        assert_eq!(nearby.len(), 2);
        assert_eq!(nearby[0].name, "Sony WH-1000XM5");
        assert_eq!(nearby[1].name, "CC:DD:EE:FF:00:11");
    }

    #[test]
    fn resolves_bluetooth_icons_and_types() {
        assert_eq!(
            resolve_bt_icon_and_type("", "Cristian’s AirPods"),
            ("audio-headset-symbolic".into(), "Auriculares".into())
        );
        assert_eq!(
            resolve_bt_icon_and_type("input-mouse", "MX Master 3S"),
            ("input-mouse-symbolic".into(), "Ratón".into())
        );
        assert_eq!(
            resolve_bt_icon_and_type("input-keyboard", "Magic Keyboard"),
            ("input-keyboard-symbolic".into(), "Teclado".into())
        );
    }
}
