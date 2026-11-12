//! User preferences, persisted as `key=value` lines in `ui.conf`.

use std::path::Path;

use crate::theme::Appearance;

/// What desktop notifications reveal (Signal's "Show" setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NotificationContent {
    #[default]
    NameAndMessage,
    NameOnly,
    NoNameOrMessage,
}

impl NotificationContent {
    pub const ALL: [NotificationContent; 3] =
        [Self::NameAndMessage, Self::NameOnly, Self::NoNameOrMessage];

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAndMessage => "Name, content, and actions",
            Self::NameOnly => "Name only",
            Self::NoNameOrMessage => "No name or content",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::NameAndMessage => "name_and_message",
            Self::NameOnly => "name_only",
            Self::NoNameOrMessage => "none",
        }
    }

    /// Applies the setting to a notification's (title, body).
    pub fn redact(self, title: &str, body: &str) -> (String, String) {
        match self {
            Self::NameAndMessage => (title.to_string(), body.to_string()),
            Self::NameOnly => (title.to_string(), "New message".to_string()),
            Self::NoNameOrMessage => ("Halyard".to_string(), "New message".to_string()),
        }
    }
}

/// The sound of an incoming call. Signal's apps have one ringtone; Halyard
/// also offers a few of its own (made in code, see `audio::ringtone_samples`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ringtone {
    /// Signal Desktop's ringtone.
    #[default]
    Signal,
    /// A telephone double ring.
    Classic,
    /// Three falling bell notes.
    Chime,
    /// A quick marimba run.
    Marimba,
    /// Soft beeps.
    Pulse,
    /// Soft piano notes with a shimmer and an echo.
    SciFiPiano,
    /// No sound; the call still shows and notifies.
    Silent,
}

impl Ringtone {
    pub const ALL: [Ringtone; 7] = [
        Self::Signal,
        Self::Classic,
        Self::Chime,
        Self::Marimba,
        Self::Pulse,
        Self::SciFiPiano,
        Self::Silent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Signal => "Signal",
            Self::Classic => "Classic",
            Self::Chime => "Chime",
            Self::Marimba => "Marimba",
            Self::Pulse => "Pulse",
            Self::SciFiPiano => "Sci-fi piano",
            Self::Silent => "Silent",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::Classic => "classic",
            Self::Chime => "chime",
            Self::Marimba => "marimba",
            Self::Pulse => "pulse",
            Self::SciFiPiano => "scifi_piano",
            Self::Silent => "silent",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub appearance: Appearance,
    pub notifications: bool,
    pub notification_content: NotificationContent,
    pub read_receipts: bool,
    pub typing_indicators: bool,
    /// Enter sends (Shift+Enter for a newline); otherwise ⌘/Ctrl+Enter sends.
    pub enter_sends: bool,
    /// UI zoom factor (1.0 = 100%).
    pub zoom: f32,
    pub ringtone: Ringtone,
}

/// Zoom steps offered by ⌘/Ctrl +/−, like Signal Desktop.
pub const ZOOM_STEPS: [f32; 7] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75];

/// The next zoom step up (`dir > 0`) or down from `current`.
pub fn step_zoom(current: f32, dir: i32) -> f32 {
    let ix = ZOOM_STEPS
        .iter()
        .position(|z| (z - current).abs() < 0.01)
        .unwrap_or(2) as i32;
    ZOOM_STEPS[(ix + dir).clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize]
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            notifications: true,
            notification_content: NotificationContent::NameAndMessage,
            read_receipts: true,
            typing_indicators: true,
            enter_sends: true,
            zoom: 1.0,
            ringtone: Ringtone::Signal,
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

impl Preferences {
    pub fn load(path: &Path) -> Self {
        let mut p = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return p;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "appearance" => {
                    p.appearance = match v.trim() {
                        "light" => Appearance::Light,
                        "dark" => Appearance::Dark,
                        _ => Appearance::System,
                    }
                }
                "notifications" => p.notifications = parse_bool(v).unwrap_or(p.notifications),
                "notification_content" => {
                    p.notification_content = NotificationContent::ALL
                        .into_iter()
                        .find(|c| c.key() == v.trim())
                        .unwrap_or_default()
                }
                "read_receipts" => p.read_receipts = parse_bool(v).unwrap_or(p.read_receipts),
                "typing_indicators" => {
                    p.typing_indicators = parse_bool(v).unwrap_or(p.typing_indicators)
                }
                "enter_sends" => p.enter_sends = parse_bool(v).unwrap_or(p.enter_sends),
                "ringtone" => {
                    p.ringtone = Ringtone::ALL
                        .into_iter()
                        .find(|r| r.key() == v.trim())
                        .unwrap_or_default()
                }
                "zoom" => {
                    if let Ok(z) = v.trim().parse::<f32>()
                        && (0.5..=2.0).contains(&z)
                    {
                        p.zoom = z;
                    }
                }
                _ => {}
            }
        }
        p
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let appearance = match self.appearance {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        };
        let text = format!(
            "appearance={appearance}\nnotifications={}\nnotification_content={}\nread_receipts={}\n\
             typing_indicators={}\nenter_sends={}\nzoom={}\nringtone={}\n",
            self.notifications,
            self.notification_content.key(),
            self.read_receipts,
            self.typing_indicators,
            self.enter_sends,
            self.zoom,
            self.ringtone.key(),
        );
        let tmp = path.with_extension("conf.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.conf");
        assert_eq!(Preferences::load(&path), Preferences::default());
        let p = Preferences {
            appearance: Appearance::Dark,
            notifications: false,
            notification_content: NotificationContent::NameOnly,
            read_receipts: false,
            typing_indicators: false,
            enter_sends: false,
            zoom: 1.25,
            ringtone: Ringtone::Marimba,
        };
        p.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), p);
    }

    #[test]
    fn reads_legacy_file_and_ignores_junk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.conf");
        std::fs::write(&path, "appearance=light\nbogus\nread_receipts=maybe\n").unwrap();
        let p = Preferences::load(&path);
        assert_eq!(p.appearance, Appearance::Light);
        assert!(p.read_receipts, "invalid values keep the default");
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(step_zoom(1.0, 1), 1.1);
        assert_eq!(step_zoom(1.0, -1), 0.9);
        assert_eq!(step_zoom(1.75, 1), 1.75);
        assert_eq!(step_zoom(0.75, -1), 0.75);
        assert_eq!(step_zoom(1.33, 1), 1.1, "unknown values restart from 100%");
    }

    #[test]
    fn redaction() {
        assert_eq!(
            NotificationContent::NameOnly.redact("Alice", "secret"),
            ("Alice".into(), "New message".into())
        );
        assert_eq!(
            NotificationContent::NoNameOrMessage.redact("Alice", "secret"),
            ("Halyard".into(), "New message".into())
        );
    }
}
