use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, FileDialog, FileFilter,
    Label, Orientation, PasswordEntry, PolicyType, Popover, ScrolledWindow, StringList, Switch,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

use crate::wifi_pane::{
    build_squircle_chevron_menu_button, detail_kv_row, make_help_circle_button,
    make_popover_action_row, make_squircle_badge, make_squircle_button, wrap_squircle_card,
    wrap_squircle_round_control, RoundControlStyle, WIFI_BADGE_SQUIRCLE_RADIUS,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnProtocolKind {
    L2tpIpsec,
    CiscoIpsec,
    Ikev2,
}

impl VpnProtocolKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::L2tpIpsec => "L2TP sobre IPSec",
            Self::CiscoIpsec => "Cisco IPSec",
            Self::Ikev2 => "IKEv2",
        }
    }

    pub fn nm_vpn_type(self) -> Option<&'static str> {
        match self {
            Self::L2tpIpsec => Some("l2tp"),
            Self::CiscoIpsec => Some("vpnc"),
            Self::Ikev2 => Some("libreswan"),
        }
    }

    pub fn required_fedora_package(self) -> &'static str {
        match self {
            Self::L2tpIpsec => "NetworkManager-l2tp",
            Self::CiscoIpsec => "NetworkManager-vpnc",
            Self::Ikev2 => "NetworkManager-libreswan",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpnProfileEntry {
    pub uuid: String,
    pub name: String,
    pub conn_type: String,
    pub protocol_label: String,
    pub device: String,
    pub connected: bool,
    pub connecting: bool,
    pub autoconnect: bool,
    pub remote_server: String,
}

#[derive(Debug, Clone, Default)]
pub struct VpnSnapshot {
    pub nm_available: bool,
    pub profiles: Vec<VpnProfileEntry>,
    pub installed_plugins: BTreeSet<String>,
}

/// Crea el panel de configuración de VPN inspirado en `assets/screenshots/VPN.png`.
pub fn create_vpn_pane(parent_window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
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

    // Tarjeta principal squircle: muestra "Sin configuraciones de VPN" si está vacía o los perfiles configurados.
    let profiles_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    pane.append(&wrap_squircle_card(
        &profiles_card_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Fila inferior bajo la tarjeta:
    // Izquierda: [Acerca de las redes VPN y la privacidad...]
    // Derecha: [+ ⌄] [?]
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(2)
        .build();

    let (privacy_wrap, privacy_btn) = make_squircle_button(
        "Acerca de las redes VPN y la privacidad...",
        false,
        Some("Información sobre enrutamiento de tráfico y privacidad en VPN"),
    );
    footer.append(&privacy_wrap);
    footer.append(&GtkBox::builder().hexpand(true).build());

    let (add_wrap, add_menu_btn) =
        build_squircle_chevron_menu_button("list-add-symbolic", "Añadir configuración de VPN");
    footer.append(&add_wrap);

    let (help_wrap, help_btn) = make_help_circle_button("Ayuda sobre VPN");
    let help_pop = build_vpn_help_popover();
    help_btn.set_popover(Some(&help_pop));
    footer.append(&help_wrap);

    pane.append(&footer);
    scrolled.set_child(Some(&pane));

    // =========================================================================
    // Estado reactivo y refresco vía nmcli
    // =========================================================================
    let latest_snap = Rc::new(RefCell::new(VpnSnapshot::default()));
    let reload_lock = Rc::new(Cell::new(false));
    let trigger_reload: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let run_reload: Rc<dyn Fn()> = {
        let lock_c = reload_lock.clone();
        let snap_ref = latest_snap.clone();
        let box_c = profiles_card_box.clone();
        let win_c = parent_window.clone();
        let ban_c = status_banner.clone();
        let trig_ref = trigger_reload.clone();

        Rc::new(move || {
            if lock_c.replace(true) {
                return;
            }
            let (tx, rx) = async_channel::bounded::<VpnSnapshot>(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(collect_vpn_snapshot());
            });

            let lock_d = lock_c.clone();
            let snap_d = snap_ref.clone();
            let box_d = box_c.clone();
            let win_d = win_c.clone();
            let ban_d = ban_c.clone();
            let trig_d = trig_ref.clone();

            glib::spawn_future_local(async move {
                if let Ok(snapshot) = rx.recv().await {
                    *snap_d.borrow_mut() = snapshot.clone();
                    if !snapshot.nm_available {
                        ban_d.set_label(
                            "NetworkManager (nmcli) no disponible o inaccesible en este entorno.",
                        );
                        ban_d.set_visible(true);
                    } else if ban_d.text().contains("NetworkManager (nmcli)") {
                        ban_d.set_visible(false);
                    }
                    let reload_cb: Rc<dyn Fn()> = Rc::new({
                        let t = trig_d.clone();
                        move || {
                            if let Some(f) = t.borrow().as_ref() {
                                f();
                            }
                        }
                    });
                    populate_vpn_profiles_card(&box_d, &snapshot, &win_d, &ban_d, &reload_cb);
                }
                lock_d.set(false);
            });
        })
    };
    *trigger_reload.borrow_mut() = Some(run_reload.clone());

    // Acciones estáticas: Privacidad y Menú [+ ⌄]
    {
        let win_c = parent_window.clone();
        privacy_btn.connect_clicked(move |_| {
            show_vpn_privacy_dialog(&win_c);
        });
    }
    {
        let add_pop =
            build_add_vpn_popover(parent_window, &latest_snap, &status_banner, &run_reload);
        add_menu_btn.set_popover(Some(&add_pop));
    }

    run_reload();
    (scrolled, run_reload)
}

fn populate_vpn_profiles_card(
    container: &GtkBox,
    snap: &VpnSnapshot,
    parent_window: &ApplicationWindow,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    if snap.profiles.is_empty() {
        let empty_lbl = Label::builder()
            .label("Sin configuraciones de VPN")
            .css_classes(["vpn-empty-label"])
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(true)
            .build();
        container.append(&empty_lbl);
        return;
    }

    for (idx, profile) in snap.profiles.iter().enumerate() {
        if idx > 0 {
            container.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["bt-device-row"])
            .build();

        let badge = make_squircle_badge(
            30,
            "network-vpn-symbolic",
            16,
            WIFI_BADGE_SQUIRCLE_RADIUS,
        );
        badge.set_valign(Align::Center);

        let text_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        text_col.append(
            &Label::builder()
                .label(&profile.name)
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build(),
        );

        let sub_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();
        let (dot_cls, status_txt) = if profile.connected {
            ("net-dot-green", "Conectado")
        } else if profile.connecting {
            ("net-dot-amber", "Conectando...")
        } else {
            ("net-dot-red", "Sin conexión")
        };
        let dot = Label::builder()
            .label("●")
            .css_classes([dot_cls])
            .valign(Align::Center)
            .build();
        let sub_lbl = Label::builder()
            .label(&format!("{status_txt} · {}", profile.protocol_label))
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build();
        sub_row.append(&dot);
        sub_row.append(&sub_lbl);
        text_col.append(&sub_row);

        let sw = Switch::builder()
            .active(profile.connected || profile.connecting)
            .css_classes(["macos-switch"])
            .valign(Align::Center)
            .build();
        let uuid_sw = profile.uuid.clone();
        let name_sw = profile.name.clone();
        let ban_sw = banner.clone();
        let rel_sw = trigger_reload.clone();
        sw.connect_state_set(move |s, active| {
            s.set_sensitive(false);
            let u = uuid_sw.clone();
            let n = name_sw.clone();
            let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
            std::thread::spawn(move || {
                let sub = if active { "up" } else { "down" };
                let res = run_nmcli(&["connection", sub, "uuid", &u]).map(|_| ());
                let _ = tx.send_blocking(res);
            });
            let s_d = s.clone();
            let b_d = ban_sw.clone();
            let r_d = rel_sw.clone();
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(())) => b_d.set_visible(false),
                    Ok(Err(err)) => {
                        b_d.set_label(&format!("VPN ({n}): {err}"));
                        b_d.set_visible(true);
                    }
                    Err(_) => {}
                }
                s_d.set_sensitive(true);
                r_d();
            });
            glib::Propagation::Proceed
        });

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
            .tooltip_text("Detalles y configuración de esta VPN")
            .build();
        let info_wrap =
            wrap_squircle_round_control(&info_btn, 20, RoundControlStyle::CircleOutline);

        let win_i = parent_window.clone();
        let prof_i = profile.clone();
        let ban_i = banner.clone();
        let rel_i = trigger_reload.clone();
        info_btn.connect_clicked(move |_| {
            show_vpn_details_dialog(&win_i, &prof_i, &ban_i, &rel_i);
        });

        row.append(&badge);
        row.append(&text_col);
        row.append(&sw);
        row.append(&info_wrap);
        container.append(&row);
    }
}

