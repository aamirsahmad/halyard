//! Right pane: conversation header, message timeline and composer.

use gpui_kit::ExternalPaths;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    div,
};
use signal_core::Conversation;

use crate::app::SignalApp;
use crate::assets::Icons;
use crate::theme::Palette;
use crate::views::avatar::Avatar;
use crate::views::message::render_timeline_item;
use crate::views::rem_px;

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
        let drop_bg = p.hover;
        div()
            .id("conversation-pane")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(p.background)
            .child(self.render_header(&conv, p, cx))
            .children(self.render_conversation_search(p, cx))
            .child(self.render_timeline(p, window, cx))
            .children(self.render_typing(&conv, p))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_bg))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_attachments(paths.paths().to_vec(), cx);
            }))
            .child(if conv.blocked || !conv.accepted {
                self.render_request_banner(&conv, p, cx)
            } else {
                self.render_composer(p, cx)
            })
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
        } else if self.is_note_to_self(&id) {
            "Only you can see these messages".to_string()
        } else {
            self.db
                .contact(&id.direct_uuid().unwrap_or_default())
                .ok()
                .flatten()
                .and_then(|c| c.phone)
                .unwrap_or_default()
        };
        let entity = cx.entity().downgrade();
        let menu_conv = conv.clone();
        let note = self.is_note_to_self(&id);
        div()
            .id("conversation-header")
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(rem_px(56.))
            .px_4()
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .id("conversation-details")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.show_details(id, window, cx)),
                    )
                    .child(
                        Avatar::new(id.color_seed(), conv.title.clone())
                            .note_to_self(self.is_note_to_self(&id))
                            .group(id.is_group())
                            .size(rem_px(32.)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .id("conversation-title")
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(conv.title.clone())
                                            .test_support(),
                                    )
                                    .when_some(conv.expire_timer, |d, secs| {
                                        d.child(
                                            div()
                                                .id("header-timer")
                                                .flex()
                                                .flex_none()
                                                .items_center()
                                                .gap_1()
                                                .text_size(rem_px(12.))
                                                .text_color(p.text_secondary)
                                                .child(Icon::new(Icons::Clock).xsmall())
                                                .child(signal_core::timer_label(secs))
                                                .test_support(),
                                        )
                                    }),
                            )
                            .when(!subtitle.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(rem_px(12.))
                                        .text_color(p.text_secondary)
                                        .child(subtitle),
                                )
                            }),
                    )
                    .test_support(),
            )
            .child(
                Button::new("conversation-search")
                    .ghost()
                    .small()
                    .icon(IconName::Search)
                    .tooltip("Search in chat")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_conversation_search(window, cx)
                    })),
            )
            .child(
                Button::new("conversation-menu")
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .tooltip("More")
                    .dropdown_menu(move |menu, window, cx| {
                        crate::views::menus::conversation_menu(
                            menu,
                            entity.clone(),
                            &menu_conv,
                            note,
                            crate::views::menus::MenuPlace::Header,
                            window,
                            cx,
                        )
                    }),
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
            s.padding.bottom = Some(rem_px(0.).into());
            s.padding.left = Some(rem_px(16.).into());
            s.padding.right = Some(rem_px(16.).into());
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

    fn render_conversation_search(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.conv_search_open {
            return None;
        }
        let query_len = self.conv_search.read(cx).value().trim().chars().count();
        let results: Vec<AnyElement> = self
            .conv_search_results
            .iter()
            .map(|m| {
                let msg = m.clone();
                let who = if m.outgoing {
                    "You".to_string()
                } else {
                    self.display_name(&m.author)
                };
                div()
                    .id(ElementId::Name(format!("search-result-{}", m.id).into()))
                    .flex()
                    .flex_col()
                    .px_3()
                    .py_2()
                    .rounded(rem_px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.jump_to_result(msg.clone(), window, cx)
                    }))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(rem_px(12.))
                            .text_color(p.text_secondary)
                            .child(who)
                            .child(signal_core::format::conversation_list_time(
                                m.sent_at,
                                signal_core::now_millis(),
                            )),
                    )
                    .child(div().truncate().text_size(rem_px(13.)).child(m.preview()))
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let empty = results.is_empty() && query_len >= 2;
        Some(
            div()
                .id("conversation-search-bar")
                .flex()
                .flex_col()
                .flex_none()
                .gap_2()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div().flex_1().child(
                                gpui_kit::component::input::Input::new(&self.conv_search)
                                    .id("conversation-search-input")
                                    .small(),
                            ),
                        )
                        .child(
                            Button::new("conversation-search-close")
                                .ghost()
                                .small()
                                .label("Done")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_conversation_search(window, cx)
                                })),
                        ),
                )
                .when(!results.is_empty(), |d| {
                    d.child(
                        div()
                            .id("conversation-search-results")
                            .max_h(rem_px(260.))
                            .overflow_y_scroll()
                            .children(results),
                    )
                })
                .when(empty, |d| {
                    d.child(
                        div()
                            .text_size(rem_px(13.))
                            .text_color(p.text_secondary)
                            .child("No results"),
                    )
                })
                .test_support()
                .into_any_element(),
        )
    }

    /// Shown instead of the composer for message requests and blocked chats.
    fn render_request_banner(
        &self,
        conv: &Conversation,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = conv.id;
        let (text, buttons): (String, Vec<AnyElement>) = if conv.blocked {
            (
                format!(
                    "You blocked {}. You won't receive their messages.",
                    conv.title
                ),
                vec![
                    Button::new("unblock")
                        .primary()
                        .label("Unblock")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_blocked(id, false, cx)),
                        )
                        .into_any_element(),
                ],
            )
        } else {
            (
                format!(
                    "{} wants to message you. Accept to let them see that you've read their messages and to reply.",
                    conv.title
                ),
                vec![
                    Button::new("request-delete")
                        .outline()
                        .label("Delete")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_delete_conversation(id, window, cx)
                        }))
                        .into_any_element(),
                    Button::new("request-block")
                        .outline()
                        .label("Block")
                        .on_click(cx.listener(move |this, _, _, cx| this.block_and_delete(id, cx)))
                        .into_any_element(),
                    Button::new("request-accept")
                        .primary()
                        .label("Accept")
                        .on_click(cx.listener(move |this, _, _, cx| this.accept_request(id, cx)))
                        .into_any_element(),
                ],
            )
        };
        div()
            .id("request-banner")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_3()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(p.border)
            .child(
                div()
                    .text_center()
                    .text_size(rem_px(13.))
                    .text_color(p.text_secondary)
                    .child(text),
            )
            .child(div().flex().gap_2().children(buttons))
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
                .text_size(rem_px(12.))
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
                .rounded(rem_px(8.))
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
                                .text_size(rem_px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(author),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rem_px(13.))
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

        let editing = self.editing.as_ref().map(|m| {
            div()
                .id("edit-banner")
                .flex()
                .items_center()
                .gap_2()
                .mx_4()
                .mt_2()
                .p_2()
                .rounded(rem_px(8.))
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
                                .text_size(rem_px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Edit message"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rem_px(13.))
                                .text_color(p.text_secondary)
                                .child(m.preview()),
                        ),
                )
                .child(
                    Button::new("cancel-edit")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Icons::X))
                        .tooltip("Cancel editing")
                        .on_click(cx.listener(|this, _, window, cx| this.cancel_edit(window, cx))),
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
                                .rounded(rem_px(8.))
                                .bg(p.input_bg)
                                .text_size(rem_px(12.))
                                .child(Icon::new(Icons::Paperclip).xsmall())
                                .child(div().max_w(rem_px(180.)).truncate().child(name))
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
            .children(editing)
            .children(attachments)
            .child(match self.recorder.as_ref().map(|r| r.elapsed()) {
                Some(elapsed) => div()
                    .id("recording-bar")
                    .flex()
                    .items_center()
                    .gap_3()
                    .mx_4()
                    .mt_2()
                    .px_4()
                    .py_2()
                    .rounded(rem_px(18.))
                    .bg(p.input_bg)
                    .child(div().size(rem_px(10.)).rounded_full().bg(p.danger))
                    .child(div().flex_1().child(format!(
                        "Recording… {}",
                        crate::audio::format_clock(elapsed)
                    )))
                    .child(
                        Button::new("recording-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_recording(cx))),
                    )
                    .child(
                        Button::new("recording-send")
                            .primary()
                            .small()
                            .icon(Icon::new(Icons::SendHorizontal))
                            .label("Send")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_recording(window, cx)),
                            ),
                    )
                    .test_support()
                    .into_any_element(),
                None => div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .px_4()
                    .pt_2()
                    .child(
                        Popover::new("emoji-popover")
                            .anchor(gpui_kit::Anchor::BottomLeft)
                            .trigger(
                                Button::new("emoji")
                                    .ghost()
                                    .icon(Icon::new(Icons::FaceSlightlySmiling))
                                    .tooltip("Emoji"),
                            )
                            .content({
                                let picker = self.composer_emoji.clone();
                                move |_, _, _| picker.clone()
                            }),
                    )
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
                            .rounded(rem_px(18.))
                            .bg(p.input_bg)
                            .px_2()
                            .child(
                                Textarea::new(&self.composer)
                                    .appearance(false)
                                    .aria_label("Message"),
                            ),
                    )
                    .child(if can_send || self.editing.is_some() {
                        Button::new("send")
                            .when(can_send, |b| b.primary())
                            .when(!can_send, |b| b.ghost())
                            .icon(Icon::new(Icons::SendHorizontal))
                            .tooltip("Send")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_composed(window, cx)),
                            )
                            .into_any_element()
                    } else {
                        Button::new("record")
                            .ghost()
                            .icon(Icon::new(Icons::Mic))
                            .tooltip("Record voice message")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_recording(window, cx)),
                            )
                            .into_any_element()
                    })
                    .into_any_element(),
            })
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
                .size(rem_px(72.))
                .rounded_full()
                .bg(p.accent)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(Icons::MessageCircle)
                        .size(rem_px(36.))
                        .text_color(p.accent_text),
                ),
        )
        .child(
            div()
                .text_size(rem_px(22.))
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
                .text_size(rem_px(12.))
                .text_color(p.text_secondary)
                .child(Icon::new(Icons::Lock).xsmall())
                .child("Messages are end-to-end encrypted"),
        )
        .test_support()
        .into_any_element()
}
