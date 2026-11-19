use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tracing::{info, warn};

pub const EVERYONE_DURATION_SECS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirDropMode {
    Off,
    ContactsOnly,
    Everyone,
}

impl AirDropMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AirDropMode::Off => "off",
            AirDropMode::ContactsOnly => "contacts",
            AirDropMode::Everyone => "everyone",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "everyone" | "todos" | "on" => AirDropMode::Everyone,
            "contacts" | "contactos" => AirDropMode::ContactsOnly,
            _ => AirDropMode::Off,
        }
    }

    #[allow(dead_code)]
    pub fn display_name(&self) -> &'static str {
        match self {
            AirDropMode::Off => "Desactivado",
            AirDropMode::ContactsOnly => "Solo contactos",
            AirDropMode::Everyone => "Todos (10 min)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OwlyMissingItem {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub say: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OwlyDoctorReport {
    #[serde(default)]
    pub hardware: bool,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub station: Option<String>,
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub channel: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub missing: Vec<OwlyMissingItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AirDropPeer {
    pub id: String,
    pub name: String,
    pub model: String,
    pub address: String,
    pub via_discover: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirDropStatusSnapshot {
    pub mode: String,
    pub radio_state: String,
    pub remaining_seconds: u32,
    pub backend_installed: bool,
    pub hardware_ready: bool,
    pub radio_mode: Option<String>,
    pub station: Option<String>,
    pub driver: Option<String>,
    pub device_pci: Option<String>,
    pub channel: Option<u32>,
    pub hardware_status: Option<String>,
    pub device_name: String,
    pub downloads_dir: String,
    pub last_error: Option<String>,
    pub issues: Vec<String>,
    pub missing: Vec<String>,
    pub peers: Vec<AirDropPeer>,
    pub last_received_files: Vec<String>,
}

pub struct AirDropEngine {
    pub mode: AirDropMode,
    pub everyone_until: Option<Instant>,
    pub radio_state: String,
    pub last_error: Option<String>,
    pub discovered_peers: Vec<AirDropPeer>,
    pub last_received_files: Vec<String>,
    pub device_name: String,
    pub downloads_dir: PathBuf,
    generation: u64,
    radio_child: Option<Child>,
    receiver_child: Option<Child>,
    doctor_cache: Option<(Instant, bool, Option<OwlyDoctorReport>)>,
}

impl AirDropEngine {
    pub fn new() -> Self {
        let downloads_dir = resolve_downloads_dir();
        let _ = std::fs::create_dir_all(&downloads_dir);
        let device_name = resolve_computer_name();
        Self {
            mode: AirDropMode::Off,
            everyone_until: None,
            radio_state: "off".to_string(),
            last_error: None,
            discovered_peers: Vec::new(),
            last_received_files: Vec::new(),
            device_name,
            downloads_dir,
            generation: 0,
            radio_child: None,
            receiver_child: None,
            doctor_cache: None,
        }
    }

    pub fn remaining_seconds(&self) -> u32 {
        if self.mode == AirDropMode::Everyone {
            if let Some(until) = self.everyone_until {
                let now = Instant::now();
                if until > now {
                    return until.duration_since(now).as_secs() as u32;
                }
            }
        }
        0
    }

    pub fn refresh_expiration_if_needed(&mut self) -> bool {
        if self.mode == AirDropMode::Everyone {
            if let Some(until) = self.everyone_until {
                if Instant::now() >= until {
                    info!("⏳ [AirDrop] Visibilidad temporal 'Todos' (10 min) expirada; desactivando radio AWDL y restaurando Wi-Fi.");
                    self.stop_processes_locked();
                    self.mode = AirDropMode::Off;
                    self.everyone_until = None;
                    self.radio_state = "off".to_string();
                    self.discovered_peers.clear();
                    notify_airdrop(
                        "AirDrop desactivado",
                        "Terminó la ventana de 10 minutos. Wi-Fi restaurado.",
                    );
                    return true;
                }
            }
        }
        false
    }

    pub fn stop_processes_locked(&mut self) {
        if let Some(mut recv) = self.receiver_child.take() {
            terminate_child_gracefully(&mut recv, Duration::from_millis(1200));
        }
        if let Some(mut radio) = self.radio_child.take() {
            terminate_child_gracefully(&mut radio, Duration::from_millis(3500));
        }
    }

    pub fn doctor_report(&mut self, force: bool) -> (bool, Option<OwlyDoctorReport>) {
        if !force {
            if let Some((ts, installed, ref cached)) = self.doctor_cache {
                if ts.elapsed() < Duration::from_secs(30) {
                    return (installed, cached.clone());
                }
            }
        }
        let installed = find_owly_binary().is_some();
        let report = if installed {
            run_owly_doctor()
        } else {
            None
        };
        self.doctor_cache = Some((Instant::now(), installed, report.clone()));
        (installed, report)
    }

    pub fn status_snapshot(&mut self, force_doctor: bool) -> AirDropStatusSnapshot {
        self.refresh_expiration_if_needed();
        let (backend_installed, report) = self.doctor_report(force_doctor);
        let (hardware_ready, radio_mode, station, driver, device_pci, channel, hardware_status, issues, missing) =
            if let Some(rep) = report {
                let ready = rep.hardware && rep.missing.is_empty();
                (
                    ready,
                    rep.mode,
                    rep.station,
                    rep.driver,
                    rep.device,
                    rep.channel,
                    rep.status,
                    rep.issues,
                    rep.missing.into_iter().map(|m| m.say).collect(),
                )
            } else {
                (
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Vec::new(),
                    if backend_installed {
                        vec!["No se pudo consultar owlyd (verifica que owlyd esté instalado en el bus del sistema).".to_string()]
                    } else {
                        vec!["El binario 'owly' no está instalado en el sistema.".to_string()]
                    },
                )
            };

        AirDropStatusSnapshot {
            mode: self.mode.as_str().to_string(),
            radio_state: self.radio_state.clone(),
            remaining_seconds: self.remaining_seconds(),
            backend_installed,
            hardware_ready,
            radio_mode,
            station,
            driver,
            device_pci,
            channel,
            hardware_status,
            device_name: self.device_name.clone(),
            downloads_dir: self.downloads_dir.display().to_string(),
            last_error: self.last_error.clone(),
            issues,
            missing,
            peers: self.discovered_peers.clone(),
            last_received_files: self.last_received_files.clone(),
        }
    }
}

impl Drop for AirDropEngine {
    fn drop(&mut self) {
        self.stop_processes_locked();
    }
}

pub fn set_airdrop_mode(engine: &Arc<Mutex<AirDropEngine>>, mode: AirDropMode) -> bool {
    let (generation, was_active, device_name, downloads_dir, radio_mode) = {
        let mut st = engine.lock().unwrap();
        let prev_active = st.mode == AirDropMode::Everyone
            || st.radio_state == "active"
            || st.radio_state == "starting";
        st.generation = st.generation.wrapping_add(1);
        let gen = st.generation;
        st.stop_processes_locked();
        st.last_error = None;
        st.discovered_peers.clear();

        match mode {
            AirDropMode::Off => {
                st.mode = AirDropMode::Off;
                st.everyone_until = None;
                st.radio_state = "off".to_string();
                info!("📡 [AirDrop] Desactivado (radio AWDL detenido)");
                if prev_active {
                    notify_airdrop(
                        "AirDrop desactivado",
                        "Radio AWDL detenido y conexión Wi-Fi restaurada.",
                    );
                }
                return true;
            }
            AirDropMode::ContactsOnly => {
                st.mode = AirDropMode::ContactsOnly;
                st.everyone_until = None;
                st.radio_state = "contacts_unsupported".to_string();
                st.last_error = Some(
                    "El modo 'Solo contactos' requiere verificación de Apple ID. Selecciona 'Todos durante 10 min' para recibir archivos desde tu MacBook o iPhone."
                        .to_string(),
                );
                info!("📡 [AirDrop] Modo 'Solo contactos' seleccionado (radio AWDL inactivo para conservar Wi-Fi)");
                if prev_active {
                    notify_airdrop(
                        "AirDrop: Solo contactos",
                        "Para recibir archivos desde Mac/iPhone en Linux usa 'Todos durante 10 min'. Wi-Fi restaurado.",
                    );
                }
                return true;
            }
            AirDropMode::Everyone => {
                let (installed, report) = st.doctor_report(true);
                if !installed {
                    st.mode = AirDropMode::Off;
                    st.everyone_until = None;
                    st.radio_state = "error".to_string();
                    st.last_error = Some("No se encontró el binario 'owly' en el sistema.".to_string());
                    notify_airdrop("Error de AirDrop", "No se encontró el motor 'owly' instalado.");
                    return false;
                }
                if let Some(ref rep) = report {
                    if !rep.hardware || !rep.missing.is_empty() {
                        let reason = rep
                            .missing
                            .first()
                            .map(|m| m.say.clone())
                            .unwrap_or_else(|| "Tarjeta Wi-Fi no compatible con AWDL.".to_string());
                        st.mode = AirDropMode::Off;
                        st.everyone_until = None;
                        st.radio_state = "error".to_string();
                        st.last_error = Some(reason.clone());
                        notify_airdrop("AirDrop no disponible", &reason);
                        return false;
                    }
                }
                st.mode = AirDropMode::Everyone;
                st.everyone_until = Some(Instant::now() + Duration::from_secs(EVERYONE_DURATION_SECS));
                st.radio_state = "starting".to_string();
                let rmode = report.and_then(|r| r.mode);
                (
                    gen,
                    prev_active,
                    st.device_name.clone(),
                    st.downloads_dir.clone(),
                    rmode,
                )
            }
        }
    };

    let _ = was_active;
    let Some(owly_bin) = find_owly_binary() else {
        return false;
    };

    info!(
        "📡 [AirDrop] Iniciando radio AWDL con {:?} (--for {}s) como '{}'...",
        owly_bin, EVERYONE_DURATION_SECS, device_name
    );

    let radio_child = match Command::new(&owly_bin)
        .args(["radio", "up", "--for", &EVERYONE_DURATION_SECS.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            let msg = format!("No se pudo iniciar 'owly radio up': {}", err);
            warn!("⚠️ [AirDrop] {}", msg);
            let mut st = engine.lock().unwrap();
            if st.generation == generation {
                st.mode = AirDropMode::Off;
                st.everyone_until = None;
                st.radio_state = "error".to_string();
                st.last_error = Some(msg.clone());
            }
            notify_airdrop("Error iniciando AirDrop", &msg);
            return false;
        }
    };

    {
        let mut st = engine.lock().unwrap();
        if st.generation != generation {
            let mut stale = radio_child;
            terminate_child_gracefully(&mut stale, Duration::from_secs(2));
            return false;
        }
        st.radio_child = Some(radio_child);
    }

    // Hilo supervisor que espera awdl0, arranca owly receive, notifica archivos entrantes y vigila expiración
    let engine_sup = Arc::clone(engine);
    let owly_sup = owly_bin.clone();
    std::thread::spawn(move || {
        let ready = wait_for_awdl0_ready(&engine_sup, generation, Duration::from_secs(18));
        if let Err(err_msg) = ready {
            warn!("⚠️ [AirDrop] Fallo al levantar awdl0: {}", err_msg);
            let mut st = engine_sup.lock().unwrap();
            if st.generation == generation {
                st.stop_processes_locked();
                st.mode = AirDropMode::Off;
                st.everyone_until = None;
                st.radio_state = "error".to_string();
                st.last_error = Some(err_msg.clone());
                notify_airdrop("No se pudo activar AirDrop", &err_msg);
            }
            return;
        }

        // Arrancar receptor AirDrop en ~/Descargas (o ~/Downloads)
        let mut recv_cmd = Command::new(&owly_sup);
        recv_cmd
            .args([
                "receive",
                "--dir",
                &downloads_dir.to_string_lossy(),
                "--name",
                &device_name,
                "--accept",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut recv_child = match recv_cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                let msg = format!("No se pudo iniciar 'owly receive': {}", err);
                warn!("⚠️ [AirDrop] {}", msg);
                let mut st = engine_sup.lock().unwrap();
                if st.generation == generation {
                    st.stop_processes_locked();
                    st.mode = AirDropMode::Off;
                    st.everyone_until = None;
                    st.radio_state = "error".to_string();
                    st.last_error = Some(msg.clone());
                    notify_airdrop("Error en receptor AirDrop", &msg);
                }
                return;
            }
        };

        let recv_stdout = recv_child.stdout.take();
        let recv_stderr = recv_child.stderr.take();

        {
            let mut st = engine_sup.lock().unwrap();
            if st.generation != generation {
                terminate_child_gracefully(&mut recv_child, Duration::from_secs(1));
                return;
            }
            st.receiver_child = Some(recv_child);
            st.radio_state = "active".to_string();
            st.last_error = None;
        }

        let mode_hint = if radio_mode.as_deref() == Some("exclusive") {
            " (modo exclusivo: Wi-Fi pausado temporalmente)"
        } else {
            ""
        };
        info!(
            "✅ [AirDrop] Activo en awdl0 como '{}', guardando archivos en {:?}{}",
            device_name, downloads_dir, mode_hint
        );
        notify_airdrop(
            "AirDrop activo (10 min)",
            &format!(
                "Visible como '{}' para MacBook/iPhone{}.\nGuardando en {}",
                device_name,
                mode_hint,
                downloads_dir.display()
            ),
        );

        // Hilos de lectura de logs de owly receive
        if let Some(stdout) = recv_stdout {
            let eng_out = Arc::clone(&engine_sup);
            let dir_out = downloads_dir.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        info!("📥 [owly receive] {}", trimmed);
                        if let Some(item) = parse_owly_receive_line(trimmed) {
                            record_received_item(&eng_out, generation, &item, &dir_out);
                        }
                    }
                }
            });
        }
        if let Some(stderr) = recv_stderr {
            let eng_err = Arc::clone(&engine_sup);
            let dir_err = downloads_dir.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        info!("📥 [owly receive stderr] {}", trimmed);
                        if let Some(item) = parse_owly_receive_line(trimmed) {
                            record_received_item(&eng_err, generation, &item, &dir_err);
                        }
                    }
                }
            });
        }

        // Búsqueda inicial en segundo plano para descubrir MacBook/iPhone cercanos
        let eng_scan = Arc::clone(&engine_sup);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            let active = {
                let st = eng_scan.lock().unwrap();
                st.generation == generation && st.radio_state == "active"
            };
            if active {
                let _ = scan_airdrop_peers(&eng_scan, 6);
            }
        });

        // Vigilancia de archivos nuevos en downloads_dir y estado de los procesos hijos
        let mut known_files = snapshot_dir_files(&downloads_dir);
        loop {
            std::thread::sleep(Duration::from_secs(2));

            let should_continue = {
                let mut st = engine_sup.lock().unwrap();
                if st.generation != generation {
                    false
                } else if st.refresh_expiration_if_needed() {
                    false
                } else {
                    // Verificar si owly radio up finalizó inesperadamente o cumplió su --for
                    let radio_exited = st
                        .radio_child
                        .as_mut()
                        .and_then(|c| c.try_wait().ok().flatten())
                        .is_some();
                    if radio_exited {
                        info!("📡 [AirDrop] Sesión de radio AWDL finalizada; cerrando receptor.");
                        st.stop_processes_locked();
                        st.mode = AirDropMode::Off;
                        st.everyone_until = None;
                        st.radio_state = "off".to_string();
                        st.discovered_peers.clear();
                        notify_airdrop(
                            "AirDrop finalizado",
                            "El radio AWDL se cerró y tu Wi-Fi fue restaurado.",
                        );
                        false
                    } else {
                        true
                    }
                }
            };

            if !should_continue {
                break;
            }

            // Detectar archivos recién guardados en Descargas mientras AirDrop escucha
            let current_files = snapshot_dir_files(&downloads_dir);
            for (name, mtime) in &current_files {
                let is_new = match known_files.get(name) {
                    None => true,
                    Some(prev) => mtime > prev,
                };
                if is_new && !name.starts_with('.') {
                    record_received_item(&engine_sup, generation, name, &downloads_dir);
                }
            }
            known_files = current_files;
        }
    });

    true
}

