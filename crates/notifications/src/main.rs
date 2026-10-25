use anyhow::Result;
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Image, Label, Orientation,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::logging::init_logging;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tracing::info;
use zbus::zvariant::Value;

static NOTIFICATION_COUNTER: AtomicU32 = AtomicU32::new(1);

const BANNER_CSS: &str = r#"
window.malus-notification-window {
    background-color: transparent;
}

.notification-banner {
    background-color: rgba(28, 28, 32, 0.85);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 16px;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.50);
    padding: 12px 14px;
    min-width: 320px;
}

.notif-header {
    margin-bottom: 6px;
}

.notif-app-name {
    font-size: 12px;
    font-weight: 600;
    color: #a1a1aa;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.notif-time {
    font-size: 11px;
    color: #71717a;
}

.notif-title {
    font-size: 14px;
    font-weight: 700;
    color: #ffffff;
    margin-bottom: 2px;
}

.notif-body {
    font-size: 13px;
    color: #d4d4d8;
    line-height: 1.3;
}

.notif-close-btn {
    background: transparent;
    border: none;
    border-radius: 10px;
    padding: 2px 6px;
    color: #71717a;
}

.notif-close-btn:hover {
    background-color: rgba(255, 255, 255, 0.12);
    color: #ffffff;
}
"#;

pub struct NotificationServer {
    sender: async_channel::Sender<NotificationMsg>,
}

#[derive(Debug, Clone)]
pub struct NotificationMsg {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub sound: bool,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    async fn notify(
        &mut self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, Value<'_>>,
        _expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            NOTIFICATION_COUNTER.fetch_add(1, Ordering::SeqCst)
        };

        info!(
            "Notificación recibida [id={}] de '{}': '{}' - '{}'",
            id, app_name, summary, body
        );

        let msg = NotificationMsg {
            id,
            app_name,
            app_icon,
            summary,
            body,
            sound: _hints.contains_key("sound-name"),
        };

        let _ = self.sender.send(msg).await;
        id
    }

    async fn close_notification(&mut self, id: u32) {
        info!("Cierre de notificación solicitado para id={}", id);
    }

    async fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["body", "actions", "icon-static", "persistence", "sound"]
    }

    async fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("Malus Notification Daemon", "Malus DE", "0.1.0", "1.2")
    }
}

fn main() -> Result<()> {
    init_logging("malus_de_notifications");
    info!("Iniciando Malus DE Notification Daemon (D-Bus / GTK4 Layer Shell)...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let (sender, receiver) = async_channel::unbounded::<NotificationMsg>();

    // Iniciar servidor D-Bus en hilo de fondo usando zbus::block_on
    let s_clone = sender.clone();
    std::thread::spawn(move || {
        let rt = zbus::block_on(async {
            let _connection = zbus::connection::Builder::session()?
                .name("org.freedesktop.Notifications")?
                .serve_at("/org/freedesktop/Notifications", NotificationServer { sender: s_clone })?
                .build()
                .await?;

            info!("Servicio D-Bus org.freedesktop.Notifications registrado con éxito.");
            std::future::pending::<()>().await;
            Ok::<(), zbus::Error>(())
        });
        if let Err(e) = rt {
            tracing::warn!("Aviso al enlazar D-Bus Notifications: {:?}", e);
        }
    });

    let app = Application::builder()
        .application_id("org.malus.de.Notifications")
        .build();

    app.connect_startup(|_| {
        let provider = CssProvider::new();
        provider.load_from_string(BANNER_CSS);
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    app.connect_activate(|_| {});

    let app_handle = app.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(msg) = receiver.recv().await {
            display_banner(&app_handle, msg);
        }
    });

    app.run_with_args(&["malus-de-notifications"]);
    Ok(())
}

fn display_banner(app: &Application, msg: NotificationMsg) {
    if msg.sound {
        if let Some(display) = gtk4::gdk::Display::default() {
            display.beep();
        } else {
            tracing::warn!(
                "No se pudo emitir el sonido de notificación: no hay pantalla GTK disponible"
            );
        }
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Notificación")
        .css_classes(["malus-notification-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-notification");
        // Anclar a esquina superior derecha justo debajo del Top Bar
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, 42);
        window.set_margin(Edge::Right, 16);
    } else {
        window.set_decorated(false);
    }

    let banner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["notification-banner"])
        .spacing(4)
        .build();

    // Cabecera: Icono + Nombre de App + Tiempo + Botón Cerrar
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["notif-header"])
        .spacing(6)
        .build();

    let icon_name = if msg.app_icon.is_empty() {
        "dialog-information-symbolic"
    } else {
        &msg.app_icon
    };
    let app_icon = Image::from_icon_name(icon_name);
    header.append(&app_icon);

    let app_name = Label::builder()
        .label(&msg.app_name)
        .css_classes(["notif-app-name"])
        .build();
    header.append(&app_name);

    let spacer = GtkBox::builder().orientation(Orientation::Horizontal).hexpand(true).build();
    header.append(&spacer);

    let time_label = Label::builder()
        .label("ahora")
        .css_classes(["notif-time"])
        .build();
    header.append(&time_label);

    let close_btn = Button::builder()
        .label("✕")
        .css_classes(["notif-close-btn"])
        .build();
    let win_close = window.clone();
    close_btn.connect_clicked(move |_| {
        win_close.close();
    });
    header.append(&close_btn);
    banner.append(&header);

    // Contenido: Título / Resumen
    let title = Label::builder()
        .label(&msg.summary)
        .css_classes(["notif-title"])
        .xalign(0.0)
        .build();
    banner.append(&title);

    // Cuerpo del mensaje
    if !msg.body.is_empty() {
        let body = Label::builder()
            .label(&msg.body)
            .css_classes(["notif-body"])
            .wrap(true)
            .xalign(0.0)
            .build();
        banner.append(&body);
    }

    window.set_child(Some(&banner));
    window.present();

    // Auto-desvanecer en 6 segundos
    let win_auto = window.clone();
    glib::timeout_add_local(Duration::from_secs(6), move || {
        win_auto.close();
        ControlFlow::Break
    });
}
