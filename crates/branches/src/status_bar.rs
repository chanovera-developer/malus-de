use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, Orientation};

pub struct StatusBar {
    container: GtkBox,
    items_label: Label,
    free_space_label: Label,
}

impl StatusBar {
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 12);
        container.add_css_class("branches-statusbar");

        let items_label = Label::builder()
            .label("0 ítems")
            .halign(Align::Start)
            .hexpand(true)
            .build();

        let free_space_label = Label::builder()
            .label("")
            .halign(Align::End)
            .build();

        container.append(&items_label);
        container.append(&free_space_label);

        Self {
            container,
            items_label,
            free_space_label,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn update(&self, total_items: usize, selected_items: usize, free_space_str: &str) {
        if selected_items > 0 {
            self.items_label.set_label(&format!(
                "{} de {} ítems seleccionados",
                selected_items, total_items
            ));
        } else {
            self.items_label.set_label(&format!("{} ítems", total_items));
        }

        self.free_space_label.set_label(free_space_str);
    }
}
