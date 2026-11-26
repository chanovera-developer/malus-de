use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, EventControllerKey, FlowBox, Image, Label,
    Orientation, Overlay, PolicyType, ScrolledWindow, SearchEntry, SelectionMode, Window,
};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use malus_de_common::config::{ControlCenterConfig, ControlWidgetKind, MenuBarConfig};
use std::cell::RefCell;
use std::rc::Rc;

use crate::ui::cc_dnd::{
    attach_drag_source, squircle_path, unpin_widget_from_tray, DragPayload,
};

#[derive(Clone, Copy)]
struct EditorCategory {
    id: &'static str,
    label: &'static str,
    icon_name: &'static str,
    badge_class: &'static str,
}

const CATEGORIES: &[EditorCategory] = &[
    EditorCategory {
        id: "all",
        label: "Todos los controles",
        icon_name: "view-grid-symbolic",
        badge_class: "cc-ed-cat-gray",
    },
    EditorCategory {
        id: "connectivity",
        label: "Conectividad",
        icon_name: "network-wireless-symbolic",
        badge_class: "cc-ed-cat-blue",
    },
    EditorCategory {
        id: "now_playing",
        label: "Ahora suena",
        icon_name: "audio-x-generic-symbolic",
        badge_class: "cc-ed-cat-pink",
    },
    EditorCategory {
        id: "focus",
        label: "Enfoque",
        icon_name: "weather-clear-night-symbolic",
        badge_class: "cc-ed-cat-indigo",
    },
    EditorCategory {
        id: "desktop",
        label: "Escritorio y Finder",
        icon_name: "user-desktop-symbolic",
        badge_class: "cc-ed-cat-dark",
    },
    EditorCategory {
        id: "display",
        label: "Pantalla y brillo",
        icon_name: "display-brightness-symbolic",
        badge_class: "cc-ed-cat-blue",
    },
    EditorCategory {
        id: "sound",
        label: "Sonidos",
        icon_name: "audio-volume-high-symbolic",
        badge_class: "cc-ed-cat-pink",
    },
];

pub struct ControlCenterEditorHandle {
    pub window: Window,
    refresh_cards: Rc<dyn Fn()>,
}

impl ControlCenterEditorHandle {
    pub fn show(&self) {
        (self.refresh_cards)();
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn refresh(&self) {
        (self.refresh_cards)();
    }

    pub fn sync_theme(&self, is_dark: bool) {
        if is_dark {
            self.window.remove_css_class("light-mode");
        } else {
            self.window.add_css_class("light-mode");
        }
    }
}

pub fn create_control_center_editor_window(
    app: Option<&gtk4::Application>,
    on_config_changed: Rc<dyn Fn()>,
    on_done: Rc<dyn Fn()>,
) -> ControlCenterEditorHandle {
    let mut win_builder = Window::builder()
        .title("Editar controles")
        .decorated(false)
        .resizable(false)
        .default_width(720)
        .default_height(540)
        .css_classes(["cc-editor-window"]);
    if let Some(a) = app {
        win_builder = win_builder.application(a);
    }
    let window = win_builder.build();
    if !malus_de_common::config::is_dark_mode_active() {
        window.add_css_class("light-mode");
    }

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-control-center-editor");
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    }

    let shell = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-editor-shell"])
        .width_request(720)
        .height_request(540)
        .build();
    shell.set_overflow(gtk4::Overflow::Hidden);

    // ==========================================
    // Barra lateral izquierda (búsqueda + categorías)
    // ==========================================
    let sidebar = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-editor-sidebar"])
        .width_request(224)
        .hexpand(false)
        .halign(Align::Start)
        .spacing(10)
        .build();

    let search_entry = SearchEntry::builder()
        .placeholder_text("Buscar controles")
        .css_classes(["cc-editor-search"])
        .width_chars(10)
        .max_width_chars(16)
        .hexpand(true)
        .build();
    sidebar.append(&search_entry);