pub fn scan_airdrop_peers(
    engine: &Arc<Mutex<AirDropEngine>>,
    seconds: u32,
) -> Result<Vec<AirDropPeer>, String> {
    let Some(owly_bin) = find_owly_binary() else {
        return Err("El binario 'owly' no está instalado.".to_string());
    };

    let (device_name, radio_ready) = {
        let mut st = engine.lock().unwrap();
        st.refresh_expiration_if_needed();
        let ready = st.radio_state == "active" || awdl0_has_link_local();
        (st.device_name.clone(), ready)
    };

    if !radio_ready {
        return Err(
            "Activa AirDrop en modo 'Todos durante 10 min' primero para levantar la interfaz awdl0."
                .to_string(),
        );
    }

    let browse_secs = seconds.clamp(3, 25);
    info!(
        "🔍 [AirDrop] Buscando dispositivos Apple cercanos durante {}s...",
        browse_secs
    );

    let browse_out = Command::new(&owly_bin)
        .args([
            "browse",
            "--name",
            &device_name,
            "--seconds",
            &browse_secs.to_string(),
        ])
        .output()
        .map_err(|e| format!("Error ejecutando 'owly browse': {}", e))?;

    let stdout = String::from_utf8_lossy(&browse_out.stdout);
    let stderr = String::from_utf8_lossy(&browse_out.stderr);
    let mut peers = parse_owly_browse_output(&stdout);

    // También consultar `owly radio peers` como respaldo si algún peer AWDL aún no respondió /Discover
    if let Ok(radio_peers_out) = Command::new(&owly_bin).args(["radio", "peers"]).output() {
        let rp_stdout = String::from_utf8_lossy(&radio_peers_out.stdout);
        let raw_peers = parse_owly_radio_peers_output(&rp_stdout);
        for rp in raw_peers {
            let already = peers.iter().any(|p| {
                (!p.address.is_empty() && p.address.eq_ignore_ascii_case(&rp.address))
                    || p.id.eq_ignore_ascii_case(&rp.id)
            });
            if !already {
                peers.push(rp);
            }
        }
    }

    if peers.is_empty() && !stderr.trim().is_empty() {
        info!("🔍 [AirDrop browse] {}", stderr.trim());
    } else {
        info!("🔍 [AirDrop] {} dispositivo(s) encontrado(s)", peers.len());
    }

    {
        let mut st = engine.lock().unwrap();
        st.discovered_peers = peers.clone();
    }

    Ok(peers)
}

