//! Keyboard actions and their default bindings.

use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(signal, [NewChat, FocusSearch, NextChat, PrevChat, CloseChat, Quit]);

/// Shortcuts using the platform's primary modifier: ⌘ on macOS, Ctrl elsewhere.
#[cfg(target_os = "macos")]
mod keys {
    pub const NEW_CHAT: &str = "cmd-n";
    pub const FIND: &str = "cmd-f";
    pub const QUIT: &str = "cmd-q";
}
#[cfg(not(target_os = "macos"))]
mod keys {
    pub const NEW_CHAT: &str = "ctrl-n";
    pub const FIND: &str = "ctrl-f";
    pub const QUIT: &str = "ctrl-q";
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
        KeyBinding::new(keys::QUIT, Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}
