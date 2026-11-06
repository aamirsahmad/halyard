//! Message actions menu (right-click on a bubble, or the ⋯ hover button) and
//! the dialogs it opens: forward, message info and the reaction picker.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    WeakEntity, Window, div,
};
use signal_core::{ConversationId, Message, MessageStatus, now_millis};

use crate::app::SignalApp;
use crate::assets::Icons;
use crate::theme::Palette;
use crate::views::avatar::Avatar;
use crate::views::rem_px;

fn item(
    app: &WeakEntity<SignalApp>,
    label: &'static str,
    f: impl Fn(&mut SignalApp, &mut Window, &mut Context<SignalApp>) + 'static,
) -> PopupMenuItem {
    let app = app.clone();
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        let _ = app.update(cx, |this, cx| f(this, window, cx));
    })
}

pub fn message_menu(mut menu: PopupMenu, app: WeakEntity<SignalApp>, m: &Message) -> PopupMenu {
    let id = m.id;
    if m.deleted || m.is_notice() {
        return menu.item(item(&app, "Delete for me", move |this, _, cx| {
            this.delete_for_me(id, cx)
        }));
    }
    menu = menu.item(
        item(&app, "Reply", move |this, window, cx| {
            this.reply_to(id, window, cx)
        })
        .icon(Icon::new(Icons::Reply)),
    );
    menu = menu.item(
        item(&app, "React…", move |this, window, cx| {
            this.open_reaction_picker(id, window, cx)
        })
        .icon(Icon::new(Icons::FaceSlightlySmilingPlus)),
    );
    if m.editable(now_millis()) {
        menu = menu.item(
            item(&app, "Edit", move |this, window, cx| {
                this.start_edit(id, window, cx)
            })
            .icon(Icon::new(Icons::Pencil)),
        );
    }
    if !m.body.is_empty() {
        menu = menu.item(
            item(&app, "Copy text", move |this, _, cx| this.copy_text(id, cx))
                .icon(Icon::new(Icons::Copy)),
        );
    }
    menu = menu
        .item(
            item(&app, "Forward…", move |this, window, cx| {
                this.open_forward(id, window, cx)
            })
            .icon(Icon::new(Icons::Forward)),
        )
        .item(
            item(&app, "Info", move |this, window, cx| {
                this.show_message_info(id, window, cx)
            })
            .icon(Icon::new(Icons::Info)),
        );
    if m.status == MessageStatus::Failed {
        menu = menu
            .separator()
            .item(item(&app, "Retry sending", move |this, _, cx| {
                this.resend(id, cx)
            }));
    }
    menu = menu.separator().item(
        item(&app, "Delete for me", move |this, _, cx| {
            this.delete_for_me(id, cx)
        })
        .icon(Icon::new(Icons::Trash)),
    );
    if m.outgoing {
        menu = menu.item(item(&app, "Delete for everyone", move |this, _, cx| {
            this.delete_for_everyone(id, cx)
        }));
    }
    menu
}

fn full_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%a, %b %-d, %Y at %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