    let cat_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .propagate_natural_width(false)
        .vexpand(true)
        .build();
    let cat_list = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Start)
        .vexpand(false)
        .spacing(2)
        .build();
    cat_scroll.set_child(Some(&cat_list));
    sidebar.append(&cat_scroll);

    // ==========================================
    // Panel derecho (Novedades + galería + barra inferior)
    // ==========================================
    let right_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-editor-main"])
        .hexpand(true)
        .vexpand(true)
        .build();

    let content_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-editor-content"])
        .spacing(14)
        .build();
    content_scroll.set_child(Some(&content_box));
    right_col.append(&content_scroll);

    // Barra inferior con texto de ayuda y botón azul "Listo"
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-editor-footer"])
        .spacing(14)
        .halign(Align::Fill)
        .valign(Align::End)
        .hexpand(true)
        .vexpand(false)
        .build();

    let hint_lbl = Label::builder()
        .label("Arrastra un control para colocarlo en el centro de control o en la barra de menús...")
        .css_classes(["cc-editor-hint"])
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .build();

    let done_lbl = Label::builder()
        .label("Listo")
        .css_classes(["cc-editor-done-label"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let done_btn = Button::builder()
        .css_classes(["cc-editor-done-btn"])
        .halign(Align::End)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .has_frame(false)
        .child(&done_lbl)
        .build();
    apply_squircle_accent_button(&done_btn, 10.0, 5, 18);

    footer.append(&hint_lbl);
    footer.append(&done_btn);
    right_col.append(&footer);

    shell.append(&sidebar);
    shell.append(&right_col);

    let squircle_container = Overlay::builder()
        .width_request(720)
        .height_request(540)
        .halign(Align::Fill)
        .valign(Align::Fill)
        .build();

    // Contenedor squircle continuo (n = 4.2, r = 24px) que recorta el fondo, la barra lateral
    // y el pie de ventana usando las coordenadas exactas (`compute_bounds`) de cada sección.
    let editor_bg = DrawingArea::new();
    editor_bg.set_can_target(false);
    editor_bg.set_hexpand(true);
    editor_bg.set_vexpand(true);
    let container_weak = squircle_container.downgrade();
    let sidebar_weak = sidebar.downgrade();
    let footer_weak = footer.downgrade();
    editor_bg.set_draw_func(move |_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        if wf <= 2.0 || hf <= 2.0 {
            return;
        }
        let dark = malus_de_common::config::is_dark_mode_active();
        let scale = malus_de_common::config::liquid_glass_scale_for(
            malus_de_common::config::load_liquid_glass_opacity(),
        );
        let base_a = malus_de_common::config::scaled_liquid_glass_alpha(0.74, scale, 0.16, 0.96);

        let _ = cr.save();
        squircle_path(cr, 0.5, 0.5, wf - 1.0, hf - 1.0, 34.0);
        cr.clip_preserve();

        // Fondo base translúcido
        if dark {
            cr.set_source_rgba(30.0 / 255.0, 30.0 / 255.0, 36.0 / 255.0, base_a);
        } else {
            cr.set_source_rgba(244.0 / 255.0, 246.0 / 255.0, 250.0 / 255.0, base_a);
        }
        let _ = cr.fill();

        // Borde derecho exacto del sidebar en coordenadas del contenedor squircle
        let sidebar_w = container_weak
            .upgrade()
            .zip(sidebar_weak.upgrade())
            .and_then(|(cont, sb)| sb.compute_bounds(&cont))
            .map(|b| (b.x() + b.width()) as f64)
            .filter(|&sw| sw > 120.0 && sw < wf - 120.0)
            .unwrap_or(224.0)
            .round();
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.04);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.32);
        }
        cr.rectangle(0.0, 0.0, sidebar_w, hf);
        let _ = cr.fill();

        // Divisor vertical alineado exactamente con el borde derecho real del sidebar
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.11);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.08);
        }
        cr.set_line_width(1.0);
        cr.move_to(sidebar_w + 0.5, 0.0);
        cr.line_to(sidebar_w + 0.5, hf);
        let _ = cr.stroke();

        // Borde superior exacto del pie inferior derecho en coordenadas del contenedor squircle
        let footer_y = container_weak
            .upgrade()
            .zip(footer_weak.upgrade())
            .and_then(|(cont, ft)| ft.compute_bounds(&cont))
            .map(|b| b.y() as f64)
            .filter(|&fy| fy > hf * 0.5 && fy < hf - 20.0)
            .unwrap_or(hf - 58.0)
            .round();
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.03);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.28);
        }
        cr.rectangle(sidebar_w, footer_y, wf - sidebar_w, hf - footer_y);
        let _ = cr.fill();

        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.08);
        }
        cr.move_to(sidebar_w, footer_y + 0.5);
        cr.line_to(wf, footer_y + 0.5);
        let _ = cr.stroke();

        let _ = cr.restore();

        // Borde squircle exterior limpio de 1px
        let _ = cr.save();
        squircle_path(cr, 0.5, 0.5, wf - 1.0, hf - 1.0, 34.0);
        if dark {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.10);
        }
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let _ = cr.restore();
    });

    let bg_weak = editor_bg.downgrade();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(area) = bg_weak.upgrade() {
            area.queue_draw();
        }
    });

    // Redibujar divisores del fondo justo después del primer layout real al mostrarse la ventana
    {
        let bg_map = editor_bg.clone();
        window.connect_show(move |_| {
            let bg1 = bg_map.clone();
            glib::idle_add_local_once(move || bg1.queue_draw());
            let bg2 = bg_map.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
                bg2.queue_draw();
            });
        });
    }

    squircle_container.set_child(Some(&editor_bg));
    squircle_container.add_overlay(&shell);
    squircle_container.set_measure_overlay(&shell, true);

    window.set_child(Some(&squircle_container));

    let selected_cat = Rc::new(RefCell::new(String::from("all")));
    let search_text = Rc::new(RefCell::new(String::new()));
    let cat_buttons: Rc<RefCell<Vec<(String, Button)>>> = Rc::new(RefCell::new(Vec::new()));

    let refresh_cards: Rc<dyn Fn()> = {
        let content_c = content_box.clone();
        let cat_c = selected_cat.clone();
        let search_c = search_text.clone();
        let on_cfg = on_config_changed.clone();
        let self_refresh_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let holder_inner = self_refresh_holder.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            let active_cat = cat_c.borrow().clone();
            let query = search_c.borrow().trim().to_lowercase();
            let mb_cfg = MenuBarConfig::load();
            let mut cc_cfg = ControlCenterConfig::load();
            cc_cfg.sync_with_menubar(&mb_cfg);

            while let Some(child) = content_c.first_child() {
                content_c.remove(&child);
            }

            if active_cat == "all" && query.is_empty() {
                content_c.append(&build_whats_new_banner());
            }

            let sections: Vec<(&str, &str)> = if active_cat == "all" {
                vec![
                    ("connectivity", "Conectividad"),
                    ("now_playing", "Ahora suena"),
                    ("desktop", "Escritorio y Finder"),
                    ("focus", "Enfoque"),
                    ("display", "Pantalla y brillo"),
                    ("sound", "Sonidos"),
                ]
            } else {
                CATEGORIES
                    .iter()
                    .find(|c| c.id == active_cat)
                    .map(|c| vec![(c.id, c.label)])
                    .unwrap_or_default()
            };

            let mut total_shown = 0usize;
            for (sec_id, sec_title) in sections {
                let matching: Vec<ControlWidgetKind> = ControlWidgetKind::ALL
                    .iter()
                    .copied()
                    .filter(|k| k.category_id() == sec_id)
                    .filter(|k| {
                        if query.is_empty() {
                            return true;
                        }
                        k.display_name(malus_de_common::is_spanish_locale()).to_lowercase().contains(&query)
                            || sec_title.to_lowercase().contains(&query)
                            || k.id().contains(&query)
                    })
                    .collect();

                if matching.is_empty() {
                    continue;
                }
                total_shown += matching.len();

                let sec_lbl = Label::builder()
                    .label(sec_title)
                    .css_classes(["cc-editor-section-title"])
                    .xalign(0.0)
                    .build();
                content_c.append(&sec_lbl);

                let flow = FlowBox::builder()
                    .selection_mode(SelectionMode::None)
                    .column_spacing(12)
                    .row_spacing(14)
                    .homogeneous(false)
                    .max_children_per_line(3)
                    .min_children_per_line(1)
                    .build();

                for kind in matching {
                    let in_cc = cc_cfg.contains(kind);
                    let in_tray = mb_cfg.is_widget_in_tray(kind);
                    let card = build_gallery_widget_card(
                        kind,
                        in_cc,
                        in_tray,
                        on_cfg.clone(),
                        holder_inner.clone(),
                    );
                    flow.insert(&card, -1);
                }
                content_c.append(&flow);
            }

            if total_shown == 0 {
                let empty = Label::builder()
                    .label("No se encontraron controles")
                    .css_classes(["cc-editor-empty"])
                    .margin_top(40)
                    .build();
                content_c.append(&empty);
            }
        });
        *self_refresh_holder.borrow_mut() = Some(f.clone());
        f
    };

    // Poblar botones de categoría en la barra lateral
    for cat in CATEGORIES {
        let btn = Button::builder()
            .css_classes(["cc-editor-cat-btn"])
            .has_frame(false)
            .halign(Align::Fill)
            .valign(Align::Start)
            .hexpand(true)
            .vexpand(false)
            .build();
        if cat.id == "all" {
            btn.add_css_class("cc-editor-cat-btn-active");
        }

        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .halign(Align::Fill)
            .valign(Align::Center)
            .hexpand(true)
            .build();

        let badge = GtkBox::builder()
            .css_classes(["cc-editor-cat-badge", cat.badge_class])
            .width_request(24)
            .height_request(24)
            .halign(Align::Start)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        let ico = Image::builder()
            .icon_name(cat.icon_name)
            .pixel_size(14)
            .width_request(24)
            .height_request(24)
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        badge.append(&ico);

        let lbl = Label::builder()
            .label(cat.label)
            .css_classes(["cc-editor-cat-label"])
            .halign(Align::Start)
            .valign(Align::Center)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .hexpand(true)
            .vexpand(false)
            .build();

        row.append(&badge);
        row.append(&lbl);
        btn.set_child(Some(&row));

        let cat_id = cat.id.to_string();
        let sel_c = selected_cat.clone();
        let btns_c = cat_buttons.clone();
        let ref_c = refresh_cards.clone();
        btn.connect_clicked(move |_| {
            *sel_c.borrow_mut() = cat_id.clone();
            for (id, b) in btns_c.borrow().iter() {
                if id == &cat_id {
                    b.add_css_class("cc-editor-cat-btn-active");
                } else {
                    b.remove_css_class("cc-editor-cat-btn-active");
                }
            }
            ref_c();
        });

        cat_buttons.borrow_mut().push((cat.id.to_string(), btn.clone()));
        cat_list.append(&btn);
    }

    // Búsqueda reactiva
    {
        let search_c = search_text.clone();
        let ref_c = refresh_cards.clone();
        search_entry.connect_search_changed(move |entry| {
            *search_c.borrow_mut() = entry.text().to_string();
            ref_c();
        });
    }

    // Botón Listo
    {
        let win_c = window.clone();
        let done_c = on_done.clone();
        done_btn.connect_clicked(move |_| {
            win_c.set_visible(false);
            done_c();
        });
    }

    // Tecla Escape cierra el editor y sale del modo edición
    let key_ctrl = EventControllerKey::new();
    {
        let win_c = window.clone();
        let done_c = on_done;
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                win_c.set_visible(false);
                done_c();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(key_ctrl);

    refresh_cards();

    ControlCenterEditorHandle {
        window,
        refresh_cards,
    }
}

