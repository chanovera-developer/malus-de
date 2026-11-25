use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, DropDown, Entry, Image, Label, Orientation,
    PolicyType, Popover, ScrolledWindow, Stack, StackTransitionType, StringList, Window,
};
use malus_de_common::config::PrintersConfig;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const PRINTERS_PANEL_ID: &str = "printers";
pub const PRINTERS_PANEL_TITLE: &str = "Impresoras y escáneres";

/// Tamaños de papel estándar: `(clave_cups, etiqueta_es)`.
const PAPER_SIZES: &[(&str, &str)] = &[
    ("Letter", "Carta (EE.UU.)"),
    ("Legal", "Oficio (EE.UU.)"),
    ("A4", "A4"),
    ("A5", "A5"),
    ("Tabloid", "Tabloide"),
    ("Executive", "Ejecutivo"),
    ("Env10", "Sobre #10"),
];

const IP_PROTOCOLS: &[(&str, &str)] = &[
    ("ipp", "Protocolo de impresión de Internet (IPP)"),
    ("ipps", "IPP seguro (IPPS)"),
    ("lpd", "Line Printer Daemon (LPD)"),
    ("socket", "HP JetDirect (Socket)"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct CupsPrinter {
    name: String,
    uri: String,
    status_label: String,
    paused: bool,
    is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveredDevice {
    kind: String,
    uri: String,
    display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SaneScanner {
    device_id: String,
    description: String,
}

#[derive(Debug, Clone)]
struct PrintersSnapshot {
    printers: Vec<CupsPrinter>,
    scanners: Vec<SaneScanner>,
    system_default: Option<String>,
}

fn paper_size_to_idx(key: &str) -> u32 {
    PAPER_SIZES
        .iter()
        .position(|(k, _)| k.eq_ignore_ascii_case(key))
        .unwrap_or(0) as u32
}

fn idx_to_paper_size(idx: u32) -> &'static str {
    PAPER_SIZES
        .get(idx as usize)
        .map(|(k, _)| *k)
        .unwrap_or("Letter")
}

/// Parsea `LC_ALL=C lpstat -p -d -v` en una lista estructurada de impresoras CUPS.
fn parse_lpstat_output(raw: &str) -> (Vec<CupsPrinter>, Option<String>) {
    let mut printers: Vec<CupsPrinter> = Vec::new();
    let mut uris: HashMap<String, String> = HashMap::new();
    let mut system_default: Option<String> = None;

    for line in raw.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("system default destination:") {
            let name = rest.trim();
            if !name.is_empty() {
                system_default = Some(name.to_string());
            }
        } else if let Some(rest) = l.strip_prefix("device for ") {
            if let Some((name, uri)) = rest.split_once(':') {
                uris.insert(name.trim().to_string(), uri.trim().to_string());
            }
        } else if let Some(rest) = l.strip_prefix("printer ") {
            let mut parts = rest.splitn(2, ' ');
            let Some(name) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            let state_desc = parts.next().unwrap_or("").to_lowercase();
            let (status_label, paused) = if state_desc.contains("disabled") {
                ("En pausa".to_string(), true)
            } else if state_desc.contains("now printing") {
                ("Imprimiendo".to_string(), false)
            } else {
                ("Inactiva".to_string(), false)
            };
            if !printers.iter().any(|p| p.name == name) {
                printers.push(CupsPrinter {
                    name: name.to_string(),
                    uri: String::new(),
                    status_label,
                    paused,
                    is_default: false,
                });
            }
        }
    }

    for p in &mut printers {
        if let Some(u) = uris.get(&p.name) {
            p.uri = u.clone();
        }
        if system_default.as_deref() == Some(p.name.as_str()) {
            p.is_default = true;
        }
    }

    (printers, system_default)
}

/// Parsea `LC_ALL=C scanimage -L` en escáneres SANE detectados.
fn parse_scanimage_output(raw: &str) -> Vec<SaneScanner> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("device `") {
            if let Some((dev_id, desc_part)) = rest.split_once("' is a ") {
                let id = dev_id.trim();
                let desc = desc_part.trim();
                if !id.is_empty() {
                    out.push(SaneScanner {
                        device_id: id.to_string(),
                        description: if desc.is_empty() {
                            id.to_string()
                        } else {
                            desc.to_string()
                        },
                    });
                }
            }
        }
    }
    out
}

/// Parsea `lpinfo -v` filtrando URIs reales de dispositivos (USB, IPP, DNSSD, LPD, Socket).
fn parse_lpinfo_devices(raw: &str) -> Vec<DiscoveredDevice> {
    let mut devices = Vec::new();
    for line in raw.lines() {
        let l = line.trim();
        let mut parts = l.split_whitespace();
        let Some(kind) = parts.next() else {
            continue;
        };
        let Some(uri) = parts.next() else {
            continue;
        };
        // Descartar backends genéricos sin dirección concreta (ej. "network ipp", "network http")
        if !uri.contains("://") && !uri.starts_with("usb:") {
            continue;
        }
        let kind_label = match kind {
            "direct" => "USB / Local",
            "network" => "Red",
            other => other,
        };
        let display_name = derive_device_display_name(uri);
        devices.push(DiscoveredDevice {
            kind: kind_label.to_string(),
            uri: uri.to_string(),
            display_name,
        });
    }
    devices
}

