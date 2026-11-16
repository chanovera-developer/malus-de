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

/// Construye el `ContentProvider` de arrastre exponiendo tanto `GdkFileList` como `text/uri-list`
/// para garantizar interoperabilidad Wayland con otras ventanas (como el Dock).
pub(super) fn build_file_drag_content_provider(path: &std::path::Path) -> gtk4::gdk::ContentProvider {
    use gtk4::prelude::*;

    let file = gio::File::for_path(path);
    let uri_payload = format!("{}\r\n", file.uri());
    let file_list = gtk4::gdk::FileList::from_array(&[file]);
    gtk4::gdk::ContentProvider::new_union(&[
        gtk4::gdk::ContentProvider::for_value(&file_list.to_value()),
        gtk4::gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &glib::Bytes::from_owned(uri_payload),
        ),
    ])
}