fn build_add_vpn_popover(
    parent_window: &ApplicationWindow,
    latest_snap: &Rc<RefCell<VpnSnapshot>>,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) -> Popover {
    let pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    for (label, proto) in [
        ("L2TP sobre IPSec...", VpnProtocolKind::L2tpIpsec),
        ("Cisco IPSec...", VpnProtocolKind::CiscoIpsec),
        ("IKEv2...", VpnProtocolKind::Ikev2),
    ] {
        let (wrap, btn) = make_popover_action_row(label, false, None);
        let pop_c = pop.clone();
        let win_c = parent_window.clone();
        let snap_c = latest_snap.clone();
        let ban_c = banner.clone();
        let rel_c = trigger_reload.clone();
        btn.connect_clicked(move |_| {
            pop_c.popdown();
            let plugins = snap_c.borrow().installed_plugins.clone();
            show_create_vpn_dialog(&win_c, proto, &plugins, &ban_c, &rel_c);
        });
        menu_box.append(&wrap);
    }

    // Separador sutil + importación nativa Linux (.ovpn / WireGuard .conf)
    menu_box.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let (ovpn_wrap, ovpn_btn) = make_popover_action_row("Importar OpenVPN (.ovpn)...", false, None);
    let (wg_wrap, wg_btn) = make_popover_action_row("Importar WireGuard (.conf)...", false, None);

    let pop_o = pop.clone();
    let win_o = parent_window.clone();
    let ban_o = banner.clone();
    let rel_o = trigger_reload.clone();
    ovpn_btn.connect_clicked(move |_| {
        pop_o.popdown();
        prompt_import_vpn_file(&win_o, "openvpn", "Archivos OpenVPN (*.ovpn, *.conf)", &["ovpn", "conf"], &ban_o, &rel_o);
    });

    let pop_w = pop.clone();
    let win_w = parent_window.clone();
    let ban_w = banner.clone();
    let rel_w = trigger_reload.clone();
    wg_btn.connect_clicked(move |_| {
        pop_w.popdown();
        prompt_import_vpn_file(&win_w, "wireguard", "Configuración WireGuard (*.conf)", &["conf"], &ban_w, &rel_w);
    });

    menu_box.append(&ovpn_wrap);
    menu_box.append(&wg_wrap);

    pop.set_child(Some(&menu_box));
    pop
}

