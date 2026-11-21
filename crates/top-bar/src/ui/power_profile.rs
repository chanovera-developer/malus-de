use gtk4::prelude::*;
use gtk4::{Button, Image};
use std::cell::Cell;
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
}

pub fn create_power_profile_widget() -> Button {
    let mode_state = Rc::new(Cell::new(PowerMode::Balanced));

    let btn = Button::builder()
        .css_classes(["power-profile-pill", "topbar-btn"])
        .tooltip_text("Modo de Rendimiento (power-profiles-daemon)")
        .has_frame(false)
        .build();

    let icon = Image::from_icon_name(PowerMode::Balanced.icon_name());
    btn.set_child(Some(&icon));

    let btn_clone = btn.clone();
    let state_clone = mode_state.clone();

    btn.connect_clicked(move |_| {
        let current = state_clone.get();
        let next_mode = current.next();
        state_clone.set(next_mode);

        let new_icon = Image::from_icon_name(next_mode.icon_name());
        btn_clone.set_child(Some(&new_icon));
        btn_clone.set_tooltip_text(Some(&format!(
            "Modo: {} (Click para alternar)",
            next_mode.label()
        )));

        info!("Cambiando Modo de Rendimiento a: {:?}", next_mode);

        // Notificar asíncronamente a power-profiles-daemon si está disponible
        let mode_name = next_mode.dbus_name();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(connection) = zbus::Connection::system().await {
                let _ = connection
                    .call_method(
                        Some("net.hadess.PowerProfiles"),
                        "/net/hadess/PowerProfiles",
                        Some("org.freedesktop.DBus.Properties"),
                        "Set",
                        &(
                            "net.hadess.PowerProfiles",
                            "ActiveProfile",
                            zbus::zvariant::Value::from(mode_name),
                        ),
                    )
                    .await;
            }
        });
    });

    // Consultar el perfil inicial de power-profiles-daemon
    let btn_init = btn.clone();
    let state_init = mode_state.clone();
    glib::MainContext::default().spawn_local(async move {
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
                        let detected_mode = match profile_str.as_str() {
                            "performance" => PowerMode::Performance,
                            "power-saver" => PowerMode::PowerSaver,
                            _ => PowerMode::Balanced,
                        };
                        state_init.set(detected_mode);
                        let img = Image::from_icon_name(detected_mode.icon_name());
                        btn_init.set_child(Some(&img));
                        btn_init
                            .set_tooltip_text(Some(&format!("Modo: {}", detected_mode.label())));
                    }
                }
            }
        }
    });

    btn
}
