//! Signal Desktop's colour palette, for light and dark appearance.

use gpui_kit::{App, Hsla, rgb};

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};

/// Signal's "ultramarine" accent.
pub const ULTRAMARINE: u32 = 0x2C6BED;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub background: Hsla,
    pub pane: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub bubble_incoming: Hsla,
    pub bubble_incoming_text: Hsla,
    pub bubble_outgoing: Hsla,
    pub bubble_outgoing_text: Hsla,
    pub bubble_outgoing_meta: Hsla,
    pub quote_incoming: Hsla,
    pub quote_outgoing: Hsla,
    pub input_bg: Hsla,
    pub badge: Hsla,
    pub danger: Hsla,
    pub warning_bg: Hsla,
    pub reaction_bg: Hsla,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            background: rgb(0xFFFFFF).into(),
            pane: rgb(0xFFFFFF).into(),
            border: rgb(0xE9E9E9).into(),
            text: rgb(0x1B1B1B).into(),
            text_secondary: rgb(0x5E5E5E).into(),
            accent: rgb(ULTRAMARINE).into(),
            accent_text: rgb(0xFFFFFF).into(),
            hover: rgb(0xF6F6F6).into(),
            selected: rgb(0xE9E9E9).into(),
            bubble_incoming: rgb(0xE9E9E9).into(),
            bubble_incoming_text: rgb(0x1B1B1B).into(),
            bubble_outgoing: rgb(ULTRAMARINE).into(),
            bubble_outgoing_text: rgb(0xFFFFFF).into(),
            bubble_outgoing_meta: gpui_kit::hsla(0., 0., 1., 0.8),
            quote_incoming: gpui_kit::hsla(0., 0., 1., 0.6),
            quote_outgoing: gpui_kit::hsla(0., 0., 1., 0.25),
            input_bg: rgb(0xF6F6F6).into(),
            badge: rgb(ULTRAMARINE).into(),
            danger: rgb(0xD92D20).into(),
            warning_bg: rgb(0xFFF4D1).into(),
            reaction_bg: rgb(0xF6F6F6).into(),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            background: rgb(0x121212).into(),
            pane: rgb(0x1B1B1B).into(),
            border: rgb(0x2E2E2E).into(),
            text: rgb(0xE9E9E9).into(),
            text_secondary: rgb(0xA4A4A4).into(),
            accent: rgb(ULTRAMARINE).into(),
            accent_text: rgb(0xFFFFFF).into(),
            hover: rgb(0x262626).into(),
            selected: rgb(0x303030).into(),
            bubble_incoming: rgb(0x3B3B3B).into(),
            bubble_incoming_text: rgb(0xE9E9E9).into(),
            bubble_outgoing: rgb(ULTRAMARINE).into(),
            bubble_outgoing_text: rgb(0xFFFFFF).into(),
            bubble_outgoing_meta: gpui_kit::hsla(0., 0., 1., 0.8),
            quote_incoming: gpui_kit::hsla(0., 0., 0., 0.25),
            quote_outgoing: gpui_kit::hsla(0., 0., 1., 0.25),
            input_bg: rgb(0x2E2E2E).into(),
            badge: rgb(ULTRAMARINE).into(),
            danger: rgb(0xF97066).into(),
            warning_bg: rgb(0x4A3B0B).into(),
            reaction_bg: rgb(0x2E2E2E).into(),
        }
    }

    /// Palette matching the active GPUI Kit theme mode.
    pub fn current(cx: &App) -> Self {
        if cx.theme().mode.is_dark() {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

/// User preference for the colour scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    fn as_str(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "system" => Some(Appearance::System),
            "light" => Some(Appearance::Light),
            "dark" => Some(Appearance::Dark),
            _ => None,
        }
    }

    /// Reads the saved preference (`appearance=<value>` lines); defaults to System.
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("appearance=").and_then(Self::parse))
            })
            .unwrap_or_default()
    }

    pub fn save(self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, format!("appearance={}\n", self.as_str()))
    }
}

/// Applies the appearance preference to the GPUI Kit theme.
pub fn apply_appearance(
    appearance: Appearance,
    window: Option<&mut gpui_kit::Window>,
    cx: &mut App,
) {
    match appearance {
        Appearance::System => Theme::sync_system_appearance(window, cx),
        Appearance::Light => Theme::change(ThemeMode::Light, window, cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

/// Converts a 0xRRGGBB value to a colour.
pub fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.conf");
        assert_eq!(Appearance::load(&path), Appearance::System);
        Appearance::Dark.save(&path).unwrap();
        assert_eq!(Appearance::load(&path), Appearance::Dark);
        std::fs::write(&path, "appearance=purple\n").unwrap();
        assert_eq!(Appearance::load(&path), Appearance::System);
    }

    #[test]
    fn palettes_differ_and_keep_accent() {
        let l = Palette::light();
        let d = Palette::dark();
        assert!(!l.dark && d.dark);
        assert_eq!(l.accent, d.accent);
        assert_ne!(l.background, d.background);
        // outgoing text must contrast with the accent bubble
        assert!(l.bubble_outgoing_text.l > l.bubble_outgoing.l);
    }
}
