//! Rendering of the root view.

pub mod avatar;
mod conversation;
pub mod details;
pub mod emoji_picker;
mod lightbox;
mod link;
pub mod menus;
mod message;
pub mod message_menu;
pub mod preferences;
pub mod rich_text;
mod sidebar;

pub use sidebar::{conversation_dom_id, conversation_element_id};

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div,
};
use signal_core::AccountState;

use crate::actions::{
    CloseChat, FocusSearch, NewChat, NextChat, OpenPreferences, PrevChat, ShowShortcuts,
};
use crate::app::{ListMode, SignalApp};
use crate::theme::Palette;

impl Render for SignalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        let root = div()
            .id("signal-root")
            .key_context("SignalApp")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(p.background)
            .text_color(p.text)
            .text_size(gpui_kit::px(14.));

        match &self.state.account {
            AccountState::Registered(_) => root
                .on_action(cx.listener(|this, _: &NewChat, window, cx| {
                    this.set_list_mode(ListMode::NewChat, window, cx)
                }))
                .on_action(
                    cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
                )
                .on_action(
                    cx.listener(|this, _: &NextChat, window, cx| {
                        this.select_relative(1, window, cx)
                    }),
                )
                .on_action(cx.listener(|this, _: &PrevChat, window, cx| {
                    this.select_relative(-1, window, cx)
                }))
                .on_action(cx.listener(|this, _: &OpenPreferences, window, cx| {
                    this.open_preferences(window, cx)
                }))
                .on_action(cx.listener(|this, _: &ShowShortcuts, window, cx| {
                    this.open_shortcuts(window, cx)
                }))
                .on_action(cx.listener(|this, _: &CloseChat, window, cx| {
                    if this.editing.is_some() {
                        this.cancel_edit(window, cx);
                    } else if this.replying_to.is_some() {
                        this.cancel_reply(cx);
                    } else if this.list_mode != ListMode::Chats {
                        this.set_list_mode(ListMode::Chats, window, cx);
                    }
                }))
                .child(self.render_sidebar(&p, window, cx))
                .child(self.render_conversation_pane(&p, window, cx)),
            _ => root.child(self.render_link_screen(&p, window, cx)),
        }
    }
}
