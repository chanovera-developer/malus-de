use gio::Settings;
use gtk4::prelude::*;
use gtk4::Popover;
use std::cell::RefCell;

thread_local! {
    static REGISTERED_POPOVERS: RefCell<Vec<glib::WeakRef<Popover>>> = RefCell::new(Vec::new());
    static THEME_SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

pub fn is_dark_mode_enabled() -> bool {
    Settings::new("org.gnome.desktop.interface")
        .string("color-scheme")
        .contains("prefer-dark")
}

pub fn ensure_theme_monitor() {
    THEME_SETTINGS.with(|current| {
        if current.borrow().is_some() {
            return;
        }

        let settings = Settings::new("org.gnome.desktop.interface");
        settings.connect_changed(Some("color-scheme"), |settings, _| {
            update_all_popovers_theme(settings.string("color-scheme").contains("prefer-dark"));
        });
        current.replace(Some(settings));
    });
}

pub fn apply_theme_to_popover(popover: &Popover, is_dark: bool) {
    if is_dark {
        popover.remove_css_class("light-popover");
    } else {
        popover.add_css_class("light-popover");
    }
    update_menu_text_theme(popover.upcast_ref(), is_dark);
    popover.queue_draw();
}

fn update_menu_text_theme(widget: &gtk4::Widget, is_dark: bool) {
    if widget.has_css_class("menu-row-label") {
        widget.remove_css_class("menu-light-text");
        widget.remove_css_class("menu-dark-text");
        widget.add_css_class(if is_dark {
            "menu-dark-text"
        } else {
            "menu-light-text"
        });
    } else if widget.has_css_class("menu-shortcut-hint") {
        widget.remove_css_class("menu-light-text");
        widget.remove_css_class("menu-dark-text");
        widget.add_css_class(if is_dark {
            "menu-dark-text"
        } else {
            "menu-light-text"
        });
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        update_menu_text_theme(&current, is_dark);
    }
}

pub fn register_popover(popover: &Popover) {
    ensure_theme_monitor();

    let is_dark = is_dark_mode_enabled();
    apply_theme_to_popover(popover, is_dark);

    let pop_weak = popover.downgrade();
    popover.connect_show(move |_| {
        if let Some(pop) = pop_weak.upgrade() {
            apply_theme_to_popover(&pop, is_dark_mode_enabled());
        }
    });

    REGISTERED_POPOVERS.with(|list| {
        let mut l = list.borrow_mut();
        l.retain(|w| w.upgrade().is_some());
        l.push(popover.downgrade());
    });
}

pub fn update_all_popovers_theme(is_dark: bool) {
    REGISTERED_POPOVERS.with(|list| {
        let mut l = list.borrow_mut();
        l.retain(|w| {
            if let Some(pop) = w.upgrade() {
                apply_theme_to_popover(&pop, is_dark);
                true
            } else {
                false
            }
        });
    });
}