fn build_vpn_help_popover() -> Popover {
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
            .label("Redes VPN en Malus DE")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    box_inner.append(
        &Label::builder()
            .label("• Usa el menú “+ ⌄” para crear configuraciones IKEv2 (Libreswan), L2TP, Cisco IPSec o importar archivos .ovpn / WireGuard.\n• L2TP sobre IPSec y Cisco IPSec requieren sus plugins correspondientes de NetworkManager en Linux (docs/decisiones/0009-vpn-pendiente.md).")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    pop.set_child(Some(&box_inner));
    pop
}

fn attach_scrim(dialog: &gtk4::Window, parent: &ApplicationWindow) {
    crate::set_modal_scrim_visible(parent, true);
    let p1 = parent.clone();
    dialog.connect_close_request(move |_| {
        crate::set_modal_scrim_visible(&p1, false);
        glib::Propagation::Proceed
    });
    let p2 = parent.clone();
    dialog.connect_destroy(move |_| {
        crate::set_modal_scrim_visible(&p2, false);
    });
}

fn show_vpn_privacy_dialog(parent: &ApplicationWindow) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title("Acerca de las redes VPN y la privacidad")
        .default_width(420)
        .build();
    attach_scrim(&dialog, parent);

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
    card.append(
        &Label::builder()
            .label("Acerca de las redes VPN y la privacidad")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    card.append(
        &Label::builder()
            .label("Una red privada virtual (VPN) crea un túnel cifrado entre este equipo y el servidor VPN configurado. Mientras una VPN está activa, el administrador de la red o proveedor del servicio VPN puede ver el tráfico que circula por dicho túnel.\n\nMalus DE administra las conexiones VPN directamente mediante NetworkManager y los secretos del usuario sin enviar telemetría a servicios externos.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    ok_wrap.set_halign(Align::End);
    let d_c = dialog.clone();
    ok_btn.connect_clicked(move |_| d_c.close());
    vbox.append(&ok_wrap);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_create_vpn_dialog(
    parent: &ApplicationWindow,
    proto: VpnProtocolKind,
    installed_plugins: &BTreeSet<String>,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let plugin_installed = proto
        .nm_vpn_type()
        .is_none_or(|t| installed_plugins.contains(t));

    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&format!("Añadir configuración {}", proto.title()))
        .default_width(420)
        .build();
    attach_scrim(&dialog, parent);

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

    if !plugin_installed {
        // TODO(malus-vpn): L2TP (NetworkManager-l2tp) y Cisco IPSec (NetworkManager-vpnc) requieren sus plugins NM en Fedora.
        card.append(
            &Label::builder()
                .label(&format!(
                    "Aviso: el plugin de NetworkManager para {} ({}) no está instalado actualmente en este sistema. Puedes seguir configurando si lo instalas luego, o usar IKEv2 / OpenVPN / WireGuard.",
                    proto.title(),
                    proto.required_fedora_package()
                ))
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }

    let name_entry = Entry::builder().placeholder_text("Nombre visible").build();
    let server_entry = Entry::builder()
        .placeholder_text("Dirección del servidor (ej. vpn.empresa.com)")
        .build();
    let remote_id_entry = Entry::builder()
        .placeholder_text("ID remoto / Dominio (opcional)")
        .build();
    let user_entry = Entry::builder()
        .placeholder_text("Nombre de la cuenta / Usuario")
        .build();
    let pass_entry = PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Contraseña")
        .build();
    let psk_entry = PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Secreto compartido (PSK)")
        .build();

    let auth_types = StringList::new(&["Nombre de usuario", "Certificado / Secreto compartido"]);
    let auth_dd = DropDown::builder().model(&auth_types).selected(0).build();

    card.append(&labeled_field("Nombre visible", &name_entry));
    card.append(&labeled_field("Dirección del servidor", &server_entry));
    if matches!(proto, VpnProtocolKind::Ikev2) {
        card.append(&labeled_field("ID remoto", &remote_id_entry));
        card.append(&labeled_field("Autenticación de usuario", &auth_dd));
    }
    card.append(&labeled_field("Cuenta / Usuario", &user_entry));
    card.append(&labeled_field("Contraseña", &pass_entry));
    if matches!(
        proto,
        VpnProtocolKind::L2tpIpsec | VpnProtocolKind::CiscoIpsec
    ) {
        card.append(&labeled_field("Secreto compartido", &psk_entry));
    }

    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (create_wrap, create_btn) = make_squircle_button("Crear", true, None);
    create_btn.set_sensitive(plugin_installed);
    footer.append(&cancel_wrap);
    footer.append(&create_wrap);

    let d_c = dialog.clone();
    cancel_btn.connect_clicked(move |_| d_c.close());

    let d_ok = dialog.clone();
    let ban_c = banner.clone();
    let rel_c = trigger_reload.clone();
    create_btn.connect_clicked(move |_| {
        let name = name_entry.text().trim().to_string();
        let server = server_entry.text().trim().to_string();
        let remote_id = remote_id_entry.text().trim().to_string();
        let user = user_entry.text().trim().to_string();
        let pass = pass_entry.text().to_string();
        let psk = psk_entry.text().to_string();
        if name.is_empty() || server.is_empty() {
            return;
        }
        d_ok.close();

        ban_c.set_label(&format!("Creando perfil VPN «{name}»..."));
        ban_c.set_visible(true);

        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let res = create_nm_vpn_profile(proto, &name, &server, &remote_id, &user, &pass, &psk);
            let _ = tx.send_blocking(res);
        });
        let b = ban_c.clone();
        let r = rel_c.clone();
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(())) => b.set_visible(false),
                Ok(Err(err)) => {
                    b.set_label(&format!("No se pudo crear la VPN: {err}"));
                    b.set_visible(true);
                }
                Err(_) => b.set_visible(false),
            }
            r();
        });
    });

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn labeled_field<W: IsA<gtk4::Widget>>(label: &str, widget: &W) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    let lbl = Label::builder()
        .label(label)
        .css_classes(["card-section-sub"])
        .width_request(145)
        .xalign(0.0)
        .build();
    widget.as_ref().set_hexpand(true);
    row.append(&lbl);
    row.append(widget);
    row
}

