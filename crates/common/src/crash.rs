//! Captura de panics y generación de informes de fallo en texto plano (`.txt`) sanitizados.
//!
//! Cada reporte se almacena en `${XDG_DATA_HOME:-$HOME/.local/share}/malus-de/crashes/`
//! con el prefijo `pending-` hasta que el diálogo gráfico lo presenta al usuario y lo
//! promueve a `malus-crash-`.

use std::backtrace::Backtrace;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CRASH_REPORT_EMAIL: &str = "malus-de@chano.dev";
pub const DEFAULT_CRASH_ENDPOINT: &str = "https://crash.chano.dev/report";

static HOOK_INSTALLED: Once = Once::new();
static HANDLING_PANIC: AtomicBool = AtomicBool::new(false);

/// Devuelve el directorio de almacenamiento para volcados de fallo (`~/.local/share/malus-de/crashes`).
pub fn crash_reports_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            home.join(".local/share")
        });
    base.join("malus-de").join("crashes")
}

/// Ruta al registro de sesión (`~/.local/share/malus-de/session.log`).
pub fn session_log_path() -> PathBuf {
    crash_reports_dir()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("session.log")
}

/// Instala el hook global de panics una sola vez para el binario indicado.
pub fn install_panic_hook(component: &str) {
    if std::env::var_os("MALUS_DISABLE_CRASH_HOOK").is_some()
        || component == "malus_de_crash_reporter"
    {
        return;
    }

    let component_name = component.to_string();
    HOOK_INSTALLED.call_once(|| {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Reenviar siempre al hook previo para conservar el trazo en stderr / session.log
            prev_hook(info);

            if HANDLING_PANIC
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return;
            }

            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "ubicación desconocida".to_string());

            let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "panic sin mensaje explícito".to_string()
            };

            let thread_name = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            let backtrace = Backtrace::force_capture().to_string();

            if let Ok(report_path) = write_panic_report(
                &component_name,
                &thread_name,
                &location,
                &message,
                &backtrace,
            ) {
                // Si falló un componente auxiliar mientras sigue viva la sesión Wayland,
                // abrir el diálogo inmediatamente sin bloquear la salida.
                let is_compositor = component_name == "malus_de_compositor";
                let has_display = std::env::var_os("WAYLAND_DISPLAY").is_some()
                    || std::env::var_os("DISPLAY").is_some();
                if !is_compositor && has_display {
                    spawn_crash_reporter(&report_path);
                }
            }

            HANDLING_PANIC.store(false, Ordering::SeqCst);
        }));
    });
}

fn spawn_crash_reporter(report_path: &Path) {
    let local_bin = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("malus-de-crash-reporter")))
        .filter(|path| path.is_file());

    let mut cmd = local_bin
        .map(Command::new)
        .unwrap_or_else(|| Command::new("malus-de-crash-reporter"));
    cmd.arg("--report").arg(report_path);
    let _ = cmd.spawn();
}

/// Formatea un volcado de panic en texto plano sanitizado y lo escribe como `pending-*.txt`.
pub fn write_panic_report(
    component: &str,
    thread_name: &str,
    location: &str,
    message: &str,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    let dir = crash_reports_dir();
    fs::create_dir_all(&dir)?;

    let stamp = unix_timestamp_secs();
    let safe_comp: String = component
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let path = dir.join(format!("pending-{stamp}-{safe_comp}.txt"));

    let home_dir = std::env::var("HOME").ok();
    let log_tail = read_tail_lines(&session_log_path(), 140);
    let report = format_panic_report_text(
        component,
        thread_name,
        location,
        message,
        backtrace,
        &log_tail,
        home_dir.as_deref(),
    );

    fs::write(&path, report)?;
    Ok(path)
}

