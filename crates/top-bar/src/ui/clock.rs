use glib::ControlFlow;
use gtk4::Label;
use std::cell::{Cell, RefCell};
use std::time::Duration;

thread_local! {
    static CLOCK_LABEL: RefCell<Option<glib::WeakRef<Label>>> = const { RefCell::new(None) };
    static BLINK_STATE: Cell<bool> = const { Cell::new(true) };
    static CACHED_MENUBAR_CFG: RefCell<Option<malus_de_common::config::MenuBarConfig>> = const { RefCell::new(None) };
    static CLOCK_TIMER_ID: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    static LAST_CLOCK_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn cached_menubar_config() -> malus_de_common::config::MenuBarConfig {
    CACHED_MENUBAR_CFG.with(|cell| {
        if let Some(cfg) = cell.borrow().as_ref() {
            return cfg.clone();
        }
        let loaded = malus_de_common::config::MenuBarConfig::load();
        *cell.borrow_mut() = Some(loaded.clone());
        loaded
    })
}

fn schedule_next_clock_tick() {
    CLOCK_TIMER_ID.with(|cell| {
        if let Some(old_id) = cell.borrow_mut().take() {
            old_id.remove();
        }
    });

    let cfg = cached_menubar_config();
    let needs_1s = cfg.clock_show_seconds || cfg.clock_flash_separators;
    let delay_secs = if needs_1s {
        1u64
    } else {
        glib::DateTime::now_local()
            .map(|now| (60 - now.second().clamp(0, 59)) as u64)
            .unwrap_or(60)
            .max(1)
    };

    let source_id = glib::timeout_add_local_once(Duration::from_secs(delay_secs), move || {
        CLOCK_TIMER_ID.with(|cell| {
            let _ = cell.borrow_mut().take();
        });
        BLINK_STATE.with(|b| b.set(!b.get()));
        let still_alive = CLOCK_LABEL.with(|cell| {
            if let Some(lbl) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                update_clock(&lbl);
                true
            } else {
                false
            }
        });
        if still_alive {
            schedule_next_clock_tick();
        }
    });

    CLOCK_TIMER_ID.with(|cell| {
        *cell.borrow_mut() = Some(source_id);
    });
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
    schedule_next_clock_tick();

    clock_label
}

pub fn refresh_clock_now() {
    CACHED_MENUBAR_CFG.with(|cell| {
        *cell.borrow_mut() = Some(malus_de_common::config::MenuBarConfig::load());
    });
    let has_label = CLOCK_LABEL.with(|cell| {
        if let Some(lbl) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
            update_clock(&lbl);
            true
        } else {
            false
        }
    });
    if has_label {
        schedule_next_clock_tick();
    }
}

fn update_clock(label: &Label) {
    let cfg = cached_menubar_config();
    let blink_on = BLINK_STATE.with(|b| b.get());
    let fmt = cfg.clock_strftime_format(blink_on);
    if let Ok(now) = glib::DateTime::now_local() {
        if let Ok(formatted) = now.format(&fmt) {
            let text = formatted.as_str();
            let changed = LAST_CLOCK_TEXT.with(|last| {
                let mut l = last.borrow_mut();
                if l.as_str() != text {
                    l.clear();
                    l.push_str(text);
                    true
                } else {
                    false
                }
            });
            if changed {
                label.set_text(text);
            }
        }
    }
}

#[allow(dead_code)]
fn _keep_control_flow(_: ControlFlow) {}