fn show_vpn_details_dialog(
    parent: &ApplicationWindow,
    profile: &VpnProfileEntry,
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(&profile.name)
        .default_width(400)
        .build();
    attach_scrim(&dialog, parent);

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
    card.append(&detail_kv_row("Configuración", &profile.name));
    card.append(&detail_kv_row("Tipo", &profile.protocol_label));
    if !profile.remote_server.is_empty() {
        card.append(&detail_kv_row("Servidor", &profile.remote_server));
    }
    card.append(&detail_kv_row(
        "Estado",
        if profile.connected {
            "Conectado"
        } else {
            "Sin conexión"
        },
    ));
    if !profile.device.is_empty() {
        card.append(&detail_kv_row("Interfaz", &profile.device));
    }

    let auto_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    auto_row.append(
        &Label::builder()
            .label("Conectar automáticamente (bajo demanda)")
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let auto_sw = Switch::builder()
        .active(profile.autoconnect)
        .css_classes(["macos-switch"])
        .build();
    let u_auto = profile.uuid.clone();
    let r_auto = trigger_reload.clone();
    auto_sw.connect_state_set(move |_, val| {
        let u = u_auto.clone();
        let r = r_auto.clone();
        let (tx, rx) = async_channel::bounded::<()>(1);
        std::thread::spawn(move || {
            let _ = run_nmcli(&[
                "connection",
                "modify",
                "uuid",
                &u,
                "connection.autoconnect",
                if val { "yes" } else { "no" },
            ]);
            let _ = tx.send_blocking(());
        });
        glib::spawn_future_local(async move {
            let _ = rx.recv().await;
            r();
        });
        glib::Propagation::Proceed
    });
    auto_row.append(&auto_sw);
    card.append(&auto_row);
    vbox.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let (del_wrap, del_btn) = make_squircle_button("Eliminar configuración...", false, None);
    let d_del = dialog.clone();
    let u_del = profile.uuid.clone();
    let b_del = banner.clone();
    let r_del = trigger_reload.clone();
    del_btn.connect_clicked(move |_| {
        d_del.close();
        let u = u_del.clone();
        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let res = run_nmcli(&["connection", "delete", "uuid", &u]).map(|_| ());
            let _ = tx.send_blocking(res);
        });
        let b = b_del.clone();
        let r = r_del.clone();
        glib::spawn_future_local(async move {
            if let Ok(Err(err)) = rx.recv().await {
                b.set_label(&format!("No se pudo eliminar la VPN: {err}"));
                b.set_visible(true);
            } else {
                b.set_visible(false);
            }
            r();
        });
    });

    footer.append(&del_wrap);
    footer.append(&GtkBox::builder().hexpand(true).build());
    let (ok_wrap, ok_btn) = make_squircle_button("Aceptar", true, None);
    let d_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| d_ok.close());
    footer.append(&ok_wrap);

    vbox.append(&footer);
    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn prompt_import_vpn_file(
    parent: &ApplicationWindow,
    nm_import_type: &'static str,
    filter_name: &str,
    patterns: &[&str],
    banner: &Label,
    trigger_reload: &Rc<dyn Fn()>,
) {
    let filter = FileFilter::new();
    filter.set_name(Some(filter_name));
    for pat in patterns {
        filter.add_suffix(pat);
    }
    let filters = gtk4::gio::ListStore::new::<FileFilter>();
    filters.append(&filter);

    let dialog = FileDialog::builder()
        .title("Importar configuración de VPN")
        .accept_label("Importar")
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();

    let b = banner.clone();
    let r = trigger_reload.clone();
    dialog.open(Some(parent), None::<&gtk4::gio::Cancellable>, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                let path_str = path.to_string_lossy().to_string();
                let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
                std::thread::spawn(move || {
                    let res =
                        run_nmcli(&["connection", "import", "type", nm_import_type, "file", &path_str])
                            .map(|_| ());
                    let _ = tx.send_blocking(res);
                });
                let b_d = b.clone();
                let r_d = r.clone();
                glib::spawn_future_local(async move {
                    match rx.recv().await {
                        Ok(Ok(())) => b_d.set_visible(false),
                        Ok(Err(err)) => {
                            b_d.set_label(&format!("No se pudo importar VPN: {err}"));
                            b_d.set_visible(true);
                        }
                        Err(_) => {}
                    }
                    r_d();
                });
            }
        }
    });
}