/// Genera el texto completo legible del informe de fallo y sanitiza la ruta `$HOME`.
pub fn format_panic_report_text(
    component: &str,
    thread_name: &str,
    location: &str,
    message: &str,
    backtrace: &str,
    session_log_tail: &str,
    home_dir: Option<&str>,
) -> String {
    let timestamp = current_utc_iso_like();
    let distro = read_distro_pretty_name();
    let kernel = read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "desconocido".into());
    let wayland = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "(no definido)".into());
    let x11 = std::env::var("DISPLAY").unwrap_or_else(|_| "(no definido)".into());

    let raw = format!(
        "============================================================\n\
         MALUS DE - INFORME DE FALLO (CRASH DUMP)\n\
         ============================================================\n\
         Fecha (UTC): {timestamp}\n\
         Componente:  {component}\n\
         Hilo:        {thread_name}\n\
         Ubicación:   {location}\n\
         Distro:      {distro}\n\
         Kernel:      {kernel} ({arch})\n\
         Wayland:     {wayland}\n\
         Display X11: {x11}\n\
         \n\
         --- MENSAJE DEL ERROR ---\n\
         {message}\n\
         \n\
         --- BACKTRACE DE RUST ---\n\
         {backtrace}\n\
         \n\
         --- ÚLTIMAS LÍNEAS DE session.log ---\n\
         {log_section}\n",
        arch = std::env::consts::ARCH,
        log_section = if session_log_tail.trim().is_empty() {
            "(session.log no disponible o vacío)"
        } else {
            session_log_tail
        },
    );

    sanitize_home_paths(&raw, home_dir)
}

/// Reemplaza rutas absolutas del directorio personal del usuario por `~` para evitar filtrar su nombre de usuario.
pub fn sanitize_home_paths(text: &str, home_dir: Option<&str>) -> String {
    match home_dir.map(str::trim).filter(|h| !h.is_empty() && *h != "/") {
        Some(home) => text.replace(home, "~"),
        None => text.to_string(),
    }
}

/// Lista reportes pendientes (`pending-*.txt`) ordenados del más reciente al más antiguo.
pub fn list_pending_reports() -> Vec<PathBuf> {
    let dir = crash_reports_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| name.starts_with("pending-") && name.ends_with(".txt"))
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

/// Renombra `pending-*.txt` a `malus-crash-*.txt` para que no vuelva a abrirse en el próximo inicio.
pub fn archive_pending_report(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return path.to_path_buf();
    };
    if let Some(rest) = name.strip_prefix("pending-") {
        let archived = path.with_file_name(format!("malus-crash-{rest}"));
        if fs::rename(path, &archived).is_ok() {
            return archived;
        }
    }
    path.to_path_buf()
}

/// Payload JSON enviado al endpoint HTTPS del recolector de fallos.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrashReportPayload {
    pub recipient: String,
    pub component: String,
    pub summary: String,
    pub filename: String,
    pub report: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct CrashReporterConfig {
    endpoint: Option<String>,
}

fn read_configured_endpoint() -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(xdg);
        candidates.push(p.join("crash-reporter.toml"));
        candidates.push(p.join("malus-de").join("crash-reporter.toml"));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        candidates.push(
            PathBuf::from(home)
                .join(".config")
                .join("malus-de")
                .join("crash-reporter.toml"),
        );
    }

    for path in candidates {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(cfg) = toml::from_str::<CrashReporterConfig>(&content) {
            if let Some(url) = cfg.endpoint.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                return Some(url);
            }
        }
    }
    None
}

/// Devuelve la URL del endpoint HTTPS encargado de reenviar el reporte a `malus-de@chano.dev`.
pub fn crash_report_endpoint() -> String {
    if let Ok(env_url) = std::env::var("MALUS_CRASH_ENDPOINT") {
        let trimmed = env_url.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    read_configured_endpoint().unwrap_or_else(|| DEFAULT_CRASH_ENDPOINT.to_string())
}

/// Envía el informe completo mediante HTTP POST JSON al endpoint configurado (Cloudflare Worker).
pub fn send_report_via_endpoint(
    endpoint: &str,
    component: &str,
    summary: &str,
    report_path: &Path,
    report_text: &str,
) -> Result<String, String> {
    let filename = report_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("malus-crash-report.txt")
        .to_string();

    let payload = CrashReportPayload {
        recipient: CRASH_REPORT_EMAIL.to_string(),
        component: component.to_string(),
        summary: summary.to_string(),
        filename,
        report: report_text.to_string(),
    };

    let json_bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("No se pudo serializar el reporte: {e}"))?;

    let mut child = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "15",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-H",
            "Accept: application/json",
            "-H",
            "User-Agent: malus-de-crash-reporter/0.1",
            "-w",
            "\n%{http_code}",
            "--data-binary",
            "@-",
            endpoint,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("No se pudo ejecutar curl: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&json_bytes)
            .map_err(|e| format!("Error enviando datos al proceso curl: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Error esperando respuesta de red: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("curl terminó con código {:?}", output.status.code())
        } else {
            stderr
        });
    }

    let raw_stdout = String::from_utf8_lossy(&output.stdout);
    parse_curl_http_output(&raw_stdout)
}

