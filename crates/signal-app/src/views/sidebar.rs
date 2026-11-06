//! Left pane: header, search, connection banner and the conversation list.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, px,
};
use signal_core::format::conversation_list_time;
use signal_core::{ConnectionState, Conversation, ConversationId, Message, now_millis};

use crate::app::{ListMode, SignalApp};
use crate::assets::Icons;
use crate::theme::{Appearance, Palette};
use crate::views::avatar::Avatar;

pub const SIDEBAR_WIDTH: f32 = 320.;

/// Stable element id of a conversation row (also used by UI tests).
pub fn conversation_dom_id(id: &ConversationId) -> String {
    format!("conversation-{id}")
}

pub fn conversation_element_id(id: &ConversationId) -> ElementId {
    ElementId::Name(conversation_dom_id(id).into())
}

impl SignalApp {
    pub(super) fn render_sidebar(
        &mut self,
        p: &Palette,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = match self.list_mode {
            ListMode::Chats => "Chats",
            ListMode::Archived => "Archived chats",
            ListMode::NewChat => "New chat",
        };
        let me = self.me();
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(56.))
            .px_4()
            .flex_none()
            .when(self.list_mode != ListMode::Chats, |d| {
                d.child(
                    Button::new("back")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowLeft)
                        .tooltip("Back")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_list_mode(ListMode::Chats, window, cx)
                        })),
                )
            })
            .when(self.list_mode == ListMode::Chats, |d| {
                d.when_some(me, |d, me| {
                    d.child(
                        div()
                            .id("my-avatar")
                            .cursor_pointer()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_profile(window, cx)),
                            )
                            .child(Avatar::new(me.as_bytes().to_vec(), "You").size(px(28.)))
                            .test_support(),
                    )
                })
            })
            .child(
                div()
                    .flex_1()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .when(self.list_mode == ListMode::Chats, |d| {
                d.child(
                    Button::new("new-chat")
                        .ghost()
                        .small()
                        .icon(Icon::new(Icons::SquarePen))
                        .tooltip("New chat")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_list_mode(ListMode::NewChat, window, cx)
                        })),
                )
                .child(self.render_main_menu(cx))
            });

        let search = div().px_4().pb_2().flex_none().child(
            Input::new(&self.search)
                .id("search")
                .small()
                .cleanable(true)
                .prefix(
                    Icon::new(IconName::Search)
                        .small()
                        .text_color(p.text_secondary),
                ),
        );

        let list: AnyElement = match self.list_mode {
            ListMode::NewChat => self.render_contact_picker(p, cx),
            _ => self.render_conversation_list(p, cx),
        };

        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .bg(p.pane)
            .border_r_1()
            .border_color(p.border)
            .child(header)
            .child(search)
            .children(self.render_connection_banner(p))
            .child(list)
            .test_support()
            .into_any_element()
    }

    fn render_main_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let appearance = self.prefs.appearance;
        Button::new("main-menu").ghost().small().icon(IconName::EllipsisVertical).tooltip("More").dropdown_menu(
            move |menu, _, _| {
                let e = entity.clone();
                let item = |label: &'static str, f: fn(&mut SignalApp, &mut Window, &mut Context<SignalApp>)| {
                    let e = e.clone();
                    PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ = e.update(cx, |this, cx| f(this, window, cx));
                    })
                };
                let theme_item = |label: &'static str, a: Appearance| {
                    let e = e.clone();
                    PopupMenuItem::new(label).checked(appearance == a).on_click(move |_, window, cx| {
                        let _ = e.update(cx, |this, cx| this.set_appearance(a, window, cx));
                    })
                };
                menu.item(item("Profile", |this, window, cx| this.open_profile(window, cx)))
                    .item(item("Preferences…", |this, window, cx| this.open_preferences(window, cx)))
                    .item(item("Keyboard shortcuts", |this, window, cx| this.open_shortcuts(window, cx)))
                    .separator()
                    .item(item("Archived chats", |this, window, cx| this.set_list_mode(ListMode::Archived, window, cx)))
                    .item(item("Sync contacts", |this, _, cx| {
                        this.commands.send(signal_core::BackendCommand::SyncContacts);
                        cx.notify();
                    }))
                    .separator()
                    .label("Theme")
                    .item(theme_item("System", Appearance::System))
                    .item(theme_item("Light", Appearance::Light))
                    .item(theme_item("Dark", Appearance::Dark))
                    .separator()
                    .item(item("Unlink this device…", |this, window, cx| this.confirm_unlink(window, cx)))
            },
        )
    }

    fn render_connection_banner(&self, p: &Palette) -> Option<AnyElement> {
        let (text, busy) = match &self.state.connection {
            ConnectionState::Online => return None,
            ConnectionState::Disconnected => return None,
            ConnectionState::Connecting => ("Connecting…".to_string(), true),
            ConnectionState::Syncing => ("Loading messages…".to_string(), true),
            ConnectionState::Offline { retry_in_secs, .. } => (
                format!("Not connected. Retrying in {retry_in_secs}s…"),
                false,
            ),
        };
        Some(
            div()
                .id("connection-banner")
                .mx_4()
                .mb_2()
                .px_3()
                .py_2()
                .rounded(px(8.))
                .text_size(px(12.))
                .when(!busy, |d| d.bg(p.warning_bg))
                .when(busy, |d| d.bg(p.hover))
                .text_color(p.text_secondary)
                .child(text)
                .test_support()
                .into_any_element(),
        )
    }

    fn render_conversation_list(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let now = now_millis();
        let selected = self.state.selected().copied();
        let visible: Vec<Conversation> = self
            .state
            .visible_conversations()
            .into_iter()
            .cloned()
            .collect();
        let archived_count = self.state.archived_count();
        let searching = !self.state.filter().trim().is_empty();
        let mut rows: Vec<AnyElement> = Vec::new();

        if visible.is_empty() && !searching {
            rows.push(
                div()
                    .id("empty-list")
                    .p_6()
                    .text_center()
                    .text_color(p.text_secondary)
                    .child(if self.list_mode == ListMode::Archived {
                        "No archived chats."
                    } else {
                        "No chats yet. Start a new chat with the ✎ button."
                    })
                    .test_support()
                    .into_any_element(),
            );
        }
        if searching && !visible.is_empty() {
            rows.push(section_label("Chats", p));
        }
        for conv in &visible {
            rows.push(self.render_conversation_row(conv, selected == Some(conv.id), now, p, cx));
        }
        if searching {
            if !self.message_results.is_empty() {
                rows.push(section_label("Messages", p));
                let results = self.message_results.clone();
                for m in &results {
                    rows.push(self.render_message_result(m, now, p, cx));
                }
            } else if visible.is_empty() {
                rows.push(
                    div()
                        .id("no-results")
                        .p_6()
                        .text_center()
                        .text_color(p.text_secondary)
                        .child(format!("No results for “{}”", self.state.filter().trim()))
                        .test_support()
                        .into_any_element(),
                );
            }
        }
        if self.list_mode == ListMode::Chats && !searching && archived_count > 0 {
            rows.push(
                div()
                    .id("archived-link")
                    .mx_4()
                    .my_2()
                    .py_2()
                    .rounded(px(8.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(p.text_secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(p.hover))
                    .child(format!("Archived chats ({archived_count})"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_list_mode(ListMode::Archived, window, cx)
                    }))
                    .test_support()
                    .into_any_element(),
            );
        }

        div()
            .id("conversation-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_2()
            .children(rows)
            .test_support()
            .into_any_element()
    }

    fn render_conversation_row(
        &self,
        conv: &Conversation,
        selected: bool,
        now: i64,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = conv.id;
        let unread = conv.unread_count;
        let draft = conv
            .draft
            .clone()
            .filter(|d| !d.trim().is_empty())
            .filter(|_| !selected);
        let typing = !self.state.typing_in(&id).is_empty();
        let preview: AnyElement = if conv.blocked {
            div().italic().child("Blocked").into_any_element()
        } else if !conv.accepted {
            div().italic().child("Message request").into_any_element()
        } else if typing {
            div().italic().child("typing…").into_any_element()
        } else if let Some(d) = draft {
            div()
                .flex()
                .gap_1()
                .child(div().text_color(p.danger).child("Draft:"))
                .child(div().truncate().child(first_line(&d)))
                .into_any_element()
        } else {
            let prefix = if conv.last_preview_outgoing {
                "You: "
            } else {
                ""
            };
            div()
                .truncate()
                .child(format!("{prefix}{}", conv.last_preview))
                .into_any_element()
        };

        let row = div()
            .id(conversation_element_id(&id))
            .flex()
            .items_center()
            .gap_3()
            .h(px(72.))
            .px_3()
            .rounded(px(10.))
            .cursor_pointer()
            .when(selected, |d| d.bg(p.selected))
            .when(!selected, |d| d.hover(|s| s.bg(p.hover)))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_conversation(id, window, cx)),
            )
            .child(
                Avatar::new(id.color_seed(), conv.title.clone())
                    .note_to_self(self.is_note_to_self(&id))
                    .group(id.is_group())
                    .size(px(48.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(if conv.has_unread() {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::SEMIBOLD
                                    })
                                    .child(conv.title.clone()),
                            )
                            .when(conv.muted, |d| {
                                d.child(
                                    Icon::new(Icons::BellOff)
                                        .xsmall()
                                        .text_color(p.text_secondary),
                                )
                            })
                            .when(conv.pinned, |d| {
                                d.child(Icon::new(Icons::Pin).xsmall().text_color(p.text_secondary))
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(p.text_secondary)
                                    .child(conversation_list_time(conv.last_activity, now)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.))
                                    .text_color(if unread > 0 { p.text } else { p.text_secondary })
                                    .child(preview),
                            )
                            .when(unread == 0 && conv.marked_unread, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("marked-unread-{id}")))
                                        .flex_none()
                                        .size(px(12.))
                                        .rounded_full()
                                        .bg(p.badge)
                                        .test_support(),
                                )
                            })
                            .when(unread > 0, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("unread-{id}")))
                                        .flex_none()
                                        .min_w(px(20.))
                                        .h(px(20.))
                                        .px(px(6.))
                                        .rounded_full()
                                        .bg(if conv.muted {
                                            p.text_secondary
                                        } else {
                                            p.badge
                                        })
                                        .text_color(p.accent_text)
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::BOLD)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(if unread > 999 {
                                            "999+".to_string()
                                        } else {
                                            unread.to_string()
                                        }),
                                )
                            }),
                    ),
            )
            .test_support();
        let app = cx.entity().downgrade();
        let menu_conv = conv.clone();
        let note = self.is_note_to_self(&id);
        div()
            .id(SharedString::from(format!("conversation-menu-{id}")))
            .child(row)
            .context_menu(move |menu, window, cx| {
                crate::views::menus::conversation_menu(
                    menu,
                    app.clone(),
                    &menu_conv,
                    note,
                    crate::views::menus::MenuPlace::ChatList,
                    window,
                    cx,
                )
            })
            .into_any_element()
    }

    fn render_message_result(
        &self,
        m: &Message,
        now: i64,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let conv_id = m.conversation;
        let title = self
            .state
            .conversation(&conv_id)
            .map(|c| c.title.clone())
            .or_else(|| {
                self.db
                    .conversation(&conv_id)
                    .ok()
                    .flatten()
                    .map(|c| c.title)
            })
            .unwrap_or_else(|| "Unknown".into());
        div()
            .id(ElementId::Name(format!("result-{}", m.id).into()))
            .flex()
            .items_center()
            .gap_3()
            .h(px(64.))
            .px_3()
            .rounded(px(10.))
            .cursor_pointer()
            .hover(|s| s.bg(p.hover))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_conversation(conv_id, window, cx)),
            )
            .child(
                Avatar::new(conv_id.color_seed(), title.clone())
                    .note_to_self(self.is_note_to_self(&conv_id))
                    .group(conv_id.is_group())
                    .size(px(36.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(p.text_secondary)
                                    .child(conversation_list_time(m.sent_at, now)),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(p.text_secondary)
                            .child(m.preview()),
                    ),
            )
            .test_support()
            .into_any_element()
    }

    fn render_contact_picker(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let contacts: Vec<Conversation> = self.filtered_contacts().into_iter().cloned().collect();
        let mut rows: Vec<AnyElement> = Vec::new();
        if contacts.is_empty() {
            rows.push(
                div()
                    .id("no-contacts")
                    .p_6()
                    .text_center()
                    .text_color(p.text_secondary)
                    .child("No contacts found. Contacts sync from your phone.")
                    .test_support()
                    .into_any_element(),
            );
        }
        for c in contacts {
            let id = c.id;
            rows.push(
                div()
                    .id(ElementId::Name(format!("contact-{id}").into()))
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(56.))
                    .px_3()
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_conversation(id, window, cx)
                    }))
                    .child(
                        Avatar::new(id.color_seed(), c.title.clone())
                            .note_to_self(self.is_note_to_self(&id))
                            .group(id.is_group())
                            .size(px(36.)),
                    )
                    .child(div().flex_1().truncate().child(c.title.clone()))
                    .test_support()
                    .into_any_element(),
            );
        }
        div()
            .id("contact-picker")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .children(rows)
            .test_support()
            .into_any_element()
    }
}

fn section_label(label: &'static str, p: &Palette) -> AnyElement {
    div()
        .px_3()
        .pt_3()
        .pb_1()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text_secondary)
        .child(label)
        .into_any_element()
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().to_string()
}