// ============================================================================
// Backend NetworkManager (nmcli) y utilidades testables
// ============================================================================

fn run_nmcli(args: &[&str]) -> Result<String, String> {
    let out = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| format!("nmcli: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("nmcli salió con código {}", out.status)
        } else {
            err
        })
    }
}

fn create_nm_vpn_profile(
    proto: VpnProtocolKind,
    name: &str,
    server: &str,
    remote_id: &str,
    user: &str,
    password: &str,
    psk: &str,
) -> Result<(), String> {
    let Some(vpn_type) = proto.nm_vpn_type() else {
        return Err("Usa Importar WireGuard (.conf) para perfiles WireGuard.".to_string());
    };
    let data = match proto {
        VpnProtocolKind::Ikev2 => {
            let mut pairs = vec![format!("right={server}"), "ikev2=insist".to_string()];
            if !remote_id.is_empty() {
                pairs.push(format!("rightid={remote_id}"));
            }
            if !user.is_empty() {
                pairs.push(format!("leftxauthusername={user}"));
            }
            pairs.join(", ")
        }
        VpnProtocolKind::L2tpIpsec => {
            let mut pairs = vec![format!("gateway={server}"), "ipsec-enabled=yes".to_string()];
            if !user.is_empty() {
                pairs.push(format!("user={user}"));
            }
            if !psk.is_empty() {
                pairs.push(format!("ipsec-psk={psk}"));
            }
            pairs.join(", ")
        }
        VpnProtocolKind::CiscoIpsec => {
            let mut pairs = vec![format!("IPSec gateway={server}")];
            if !user.is_empty() {
                pairs.push(format!("Xauth username={user}"));
            }
            if !remote_id.is_empty() {
                pairs.push(format!("IPSec ID={remote_id}"));
            }
            pairs.join(", ")
        }
    };

    run_nmcli(&[
        "connection",
        "add",
        "type",
        "vpn",
        "vpn-type",
        vpn_type,
        "con-name",
        name,
        "ifname",
        "*",
        "--",
        "vpn.data",
        &data,
    ])?;

    if !password.is_empty() {
        let secret_key = match proto {
            VpnProtocolKind::CiscoIpsec => "Xauth password",
            VpnProtocolKind::Ikev2 => "xauthpassword",
            _ => "password",
        };
        let _ = run_nmcli(&[
            "connection",
            "modify",
            "id",
            name,
            "vpn.secrets",
            &format!("{secret_key}={password}"),
        ]);
    }
    Ok(())
}

