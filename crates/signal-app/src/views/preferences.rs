//! Preferences, keyboard shortcuts and profile dialogs.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::switch::Switch;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, Keystroke,
    ParentElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window, div, px,
};
use signal_core::AccountState;

use crate::app::SignalApp;
use crate::settings::{NotificationContent, Preferences};
use crate::theme::{Appearance, Palette};
use crate::views::avatar::Avatar;

fn section(title: &'static str, p: &Palette) -> AnyElement {
    div()
        .pt_3()
        .pb_1()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text_secondary)
        .child(title)
        .into_any_element()
}

fn switch(
    app: &WeakEntity<SignalApp>,
    id: &'static str,
    label: &'static str,
    checked: bool,
    set: fn(&mut Preferences, bool),
) -> AnyElement {
    let app = app.clone();
    div()
        .py_1()
        .child(Switch::new(id).label(label).checked(checked).on_click(
            move |v: &bool, window, cx| {
                let v = *v;
                let _ = app.update(cx, |this, cx| this.update_prefs(|p| set(p, v), window, cx));
            },
        ))
        .into_any_element()
}

/// Shortcut reference shown in the shortcuts dialog.
#[cfg(target_os = "macos")]
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("cmd-n", "New chat"),
    ("cmd-f", "Search chats"),
    ("alt-down", "Next chat"),
    ("alt-up", "Previous chat"),
    ("cmd-,", "Preferences"),
    ("cmd-/", "Keyboard shortcuts"),
    ("enter", "Send message"),
    ("shift-enter", "New line"),
    ("escape", "Cancel reply / edit, close panels"),
    ("cmd-q", "Quit"),
];
#[cfg(not(target_os = "macos"))]
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("ctrl-n", "New chat"),
    ("ctrl-f", "Search chats"),
    ("alt-down", "Next chat"),
    ("alt-up", "Previous chat"),
    ("ctrl-,", "Preferences"),
    ("ctrl-/", "Keyboard shortcuts"),
    ("enter", "Send message"),
    ("shift-enter", "New line"),
    ("escape", "Cancel reply / edit, close panels"),
    ("ctrl-q", "Quit"),
];

impl SignalApp {
    pub fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = Palette::current(cx);
            let Some(prefs) = app.upgrade().map(|e| e.read(cx).prefs().clone()) else {
                return dialog;
            };
            let a_theme = app.clone();
            let a_content = app.clone();
            let theme_ix = match prefs.appearance {
                Appearance::System => 0,
                Appearance::Light => 1,
                Appearance::Dark => 2,
            };
            let content_ix = NotificationContent::ALL
                .iter()
                .position(|c| *c == prefs.notification_content)
                .unwrap_or(0);
            dialog.title("Preferences").w(px(480.)).child(
                div()
                    .id("preferences")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(section("Appearance", &p))
                    .child(
                        RadioGroup::horizontal("pref-theme")
                            .children(["System", "Light", "Dark"])
                            .selected_index(Some(theme_ix))
                            .on_change(move |ix: &usize, window, cx| {
                                let a =
                                    [Appearance::System, Appearance::Light, Appearance::Dark][*ix];
                                let _ = a_theme
                                    .update(cx, |this, cx| this.set_appearance(a, window, cx));
                            }),
                    )
                    .child(section("Notifications", &p))
                    .child(switch(
                        &app,
                        "pref-notifications",
                        "Show notifications",
                        prefs.notifications,
                        |p, v| p.notifications = v,
                    ))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(p.text_secondary)
                            .child("Show"),
                    )
                    .child(
                        RadioGroup::vertical("pref-notification-content")
                            .children(NotificationContent::ALL.map(|c| c.label()))
                            .selected_index(Some(content_ix))
                            .on_change(move |ix: &usize, window, cx| {
                                let c = NotificationContent::ALL[*ix];
                                let _ = a_content.update(cx, |this, cx| {
                                    this.update_prefs(|p| p.notification_content = c, window, cx)
                                });
                            }),
                    )
                    .child(section("Chats", &p))
                    .child(switch(
                        &app,
                        "pref-enter-sends",
                        "Send messages with Enter",
                        prefs.enter_sends,
                        |p, v| p.enter_sends = v,
                    ))
                    .child(section("Privacy", &p))
                    .child(switch(
                        &app,
                        "pref-read-receipts",
                        "Read receipts",
                        prefs.read_receipts,
                        |p, v| p.read_receipts = v,
                    ))
                    .child(switch(
                        &app,
                        "pref-typing",
                        "Typing indicators",
                        prefs.typing_indicators,
                        |p, v| p.typing_indicators = v,
                    ))
                    .child(div().text_size(px(12.)).text_color(p.text_secondary).child(
                        "If read receipts or typing indicators are off, you won't send them.",
                    ))
                    .test_support(),
            )
        });
    }

    pub fn open_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = Palette::current(cx);
            dialog.title("Keyboard shortcuts").w(px(420.)).child(
                div()
                    .id("shortcuts")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(SHORTCUTS.iter().map(|(keys, label)| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_color(p.text).child(*label))
                            .child({
                                match Keystroke::parse(keys) {
                                    Ok(k) => Kbd::new(k).into_any_element(),
                                    Err(_) => div().child(*keys).into_any_element(),
                                }
                            })
                    }))
                    .test_support(),
            )
        });
    }

    pub fn open_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let AccountState::Registered(info) = self.state.account.clone() else {
            return;
        };
        let app = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = Palette::current(cx);
            let row = |label: &'static str, value: String| {
                div()
                    .flex()
                    .gap_3()
                    .py_1()
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_color(p.text_secondary)
                            .child(label),
                    )
                    .child(div().flex_1().child(value))
            };
            let app = app.clone();
            dialog
                .title("Profile")
                .w(px(440.))
                .child(
                    div()
                        .id("profile")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(px(13.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .pb_2()
                                .child(
                                    Avatar::new(info.aci.as_bytes().to_vec(), "You").size(px(72.)),
                                )
                                .child(
                                    div()
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(
                                            info.phone_number
                                                .clone()
                                                .unwrap_or_else(|| "Your account".into()),
                                        ),
                                ),
                        )
                        .child(row(
                            "Device",
                            info.device_name
                                .clone()
                                .unwrap_or_else(|| "This computer".into()),
                        ))
                        .child(row("Device ID", info.device_id.to_string()))
                        .child(row("Signal ID", info.aci.to_string()))
                        .test_support(),
                )
                .footer(
                    div().flex().justify_end().child(
                        Button::new("profile-unlink")
                            .danger()
                            .label("Unlink this device…")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = app.update(cx, |this, cx| this.confirm_unlink(window, cx));
                            }),
                    ),
                )
        });
    }
}