fn build_whats_new_banner() -> GtkBox {
    let banner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-editor-banner"])
        .spacing(16)
        .build();

    let text_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .hexpand(true)
        .valign(Align::Center)
        .build();

    let title = Label::builder()
        .label("Novedades")
        .css_classes(["cc-editor-banner-title"])
        .xalign(0.0)
        .build();

    let body = Label::builder()
        .label(
            "El centro de control es más flexible que nunca: puedes personalizarlo de acuerdo a \
             tus prioridades cambiando el orden de los elementos. También puedes arrastrar tus \
             controles favoritos a la barra de menús y viceversa.",
        )
        .css_classes(["cc-editor-banner-body"])
        .wrap(true)
        .xalign(0.0)
        .build();

    text_col.append(&title);
    text_col.append(&body);

    // Ilustración minimalista del Centro de Control flotante
    let preview = DrawingArea::builder()
        .content_width(142)
        .content_height(92)
        .halign(Align::End)
        .valign(Align::Center)
        .build();
    preview.set_draw_func(|_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        // Fondo degradado tipo escritorio Tahoe
        let pat = gtk4::cairo::LinearGradient::new(0.0, 0.0, wf, hf);
        pat.add_color_stop_rgba(0.0, 0.48, 0.56, 0.85, 0.85);
        pat.add_color_stop_rgba(0.55, 0.55, 0.45, 0.78, 0.88);
        pat.add_color_stop_rgba(1.0, 0.72, 0.42, 0.62, 0.88);
        let _ = cr.set_source(&pat);
        rounded_rect_path(cr, 0.0, 0.0, wf, hf, 12.0);
        let _ = cr.fill();

        // Ventana de galería izquierda
        cr.set_source_rgba(0.95, 0.96, 0.99, 0.72);
        rounded_rect_path(cr, 10.0, 12.0, 74.0, 68.0, 8.0);
        let _ = cr.fill();

        // Píldoras del Centro de Control a la derecha
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.52);
        rounded_rect_path(cr, 92.0, 12.0, 20.0, 11.0, 5.5);
        let _ = cr.fill();
        rounded_rect_path(cr, 92.0, 26.0, 20.0, 11.0, 5.5);
        let _ = cr.fill();
        rounded_rect_path(cr, 115.0, 12.0, 18.0, 25.0, 6.0);
        let _ = cr.fill();
        rounded_rect_path(cr, 92.0, 41.0, 41.0, 11.0, 5.5);
        let _ = cr.fill();
        rounded_rect_path(cr, 92.0, 56.0, 41.0, 11.0, 5.5);
        let _ = cr.fill();
    });

    banner.append(&text_col);
    banner.append(&preview);
    banner
}

