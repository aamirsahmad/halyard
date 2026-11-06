//! Conversation- and message-level actions (menus, dialogs, requests).

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{Context, Window};
use signal_core::{BackendCommand, Changes, ConversationId, Millis, Mutation, now_millis};

use crate::app::{SignalApp, TimelineChange};

impl SignalApp {
    /// Applies a local-only mutation (mute, mark unread, delete…) and mirrors
    /// the result into the UI state.
    pub(crate) fn apply_local(&mut self, mutation: Mutation, cx: &mut Context<Self>) -> Changes {
        let changes = match self.db.apply(mutation) {
            Ok(ch) => ch,
            Err(e) => {
                tracing::error!(%e, "local change failed");
                return Changes::default();
            }
        };
        for f in &changes.removed_files {
            self.attachments.remove_stored_file(f);
        }
        let mut timeline = false;
        for event in changes.clone().into_events() {
            let names = self.names.clone();
            let fx = self
                .state
                .apply(event, |u| names.get(u).cloned().unwrap_or_default());
            timeline |= fx.timeline_changed;
        }
        if timeline {
            self.rebuild_timeline(TimelineChange::Update, cx);
        }
        cx.notify();
        changes
    }

    /// Mutes for `duration` (`None` = always), or unmutes with `unmute`.
    pub fn mute_for(
        &mut self,
        id: ConversationId,
        duration: Option<Millis>,
        cx: &mut Context<Self>,
    ) {
        let until = Some(duration.map_or(i64::MAX, |d| now_millis().saturating_add(d)));
        self.apply_local(
            Mutation::SetMuted {
                conversation: id,
                until,
            },
            cx,
        );
    }

    pub fn unmute(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        self.apply_local(
            Mutation::SetMuted {
                conversation: id,
                until: None,
            },
            cx,
        );
    }

    pub fn set_marked_unread(&mut self, id: ConversationId, unread: bool, cx: &mut Context<Self>) {
        if unread {
            if self.state.selected() == Some(&id) {
                self.close_conversation(cx);
            }
            self.apply_local(
                Mutation::SetMarkedUnread {
                    conversation: id,
                    unread: true,
                },
                cx,
            );
        } else {
            // marking read also sends read receipts for real unread messages
            self.commands
                .send(BackendCommand::MarkRead { conversation: id });
            self.apply_local(Mutation::MarkConversationRead { conversation: id }, cx);
        }
    }

    pub fn confirm_delete_conversation(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let entity = entity.clone();
            dialog
                .title("Delete chat?")
                .description("This chat will be deleted from this device. Your other devices are not affected.")
                .ok_text("Delete")
                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| this.delete_conversation(id, cx));
                    true
                })
        });
    }

    pub fn delete_conversation(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        self.apply_local(Mutation::DeleteConversation { conversation: id }, cx);
    }

    pub fn set_blocked(&mut self, id: ConversationId, blocked: bool, cx: &mut Context<Self>) {
        self.apply_local(
            Mutation::SetBlocked {
                conversation: id,
                blocked,
            },
            cx,
        );
    }

    pub fn accept_request(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        self.apply_local(Mutation::Accept { conversation: id }, cx);
        self.commands
            .send(BackendCommand::MarkRead { conversation: id });
    }

    /// Blocks and deletes a message request.
    pub fn block_and_delete(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        self.set_blocked(id, true, cx);
        self.delete_conversation(id, cx);
    }

    pub fn set_expire_timer(
        &mut self,
        id: ConversationId,
        secs: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        self.commands.send(BackendCommand::SetExpireTimer {
            conversation: id,
            secs,
        });
        cx.notify();
    }

    pub fn request_safety_number(&mut self, contact: uuid::Uuid, cx: &mut Context<Self>) {
        self.commands
            .send(BackendCommand::RequestSafetyNumber { contact });
        cx.notify();
    }

    /// Shows a safety number answered by the backend.
    pub fn show_safety_number(
        &mut self,
        contact: uuid::Uuid,
        number: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.display_name(&contact);
        let p = crate::theme::Palette::current(cx);
        window.open_dialog(cx, move |dialog, _, _| {
            use gpui_kit::{FontWeight, InteractiveElement as _, ParentElement as _, Styled as _, TestSupportExt as _, div, px};
            let body = match &number {
                Some(n) => {
                    let groups = safety_number_groups(n);
                    div()
                        .id("safety-number")
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div().text_size(px(13.)).text_color(p.text_secondary).child(format!(
                                "To verify end-to-end encryption with {name}, compare these numbers with their device. \
                                 You can also scan the code on their phone."
                            )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_center()
                                .gap_x_4()
                                .gap_y_2()
                                .p_4()
                                .rounded(px(12.))
                                .bg(p.input_bg)
                                .font_family("monospace")
                                .text_size(px(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .children(groups.into_iter().map(|g| div().w(px(72.)).child(g))),
                        )
                        .test_support()
                }
                None => div()
                    .id("safety-number")
                    .text_color(p.text_secondary)
                    .child(format!("You haven't exchanged messages with {name} yet, so there's no safety number."))
                    .test_support(),
            };
            dialog.title("Safety number").w(px(460.)).child(body)
        });
    }

    pub fn delete_for_me(&mut self, message_id: i64, cx: &mut Context<Self>) {
        self.apply_local(Mutation::DeleteForMe { message_id }, cx);
    }

    /// Forwards a message's text and attachments to `targets`.
    pub fn forward(
        &mut self,
        message_id: i64,
        targets: Vec<ConversationId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(msg) = self.state.message(message_id).cloned() else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        let attachments = msg
            .attachments
            .iter()
            .filter_map(|a| {
                let path = a.local_path.as_ref()?;
                // encrypted store files are decrypted to a private copy for re-upload
                self.attachments.path_for_open(path, &a.display_name()).ok()
            })
            .collect();
        let n = targets.len();
        self.commands.send(BackendCommand::Forward {
            targets,
            body: msg.body.clone(),
            attachments,
        });
        window.push_notification(
            Notification::success(if n == 1 {
                "Message forwarded".to_string()
            } else {
                format!("Message forwarded to {n} chats")
            }),
            cx,
        );
        cx.notify();
    }
}

/// Splits a 60-digit safety number into Signal's 12 groups of 5.
pub fn safety_number_groups(number: &str) -> Vec<String> {
    let digits: Vec<char> = number.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.chunks(5).map(|c| c.iter().collect()).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn safety_number_grouping() {
        let n: String = (0..60).map(|i| char::from(b'0' + (i % 10) as u8)).collect();
        let g = super::safety_number_groups(&n);
        assert_eq!(g.len(), 12);
        assert!(g.iter().all(|x| x.len() == 5));
        assert_eq!(g[0], "01234");
    }
}