pub fn send_airdrop_file(
    engine: &Arc<Mutex<AirDropEngine>>,
    target: &str,
    path: &str,
) -> Result<String, String> {
    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(format!("El archivo no existe: {}", path));
    }
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();

    send_airdrop_payload(engine, target, &file_name, |cmd| {
        cmd.arg(path);
    })
}

pub fn send_airdrop_url(
    engine: &Arc<Mutex<AirDropEngine>>,
    target: &str,
    url: &str,
) -> Result<String, String> {
    if url.trim().is_empty() {
        return Err("La URL está vacía.".to_string());
    }
    send_airdrop_payload(engine, target, url, |cmd| {
        cmd.args(["--url", url]);
    })
}

fn send_airdrop_payload<F>(
    engine: &Arc<Mutex<AirDropEngine>>,
    target: &str,
    label: &str,
    configure_payload: F,
) -> Result<String, String>
where
    F: FnOnce(&mut Command),
{
    let Some(owly_bin) = find_owly_binary() else {
        return Err("El binario 'owly' no está instalado.".to_string());
    };

    let (device_name, need_temp_radio) = {
        let mut st = engine.lock().unwrap();
        st.refresh_expiration_if_needed();
        let active = st.radio_state == "active" || awdl0_has_link_local();
        (st.device_name.clone(), !active)
    };

    let mut temp_radio: Option<Child> = None;
    if need_temp_radio {
        info!("📡 [AirDrop] Levantando radio AWDL temporal (120s) para enviar '{}'...", label);
        notify_airdrop(
            "AirDrop: iniciando envío",
            &format!("Activando radio AWDL temporal para enviar '{}'...", label),
        );
        let mut child = Command::new(&owly_bin)
            .args(["radio", "up", "--for", "120"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("No se pudo iniciar 'owly radio up': {}", e))?;

        let deadline = Instant::now() + Duration::from_secs(18);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                let err_txt = read_child_stderr(&mut child);
                return Err(format!(
                    "El radio AWDL se detuvo antes de estar listo ({}): {}",
                    status, err_txt
                ));
            }
            if awdl0_has_link_local() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !awdl0_has_link_local() {
            terminate_child_gracefully(&mut child, Duration::from_secs(2));
            return Err("Tiempo de espera agotado levantando la interfaz awdl0.".to_string());
        }
        temp_radio = Some(child);
    } else {
        notify_airdrop(
            "AirDrop: enviando",
            &format!(
                "Enviando '{}'{}...",
                label,
                if target.trim().is_empty() || target == "auto" {
                    String::new()
                } else {
                    format!(" a {}", target)
                }
            ),
        );
    }

    let mut cmd = Command::new(&owly_bin);
    cmd.args(["send", "--name", &device_name, "--seconds", "30"]);
    let trimmed_target = target.trim();
    if !trimmed_target.is_empty() && !trimmed_target.eq_ignore_ascii_case("auto") {
        cmd.args(["--to", trimmed_target]);
    }
    configure_payload(&mut cmd);

    let output = cmd.output();

    if let Some(mut child) = temp_radio {
        terminate_child_gracefully(&mut child, Duration::from_secs(3));
    }

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let msg = if stdout.is_empty() {
                format!("'{}' enviado correctamente por AirDrop.", label)
            } else {
                stdout
            };
            info!("✅ [AirDrop] {}", msg);
            notify_airdrop("AirDrop enviado", &msg);
            Ok(msg)
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let detail = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("owly send terminó con código {}", out.status)
            };
            warn!("⚠️ [AirDrop] Error enviando '{}': {}", label, detail);
            notify_airdrop("Error al enviar por AirDrop", &detail);
            Err(detail)
        }
        Err(err) => {
            let detail = format!("Error ejecutando 'owly send': {}", err);
            warn!("⚠️ [AirDrop] {}", detail);
            notify_airdrop("Error al enviar por AirDrop", &detail);
            Err(detail)
        }
    }
}

