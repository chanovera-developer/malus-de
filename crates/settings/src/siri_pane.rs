use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, Image, Label, Orientation, PolicyType,
    ScrolledWindow,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::f64::consts::TAU;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS};

pub const SIRI_PANEL_ID: &str = "siri";
pub const SIRI_PANEL_TITLE: &str = "Pip";

/// Crea el panel «Pip» (asistente de Malus DE) estilo macOS Tahoe en Ajustes.
/// Todas las funciones del asistente llevan la etiqueta «por implementar» mientras
/// Malus DE no integre un asistente propio; el enlace inferior abre la subvista
/// de accesibilidad correspondiente (`a11y-siri`).
pub fn create_siri_pane(navigate: Rc<dyn Fn(&str)>) -> ScrolledWindow {
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

    // 1. Tarjeta de cabecera (icono Siri + descripción + etiqueta "por implementar")
    let header_card = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    header_card.append(&make_siri_orb_badge());

    let header_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    header_col.append(
        &Label::builder()
            .label(SIRI_PANEL_TITLE)
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    header_col.append(
        &Label::builder()
            .label("Pip te ayuda a realizar tareas con tan sólo pedírselo. Malus DE todavía no cuenta con un asistente integrado.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    header_card.append(&header_col);
    header_card.append(&make_pending_badge());
    pane.append(&wrap_squircle_card(&header_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 2. Tarjeta de activación ("Activar Pip")
    let activate_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let activate_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    activate_row.append(
        &Label::builder()
            .label("Activar Pip")
            .css_classes(["siri-accent-action"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    activate_row.append(&make_pending_badge());
    activate_card.append(&activate_row);
    pane.append(&wrap_squircle_card(&activate_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 3. Tarjeta Idioma / Voz / Respuestas
    let voice_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    voice_card.append(&make_pending_detail_row("Idioma", Some("español (México)")));
    voice_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    voice_card.append(&make_pending_detail_row("Voz", Some("Mexicana (Voz 2)")));
    voice_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    voice_card.append(&make_pending_detail_row("Respuestas", None));
    pane.append(&wrap_squircle_card(&voice_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&make_footnote(
        "Esta voz se usará para las indicaciones detalladas de Mapas, la función Escuchar lectura de la página, ciertas funciones de accesibilidad y Atajos.",
    ));

    // 4. Sección "Extensión"
    pane.append(
        &Label::builder()
            .label("Extensión")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );

    let ext_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    ext_card.append(&make_pending_detail_row("ChatGPT", Some("Configurar")));
    pane.append(&wrap_squircle_card(&ext_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&make_footnote(
        "Tu equipo puede usar una extensión de IA para ayudarte a escribir, crear imágenes, responder preguntas y más.",
    ));

    // 5. Tarjeta "Consulta visual automática"
    let visual_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let visual_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    visual_row.append(
        &Label::builder()
            .label("Consulta visual automática")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    visual_row.append(&make_pending_badge());
    let visual_sw = Switch::builder()
        .active(false)
        .sensitive(false)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    visual_row.append(&*visual_sw);
    visual_card.append(&visual_row);
    pane.append(&wrap_squircle_card(&visual_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&make_footnote(
        "Identifica los objetos y lugares que se muestran en tus capturas de pantalla.",
    ));

    // 6. Tarjeta "Apps excluidas"
    let excluded_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    excluded_card.append(&make_pending_detail_row("Apps excluidas", None));
    pane.append(&wrap_squircle_card(&excluded_card, WIFI_CARD_SQUIRCLE_RADIUS));
    pane.append(&make_footnote(
        "Pip usará inteligencia del sistema para buscar información y llevar a cabo acciones en tus apps, protegiendo tu privacidad y manteniéndote en control de tus datos personales.",
    ));

    // 7. Tarjeta "Sugerencias"
    let suggestions_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    suggestions_card.append(&make_pending_detail_row("Sugerencias", None));
    pane.append(&wrap_squircle_card(&suggestions_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // 8. Tarjeta "Configuración de accesibilidad" (navega a Accesibilidad > Siri)
    let a11y_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let a11y_row_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    a11y_row_box.append(
        &Label::builder()
            .label("Configuración de accesibilidad")
            .css_classes(["siri-accent-action"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    let a11y_btn = Button::builder()
        .child(&a11y_row_box)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    a11y_btn.connect_clicked(move |_| navigate("a11y-siri"));
    a11y_card.append(&a11y_btn);
    pane.append(&wrap_squircle_card(&a11y_card, WIFI_CARD_SQUIRCLE_RADIUS));

    scrolled
}

fn make_pending_detail_row(title: &str, value: Option<&str>) -> GtkBox {
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
    if let Some(val) = value {
        row.append(
            &Label::builder()
                .label(val)
                .css_classes(["card-section-sub"])
                .valign(Align::Center)
                .build(),
        );
    }
    row.append(&make_pending_badge());
    row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    row
}

fn make_footnote(text: &str) -> Label {
    Label::builder()
        .label(text)
        .css_classes(["siri-footnote"])
        .wrap(true)
        .xalign(0.0)
        .build()
}

/// Dibuja un ícono squircle 34×34 con orbe iridiscente estilo asistente de voz.
fn make_siri_orb_badge() -> DrawingArea {
    let area = DrawingArea::builder()
        .content_width(34)
        .content_height(34)
        .valign(Align::Center)
        .halign(Align::Center)
        .build();

    area.set_draw_func(|_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        if wf <= 2.0 || hf <= 2.0 {
            return;
        }
        let _ = cr.save();
        crate::squircle_path(cr, wf, hf, WIFI_BADGE_SQUIRCLE_RADIUS);
        cr.clip();

        // Fondo luminoso suave estilo macOS Tahoe
        let bg = gtk4::cairo::LinearGradient::new(0.0, 0.0, wf, hf);
        bg.add_color_stop_rgba(0.0, 0.96, 0.94, 0.99, 1.0);
        bg.add_color_stop_rgba(1.0, 0.89, 0.94, 0.99, 1.0);
        let _ = cr.set_source(&bg);
        let _ = cr.paint();

        let cx = wf * 0.5;
        let cy = hf * 0.5;
        let r = wf.min(hf) * 0.37;

        // Orbe base radial
        let orb =
            gtk4::cairo::RadialGradient::new(cx - r * 0.25, cy - r * 0.25, r * 0.1, cx, cy, r);
        orb.add_color_stop_rgba(0.0, 0.99, 0.72, 0.88, 0.95);
        orb.add_color_stop_rgba(0.45, 0.62, 0.48, 0.98, 0.95);
        orb.add_color_stop_rgba(0.82, 0.18, 0.74, 0.98, 0.98);
        orb.add_color_stop_rgba(1.0, 0.42, 0.30, 0.90, 0.96);
        cr.arc(cx, cy, r, 0.0, TAU);
        let _ = cr.set_source(&orb);
        let _ = cr.fill();

        // Dos lazos luminosos cruzados en el interior del orbe
        cr.set_line_width(1.6);
        cr.set_source_rgba(1.0, 0.92, 0.98, 0.88);
        let _ = cr.save();
        cr.translate(cx, cy);
        cr.rotate(0.45);
        cr.scale(1.0, 0.52);
        cr.arc(0.0, 0.0, r * 0.76, 0.0, TAU);
        let _ = cr.restore();
        let _ = cr.stroke();

        cr.set_source_rgba(0.75, 0.98, 1.0, 0.84);
        let _ = cr.save();
        cr.translate(cx, cy);
        cr.rotate(-0.45);
        cr.scale(1.0, 0.52);
        cr.arc(0.0, 0.0, r * 0.76, 0.0, TAU);
        let _ = cr.restore();
        let _ = cr.stroke();

        // Borde sutil del squircle
        crate::squircle_path(cr, wf - 1.0, hf - 1.0, WIFI_BADGE_SQUIRCLE_RADIUS);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.08);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let _ = cr.restore();
    });

    area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siri_panel_constants_match_sidebar_and_header() {
        assert_eq!(SIRI_PANEL_ID, "siri");
        assert_eq!(SIRI_PANEL_TITLE, "Pip");
    }
}
