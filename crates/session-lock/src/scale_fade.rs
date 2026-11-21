//! Contenedor GTK4 ligero que aplica escala centrada (`gsk::Transform`) y opacidad
//! aceleradas por GPU mediante `WidgetImpl::snapshot`.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::Cell;

mod imp {
    use super::*;

    pub struct ScaleFadeBox {
        pub scale: Cell<f32>,
        pub opacity: Cell<f64>,
    }

    impl Default for ScaleFadeBox {
        fn default() -> Self {
            Self {
                scale: Cell::new(1.0),
                opacity: Cell::new(1.0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ScaleFadeBox {
        const NAME: &'static str = "MalusLockScaleFadeBox";
        type Type = super::ScaleFadeBox;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for ScaleFadeBox {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ScaleFadeBox {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let obj = self.obj();
            let alpha = self.opacity.get().clamp(0.0, 1.0);
            if alpha <= 0.001 {
                return;
            }
            let scale = self.scale.get().max(0.05);
            let w = obj.width() as f32;
            let h = obj.height() as f32;

            if alpha < 0.999 {
                snapshot.push_opacity(alpha);
            }
            if (scale - 1.0).abs() > 0.0005 && w > 0.0 && h > 0.0 {
                snapshot.save();
                snapshot.translate(&gtk4::graphene::Point::new(w * 0.5, h * 0.5));
                snapshot.scale(scale, scale);
                snapshot.translate(&gtk4::graphene::Point::new(-w * 0.5, -h * 0.5));
                if let Some(child) = obj.first_child() {
                    obj.snapshot_child(&child, snapshot);
                }
                snapshot.restore();
            } else if let Some(child) = obj.first_child() {
                obj.snapshot_child(&child, snapshot);
            }
            if alpha < 0.999 {
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct ScaleFadeBox(ObjectSubclass<imp::ScaleFadeBox>)
        @extends gtk4::Widget;
}

impl ScaleFadeBox {
    pub fn new() -> Self {
        let obj: Self = glib::Object::new();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj.set_overflow(gtk4::Overflow::Hidden);
        obj
    }

    pub fn set_child(&self, child: &impl IsA<gtk4::Widget>) {
        while let Some(prev) = self.first_child() {
            prev.unparent();
        }
        child.set_parent(self);
    }

    pub fn set_scale_and_opacity(&self, scale: f32, opacity: f64) {
        self.imp().scale.set(scale);
        self.imp().opacity.set(opacity.clamp(0.0, 1.0));
        self.queue_draw();
    }
}