fn wait_for_awdl0_ready(
    engine: &Arc<Mutex<AirDropEngine>>,
    generation: u64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        {
            let mut st = engine.lock().unwrap();
            if st.generation != generation {
                return Err("Activación cancelada.".to_string());
            }
            if let Some(ref mut child) = st.radio_child {
                if let Ok(Some(status)) = child.try_wait() {
                    let err_txt = read_child_stderr(child);
                    return Err(if err_txt.is_empty() {
                        format!("owly radio up terminó prematuramente ({})", status)
                    } else {
                        err_txt
                    });
                }
            }
        }

        if awdl0_has_link_local() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("La interfaz awdl0 no obtuvo dirección IPv6 link-local a tiempo.".to_string())
}

fn read_child_stderr(child: &mut Child) -> String {
    use std::io::Read;
    let mut s = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut s);
    }
    s.trim().to_string()
}

pub fn awdl0_has_link_local() -> bool {
    if !Path::new("/sys/class/net/awdl0").exists() {
        return false;
    }
    if let Ok(out) = Command::new("ip")
        .args(["-6", "-o", "addr", "show", "dev", "awdl0", "scope", "link"])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("fe80:") {
                return true;
            }
        }
    }
    false
}

fn record_received_item(
    engine: &Arc<Mutex<AirDropEngine>>,
    generation: u64,
    item_name: &str,
    downloads_dir: &Path,
) {
    let clean = item_name.trim();
    if clean.is_empty() {
        return;
    }
    let should_notify = {
        let mut st = engine.lock().unwrap();
        if st.generation != generation {
            return;
        }
        if st.last_received_files.last().map(String::as_str) == Some(clean) {
            false
        } else {
            st.last_received_files.push(clean.to_string());
            if st.last_received_files.len() > 20 {
                st.last_received_files.remove(0);
            }
            true
        }
    };
    if should_notify {
        info!("✅ [AirDrop] Archivo recibido: {}", clean);
        notify_airdrop(
            "AirDrop recibido",
            &format!("'{}' guardado en {}", clean, downloads_dir.display()),
        );
    }
}

