use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Label, MenuButton, Orientation, PolicyType, Popover,
    ScrolledWindow,
};
use malus_de_common::config::CanopyConfig;
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::RefCell;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_button,
    make_squircle_colored_badge, wrap_squircle_card, wrap_squircle_round_control,
    RoundControlStyle, WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const CANOPY_PANEL_ID: &str = "spotlight";
pub const CANOPY_PANEL_TITLE: &str = "Canopy";

const RETENTION_OPTIONS: [(&str, u64); 3] = [
    ("30 minutos", 30),
    ("8 horas", 480),
    ("7 días", 10_080),
];

fn retention_index_for_minutes(minutes: u64) -> usize {
    RETENTION_OPTIONS
        .iter()
        .position(|(_, m)| *m == minutes)
        .unwrap_or(1)
}

/// Crea el panel «Canopy» (buscador rápido de Malus DE) siguiendo el diseño de macOS Tahoe.
/// Los controles con respaldo real (`Apps`, `Archivos`, `Carpetas`, `Resultados del portapapeles`,
/// retención y borrado del historial) persisten en `~/.config/malus-de/canopy.toml`; las funciones
/// no implementadas todavía muestran la etiqueta «por implementar».
pub fn create_canopy_pane() -> (ScrolledWindow, Rc<dyn Fn()>) {
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
    scrolled.set_child(Some(&pane));

    let cfg = Rc::new(RefCell::new(CanopyConfig::load()));

    // 1. Tarjeta superior (ícono lupa azul + Canopy + descripción)
    let banner_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    banner_card.append(&make_squircle_colored_badge(
        34,
        "system-search-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        (0.20, 0.56, 0.99),
        (0.04, 0.42, 0.92),
    ));
    let banner_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    banner_col.append(
        &Label::builder()
            .label(CANOPY_PANEL_TITLE)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    banner_col.append(
        &Label::builder()
            .label("Canopy te ayuda a realizar búsquedas rápidamente en la computadora y muestra aplicaciones, archivos, carpetas, cálculos y acciones del sistema.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    banner_card.append(&banner_col);
    pane.append(&wrap_squircle_card(&banner_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Tarjeta "Mostrar contenido relacionado" (por implementar)
    let related_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    related_card.append(&make_pending_switch_card_content(
        "Mostrar contenido relacionado",
        "Permite que se muestre contenido relacionado al realizar una búsqueda o al consultar texto, objetos y fotos.",
    ));
    pane.append(&wrap_squircle_card(&related_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 3. Tarjeta "Restablecer teclas rápidas" + "Historial de búsqueda de Canopy" (por implementar)
    let shortcuts_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    shortcuts_card.append(&make_pending_button_row(
        "Restablecer teclas rápidas",
        Some("Restablece las teclas rápidas de Canopy a su configuración predeterminada."),
        "Restablecer teclas rápidas...",
    ));
    shortcuts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    shortcuts_card.append(&make_pending_button_row(
        "Historial de búsqueda de Canopy",
        None,
        "Eliminar historial de búsqueda...",
    ));
    pane.append(&wrap_squircle_card(&shortcuts_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 4. Tarjeta "Ayudar a mejorar Buscar" (por implementar)
    let improve_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    improve_card.append(&make_pending_switch_card_content(
        "Ayudar a mejorar Buscar",
        "Ayuda a mejorar Buscar al permitir recopilar estadísticas anónimas de uso de Canopy y Pip.",
    ));
    pane.append(&wrap_squircle_card(&improve_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 5. Tarjeta "Resultados del sistema"
    let sys_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let sys_hdr = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["wifi-pref-row"])
        .build();
    sys_hdr.append(
        &Label::builder()
            .label("Resultados del sistema")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    sys_hdr.append(
        &Label::builder()
            .label("Permite que los elementos y su contenido se muestren en Canopy.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    sys_card.append(&sys_hdr);
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 5.1 Apps (real)
    let (apps_row, apps_sw) = make_working_switch_row("Apps", cfg.borrow().show_apps, {
        let c = cfg.clone();
        move |active| {
            c.borrow_mut().show_apps = active;
            let _ = c.borrow().save();
        }
    });
    sys_card.append(&apps_row);
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 5.2 Apps para teléfono (por implementar)
    sys_card.append(&make_pending_switch_row("Apps para teléfono"));
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 5.3 Archivos (real + botón circular de información)
    let files_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    files_row.append(
        &Label::builder()
            .label("Archivos")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    files_row.append(&make_files_info_button());
    let files_sw = Switch::builder()
        .active(cfg.borrow().show_files)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    {
        let c = cfg.clone();
        files_sw.connect_state_set(move |_, active| {
            c.borrow_mut().show_files = active;
            let _ = c.borrow().save();
            glib::Propagation::Proceed
        });
    }
    files_row.append(&*files_sw);
    sys_card.append(&files_row);
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 5.4 Carpetas (real)
    let (folders_row, folders_sw) =
        make_working_switch_row("Carpetas", cfg.borrow().show_folders, {
            let c = cfg.clone();
            move |active| {
                c.borrow_mut().show_folders = active;
                let _ = c.borrow().save();
            }
        });
    sys_card.append(&folders_row);
    sys_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 5.5 Elementos de menú (por implementar)
    sys_card.append(&make_pending_switch_row("Elementos de menú"));
    pane.append(&wrap_squircle_card(&sys_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 6. Tarjeta "Resultados del portapapeles" (real)
    let clip_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let clip_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let clip_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    clip_text.append(
        &Label::builder()
            .label("Resultados del portapapeles")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    clip_text.append(
        &Label::builder()
            .label("Permite que Canopy busque y muestre los elementos copiados en el portapapeles. Es posible que se muestren resultados de búsquedas con información personal o confidencial.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    clip_hdr.append(&clip_text);
    let clip_sw = Switch::builder()
        .active(cfg.borrow().show_clipboard)
        .css_classes(["macos-switch"])
        .valign(Align::Start)
        .build();
    clip_hdr.append(&*clip_sw);
    clip_card.append(&clip_hdr);
    clip_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // 6.1 Selector "El historial del portapapeles está disponible en Canopy"
    let ret_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    ret_row.append(
        &Label::builder()
            .label("El historial del portapapeles está disponible en Canopy")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );

    let initial_ret_idx = retention_index_for_minutes(cfg.borrow().clipboard_retention_minutes);
    let ret_lbl = Label::builder()
        .label(RETENTION_OPTIONS[initial_ret_idx].0)
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .build();
    let ret_btn_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    ret_btn_box.append(&ret_lbl);
    ret_btn_box.append(
        &gtk4::Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let ret_menu_btn = MenuButton::builder()
        .child(&ret_btn_box)
        .has_frame(false)
        .css_classes(["macos-inline-selector-btn"])
        .valign(Align::Center)
        .sensitive(cfg.borrow().show_clipboard)
        .build();
    let ret_popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .css_classes(["macos-popover"])
        .build();
    let ret_pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["wifi-popover-box"])
        .build();
    for &(label, minutes) in &RETENTION_OPTIONS {
        let (item_wrap, item_btn) = make_popover_action_row(label, false, None);
        let c = cfg.clone();
        let lbl_c = ret_lbl.clone();
        let pop_c = ret_popover.clone();
        item_btn.connect_clicked(move |_| {
            c.borrow_mut().clipboard_retention_minutes = minutes;
            let _ = c.borrow().save();
            lbl_c.set_label(label);
            pop_c.popdown();
        });
        ret_pop_box.append(&item_wrap);
    }
    ret_popover.set_child(Some(&ret_pop_box));
    ret_menu_btn.set_popover(Some(&ret_popover));
    ret_row.append(&ret_menu_btn);
    clip_card.append(&ret_row);

    // 6.2 Botón "Borrar el historial del portapapeles" (real)
    let clear_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let clear_feedback = Label::builder()
        .label("")
        .css_classes(["card-section-sub"])
        .valign(Align::Center)
        .visible(false)
        .build();
    let (clear_wrap, clear_btn) =
        make_squircle_button("Borrar el historial del portapapeles", false, None);
    clear_btn.set_sensitive(cfg.borrow().show_clipboard);
    {
        let fb = clear_feedback.clone();
        clear_btn.connect_clicked(move |_| {
            if CanopyConfig::clear_clipboard_history().is_ok() {
                fb.set_label("Historial borrado");
                fb.set_visible(true);
            }
        });
    }
    clear_row.append(&clear_feedback);
    clear_row.append(&clear_wrap);
    clip_card.append(&clear_row);

    {
        let c = cfg.clone();
        let menu_c = ret_menu_btn.clone();
        let btn_c = clear_btn.clone();
        clip_sw.connect_state_set(move |_, active| {
            c.borrow_mut().show_clipboard = active;
            let _ = c.borrow().save();
            menu_c.set_sensitive(active);
            btn_c.set_sensitive(active);
            glib::Propagation::Proceed
        });
    }

    pane.append(&wrap_squircle_card(&clip_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 7. Pie inferior ("Buscar y la privacidad..." + "?")
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::End)
        .margin_top(2)
        .build();
    footer.append(&make_pending_badge());
    let (priv_wrap, priv_btn) =
        make_squircle_button("Buscar y la privacidad...", false, Some("Por implementar"));
    priv_btn.set_sensitive(false);
    let (help_wrap, _) = make_help_circle_button("Ayuda sobre Canopy");
    footer.append(&priv_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    let refresh: Rc<dyn Fn()> = {
        let c = cfg.clone();
        let a_sw = apps_sw;
        let f_sw = files_sw;
        let d_sw = folders_sw;
        let cl_sw = clip_sw;
        let r_lbl = ret_lbl;
        let r_menu = ret_menu_btn;
        let c_btn = clear_btn;
        let c_fb = clear_feedback;
        Rc::new(move || {
            let loaded = CanopyConfig::load();
            *c.borrow_mut() = loaded.clone();
            a_sw.set_active(loaded.show_apps);
            f_sw.set_active(loaded.show_files);
            d_sw.set_active(loaded.show_folders);
            cl_sw.set_active(loaded.show_clipboard);
            let idx = retention_index_for_minutes(loaded.clipboard_retention_minutes);
            r_lbl.set_label(RETENTION_OPTIONS[idx].0);
            r_menu.set_sensitive(loaded.show_clipboard);
            c_btn.set_sensitive(loaded.show_clipboard);
            c_fb.set_visible(false);
        })
    };

    (scrolled, refresh)
}

fn make_working_switch_row(
    title: &str,
    initial: bool,
    on_toggle: impl Fn(bool) + 'static,
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

fn make_pending_switch_row(title: &str) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    row.append(&make_pending_badge());
    let sw = Switch::builder()
        .active(false)
        .sensitive(false)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    row.append(&*sw);
    row
}

fn make_pending_switch_card_content(title: &str, description: &str) -> GtkBox {
    let box_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .css_classes(["wifi-pref-row"])
        .build();
    let top = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    top.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    top.append(&make_pending_badge());
    let sw = Switch::builder()
        .active(false)
        .sensitive(false)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    top.append(&*sw);
    box_col.append(&top);
    box_col.append(
        &Label::builder()
            .label(description)
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    box_col
}

fn make_pending_button_row(title: &str, subtitle: Option<&str>, button_label: &str) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    text_col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    if let Some(sub) = subtitle {
        text_col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    row.append(&text_col);
    row.append(&make_pending_badge());
    let (btn_wrap, btn) = make_squircle_button(button_label, false, Some("Por implementar"));
    btn.set_sensitive(false);
    row.append(&btn_wrap);
    row
}

fn make_files_info_button() -> gtk4::Overlay {
    let btn = Button::builder()
        .label("i")
        .has_frame(false)
        .tooltip_text("Carpetas indexadas por Canopy")
        .css_classes(["wifi-help-btn"])
        .width_request(20)
        .height_request(20)
        .valign(Align::Center)
        .build();

    let popover = Popover::builder()
        .has_arrow(true)
        .autohide(true)
        .css_classes(["macos-popover"])
        .build();
    let pop_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    pop_box.append(
        &Label::builder()
            .label("Carpetas indexadas")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    pop_box.append(
        &Label::builder()
            .label("Canopy indexa tus archivos en Escritorio, Documentos, Descargas, Proyectos, Imágenes, Música y Vídeos.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .max_width_chars(38)
            .xalign(0.0)
            .build(),
    );
    popover.set_child(Some(&pop_box));
    popover.set_parent(&btn);

    let pop_c = popover.clone();
    btn.connect_clicked(move |_| pop_c.popup());

    wrap_squircle_round_control(&btn, 20, RoundControlStyle::CircleOutline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canopy_panel_constants_and_retention_indices_are_consistent() {
        assert_eq!(CANOPY_PANEL_ID, "spotlight");
        assert_eq!(CANOPY_PANEL_TITLE, "Canopy");
        assert_eq!(retention_index_for_minutes(30), 0);
        assert_eq!(retention_index_for_minutes(480), 1);
        assert_eq!(retention_index_for_minutes(10_080), 2);
        assert_eq!(retention_index_for_minutes(999), 1);
    }
}
