use anyhow::{Context, Result};
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, CheckButton, CssProvider, DrawingArea, EventControllerKey,
    EventControllerMotion, GestureDrag, Label, MenuButton, Orientation, Overlay, Popover,
    Separator, ToggleButton,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

pub const TOOLBAR_CSS: &str = include_str!("styles/toolbar.css");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarMode {
    CaptureFull,
    CaptureWindow,
    CaptureArea,
    RecordFull,
    RecordArea,
}

impl ToolbarMode {
    pub fn is_video(&self) -> bool {
        matches!(self, ToolbarMode::RecordFull | ToolbarMode::RecordArea)
    }

    pub fn is_area(&self) -> bool {
        matches!(self, ToolbarMode::CaptureArea | ToolbarMode::RecordArea)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveDestination {
    Screenshots,
    Desktop,
    Documents,
    Clipboard,
}

#[derive(Clone, Debug)]
pub struct ToolbarOptions {
    pub destination: SaveDestination,
    pub timer_seconds: u32,
    pub record_audio: bool,
    pub show_cursor: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectionRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub surface_width: f64,
    pub surface_height: f64,
}

#[derive(Clone, Debug)]
pub enum ToolbarResult {
    Capture {
        mode: ToolbarMode,
        rect: Option<SelectionRect>,
        options: ToolbarOptions,
    },
    Record {
        mode: ToolbarMode,
        rect: Option<SelectionRect>,
        options: ToolbarOptions,
    },
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragHandle {
    None,
    MoveBox,
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NewSelection,
}

struct InteractionState {
    mode: ToolbarMode,
    rect: SelectionRect,
    active_handle: DragHandle,
    drag_start_pointer: (f64, f64),
    drag_start_rect: SelectionRect,
    hover_pointer: Option<(f64, f64)>,
}

pub fn run_interactive_toolbar(default_mode: Option<ToolbarMode>) -> Result<ToolbarResult> {
    gtk4::init().context("No se pudo inicializar GTK4 para la barra de captura")?;

    let display = gtk4::gdk::Display::default()
        .context("No se encontró una pantalla gráfica Wayland para la barra de captura")?;
    let monitor = display
        .monitors()
        .item(0)
        .and_then(|item| item.downcast::<gtk4::gdk::Monitor>().ok())
        .context("No se encontró monitor conectado")?;

    let provider = CssProvider::new();
    provider.load_from_string(TOOLBAR_CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let initial_mode = default_mode.unwrap_or(ToolbarMode::CaptureArea);

    let initial_rect = SelectionRect {
        x: 200.0,
        y: 150.0,
        width: 800.0,
        height: 500.0,
        surface_width: 1920.0,
        surface_height: 1080.0,
    };

    let state = Rc::new(RefCell::new(InteractionState {
        mode: initial_mode,
        rect: initial_rect,
        active_handle: DragHandle::None,
        drag_start_pointer: (0.0, 0.0),
        drag_start_rect: initial_rect,
        hover_pointer: None,
    }));

    let result_holder = Rc::new(RefCell::new(ToolbarResult::Cancel));
    let main_loop = glib::MainLoop::new(None, false);

    let window = gtk4::Window::builder()
        .title("Barra de Captura de Pantalla Malus")
        .decorated(false)
        .css_classes(["malus-screenshot-selector"])
        .build();

    let overlay = Overlay::new();

    // 1. LIENZO DE SELECCIÓN Y OSCURECIMIENTO
    let drawing_area = DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    {
        let state = state.clone();
        drawing_area.set_draw_func(move |area, context, _, _| {
            let st = state.borrow();
            let sw = area.width().max(1) as f64;
            let sh = area.height().max(1) as f64;

            match st.mode {
                ToolbarMode::CaptureFull | ToolbarMode::RecordFull => {
                    // Pantalla completa despejada, sin scrim oscuro
                    context.set_source_rgba(0.0, 0.0, 0.0, 0.15);
                    let _ = context.paint();
                }
                ToolbarMode::CaptureWindow => {
                    // Modo ventana: atenuar ligeramente
                    context.set_source_rgba(0.0, 0.0, 0.0, 0.40);
                    let _ = context.paint();
                }
                ToolbarMode::CaptureArea | ToolbarMode::RecordArea => {
                    // Fondo atenuado
                    context.set_source_rgba(0.0, 0.0, 0.0, 0.45);
                    let _ = context.paint();

                    let r = st.rect;
                    let rx = r.x.clamp(0.0, sw);
                    let ry = r.y.clamp(0.0, sh);
                    let rw = r.width.clamp(10.0, sw - rx);
                    let rh = r.height.clamp(10.0, sh - ry);

                    // Recorte nítido (despejar el contenido debajo)
                    context.set_operator(gtk4::cairo::Operator::Clear);
                    context.rectangle(rx, ry, rw, rh);
                    let _ = context.fill();

                    // Borde de selección blanco con trazo nítido
                    context.set_operator(gtk4::cairo::Operator::Over);
                    context.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                    context.set_line_width(2.0);
                    context.rectangle(rx, ry, rw, rh);
                    let _ = context.stroke();

                    // 8 tiradores de redimensión (círculos blancos con borde sutil)
                    let handle_radius = 5.0;
                    let handles = [
                        (rx, ry),                     // NW
                        (rx + rw * 0.5, ry),          // N
                        (rx + rw, ry),                // NE
                        (rx + rw, ry + rh * 0.5),     // E
                        (rx + rw, ry + rh),           // SE
                        (rx + rw * 0.5, ry + rh),     // S
                        (rx, ry + rh),                // SW
                        (rx, ry + rh * 0.5),          // W
                    ];

                    for (hx, hy) in handles {
                        context.arc(hx, hy, handle_radius, 0.0, 2.0 * PI);
                        context.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                        let _ = context.fill_preserve();
                        context.set_source_rgba(0.12, 0.12, 0.14, 0.85);
                        context.set_line_width(1.5);
                        let _ = context.stroke();
                    }

                    // Etiqueta de dimensiones (píldora translúcida con ancho x alto)
                    let badge_text = format!("{} × {}", rw.round() as i32, rh.round() as i32);
                    context.set_font_size(12.0);
                    if let Ok(extents) = context.text_extents(&badge_text) {
                        let pad_x = 8.0;
                        let pad_y = 4.0;
                        let bw = extents.width() + pad_x * 2.0;
                        let bh = extents.height() + pad_y * 2.0;

                        let mut badge_y = ry + rh + 10.0;
                        if badge_y + bh > sh - 100.0 {
                            badge_y = (ry - bh - 10.0).max(10.0);
                        }
                        let badge_x = (rx + (rw - bw) * 0.5).clamp(10.0, sw - bw - 10.0);

                        context.set_source_rgba(0.12, 0.12, 0.14, 0.88);
                        draw_rounded_rectangle(context, badge_x, badge_y, bw, bh, 6.0);
                        let _ = context.fill();

                        context.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                        context.move_to(badge_x + pad_x, badge_y + bh - pad_y);
                        let _ = context.show_text(&badge_text);
                    }
                }
            }
        });
    }

    overlay.set_child(Some(&drawing_area));

    // 2. BARRA DE HERRAMIENTAS FLOTANTE INFERIOR
    let toolbar_box = GtkBox::new(Orientation::Horizontal, 6);
    toolbar_box.set_halign(Align::Center);
    toolbar_box.set_valign(Align::End);
    toolbar_box.set_margin_bottom(36);
    toolbar_box.add_css_class("screenshot-toolbar-pill");

    // Botón Cerrar (✕)
    let close_btn = Button::from_icon_name("window-close-symbolic");
    close_btn.set_tooltip_text(Some("Cerrar (Esc)"));
    close_btn.add_css_class("screenshot-pill-btn");
    {
        let window = window.clone();
        let loop_quit = main_loop.clone();
        close_btn.connect_clicked(move |_| {
            window.close();
            loop_quit.quit();
        });
    }
    toolbar_box.append(&close_btn);

    toolbar_box.append(&Separator::new(Orientation::Vertical));

    // Grupo Foto (Pantalla completa, Ventana, Área)
    let photo_group = GtkBox::new(Orientation::Horizontal, 2);
    photo_group.add_css_class("linked");

    let btn_full = ToggleButton::builder()
        .icon_name("video-display-symbolic")
        .tooltip_text("Capturar toda la pantalla")
        .css_classes(["screenshot-pill-btn"])
        .build();

    let btn_win = ToggleButton::builder()
        .icon_name("window-new-symbolic")
        .tooltip_text("Capturar ventana")
        .css_classes(["screenshot-pill-btn"])
        .build();

    let btn_area = ToggleButton::builder()
        .icon_name("edit-select-all-symbolic")
        .tooltip_text("Capturar parte seleccionada")
        .css_classes(["screenshot-pill-btn"])
        .build();

    photo_group.append(&btn_full);
    photo_group.append(&btn_win);
    photo_group.append(&btn_area);
    toolbar_box.append(&photo_group);

    toolbar_box.append(&Separator::new(Orientation::Vertical));

    // Grupo Video (Grabar pantalla completa, Grabar área seleccionada)
    let video_group = GtkBox::new(Orientation::Horizontal, 2);
    video_group.add_css_class("linked");

    let btn_rec_full = ToggleButton::builder()
        .icon_name("camera-video-symbolic")
        .tooltip_text("Grabar toda la pantalla")
        .css_classes(["screenshot-pill-btn"])
        .build();

    let btn_rec_area = ToggleButton::builder()
        .icon_name("media-record-symbolic")
        .tooltip_text("Grabar parte seleccionada")
        .css_classes(["screenshot-pill-btn"])
        .build();

    video_group.append(&btn_rec_full);
    video_group.append(&btn_rec_area);
    toolbar_box.append(&video_group);

    toolbar_box.append(&Separator::new(Orientation::Vertical));

    // Menú Opciones (Popover desplegable)
    let options_btn = MenuButton::new();
    options_btn.set_label("Opciones ▾");
    options_btn.add_css_class("screenshot-pill-btn");

    let popover = Popover::new();
    let popover_vbox = GtkBox::new(Orientation::Vertical, 8);
    popover_vbox.set_margin_top(10);
    popover_vbox.set_margin_bottom(10);
    popover_vbox.set_margin_start(12);
    popover_vbox.set_margin_end(12);

    let save_title = Label::new(Some("GUARDAR EN"));
    save_title.set_halign(Align::Start);
    save_title.add_css_class("dim-label");
    popover_vbox.append(&save_title);

    let radio_screenshots = CheckButton::with_label("Carpeta Capturas de pantalla");
    radio_screenshots.set_active(true);
    let radio_desktop = CheckButton::with_label("Escritorio");
    radio_desktop.set_group(Some(&radio_screenshots));
    let radio_clipboard = CheckButton::with_label("Portapapeles");
    radio_clipboard.set_group(Some(&radio_screenshots));

    popover_vbox.append(&radio_screenshots);
    popover_vbox.append(&radio_desktop);
    popover_vbox.append(&radio_clipboard);

    popover_vbox.append(&Separator::new(Orientation::Horizontal));

    let timer_title = Label::new(Some("TEMPORIZADOR"));
    timer_title.set_halign(Align::Start);
    timer_title.add_css_class("dim-label");
    popover_vbox.append(&timer_title);

    let timer_0 = CheckButton::with_label("Ninguno");
    timer_0.set_active(true);
    let timer_5 = CheckButton::with_label("5 segundos");
    timer_5.set_group(Some(&timer_0));
    let timer_10 = CheckButton::with_label("10 segundos");
    timer_10.set_group(Some(&timer_0));

    popover_vbox.append(&timer_0);
    popover_vbox.append(&timer_5);
    popover_vbox.append(&timer_10);

    popover_vbox.append(&Separator::new(Orientation::Horizontal));

    let audio_chk = CheckButton::with_label("Grabar audio del micrófono");
    popover_vbox.append(&audio_chk);

    popover.set_child(Some(&popover_vbox));
    options_btn.set_popover(Some(&popover));
    toolbar_box.append(&options_btn);

    // Botón de Acción Principal (Capturar / Grabar)
    let action_btn = Button::with_label(if initial_mode.is_video() {
        "Grabar"
    } else {
        "Capturar"
    });
    action_btn.add_css_class("screenshot-action-btn");
    action_btn.add_css_class(if initial_mode.is_video() {
        "record"
    } else {
        "capture"
    });
    toolbar_box.append(&action_btn);

    overlay.add_overlay(&toolbar_box);
    window.set_child(Some(&overlay));

    // Sincronización visual de botones
    let sync_buttons = {
        let btn_full = btn_full.clone();
        let btn_win = btn_win.clone();
        let btn_area = btn_area.clone();
        let btn_rec_full = btn_rec_full.clone();
        let btn_rec_area = btn_rec_area.clone();
        let action_btn = action_btn.clone();
        move |mode: ToolbarMode| {
            btn_full.set_active(mode == ToolbarMode::CaptureFull);
            btn_win.set_active(mode == ToolbarMode::CaptureWindow);
            btn_area.set_active(mode == ToolbarMode::CaptureArea);
            btn_rec_full.set_active(mode == ToolbarMode::RecordFull);
            btn_rec_area.set_active(mode == ToolbarMode::RecordArea);

            if mode.is_video() {
                action_btn.set_label("Grabar");
                action_btn.remove_css_class("capture");
                action_btn.add_css_class("record");
            } else {
                action_btn.set_label("Capturar");
                action_btn.remove_css_class("record");
                action_btn.add_css_class("capture");
            }
        }
    };

    sync_buttons(initial_mode);

    // Conectar cambios de modo
    let connect_mode = |btn: &ToggleButton, target_mode: ToolbarMode| {
        let state = state.clone();
        let area = drawing_area.clone();
        let sync = sync_buttons.clone();
        btn.connect_toggled(move |b| {
            if b.is_active() {
                state.borrow_mut().mode = target_mode;
                sync(target_mode);
                area.queue_draw();
            }
        });
    };

    connect_mode(&btn_full, ToolbarMode::CaptureFull);
    connect_mode(&btn_win, ToolbarMode::CaptureWindow);
    connect_mode(&btn_area, ToolbarMode::CaptureArea);
    connect_mode(&btn_rec_full, ToolbarMode::RecordFull);
    connect_mode(&btn_rec_area, ToolbarMode::RecordArea);

    // GESTOS DE RATÓN PARA REDIMENSIONAR / MOVER ÁREA DE SELECCIÓN
    let motion = EventControllerMotion::new();
    {
        let state = state.clone();
        motion.connect_motion(move |_, x, y| {
            state.borrow_mut().hover_pointer = Some((x, y));
        });
    }
    drawing_area.add_controller(motion);

    let drag = GestureDrag::new();
    {
        let state_begin = state.clone();
        let area_begin = drawing_area.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut st = state_begin.borrow_mut();
            if !st.mode.is_area() {
                return;
            }

            let r = st.rect;
            let handle_hit_dist = 14.0;
            let handles = [
                (DragHandle::NorthWest, r.x, r.y),
                (DragHandle::North, r.x + r.width * 0.5, r.y),
                (DragHandle::NorthEast, r.x + r.width, r.y),
                (DragHandle::East, r.x + r.width, r.y + r.height * 0.5),
                (DragHandle::SouthEast, r.x + r.width, r.y + r.height),
                (DragHandle::South, r.x + r.width * 0.5, r.y + r.height),
                (DragHandle::SouthWest, r.x, r.y + r.height),
                (DragHandle::West, r.x, r.y + r.height * 0.5),
            ];

            let mut matched_handle = DragHandle::None;
            for (h, hx, hy) in handles {
                let dx = x - hx;
                let dy = y - hy;
                if dx * dx + dy * dy <= handle_hit_dist * handle_hit_dist {
                    matched_handle = h;
                    break;
                }
            }

            if matched_handle == DragHandle::None {
                if x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height {
                    matched_handle = DragHandle::MoveBox;
                } else {
                    matched_handle = DragHandle::NewSelection;
                    st.rect = SelectionRect {
                        x,
                        y,
                        width: 1.0,
                        height: 1.0,
                        surface_width: area_begin.width() as f64,
                        surface_height: area_begin.height() as f64,
                    };
                }
            }

            st.active_handle = matched_handle;
            st.drag_start_pointer = (x, y);
            st.drag_start_rect = st.rect;
            area_begin.queue_draw();
        });
    }

    {
        let state_update = state.clone();
        let area_update = drawing_area.clone();
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            let mut st = state_update.borrow_mut();
            if !st.mode.is_area() {
                return;
            }

            let sw = area_update.width().max(1) as f64;
            let sh = area_update.height().max(1) as f64;
            let orig = st.drag_start_rect;

            match st.active_handle {
                DragHandle::MoveBox => {
                    let new_x = (orig.x + offset_x).clamp(0.0, (sw - orig.width).max(0.0));
                    let new_y = (orig.y + offset_y).clamp(0.0, (sh - orig.height).max(0.0));
                    st.rect.x = new_x;
                    st.rect.y = new_y;
                }
                DragHandle::NewSelection => {
                    let start_x = st.drag_start_pointer.0;
                    let start_y = st.drag_start_pointer.1;
                    let cur_x = (start_x + offset_x).clamp(0.0, sw);
                    let cur_y = (start_y + offset_y).clamp(0.0, sh);
                    st.rect = SelectionRect {
                        x: start_x.min(cur_x),
                        y: start_y.min(cur_y),
                        width: (start_x - cur_x).abs().max(4.0),
                        height: (start_y - cur_y).abs().max(4.0),
                        surface_width: sw,
                        surface_height: sh,
                    };
                }
                DragHandle::NorthWest => {
                    let right = orig.x + orig.width;
                    let bottom = orig.y + orig.height;
                    let new_x = (orig.x + offset_x).clamp(0.0, right - 20.0);
                    let new_y = (orig.y + offset_y).clamp(0.0, bottom - 20.0);
                    st.rect = SelectionRect {
                        x: new_x,
                        y: new_y,
                        width: right - new_x,
                        height: bottom - new_y,
                        surface_width: sw,
                        surface_height: sh,
                    };
                }
                DragHandle::North => {
                    let bottom = orig.y + orig.height;
                    let new_y = (orig.y + offset_y).clamp(0.0, bottom - 20.0);
                    st.rect.y = new_y;
                    st.rect.height = bottom - new_y;
                }
                DragHandle::NorthEast => {
                    let bottom = orig.y + orig.height;
                    let new_y = (orig.y + offset_y).clamp(0.0, bottom - 20.0);
                    let new_w = (orig.width + offset_x).clamp(20.0, sw - orig.x);
                    st.rect.y = new_y;
                    st.rect.height = bottom - new_y;
                    st.rect.width = new_w;
                }
                DragHandle::East => {
                    st.rect.width = (orig.width + offset_x).clamp(20.0, sw - orig.x);
                }
                DragHandle::SouthEast => {
                    st.rect.width = (orig.width + offset_x).clamp(20.0, sw - orig.x);
                    st.rect.height = (orig.height + offset_y).clamp(20.0, sh - orig.y);
                }
                DragHandle::South => {
                    st.rect.height = (orig.height + offset_y).clamp(20.0, sh - orig.y);
                }
                DragHandle::SouthWest => {
                    let right = orig.x + orig.width;
                    let new_x = (orig.x + offset_x).clamp(0.0, right - 20.0);
                    st.rect.x = new_x;
                    st.rect.width = right - new_x;
                    st.rect.height = (orig.height + offset_y).clamp(20.0, sh - orig.y);
                }
                DragHandle::West => {
                    let right = orig.x + orig.width;
                    let new_x = (orig.x + offset_x).clamp(0.0, right - 20.0);
                    st.rect.x = new_x;
                    st.rect.width = right - new_x;
                }
                DragHandle::None => {}
            }
            area_update.queue_draw();
        });
    }