fn snapshot_dir_files(dir: &Path) -> HashMap<String, SystemTime> {
    let mut map = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    map.insert(name, mtime);
                }
            }
        }
    }
    map
}

fn terminate_child_gracefully(child: &mut Child, grace: Duration) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    let pid = child.id();
    // Enviar SIGINT primero (como Ctrl-C) para que owly libere el lease en owlyd limpiamente
    let _ = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status();

    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn find_owly_binary() -> Option<PathBuf> {
    for candidate in ["/usr/bin/owly", "/usr/local/bin/owly"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let p = dir.join("owly");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn run_owly_doctor() -> Option<OwlyDoctorReport> {
    let bin = find_owly_binary()?;
    let out = Command::new(bin).args(["doctor", "--json"]).output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<OwlyDoctorReport>(&stdout).ok()
}

pub fn resolve_downloads_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    // Respetar XDG_DOWNLOAD_DIR desde ~/.config/user-dirs.dirs si existe
    let user_dirs = home.join(".config/user-dirs.dirs");
    if let Ok(content) = std::fs::read_to_string(&user_dirs) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("XDG_DOWNLOAD_DIR=") {
                let unquoted = rest.trim_matches('"');
                let expanded = unquoted.replace("$HOME", &home.to_string_lossy());
                let p = PathBuf::from(expanded);
                if !p.as_os_str().is_empty() {
                    return p;
                }
            }
        }
    }

    let es_dir = home.join("Descargas");
    if es_dir.exists() {
        return es_dir;
    }
    home.join("Downloads")
}

