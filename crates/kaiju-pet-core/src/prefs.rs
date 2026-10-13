use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPos {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyPrefs {
    pub waitingforinput: bool,
    pub stuck: bool,
    pub limited: bool,
    pub error: bool,
    pub completed: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        NotifyPrefs {
            waitingforinput: true,
            stuck: true,
            limited: true,
            error: true,
            completed: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub daemon_url: String,
    pub window: WindowPos,
    pub sound: bool,
    pub reduced_motion: bool,
    pub pause_until: Option<String>,
    pub notify: NotifyPrefs,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            daemon_url: "http://127.0.0.1:7800".to_string(),
            window: WindowPos { x: 1200, y: 760 },
            sound: true,
            reduced_motion: false,
            pause_until: None,
            notify: NotifyPrefs::default(),
        }
    }
}

/// Clamp a window origin so it stays inside `(min_x, min_y, max_x, max_y)`.
pub fn clamp_to_bounds(pos: WindowPos, bounds: (i32, i32, i32, i32)) -> WindowPos {
    let (min_x, min_y, max_x, max_y) = bounds;
    WindowPos {
        x: pos.x.clamp(min_x, max_x),
        y: pos.y.clamp(min_y, max_y),
    }
}

/// `~/.config/kaiju/pet.json` (honours `XDG_CONFIG_HOME`).
pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("kaiju").join("pet.json")
}

pub fn load() -> Prefs {
    match std::fs::read_to_string(config_path()) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_default(),
        Err(_) => Prefs::default(),
    }
}

pub fn save(p: &Prefs) -> Result<(), std::io::Error> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(p)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_keeps_given_fields_and_defaults_the_rest() {
        let json = r#"{"daemon_url":"http://127.0.0.1:9000","notify":{"stuck":false}}"#;
        let p: Prefs = serde_json::from_str(json).unwrap();
        assert_eq!(p.daemon_url, "http://127.0.0.1:9000");
        assert!(!p.notify.stuck);
        assert!(p.notify.error);
        assert_eq!(p.sound, Prefs::default().sound);
    }

    #[test]
    fn defaults_point_at_loopback() {
        let p = Prefs::default();
        assert_eq!(p.daemon_url, "http://127.0.0.1:7800");
        assert!(p.notify.waitingforinput);
    }

    #[test]
    fn clamps_offscreen_position_into_bounds() {
        // window pushed far right/below; bounds are a 1920x1080 display.
        let clamped = clamp_to_bounds(WindowPos { x: 5000, y: 5000 }, (0, 0, 1920, 1080));
        assert!(clamped.x <= 1920 && clamped.x >= 0);
        assert!(clamped.y <= 1080 && clamped.y >= 0);
    }

    #[test]
    fn keeps_visible_position_unchanged() {
        let pos = WindowPos { x: 1200, y: 760 };
        assert_eq!(
            clamp_to_bounds(pos, (0, 0, 1920, 1080)),
            WindowPos { x: 1200, y: 760 }
        );
    }

    #[test]
    fn roundtrips_through_json() {
        let p = Prefs::default();
        let json = serde_json::to_string(&p).unwrap();
        let back: Prefs = serde_json::from_str(&json).unwrap();
        assert_eq!(back.daemon_url, p.daemon_url);
    }
}