    {
        let state_end = state.clone();
        drag.connect_drag_end(move |_, _, _| {
            state_end.borrow_mut().active_handle = DragHandle::None;
        });
    }
    drawing_area.add_controller(drag);

    // CLICK EN BOTÓN DE ACCIÓN (CAPTURAR / GRABAR)
    {
        let state = state.clone();
        let result_holder = result_holder.clone();
        let window = window.clone();
        let loop_quit = main_loop.clone();
        let radio_desktop = radio_desktop.clone();
        let radio_clipboard = radio_clipboard.clone();
        let timer_5 = timer_5.clone();
        let timer_10 = timer_10.clone();
        let audio_chk = audio_chk.clone();
        let area = drawing_area.clone();

        action_btn.connect_clicked(move |_| {
            let st = state.borrow();
            let dest = if radio_clipboard.is_active() {
                SaveDestination::Clipboard
            } else if radio_desktop.is_active() {
                SaveDestination::Desktop
            } else {
                SaveDestination::Screenshots
            };

            let timer_seconds = if timer_10.is_active() {
                10
            } else if timer_5.is_active() {
                5
            } else {
                0
            };

            let options = ToolbarOptions {
                destination: dest,
                timer_seconds,
                record_audio: audio_chk.is_active(),
                show_cursor: true,
            };

            let mut final_rect = st.rect;
            final_rect.surface_width = area.width() as f64;
            final_rect.surface_height = area.height() as f64;

            let rect_opt = if st.mode.is_area() {
                Some(final_rect)
            } else {
                None
            };

            if st.mode.is_video() {
                *result_holder.borrow_mut() = ToolbarResult::Record {
                    mode: st.mode,
                    rect: rect_opt,
                    options,
                };
            } else {
                *result_holder.borrow_mut() = ToolbarResult::Capture {
                    mode: st.mode,
                    rect: rect_opt,
                    options,
                };
            }

            window.close();
            loop_quit.quit();
        });
    }

    // TECLADO (Escape para cancelar, Enter para disparar)
    let keys = EventControllerKey::new();
    {
        let window = window.clone();
        let loop_quit = main_loop.clone();
        let action_btn = action_btn.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                window.close();
                loop_quit.quit();
                glib::Propagation::Stop
            } else if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                action_btn.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(keys);

    let loop_destroy = main_loop.clone();
    window.connect_destroy(move |_| {
        loop_destroy.quit();
    });

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-screenshot-toolbar");
        window.set_monitor(&monitor);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
    } else {
        window.fullscreen();
    }

    window.present();
    main_loop.run();

    let res = result_holder.borrow().clone();
    // Pausa breve para permitir que el compositor desmapee la interfaz antes de capturar el fotograma
    std::thread::sleep(Duration::from_millis(120));
    Ok(res)
}

fn draw_rounded_rectangle(
    context: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
) {
    context.new_sub_path();
    context.arc(x + w - r, y + r, r, -PI * 0.5, 0.0);
    context.arc(x + w - r, y + h - r, r, 0.0, PI * 0.5);
    context.arc(x + r, y + h - r, r, PI * 0.5, PI);
    context.arc(x + r, y + r, r, PI, PI * 1.5);
    context.close_path();
}