fn derive_device_display_name(uri: &str) -> String {
    if let Some(rest) = uri.split("://").nth(1) {
        let clean = rest
            .split(['/', '?'])
            .next()
            .unwrap_or(rest)
            .replace("%20", " ");
        if !clean.trim().is_empty() {
            return clean.trim().to_string();
        }
    }
    uri.to_string()
}

/// Convierte un nombre legible en un nombre de cola CUPS válido (`A-Za-z0-9_-`).
fn sanitize_cups_queue_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch.is_whitespace() || ch == '.' {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "Impresora_Malus".to_string()
    } else {
        trimmed
    }
}

fn probe_printers_snapshot() -> PrintersSnapshot {
    let lpstat_raw = Command::new("lpstat")
        .env("LC_ALL", "C")
        .args(["-p", "-d", "-v"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let (printers, system_default) = parse_lpstat_output(&lpstat_raw);

    let scanners = Command::new("scanimage")
        .env("LC_ALL", "C")
        .arg("-L")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_scanimage_output(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();

    PrintersSnapshot {
        printers,
        scanners,
        system_default,
    }
}

fn probe_available_devices() -> Vec<DiscoveredDevice> {
    let raw = Command::new("lpinfo")
        .env("LC_ALL", "C")
        .arg("-v")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    parse_lpinfo_devices(&raw)
}

fn apply_cups_default_printer(printer_name: &str) {
    let name = printer_name.to_string();
    std::thread::spawn(move || {
        let _ = Command::new("lpoptions").args(["-d", &name]).status();
    });
}

fn apply_cups_paper_size(paper_key: &str, printers: &[CupsPrinter]) {
    let key = paper_key.to_string();
    let names: Vec<String> = printers.iter().map(|p| p.name.clone()).collect();
    std::thread::spawn(move || {
        for name in names {
            let _ = Command::new("lpoptions")
                .args(["-p", &name, "-o", &format!("media={key}")])
                .status();
        }
    });
}

fn query_printer_queue(printer_name: &str) -> String {
    let Ok(out) = Command::new("lpq")
        .env("LC_ALL", "C")
        .args(["-P", printer_name])
        .output()
    else {
        return "No se pudo consultar la cola de impresión.".to_string();
    };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() || text.to_lowercase().contains("no entries") {
        "No hay trabajos pendientes en la cola.".to_string()
    } else {
        text
    }
}

fn make_row_divider() -> GtkBox {
    GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["wifi-row-separator"])
        .hexpand(true)
        .build()
}

fn open_printer_detail_modal(
    parent: &ApplicationWindow,
    printer: CupsPrinter,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = Window::builder()
        .title(&printer.name)
        .transient_for(parent)
        .modal(true)
        .default_width(470)
        .resizable(false)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .margin_top(18)
        .margin_bottom(16)
        .margin_start(20)
        .margin_end(20)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(Align::Center)
        .build();
    let icon = Image::builder()
        .icon_name("printer-symbolic")
        .pixel_size(32)
        .build();
    let title_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    title_col.append(
        &Label::builder()
            .label(&printer.name)
            .halign(Align::Start)
            .css_classes(["title-3"])
            .build(),
    );
    let subtitle_txt = if printer.is_default {
        format!("{} · Predeterminada", printer.status_label)
    } else {
        printer.status_label.clone()
    };
    title_col.append(
        &Label::builder()
            .label(&subtitle_txt)
            .halign(Align::Start)
            .css_classes(["card-section-sub"])
            .build(),
    );
    header.append(&icon);
    header.append(&title_col);
    root.append(&header);

    // Tarjeta de información y cola
    let info_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    let uri_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    uri_row.append(
        &Label::builder()
            .label("Ubicación / URI")
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    uri_row.append(
        &Label::builder()
            .label(if printer.uri.is_empty() {
                "Local"
            } else {
                &printer.uri
            })
            .halign(Align::End)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .max_width_chars(30)
            .css_classes(["card-section-sub"])
            .build(),
    );
    info_inner.append(&uri_row);
    info_inner.append(&make_row_divider());

    let queue_row = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .css_classes(["wifi-pref-row"])
        .build();
    queue_row.append(
        &Label::builder()
            .label("Cola de impresión")
            .halign(Align::Start)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let queue_lbl = Label::builder()
        .label("Consultando cola…")
        .halign(Align::Start)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["card-section-sub"])
        .build();
    queue_row.append(&queue_lbl);
    info_inner.append(&queue_row);

    root.append(&wrap_squircle_card(&info_inner, WIFI_CARD_SQUIRCLE_RADIUS));

    // Acciones rápidas
    let actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::Start)
        .build();

    let (test_wrap, test_btn) = make_squircle_button("Página de prueba", false, None);
    let pause_title = if printer.paused { "Reanudar" } else { "Pausar" };
    let (pause_wrap, pause_btn) = make_squircle_button(pause_title, false, None);
    let (cancel_wrap, cancel_btn) = make_squircle_button("Vaciar cola", false, None);
    actions_row.append(&test_wrap);
    actions_row.append(&pause_wrap);
    actions_row.append(&cancel_wrap);
    root.append(&actions_row);

    let feedback_lbl = Label::builder()
        .halign(Align::Start)
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(["card-section-sub"])
        .build();
    root.append(&feedback_lbl);

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .margin_top(4)
        .build();
    let remove_btn = Button::builder()
        .label("Eliminar impresora")
        .css_classes(["destructive-action"])
        .build();
    let spacer = GtkBox::builder().hexpand(true).build();
    let done_btn = Button::builder()
        .label("Listo")
        .css_classes(["suggested-action"])
        .build();
    footer.append(&remove_btn);
    footer.append(&spacer);
    footer.append(&done_btn);
    root.append(&footer);

    dialog.set_child(Some(&root));

    // Cargar estado de la cola en segundo plano
    {
        let pname = printer.name.clone();
        let qlbl = queue_lbl.clone();
        glib::MainContext::default().spawn_local(async move {
            let Ok(text) = gtk4::gio::spawn_blocking(move || query_printer_queue(&pname)).await
            else {
                return;
            };
            qlbl.set_label(&text);
        });
    }

    // Página de prueba
    {
        let pname = printer.name.clone();
        let fb = feedback_lbl.clone();
        let qlbl = queue_lbl.clone();
        test_btn.connect_clicked(move |_| {
            let pname = pname.clone();
            let fb = fb.clone();
            let qlbl = qlbl.clone();
            fb.set_label("Enviando página de prueba…");
            fb.set_visible(true);
            glib::MainContext::default().spawn_local(async move {
                let pname_bg = pname.clone();
                let Ok(ok) = gtk4::gio::spawn_blocking(move || {
                    let test_file = "/usr/share/cups/data/testprint";
                    if std::path::Path::new(test_file).exists() {
                        Command::new("lp")
                            .args(["-d", &pname_bg, test_file])
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false)
                    } else {
                        let tmp = std::env::temp_dir().join("malus-de-testprint.txt");
                        let _ = std::fs::write(
                            &tmp,
                            "Malus DE — Página de prueba de impresión\nCUPS funcionando correctamente.\n",
                        );
                        Command::new("lp")
                            .args(["-d", &pname_bg])
                            .arg(&tmp)
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false)
                    }
                })
                .await
                else {
                    return;
                };
                if ok {
                    fb.set_label("Página de prueba enviada a la cola.");
                } else {
                    fb.set_label("No se pudo enviar la página de prueba (verifica el servicio CUPS).");
                }
                let q = gtk4::gio::spawn_blocking(move || query_printer_queue(&pname))
                    .await
                    .unwrap_or_default();
                qlbl.set_label(&q);
            });
        });
    }

    // Pausar / Reanudar
    {
        let pname = printer.name.clone();
        let paused = printer.paused;
        let fb = feedback_lbl.clone();
        let on_chg = on_changed.clone();
        let dlg_c = dialog.clone();
        pause_btn.connect_clicked(move |_| {
            let pname = pname.clone();
            let fb = fb.clone();
            let on_chg = on_chg.clone();
            let dlg_c = dlg_c.clone();
            glib::MainContext::default().spawn_local(async move {
                let cmd = if paused { "cupsenable" } else { "cupsdisable" };
                let Ok(ok) = gtk4::gio::spawn_blocking(move || {
                    Command::new("pkexec")
                        .args([cmd, &pname])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                })
                .await
                else {
                    return;
                };
                if ok {
                    on_chg();
                    crate::close_attached_modal(&dlg_c);
                } else {
                    fb.set_label("Operación cancelada o sin permisos suficientes.");
                    fb.set_visible(true);
                }
            });
        });
    }

    // Vaciar cola
    {
        let pname = printer.name.clone();
        let fb = feedback_lbl.clone();
        let qlbl = queue_lbl.clone();
        cancel_btn.connect_clicked(move |_| {
            let pname = pname.clone();
            let fb = fb.clone();
            let qlbl = qlbl.clone();
            glib::MainContext::default().spawn_local(async move {
                let p2 = pname.clone();
                let _ = gtk4::gio::spawn_blocking(move || {
                    let _ = Command::new("cancel").args(["-a", &p2]).status();
                })
                .await;
                fb.set_label("Se cancelaron los trabajos pendientes.");
                fb.set_visible(true);
                let q = gtk4::gio::spawn_blocking(move || query_printer_queue(&pname))
                    .await
                    .unwrap_or_default();
                qlbl.set_label(&q);
            });
        });
    }

    // Eliminar impresora
    {
        let pname = printer.name.clone();
        let fb = feedback_lbl.clone();
        let on_chg = on_changed.clone();
        let dlg_c = dialog.clone();
        remove_btn.connect_clicked(move |_| {
            let pname = pname.clone();
            let fb = fb.clone();
            let on_chg = on_chg.clone();
            let dlg_c = dlg_c.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(ok) = gtk4::gio::spawn_blocking(move || {
                    Command::new("pkexec")
                        .args(["lpadmin", "-x", &pname])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                })
                .await
                else {
                    return;
                };
                if ok {
                    on_chg();
                    crate::close_attached_modal(&dlg_c);
                } else {
                    fb.set_label("No se pudo eliminar la impresora (operación cancelada).");
                    fb.set_visible(true);
                }
            });
        });
    }

    let dlg_done = dialog.clone();
    done_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_done);
    });

    crate::present_attached_modal(&dialog, parent);
}