fn parse_curl_http_output(raw_stdout: &str) -> Result<String, String> {
    let trimmed = raw_stdout.trim_end_matches(&['\r', '\n'][..]);
    let (body, code_str) = match trimmed.rsplit_once('\n') {
        Some((b, c)) => (b.trim(), c.trim()),
        None => ("", trimmed.trim()),
    };

    let status: u16 = code_str
        .parse()
        .map_err(|_| format!("Respuesta HTTP inválida del servidor ({code_str})"))?;

    if (200..300).contains(&status) {
        return Ok(format!(
            "Reporte enviado directamente a {CRASH_REPORT_EMAIL}."
        ));
    }

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err_msg) = json.get("error").and_then(|v| v.as_str()) {
            return Err(format!("HTTP {status}: {err_msg}"));
        }
    }

    Err(if body.is_empty() {
        format!("El servidor respondió con estado HTTP {status}")
    } else {
        format!("HTTP {status}: {}", body.chars().take(180).collect::<String>())
    })
}

/// Construye un enlace `mailto:` compacto para abrir el cliente de correo predeterminado.
pub fn build_mailto_uri(recipient: &str, component: &str, report_path: &Path, summary_line: &str) -> String {
    let subject = format!("Reporte de fallo Malus DE ({component})");
    let body = format!(
        "Hola,\n\n\
         Adjunto el archivo de diagnóstico generado tras un cierre inesperado en Malus DE.\n\n\
         Componente: {component}\n\
         Resumen: {summary_line}\n\
         Archivo guardado en: {path}\n\n\
         (También copiado al portapapeles por si prefieres pegarlo directamente.)\n",
        path = report_path.display()
    );
    format!(
        "mailto:{recipient}?subject={}&body={}",
        percent_encode(&subject),
        percent_encode(&body),
    )
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

fn read_tail_lines(path: &Path, max_lines: usize) -> String {
    let Ok(content) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_distro_pretty_name() -> String {
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
                return rest.trim_matches('"').to_string();
            }
        }
    }
    "Linux".to_string()
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn current_utc_iso_like() -> String {
    let secs = unix_timestamp_secs();
    // Conversión civil Howard Hinnant sin dependencias externas
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_home_paths_masks_username_directories() {
        let raw = "Error opening /home/lolo/.local/share/malus-de/session.log in /home/lolo/app";
        let sanitized = sanitize_home_paths(raw, Some("/home/lolo"));
        assert_eq!(
            sanitized,
            "Error opening ~/.local/share/malus-de/session.log in ~/app"
        );
    }

    #[test]
    fn archive_pending_report_rewrites_prefix_cleanly() {
        let dir = std::env::temp_dir().join(format!("malus-crash-test-{}", unix_timestamp_secs()));
        fs::create_dir_all(&dir).unwrap();
        let pending = dir.join("pending-1700000000-malus_de_compositor.txt");
        fs::write(&pending, "panic test").unwrap();

        let archived = archive_pending_report(&pending);
        assert_eq!(
            archived.file_name().and_then(|s| s.to_str()),
            Some("malus-crash-1700000000-malus_de_compositor.txt")
        );
        assert!(archived.exists());
        assert!(!pending.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_mailto_uri_targets_requested_email_and_encodes_fields() {
        let uri = build_mailto_uri(
            CRASH_REPORT_EMAIL,
            "malus-de-compositor",
            Path::new("/tmp/malus-crash-1.txt"),
            "exit SIGSEGV",
        );
        assert!(uri.starts_with("mailto:malus-de@chano.dev?subject="));
        assert!(uri.contains("&body="));
        assert!(!uri.contains(' '));
    }

    #[test]
    fn parse_curl_http_output_handles_success_and_json_errors() {
        assert!(parse_curl_http_output("{\"ok\":true}\n200").is_ok());
        assert!(parse_curl_http_output("\n204\n").is_ok());

        let err = parse_curl_http_output("{\"error\":\"rate limited\"}\n429").unwrap_err();
        assert!(err.contains("429"));
        assert!(err.contains("rate limited"));

        let err_empty = parse_curl_http_output("\n502").unwrap_err();
        assert!(err_empty.contains("502"));
    }

    #[test]
    fn crash_reporter_config_parses_custom_endpoint() {
        let cfg: CrashReporterConfig =
            toml::from_str("endpoint = \"https://example.chano.dev/report\"\n").unwrap();
        assert_eq!(
            cfg.endpoint.as_deref(),
            Some("https://example.chano.dev/report")
        );
    }
}
