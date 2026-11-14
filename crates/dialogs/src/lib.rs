use gtk4::gdk::Key;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, EventControllerKey,
    Expander, FileDialog, Label, Orientation, PolicyType, ScrolledWindow, TextView, Window,
    WrapMode,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

thread_local! {
    static CONFIRMATION_CSS_LOADED: Cell<bool> = const { Cell::new(false) };
}

const CONFIRMATION_CSS: &str = include_str!("styles/confirmation.css");

fn ensure_dialog_css_loaded() {
    CONFIRMATION_CSS_LOADED.with(|loaded| {
        if loaded.get() {
            return;
        }
        if let Some(display) = gtk4::gdk::Display::default() {
            let provider = CssProvider::new();
            provider.load_from_string(CONFIRMATION_CSS);
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            loaded.set(true);
        }
    });
}

pub fn show_confirmation(
    app: &Application,
    parent: Option<&Window>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    on_confirm: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    ensure_dialog_css_loaded();

    let dialog = ApplicationWindow::builder()
        .application(app)
        .title("Confirmación de acción del sistema")
        .css_classes(["malus-confirmation-window"])
        .modal(true)
        .decorated(false)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .css_classes(["malus-confirmation-card"])
        .build();
    let heading = Label::builder()
        .label(heading)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-heading"])
        .build();
    let body = Label::builder()
        .label(body)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-body"])
        .build();
    card.append(&heading);
    card.append(&body);

    let buttons = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk4::Align::End)
        .build();
    let cancel_button = Button::with_label("Cancelar");
    cancel_button.add_css_class("malus-confirmation-cancel");
    let confirm_button = Button::with_label(confirm_label);
    confirm_button.add_css_class("destructive-action");
    buttons.append(&cancel_button);
    buttons.append(&confirm_button);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let on_cancel = Rc::new(RefCell::new(Some(on_cancel)));
    dialog.connect_close_request({
        let on_cancel = on_cancel.clone();
        move |_| {
            if let Some(callback) = on_cancel.borrow_mut().take() {
                callback();
            }
            glib::Propagation::Proceed
        }
    });

    let dialog_on_cancel = dialog.clone();
    cancel_button.connect_clicked(move |_| dialog_on_cancel.close());

    let dialog_on_confirm = dialog.clone();
    let on_cancel_confirm = on_cancel;
    let on_confirm = RefCell::new(Some(on_confirm));
    confirm_button.connect_clicked(move |_| {
        on_cancel_confirm.borrow_mut().take();
        if let Some(callback) = on_confirm.borrow_mut().take() {
            callback();
        }
        dialog_on_confirm.close();
    });

    let key_controller = EventControllerKey::new();
    let dialog_on_escape = dialog.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == Key::Escape {
            dialog_on_escape.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    dialog.add_controller(key_controller);
    dialog.present();
    cancel_button.grab_focus();
}

