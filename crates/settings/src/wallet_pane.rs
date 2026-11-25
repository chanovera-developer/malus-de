use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, Image, Label, Orientation,
    PolicyType, Popover, ScrolledWindow, StringList, Window,
};
use malus_de_common::config::{WalletCardEntry, WalletConfig};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const WALLET_PANEL_ID: &str = "wallet";
pub const WALLET_PANEL_TITLE: &str = "Wallet";

#[derive(Clone, Copy)]
enum DetailFieldKind {
    Shipping,
    Email,
    Phone,
}

impl DetailFieldKind {
    fn modal_title(self) -> &'static str {
        match self {
            Self::Shipping => "Agregar domicilio de envío",
            Self::Email => "Agregar correo electrónico",
            Self::Phone => "Agregar teléfono",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Shipping => "Ej. Av. Reforma 120, Ciudad de México",
            Self::Email => "Ej. nombre@dominio.com",
            Self::Phone => "Ej. 55 1234 5678",
        }
    }

    fn add_option_label(self) -> &'static str {
        match self {
            Self::Shipping => "Agregar domicilio...",
            Self::Email => "Agregar correo...",
            Self::Phone => "Agregar teléfono...",
        }
    }
}

fn build_detail_options(values: &[String], kind: DetailFieldKind) -> Vec<String> {
    let mut out = Vec::new();
    if values.is_empty() {
        out.push("Sin configurar".to_string());
    } else {
        out.extend(values.iter().cloned());
    }
    out.push(kind.add_option_label().to_string());
    out
}

fn show_add_detail_value_modal(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<WalletConfig>>,
    kind: DetailFieldKind,
    on_saved: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(410)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    root.append(
        &Label::builder()
            .label(kind.modal_title())
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );

    let entry = Entry::builder()
        .placeholder_text(kind.placeholder())
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    root.append(&entry);

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let (save_wrap, save_btn) = make_squircle_button("Guardar", true, None);
    actions.append(&cancel_wrap);
    actions.append(&save_wrap);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dlg_cancel = dialog.clone();
    let cb_cancel = on_saved.clone();
    cancel_btn.connect_clicked(move |_| {
        cb_cancel();
        crate::close_attached_modal(&dlg_cancel);
    });

    let save_action = {
        let cfg_c = cfg.clone();
        let ent_c = entry.clone();
        let dlg_c = dialog.clone();
        let cb = on_saved;
        Rc::new(move || {
            let val = ent_c.text().trim().to_string();
            if !val.is_empty() {
                let mut c = cfg_c.borrow_mut();
                match kind {
                    DetailFieldKind::Shipping => {
                        c.shipping_addresses.push(val);
                        c.default_shipping_idx = c.shipping_addresses.len().saturating_sub(1);
                    }
                    DetailFieldKind::Email => {
                        c.emails.push(val);
                        c.default_email_idx = c.emails.len().saturating_sub(1);
                    }
                    DetailFieldKind::Phone => {
                        c.phones.push(val);
                        c.default_phone_idx = c.phones.len().saturating_sub(1);
                    }
                }
                let _ = c.save();
            }
            cb();
            crate::close_attached_modal(&dlg_c);
        })
    };

    let sa_btn = save_action.clone();
    save_btn.connect_clicked(move |_| sa_btn());
    entry.connect_activate(move |_| save_action());

    crate::present_attached_modal(&dialog, parent);
    entry.grab_focus();
}

