use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Image, Label, Orientation, PolicyType, ScrolledWindow};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct PathBar {
    container: ScrolledWindow,
    hbox: GtkBox,
    on_navigate: Rc<dyn Fn(PathBuf)>,
}

impl PathBar {
    pub fn new<F: Fn(PathBuf) + 'static>(on_navigate: F) -> Self {
        let on_navigate = Rc::new(on_navigate);

        let hbox = GtkBox::new(Orientation::Horizontal, 2);
        hbox.set_valign(Align::Center);

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Never)
            .css_classes(["branches-pathbar"])
            .child(&hbox)
            .build();

        Self {
            container,
            hbox,
            on_navigate,
        }
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.container
    }

    pub fn update_path(&self, path: &Path) {
        // Limpiar crumbs anteriores
        while let Some(child) = self.hbox.first_child() {
            self.hbox.remove(&child);
        }

        // Descomponer ruta en componentes
        let mut ancestors = Vec::new();
        let mut curr = Some(path);
        while let Some(p) = curr {
            ancestors.push(p.to_path_buf());
            curr = p.parent();
        }
        ancestors.reverse();

        for (idx, ancestor) in ancestors.iter().enumerate() {
            if idx > 0 {
                let sep = Label::builder()
                    .label("›")
                    .css_classes(["branches-crumb-separator"])
                    .build();
                self.hbox.append(&sep);
            }

            let name = if ancestor == Path::new("/") {
                "Macintosh HD".to_string()
            } else if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
                if ancestor == &home {
                    "Inicio".to_string()
                } else {
                    ancestor
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "/".to_string())
                }
            } else {
                ancestor
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "/".to_string())
            };

            let btn = Button::builder()
                .css_classes(["branches-crumb-btn"])
                .build();

            let btn_box = GtkBox::new(Orientation::Horizontal, 4);

            if idx == 0 {
                let disk_icon = Image::from_icon_name("drive-harddisk-symbolic");
                disk_icon.set_pixel_size(12);
                btn_box.append(&disk_icon);
            } else if idx == ancestors.len() - 1 {
                let folder_icon = Image::from_icon_name("folder-symbolic");
                folder_icon.set_pixel_size(12);
                btn_box.append(&folder_icon);
            }

            let label = Label::new(Some(&name));
            btn_box.append(&label);

            btn.set_child(Some(&btn_box));

            let target = ancestor.clone();
            let cb = self.on_navigate.clone();
            btn.connect_clicked(move |_| {
                cb(target.clone());
            });

            self.hbox.append(&btn);
        }
    }
}