fn collect_vpn_snapshot() -> VpnSnapshot {
    let mut snap = VpnSnapshot {
        installed_plugins: detect_installed_vpn_plugins(),
        ..Default::default()
    };

    let Ok(conns_raw) = run_nmcli(&[
        "-t",
        "-f",
        "UUID,NAME,TYPE,DEVICE,AUTOCONNECT",
        "connection",
        "show",
    ]) else {
        return snap;
    };
    snap.nm_available = true;
    let active_raw =
        run_nmcli(&["-t", "-f", "UUID,TYPE,STATE,DEVICE", "connection", "show", "--active"])
            .unwrap_or_default();

    let mut profiles = parse_nmcli_vpn_connections(&conns_raw, &active_raw);
    for p in &mut profiles {
        if p.conn_type == "vpn" {
            if let Ok(det) = run_nmcli(&["-t", "-f", "vpn.service-type,vpn.data", "connection", "show", "uuid", &p.uuid]) {
                enrich_vpn_profile_metadata(p, &det);
            }
        }
    }
    snap.profiles = profiles;
    snap
}

pub(crate) fn detect_installed_vpn_plugins() -> BTreeSet<String> {
    let mut plugins = BTreeSet::new();
    for dir in [
        "/usr/lib/NetworkManager/VPN",
        "/usr/lib64/NetworkManager/VPN",
        "/etc/NetworkManager/VPN",
    ] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(name) = parse_nm_vpn_plugin_filename(&entry.path()) {
                    plugins.insert(name);
                }
            }
        }
    }
    plugins
}

pub(crate) fn parse_nm_vpn_plugin_filename(path: &Path) -> Option<String> {
    let fname = path.file_name()?.to_str()?;
    let trimmed = fname
        .strip_prefix("nm-")?
        .strip_suffix("-service.name")?;
    Some(trimmed.to_lowercase())
}

