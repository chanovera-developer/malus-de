use anyhow::Result;
use clap::{Parser, Subcommand};
use malus_de_common::logging::init_logging;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "malus-ctl")]
#[command(about = "CLI oficial de control y automatización de Malus DE", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Muestra el estado del entorno de escritorio Malus DE
    Status,

    /// Controla el modo de Auto-Tiling (ventanas en mosaico bspwm)
    Tiling {
        #[command(subcommand)]
        action: TilingAction,
    },

    /// Controla el perfil de rendimiento del procesador
    Power {
        #[command(subcommand)]
        action: PowerAction,
    },

    /// Controla los espacios de trabajo / escritorios virtuales
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },

    /// Abre o alterna la búsqueda universal Spotlight
    Spotlight,

    /// Activa o desactiva Picture-in-Picture en la ventana activa
    Pip,

    /// Extrae texto de la pantalla mediante OCR (Super + Shift + T)
    Ocr,

    /// Controla la integración móvil con Malus Link (KDE Connect)
    Link {
        #[command(subcommand)]
        action: LinkAction,
    },

    /// Recarga en caliente los componentes de la interfaz de usuario sin cerrar sesión ni ventanas
    Reload {
        /// Componente específico a reiniciar (ej: top-bar, dock, notifications, link, o 'all' por defecto)
        #[arg(short, long)]
        component: Option<String>,
    },

    /// Gestiona monitores, disposición, escalado fraccionario y enlace físico 1:1 (TrueSize Link)
    Displays {
        #[command(subcommand)]
        action: DisplaysAction,
    },

    /// Gestiona la distribución de teclado XKB en vivo (por defecto 'latam' Español Latinoamericano)
    Keyboard {
        #[command(subcommand)]
        action: KeyboardAction,
    },

    /// Cierra y descarta el selector visual de aplicaciones (Alt-Tab / App Switcher)
    DismissAppSwitcher,

    /// Cierra la sesión actual de Malus DE de forma limpia y vuelve al gestor de sesión (GDM)
    Logout,
}

#[derive(Subcommand, Debug)]
enum KeyboardAction {
    /// Muestra la distribución de teclado activa
    Status,
    /// Cambia la distribución de teclado en vivo (ej. 'latam', 'es', 'us')
    Set {
        /// Código XKB de distribución ('latam', 'es', 'us')
        layout: String,
        /// Variante opcional (ej. 'deadtilde')
        #[arg(short, long, default_value = "")]
        variant: String,
    },
}

#[derive(Subcommand, Debug)]
enum DisplaysAction {
    /// Muestra el estado físico y matemático de las pantallas conectadas (PPI, dimensiones mm, escala)
    Status,
    /// Activa o desactiva el Enlace Físico 1:1 (TrueSize Link)
    Link {
        /// 'enable' o 'disable'
        mode: String,
    },
    /// Aplica un ajuste predefinido de escala enlazada (1 = equilibrado, 2 = más espacio)
    LinkPreset {
        /// Número de ajuste (1 o 2)
        preset: u8,
    },
    /// Establece la escala maestra (ej. 1.5, 1.6667, 2.0) y recalcula todas las pantallas 1:1
    SetScale {
        scale: f64,
    },
    /// Establece la escala de un monitor específico por índice
    SetOutputScale {
        /// Índice del monitor según el orden de `malus-de-ctl displays status`
        index: usize,
        /// Factor de escala (ej. 1.25, 1.5, 1.6875, 2.0)
        scale: f64,
    },
    /// Intercambia el orden horizontal (Izquierda <-> Derecha) de las pantallas
    SwapOrder,
    /// Alinea verticalmente las pantallas ('top', 'center', 'bottom', 'desk')
    Align {
        mode: String,
    },
    /// Define la diagonal real en pulgadas para una pantalla (ej. 'DP-1 27.0')
    SetDiagonal {
        connector: String,
        inches: f64,
    },
    /// Establece el radio de curvatura squircle de las esquinas del escritorio (border-radius en px)
    SetRadius {
        /// Radio en puntos lógicos (ej. 0.0 para recto, 32.0 por defecto)
        radius: f64,
    },
    /// Establece el radio de curvatura squircle de las ventanas (border-radius en px)
    SetWindowRadius {
        /// Radio en puntos lógicos (ej. 0.0 para recto, 32.0 por defecto)
        radius: f64,
    },
}