pub fn resolve_computer_name() -> String {
    if let Ok(raw) = std::fs::read_to_string("/etc/hostname") {
        let name = raw
            .trim()
            .split('.')
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() && name != "localhost" {
            return name.to_string();
        }
    }
    "Malus DE".to_string()
}

pub fn notify_airdrop(summary: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args([
            "--app-name=AirDrop",
            "--icon=network-wireless-hotspot-symbolic",
            summary,
            body,
        ])
        .spawn();
}

/// Parsea líneas de salida de `owly browse` para extraer receptores AirDrop descubiertos.
pub fn parse_owly_browse_output(stdout: &str) -> Vec<AirDropPeer> {
    let mut peers = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.starts_with("no ")
            || lower.starts_with("trying ")
            || lower.starts_with("not browsing")
            || lower.starts_with("not trying")
            || lower.starts_with("waiting ")
            || lower.starts_with("browsing ")
            || lower.contains("/discover failed")
        {
            continue;
        }

        // Extraer dirección MAC/AWDL o IPv6 si aparece en la línea
        let address = extract_peer_address(line).unwrap_or_default();

        // Separación por tabuladores o dobles espacios (formato tabla común de CLI)
        let columns: Vec<&str> = if line.contains('\t') {
            line.split('\t')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            line.split("  ")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        };

        let (name, model, id) = if columns.len() >= 2 {
            let first = columns[0];
            let (clean_name, parsed_model) = split_name_and_model(first);
            let model = if !parsed_model.is_empty() {
                parsed_model
            } else if columns.len() >= 3 && !looks_like_address(columns[1]) {
                columns[1].to_string()
            } else {
                String::new()
            };
            let id = if !clean_name.is_empty() {
                clean_name.clone()
            } else if !address.is_empty() {
                address.clone()
            } else {
                columns[0].to_string()
            };
            (clean_name, model, id)
        } else {
            let without_addr = if !address.is_empty() {
                line.replace(&address, "")
                    .trim_matches(|c: char| c.is_whitespace() || c == '-' || c == '[' || c == ']')
                    .to_string()
            } else {
                line.to_string()
            };
            let (clean_name, parsed_model) = split_name_and_model(&without_addr);
            let final_name = if clean_name.is_empty() {
                if !address.is_empty() {
                    format!("Dispositivo Apple ({})", address)
                } else {
                    line.to_string()
                }
            } else {
                clean_name
            };
            let id = if !without_addr.is_empty() && !final_name.starts_with("Dispositivo Apple") {
                final_name.clone()
            } else if !address.is_empty() {
                address.clone()
            } else {
                final_name.clone()
            };
            (final_name, parsed_model, id)
        };

        if seen_ids.insert(id.to_lowercase()) {
            peers.push(AirDropPeer {
                id,
                name,
                model,
                address,
                via_discover: true,
            });
        }
    }

    peers
}