impl SignalApp {
    /// Reaction picker with every emoji for one message.
    pub fn open_reaction_picker(
        &mut self,
        message_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reaction_target.set(Some(message_id));
        self.reaction_picker.update(cx, |p, cx| p.reset(window, cx));
        let picker = self.reaction_picker.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("React")
                .w(gpui_kit::px(380.))
                .child(picker.clone())
        });
    }

    pub fn open_forward(&mut self, message_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.state.message(message_id).cloned() else {
            return;
        };
        let me = self.me();
        // recent chats first, then other contacts
        let mut targets: Vec<(ConversationId, String)> = self
            .state
            .conversations()
            .iter()
            .filter(|c| c.accepted && !c.blocked)
            .map(|c| (c.id, c.title.clone()))
            .collect();
        for c in self.db.all_conversations().unwrap_or_default() {
            if !targets.iter().any(|(id, _)| *id == c.id) && c.id.direct_uuid() != me {
                targets.push((c.id, c.title));
            }
        }
        let selected: Rc<RefCell<BTreeSet<ConversationId>>> = Rc::default();
        let app = cx.entity().downgrade();
        let palette = Palette::current(cx);
        let preview = msg.preview();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows: Vec<AnyElement> = targets
                .iter()
                .map(|(id, title)| {
                    let id = *id;
                    let sel = selected.clone();
                    let checked = selected.borrow().contains(&id);
                    div()
                        .id(ElementId::Name(format!("forward-{id}").into()))
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_1()
                        .child(
                            Checkbox::new(ElementId::Name(format!("forward-check-{id}").into()))
                                .checked(checked)
                                .on_click(move |checked, window, _| {
                                    if *checked {
                                        sel.borrow_mut().insert(id);
                                    } else {
                                        sel.borrow_mut().remove(&id);
                                    }
                                    window.refresh();
                                }),
                        )
                        .child(
                            Avatar::new(id.color_seed(), title.clone())
                                .group(id.is_group())
                                .size(rem_px(28.)),
                        )
                        .child(div().truncate().child(title.clone()))
                        .test_support()
                        .into_any_element()
                })
                .collect();
            let n = selected.borrow().len();
            let app = app.clone();
            let sel = selected.clone();
            dialog
                .title("Forward message")
                .w(gpui_kit::px(420.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .p_2()
                                .rounded(rem_px(8.))
                                .bg(palette.input_bg)
                                .text_size(rem_px(13.))
                                .line_clamp(2)
                                .child(preview.clone()),
                        )
                        .child(
                            div()
                                .id("forward-list")
                                .max_h(rem_px(320.))
                                .overflow_y_scroll()
                                .children(rows),
                        ),
                )
                .footer(
                    div().flex().justify_end().child(
                        Button::new("forward-send")
                            .primary()
                            .label(if n > 1 {
                                format!("Send to {n} chats")
                            } else {
                                "Send".to_string()
                            })
                            .on_click(move |_, window, cx| {
                                let targets: Vec<ConversationId> =
                                    sel.borrow().iter().copied().collect();
                                if targets.is_empty() {
                                    return;
                                }
                                let _ = app.update(cx, |this, cx| {
                                    this.forward(message_id, targets, window, cx)
                                });
                                window.close_dialog(cx);
                            }),
                    ),
                )
        });
    }

    pub fn show_message_info(
        &mut self,
        message_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(m) = self.state.message(message_id).cloned() else {
            return;
        };
        let palette = Palette::current(cx);
        let names = self.names.clone();
        let me = self.me();
        let name = move |u: &uuid::Uuid| {
            if Some(*u) == me {
                "You".to_string()
            } else {
                names
                    .get(u)
                    .cloned()
                    .unwrap_or_else(|| "Unknown contact".into())
            }
        };
        let mut rows: Vec<(&'static str, String)> = vec![(
            if m.outgoing { "Sent" } else { "Sent by sender" },
            full_time(m.sent_at),
        )];
        if !m.outgoing {
            rows.push(("From", name(&m.author)));
            rows.push(("Received", full_time(m.received_at)));
        } else {
            let status = match m.status {
                MessageStatus::Sending => "Sending",
                MessageStatus::Sent => "Sent",
                MessageStatus::Delivered => "Delivered",
                MessageStatus::Read => "Read",
                MessageStatus::Failed => "Failed to send",
                MessageStatus::Received => "Received",
            };
            rows.push(("Status", status.into()));
        }
        if let Some(secs) = m.expires_in_secs {
            rows.push(("Disappears", signal_core::timer_label(secs)));
        }
        for a in &m.attachments {
            rows.push((
                "Attachment",
                format!(
                    "{} · {}",
                    a.display_name(),
                    signal_core::format::file_size(a.size)
                ),
            ));
        }
        let reactions: Vec<(String, String)> = m
            .reactions
            .iter()
            .map(|r| (r.emoji.clone(), name(&r.reactor)))
            .collect();
        let mut history: Vec<(String, String)> = Vec::new();
        if !m.edit_history.is_empty() {
            history.push(("Current".into(), m.body.clone()));
            history.extend(
                m.edit_history
                    .iter()
                    .rev()
                    .map(|r| (full_time(r.at), r.body.clone())),
            );
        }
        window.open_dialog(cx, move |dialog, _, _| {
            let row = |label: String, value: String| {
                div()
                    .flex()
                    .gap_3()
                    .py_1()
                    .child(
                        div()
                            .w(rem_px(120.))
                            .flex_none()
                            .text_color(palette.text_secondary)
                            .child(label),
                    )
                    .child(div().flex_1().child(value))
            };
            dialog.title("Message info").w(gpui_kit::px(480.)).child(
                div()
                    .id("message-info")
                    .flex()
                    .flex_col()
                    .gap_4()
                    .text_size(rem_px(13.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(rows.iter().map(|(l, v)| row(l.to_string(), v.clone()))),
                    )
                    .when(!reactions.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Reactions"))
                                .children(reactions.iter().map(|(e, who)| {
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(div().text_size(rem_px(18.)).child(e.clone()))
                                        .child(who.clone())
                                })),
                        )
                    })
                    .when(!history.is_empty(), |d| {
                        d.child(
                            div()
                                .id("edit-history")
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Edit history"),
                                )
                                .children(
                                    history
                                        .iter()
                                        .map(|(when, body)| row(when.clone(), body.clone())),
                                )
                                .test_support(),
                        )
                    })
                    .test_support(),
            )
        });
    }
}
