pub mod column_view;
pub mod gallery_view;
pub mod icon_view;
pub mod list_view;

use std::path::PathBuf;

pub type NavigateCallback = std::rc::Rc<dyn Fn(PathBuf)>;
pub type SelectionCallback = std::rc::Rc<dyn Fn(usize)>;

/// Crea un botón de menú contextual estilizado como ítem unificado de Malus DE.
pub(super) fn make_context_menu_button(label: &str) -> gtk4::Button {
    use gtk4::prelude::*;

    let btn = gtk4::Button::builder()
        .label(label)
        .has_frame(false)
        .css_classes(["popover-menu-item"])
        .build();
    if let Some(child) = btn.child() {
        if let Some(lbl) = child.downcast_ref::<gtk4::Label>() {
            lbl.set_xalign(0.0);
        }
    }
    btn
}
