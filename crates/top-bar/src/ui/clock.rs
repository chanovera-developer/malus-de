use glib::ControlFlow;
use gtk4::Label;
use std::cell::{Cell, RefCell};
use std::time::Duration;

thread_local! {
    static CLOCK_LABEL: RefCell<Option<glib::WeakRef<Label>>> = const { RefCell::new(None) };
    static BLINK_STATE: Cell<bool> = const { Cell::new(true) };
}

pub fn create_clock_widget() -> Label {
    let mut builder = Label::builder().css_classes(["clock-label", "topbar-btn"]);

    if let Some(cursor) = gtk4::gdk::Cursor::from_name("pointer", None) {
        builder = builder.cursor(&cursor);
    }

    let clock_label = builder.build();
    CLOCK_LABEL.with(|cell| {
        *cell.borrow_mut() = Some(gtk4::prelude::ObjectExt::downgrade(&clock_label));
    });

    update_clock(&clock_label);

    let label_clone = clock_label.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        BLINK_STATE.with(|b| b.set(!b.get()));
        update_clock(&label_clone);
        ControlFlow::Continue
    });

    clock_label
}

pub fn refresh_clock_now() {
    CLOCK_LABEL.with(|cell| {
        if let Some(lbl) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
            update_clock(&lbl);
        }
    });
}

fn update_clock(label: &Label) {
    let cfg = malus_de_common::config::MenuBarConfig::load();
    let blink_on = BLINK_STATE.with(|b| b.get());
    let fmt = cfg.clock_strftime_format(blink_on);
    if let Ok(now) = glib::DateTime::now_local() {
        if let Ok(formatted) = now.format(&fmt) {
            label.set_text(&formatted);
        }
    }
}