fn build_gallery_widget_card(
    kind: ControlWidgetKind,
    in_cc: bool,
    in_tray: bool,
    on_config_changed: Rc<dyn Fn()>,
    refresh_self: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
) -> GtkBox {
    let outer = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .halign(Align::Center)
        .valign(Align::Start)
        .build();

    let (col_span, row_span) = kind.default_span();
    let tile_btn = Button::builder()
        .css_classes(["cc-editor-preview-tile"])
        .has_frame(false)
        .tooltip_text(if in_cc {
            "En el Centro de Control (arrastra para mover a la barra de menús)"
        } else if in_tray {
            "En la barra de menús (clic o arrastra para moverlo al Centro de Control)"
        } else {
            "Clic para añadir o arrastra al Centro de Control / barra de menús"
        })
        .build();

    match (col_span, row_span) {
        (1, 1) => {
            tile_btn.set_size_request(76, 76);
            tile_btn.add_css_class("cc-editor-preview-circle");
            let ico = Image::builder()
                .icon_name(kind.icon_name())
                .pixel_size(18)
                .halign(Align::Center)
                .valign(Align::Center)
                .build();
            tile_btn.set_child(Some(&ico));
        }
        (2, 2) => {
            tile_btn.set_size_request(132, 120);
            tile_btn.add_css_class("cc-editor-preview-media");
            let v = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(6)
                .valign(Align::Center)
                .build();
            let art = GtkBox::builder()
                .css_classes(["cc-editor-preview-art"])
                .width_request(36)
                .height_request(36)
                .halign(Align::Start)
                .build();
            let t = Label::builder()
                .label("Sin reproducción")
                .css_classes(["cc-editor-preview-title"])
                .xalign(0.0)
                .build();
            let ctrls = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(12)
                .halign(Align::Center)
                .build();
            for n in [
                "media-skip-backward-symbolic",
                "media-playback-start-symbolic",
                "media-skip-forward-symbolic",
            ] {
                let im = Image::from_icon_name(n);
                im.set_pixel_size(14);
                ctrls.append(&im);
            }
            v.append(&art);
            v.append(&t);
            v.append(&ctrls);
            tile_btn.set_child(Some(&v));
        }
        (4, 1) => {
            tile_btn.set_size_request(186, 66);
            tile_btn.add_css_class("cc-editor-preview-pill");
            let v = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(8)
                .valign(Align::Center)
                .build();
            let t = Label::builder()
                .label(kind.display_name(malus_de_common::is_spanish_locale()))
                .css_classes(["cc-editor-preview-title"])
                .xalign(0.0)
                .build();
            let bar_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(6)
                .valign(Align::Center)
                .build();
            let ico = Image::from_icon_name(kind.icon_name());
            ico.set_pixel_size(14);
            let bar = GtkBox::builder()
                .css_classes(["cc-editor-preview-slider"])
                .hexpand(true)
                .valign(Align::Center)
                .build();
            bar_row.append(&ico);
            bar_row.append(&bar);
            v.append(&t);
            v.append(&bar_row);
            tile_btn.set_child(Some(&v));
        }
        _ => {
            // 2x1 pill
            tile_btn.set_size_request(152, 66);
            tile_btn.add_css_class("cc-editor-preview-pill");
            let row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(10)
                .valign(Align::Center)
                .build();
            let circle = GtkBox::builder()
                .css_classes(["cc-editor-preview-icon-circle"])
                .width_request(34)
                .height_request(34)
                .halign(Align::Center)
                .valign(Align::Center)
                .build();
            let ico = Image::builder()
                .icon_name(kind.icon_name())
                .pixel_size(16)
                .halign(Align::Center)
                .valign(Align::Center)
                .hexpand(true)
                .vexpand(true)
                .build();
            circle.append(&ico);

            let t = Label::builder()
                .label(kind.display_name(malus_de_common::is_spanish_locale()))
                .css_classes(["cc-editor-preview-title"])
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .hexpand(true)
                .build();
            row.append(&circle);
            row.append(&t);
            tile_btn.set_child(Some(&row));
        }
    }

    let overlay = Overlay::new();
    overlay.set_child(Some(&tile_btn));

    let badge = Label::builder()
        .label(if in_cc {
            "✓"
        } else if in_tray {
            "↑"
        } else {
            "+"
        })
        .css_classes(if in_cc {
            &["cc-editor-card-badge", "cc-editor-card-badge-added"][..]
        } else {
            &["cc-editor-card-badge"][..]
        })
        .halign(Align::End)
        .valign(Align::Start)
        .margin_top(4)
        .margin_end(4)
        .build();
    overlay.add_overlay(&badge);

    // Clic rápido: mueve/añade el control al Centro de Control (quitándolo de la barra si estaba allí)
    {
        let on_cfg = on_config_changed.clone();
        let ref_self = refresh_self.clone();
        tile_btn.connect_clicked(move |_| {
            unpin_widget_from_tray(kind);
            let mut cfg = ControlCenterConfig::load();
            if cfg.add_widget(kind) {
                let _ = cfg.save();
            }
            on_cfg();
            if let Some(ref f) = *ref_self.borrow() {
                f();
            }
        });
    }

    // Arrastre directo al Centro de Control o a la barra de menús
    attach_drag_source(
        &tile_btn,
        DragPayload::FromControlCenter(kind),
        None,
        None,
    );

    let caption = Label::builder()
        .label(kind.display_name(malus_de_common::is_spanish_locale()))
        .css_classes(["cc-editor-card-caption"])
        .halign(Align::Center)
        .build();

    outer.append(&overlay);
    outer.append(&caption);
    outer
}