/// Modal anclado «Agregar impresora, escáner o fax...» con pestañas Predeterminado / IP / Fax.
fn open_add_printer_modal(parent: &ApplicationWindow, on_added: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .title("Agregar impresora, escáner o fax")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .resizable(false)
        .build();

    let root = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .margin_top(18)
        .margin_bottom(16)
        .margin_start(20)
        .margin_end(20)
        .build();

    root.append(
        &Label::builder()
            .label("Agregar impresora, escáner o fax")
            .halign(Align::Start)
            .css_classes(["title-3"])
            .build(),
    );

    // Barra segmentada de 3 pestañas: Predeterminado | IP | Fax
    let seg_track = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .homogeneous(true)
        .css_classes(["bat-segmented-track"])
        .hexpand(true)
        .build();
    let tab_def_btn = Button::builder()
        .label("Predeterminado")
        .css_classes(["bat-seg-tab", "bat-seg-tab-active"])
        .hexpand(true)
        .build();
    let tab_ip_btn = Button::builder()
        .label("IP")
        .css_classes(["bat-seg-tab"])
        .hexpand(true)
        .build();
    let tab_fax_btn = Button::builder()
        .label("Fax")
        .css_classes(["bat-seg-tab"])
        .hexpand(true)
        .build();
    seg_track.append(&tab_def_btn);
    seg_track.append(&tab_ip_btn);
    seg_track.append(&tab_fax_btn);
    root.append(&seg_track);

    let active_mode = Rc::new(Cell::new(0u8));
    let selected_uri: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let modal_stack = Stack::builder()
        .transition_type(StackTransitionType::Crossfade)
        .transition_duration(120)
        .vhomogeneous(false)
        .hexpand(true)
        .build();

    // --- Pestaña 1: Predeterminado (descubrimiento real vía lpinfo -v) ---
    let def_page = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .build();

    let discovered_list = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();
    let scan_status = Label::builder()
        .label("Buscando impresoras USB y de red con CUPS…")
        .halign(Align::Center)
        .margin_top(16)
        .margin_bottom(16)
        .css_classes(["card-section-sub"])
        .build();
    discovered_list.append(&scan_status);

    let discovered_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(130)
        .max_content_height(180)
        .child(&discovered_list)
        .build();
    let discovered_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["wifi-card-compact"])
        .build();
    discovered_card_box.append(&discovered_scroll);
    def_page.append(&wrap_squircle_card(
        &discovered_card_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    let def_name_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    def_name_row.append(
        &Label::builder()
            .label("Nombre:")
            .width_chars(10)
            .xalign(0.0)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let def_name_entry = Entry::builder()
        .placeholder_text("Ej. HP_LaserJet_Oficina")
        .hexpand(true)
        .build();
    def_name_row.append(&def_name_entry);
    def_page.append(&def_name_row);

    let def_uri_lbl = Label::builder()
        .label("Selecciona un dispositivo detectado arriba o usa la pestaña IP.")
        .halign(Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .css_classes(["card-section-sub"])
        .build();
    def_page.append(&def_uri_lbl);
    modal_stack.add_named(&def_page, Some("default"));

    // --- Pestaña 2: IP (agregar impresora de red por dirección IPP/IPPS/LPD/Socket) ---
    let ip_page = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    let ip_addr_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    ip_addr_row.append(
        &Label::builder()
            .label("Dirección")
            .width_chars(12)
            .xalign(0.0)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let ip_addr_entry = Entry::builder()
        .placeholder_text("192.168.1.50 o impresora.local")
        .hexpand(true)
        .build();
    ip_addr_row.append(&ip_addr_entry);
    ip_page.append(&ip_addr_row);
    ip_page.append(&make_row_divider());

    let ip_proto_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    ip_proto_row.append(
        &Label::builder()
            .label("Protocolo")
            .width_chars(12)
            .xalign(0.0)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let proto_labels: Vec<&str> = IP_PROTOCOLS.iter().map(|(_, l)| *l).collect();
    let ip_proto_dd = DropDown::builder()
        .model(&StringList::new(&proto_labels))
        .selected(0)
        .css_classes(["wifi-compact-dropdown"])
        .hexpand(true)
        .build();
    ip_proto_row.append(&ip_proto_dd);
    ip_page.append(&ip_proto_row);
    ip_page.append(&make_row_divider());

    let ip_queue_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    ip_queue_row.append(
        &Label::builder()
            .label("Cola")
            .width_chars(12)
            .xalign(0.0)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let ip_queue_entry = Entry::builder()
        .placeholder_text("ipp/print (opcional)")
        .hexpand(true)
        .build();
    ip_queue_row.append(&ip_queue_entry);
    ip_page.append(&ip_queue_row);
    ip_page.append(&make_row_divider());

    let ip_name_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .css_classes(["wifi-pref-row"])
        .build();
    ip_name_row.append(
        &Label::builder()
            .label("Nombre")
            .width_chars(12)
            .xalign(0.0)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let ip_name_entry = Entry::builder()
        .placeholder_text("Impresora_Red")
        .hexpand(true)
        .build();
    ip_name_row.append(&ip_name_entry);
    ip_page.append(&ip_name_row);

    let ip_card = wrap_squircle_card(&ip_page, WIFI_CARD_SQUIRCLE_RADIUS);
    modal_stack.add_named(&ip_card, Some("ip"));

    // --- Pestaña 3: Fax (por implementar) ---
    let fax_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .css_classes(["wifi-pref-row"])
        .build();
    let fax_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    fax_title_row.append(
        &Label::builder()
            .label("Configurar módem de fax")
            .halign(Align::Start)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    fax_title_row.append(&make_pending_badge());
    fax_inner.append(&fax_title_row);
    fax_inner.append(
        &Label::builder()
            .label(
                "El envío y recepción de fax mediante módem telefónico aún no está disponible en Malus DE.",
            )
            .halign(Align::Start)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["card-section-sub"])
            .build(),
    );
    modal_stack.add_named(
        &wrap_squircle_card(&fax_inner, WIFI_CARD_SQUIRCLE_RADIUS),
        Some("fax"),
    );

    modal_stack.set_visible_child_name("default");
    root.append(&modal_stack);

    let feedback_lbl = Label::builder()
        .halign(Align::Start)
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(["card-section-sub"])
        .build();
    root.append(&feedback_lbl);

    // Botones inferiores
    let btn_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(2)
        .build();
    let (rescan_wrap, rescan_btn) = make_squircle_button("Actualizar", false, None);
    let spacer = GtkBox::builder().hexpand(true).build();
    let (cancel_wrap, cancel_btn) = make_squircle_button("Cancelar", false, None);
    let add_btn = Button::builder()
        .label("Agregar")
        .css_classes(["suggested-action"])
        .build();
    btn_row.append(&rescan_wrap);
    btn_row.append(&spacer);
    btn_row.append(&cancel_wrap);
    btn_row.append(&add_btn);
    root.append(&btn_row);

    dialog.set_child(Some(&root));

    // Cambio de pestañas del modal
    let select_modal_tab = {
        let stack_c = modal_stack.clone();
        let b0 = tab_def_btn.clone();
        let b1 = tab_ip_btn.clone();
        let b2 = tab_fax_btn.clone();
        let mode_c = active_mode.clone();
        let add_b = add_btn.clone();
        let rescan_w = rescan_wrap.clone();
        Rc::new(move |idx: u8| {
            mode_c.set(idx);
            let name = match idx {
                1 => "ip",
                2 => "fax",
                _ => "default",
            };
            stack_c.set_visible_child_name(name);
            for (btn, i) in [(&b0, 0u8), (&b1, 1u8), (&b2, 2u8)] {
                if i == idx {
                    btn.add_css_class("bat-seg-tab-active");
                } else {
                    btn.remove_css_class("bat-seg-tab-active");
                }
            }
            add_b.set_sensitive(idx != 2);
            rescan_w.set_visible(idx == 0);
        })
    };

    {
        let s = select_modal_tab.clone();
        tab_def_btn.connect_clicked(move |_| s(0));
    }
    {
        let s = select_modal_tab.clone();
        tab_ip_btn.connect_clicked(move |_| s(1));
    }
    {
        let s = select_modal_tab.clone();
        tab_fax_btn.connect_clicked(move |_| s(2));
    }

    // Descubrimiento asíncrono con lpinfo -v
    let run_discovery = {
        let list_c = discovered_list.clone();
        let sel_uri = selected_uri.clone();
        let name_entry = def_name_entry.clone();
        let uri_lbl = def_uri_lbl.clone();
        Rc::new(move || {
            while let Some(child) = list_c.first_child() {
                list_c.remove(&child);
            }
            list_c.append(
                &Label::builder()
                    .label("Buscando impresoras USB y de red con CUPS…")
                    .halign(Align::Center)
                    .margin_top(16)
                    .margin_bottom(16)
                    .css_classes(["card-section-sub"])
                    .build(),
            );
            let list_r = list_c.clone();
            let sel_r = sel_uri.clone();
            let name_r = name_entry.clone();
            let uri_r = uri_lbl.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(devices) = gtk4::gio::spawn_blocking(probe_available_devices).await else {
                    return;
                };
                while let Some(child) = list_r.first_child() {
                    list_r.remove(&child);
                }
                if devices.is_empty() {
                    list_r.append(
                        &Label::builder()
                            .label(
                                "No se detectaron impresoras automáticamente. Conecta una por USB o agrégala en la pestaña IP.",
                            )
                            .halign(Align::Center)
                            .wrap(true)
                            .margin_top(16)
                            .margin_bottom(16)
                            .margin_start(14)
                            .margin_end(14)
                            .css_classes(["card-section-sub"])
                            .build(),
                    );
                    return;
                }
                for (idx, dev) in devices.into_iter().enumerate() {
                    if idx > 0 {
                        list_r.append(&make_row_divider());
                    }
                    let row_btn = Button::builder()
                        .has_frame(false)
                        .css_classes(["wifi-network-row"])
                        .build();
                    let row_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(10)
                        .css_classes(["wifi-pref-row"])
                        .build();
                    row_box.append(
                        &Image::builder()
                            .icon_name("printer-symbolic")
                            .pixel_size(18)
                            .build(),
                    );
                    let col = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(1)
                        .hexpand(true)
                        .build();
                    col.append(
                        &Label::builder()
                            .label(&dev.display_name)
                            .halign(Align::Start)
                            .css_classes(["wifi-row-title"])
                            .build(),
                    );
                    col.append(
                        &Label::builder()
                            .label(&format!("{} · {}", dev.kind, dev.uri))
                            .halign(Align::Start)
                            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                            .css_classes(["card-section-sub"])
                            .build(),
                    );
                    row_box.append(&col);
                    row_btn.set_child(Some(&row_box));

                    let uri_val = dev.uri.clone();
                    let disp_val = dev.display_name.clone();
                    let sel_btn = sel_r.clone();
                    let name_btn = name_r.clone();
                    let uri_lbl_btn = uri_r.clone();
                    row_btn.connect_clicked(move |_| {
                        *sel_btn.borrow_mut() = Some(uri_val.clone());
                        if name_btn.text().trim().is_empty() {
                            name_btn.set_text(&sanitize_cups_queue_name(&disp_val));
                        }
                        uri_lbl_btn.set_label(&format!("URI seleccionada: {uri_val}"));
                    });

                    list_r.append(&row_btn);
                }
            });
        })
    };

    run_discovery();
    {
        let rd = run_discovery.clone();
        rescan_btn.connect_clicked(move |_| rd());
    }

    // Agregar impresora mediante pkexec lpadmin
    {
        let mode_c = active_mode.clone();
        let sel_uri = selected_uri.clone();
        let def_name = def_name_entry.clone();
        let ip_addr = ip_addr_entry.clone();
        let ip_proto = ip_proto_dd.clone();
        let ip_queue = ip_queue_entry.clone();
        let ip_name = ip_name_entry.clone();
        let fb = feedback_lbl.clone();
        let dlg_c = dialog.clone();
        let on_add = on_added.clone();
        add_btn.connect_clicked(move |_| {
            let (queue_name, uri) = if mode_c.get() == 0 {
                let Some(u) = sel_uri.borrow().clone() else {
                    fb.set_label("Selecciona una impresora de la lista o usa la pestaña IP.");
                    fb.set_visible(true);
                    return;
                };
                let raw_name = def_name.text().to_string();
                let name = if raw_name.trim().is_empty() {
                    sanitize_cups_queue_name(&derive_device_display_name(&u))
                } else {
                    sanitize_cups_queue_name(&raw_name)
                };
                (name, u)
            } else if mode_c.get() == 1 {
                let addr = ip_addr.text().trim().to_string();
                if addr.is_empty() {
                    fb.set_label("Ingresa una dirección IP o nombre de host válido.");
                    fb.set_visible(true);
                    return;
                }
                let scheme = IP_PROTOCOLS
                    .get(ip_proto.selected() as usize)
                    .map(|(s, _)| *s)
                    .unwrap_or("ipp");
                let queue_path = ip_queue.text().trim().trim_start_matches('/').to_string();
                let uri = if scheme == "socket" {
                    format!("socket://{addr}:9100")
                } else if queue_path.is_empty() {
                    format!("{scheme}://{addr}/ipp/print")
                } else {
                    format!("{scheme}://{addr}/{queue_path}")
                };
                let raw_name = ip_name.text().to_string();
                let name = if raw_name.trim().is_empty() {
                    sanitize_cups_queue_name(&format!("Impresora_{addr}"))
                } else {
                    sanitize_cups_queue_name(&raw_name)
                };
                (name, uri)
            } else {
                return;
            };

            fb.set_label("Instalando impresora en CUPS…");
            fb.set_visible(true);

            let fb_r = fb.clone();
            let dlg_r = dlg_c.clone();
            let on_add_r = on_add.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(result) = gtk4::gio::spawn_blocking(move || {
                    let out = Command::new("pkexec")
                        .args(["lpadmin", "-p", &queue_name, "-E", "-v", &uri, "-m", "everywhere"])
                        .output();
                    match out {
                        Ok(o) if o.status.success() => Ok(()),
                        Ok(o) => {
                            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                            Err(if err.is_empty() {
                                "No se pudo agregar la impresora (operación cancelada o URI no compatible).".to_string()
                            } else {
                                err
                            })
                        }
                        Err(e) => Err(format!("Error al ejecutar lpadmin: {e}")),
                    }
                })
                .await
                else {
                    return;
                };

                match result {
                    Ok(()) => {
                        on_add_r();
                        crate::close_attached_modal(&dlg_r);
                    }
                    Err(msg) => {
                        fb_r.set_label(&msg);
                        fb_r.set_visible(true);
                    }
                }
            });
        });
    }

    let dlg_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        crate::close_attached_modal(&dlg_cancel);
    });

    crate::present_attached_modal(&dialog, parent);
}

pub fn create_printers_pane(parent: &ApplicationWindow) -> (GtkBox, Rc<dyn Fn()>) {
    let prefs = Rc::new(RefCell::new(PrintersConfig::load()));
    let current_printers: Rc<RefCell<Vec<CupsPrinter>>> = Rc::new(RefCell::new(Vec::new()));
    let default_options_keys: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(vec!["last_used".to_string()]));
    let syncing = Rc::new(Cell::new(false));

    let container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();

    // =========================================================================
    // Tarjeta 1: Impresora predeterminada + Tamaño del papel predeterminado
    // =========================================================================
    let prefs_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();

    let def_printer_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .valign(Align::Center)
        .build();
    def_printer_row.append(
        &Label::builder()
            .label("Impresora predeterminada")
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let def_printer_dd = DropDown::builder()
        .model(&StringList::new(&["Última impresora utilizada"]))
        .selected(0)
        .css_classes(["wifi-compact-dropdown"])
        .valign(Align::Center)
        .build();
    def_printer_row.append(&def_printer_dd);
    prefs_card_box.append(&def_printer_row);
    prefs_card_box.append(&make_row_divider());

    let paper_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .valign(Align::Center)
        .build();
    paper_row.append(
        &Label::builder()
            .label("Tamaño del papel predeterminado")
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["wifi-row-title"])
            .build(),
    );
    let paper_labels: Vec<&str> = PAPER_SIZES.iter().map(|(_, l)| *l).collect();
    let paper_dd = DropDown::builder()
        .model(&StringList::new(&paper_labels))
        .selected(paper_size_to_idx(&prefs.borrow().default_paper_size))
        .css_classes(["wifi-compact-dropdown"])
        .valign(Align::Center)
        .build();
    paper_row.append(&paper_dd);
    prefs_card_box.append(&paper_row);

    container.append(&wrap_squircle_card(
        &prefs_card_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // =========================================================================
    // Tarjeta 2: Lista de impresoras y escáneres («Sin impresoras» si está vacía)
    // =========================================================================
    let list_card_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .css_classes(["wifi-card-compact"])
        .build();
    list_card_box.append(
        &Label::builder()
            .label("Sin impresoras")
            .halign(Align::Center)
            .margin_top(10)
            .margin_bottom(10)
            .css_classes(["card-section-sub"])
            .build(),
    );
    container.append(&wrap_squircle_card(
        &list_card_box,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // =========================================================================
    // Pie de página: «Agregar impresora, escáner o fax...» + botón «?»
    // =========================================================================
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();

    let (add_wrap, add_btn) =
        make_squircle_button("Agregar impresora, escáner o fax...", false, None);
    let (help_wrap, help_btn) = make_help_circle_button("Ayuda de Impresoras y escáneres");
    let help_pop = Popover::builder().has_arrow(true).build();
    let help_lbl = Label::builder()
        .label(
            "Administra impresoras CUPS locales o en red (IPP Everywhere) y escáneres SANE.\n\nUsa «Agregar impresora, escáner o fax...» para descubrir equipos conectados por USB o agregar una impresora por dirección IP.",
        )
        .wrap(true)
        .max_width_chars(38)
        .xalign(0.0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    help_pop.set_child(Some(&help_lbl));
    help_btn.set_popover(Some(&help_pop));

    footer.append(&add_wrap);
    footer.append(&help_wrap);
    container.append(&footer);

    // =========================================================================
    // Señales interactivas
    // =========================================================================
    {
        let prefs_c = prefs.clone();
        let keys_c = default_options_keys.clone();
        let sync_c = syncing.clone();
        def_printer_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let idx = dd.selected() as usize;
            let key = keys_c
                .borrow()
                .get(idx)
                .cloned()
                .unwrap_or_else(|| "last_used".to_string());
            prefs_c.borrow_mut().default_printer = key.clone();
            let _ = prefs_c.borrow().save();
            if key != "last_used" {
                apply_cups_default_printer(&key);
            }
        });
    }

    {
        let prefs_c = prefs.clone();
        let printers_c = current_printers.clone();
        let sync_c = syncing.clone();
        paper_dd.connect_selected_notify(move |dd| {
            if sync_c.get() {
                return;
            }
            let paper = idx_to_paper_size(dd.selected());
            prefs_c.borrow_mut().default_paper_size = paper.to_string();
            let _ = prefs_c.borrow().save();
            apply_cups_paper_size(paper, &printers_c.borrow());
        });
    }

    // Refresher compartido (consulta lpstat + scanimage en segundo plano)
    let refresh_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh: Rc<dyn Fn()> = {
        let prefs_c = prefs.clone();
        let printers_c = current_printers.clone();
        let keys_c = default_options_keys.clone();
        let sync_c = syncing.clone();
        let def_dd = def_printer_dd.clone();
        let pap_dd = paper_dd.clone();
        let list_box = list_card_box.clone();
        let parent_c = parent.clone();
        let holder_c = refresh_holder.clone();
        Rc::new(move || {
            let loaded = PrintersConfig::load();
            sync_c.set(true);
            pap_dd.set_selected(paper_size_to_idx(&loaded.default_paper_size));
            *prefs_c.borrow_mut() = loaded.clone();
            sync_c.set(false);

            let printers_r = printers_c.clone();
            let keys_r = keys_c.clone();
            let sync_r = sync_c.clone();
            let def_dd_r = def_dd.clone();
            let list_r = list_box.clone();
            let parent_r = parent_c.clone();
            let holder_r = holder_c.clone();
            let saved_default = loaded.default_printer.clone();

            glib::MainContext::default().spawn_local(async move {
                let Ok(snap) = gtk4::gio::spawn_blocking(probe_printers_snapshot).await else {
                    return;
                };

                // Actualizar desplegable de impresora predeterminada
                let mut labels = vec!["Última impresora utilizada".to_string()];
                let mut keys = vec!["last_used".to_string()];
                for p in &snap.printers {
                    labels.push(p.name.clone());
                    keys.push(p.name.clone());
                }
                let desired_key = if saved_default != "last_used" {
                    saved_default
                } else {
                    snap.system_default
                        .clone()
                        .unwrap_or_else(|| "last_used".to_string())
                };
                let sel_idx = keys
                    .iter()
                    .position(|k| k == &desired_key)
                    .unwrap_or(0) as u32;
                let model = {
                    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                    StringList::new(&refs)
                };
                *keys_r.borrow_mut() = keys;
                *printers_r.borrow_mut() = snap.printers.clone();

                sync_r.set(true);
                def_dd_r.set_model(Some(&model));
                def_dd_r.set_selected(sel_idx);
                sync_r.set(false);

                // Reconstruir lista de impresoras y escáneres
                while let Some(child) = list_r.first_child() {
                    list_r.remove(&child);
                }

                if snap.printers.is_empty() && snap.scanners.is_empty() {
                    list_r.append(
                        &Label::builder()
                            .label("Sin impresoras")
                            .halign(Align::Center)
                            .margin_top(10)
                            .margin_bottom(10)
                            .css_classes(["card-section-sub"])
                            .build(),
                    );
                    return;
                }

                let mut added_rows = 0usize;
                for printer in snap.printers {
                    if added_rows > 0 {
                        list_r.append(&make_row_divider());
                    }
                    added_rows += 1;

                    let row_btn = Button::builder()
                        .has_frame(false)
                        .css_classes(["wifi-network-row"])
                        .build();
                    let row = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(12)
                        .css_classes(["wifi-pref-row"])
                        .valign(Align::Center)
                        .build();
                    row.append(
                        &Image::builder()
                            .icon_name("printer-symbolic")
                            .pixel_size(20)
                            .valign(Align::Center)
                            .build(),
                    );
                    let col = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(2)
                        .hexpand(true)
                        .build();
                    col.append(
                        &Label::builder()
                            .label(&printer.name)
                            .halign(Align::Start)
                            .css_classes(["wifi-row-title"])
                            .build(),
                    );
                    let status_sub = if printer.is_default {
                        format!("{} · Predeterminada", printer.status_label)
                    } else {
                        printer.status_label.clone()
                    };
                    col.append(
                        &Label::builder()
                            .label(&status_sub)
                            .halign(Align::Start)
                            .css_classes(["card-section-sub"])
                            .build(),
                    );
                    row.append(&col);
                    row.append(
                        &Image::builder()
                            .icon_name("go-next-symbolic")
                            .pixel_size(14)
                            .valign(Align::Center)
                            .build(),
                    );
                    row_btn.set_child(Some(&row));

                    let parent_d = parent_r.clone();
                    let holder_d = holder_r.clone();
                    let printer_d = printer.clone();
                    row_btn.connect_clicked(move |_| {
                        let cb: Rc<dyn Fn()> = {
                            let h = holder_d.clone();
                            Rc::new(move || {
                                if let Some(rf) = h.borrow().as_ref() {
                                    rf();
                                }
                            })
                        };
                        open_printer_detail_modal(&parent_d, printer_d.clone(), cb);
                    });
                    list_r.append(&row_btn);
                }

                for scanner in snap.scanners {
                    if added_rows > 0 {
                        list_r.append(&make_row_divider());
                    }
                    added_rows += 1;

                    let row = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(12)
                        .css_classes(["wifi-pref-row"])
                        .valign(Align::Center)
                        .build();
                    row.append(
                        &Image::builder()
                            .icon_name("scanner-symbolic")
                            .pixel_size(20)
                            .valign(Align::Center)
                            .build(),
                    );
                    let col = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(2)
                        .hexpand(true)
                        .build();
                    col.append(
                        &Label::builder()
                            .label(&scanner.description)
                            .halign(Align::Start)
                            .css_classes(["wifi-row-title"])
                            .build(),
                    );
                    col.append(
                        &Label::builder()
                            .label(&format!("Escáner · {}", scanner.device_id))
                            .halign(Align::Start)
                            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                            .css_classes(["card-section-sub"])
                            .build(),
                    );
                    row.append(&col);
                    let (scan_wrap, scan_btn) = make_squircle_button("Abrir escáner…", false, None);
                    scan_btn.connect_clicked(|_| {
                        let _ = Command::new("simple-scan").spawn();
                    });
                    row.append(&scan_wrap);
                    list_r.append(&row);
                }
            });
        })
    };

    *refresh_holder.borrow_mut() = Some(refresh.clone());

    {
        let parent_c = parent.clone();
        let rf = refresh.clone();
        add_btn.connect_clicked(move |_| {
            open_add_printer_modal(&parent_c, rf.clone());
        });
    }

    refresh();

    (container, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lpstat_extracts_printers_uris_and_default() {
        let sample = "\
printer HP_LaserJet is idle.  enabled since Mon 24 Nov 2026
printer Canon_Pixma disabled since Mon 24 Nov 2026 - Paused
system default destination: HP_LaserJet
device for HP_LaserJet: ipp://192.168.1.40/ipp/print
device for Canon_Pixma: usb://Canon/PIXMA?serial=1234
";
        let (printers, def) = parse_lpstat_output(sample);
        assert_eq!(def.as_deref(), Some("HP_LaserJet"));
        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].name, "HP_LaserJet");
        assert!(printers[0].is_default);
        assert!(!printers[0].paused);
        assert_eq!(printers[0].uri, "ipp://192.168.1.40/ipp/print");
        assert_eq!(printers[1].name, "Canon_Pixma");
        assert!(printers[1].paused);
        assert_eq!(printers[1].status_label, "En pausa");
    }

    #[test]
    fn parse_scanimage_and_lpinfo_and_queue_sanitization_work() {
        let scan_sample =
            "device `escl:http://192.168.1.40:80' is a HP LaserJet MFP flatbed scanner\n";
        let scanners = parse_scanimage_output(scan_sample);
        assert_eq!(scanners.len(), 1);
        assert_eq!(scanners[0].device_id, "escl:http://192.168.1.40:80");
        assert_eq!(scanners[0].description, "HP LaserJet MFP flatbed scanner");

        let lpinfo_sample = "\
network ipp
direct usb://HP/LaserJet%20Pro?serial=XYZ
network dnssd://Brother%20HL._ipp._tcp.local/
";
        let devices = parse_lpinfo_devices(lpinfo_sample);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].kind, "USB / Local");
        assert_eq!(devices[1].kind, "Red");

        assert_eq!(sanitize_cups_queue_name("HP LaserJet 1020"), "HP_LaserJet_1020");
        assert_eq!(paper_size_to_idx("A4"), 2);
        assert_eq!(idx_to_paper_size(2), "A4");
    }
}
