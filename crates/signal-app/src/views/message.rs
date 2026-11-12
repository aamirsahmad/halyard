//! Timeline rows: date separators and Signal-style message bubbles.

use gpui_kit::Anchor;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _};
use gpui_kit::component::popover::Popover;

use crate::views::message_menu::message_menu;
use crate::views::rem_px;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    TestSupportExt as _, WeakEntity, Window, div, img,
};
use signal_core::format::{file_size, message_time};
use signal_core::timeline::{TimelineItem, TimelineMessage};
use signal_core::{
    Attachment, Message, MessageKind, MessageStatus, avatar, member_notice, now_millis,
    timer_notice, timer_short,
};

use crate::app::{QUICK_REACTIONS, SignalApp, TimelineData};
use crate::assets::Icons;
use crate::audio::format_clock;
use crate::theme::{Palette, hex};
use crate::views::avatar::Avatar;

const BUBBLE_RADIUS: f32 = 18.;
const GROUPED_RADIUS: f32 = 4.;
/// Signal Desktop's bubble width rule (stylesheets/_modules.scss and
/// `getWidthBreakpoint`): wide timelines (> 606px) allow half the window,
/// medium ones (> 514px) 370px, narrow ones 306px (less if it doesn't fit).
/// `window_width` is in rem×16 units, like all our sizes.
pub fn max_bubble_width(window_width: f32) -> f32 {
    use crate::views::sidebar::{
        COMPACT_BREAKPOINT, COMPACT_SIDEBAR_WIDTH, NAV_TABS_WIDTH, SIDEBAR_WIDTH,
    };
    let sidebar = if window_width < COMPACT_BREAKPOINT {
        COMPACT_SIDEBAR_WIDTH
    } else {
        NAV_TABS_WIDTH + SIDEBAR_WIDTH
    };
    let timeline = window_width - sidebar - 32.; // row padding
    if timeline > 606. {
        window_width * 0.5
    } else if timeline > 514. {
        370.
    } else {
        306f32.min(timeline - 16. - 22.).max(120.)
    }
}
const MAX_IMAGE: f32 = 300.;

pub fn message_element_id(m: &Message) -> ElementId {
    ElementId::Name(format!("message-{}", m.id).into())
}