/// Presenta un diálogo compacto estilo macOS tras un cierre inesperado, con botón directo
/// para enviar el reporte a `malus-de@chano.dev` y detalles técnicos desplegables opcionales.
pub fn show_crash_report_dialog(
    app: &Application,
    report_path: PathBuf,
    report_text: String,
    _extra_archived_count: usize,
) {
    ensure_dialog_css_loaded();

    let component = extract_report_field(&report_text, "Componente:")
        .unwrap_or_else(|| "Malus DE".to_string());
    let summary = extract_report_summary(&report_text);

    let dialog = ApplicationWindow::builder()
        .application(app)
        .title("Informe de error — Malus DE")
        .css_classes(["malus-confirmation-window"])
        .decorated(false)
        .resizable(false)
        .build();

    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .css_classes(["malus-confirmation-card", "malus-crash-card"])
        .build();

    let heading = Label::builder()
        .label(format!("{component} se cerró inesperadamente"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-heading"])
        .build();

    let body = Label::builder()
        .label("¿Quieres enviar el informe de error para ayudar a solucionar este problema?")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["malus-confirmation-body"])
        .build();

    let text_view = TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(WrapMode::WordChar)
        .css_classes(["malus-crash-text"])
        .build();
    text_view.buffer().set_text(&report_text);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .vexpand(true)
        .css_classes(["malus-crash-scroll"])
        .child(&text_view)
        .build();

    let details_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .build();
    let save_btn = Button::with_label("Guardar .txt...");
    save_btn.add_css_class("malus-confirmation-cancel");
    save_btn.set_halign(gtk4::Align::Start);
    details_box.append(&scroll);
    details_box.append(&save_btn);

    let details_expander = Expander::builder()
        .label("Ver detalles")
        .expanded(false)
        .css_classes(["malus-crash-expander"])
        .child(&details_box)
        .build();

    let status_label = Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["malus-crash-status"])
        .build();

    let buttons = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk4::Align::End)
        .build();

    let close_btn = Button::with_label("Cerrar");
    close_btn.add_css_class("malus-confirmation-cancel");

    let send_btn = Button::with_label("Enviar reporte");
    send_btn.add_css_class("malus-crash-primary");

    buttons.append(&close_btn);
    buttons.append(&send_btn);

    card.append(&heading);
    card.append(&body);
    card.append(&details_expander);
    card.append(&status_label);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let dialog_close = dialog.clone();
    close_btn.connect_clicked(move |_| dialog_close.close());

    let report_for_save = report_text.clone();
    let default_name = report_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("malus-crash-report.txt")
        .to_string();
    let dialog_for_save = dialog.clone();
    let status_for_save = status_label.clone();
    save_btn.connect_clicked(move |_| {
        let chooser = FileDialog::builder()
            .title("Guardar informe de error (.txt)")
            .initial_name(&default_name)
            .build();
        if let Some(folder) = default_export_folder() {
            chooser.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        let text_to_write = report_for_save.clone();
        let status_done = status_for_save.clone();
        chooser.save(Some(&dialog_for_save), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(dest_path) = file.path() {
                    status_done.set_visible(true);
                    match std::fs::write(&dest_path, text_to_write.as_bytes()) {
                        Ok(()) => status_done.set_text(&format!(
                            "✓ Guardado en {}",
                            dest_path.display()
                        )),
                        Err(err) => status_done.set_text(&format!(
                            "No se pudo guardar: {err}"
                        )),
                    }
                }
            }
        });
    });

    let report_for_send = report_text;
    let path_for_send = report_path;
    let component_for_send = component;
    let summary_for_send = summary;
    let status_for_send = status_label;
    let send_btn_self = send_btn.clone();
    let expander_on_error = details_expander;
    send_btn.connect_clicked(move |_| {
        send_btn_self.set_sensitive(false);
        send_btn_self.set_label("Enviando...");
        status_for_send.set_visible(true);
        status_for_send.set_text("Enviando reporte...");

        let endpoint = malus_de_common::crash::crash_report_endpoint();
        let component_bg = component_for_send.clone();
        let summary_bg = summary_for_send.clone();
        let path_bg = path_for_send.clone();
        let report_bg = report_for_send.clone();

        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let result = malus_de_common::crash::send_report_via_endpoint(
                &endpoint,
                &component_bg,
                &summary_bg,
                &path_bg,
                &report_bg,
            );
            let _ = tx.send(result);
        });

        let btn_ui = send_btn_self.clone();
        let status_ui = status_for_send.clone();
        let exp_ui = expander_on_error.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(ok_msg)) => {
                    btn_ui.set_label("Enviado ✓");
                    btn_ui.set_sensitive(false);
                    status_ui.set_text(&format!("✓ {ok_msg}"));
                    glib::ControlFlow::Break
                }
                Ok(Err(err_msg)) => {
                    btn_ui.set_label("Reintentar");
                    btn_ui.set_sensitive(true);
                    exp_ui.set_expanded(true);
                    status_ui.set_text(&format!(
                        "No se pudo enviar ({err_msg}). Puedes guardar el .txt abajo."
                    ));
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    btn_ui.set_label("Reintentar");
                    btn_ui.set_sensitive(true);
                    exp_ui.set_expanded(true);
                    status_ui.set_text("Error al enviar. Puedes guardar el .txt abajo.");
                    glib::ControlFlow::Break
                }
            }
        });
    });

    let key_controller = EventControllerKey::new();
    let dialog_on_escape = dialog.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == Key::Escape {
            dialog_on_escape.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    dialog.add_controller(key_controller);
    dialog.present();
    send_btn.grab_focus();
}

fn default_export_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    for candidate in ["Escritorio", "Desktop", "Descargas", "Downloads"] {
        let p = home.join(candidate);
        if p.is_dir() {
            return Some(p);
        }
    }
    Some(home)
}

fn extract_report_field(report: &str, prefix: &str) -> Option<String> {
    for line in report.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_report_summary(report: &str) -> String {
    let mut after_marker = false;
    for line in report.lines() {
        if line.contains("--- MENSAJE DEL ERROR ---") {
            after_marker = true;
            continue;
        }
        if after_marker {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(160).collect();
            }
        }
    }
    extract_report_field(report, "Ubicación:")
        .or_else(|| extract_report_field(report, "Señal / Código:"))
        .unwrap_or_else(|| "Cierre inesperado".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_report_field_and_summary_parse_dump_text() {
        let dump = "\
Componente:  malus-de-compositor\n\
Ubicación:   state.rs:10:5\n\
\n\
--- MENSAJE DEL ERROR ---\n\
assertion failed: something went wrong\n";
        assert_eq!(
            extract_report_field(dump, "Componente:"),
            Some("malus-de-compositor".to_string())
        );
        assert_eq!(
            extract_report_summary(dump),
            "assertion failed: something went wrong"
        );
    }
}