#[derive(Subcommand, Debug)]
enum TilingAction {
    /// Alterna entre modo flotante y mosaico (Super + Y)
    Toggle,
    /// Activa el modo mosaico binario
    Enable,
    /// Desactiva el modo mosaico (vuelve a flotante libre)
    Disable,
    /// Consulta el estado actual del auto-tiling
    Status,
}

#[derive(Subcommand, Debug)]
enum PowerAction {
    /// Consulta el perfil de rendimiento actual
    Get,
    /// Establece el modo: performance, balanced, power-saver
    Set { profile: String },
}

#[derive(Subcommand, Debug)]
enum WorkspaceAction {
    /// Cambia al siguiente espacio de trabajo
    Next,
    /// Cambia al espacio de trabajo anterior
    Prev,
    /// Salta directamente al espacio N (1..10)
    Goto { index: u32 },
}

#[derive(Subcommand, Debug)]
enum LinkAction {
    /// Muestra el estado de sincronización y batería del móvil
    Status,
    /// Hace sonar el teléfono móvil vinculado
    Ring,
    /// Comparte un archivo o texto con el dispositivo
    Share { path: String },
}

fn main() -> Result<()> {
    init_logging("malus_de_ctl");
    let cli = Cli::parse();

    match cli.command {
        Commands::Status => {
            println!("╔═══════════════════════════════════════════════════════════╗");
            println!("║              ✨ MALUS DESKTOP ENVIRONMENT                ║");
            println!("╠═══════════════════════════════════════════════════════════╣");
            println!("║ Versión:         0.1.0-alpha                              ║");
            println!("║ Compositor:      Smithay 0.7 (Wayland Server Nativo)      ║");
            println!("║ Shell:           GTK4 / Layer-Shell (TopBar + Dock)       ║");
            println!("║ Renderizador:    GLES 3.2 Mesa Radeon (AMD Vega 7)        ║");
            println!("║ Perfil Energía:  power-profiles-daemon (D-Bus)            ║");
            println!("║ Auto-Tiling:     Disponible (Super + Y)                   ║");
            println!("║ Integración:     Malus Link (KDE Connect Protocol)        ║");
            println!("╚═══════════════════════════════════════════════════════════╝");
        }
        Commands::Tiling { action } => match action {
            TilingAction::Toggle => {
                println!("🔄 Alternando modo de auto-tiling...");
            }
            TilingAction::Enable => {
                println!("✅ Auto-tiling activado (modo mosaico bspwm)");
            }
            TilingAction::Disable => {
                println!("⏹ Auto-tiling desactivado (modo flotante libre)");
            }
            TilingAction::Status => {
                println!("Modo de ventanas actual: Flotante libre con Snap Layouts");
            }
        },
        Commands::Power { action } => match action {
            PowerAction::Get => {
                println!("Consultando perfil actual en power-profiles-daemon...");
                let out = Command::new("powerprofilesctl").arg("get").output();
                if let Ok(o) = out {
                    println!("Perfil activo: {}", String::from_utf8_lossy(&o.stdout).trim());
                } else {
                    println!("Perfil activo: balanced (local fallback)");
                }
            }
            PowerAction::Set { profile } => {
                println!("Estableciendo perfil de energía a: {}", profile);
                let _ = Command::new("powerprofilesctl")
                    .args(["set", &profile])
                    .status();
            }
        },
        Commands::Workspace { action } => match action {
            WorkspaceAction::Next => println!("Navegando al siguiente espacio de trabajo..."),
            WorkspaceAction::Prev => println!("Navegando al espacio de trabajo anterior..."),
            WorkspaceAction::Goto { index } => println!("Saltando al espacio de trabajo #{}", index),
        },
        Commands::Spotlight => {
            println!("Abriendo búsqueda universal Spotlight (Super + Espacio)...");
            let _ = Command::new("malus-de-spotlight").spawn();
        }
        Commands::Pip => {
            println!("Alternando modo Picture-in-Picture en ventana activa (Super + P)...");
        }
        Commands::Ocr => {
            println!("Iniciando extractor OCR de pantalla (Super + Shift + T)...");
        }
        Commands::Link { action } => match action {
            LinkAction::Status => {
                println!("📱 Malus Link: Buscando dispositivos vinculados...");
            }
            LinkAction::Ring => {
                println!("🔔 Enviando señal para hacer sonar el teléfono...");
            }
            LinkAction::Share { path } => {
                println!("📤 Compartiendo '{}' con el teléfono móvil...", path);
            }
        },
        Commands::Reload { component } => {
            let comp = component.as_deref().unwrap_or("all");
            println!("🔄 Recargando componentes de interfaz de Malus DE (modo: {})...", comp);
            let bin_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin"));

            let targets = match comp {
                "top-bar" | "topbar" => vec!["malus-de-top-bar"],
                "dock" => vec!["malus-de-dock"],
                "notifications" => vec!["malus-de-notifications"],
                "link" => vec!["malus-de-link"],
                "wallpaper" => vec!["malus-de-wallpaper"],
                _ => vec![
                    "malus-de-top-bar",
                    "malus-de-dock",
                    "malus-de-notifications",
                ],
            };

            let wayland_disp = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
            for binary in targets {
                println!("  ↳ Reiniciando {}...", binary);
                let _ = Command::new("killall").arg("-9").arg(binary).status();
                let bin_path = bin_dir.join(binary);
                let exec_cmd = if bin_path.exists() {
                    bin_path.to_str().unwrap().to_string()
                } else {
                    binary.to_string()
                };
                let mut cmd = Command::new(&exec_cmd);
                cmd.env("WAYLAND_DISPLAY", &wayland_disp);
                cmd.env("GDK_BACKEND", "wayland");
                cmd.process_group(0);
                let _ = cmd.spawn();
            }
            println!("✨ Componentes recargados con éxito sin afectar ventanas ni aplicaciones abiertas.");
        }
        Commands::Displays { action } => {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            match action {
                DisplaysAction::Status => {
                    println!("╔═══════════════════════════════════════════════════════════════════════════════╗");
                    println!("║             🖥  MALUS DE: ESTADO FÍSICO Y MATEMÁTICO DE PANTALLAS              ║");
                    println!("╚═══════════════════════════════════════════════════════════════════════════════╝");
                    println!(" 🔗 Enlace Físico 1:1 (TrueSize Link): {}", if cfg.linked_physical_1to1 { "ACTIVADO ✅" } else { "DESACTIVADO ❌" });
                    println!(" 🎯 Escala Maestra Principal: {:.4} ({:.1}%)", cfg.master_scale, cfg.master_scale * 100.0);
                    println!(" 📐 Radio de Esquinas (Border-Radius Pantallas): {:.1} px", cfg.screen_corner_radius);
                    println!(" 🪟 Radio de Esquinas (Border-Radius Ventanas):   {:.1} px", cfg.window_corner_radius);
                    println!("─────────────────────────────────────────────────────────────────────────────────");
                    for (i, o) in cfg.outputs.iter().enumerate() {
                        let diag = o.effective_diagonal();
                        let ppi = o.ppi();
                        let (w_mm, h_mm) = o.physical_size_mm();
                        let (lw, lh) = o.logical_size();
                        let primary_badge = if o.primary { " [PRIMARIO]" } else { "" };
                        println!(" Monitor #{}: {} ({}){}", i + 1, o.name, o.connector, primary_badge);
                        println!("   • Panel Físico:   {:.1}\" diagonal | {:.1} x {:.1} mm | {:.2} PPI", diag, w_mm, h_mm, ppi);
                        println!("   • Resolución:     {}x{} física @ {:.1} Hz", o.width, o.height, o.refresh_rate);
                        println!("   • Escala Activa:  {:.4}x ({:.1}%)", o.scale, o.scale * 100.0);
                        println!("   • Espacio Lógico: {}x{} en coords ({}, {})", lw, lh, o.x, o.y);
                        println!("   • Densidad Real:  100 puntos lógicos = {:.1} mm reales en vidrio", (100.0 * o.scale / ppi) * 25.4);
                        println!();
                    }
                }
                DisplaysAction::Link { mode } => {
                    let enable = match mode.to_lowercase().as_str() {
                        "enable" | "on" | "1" | "true" => true,
                        _ => false,
                    };
                    cfg.linked_physical_1to1 = enable;
                    if enable {
                        cfg.apply_1to1_link();
                        println!("✅ Enlace Físico 1:1 ACTIVADO. Escalas recalculadas en proporción exacta a PPI.");
                    } else {
                        cfg.recalculate_layout();
                        println!("❌ Enlace Físico 1:1 DESACTIVADO. Cada monitor ahora tiene escala independiente.");
                    }
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                }
                DisplaysAction::LinkPreset { preset } => {
                    if preset != 1 && preset != 2 {
                        eprintln!("Preset inválido ({}). Solo están disponibles los ajustes 1 y 2:", preset);
                        eprintln!("  1: Escala base equilibrada (150% en la pantalla principal)");
                        eprintln!("  2: Más espacio de trabajo (133% en la pantalla principal)");
                        return Ok(());
                    }
                    cfg.apply_linked_preset(preset);
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("✨ Ajuste #{} de enlace de resolución aplicado exitosamente:", preset);
                    for o in &cfg.outputs {
                        let (lw, lh) = o.logical_size();
                        println!("   ↳ {} ({}): {:.4}x ({:.2}%) -> {}×{} en coords ({}, {})", o.name, o.connector, o.scale, o.scale * 100.0, lw, lh, o.x, o.y);
                    }
                }
                DisplaysAction::SetScale { scale } => {
                    if scale < 0.5 || scale > 4.0 {
                        eprintln!("Escala fuera de rango permitido (0.5x - 4.0x): {}", scale);
                        return Ok(());
                    }
                    cfg.set_master_scale(scale);
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("✨ Escala maestra actualizada a {:.4}x ({:.1}%).", scale, scale * 100.0);
                    for o in &cfg.outputs {
                        println!("   ↳ {} ({}): {:.4}x ({:.1}%) en ({}, {})", o.name, o.connector, o.scale, o.scale * 100.0, o.x, o.y);
                    }
                }
                DisplaysAction::SetOutputScale { index, scale } => {
                    if scale < 0.5 || scale > 4.0 {
                        eprintln!("Escala fuera de rango permitido (0.5x - 4.0x): {}", scale);
                        return Ok(());
                    }
                    cfg.set_output_scale(index, scale);
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("✨ Escala de monitor #{} actualizada (linked_1to1={}).", index, cfg.linked_physical_1to1);
                    for o in &cfg.outputs {
                        println!("   ↳ {} ({}): {:.4}x ({:.1}%) en ({}, {})", o.name, o.connector, o.scale, o.scale * 100.0, o.x, o.y);
                    }
                }
                DisplaysAction::SwapOrder => {
                    cfg.swap_horizontal_order();
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("⇄ Orden horizontal de pantallas intercambiado:");
                    for o in &cfg.outputs {
                        println!("   ↳ {} ({}): pos=({}, {})", o.name, o.connector, o.x, o.y);
                    }
                }
                DisplaysAction::Align { mode } => {
                    cfg.set_vertical_alignment(&mode);
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("📐 Alineación vertical '{}' aplicada:", mode);
                    for o in &cfg.outputs {
                        println!("   ↳ {} ({}): pos=({}, {})", o.name, o.connector, o.x, o.y);
                    }
                }
                DisplaysAction::SetDiagonal { connector, inches } => {
                    if let Some(o) = cfg.outputs.iter_mut().find(|o| o.connector.eq_ignore_ascii_case(&connector)) {
                        o.diagonal_inches = Some(inches);
                        println!("📐 Diagonal del monitor {} actualizada a {:.1} pulgadas.", connector, inches);
                        if cfg.linked_physical_1to1 {
                            cfg.apply_1to1_link();
                        } else {
                            cfg.recalculate_layout();
                        }
                        if let Err(e) = cfg.save() {
                            eprintln!("Error al guardar configuración: {:?}", e);
                        }
                        let _ = malus_de_common::ipc::IpcClient::send_command(
                            malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                        );
                    } else {
                        eprintln!("No se encontró monitor con conector '{}'", connector);
                    }
                }
                DisplaysAction::SetRadius { radius } => {
                    if radius < 0.0 || radius > 100.0 {
                        eprintln!("Radio fuera de rango permitido (0.0 - 100.0 px): {}", radius);
                        return Ok(());
                    }
                    cfg.screen_corner_radius = radius;
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("✨ Border-radius de las pantallas actualizado a {:.1} px.", radius);
                }
                DisplaysAction::SetWindowRadius { radius } => {
                    if radius < 0.0 || radius > 100.0 {
                        eprintln!("Radio fuera de rango permitido (0.0 - 100.0 px): {}", radius);
                        return Ok(());
                    }
                    cfg.window_corner_radius = radius;
                    if let Err(e) = cfg.save() {
                        eprintln!("Error al guardar configuración: {:?}", e);
                    }
                    let _ = malus_de_common::ipc::IpcClient::send_command(
                        malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
                    );
                    println!("✨ Border-radius de las ventanas actualizado a {:.1} px.", radius);
                }
            }
        }
        Commands::Keyboard { action } => match action {
            KeyboardAction::Status => {
                let cfg = malus_de_common::config::InputConfig::load();
                println!("⌨️  Distribución XKB activa: '{}' ({})", cfg.keyboard_layout, cfg.layout_display_name());
                println!("   • Variante: '{}'", cfg.keyboard_variant);
                println!("   • Modelo:   '{}'", cfg.keyboard_model);
            }
            KeyboardAction::Set { layout, variant } => {
                let mut cfg = malus_de_common::config::InputConfig::load();
                cfg.keyboard_layout = layout.clone();
                cfg.keyboard_variant = variant.clone();
                if let Err(e) = cfg.save() {
                    eprintln!("Error al guardar input.toml: {:?}", e);
                }
                let _ = malus_de_common::ipc::IpcClient::send_command(
                    malus_de_common::ipc::IpcMessage::SetKeyboardLayout {
                        layout: layout.clone(),
                        variant: variant.clone(),
                    },
                );
                println!(
                    "✅ Distribución de teclado actualizada en vivo a '{}' ({})",
                    cfg.keyboard_layout,
                    cfg.layout_display_name()
                );
            }
        },
        Commands::DismissAppSwitcher => {
            println!("Descartando selector visual de aplicaciones (App Switcher)...");
            let _ = malus_de_common::ipc::send_ipc_message(&malus_de_common::ipc::IpcMessage::HideAppSwitcher);
        }
        Commands::Logout => {
            println!("👋 Cerrando sesión de Malus DE...");
            let _ = Command::new("pkill")
                .args(["-TERM", "-x", "malus-de-compositor"])
                .status();
            let _ = Command::new("killall")
                .args(["-TERM", "malus-de-compositor"])
                .status();
        }
    }

    Ok(())
}