/// Parsea direcciones MAC/AWDL desde `owly radio peers`.
pub fn parse_owly_radio_peers_output(stdout: &str) -> Vec<AirDropPeer> {
    let mut peers = Vec::new();
    let mut seen = HashSet::new();
    for line in stdout.lines() {
        if let Some(mac) = extract_mac_address(line) {
            if seen.insert(mac.to_lowercase()) {
                peers.push(AirDropPeer {
                    id: mac.clone(),
                    name: format!("Dispositivo Apple ({})", mac),
                    model: "AWDL".to_string(),
                    address: mac,
                    via_discover: false,
                });
            }
        }
    }
    peers
}

/// Extrae un nombre de archivo o URL si `owly receive` imprime una línea de recepción completada.
pub fn parse_owly_receive_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    for prefix in [
        "saved ",
        "Saved ",
        "wrote ",
        "Wrote ",
        "received ",
        "Received ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let candidate = rest.trim().trim_matches('"').trim_matches('\'');
            if !candidate.is_empty() {
                let file_only = Path::new(candidate)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(candidate);
                return Some(file_only.to_string());
            }
        }
    }
    None
}

fn split_name_and_model(raw: &str) -> (String, String) {
    if let Some(open) = raw.rfind('(') {
        if raw.ends_with(')') && open > 0 {
            let name = raw[..open].trim().to_string();
            let model = raw[open + 1..raw.len() - 1].trim().to_string();
            return (name, model);
        }
    }
    (raw.trim().to_string(), String::new())
}

