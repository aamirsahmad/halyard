//! Timeline rows: date separators and Signal-style message bubbles.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, StyledImage as _, WeakEntity,
    Window, div, img, px,
};
use signal_core::format::{file_size, message_time};
use signal_core::timeline::{TimelineItem, TimelineMessage};
use signal_core::{Attachment, Message, MessageStatus, avatar, now_millis};

use crate::app::{QUICK_REACTIONS, SignalApp, TimelineData};
use crate::assets::Icons;
use crate::theme::{Palette, hex};
use crate::views::avatar::Avatar;

const BUBBLE_RADIUS: f32 = 18.;
const GROUPED_RADIUS: f32 = 4.;
const MAX_BUBBLE_WIDTH: f32 = 480.;
const MAX_IMAGE: f32 = 300.;

pub fn message_element_id(m: &Message) -> ElementId {
    ElementId::Name(format!("message-{}", m.id).into())
}

pub fn render_timeline_item(
    data: &TimelineData,
    index: usize,
    p: &Palette,
    app: WeakEntity<SignalApp>,
    _: &mut Window,
    _: &mut App,
) -> AnyElement {
    match data.items.get(index) {
        Some(TimelineItem::DateSeparator(label)) => div()
            .flex()
            .justify_center()
            .py_3()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_secondary)
                    .child(label.clone()),
            )
            .into_any_element(),
        Some(TimelineItem::Message(tm)) => match data.messages.get(tm.index) {
            Some(m) => render_message(data, tm, m, p, app),
            None => div().into_any_element(),
        },
        None => div().into_any_element(),
    }
}

