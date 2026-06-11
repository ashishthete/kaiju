//! Per-device pairing tokens, persisted to `~/.kaiju/devices.json`
//! (override with `KAIJU_DEVICES`). Mirrors `settings.rs`: optional file,
//! best-effort load, explicit save. Each paired device holds its own token in
//! its browser; revoking a device deletes its row here.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use kaiju_core::{NexusError, Result};
use serde::{Deserialize, Serialize};

/// One paired device. `token` is the secret it presents (bearer / WS query).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// The set of paired devices.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Devices {
    #[serde(default)]
    pub devices: Vec<Device>,
}

impl Devices {
    /// All device tokens (for the auth check).
    pub fn tokens(&self) -> Vec<String> {
        self.devices.iter().map(|d| d.token.clone()).collect()
    }

    /// Add a device. Returns the created row.
    pub fn add(&mut self, name: String, token: String, now: DateTime<Utc>) -> Device {
        let device = Device {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            token,
            created_at: now,
            last_seen: now,
        };
        self.devices.push(device.clone());
        device
    }

    /// Remove a device by id. Returns true if one was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        self.devices.len() != before
    }

    /// Update `last_seen` for whichever device presented this token (if any).
    pub fn touch_by_token(&mut self, token: &str, now: DateTime<Utc>) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.token == token) {
            d.last_seen = now;
        }
    }
}

/// Path to the devices file: `KAIJU_DEVICES`, else `~/.kaiju/devices.json`.
fn devices_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_DEVICES") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("devices.json"))
}

/// Read paired devices; an absent or malformed file means "no devices".
pub fn load() -> Devices {
    let Some(path) = devices_path() else {
        return Devices::default();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return Devices::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Persist devices (pretty JSON), creating the directory if needed. Tokens are
/// stored in plaintext, so on unix the file is restricted to the owner (0600).
pub fn save(devices: &Devices) -> Result<()> {
    let path = devices_path().ok_or_else(|| {
        NexusError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no devices path (set KAIJU_DEVICES or HOME)",
        ))
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(NexusError::Io)?;
    }
    let json = serde_json::to_string_pretty(devices)
        .map_err(|e| NexusError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, json).map_err(NexusError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(NexusError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-06-11T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn add_then_token_is_listed() {
        let mut d = Devices::default();
        let row = d.add("Phone".into(), "tok-1".into(), now());
        assert_eq!(d.tokens(), vec!["tok-1".to_string()]);
        assert_eq!(row.name, "Phone");
        assert!(!row.id.is_empty());
    }

    #[test]
    fn remove_by_id() {
        let mut d = Devices::default();
        let row = d.add("Phone".into(), "tok-1".into(), now());
        assert!(d.remove(&row.id));
        assert!(d.tokens().is_empty());
        assert!(!d.remove("nope"));
    }

    #[test]
    fn touch_only_matching_token() {
        let mut d = Devices::default();
        let created = "2026-06-11T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let later = "2026-06-11T01:00:00Z".parse::<DateTime<Utc>>().unwrap();
        d.add("Phone".into(), "tok-1".into(), created);
        d.touch_by_token("tok-1", later);
        d.touch_by_token("unknown", later); // no-op
        assert_eq!(d.devices.len(), 1);
        assert_eq!(d.devices[0].last_seen, later);
    }
}
