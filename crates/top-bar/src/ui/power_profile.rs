use gtk4::prelude::*;
use gtk4::{Button, Image};
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;
use tracing::info;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerMode {
    Performance,
    Balanced,
    PowerSaver,
}

impl PowerMode {
    pub fn next(&self) -> Self {
        match self {
            PowerMode::Balanced => PowerMode::Performance,
            PowerMode::Performance => PowerMode::PowerSaver,
            PowerMode::PowerSaver => PowerMode::Balanced,
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            PowerMode::Performance => "power-profile-performance-symbolic",
            PowerMode::Balanced => "power-profile-balanced-symbolic",
            PowerMode::PowerSaver => "power-profile-power-saver-symbolic",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PowerMode::Performance => "Rendimiento",
            PowerMode::Balanced => "Equilibrado",
            PowerMode::PowerSaver => "Ahorro",
        }
    }

    pub fn dbus_name(&self) -> &'static str {
        match self {
            PowerMode::Performance => "performance",
            PowerMode::Balanced => "balanced",
            PowerMode::PowerSaver => "power-saver",
        }
    }

    pub fn tuned_name(&self) -> &'static str {
        match self {
            PowerMode::Performance => "throughput-performance",
            PowerMode::Balanced => "balanced",
            PowerMode::PowerSaver => "powersave",
        }
    }
}

thread_local! {
    static POWER_BTN_REF: RefCell<Option<glib::WeakRef<Button>>> = const { RefCell::new(None) };
}

pub fn refresh_power_profile_visibility() {
    let visible = malus_de_common::config::MenuBarConfig::load().power_profile;
    POWER_BTN_REF.with(|cell| {
        if let Some(btn) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
            btn.set_visible(visible);
        }
    });
}

fn apply_mode_visual(btn: &Button, state: &Cell<PowerMode>, mode: PowerMode) {
    state.set(mode);
    let icon = Image::from_icon_name(mode.icon_name());
    btn.set_child(Some(&icon));
    btn.set_tooltip_text(Some(&format!(
        "Perfil de rendimiento: {} (Clic para alternar)",
        mode.label()
    )));
}

pub fn create_power_profile_widget() -> Button {
    let mode_state = Rc::new(Cell::new(PowerMode::Balanced));

    let btn = Button::builder()
        .css_classes(["power-profile-pill", "topbar-btn"])
        .tooltip_text("Perfil de rendimiento: Equilibrado (Clic para alternar)")
        .has_frame(false)
        .build();
    btn.set_visible(malus_de_common::config::MenuBarConfig::load().power_profile);

    POWER_BTN_REF.with(|cell| {
        *cell.borrow_mut() = Some(btn.downgrade());
    });

    let icon = Image::from_icon_name(PowerMode::Balanced.icon_name());
    btn.set_child(Some(&icon));

    let btn_clone = btn.clone();
    let state_clone = mode_state.clone();

    btn.connect_clicked(move |_| {
        let next_mode = state_clone.get().next();
        apply_mode_visual(&btn_clone, &state_clone, next_mode);
        info!("Cambiando Modo de Rendimiento a: {:?}", next_mode);

        let dbus_name = next_mode.dbus_name();
        let tuned_name = next_mode.tuned_name();
        glib::MainContext::default().spawn_local(async move {
            let mut applied_via_dbus = false;
            if let Ok(connection) = zbus::Connection::system().await {
                if connection
                    .call_method(
                        Some("net.hadess.PowerProfiles"),
                        "/net/hadess/PowerProfiles",
                        Some("org.freedesktop.DBus.Properties"),
                        "Set",
                        &(
                            "net.hadess.PowerProfiles",
                            "ActiveProfile",
                            zbus::zvariant::Value::from(dbus_name),
                        ),
                    )
                    .await
                    .is_ok()
                {
                    applied_via_dbus = true;
                }
            }
            if !applied_via_dbus {
                std::thread::spawn(move || {
                    let _ = Command::new("tuned-adm").args(["profile", tuned_name]).output();
                });
            }
        });
    });

    let btn_init = btn.clone();
    let state_init = mode_state;
    glib::MainContext::default().spawn_local(async move {
        let mut detected: Option<PowerMode> = None;
        if let Ok(connection) = zbus::Connection::system().await {
            if let Ok(reply) = connection
                .call_method(
                    Some("net.hadess.PowerProfiles"),
                    "/net/hadess/PowerProfiles",
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("net.hadess.PowerProfiles", "ActiveProfile"),
                )
                .await
            {
                if let Ok(val) = reply.body().deserialize::<zbus::zvariant::OwnedValue>() {
                    if let Ok(profile_str) = val.try_into() as Result<String, _> {
                        detected = Some(match profile_str.as_str() {
                            "performance" => PowerMode::Performance,
                            "power-saver" => PowerMode::PowerSaver,
                            _ => PowerMode::Balanced,
                        });
                    }
                }
            }
        }
        if detected.is_none() {
            if let Ok(out) = Command::new("tuned-adm").arg("active").output() {
                if out.status.success() {
                    let txt = String::from_utf8_lossy(&out.stdout).to_lowercase();
                    detected = Some(if txt.contains("performance") {
                        PowerMode::Performance
                    } else if txt.contains("powersave") {
                        PowerMode::PowerSaver
                    } else {
                        PowerMode::Balanced
                    });
                }
            }
        }
        if let Some(mode) = detected {
            apply_mode_visual(&btn_init, &state_init, mode);
        }
    });

    btn
}