pub fn render_timeline_item(
    data: &TimelineData,
    index: usize,
    p: &Palette,
    app: WeakEntity<SignalApp>,
    window: &mut Window,
    _: &mut App,
) -> AnyElement {
    let width = f32::from(window.viewport_size().width) / f32::from(window.rem_size()) * 16.;
    let max_bubble = max_bubble_width(width);
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
                    .text_size(rem_px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_secondary)
                    .child(label.clone()),
            )
            .into_any_element(),
        Some(TimelineItem::UnreadMarker(n)) => div()
            .id("unread-marker")
            .flex()
            .items_center()
            .gap_3()
            .py_3()
            .child(div().flex_1().h(rem_px(1.)).bg(p.border))
            .child(
                div()
                    .text_size(rem_px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_secondary)
                    .child(if *n == 1 {
                        "1 unread message".to_string()
                    } else {
                        format!("{n} unread messages")
                    }),
            )
            .child(div().flex_1().h(rem_px(1.)).bg(p.border))
            .test_support()
            .into_any_element(),
        Some(TimelineItem::Message(tm)) => match data.messages.get(tm.index) {
            Some(m) => render_message(data, tm, m, p, app, max_bubble),
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
    max_bubble: f32,
) -> AnyElement {
    if let MessageKind::GroupMember {
        change,
        member,
        member_is_me,
    } = &m.kind
    {
        let name = |u: &uuid::Uuid| data.names.get(u).cloned().unwrap_or("Someone".into());
        let actor = (!m.outgoing).then(|| name(&m.author));
        let member = (!member_is_me).then(|| name(member));
        return div()
            .id(ElementId::Name(format!("row-{}", m.id).into()))
            .flex()
            .justify_center()
            .items_center()
            .gap_1()
            .py_2()
            .text_size(rem_px(12.))
            .text_color(p.text_secondary)
            .child(Icon::new(Icons::Users).xsmall())
            .child(member_notice(actor.as_deref(), member.as_deref(), *change))
            .test_support()
            .into_any_element();
    }
    if let MessageKind::TimerChanged { secs } = &m.kind {
        let who = if m.outgoing {
            None
        } else {
            data.names
                .get(&m.author)
                .cloned()
                .or(Some("Someone".into()))
        };
        return div()
            .id(ElementId::Name(format!("row-{}", m.id).into()))
            .flex()
            .justify_center()
            .items_center()
            .gap_1()
            .py_2()
            .text_size(rem_px(12.))
            .text_color(p.text_secondary)
            .child(Icon::new(Icons::Clock).xsmall())
            .child(timer_notice(who.as_deref(), *secs))
            .test_support()
            .into_any_element();
    }
    let outgoing = m.outgoing;
    let show_avatar_column = data.is_group && !outgoing;
    let author_name = data
        .names
        .get(&m.author)
        .cloned()
        .unwrap_or_else(|| "Unknown contact".into());
    let author_color = avatar::color_for(m.author.as_bytes());
    let (bg, fg, meta) = if outgoing {
        (
            p.bubble_outgoing,
            p.bubble_outgoing_text,
            p.bubble_outgoing_meta,
        )
    } else {
        (p.bubble_incoming, p.bubble_incoming_text, p.text_secondary)
    };
    let big = rem_px(BUBBLE_RADIUS);
    let small = rem_px(GROUPED_RADIUS);
    // grouped bubbles get tight corners on the sender's side, like Signal
    let (tl, tr, bl, br) = if outgoing {
        (
            big,
            if tm.first_in_group { big } else { small },
            big,
            if tm.last_in_group { big } else { small },
        )
    } else {
        (
            if tm.first_in_group { big } else { small },
            big,
            if tm.last_in_group { big } else { small },
            big,
        )
    };
    let id = m.id;

    let mut bubble = div()
        .id(message_element_id(m))
        .flex()
        .flex_col()
        .max_w(rem_px(max_bubble))
        .min_w(rem_px(60.))
        .flex_shrink_1()
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
                .text_size(rem_px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hex(if p.dark {
                    author_color.bg
                } else {
                    author_color.fg
                }))
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
                data.names
                    .get(&q.author)
                    .cloned()
                    .unwrap_or_else(|| "Unknown contact".into())
            };
            let (q_author, q_sent) = (q.author, q.sent_at);
            let a = app.clone();
            bubble = bubble.child(
                div()
                    .id(ElementId::Name(format!("quote-{id}").into()))
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        let _ = a.update(cx, |this, cx| {
                            this.scroll_to_message(q_author, q_sent, window, cx)
                        });
                    })
                    .flex()
                    .flex_col()
                    .p_2()
                    .rounded(rem_px(8.))
                    .border_l_4()
                    .border_color(if outgoing {
                        p.bubble_outgoing_text
                    } else {
                        p.accent
                    })
                    .bg(if outgoing {
                        p.quote_outgoing
                    } else {
                        p.quote_incoming
                    })
                    .child(
                        div()
                            .text_size(rem_px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(quoted_author),
                    )
                    .child(
                        div()
                            .text_size(rem_px(13.))
                            .line_clamp(3)
                            .child(q.text.clone()),
                    ),
            );
        }
        for (i, a) in m.attachments.iter().enumerate() {
            bubble = bubble.child(render_attachment(data, &app, id, i, a, fg, meta));
        }
        if let Some(lp) = &m.link_preview {
            let url = lp.url.clone();
            bubble = bubble.child(
                div()
                    .id(ElementId::Name(format!("link-preview-{id}").into()))
                    .flex()
                    .flex_col()
                    .gap(rem_px(2.))
                    .p_2()
                    .rounded(rem_px(8.))
                    .bg(if outgoing {
                        p.quote_outgoing
                    } else {
                        p.quote_incoming
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .when_some(lp.title.clone(), |d, t| {
                        d.child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(rem_px(13.))
                                .line_clamp(2)
                                .child(t),
                        )
                    })
                    .when_some(lp.description.clone(), |d, t| {
                        d.child(div().text_size(rem_px(12.)).line_clamp(2).child(t))
                    })
                    .child(
                        div()
                            .text_size(rem_px(11.))
                            .text_color(meta)
                            .child(lp.domain()),
                    )
                    .test_support(),
            );
        }
        if !m.body.is_empty() {
            bubble = bubble.child(crate::views::rich_text::render_body(
                m.id,
                &m.body,
                &m.body_ranges,
                &data.names,
                data.me,
                fg,
                outgoing,
            ));
        }
    }

    // footer: edited • time • status
    bubble = bubble.child(
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap_1()
            .text_size(rem_px(11.))
            .text_color(meta)
            .when(m.edited_at.is_some() && !m.deleted, |d| d.child("Edited"))
            .when_some(m.expires_in_secs, |d, _| {
                d.child(Icon::new(Icons::Clock).xsmall())
                    .when_some(m.expires_at, |d, at| {
                        d.child(timer_short(at - now_millis()))
                    })
            })
            .child({
                let full = chrono::DateTime::from_timestamp_millis(m.sent_at)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%A, %B %-d, %Y %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                div()
                    .id(ElementId::Name(format!("time-{id}").into()))
                    .child(message_time(m.sent_at, now_millis()))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx)
                    })
            })
            .when(outgoing, |d| d.child(status_icon(m.status, meta))),
    );

    let bubble = bubble.context_menu({
        let app = app.clone();
        let m = m.clone();
        move |menu, _, _| message_menu(menu, app.clone(), &m)
    });

    // hover actions beside the bubble: react, reply, more — like Signal
    let group_name: gpui_kit::SharedString = format!("msg-hover-{id}").into();
    let actions = (!m.deleted).then(|| {
        let a_reply = app.clone();
        let a_react = app.clone();
        let a_more = app.clone();
        let menu_msg = m.clone();
        let palette = *p;
        div()
            .flex()
            .items_center()
            .invisible()
            .group_hover(group_name.clone(), |s| s.visible())
            .child(
                Popover::new(ElementId::Name(format!("react-popover-{id}").into()))
                    .anchor(if outgoing {
                        Anchor::BottomRight
                    } else {
                        Anchor::BottomLeft
                    })
                    .trigger(
                        Button::new(ElementId::Name(format!("react-{id}").into()))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Icons::FaceSlightlySmilingPlus))
                            .tooltip("React"),
                    )
                    .content(move |_, _, cx| {
                        let popover = cx.entity();
                        let app = a_react.clone();
                        div()
                            .id(ElementId::Name(format!("reaction-bar-{id}").into()))
                            .flex()
                            .items_center()
                            .gap_1()
                            .children(QUICK_REACTIONS.iter().map(|emoji| {
                                let app = app.clone();
                                let popover = popover.clone();
                                div()
                                    .id(ElementId::Name(format!("quick-react-{id}-{emoji}").into()))
                                    .size(rem_px(36.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .text_size(rem_px(22.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.hover))
                                    .child(*emoji)
                                    .on_click(move |_, window, cx| {
                                        let _ =
                                            app.update(cx, |this, cx| this.react(id, emoji, cx));
                                        popover.update(cx, |p, cx| p.dismiss(window, cx));
                                    })
                                    .test_support()
                            }))
                            .child({
                                let app = app.clone();
                                let popover = popover.clone();
                                Button::new(ElementId::Name(format!("more-reactions-{id}").into()))
                                    .ghost()
                                    .small()
                                    .icon(gpui_kit::component::IconName::Plus)
                                    .tooltip("More reactions")
                                    .on_click(move |_, window, cx| {
                                        popover.update(cx, |p, cx| p.dismiss(window, cx));
                                        let _ = app.update(cx, |this, cx| {
                                            this.open_reaction_picker(id, window, cx)
                                        });
                                    })
                            })
                            .test_support()
                    }),
            )
            .child(
                Button::new(ElementId::Name(format!("reply-{id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::Reply))
                    .tooltip("Reply")
                    .on_click(move |_, window, cx| {
                        let _ = a_reply.update(cx, |this, cx| this.reply_to(id, window, cx));
                    }),
            )
            .child(
                Button::new(ElementId::Name(format!("more-{id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::component::IconName::Ellipsis)
                    .tooltip("More actions")
                    .dropdown_menu(move |menu, _, _| message_menu(menu, a_more.clone(), &menu_msg)),
            )
    });

    let reactions = m.reaction_summary();
    let reactions_row = (!reactions.is_empty()).then(|| {
        div()
            .flex()
            .gap_1()
            .mt(rem_px(-6.))
            .when(outgoing, |d| d.justify_end().pr_2())
            .when(!outgoing, |d| d.pl_2())
            .children(reactions.into_iter().map(|(emoji, count)| {
                let app = app.clone();
                div()
                    .id(ElementId::Name(format!("reaction-{id}-{emoji}").into()))
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| this.show_message_info(id, window, cx));
                    })
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(rem_px(1.))
                    .rounded_full()
                    .border_1()
                    .border_color(p.background)
                    .bg(p.reaction_bg)
                    .text_size(rem_px(12.))
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
            .text_size(rem_px(12.))
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

    // min_w_0 all the way down so a long bubble shrinks to the pane width
    // instead of overflowing it when the window is narrow
    let column = div()
        .flex()
        .flex_col()
        .flex_shrink_1()
        .min_w_0()
        .max_w_full()
        .when(outgoing, |d| d.items_end())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .max_w_full()
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
        .when(!tm.last_in_group, |d| d.pb(rem_px(2.)))
        .when(show_avatar_column, |d| {
            d.child(if tm.last_in_group {
                Avatar::new(m.author.as_bytes().to_vec(), author_name)
                    .photo(
                        data.photos
                            .get(&m.author)
                            .and_then(|p| data.attachments.image(p, "")),
                    )
                    .size(rem_px(28.))
                    .into_any_element()
            } else {
                div().w(rem_px(28.)).flex_none().into_any_element()
            })
        })
        .child(column)
        .test_support()
        .into_any_element()
}

/// Signal Desktop's message status icons (images/icons/v3/message_status):
/// a dotted ring while sending, one ticked circle when sent, two when
/// delivered, two filled circles when read. Drawn at their native 12px height.
fn status_icon(status: MessageStatus, color: gpui_kit::Hsla) -> AnyElement {
    use crate::assets::signal_icons;
    let (path, width) = match status {
        MessageStatus::Sending => (signal_icons::SENDING, 12.),
        MessageStatus::Sent => (signal_icons::SENT, 12.),
        MessageStatus::Delivered => (signal_icons::DELIVERED, 18.),
        MessageStatus::Read => (signal_icons::READ, 18.),
        MessageStatus::Failed => {
            return Icon::new(gpui_kit::component::IconName::CircleAlert)
                .xsmall()
                .text_color(color)
                .into_any_element();
        }
        MessageStatus::Received => return div().into_any_element(),
    };
    div()
        .id(ElementId::Name(format!("status-{status:?}").into()))
        .flex()
        .flex_none()
        .child(
            gpui_kit::svg()
                .path(path)
                .w(rem_px(width))
                .h(rem_px(12.))
                .text_color(color),
        )
        .test_support()
        .into_any_element()
}

fn render_attachment(
    data: &TimelineData,
    app: &WeakEntity<SignalApp>,
    msg_id: i64,
    index: usize,
    a: &Attachment,
    fg: gpui_kit::Hsla,
    meta: gpui_kit::Hsla,
) -> AnyElement {
    if a.view_once {
        let openable = !a.viewed && a.local_path.is_some();
        let label = if a.viewed {
            "Viewed"
        } else if a.local_path.is_none() {
            "View-once media · downloading…"
        } else if a.is_image() {
            "View photo"
        } else {
            "View video"
        };
        let app = app.clone();
        return div()
            .id(ElementId::Name(
                format!("view-once-{msg_id}-{index}").into(),
            ))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_full()
            .border_1()
            .border_color(meta)
            .text_color(if openable { fg } else { meta })
            .when(openable, |d| {
                d.cursor_pointer().on_click(move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.open_view_once(msg_id, index, window, cx)
                    });
                })
            })
            .child(Icon::new(Icons::Clock).xsmall())
            .child(label)
            .test_support()
            .into_any_element();
    }
    if let (true, Some(path)) = (a.is_image(), a.local_path.as_ref()) {
        let (w, h) = match (a.width, a.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => {
                let scale = (MAX_IMAGE / w as f32).min(MAX_IMAGE / h as f32).min(1.);
                (w as f32 * scale, h as f32 * scale)
            }
            _ => (MAX_IMAGE, MAX_IMAGE * 0.75),
        };
        return div()
            .id(ElementId::Name(
                format!("attachment-{msg_id}-{index}").into(),
            ))
            .rounded(rem_px(12.))
            .overflow_hidden()
            .cursor_pointer()
            .on_click({
                let app = app.clone();
                let (path, ct, name) = (path.clone(), a.content_type.clone(), a.display_name());
                move |_, window, cx| {
                    let (path, ct, name) = (path.clone(), ct.clone(), name.clone());
                    let _ = app.update(cx, |this, cx| {
                        this.open_lightbox(path, ct, name, window, cx)
                    });
                }
            })
            .child(
                match data.attachments.image(path, &a.content_type) {
                    Some(image) => img(image),
                    None => img(std::path::PathBuf::from(path)),
                }
                .w(rem_px(w))
                .h(rem_px(h))
                .object_fit(ObjectFit::Cover),
            )
            .test_support()
            .into_any_element();
    }
    if let (true, Some(path)) = (a.content_type.starts_with("audio/"), a.local_path.clone()) {
        let status = data.audio.status();
        let current = status.current == Some((msg_id, index));
        let playing = current && !status.paused;
        let fraction = match (current, status.duration) {
            (true, Some(d)) if !d.is_zero() => {
                (status.position.as_secs_f32() / d.as_secs_f32()).clamp(0., 1.)
            }
            _ => 0.,
        };
        let clock = if current {
            match status.duration {
                Some(d) => format!("{} / {}", format_clock(status.position), format_clock(d)),
                None => format_clock(status.position),
            }
        } else {
            file_size(a.size)
        };
        let app = app.clone();
        return div()
            .id(ElementId::Name(
                format!("attachment-{msg_id}-{index}").into(),
            ))
            .flex()
            .items_center()
            .gap_2()
            .min_w(rem_px(220.))
            .py_1()
            .child(
                Button::new(ElementId::Name(
                    format!("audio-play-{msg_id}-{index}").into(),
                ))
                .small()
                .rounded_full()
                .icon(if playing {
                    gpui_kit::component::IconName::Pause
                } else {
                    gpui_kit::component::IconName::Play
                })
                .tooltip(if playing { "Pause" } else { "Play" })
                .on_click(move |_, _, cx| {
                    let path = path.clone();
                    let _ = app.update(cx, |this, cx| this.toggle_audio(msg_id, index, path, cx));
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .h(rem_px(4.))
                            .w_full()
                            .rounded_full()
                            .bg(meta.opacity(0.35))
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(fg)
                                    .w(gpui_kit::relative(fraction)),
                            ),
                    )
                    .child(div().text_size(rem_px(11.)).text_color(meta).child(clock)),
            )
            .test_support()
            .into_any_element();
    }
    let downloading = a.local_path.is_none();
    let path = a.local_path.clone();
    let access = data.attachments.clone();
    let file_name = a.display_name();
    div()
        .id(ElementId::Name(
            format!("attachment-{msg_id}-{index}").into(),
        ))
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .when(!downloading, |d| d.cursor_pointer())
        .when_some(path, |d, path| {
            d.on_click(
                move |_, _, cx| match access.path_for_open(&path, &file_name) {
                    Ok(p) => cx.open_with_system(&p),
                    Err(e) => tracing::error!(%e, "failed to open attachment"),
                },
            )
        })
        .child(
            div()
                .size(rem_px(36.))
                .flex_none()
                .rounded(rem_px(8.))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(meta)
                .child(
                    Icon::new(if downloading {
                        Icons::Download
                    } else {
                        Icons::Paperclip
                    })
                    .small()
                    .text_color(fg),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(a.display_name()),
                )
                .child(
                    div()
                        .text_size(rem_px(12.))
                        .text_color(meta)
                        .child(if downloading {
                            format!("{} · Downloading…", file_size(a.size))
                        } else {
                            file_size(a.size)
                        }),
                ),
        )
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod width_tests {
    use super::max_bubble_width;

    #[test]
    fn follows_signal_desktop_breakpoints() {
        // 1120 wide: rail + list take 400, timeline 688 > 606 → half the window
        assert_eq!(max_bubble_width(1120.), 560.);
        // 1000: timeline 568 → medium
        assert_eq!(max_bubble_width(1000.), 370.);
        // 840: timeline 408 → narrow
        assert_eq!(max_bubble_width(840.), 306.);
        // 640 (compact list): timeline 524 → medium
        assert_eq!(max_bubble_width(640.), 370.);
        // 520: timeline 404 → narrow
        assert_eq!(max_bubble_width(520.), 306.);
        // tiny: shrinks to fit, with a floor
        assert_eq!(max_bubble_width(300.), 146.);
    }
}
