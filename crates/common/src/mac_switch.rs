//! Toggle personalizado estilo macOS Tahoe (`MacSwitch`).
//!
//! GTK `GtkSwitch` fuerza un ancho mínimo de `2 * ancho_del_slider`, lo que
//! impide mantener una perilla ancha (~65% del track) dentro de un toggle
//! compacto (~40px). Este widget usa un contenedor `Box` vertical (`mac-switch`)
//! con un hijo `Box` (`mac-switch-knob`) alineado a inicio/fin y controlado por
//! `StateFlags::CHECKED`, sin animación de arrastre.

use gtk4::gdk::Key;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{AccessibleRole, Align, Box as GtkBox, EventControllerKey, GestureClick, Orientation, StateFlags};
use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;

type StateSetCallback = Rc<dyn Fn(&MacSwitch, bool) -> glib::Propagation>;

/// Switch estilo macOS Tahoe con píldora externa de ~40×19 px y perilla blanca de 26×15 px.
#[derive(Clone)]
pub struct MacSwitch {
    root: GtkBox,
    knob: GtkBox,
    active: Rc<Cell<bool>>,
    on_state_set: Rc<RefCell<Option<StateSetCallback>>>,
}

impl Deref for MacSwitch {
    type Target = GtkBox;

    fn deref(&self) -> &Self::Target {
        &self.root
    }
}

impl MacSwitch {
    /// Crea un nuevo `MacSwitch` en el estado inicial indicado.
    pub fn new(active: bool) -> Self {
        let root = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .hexpand(false)
            .vexpand(false)
            .valign(Align::Center)
            .focusable(true)
            .accessible_role(AccessibleRole::Switch)
            .css_classes(["mac-switch"])
            .build();

        let knob = GtkBox::builder()
            .hexpand(false)
            .vexpand(false)
            .valign(Align::Center)
            .halign(if active { Align::End } else { Align::Start })
            .can_target(false)
            .css_classes(["mac-switch-knob"])
            .build();

        root.append(&knob);

        let sw = Self {
            root,
            knob,
            active: Rc::new(Cell::new(active)),
            on_state_set: Rc::new(RefCell::new(None)),
        };
        sw.apply_visual(active);
        sw.install_controllers();
        sw
    }

    /// Constructor estilo builder compatible con el patrón de `gtk4::Switch::builder()`.
    pub fn builder() -> MacSwitchBuilder {
        MacSwitchBuilder::default()
    }

    /// Devuelve el widget GTK raíz para añadirlo a contenedores.
    pub fn widget(&self) -> &GtkBox {
        &self.root
    }

    /// Indica si el toggle está activado.
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// Cambia el estado visual sin disparar `connect_state_set` (seguro para sincronización).
    pub fn set_active(&self, active: bool) {
        self.apply_visual(active);
    }

    /// Alias de `set_active` para compatibilidad directa con código que usaba `Switch::set_state`.
    pub fn set_state(&self, active: bool) {
        self.apply_visual(active);
    }

    /// Registra un manejador invocado cuando el usuario alterna el toggle.
    pub fn connect_state_set<F>(&self, f: F)
    where
        F: Fn(&MacSwitch, bool) -> glib::Propagation + 'static,
    {
        *self.on_state_set.borrow_mut() = Some(Rc::new(f));
    }

    fn apply_visual(&self, active: bool) {
        self.active.set(active);
        if active {
            self.root.set_state_flags(StateFlags::CHECKED, false);
            self.knob.set_halign(Align::End);
        } else {
            self.root.unset_state_flags(StateFlags::CHECKED);
            self.knob.set_halign(Align::Start);
        }
    }

    fn user_toggle(&self) {
        if !self.root.is_sensitive() {
            return;
        }
        let next = !self.active.get();
        self.apply_visual(next);
        let cb = self.on_state_set.borrow().clone();
        if let Some(cb) = cb {
            if cb(self, next) == glib::Propagation::Stop {
                self.apply_visual(!next);
            }
        }
    }

    fn install_controllers(&self) {
        let click = GestureClick::new();
        click.set_button(1);

        let root_p = self.root.clone();
        click.connect_pressed(move |_, _, _, _| {
            if root_p.is_sensitive() {
                root_p.set_state_flags(StateFlags::ACTIVE, false);
            }
        });

        let root_c = self.root.clone();
        click.connect_cancel(move |_, _| {
            root_c.unset_state_flags(StateFlags::ACTIVE);
        });

        let sw_r = self.clone();
        click.connect_released(move |_, _, x, y| {
            sw_r.root.unset_state_flags(StateFlags::ACTIVE);
            let w = sw_r.root.width() as f64;
            let h = sw_r.root.height() as f64;
            if x >= 0.0 && y >= 0.0 && x <= w && y <= h {
                sw_r.user_toggle();
            }
        });
        self.root.add_controller(click);

        let key = EventControllerKey::new();
        let sw_k = self.clone();
        key.connect_key_pressed(move |_, keyval, _, _| {
            if matches!(keyval, Key::space | Key::Return | Key::KP_Enter) {
                sw_k.user_toggle();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.root.add_controller(key);
    }
}

/// Builder ergonómico para `MacSwitch`.
#[derive(Default)]
pub struct MacSwitchBuilder {
    active: bool,
    valign: Option<Align>,
    halign: Option<Align>,
    sensitive: Option<bool>,
    tooltip_text: Option<String>,
    margin_end: Option<i32>,
    extra_classes: Vec<String>,
}

impl MacSwitchBuilder {
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn valign(mut self, valign: Align) -> Self {
        self.valign = Some(valign);
        self
    }

    pub fn halign(mut self, halign: Align) -> Self {
        self.halign = Some(halign);
        self
    }

    pub fn sensitive(mut self, sensitive: bool) -> Self {
        self.sensitive = Some(sensitive);
        self
    }

    pub fn tooltip_text(mut self, text: &str) -> Self {
        self.tooltip_text = Some(text.to_string());
        self
    }

    pub fn margin_end(mut self, margin: i32) -> Self {
        self.margin_end = Some(margin);
        self
    }

    pub fn css_classes<'a, I>(mut self, classes: I) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        for cls in classes {
            if cls != "macos-switch" && cls != "mac-switch" {
                self.extra_classes.push(cls.to_string());
            }
        }
        self
    }

    pub fn build(self) -> MacSwitch {
        let sw = MacSwitch::new(self.active);
        if let Some(v) = self.valign {
            sw.root.set_valign(v);
        }
        if let Some(h) = self.halign {
            sw.root.set_halign(h);
        }
        if let Some(s) = self.sensitive {
            sw.root.set_sensitive(s);
        }
        if let Some(ref t) = self.tooltip_text {
            sw.root.set_tooltip_text(Some(t));
        }
        if let Some(m) = self.margin_end {
            sw.root.set_margin_end(m);
        }
        for cls in self.extra_classes {
            sw.root.add_css_class(&cls);
        }
        sw
    }
}

