//! Right pane: conversation header, message timeline and composer.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, TestSupportExt as _, Window, div, px,
};
use signal_core::Conversation;

use crate::app::SignalApp;
use crate::assets::Icons;
use crate::theme::Palette;
use crate::views::avatar::Avatar;
use crate::views::message::render_timeline_item;

impl SignalApp {
    pub(super) fn render_conversation_pane(
        &mut self,
        p: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(conv) = self.state.selected_conversation().cloned() else {
            return render_welcome(p);
        };
        div()
            .id("conversation-pane")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(p.background)
            .child(self.render_header(&conv, p, cx))
            .child(self.render_timeline(p, window, cx))
            .children(self.render_typing(&conv, p))
            .child(self.render_composer(p, cx))
            .test_support()
            .into_any_element()
    }

    fn render_header(
        &self,
        conv: &Conversation,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = conv.id;
        let subtitle = if conv.is_group() {
            format!("{} members", conv.members.len())
        } else {
            self.db
                .contact(&id.direct_uuid().unwrap_or_default())
                .ok()
                .flatten()
                .and_then(|c| c.phone)
                .unwrap_or_default()
        };
        let entity = cx.entity().downgrade();
        let (pinned, muted, archived) = (conv.pinned, conv.muted, conv.archived);
        div()
            .id("conversation-header")
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(56.))
            .px_4()
            .border_b_1()
            .border_color(p.border)
            .child(Avatar::new(id.color_seed(), conv.title.clone()).group(id.is_group()).size(px(32.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .id("conversation-title")
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(conv.title.clone())
            .test_support(),
                    )
                    .when(!subtitle.is_empty(), |d| {
                        d.child(div().text_size(px(12.)).text_color(p.text_secondary).child(subtitle))
                    }),
            )
            .child(
                Button::new("conversation-menu").ghost().small().icon(IconName::Ellipsis).tooltip("More").dropdown_menu(
                    move |menu, _, _| {
                        let action = |label: &'static str, f: fn(&mut SignalApp, &mut Context<SignalApp>)| {
                            let e = entity.clone();
                            PopupMenuItem::new(label).on_click(move |_, _, cx| {
                                let _ = e.update(cx, |this, cx| f(this, cx));
                            })
                        };
                        let toggle = move |this: &mut SignalApp, cx: &mut Context<SignalApp>, which: u8| match which {
                            0 => this.toggle_pinned(id, cx),
                            1 => this.toggle_muted(id, cx),
                            _ => this.toggle_archived(id, cx),
                        };
                        let e1 = entity.clone();
                        let e2 = entity.clone();
                        let e3 = entity.clone();
                        menu.item(
                            PopupMenuItem::new(if pinned { "Unpin chat" } else { "Pin chat" })
                                .icon(Icon::new(if pinned { Icons::PinOff } else { Icons::Pin }))
                                .on_click(move |_, _, cx| {
                                    let _ = e1.update(cx, |this, cx| toggle(this, cx, 0));
                                }),
                        )
                        .item(
                            PopupMenuItem::new(if muted { "Unmute notifications" } else { "Mute notifications" })
                                .icon(Icon::new(Icons::BellOff))
                                .on_click(move |_, _, cx| {
                                    let _ = e2.update(cx, |this, cx| toggle(this, cx, 1));
                                }),
                        )
                        .item(
                            PopupMenuItem::new(if archived { "Unarchive" } else { "Archive" })
                                .icon(Icon::new(if archived { Icons::ArchiveRestore } else { Icons::Archive }))
                                .on_click(move |_, _, cx| {
                                    let _ = e3.update(cx, |this, cx| toggle(this, cx, 2));
                                }),
                        )
                        .separator()
                        .item(action("Close chat", |this, cx| this.close_conversation(cx)))
                    },
                ),
            )
            .test_support()
            .into_any_element()
    }

    fn render_timeline(
        &mut self,
        p: &Palette,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let data = self.timeline.clone();
        let palette = *p;
        let weak = cx.entity().downgrade();
        let has_more = self.state.has_more_history;
        let row_style = {
            let mut s = gpui_kit::StyleRefinement::default();
            s.padding.bottom = Some(px(0.).into());
            s.padding.left = Some(px(16.).into());
            s.padding.right = Some(px(16.).into());
            s
        };
        div()
            .id("timeline")
            .flex_1()
            .min_h_0()
            .child(
                MessageScroller::new(
                    "messages",
                    self.scroller.clone(),
                    move |index, window, cx| {
                        if index == 0 && has_more {
                            let weak = weak.clone();
                            cx.defer(move |cx| {
                                let _ = weak.update(cx, |this, cx| this.load_older(cx));
                            });
                        }
                        render_timeline_item(&data, index, &palette, weak.clone(), window, cx)
                    },
                )
                .with_row_style(row_style)
                .with_jump_button_label("Jump to latest")
                .size_full(),
            )
            .test_support()
            .into_any_element()
    }

    fn render_typing(&self, conv: &Conversation, p: &Palette) -> Option<AnyElement> {
        let typing = self.state.typing_in(&conv.id);
        if typing.is_empty() {
            return None;
        }
        let names: Vec<String> = typing.iter().map(|u| self.display_name(u)).collect();
        let text = match (conv.is_group(), names.len()) {
            (false, _) => "typing…".to_string(),
            (true, 1) => format!("{} is typing…", names[0]),
            (true, 2) => format!("{} and {} are typing…", names[0], names[1]),
            (true, n) => format!("{n} people are typing…"),
        };
        Some(
            div()
                .id("typing-indicator")
                .px_6()
                .py_1()
                .text_size(px(12.))
                .italic()
                .text_color(p.text_secondary)
                .child(text)
                .test_support()
                .into_any_element(),
        )
    }

    fn render_composer(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let reply = self.replying_to.as_ref().map(|m| {
            let author = if m.outgoing {
                "You".to_string()
            } else {
                self.display_name(&m.author)
            };
            div()
                .id("reply-preview")
                .flex()
                .items_center()
                .gap_2()
                .mx_4()
                .mt_2()
                .p_2()
                .rounded(px(8.))
                .bg(p.input_bg)
                .border_l_4()
                .border_color(p.accent)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(author),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.))
                                .text_color(p.text_secondary)
                                .child(m.preview()),
                        ),
                )
                .child(
                    Button::new("cancel-reply")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Icons::X))
                        .tooltip("Cancel reply")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_reply(cx))),
                )
                .test_support()
        });

        let attachments = (!self.pending_attachments.is_empty()).then(|| {
            div()
                .id("pending-attachments")
                .flex()
                .flex_wrap()
                .gap_2()
                .mx_4()
                .mt_2()
                .children(
                    self.pending_attachments
                        .iter()
                        .enumerate()
                        .map(|(i, path)| {
                            let name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            div()
                                .id(ElementId::Name(format!("pending-attachment-{i}").into()))
                                .flex()
                                .items_center()
                                .gap_1()
                                .pl_2()
                                .rounded(px(8.))
                                .bg(p.input_bg)
                                .text_size(px(12.))
                                .child(Icon::new(Icons::Paperclip).xsmall())
                                .child(div().max_w(px(180.)).truncate().child(name))
                                .child(
                                    Button::new(ElementId::Name(
                                        format!("remove-attachment-{i}").into(),
                                    ))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Icons::X))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| this.remove_attachment(i, cx),
                                    )),
                                )
                                .test_support()
                        }),
                )
        });

        let can_send = !self.composer.read(cx).value().trim().is_empty()
            || !self.pending_attachments.is_empty();
        div()
            .id("composer")
            .flex()
            .flex_col()
            .flex_none()
            .pb_3()
            .children(reply)
            .children(attachments)
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .px_4()
                    .pt_2()
                    .child(
                        Button::new("attach")
                            .ghost()
                            .icon(Icon::new(Icons::Paperclip))
                            .tooltip("Attach file")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.attach_files(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(px(18.))
                            .bg(p.input_bg)
                            .px_2()
                            .child(
                                Textarea::new(&self.composer)
                                    .appearance(false)
                                    .aria_label("Message"),
                            ),
                    )
                    .child(
                        Button::new("send")
                            .when(can_send, |b| b.primary())
                            .when(!can_send, |b| b.ghost())
                            .icon(Icon::new(Icons::SendHorizontal))
                            .tooltip("Send")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_composed(window, cx)),
                            ),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}

fn render_welcome(p: &Palette) -> AnyElement {
    div()
        .id("welcome")
        .flex()
        .flex_col()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_3()
        .bg(p.background)
        .child(
            div()
                .size(px(72.))
                .rounded_full()
                .bg(p.accent)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(Icons::MessageCircle)
                        .size(px(36.))
                        .text_color(p.accent_text),
                ),
        )
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Welcome to Signal"),
        )
        .child(
            div()
                .text_color(p.text_secondary)
                .child("Select a chat on the left, or start a new one."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .mt_4()
                .text_size(px(12.))
                .text_color(p.text_secondary)
                .child(Icon::new(Icons::Lock).xsmall())
                .child("Messages are end-to-end encrypted"),
        )
        .test_support()
        .into_any_element()
}
