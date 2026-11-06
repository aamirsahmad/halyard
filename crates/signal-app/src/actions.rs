//! Keyboard actions and their default bindings.

use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(
    signal,
    [
        NewChat,
        FocusSearch,
        NextChat,
        PrevChat,
        CloseChat,
        OpenPreferences,
        ShowShortcuts,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Quit
    ]
);

/// Shortcuts using the platform's primary modifier: ⌘ on macOS, Ctrl elsewhere.
#[cfg(target_os = "macos")]
mod keys {
    pub const NEW_CHAT: &str = "cmd-n";
    pub const FIND: &str = "cmd-f";
    pub const QUIT: &str = "cmd-q";
    pub const PREFS: &str = "cmd-,";
    pub const SHORTCUTS: &str = "cmd-/";
    pub const ZOOM_IN: &str = "cmd-=";
    pub const ZOOM_IN_PLUS: &str = "cmd-+";
    pub const ZOOM_OUT: &str = "cmd--";
    pub const ZOOM_RESET: &str = "cmd-0";
}
#[cfg(not(target_os = "macos"))]
mod keys {
    pub const NEW_CHAT: &str = "ctrl-n";
    pub const FIND: &str = "ctrl-f";
    pub const QUIT: &str = "ctrl-q";
    pub const PREFS: &str = "ctrl-,";
    pub const SHORTCUTS: &str = "ctrl-/";
    pub const ZOOM_IN: &str = "ctrl-=";
    pub const ZOOM_IN_PLUS: &str = "ctrl-+";
    pub const ZOOM_OUT: &str = "ctrl--";
    pub const ZOOM_RESET: &str = "ctrl-0";
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(keys::NEW_CHAT, NewChat, Some("SignalApp")),
        KeyBinding::new(keys::FIND, FocusSearch, Some("SignalApp")),
        KeyBinding::new("alt-down", NextChat, Some("SignalApp")),
        KeyBinding::new("alt-up", PrevChat, Some("SignalApp")),
        KeyBinding::new("ctrl-tab", NextChat, Some("SignalApp")),
        KeyBinding::new("ctrl-shift-tab", PrevChat, Some("SignalApp")),
        KeyBinding::new("escape", CloseChat, Some("SignalApp")),
        KeyBinding::new(keys::PREFS, OpenPreferences, Some("SignalApp")),
        KeyBinding::new(keys::SHORTCUTS, ShowShortcuts, Some("SignalApp")),
        KeyBinding::new(keys::ZOOM_IN, ZoomIn, Some("SignalApp")),
        KeyBinding::new(keys::ZOOM_IN_PLUS, ZoomIn, Some("SignalApp")),
        KeyBinding::new(keys::ZOOM_OUT, ZoomOut, Some("SignalApp")),
        KeyBinding::new(keys::ZOOM_RESET, ZoomReset, Some("SignalApp")),
        KeyBinding::new(keys::QUIT, Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}