pub(crate) fn parse_nmcli_vpn_connections(
    conns_raw: &str,
    active_raw: &str,
) -> Vec<VpnProfileEntry> {
    let mut active_states: HashMap<String, (String, String)> = HashMap::new();
    for line in active_raw.lines() {
        let parts = split_terse(line);
        if parts.len() >= 4 {
            active_states.insert(parts[0].clone(), (parts[2].clone(), parts[3].clone()));
        }
    }

    let mut list = Vec::new();
    for line in conns_raw.lines() {
        let parts = split_terse(line);
        if parts.len() < 5 {
            continue;
        }
        let uuid = &parts[0];
        let name = &parts[1];
        let ctype = &parts[2];
        let device = &parts[3];
        let autoconnect = parts[4].eq_ignore_ascii_case("yes");

        if ctype != "vpn" && ctype != "wireguard" {
            continue;
        }

        let (state_str, active_dev) = active_states
            .get(uuid)
            .cloned()
            .unwrap_or_else(|| (String::new(), device.clone()));
        let connected = state_str == "activated";
        let connecting = state_str == "activating";
        let protocol_label = if ctype == "wireguard" {
            "WireGuard".to_string()
        } else {
            "VPN".to_string()
        };

        list.push(VpnProfileEntry {
            uuid: uuid.clone(),
            name: name.clone(),
            conn_type: ctype.clone(),
            protocol_label,
            device: if active_dev == "--" {
                String::new()
            } else {
                active_dev
            },
            connected,
            connecting,
            autoconnect,
            remote_server: String::new(),
        });
    }

    list.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list
}

fn enrich_vpn_profile_metadata(profile: &mut VpnProfileEntry, detail_raw: &str) {
    for line in detail_raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k == "vpn.service-type" {
                profile.protocol_label = friendly_vpn_service_label(v.trim());
            } else if k == "vpn.data" {
                for kv in v.split(',') {
                    if let Some((dk, dv)) = kv.trim().split_once('=') {
                        let key_l = dk.trim().to_lowercase();
                        if matches!(
                            key_l.as_str(),
                            "right" | "gateway" | "remote" | "ipsec gateway"
                        ) {
                            profile.remote_server = dv.trim().to_string();
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn friendly_vpn_service_label(service_type: &str) -> String {
    let lower = service_type.to_lowercase();
    if lower.contains("libreswan") || lower.contains("strongswan") {
        "IKEv2".to_string()
    } else if lower.contains("l2tp") {
        "L2TP sobre IPSec".to_string()
    } else if lower.contains("vpnc") {
        "Cisco IPSec".to_string()
    } else if lower.contains("openvpn") {
        "OpenVPN".to_string()
    } else if lower.contains("openconnect") {
        "OpenConnect".to_string()
    } else if lower.contains("ssh") {
        "SSH VPN".to_string()
    } else {
        "VPN".to_string()
    }
}

fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == ':' || next == '\\' {
                    cur.push(next);
                    chars.next();
                    continue;
                }
            }
            cur.push(c);
        } else if c == ':' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nmcli_vpn_and_wireguard_connections() {
        let conns = "\
1111:Casa WG:wireguard:wg0:yes
2222:Trabajo IKEv2:vpn:--:no
3333:Mega_5G:802-11-wireless:wlp4s0:yes
";
        let active = "1111:wireguard:activated:wg0\n";
        let parsed = parse_nmcli_vpn_connections(conns, active);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Casa WG");
        assert!(parsed[0].connected);
        assert_eq!(parsed[0].protocol_label, "WireGuard");
        assert_eq!(parsed[1].name, "Trabajo IKEv2");
        assert!(!parsed[1].connected);
        assert_eq!(parsed[1].device, "");
    }

    #[test]
    fn parses_nm_vpn_plugin_descriptors_and_labels() {
        assert_eq!(
            parse_nm_vpn_plugin_filename(Path::new("/usr/lib/NetworkManager/VPN/nm-libreswan-service.name"))
                .as_deref(),
            Some("libreswan")
        );
        assert_eq!(
            friendly_vpn_service_label("org.freedesktop.NetworkManager.libreswan"),
            "IKEv2"
        );
        assert_eq!(
            friendly_vpn_service_label("org.freedesktop.NetworkManager.l2tp"),
            "L2TP sobre IPSec"
        );
        assert_eq!(
            friendly_vpn_service_label("org.freedesktop.NetworkManager.vpnc"),
            "Cisco IPSec"
        );
    }
}
