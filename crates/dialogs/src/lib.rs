use gtk4::gdk::Key;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, EventControllerKey, Label,
    Orientation, Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static CONFIRMATION_CSS_LOADED: Cell<bool> = const { Cell::new(false) };
}

const CONFIRMATION_CSS: &str = include_str!("styles/confirmation.css");

pub fn show_confirmation(
    app: &Application,
    parent: Option<&Window>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    on_confirm: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
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
