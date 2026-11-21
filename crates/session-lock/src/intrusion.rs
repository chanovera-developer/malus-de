//! Registro persistente de intentos fallidos de intrusión y retardo progresivo.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntrusionState {
    pub failed_attempts: u32,
    pub lockout_until_unix: u64,
    pub last_failed_local: Option<String>,
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Espera progresiva tras intentos fallidos consecutivos (segundos).
pub fn cooldown_seconds_for_attempts(failed_attempts: u32) -> u64 {
    match failed_attempts {
        0..=2 => 0,
        3 => 5,
        4 => 15,
        5 => 30,
        _ => 60,
    }
}

pub fn intrusion_state_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("malus-de").join("lock-intrusion.json")
}

pub fn load_state_from(path: &Path) -> IntrusionState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<IntrusionState>(&raw).ok())
        .unwrap_or_default()
}

pub fn save_state_to(path: &Path, state: &IntrusionState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(state) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            }
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

pub fn record_failed_attempt_at(
    path: &Path,
    state: &mut IntrusionState,
    now_unix: u64,
    now_local: &str,
) -> u64 {
    state.failed_attempts = state.failed_attempts.saturating_add(1);
    state.last_failed_local = Some(now_local.to_string());
    let wait = cooldown_seconds_for_attempts(state.failed_attempts);
    state.lockout_until_unix = now_unix.saturating_add(wait);
    save_state_to(path, state);
    wait
}

pub fn remaining_cooldown_secs(state: &IntrusionState, now_unix: u64) -> u64 {
    state.lockout_until_unix.saturating_sub(now_unix)
}

pub fn clear_state_at(path: &Path) -> IntrusionState {
    let prev = load_state_from(path);
    let _ = std::fs::remove_file(path);
    prev
}

/// Envía una notificación crítica al daemon de notificaciones tras desbloquear
/// si hubo intentos fallidos durante el bloqueo.
pub fn dispatch_intrusion_alert(attempts: u32, last_failed: Option<&str>, spanish: bool) {
    if attempts == 0 {
        return;
    }
    let summary = if spanish {
        "Alerta de seguridad de bloqueo"
    } else {
        "Lock screen security alert"
    };
    let body = match (spanish, last_failed) {
        (true, Some(ts)) => {
            format!("Se registraron {attempts} intento(s) fallido(s) de desbloqueo (último: {ts}).")
        }
        (true, None) => {
            format!("Se registraron {attempts} intento(s) fallido(s) de desbloqueo.")
        }
        (false, Some(ts)) => {
            format!("{attempts} failed unlock attempt(s) detected while locked (last: {ts}).")
        }
        (false, None) => {
            format!("{attempts} failed unlock attempt(s) detected while locked.")
        }
    };

    if let Ok(conn) = zbus::blocking::Connection::session() {
        let mut hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        hints.insert("urgency", zbus::zvariant::Value::U8(2));
        let actions: Vec<&str> = Vec::new();
        let _ = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Malus DE Session Lock",
                0u32,
                "dialog-warning-symbolic",
                summary,
                body.as_str(),
                actions,
                hints,
                0i32,
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progressive_cooldown_escalates_on_repeated_failures() {
        assert_eq!(cooldown_seconds_for_attempts(0), 0);
        assert_eq!(cooldown_seconds_for_attempts(1), 0);
        assert_eq!(cooldown_seconds_for_attempts(2), 0);
        assert_eq!(cooldown_seconds_for_attempts(3), 5);
        assert_eq!(cooldown_seconds_for_attempts(4), 15);
        assert_eq!(cooldown_seconds_for_attempts(5), 30);
        assert_eq!(cooldown_seconds_for_attempts(6), 60);
        assert_eq!(cooldown_seconds_for_attempts(12), 60);
    }

    #[test]
    fn failed_attempts_persist_cooldown_and_reset_on_unlock() {
        let dir = std::env::temp_dir().join(format!(
            "malus-intrusion-test-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("lock-intrusion.json");
        let mut state = load_state_from(&path);
        assert_eq!(state.failed_attempts, 0);

        assert_eq!(
            record_failed_attempt_at(&path, &mut state, 1000, "15:20:01"),
            0
        );
        assert_eq!(
            record_failed_attempt_at(&path, &mut state, 1002, "15:20:02"),
            0
        );
        assert_eq!(
            record_failed_attempt_at(&path, &mut state, 1005, "15:20:05"),
            5
        );
        assert_eq!(remaining_cooldown_secs(&state, 1007), 3);

        let reloaded = load_state_from(&path);
        assert_eq!(reloaded.failed_attempts, 3);
        assert_eq!(reloaded.lockout_until_unix, 1010);
        assert_eq!(reloaded.last_failed_local.as_deref(), Some("15:20:05"));

        let cleared = clear_state_at(&path);
        assert_eq!(cleared.failed_attempts, 3);
        assert_eq!(load_state_from(&path).failed_attempts, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
