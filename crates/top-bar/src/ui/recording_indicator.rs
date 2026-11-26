use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, DrawingArea, Label, Orientation};
use std::cell::{Cell, RefCell};
use std::time::Duration;
use tracing::{info, warn};

thread_local! {
    static RECORDING_BTN: RefCell<Option<glib::WeakRef<Button>>> =
        const { RefCell::new(None) };
    static RECORDING_TIMER_LABEL: RefCell<Option<glib::WeakRef<Label>>> =
        const { RefCell::new(None) };
    static RECORDING_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static RECORDING_ELAPSED: Cell<u64> = const { Cell::new(0) };
    static RECORDING_TIMER_SRC: RefCell<Option<glib::SourceId>> =
        const { RefCell::new(None) };
}

fn format_elapsed(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn is_spanish_locale() -> bool {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es")
}

/// Widget de la barra superior estilo macOS que aparece mientras hay una grabación de pantalla activa.
/// Al pulsarlo detiene inmediatamente la grabación (`IpcMessage::StopScreenRecording`).
pub fn create_recording_indicator_widget() -> Button {
    let es = is_spanish_locale();
    let tooltip = if es {
        "Detener grabación de pantalla (⌃⌘⎋ o ⇧⌘R)"
    } else {
        "Stop Screen Recording (⌃⌘⎋ or ⇧⌘R)"
    };

    let btn = Button::builder()
        .css_classes(["topbar-btn", "recording-stop-pill"])
        .tooltip_text(tooltip)
        .has_frame(false)
        .valign(Align::Center)
        .visible(false)
        .build();

    let hbox = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();

    // Ícono circular rojo con cuadrado blanco de parada estilo macOS Tahoe
    let stop_icon = DrawingArea::builder()
        .content_width(16)
        .content_height(16)
        .valign(Align::Center)
        .build();
    stop_icon.set_draw_func(|_, cr, w, h| {
        let cx = w as f64 / 2.0;
        let cy = h as f64 / 2.0;
        let r = (w.min(h) as f64 / 2.0) - 0.5;
        cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(0.95, 0.24, 0.22, 1.0);
        let _ = cr.fill();

        let sq = 6.0;
        let sx = cx - sq / 2.0;
        let sy = cy - sq / 2.0;
        crate::ui::system_menu::squircle_path(cr, 0.0, 0.0, 0.0);
        let _ = cr.new_path();
        cr.rectangle(sx, sy, sq, sq);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
        let _ = cr.fill();
    });
    hbox.append(&stop_icon);

    let timer_label = Label::builder()
        .label("00:00")
        .css_classes(["recording-timer-label"])
        .valign(Align::Center)
        .build();
    hbox.append(&timer_label);

    btn.set_child(Some(&hbox));

    btn.connect_clicked(|_| {
        info!("⏹ [TopBar] Usuario pulsó detener grabación de pantalla");
        if let Err(err) = malus_de_common::ipc::send_ipc_message(
            &malus_de_common::ipc::IpcMessage::StopScreenRecording,
        ) {
            warn!("No se pudo enviar StopScreenRecording al compositor: {err}");
        }
        update_recording_status(false, 0);
    });

    RECORDING_BTN.with(|cell| {
        *cell.borrow_mut() = Some(btn.downgrade());
    });
    RECORDING_TIMER_LABEL.with(|cell| {
        *cell.borrow_mut() = Some(timer_label.downgrade());
    });

    btn
}

/// Actualiza la visibilidad y contador del botón de detener grabación en la top-bar.
pub fn update_recording_status(is_recording: bool, elapsed_secs: u64) {
    let was_recording = RECORDING_ACTIVE.replace(is_recording);
    RECORDING_ELAPSED.set(elapsed_secs);

    RECORDING_BTN.with(|cell| {
        if let Some(btn) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
            btn.set_visible(is_recording);
        }
    });
    RECORDING_TIMER_LABEL.with(|cell| {
        if let Some(lbl) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
            lbl.set_label(&format_elapsed(elapsed_secs));
        }
    });

    if is_recording && !was_recording {
        RECORDING_TIMER_SRC.with(|slot| {
            if let Some(old) = slot.borrow_mut().take() {
                old.remove();
            }
            let id = glib::timeout_add_local(Duration::from_secs(1), || {
                if !RECORDING_ACTIVE.get() {
                    return glib::ControlFlow::Break;
                }
                let next = RECORDING_ELAPSED.get().saturating_add(1);
                RECORDING_ELAPSED.set(next);
                RECORDING_TIMER_LABEL.with(|cell| {
                    if let Some(lbl) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                        lbl.set_label(&format_elapsed(next));
                    }
                });
                glib::ControlFlow::Continue
            });
            *slot.borrow_mut() = Some(id);
        });
    } else if !is_recording {
        RECORDING_TIMER_SRC.with(|slot| {
            if let Some(old) = slot.borrow_mut().take() {
                old.remove();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::format_elapsed;

    #[test]
    fn formats_recording_timer_mm_ss() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(9), "00:09");
        assert_eq!(format_elapsed(75), "01:15");
        assert_eq!(format_elapsed(3605), "60:05");
    }
}
