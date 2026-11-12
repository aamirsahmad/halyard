//! Conversation menus shared by the chat list (right-click) and the header.

use gpui_kit::component::Icon;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Context, WeakEntity, Window};
use signal_core::{Conversation, MUTE_PRESETS, TIMER_PRESETS, timer_label};

use crate::app::SignalApp;
use crate::assets::Icons;

/// Where the menu is shown; the header offers chat-level settings too.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuPlace {
    ChatList,
    Header,
}

fn item(
    app: &WeakEntity<SignalApp>,
    label: impl Into<gpui_kit::SharedString>,
    f: impl Fn(&mut SignalApp, &mut Window, &mut Context<SignalApp>) + 'static,
) -> PopupMenuItem {
    let app = app.clone();
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        let _ = app.update(cx, |this, cx| f(this, window, cx));
    })
}

pub fn conversation_menu(
    mut menu: PopupMenu,
    app: WeakEntity<SignalApp>,
    conv: &Conversation,
    is_note_to_self: bool,
    place: MenuPlace,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let id = conv.id;
    if place == MenuPlace::Header {
        menu = menu.item(
            item(
                &app,
                if id.is_group() {
                    "Group members"
                } else {
                    "Chat details"
                },
                move |this, window, cx| this.show_details(id, window, cx),
            )
            .icon(Icon::new(Icons::Users)),
        );
        let current = conv.expire_timer;
        let a = app.clone();
        menu = menu.submenu_with_icon(
            Some(Icon::new(Icons::Clock)),
            "Disappearing messages",
            window,
            cx,
            move |mut sub, _, _| {
                sub = sub.item(
                    item(&a, "Off", move |this, _, cx| {
                        this.set_expire_timer(id, None, cx)
                    })
                    .checked(current.is_none()),
                );
                for secs in TIMER_PRESETS {
                    sub = sub.item(
                        item(&a, timer_label(secs), move |this, _, cx| {
                            this.set_expire_timer(id, Some(secs), cx)
                        })
                        .checked(current == Some(secs)),
                    );
                }
                sub
            },
        );
        menu = menu.separator();
    }

    menu = menu.item(
        item(
            &app,
            if conv.pinned {
                "Unpin chat"
            } else {
                "Pin chat"
            },
            move |this, _, cx| this.toggle_pinned(id, cx),
        )
        .icon(Icon::new(if conv.pinned {
            Icons::PinOff
        } else {
            Icons::Pin
        })),
    );

    if conv.muted {
        menu = menu.item(
            item(&app, "Unmute", move |this, _, cx| this.unmute(id, cx))
                .icon(Icon::new(Icons::Bell)),
        );
    } else if !is_note_to_self {
        let a = app.clone();
        menu = menu.submenu_with_icon(
            Some(Icon::new(Icons::BellOff)),
            "Mute notifications",
            window,
            cx,
            move |mut sub, _, _| {
                for (label, duration) in MUTE_PRESETS {
                    sub = sub.item(item(&a, label, move |this, _, cx| {
                        this.mute_for(id, duration, cx)
                    }));
                }
                sub
            },
        );
    }

    if place == MenuPlace::ChatList {
        menu = if conv.has_unread() {
            menu.item(item(&app, "Mark as read", move |this, _, cx| {
                this.set_marked_unread(id, false, cx)
            }))
        } else {
            menu.item(item(&app, "Mark as unread", move |this, _, cx| {
                this.set_marked_unread(id, true, cx)
            }))
        };
    }

    menu = menu.item(
        item(
            &app,
            if conv.archived {
                "Unarchive"
            } else {
                "Archive"
            },
            move |this, _, cx| this.toggle_archived(id, cx),
        )
        .icon(Icon::new(if conv.archived {
            Icons::ArchiveRestore
        } else {
            Icons::Archive
        })),
    );

    if place == MenuPlace::Header
        && !is_note_to_self
        && let Some(contact) = id.direct_uuid()
    {
        menu = menu.item(
            item(&app, "View safety number", move |this, _, cx| {
                this.request_safety_number(contact, cx)
            })
            .icon(Icon::new(Icons::Lock)),
        );
    }
    if place == MenuPlace::Header && !id.is_group() && !is_note_to_self {
        let blocked = conv.blocked;
        menu = menu.item(
            item(
                &app,
                if blocked { "Unblock" } else { "Block" },
                move |this, _, cx| this.set_blocked(id, !blocked, cx),
            )
            .icon(Icon::new(Icons::Ban)),
        );
    }

    if place == MenuPlace::Header
        && let signal_core::ConversationId::Group(key) = id
        && !conv.left
    {
        menu = menu.separator().item(
            item(&app, "Leave group…", move |this, window, cx| {
                this.confirm_leave_group(key, window, cx)
            })
            .icon(Icon::new(Icons::LogOut)),
        );
    }

    menu = menu.separator().item(
        item(&app, "Delete chat…", move |this, window, cx| {
            this.confirm_delete_conversation(id, window, cx)
        })
        .icon(Icon::new(Icons::Trash)),
    );
    if place == MenuPlace::Header {
        menu = menu.item(item(&app, "Close chat", |this, _, cx| {
            this.close_conversation(cx)
        }));
    }
    menu
}