fn apply_squircle_accent_button(btn: &Button, radius: f64, pad_v: i32, pad_h: i32) {
    let Some(inner) = btn.child() else {
        return;
    };
    btn.set_child(gtk4::Widget::NONE);
    inner.set_margin_top(pad_v);
    inner.set_margin_bottom(pad_v);
    inner.set_margin_start(pad_h);
    inner.set_margin_end(pad_h);
    inner.set_halign(Align::Center);
    inner.set_valign(Align::Center);

    let bg = DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let wf = w as f64;
        let hf = h as f64;
        if wf <= 2.0 || hf <= 2.0 {
            return;
        }
        let (hovered, pressed) = btn_weak
            .upgrade()
            .map(|b| {
                let f = b.state_flags();
                (
                    f.contains(gtk4::StateFlags::PRELIGHT),
                    f.contains(gtk4::StateFlags::ACTIVE),
                )
            })
            .unwrap_or((false, false));

        let (mut r, mut g, mut b_col) = malus_de_common::config::load_accent_color();
        if pressed {
            r *= 0.86;
            g *= 0.86;
            b_col *= 0.86;
        } else if hovered {
            r = (r + 0.08).min(1.0);
            g = (g + 0.08).min(1.0);
            b_col = (b_col + 0.08).min(1.0);
        }

        let _ = cr.save();
        squircle_path(cr, 0.5, 0.5, wf - 1.0, hf - 1.0, radius);
        cr.set_source_rgba(r, g, b_col, 1.0);
        let _ = cr.fill();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = Overlay::builder()
        .halign(Align::Center)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner);
    overlay.set_measure_overlay(&inner, true);
    btn.set_child(Some(&overlay));
}

fn rounded_rect_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}