fn looks_like_address(token: &str) -> bool {
    extract_mac_address(token).is_some() || token.starts_with("fe80:")
}

fn extract_peer_address(line: &str) -> Option<String> {
    for tok in line.split_whitespace() {
        let clean = tok.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | ';'));
        if clean.starts_with("fe80:") {
            return Some(clean.to_string());
        }
    }
    extract_mac_address(line)
}

fn extract_mac_address(line: &str) -> Option<String> {
    for tok in line.split_whitespace() {
        let clean = tok.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | ';'));
        let parts: Vec<&str> = clean.split(':').collect();
        if parts.len() == 6
            && parts
                .iter()
                .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Some(clean.to_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owly_doctor_json_for_intel_ax210() {
        let json = r#"{
            "channel": 6,
            "device": "8086:2725",
            "driver": "iwlwifi",
            "hardware": true,
            "issues": ["Intel cards run on channel 6"],
            "missing": [],
            "mode": "exclusive",
            "station": "wlp4s0",
            "status": "tested"
        }"#;
        let report: OwlyDoctorReport = serde_json::from_str(json).unwrap();
        assert!(report.hardware);
        assert_eq!(report.mode.as_deref(), Some("exclusive"));
        assert_eq!(report.station.as_deref(), Some("wlp4s0"));
        assert_eq!(report.channel, Some(6));
        assert!(report.missing.is_empty());
    }

    #[test]
    fn parses_owly_browse_and_ignores_status_noise() {
        let sample = "\
not browsing mDNS: test
trying AWDL peer aa:bb:cc:dd:ee:ff
MacBook Air de Cristian (Mac14,2)\tfe80::1234:56ff:fe78:9abc
iPhone de Cristian\t00:11:22:33:44:55
no AWDL peer answered /Discover
";
        let peers = parse_owly_browse_output(sample);
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].name, "MacBook Air de Cristian");
        assert_eq!(peers[0].model, "Mac14,2");
        assert_eq!(peers[0].address, "fe80::1234:56ff:fe78:9abc");
        assert_eq!(peers[1].name, "iPhone de Cristian");
        assert_eq!(peers[1].address, "00:11:22:33:44:55");
    }

    #[test]
    fn parses_owly_radio_peers_mac_addresses() {
        let sample = "peers: 42:11:aa:bb:cc:dd (rssi -48)\n";
        let peers = parse_owly_radio_peers_output(sample);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, "42:11:aa:bb:cc:dd");
        assert!(!peers[0].via_discover);
    }

    #[test]
    fn parses_owly_receive_saved_line() {
        assert_eq!(
            parse_owly_receive_line("saved /home/cristian/Descargas/foto_mac.jpg"),
            Some("foto_mac.jpg".to_string())
        );
        assert_eq!(parse_owly_receive_line("listening on awdl0:8771"), None);
    }
}

