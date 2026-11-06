//! Preferences, keyboard shortcuts and profile dialogs.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, Keystroke,
    ParentElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window, div,
};
use signal_core::AccountState;

use crate::app::SignalApp;
use crate::settings::{NotificationContent, Preferences};
use crate::theme::{Appearance, Palette};
use crate::views::avatar::Avatar;
use crate::views::rem_px;

fn section(title: &'static str, p: &Palette) -> AnyElement {
    div()
        .pt_3()
        .pb_1()
        .text_size(rem_px(12.))
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
    ("cmd-=", "Zoom in"),
    ("cmd--", "Zoom out"),
    ("cmd-0", "Actual size"),
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
    ("ctrl-=", "Zoom in"),
    ("ctrl--", "Zoom out"),
    ("ctrl-0", "Actual size"),
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
            dialog.title("Preferences").w(gpui_kit::px(480.)).child(
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
                    .child(section("Zoom", &p))
                    .child({
                        let (a_out, a_in, a_reset) = (app.clone(), app.clone(), app.clone());
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Button::new("zoom-out").outline().small().label("−").on_click(move |_, w, cx| {
                                let _ = a_out.update(cx, |this, cx| this.zoom(-1, w, cx));
                            }))
                            .child(
                                div()
                                    .id("zoom-level")
                                    .w(rem_px(56.))
                                    .text_center()
                                    .child(format!("{:.0}%", prefs.zoom * 100.))
                                    .test_support(),
                            )
                            .child(Button::new("zoom-in").outline().small().label("+").on_click(move |_, w, cx| {
                                let _ = a_in.update(cx, |this, cx| this.zoom(1, w, cx));
                            }))
                            .child(Button::new("zoom-reset").ghost().small().label("Reset").on_click(move |_, w, cx| {
                                let _ = a_reset.update(cx, |this, cx| this.zoom(0, w, cx));
                            }))
                    })
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
                            .text_size(rem_px(13.))
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
                    .child(div().text_size(rem_px(12.)).text_color(p.text_secondary).child(
                        "If read receipts or typing indicators are off, you won't send them.",
                    ))
                    .test_support(),
            )
        });
    }

    pub fn open_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = Palette::current(cx);
            dialog
                .title("Keyboard shortcuts")
                .w(gpui_kit::px(420.))
                .child(
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
        // prefill the editor with what we know about ourselves
        let me_conv = signal_core::ConversationId::Direct(info.aci);
        let full = self
            .db
            .contact(&info.aci)
            .ok()
            .flatten()
            .and_then(|c| c.profile_name.or(c.name))
            .unwrap_or_default();
        let (given, family) = match full.split_once(' ') {
            Some((g, f)) => (g.to_string(), f.to_string()),
            None => (full, String::new()),
        };
        let about = self
            .db
            .conversation(&me_conv)
            .ok()
            .flatten()
            .and_then(|c| c.about)
            .unwrap_or_default();
        self.profile_given
            .update(cx, |s, cx| s.set_value(given, window, cx));
        self.profile_family
            .update(cx, |s, cx| s.set_value(family, window, cx));
        self.profile_about
            .update(cx, |s, cx| s.set_value(about, window, cx));
        let (given_input, family_input, about_input) = (
            self.profile_given.clone(),
            self.profile_family.clone(),
            self.profile_about.clone(),
        );
        let photo = self.photo_for(&me_conv);
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = Palette::current(cx);
            let row = |label: &'static str, value: String| {
                div()
                    .flex()
                    .gap_3()
                    .py_1()
                    .child(
                        div()
                            .w(rem_px(110.))
                            .flex_none()
                            .text_color(p.text_secondary)
                            .child(label),
                    )
                    .child(div().flex_1().child(value))
            };
            let app = app.clone();
            dialog
                .title("Profile")
                .w(gpui_kit::px(440.))
                .child(
                    div()
                        .id("profile")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(rem_px(13.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .pb_2()
                                .child(
                                    Avatar::new(info.aci.as_bytes().to_vec(), "You")
                                        .photo(photo.clone())
                                        .size(rem_px(72.)),
                                )
                                .child(
                                    div()
                                        .text_size(rem_px(16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(
                                            info.phone_number
                                                .clone()
                                                .unwrap_or_else(|| "Your account".into()),
                                        ),
                                ),
                        )
                        .child(
                            gpui_kit::component::input::Input::new(&given_input)
                                .id("profile-given"),
                        )
                        .child(
                            gpui_kit::component::input::Input::new(&family_input)
                                .id("profile-family"),
                        )
                        .child(
                            gpui_kit::component::input::Input::new(&about_input)
                                .id("profile-about"),
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
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child({
                            let (app, g, f, ab) = (
                                app.clone(),
                                given_input.clone(),
                                family_input.clone(),
                                about_input.clone(),
                            );
                            Button::new("profile-save")
                                .primary()
                                .label("Save")
                                .on_click(move |_, window, cx| {
                                    let given = g.read(cx).value().trim().to_string();
                                    if given.is_empty() {
                                        window.push_notification(
                                            gpui_kit::component::notification::Notification::error(
                                                "A first name is required",
                                            ),
                                            cx,
                                        );
                                        return;
                                    }
                                    let family = Some(f.read(cx).value().trim().to_string())
                                        .filter(|s| !s.is_empty());
                                    let about = Some(ab.read(cx).value().trim().to_string())
                                        .filter(|s| !s.is_empty());
                                    let _ = app.update(cx, |this, cx| {
                                        this.commands.send(
                                            signal_core::BackendCommand::UpdateProfile {
                                                given_name: given,
                                                family_name: family,
                                                about,
                                            },
                                        );
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                })
                        })
                        .child(
                            Button::new("profile-unlink")
                                .danger()
                                .label("Unlink this device…")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ =
                                        app.update(cx, |this, cx| this.confirm_unlink(window, cx));
                                }),
                        ),
                )
        });
    }
}