fn render_message(
    data: &TimelineData,
    tm: &TimelineMessage,
    m: &Message,
    p: &Palette,
    app: WeakEntity<SignalApp>,
) -> AnyElement {
    let outgoing = m.outgoing;
    let show_avatar_column = data.is_group && !outgoing;
    let author_name = data.names.get(&m.author).cloned().unwrap_or_else(|| "Unknown contact".into());
    let author_color = avatar::color_for(m.author.as_bytes());
    let (bg, fg, meta) = if outgoing {
        (p.bubble_outgoing, p.bubble_outgoing_text, p.bubble_outgoing_meta)
    } else {
        (p.bubble_incoming, p.bubble_incoming_text, p.text_secondary)
    };
    let big = px(BUBBLE_RADIUS);
    let small = px(GROUPED_RADIUS);
    // grouped bubbles get tight corners on the sender's side, like Signal
    let (tl, tr, bl, br) = if outgoing {
        (big, if tm.first_in_group { big } else { small }, big, if tm.last_in_group { big } else { small })
    } else {
        (if tm.first_in_group { big } else { small }, big, if tm.last_in_group { big } else { small }, big)
    };
    let id = m.id;

    let mut bubble = div()
        .id(message_element_id(m))
        .flex()
        .flex_col()
        .max_w(px(MAX_BUBBLE_WIDTH))
        .min_w(px(60.))
        .px_3()
        .py_2()
        .gap_1()
        .rounded_tl(tl)
        .rounded_tr(tr)
        .rounded_bl(bl)
        .rounded_br(br)
        .bg(bg)
        .text_color(fg);

    if show_avatar_column && tm.first_in_group {
        bubble = bubble.child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hex(if p.dark { author_color.bg } else { author_color.fg }))
                .child(author_name.clone()),
        );
    }

    if m.deleted {
        bubble = bubble.child(div().italic().child(if outgoing {
            "You deleted this message."
        } else {
            "This message was deleted."
        }));
    } else {
        if let Some(q) = &m.quote {
            let quoted_author = if Some(q.author) == data.me {
                "You".to_string()
            } else {
                data.names.get(&q.author).cloned().unwrap_or_else(|| "Unknown contact".into())
            };
            bubble = bubble.child(
                div()
                    .flex()
                    .flex_col()
                    .p_2()
                    .rounded(px(8.))
                    .border_l_4()
                    .border_color(if outgoing { p.bubble_outgoing_text } else { p.accent })
                    .bg(if outgoing { p.quote_outgoing } else { p.quote_incoming })
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(quoted_author))
                    .child(div().text_size(px(13.)).line_clamp(3).child(q.text.clone())),
            );
        }
        for (i, a) in m.attachments.iter().enumerate() {
            bubble = bubble.child(render_attachment(id, i, a, fg, meta));
        }
        if !m.body.is_empty() {
            bubble = bubble.child(div().text_size(px(14.)).line_height(px(20.)).child(m.body.clone()));
        }
    }

    // footer: edited • time • status
    bubble = bubble.child(
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap_1()
            .text_size(px(11.))
            .text_color(meta)
            .when(m.edited_at.is_some() && !m.deleted, |d| d.child("Edited"))
            .when_some(m.expires_in_secs, |d, _| d.child(Icon::new(Icons::Clock).xsmall()))
            .child(message_time(m.sent_at, now_millis()))
            .when(outgoing, |d| d.child(status_icon(m.status, meta))),
    );

    let bubble = bubble.context_menu({
        let app = app.clone();
        let deletable = outgoing && !m.deleted;
        let failed = m.status == MessageStatus::Failed;
        let has_text = !m.body.is_empty();
        move |menu, _, _| {
            let a = app.clone();
            let mut menu = menu.item(PopupMenuItem::new("Reply").icon(Icon::new(Icons::Reply)).on_click(
                move |_, window, cx| {
                    let _ = a.update(cx, |this, cx| this.reply_to(id, window, cx));
                },
            ));
            if has_text {
                let a = app.clone();
                menu = menu.item(PopupMenuItem::new("Copy text").on_click(move |_, _, cx| {
                    let _ = a.update(cx, |this, cx| this.copy_text(id, cx));
                }));
            }
            menu = menu.separator();
            for emoji in QUICK_REACTIONS {
                let a = app.clone();
                menu = menu.item(PopupMenuItem::new(format!("React {emoji}")).on_click(move |_, _, cx| {
                    let _ = a.update(cx, |this, cx| this.react(id, emoji, cx));
                }));
            }
            if failed {
                let a = app.clone();
                menu = menu.separator().item(PopupMenuItem::new("Retry sending").on_click(move |_, _, cx| {
                    let _ = a.update(cx, |this, cx| this.resend(id, cx));
                }));
            }
            if deletable {
                let a = app.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("Delete for everyone").icon(Icon::new(Icons::Trash)).on_click(
                        move |_, _, cx| {
                            let _ = a.update(cx, |this, cx| this.delete_for_everyone(id, cx));
                        },
                    ),
                );
            }
            menu
        }
    });

    // hover actions beside the bubble (react / reply)
    let group_name: gpui_kit::SharedString = format!("msg-hover-{id}").into();
    let actions = (!m.deleted).then(|| {
        let a1 = app.clone();
        let a2 = app.clone();
        div()
            .flex()
            .items_center()
            .invisible()
            .group_hover(group_name.clone(), |s| s.visible())
            .child(
                Button::new(ElementId::Name(format!("react-{id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::FaceSlightlySmilingPlus))
                    .tooltip("React ❤️")
                    .on_click(move |_, _, cx| {
                        let _ = a1.update(cx, |this, cx| this.react(id, QUICK_REACTIONS[0], cx));
                    }),
            )
            .child(
                Button::new(ElementId::Name(format!("reply-{id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::Reply))
                    .tooltip("Reply")
                    .on_click(move |_, window, cx| {
                        let _ = a2.update(cx, |this, cx| this.reply_to(id, window, cx));
                    }),
            )
    });

    let reactions = m.reaction_summary();
    let reactions_row = (!reactions.is_empty()).then(|| {
        div()
            .flex()
            .gap_1()
            .mt(px(-6.))
            .when(outgoing, |d| d.justify_end().pr_2())
            .when(!outgoing, |d| d.pl_2())
            .children(reactions.into_iter().map(|(emoji, count)| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(1.))
                    .rounded_full()
                    .border_1()
                    .border_color(p.background)
                    .bg(p.reaction_bg)
                    .text_size(px(12.))
                    .child(emoji)
                    .when(count > 1, |d| d.child(count.to_string()))
            }))
    });

    let failed_row = (m.status == MessageStatus::Failed).then(|| {
        let a = app.clone();
        div()
            .flex()
            .justify_end()
            .items_center()
            .gap_1()
            .text_size(px(12.))
            .text_color(p.danger)
            .child(Icon::new(gpui_kit::component::IconName::CircleAlert).xsmall())
            .child("Not sent.")
            .child(
                div()
                    .id(ElementId::Name(format!("retry-{id}").into()))
                    .underline()
                    .cursor_pointer()
                    .child("Retry")
                    .on_click(move |_, _, cx| {
                        let _ = a.update(cx, |this, cx| this.resend(id, cx));
                    }),
            )
    });

    let column = div()
        .flex()
        .flex_col()
        .when(outgoing, |d| d.items_end())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(outgoing, |d| d.flex_row_reverse())
                .child(bubble)
                .children(actions),
        )
        .children(reactions_row)
        .children(failed_row);

    div()
        .id(ElementId::Name(format!("row-{id}").into()))
        .group(group_name)
        .flex()
        .w_full()
        .gap_2()
        .items_end()
        .when(outgoing, |d| d.justify_end())
        .when(tm.last_in_group, |d| d.pb_2())
        .when(!tm.last_in_group, |d| d.pb(px(2.)))
        .when(show_avatar_column, |d| {
            d.child(if tm.last_in_group {
                Avatar::new(m.author.as_bytes().to_vec(), author_name).size(px(28.)).into_any_element()
            } else {
                div().w(px(28.)).flex_none().into_any_element()
            })
        })
        .child(column)
        .into_any_element()
}

fn status_icon(status: MessageStatus, color: gpui_kit::Hsla) -> AnyElement {
    let icon = match status {
        MessageStatus::Sending => Icon::new(Icons::Clock),
        MessageStatus::Sent => Icon::new(gpui_kit::component::IconName::Check),
        MessageStatus::Delivered | MessageStatus::Read => Icon::new(Icons::CheckCheck),
        MessageStatus::Failed => Icon::new(gpui_kit::component::IconName::CircleAlert),
        MessageStatus::Received => return div().into_any_element(),
    };
    div()
        .flex()
        .when(status == MessageStatus::Read, |d| d.font_weight(FontWeight::BOLD))
        .child(icon.xsmall().text_color(color))
        .into_any_element()
}

fn render_attachment(msg_id: i64, index: usize, a: &Attachment, fg: gpui_kit::Hsla, meta: gpui_kit::Hsla) -> AnyElement {
    if let (true, Some(path)) = (a.is_image(), a.local_path.as_ref()) {
        let (w, h) = match (a.width, a.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => {
                let scale = (MAX_IMAGE / w as f32).min(MAX_IMAGE / h as f32).min(1.);
                (w as f32 * scale, h as f32 * scale)
            }
            _ => (MAX_IMAGE, MAX_IMAGE * 0.75),
        };
        return div()
            .id(ElementId::Name(format!("attachment-{msg_id}-{index}").into()))
            .rounded(px(12.))
            .overflow_hidden()
            .child(img(std::path::PathBuf::from(path)).w(px(w)).h(px(h)).object_fit(ObjectFit::Cover))
            .into_any_element();
    }
    let downloading = a.local_path.is_none();
    let path = a.local_path.clone();
    div()
        .id(ElementId::Name(format!("attachment-{msg_id}-{index}").into()))
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .when(!downloading, |d| d.cursor_pointer())
        .when_some(path, |d, path| {
            d.on_click(move |_, _, cx| {
                cx.open_with_system(std::path::Path::new(&path));
            })
        })
        .child(
            div()
                .size(px(36.))
                .flex_none()
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(meta)
                .child(Icon::new(if downloading { Icons::Download } else { Icons::Paperclip }).small().text_color(fg)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(a.display_name()))
                .child(div().text_size(px(12.)).text_color(meta).child(if downloading {
                    format!("{} · Downloading…", file_size(a.size))
                } else {
                    file_size(a.size)
                })),
        )
        .into_any_element()
}
