use malus_de_common::config::{NotificationAlertStyle, NotificationPreviewMode};
use std::collections::HashMap;
use std::path::PathBuf;
use zbus::zvariant::{Structure, Value};

pub const MAX_HISTORY_RECORDS: usize = 60;
pub const DEFAULT_BANNER_TIMEOUT_MS: u64 = 6_000;
pub const MIN_BANNER_TIMEOUT_MS: u64 = 1_500;
pub const MAX_BANNER_TIMEOUT_MS: u64 = 30_000;
pub const MAX_VISIBLE_ACTIONS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationAction {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationImage {
    Path(PathBuf),
    RawRgba {
        width: i32,
        height: i32,
        rowstride: i32,
        has_alpha: bool,
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct NotificationRecord {
    pub id: u32,
    pub group_key: String,
    pub desktop_entry: String,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
    pub has_default_action: bool,
    pub image: Option<NotificationImage>,
    pub timestamp_label: String,
    pub silenced_by_focus: bool,
    pub urgency: u8,
}

#[derive(Debug, Clone)]
pub struct NotificationGroup {
    pub group_key: String,
    pub app_name: String,
    pub app_icon: String,
    pub items: Vec<NotificationRecord>,
}

#[derive(Debug, Default)]
pub struct NotificationHistory {
    records: Vec<NotificationRecord>,
}

impl NotificationHistory {
    pub fn push(&mut self, record: NotificationRecord) {
        self.records.retain(|r| r.id != record.id);
        self.records.insert(0, record);
        if self.records.len() > MAX_HISTORY_RECORDS {
            self.records.truncate(MAX_HISTORY_RECORDS);
        }
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.records.len();
        self.records.retain(|r| r.id != id);
        self.records.len() != before
    }

    pub fn remove_group(&mut self, group_key: &str) -> usize {
        let before = self.records.len();
        self.records.retain(|r| r.group_key != group_key);
        before.saturating_sub(self.records.len())
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Agrupa las notificaciones por aplicación (`group_key`) preservando el orden por
    /// actividad más reciente en cada grupo.
    pub fn grouped(&self) -> Vec<NotificationGroup> {
        let mut groups: Vec<NotificationGroup> = Vec::new();
        for rec in &self.records {
            if let Some(existing) = groups.iter_mut().find(|g| g.group_key == rec.group_key) {
                existing.items.push(rec.clone());
            } else {
                let display_name = if !rec.app_name.trim().is_empty() {
                    rec.app_name.trim().to_string()
                } else if !rec.desktop_entry.trim().is_empty() {
                    rec.desktop_entry.trim().to_string()
                } else {
                    "Sistema".to_string()
                };
                groups.push(NotificationGroup {
                    group_key: rec.group_key.clone(),
                    app_name: display_name,
                    app_icon: rec.app_icon.clone(),
                    items: vec![rec.clone()],
                });
            }
        }
        groups
    }
}

pub fn notification_group_key(desktop_entry: &str, app_name: &str, app_icon: &str) -> String {
    for raw in [desktop_entry, app_name, app_icon] {
        let cleaned = raw
            .trim()
            .strip_prefix("application://")
            .unwrap_or(raw.trim())
            .trim_end_matches(".desktop")
            .trim()
            .to_ascii_lowercase();
        if !cleaned.is_empty() && !cleaned.starts_with('/') {
            return cleaned;
        }
    }
    "system".to_string()
}

/// Extrae pares `(key, label)` de `actions` (org.freedesktop.Notifications),
/// omitiendo la clave `"default"` que se ejecuta al hacer clic sobre la tarjeta.
pub fn parse_notification_actions(raw_actions: &[String]) -> Vec<NotificationAction> {
    let mut out = Vec::new();
    for pair in raw_actions.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let key = pair[0].trim();
        let label = pair[1].trim();
        if key.is_empty() || key == "default" || label.is_empty() {
            continue;
        }
        if !out.iter().any(|a: &NotificationAction| a.key == key) {
            out.push(NotificationAction {
                key: key.to_string(),
                label: label.to_string(),
            });
            if out.len() >= MAX_VISIBLE_ACTIONS {
                break;
            }
        }
    }
    out
}

/// Calcula el tiempo de auto-cierre efectivo de una notificación según la especificación
/// `org.freedesktop.Notifications` y el estilo configurado por el usuario.
///
/// - `Alerts` o `expire_timeout == 0` -> `None` (persistente hasta interacción).
/// - `expire_timeout > 0` -> acotado entre `1500 ms` y `30000 ms`.
/// - `expire_timeout < 0` (`-1`, default del servidor) -> `6000 ms`.
pub fn resolve_expire_timeout_ms(
    expire_timeout: i32,
    style: NotificationAlertStyle,
) -> Option<u64> {
    if style != NotificationAlertStyle::Banners || expire_timeout == 0 {
        return None;
    }
    if expire_timeout > 0 {
        Some((expire_timeout as u64).clamp(MIN_BANNER_TIMEOUT_MS, MAX_BANNER_TIMEOUT_MS))
    } else {
        Some(DEFAULT_BANNER_TIMEOUT_MS)
    }
}

pub fn should_hide_preview(mode: NotificationPreviewMode, session_locked: bool) -> bool {
    match mode {
        NotificationPreviewMode::Always => false,
        NotificationPreviewMode::WhenUnlocked => session_locked,
        NotificationPreviewMode::Never => true,
    }
}

pub fn format_current_hhmm() -> String {
    glib::DateTime::now_local()
        .and_then(|dt| dt.format("%H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "Ahora".to_string())
}

fn value_as_i32(v: &Value<'_>) -> Option<i32> {
    i32::try_from(v)
        .ok()
        .or_else(|| u32::try_from(v).ok().and_then(|n| i32::try_from(n).ok()))
}

fn value_as_bool(v: &Value<'_>) -> Option<bool> {
    bool::try_from(v).ok()
}

fn parse_image_data_struct(val: &Value<'_>) -> Option<NotificationImage> {
    let s = <&Structure<'_>>::try_from(val).ok()?;
    let fields = s.fields();
    if fields.len() < 7 {
        return None;
    }
    let width = value_as_i32(&fields[0])?;
    let height = value_as_i32(&fields[1])?;
    let rowstride = value_as_i32(&fields[2])?;
    let has_alpha = value_as_bool(&fields[3])?;
    let bits_per_sample = value_as_i32(&fields[4])?;
    let channels = value_as_i32(&fields[5])?;
    let bytes: Vec<u8> = match &fields[6] {
        Value::Array(arr) => arr
            .iter()
            .map(|item| u8::try_from(item).ok())
            .collect::<Option<Vec<u8>>>()?,
        _ => return None,
    };

    if width <= 0
        || height <= 0
        || width > 2048
        || height > 2048
        || bits_per_sample != 8
        || rowstride <= 0
    {
        return None;
    }
    let expected_channels = if has_alpha { 4 } else { 3 };
    if channels != expected_channels {
        return None;
    }
    let min_row_bytes = (width as usize).checked_mul(channels as usize)?;
    if (rowstride as usize) < min_row_bytes {
        return None;
    }
    let min_total = (rowstride as usize)
        .checked_mul((height - 1) as usize)?
        .checked_add(min_row_bytes)?;
    if bytes.len() < min_total {
        return None;
    }

    Some(NotificationImage::RawRgba {
        width,
        height,
        rowstride,
        has_alpha,
        data: bytes,
    })
}

/// Extrae una imagen/miniatura desde los *hints* estándar de `org.freedesktop.Notifications`
/// (`image-data`, `image_data`, `image-path`, `image_path` o `icon_data`).
pub fn extract_hint_image(hints: &HashMap<String, Value<'_>>) -> Option<NotificationImage> {
    for key in ["image-data", "image_data"] {
        if let Some(val) = hints.get(key) {
            if let Some(img) = parse_image_data_struct(val) {
                return Some(img);
            }
        }
    }
    for key in ["image-path", "image_path"] {
        if let Some(val) = hints.get(key) {
            if let Ok(raw) = <&str>::try_from(val) {
                let cleaned = raw.trim().strip_prefix("file://").unwrap_or(raw.trim());
                if !cleaned.is_empty() {
                    let path = PathBuf::from(cleaned);
                    if path.is_file() {
                        return Some(NotificationImage::Path(path));
                    }
                }
            }
        }
    }
    if let Some(val) = hints.get("icon_data") {
        if let Some(img) = parse_image_data_struct(val) {
            return Some(img);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(id: u32, app: &str, summary: &str, silenced: bool) -> NotificationRecord {
        NotificationRecord {
            id,
            group_key: notification_group_key("", app, ""),
            desktop_entry: String::new(),
            app_name: app.to_string(),
            app_icon: String::new(),
            summary: summary.to_string(),
            body: "Cuerpo".to_string(),
            actions: Vec::new(),
            has_default_action: false,
            image: None,
            timestamp_label: "00:45".to_string(),
            silenced_by_focus: silenced,
            urgency: 1,
        }
    }

    #[test]
    fn groups_notifications_by_app_and_deduplicates_replaced_ids() {
        let mut hist = NotificationHistory::default();
        hist.push(sample_record(1, "Telegram", "Hola 1", false));
        hist.push(sample_record(2, "Firefox", "Descarga lista", false));
        hist.push(sample_record(3, "Telegram", "Hola 2", true));
        // Reemplazo del id 1
        hist.push(sample_record(1, "Telegram", "Hola 1 editado", false));

        assert_eq!(hist.len(), 3);
        let groups = hist.grouped();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].group_key, "telegram");
        assert_eq!(groups[0].items.len(), 2);
        assert_eq!(groups[0].items[0].summary, "Hola 1 editado");
        assert_eq!(groups[0].items[1].summary, "Hola 2");
        assert!(groups[0].items[1].silenced_by_focus);
        assert_eq!(groups[1].group_key, "firefox");
        assert_eq!(groups[1].items.len(), 1);

        assert_eq!(hist.remove_group("telegram"), 2);
        assert_eq!(hist.len(), 1);
        hist.clear();
        assert!(hist.is_empty());
    }

    #[test]
    fn parses_actions_excluding_default_and_caps_count() {
        let raw = vec![
            "default".to_string(),
            "Abrir".to_string(),
            "reply".to_string(),
            "Responder".to_string(),
            "mark_read".to_string(),
            "Marcar leído".to_string(),
            "archive".to_string(),
            "Archivar".to_string(),
            "extra".to_string(),
            "Extra".to_string(),
        ];
        let actions = parse_notification_actions(&raw);
        assert_eq!(actions.len(), MAX_VISIBLE_ACTIONS);
        assert_eq!(actions[0].key, "reply");
        assert_eq!(actions[0].label, "Responder");
        assert_eq!(actions[1].key, "mark_read");
        assert_eq!(actions[2].key, "archive");
    }

    #[test]
    fn resolves_expire_timeout_and_when_unlocked_privacy() {
        assert_eq!(
            resolve_expire_timeout_ms(-1, NotificationAlertStyle::Banners),
            Some(DEFAULT_BANNER_TIMEOUT_MS)
        );
        assert_eq!(
            resolve_expire_timeout_ms(0, NotificationAlertStyle::Banners),
            None
        );
        assert_eq!(
            resolve_expire_timeout_ms(500, NotificationAlertStyle::Banners),
            Some(MIN_BANNER_TIMEOUT_MS)
        );
        assert_eq!(
            resolve_expire_timeout_ms(12_000, NotificationAlertStyle::Banners),
            Some(12_000)
        );
        assert_eq!(
            resolve_expire_timeout_ms(90_000, NotificationAlertStyle::Banners),
            Some(MAX_BANNER_TIMEOUT_MS)
        );
        assert_eq!(
            resolve_expire_timeout_ms(5_000, NotificationAlertStyle::Alerts),
            None
        );

        assert!(!should_hide_preview(NotificationPreviewMode::Always, true));
        assert!(!should_hide_preview(
            NotificationPreviewMode::WhenUnlocked,
            false
        ));
        assert!(should_hide_preview(
            NotificationPreviewMode::WhenUnlocked,
            true
        ));
        assert!(should_hide_preview(NotificationPreviewMode::Never, false));
    }

    #[test]
    fn extracts_raw_rgba_image_data_hint() {
        let pixels: Vec<u8> = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        let raw_struct = Value::from((2i32, 2i32, 8i32, true, 8i32, 4i32, pixels.clone()));
        let mut hints = HashMap::new();
        hints.insert("image-data".to_string(), raw_struct);

        let img = extract_hint_image(&hints).expect("debe parsear image-data válido");
        assert_eq!(
            img,
            NotificationImage::RawRgba {
                width: 2,
                height: 2,
                rowstride: 8,
                has_alpha: true,
                data: pixels,
            }
        );
    }
}