fn show_autofill_cards_modal(
    parent: &ApplicationWindow,
    cfg: &Rc<RefCell<WalletConfig>>,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(460)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(22)
        .margin_bottom(20)
        .margin_start(22)
        .margin_end(22)
        .build();

    let hdr_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    hdr_row.append(
        &Label::builder()
            .label("Tarjetas en Autorrelleno")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr_row.append(&make_pending_badge());
    root.append(&hdr_row);

    root.append(
        &Label::builder()
            .label(
                "Por seguridad, Malus DE guarda únicamente el alias, marca, titular, vencimiento \
                 y últimos 4 dígitos. El autorrelleno automático en navegador se integrará con el \
                 navegador nativo.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let list_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    root.append(&wrap_squircle_card(&list_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Formulario compacto para añadir tarjeta
    let form_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();
    form_card.append(
        &Label::builder()
            .label("Agregar tarjeta de autorrelleno")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let label_ent = Entry::builder()
        .placeholder_text("Alias (ej. Débito personal)")
        .css_classes(["sidebar-search"])
        .build();
    let num_ent = Entry::builder()
        .placeholder_text("Número o últimos 4 dígitos (solo se guardan los últimos 4)")
        .css_classes(["sidebar-search"])
        .build();
    let row_meta = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let holder_ent = Entry::builder()
        .placeholder_text("Titular")
        .css_classes(["sidebar-search"])
        .hexpand(true)
        .build();
    let exp_ent = Entry::builder()
        .placeholder_text("MM/AA")
        .css_classes(["sidebar-search"])
        .width_request(92)
        .build();
    row_meta.append(&holder_ent);
    row_meta.append(&exp_ent);

    let fb_lbl = Label::builder()
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .visible(false)
        .build();

    let (add_card_wrap, add_card_btn) = make_squircle_button("Agregar tarjeta", false, None);
    let add_btn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    add_btn_row.append(&add_card_wrap);

    form_card.append(&label_ent);
    form_card.append(&num_ent);
    form_card.append(&row_meta);
    form_card.append(&fb_lbl);
    form_card.append(&add_btn_row);
    root.append(&wrap_squircle_card(&form_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    footer.append(&ok_wrap);
    root.append(&footer);

    dialog.set_child(Some(&root));

    let rebuild_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuild_list: Rc<dyn Fn()> = {
        let cfg_c = cfg.clone();
        let list_c = list_card.clone();
        let on_ch = on_changed.clone();
        let slot_c = rebuild_slot.clone();
        Rc::new(move || {
            while let Some(child) = list_c.first_child() {
                list_c.remove(&child);
            }
            let cards = cfg_c.borrow().autofill_cards.clone();
            if cards.is_empty() {
                let empty = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .css_classes(["wifi-pref-row"])
                    .build();
                empty.append(
                    &Label::builder()
                        .label("No hay tarjetas guardadas en Autorrelleno.")
                        .css_classes(["card-section-sub"])
                        .xalign(0.0)
                        .build(),
                );
                list_c.append(&empty);
                return;
            }
            for (idx, card) in cards.iter().enumerate() {
                if idx > 0 {
                    list_c.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
                }
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .css_classes(["wifi-pref-row"])
                    .build();
                row.append(
                    &Image::builder()
                        .icon_name("auth-smartcard-symbolic")
                        .pixel_size(20)
                        .valign(Align::Center)
                        .build(),
                );
                let col = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(1)
                    .hexpand(true)
                    .build();
                col.append(
                    &Label::builder()
                        .label(&format!("{} ({} •••• {})", card.label, card.brand, card.last4))
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .build(),
                );
                let mut sub_parts = Vec::new();
                if !card.holder.is_empty() {
                    sub_parts.push(card.holder.clone());
                }
                if !card.expiry.is_empty() {
                    sub_parts.push(format!("Vence {}", card.expiry));
                }
                if !sub_parts.is_empty() {
                    col.append(
                        &Label::builder()
                            .label(&sub_parts.join(" · "))
                            .css_classes(["card-section-sub"])
                            .xalign(0.0)
                            .build(),
                    );
                }
                let (del_wrap, del_btn) = make_squircle_button("Eliminar", false, None);
                let cfg_d = cfg_c.clone();
                let id_d = card.id.clone();
                let ch_d = on_ch.clone();
                let sl_d = slot_c.clone();
                del_btn.connect_clicked(move |_| {
                    {
                        let mut c = cfg_d.borrow_mut();
                        c.remove_autofill_card(&id_d);
                        let _ = c.save();
                    }
                    ch_d();
                    if let Some(rf) = sl_d.borrow().as_ref() {
                        rf();
                    }
                });
                row.append(&col);
                row.append(&del_wrap);
                list_c.append(&row);
            }
        })
    };
    *rebuild_slot.borrow_mut() = Some(rebuild_list.clone());
    rebuild_list();

    {
        let cfg_c = cfg.clone();
        let rb_c = rebuild_list;
        let on_ch = on_changed;
        add_card_btn.connect_clicked(move |_| {
            let digits: String = num_ent
                .text()
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect();
            if digits.len() < 4 {
                fb_lbl.set_label("Introduce al menos los últimos 4 dígitos de la tarjeta.");
                fb_lbl.set_visible(true);
                return;
            }
            fb_lbl.set_visible(false);
            let brand = WalletConfig::detect_card_brand(&digits).to_string();
            let last4 = digits[digits.len() - 4..].to_string();
            let raw_label = label_ent.text().trim().to_string();
            let label = if raw_label.is_empty() {
                brand.clone()
            } else {
                raw_label
            };
            let entry = WalletCardEntry {
                label,
                brand,
                last4,
                holder: holder_ent.text().trim().to_string(),
                expiry: exp_ent.text().trim().to_string(),
                ..WalletCardEntry::default()
            };
            {
                let mut c = cfg_c.borrow_mut();
                c.add_autofill_card(entry);
                let _ = c.save();
            }
            label_ent.set_text("");
            num_ent.set_text("");
            holder_ent.set_text("");
            exp_ent.set_text("");
            on_ch();
            rb_c();
        });
    }

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

fn show_wallet_privacy_modal(parent: &ApplicationWindow) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .resizable(false)
        .default_width(430)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build();

    root.append(
        &Image::builder()
            .icon_name("auth-smartcard-symbolic")
            .pixel_size(38)
            .halign(Align::Center)
            .build(),
    );
    root.append(
        &Label::builder()
            .label("Wallet y la privacidad")
            .css_classes(["card-section-title"])
            .halign(Align::Center)
            .build(),
    );
    root.append(
        &Label::builder()
            .label(
                "Tus domicilios de envío, correos, teléfonos y metadatos de tarjetas en Autorrelleno \
                 se almacenan localmente en ~/.config/malus-de/wallet.toml y nunca se envían a \
                 servidores externos. Por seguridad nunca se almacenan números completos de tarjeta \
                 ni códigos CVV en texto plano.",
            )
            .css_classes(["card-section-sub"])
            .justify(gtk4::Justification::Center)
            .wrap(true)
            .build(),
    );

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .margin_top(4)
        .build();
    let (ok_wrap, ok_btn) = make_squircle_button("OK", true, None);
    actions.append(&ok_wrap);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dlg_ok = dialog.clone();
    ok_btn.connect_clicked(move |_| crate::close_attached_modal(&dlg_ok));
    crate::present_attached_modal(&dialog, parent);
}

/// Construye el panel «Wallet» estilo macOS Tahoe.
pub fn create_wallet_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
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

    let cfg = Rc::new(RefCell::new(WalletConfig::load()));
    let syncing = Rc::new(Cell::new(false));

    // ========================================================
    // Sección 1: Tarjetas de pago
    // ========================================================
    pane.append(
        &Label::builder()
            .label("Tarjetas de pago")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .build(),
    );

    // Tarjeta 1: Tarjeta de crédito o débito (NFC / pagos bancarios -> por implementar)
    let pay_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let pay_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let pay_left = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    pay_left.append(
        &Label::builder()
            .label("Tarjeta de crédito o débito")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    pay_left.append(&make_pending_badge());
    let (add_pay_wrap, add_pay_btn) = make_squircle_button("Agregar tarjeta...", false, None);
    add_pay_btn.set_sensitive(false);
    pay_row.append(&pay_left);
    pay_row.append(&add_pay_wrap);
    pay_card.append(&pay_row);
    pane.append(&wrap_squircle_card(&pay_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta 2: Tarjetas en Autorrelleno (con contador vivo + modal CRUD)
    let autofill_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let autofill_btn = Button::builder()
        .css_classes(["wifi-nav-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    let autofill_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .valign(Align::Center)
        .build();
    autofill_row.append(
        &Label::builder()
            .label("Tarjetas en Autorrelleno")
            .css_classes(["wifi-row-title"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let autofill_count_lbl = Label::builder()
        .label("0")
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    autofill_row.append(&autofill_count_lbl);
    autofill_row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(13)
            .css_classes(["wifi-chevron"])
            .valign(Align::Center)
            .build(),
    );
    autofill_btn.set_child(Some(&autofill_row));
    autofill_card.append(&autofill_btn);
    pane.append(&wrap_squircle_card(
        &autofill_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    pane.append(
        &Label::builder()
            .label(
                "Los datos guardados de tus tarjetas de débito y crédito utilizados para \
                 autorrellenar formularios y sitios web.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .margin_start(6)
            .margin_end(6)
            .build(),
    );

    // ========================================================
    // Sección 2: Detalles de pago
    // ========================================================
    pane.append(
        &Label::builder()
            .label("Detalles de pago")
            .css_classes(["wifi-section-heading"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    // Tarjeta 3: Valores predeterminados de transacción (100% funcional)
    let tx_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();

    let tx_hdr = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["wifi-pref-row"])
        .build();
    tx_hdr.append(
        &Label::builder()
            .label("Valores predeterminados de transacción")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    tx_hdr.append(
        &Label::builder()
            .label(
                "Elige la información del modo de pago predeterminado que quieres usar cuando \
                 hagas compras. Las direcciones y opciones pueden cambiarse al momento de la \
                 transacción.",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    tx_card.append(&tx_hdr);

    let make_detail_dd_row = |title: &str| -> (GtkBox, DropDown) {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        row.append(
            &Label::builder()
                .label(title)
                .css_classes(["wifi-row-title"])
                .hexpand(true)
                .xalign(0.0)
                .build(),
        );
        let dd = DropDown::builder()
            .model(&StringList::new(&["Sin configurar"]))
            .selected(0)
            .valign(Align::Center)
            .css_classes(["wifi-compact-dropdown"])
            .build();
        row.append(&dd);
        (row, dd)
    };

    let (ship_row, ship_dd) = make_detail_dd_row("Domicilio de envío");
    let (email_row, email_dd) = make_detail_dd_row("Correo electrónico");
    let (phone_row, phone_dd) = make_detail_dd_row("Teléfono");

    tx_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    tx_card.append(&ship_row);
    tx_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    tx_card.append(&email_row);
    tx_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    tx_card.append(&phone_row);
    pane.append(&wrap_squircle_card(&tx_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta 4: Agregar pedidos a Wallet (por implementar, persistido)
    let orders_card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    let orders_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let orders_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    let orders_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    orders_title_row.append(
        &Label::builder()
            .label("Agregar pedidos a Wallet")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    orders_title_row.append(&make_pending_badge());
    orders_col.append(&orders_title_row);
    orders_col.append(
        &Label::builder()
            .label("Los pedidos de los comercios participantes se agregarán automáticamente a Wallet.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let orders_sw = Switch::builder()
        .valign(Align::Center)
        .active(cfg.borrow().add_orders_to_wallet)
        .build();
    orders_row.append(&orders_col);
    orders_row.append(&*orders_sw);
    orders_card.append(&orders_row);
    pane.append(&wrap_squircle_card(
        &orders_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ========================================================
    // Pie inferior: Consulta cómo se utiliza tu información... y ?
    // ========================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (privacy_wrap, privacy_btn) =
        make_squircle_button("Consulta cómo se utiliza tu información...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Wallet");

    let help_popover = Popover::builder().has_arrow(true).autohide(true).build();
    let help_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .width_request(300)
        .build();
    help_box.append(
        &Label::builder()
            .label("Wallet")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    help_box.append(
        &Label::builder()
            .label(
                "Puedes administrar tarjetas en Autorrelleno y valores predeterminados de \
                 envío, correo y teléfono localmente en wallet.toml. Los pagos con tarjeta \
                 tokenizada y el seguimiento automático de pedidos están marcados como \
                 «por implementar».",
            )
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    help_popover.set_child(Some(&help_box));
    help_btn.set_popover(Some(&help_popover));

    footer.append(&privacy_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    // ========================================================
    // Refresco desde wallet.toml
    // ========================================================
    let refresh: Rc<dyn Fn()> = {
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        let count_c = autofill_count_lbl;
        let ship_c = ship_dd.clone();
        let email_c = email_dd.clone();
        let phone_c = phone_dd.clone();
        let orders_c = orders_sw.clone();
        Rc::new(move || {
            let loaded = WalletConfig::load();
            *cfg_c.borrow_mut() = loaded.clone();

            count_c.set_label(&loaded.autofill_cards.len().to_string());

            sync_c.set(true);

            let ship_opts = build_detail_options(&loaded.shipping_addresses, DetailFieldKind::Shipping);
            let ship_refs: Vec<&str> = ship_opts.iter().map(String::as_str).collect();
            ship_c.set_model(Some(&StringList::new(&ship_refs)));
            let ship_sel = if loaded.shipping_addresses.is_empty() {
                0
            } else {
                loaded
                    .default_shipping_idx
                    .min(loaded.shipping_addresses.len() - 1) as u32
            };
            ship_c.set_selected(ship_sel);

            let email_opts = build_detail_options(&loaded.emails, DetailFieldKind::Email);
            let email_refs: Vec<&str> = email_opts.iter().map(String::as_str).collect();
            email_c.set_model(Some(&StringList::new(&email_refs)));
            let email_sel = if loaded.emails.is_empty() {
                0
            } else {
                loaded.default_email_idx.min(loaded.emails.len() - 1) as u32
            };
            email_c.set_selected(email_sel);

            let phone_opts = build_detail_options(&loaded.phones, DetailFieldKind::Phone);
            let phone_refs: Vec<&str> = phone_opts.iter().map(String::as_str).collect();
            phone_c.set_model(Some(&StringList::new(&phone_refs)));
            let phone_sel = if loaded.phones.is_empty() {
                0
            } else {
                loaded.default_phone_idx.min(loaded.phones.len() - 1) as u32
            };
            phone_c.set_selected(phone_sel);

            orders_c.set_active(loaded.add_orders_to_wallet);
            sync_c.set(false);
        })
    };

    // ========================================================
    // Señales interactivas
    // ========================================================
    {
        let win_c = window.clone();
        let cfg_c = cfg.clone();
        let rf_c = refresh.clone();
        autofill_btn.connect_clicked(move |_| {
            show_autofill_cards_modal(&win_c, &cfg_c, rf_c.clone());
        });
    }

    let wire_detail_dd = |dd: &DropDown, kind: DetailFieldKind| {
        let win_c = window.clone();
        let cfg_c = cfg.clone();
        let sync_c = syncing.clone();
        let rf_c = refresh.clone();
        dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let sel = dd.selected() as usize;
            let count = {
                let c = cfg_c.borrow();
                match kind {
                    DetailFieldKind::Shipping => c.shipping_addresses.len(),
                    DetailFieldKind::Email => c.emails.len(),
                    DetailFieldKind::Phone => c.phones.len(),
                }
            };
            let add_idx = if count == 0 { 1 } else { count };
            if sel == add_idx {
                show_add_detail_value_modal(&win_c, &cfg_c, kind, rf_c.clone());
            } else if count > 0 && sel < count {
                let mut c = cfg_c.borrow_mut();
                match kind {
                    DetailFieldKind::Shipping => c.default_shipping_idx = sel,
                    DetailFieldKind::Email => c.default_email_idx = sel,
                    DetailFieldKind::Phone => c.default_phone_idx = sel,
                }
                let _ = c.save();
            }
        });
    };
    wire_detail_dd(&ship_dd, DetailFieldKind::Shipping);
    wire_detail_dd(&email_dd, DetailFieldKind::Email);
    wire_detail_dd(&phone_dd, DetailFieldKind::Phone);

    {
        let cfg_c = cfg;
        let sync_c = syncing;
        orders_sw.connect_state_set(move |_, active| {
            if !sync_c.get() {
                let mut c = cfg_c.borrow_mut();
                c.add_orders_to_wallet = active;
                let _ = c.save();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let win_c = window.clone();
        privacy_btn.connect_clicked(move |_| {
            show_wallet_privacy_modal(&win_c);
        });
    }

    refresh();
    scrolled.set_child(Some(&pane));
    (scrolled, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_dropdown_options_include_add_action() {
        let empty = build_detail_options(&[], DetailFieldKind::Shipping);
        assert_eq!(empty, vec!["Sin configurar", "Agregar domicilio..."]);

        let populated = build_detail_options(&["a@b.com".to_string()], DetailFieldKind::Email);
        assert_eq!(populated, vec!["a@b.com", "Agregar correo..."]);
    }
}
