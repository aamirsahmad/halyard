//! UI integration tests: the production `SignalApp` view rendered in a
//! headless window, driven by real clicks and keystrokes.

use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use signal_app::app::ListMode;
use signal_app::views::conversation_dom_id;
use signal_app::{RecordingSink, SignalApp};
use signal_backend::demo::{
    DEMO_ALICE, DEMO_BOB, DEMO_CAROL, DEMO_DAVE, DEMO_GROUP, DEMO_ME, DEMO_STRANGER, demo_account,
    seed,
};
use signal_core::{
    AccountState, BackendCommand, BackendEvent, ConversationId, Database, Message, MessageKind,
    MessageStatus, now_millis,
};

struct Harness {
    window: AnyWindowHandle,
    app: Entity<SignalApp>,
    sink: RecordingSink,
    _dir: tempfile::TempDir,
}

fn eid(s: String) -> gpui_kit::ElementId {
    gpui_kit::ElementId::Name(s.into())
}

fn alice() -> ConversationId {
    ConversationId::Direct(DEMO_ALICE)
}

fn setup(cx: &mut TestAppContext, registered: bool) -> Harness {
    cx.update(gpui_kit::init);
    cx.update(signal_app::actions::bind_keys);
    let dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(dir.path().join("m.db"), None).unwrap();
    seed(&mut db, now_millis()).unwrap();
    let sink = RecordingSink::default();
    let s = sink.clone();
    let (window, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(1100.), px(760.)),
                ))),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                cx.new(|cx| SignalApp::new(db, Rc::new(s.clone()), Rc::new(s), window, cx))
            },
        )
        .expect("open window")
    });
    let h = Harness {
        window,
        app,
        sink,
        _dir: dir,
    };
    // Headless test windows don't report OS focus; assume the user is looking.
    h.app
        .update(cx, |app, _| app.set_focus_override(Some(true)));
    if registered {
        emit(
            cx,
            &h,
            BackendEvent::Account(AccountState::Registered(demo_account())),
        );
    }
    h
}

fn emit(cx: &mut TestAppContext, h: &Harness, event: BackendEvent) {
    let app = h.app.clone();
    cx.update_window(h.window, |_, window, cx| {
        app.update(cx, |app, cx| app.handle_event(event, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

fn with_window<R>(
    cx: &mut TestAppContext,
    h: &Harness,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx)
    })
    .unwrap()
}

fn incoming(id: i64, conv: ConversationId, author: uuid::Uuid, body: &str) -> Message {
    Message {
        id,
        conversation: conv,
        author,
        sent_at: now_millis(),
        received_at: now_millis(),
        outgoing: false,
        body: body.into(),
        attachments: vec![],
        quote: None,
        reactions: vec![],
        status: MessageStatus::Received,
        read: false,
        edited_at: None,
        deleted: false,
        expires_in_secs: None,
        expires_at: None,
        kind: MessageKind::Normal,
        body_ranges: vec![],
        edit_history: vec![],
        link_preview: None,
    }
}

#[gpui_kit::test]
fn lists_seeded_conversations_pinned_first(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    with_window(cx, &h, |window, _| {
        let group = window.find(eid(conversation_dom_id(&ConversationId::Group(DEMO_GROUP))));
        let alice = window.find(eid(conversation_dom_id(&alice())));
        assert!(group.visible() && alice.visible());
        assert!(
            group.bounds().top() < alice.bounds().top(),
            "pinned group is listed first"
        );
        assert!(window.try_find("welcome").is_some(), "no chat open yet");
    });
}

#[gpui_kit::test]
fn clicking_a_conversation_opens_it_and_marks_it_read(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let id = eid(conversation_dom_id(&alice()));
    with_window(cx, &h, |window, cx| window.click(id.clone(), cx));
    cx.run_until_parked();
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("conversation-title").is_some());
        assert!(window.try_find("welcome").is_none());
    });
    cx.update(|cx| {
        let app = h.app.read(cx);
        assert_eq!(app.state().selected(), Some(&alice()));
        assert_eq!(app.timeline().messages.len(), 4);
    });
    assert!(h.sink.take().contains(&BackendCommand::MarkRead {
        conversation: alice()
    }));
}

#[gpui_kit::test]
fn typing_and_pressing_enter_sends_a_message(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.open_conversation(alice(), window, cx));
    });
    cx.run_until_parked();
    h.sink.take();
    with_window(cx, &h, |window, cx| {
        window.input("Hello from GPUI", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    let commands = h.sink.take();
    let sent: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            BackendCommand::SendText {
                conversation,
                body,
                quote,
                attachments,
            } => Some((
                *conversation,
                body.clone(),
                quote.clone(),
                attachments.len(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        sent,
        vec![(alice(), "Hello from GPUI".to_string(), None, 0)]
    );
    assert!(
        commands.contains(&BackendCommand::Typing {
            conversation: alice(),
            started: true
        }),
        "typing indicator is sent while composing: {commands:?}"
    );
    cx.update(|cx| assert_eq!(h.app.read(cx).composer().read(cx).value().as_ref(), ""));

    // an empty composer sends nothing
    with_window(cx, &h, |window, cx| window.press("enter", cx));
    cx.run_until_parked();
    assert!(
        !h.sink
            .take()
            .iter()
            .any(|c| matches!(c, BackendCommand::SendText { .. }))
    );
}

#[gpui_kit::test]
fn reply_includes_a_quote(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| {
            app.open_conversation(alice(), window, cx);
            let target = app.state().messages().first().unwrap().id;
            app.reply_to(target, window, cx);
        });
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, cx| {
        assert!(window.try_find("reply-preview").is_some());
        window.input("Yes!", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    let quote = h.sink.take().into_iter().find_map(|c| match c {
        BackendCommand::SendText { quote, .. } => quote,
        _ => None,
    });
    let quote = quote.expect("quote sent");
    assert_eq!(quote.author, DEMO_ALICE);
    assert!(quote.text.contains("coffee"));
    cx.update(|cx| assert!(h.app.read(cx).replying_to().is_none()));
}

#[gpui_kit::test]
fn search_filters_the_list(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let bob = eid(conversation_dom_id(&ConversationId::Direct(DEMO_BOB)));
    let carol = eid(conversation_dom_id(&ConversationId::Direct(DEMO_CAROL)));
    with_window(cx, &h, |window, cx| {
        window.click("search", cx);
        window.input("bob", cx);
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, _| {
        assert!(window.try_find(bob.clone()).is_some());
        assert!(window.try_find(carol.clone()).is_none());
    });
    // message search finds bodies too
    with_window(cx, &h, |window, cx| {
        for _ in 0..3 {
            window.press("backspace", cx);
        }
        window.input("trailhead", cx);
    });
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(h.app.read(cx).state().filter(), "trailhead"));
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("no-results").is_none(), "a message matched");
    });
}

#[gpui_kit::test]
fn new_chat_picker_opens_a_contact_without_history(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    with_window(cx, &h, |window, cx| window.click("new-chat", cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(h.app.read(cx).list_mode(), ListMode::NewChat));
    let dave = eid(format!("contact-{}", ConversationId::Direct(DEMO_DAVE)));
    with_window(cx, &h, |window, cx| {
        assert!(
            window
                .try_find(eid(format!("contact-{}", ConversationId::Direct(DEMO_ME))))
                .is_some(),
            "Note to Self is offered"
        );
        window.click(dave.clone(), cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let app = h.app.read(cx);
        assert_eq!(app.list_mode(), ListMode::Chats);
        assert_eq!(
            app.state().selected(),
            Some(&ConversationId::Direct(DEMO_DAVE))
        );
        assert!(app.state().messages().is_empty());
    });
}

#[gpui_kit::test]
fn incoming_messages_update_list_and_notify_when_inactive(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    h.app
        .update(cx, |app, _| app.set_focus_override(Some(false)));
    let mut conv = signal_core::Conversation::new(ConversationId::Direct(DEMO_BOB), "Bob Smith");
    conv.last_activity = now_millis();
    conv.last_preview = "new!".into();
    conv.unread_count = 1;
    emit(cx, &h, BackendEvent::ConversationUpdated(conv));
    emit(
        cx,
        &h,
        BackendEvent::MessageUpserted(incoming(
            999,
            ConversationId::Direct(DEMO_BOB),
            DEMO_BOB,
            "new!",
        )),
    );
    let bob = eid(conversation_dom_id(&ConversationId::Direct(DEMO_BOB)));
    let group = eid(conversation_dom_id(&ConversationId::Group(DEMO_GROUP)));
    let alice_id = eid(conversation_dom_id(&alice()));
    with_window(cx, &h, |window, _| {
        // Bob jumped above Alice (pinned group stays on top)
        assert!(
            window.find(bob.clone()).bounds().top() < window.find(alice_id.clone()).bounds().top()
        );
        assert!(
            window.find(group.clone()).bounds().top() < window.find(bob.clone()).bounds().top()
        );
    });
    cx.update(|cx| assert_eq!(h.app.read(cx).state().total_unread(), 6));
    assert_eq!(
        h.sink.take_notifications(),
        vec![("Bob Smith".to_string(), "new!".to_string())]
    );
    assert!(
        h.sink.take().is_empty(),
        "nothing is marked read while unfocused"
    );
}

#[gpui_kit::test]
fn incoming_message_in_open_chat_is_marked_read(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.open_conversation(alice(), window, cx))
    });
    h.sink.take();
    emit(
        cx,
        &h,
        BackendEvent::MessageUpserted(incoming(1000, alice(), DEMO_ALICE, "are you there?")),
    );
    assert_eq!(
        h.sink.take(),
        vec![BackendCommand::MarkRead {
            conversation: alice()
        }]
    );
    cx.update(|cx| {
        let app = h.app.read(cx);
        assert_eq!(
            app.timeline().messages.last().unwrap().body,
            "are you there?"
        );
    });
    emit(
        cx,
        &h,
        BackendEvent::Typing {
            conversation: alice(),
            who: DEMO_ALICE,
            started: true,
        },
    );
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("typing-indicator").is_some())
    });
}

#[gpui_kit::test]
fn archiving_moves_a_chat_out_of_the_list(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    let alice_id = eid(conversation_dom_id(&alice()));
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| {
            app.open_conversation(alice(), window, cx);
            app.toggle_archived(alice(), cx);
        })
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, cx| {
        assert!(window.try_find(alice_id.clone()).is_none());
        assert!(
            window.try_find("welcome").is_some(),
            "archived chat was closed"
        );
        window.click("archived-link", cx);
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, _| {
        assert!(window.try_find(alice_id.clone()).is_some())
    });
}

#[gpui_kit::test]
fn link_screen_requests_linking(cx: &mut TestAppContext) {
    let h = setup(cx, false);
    emit(cx, &h, BackendEvent::Account(AccountState::Unlinked));
    with_window(cx, &h, |window, cx| {
        assert!(window.try_find("link-screen").is_some());
        assert!(window.try_find("sidebar").is_none());
        window.click("link-device", cx);
    });
    cx.run_until_parked();
    let cmds = h.sink.take();
    assert!(
        matches!(cmds.as_slice(), [BackendCommand::Link { device_name }] if !device_name.is_empty())
    );
    emit(
        cx,
        &h,
        BackendEvent::Account(AccountState::Linking {
            url: Some("sgnl://linkdevice?uuid=x&pub_key=y".into()),
        }),
    );
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("link-qr").is_some());
        assert!(
            window.try_find("link-device").is_none(),
            "no second link while linking"
        );
    });
    emit(
        cx,
        &h,
        BackendEvent::Account(AccountState::LinkFailed("timed out".into())),
    );
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("link-error").is_some());
        assert!(window.try_find("link-device").is_some(), "can retry");
    });
    emit(
        cx,
        &h,
        BackendEvent::Account(AccountState::Registered(demo_account())),
    );
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("sidebar").is_some())
    });
}

#[gpui_kit::test]
fn keyboard_navigation_between_chats(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| {
            app.select_relative(1, window, cx);
        })
    });
    cx.update(|cx| {
        assert_eq!(
            h.app.read(cx).state().selected(),
            Some(&ConversationId::Group(DEMO_GROUP))
        );
    });
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.select_relative(1, window, cx))
    });
    cx.update(|cx| {
        assert_eq!(
            h.app.read(cx).state().selected(),
            Some(&ConversationId::Direct(DEMO_STRANGER))
        )
    });
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.select_relative(-5, window, cx))
    });
    cx.update(|cx| {
        assert_eq!(
            h.app.read(cx).state().selected(),
            Some(&ConversationId::Group(DEMO_GROUP))
        );
    });
}

#[gpui_kit::test]
fn drafts_are_kept_per_conversation(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.open_conversation(alice(), window, cx))
    });
    with_window(cx, &h, |window, cx| window.input("unfinished thought", cx));
    cx.run_until_parked();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| {
            app.open_conversation(ConversationId::Direct(DEMO_BOB), window, cx)
        })
    });
    cx.update(|cx| {
        let app = h.app.read(cx);
        assert_eq!(app.composer().read(cx).value().as_ref(), "");
        let a = app.state().conversation(&alice()).unwrap();
        assert_eq!(a.draft.as_deref(), Some("unfinished thought"));
    });
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.open_conversation(alice(), window, cx))
    });
    cx.update(|cx| {
        assert_eq!(
            h.app.read(cx).composer().read(cx).value().as_ref(),
            "unfinished thought"
        )
    });
}

#[gpui_kit::test]
fn clicking_group_header_lists_its_members(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let group = ConversationId::Group(DEMO_GROUP);
    let app = h.app.clone();
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.open_conversation(group, window, cx))
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, cx| {
        assert!(window.try_find("conversation-details-panel").is_none());
        window.click("conversation-details", cx);
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, _| {
        let panel = window.find("conversation-details-panel");
        assert!(panel.visible());
        for member in [DEMO_ME, DEMO_ALICE, DEMO_BOB, DEMO_CAROL] {
            assert!(
                window.try_find(eid(format!("member-{member}"))).is_some(),
                "member {member} listed"
            );
        }
        // Dave is a contact but not in the group
        assert!(
            window
                .try_find(eid(format!("member-{DEMO_DAVE}")))
                .is_none()
        );
    });
    cx.update(|cx| {
        let names: Vec<String> = h
            .app
            .read(cx)
            .members(&group)
            .into_iter()
            .map(|m| m.1)
            .collect();
        assert_eq!(
            names,
            vec!["You", "Alice Johnson", "Bob Smith", "Carol Diaz"]
        );
    });
}

#[gpui_kit::test]
fn note_to_self_is_listed_first_in_picker_and_titled(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let note = ConversationId::Direct(DEMO_ME);
    // the seeded note conversation shows up in the chat list with its title
    cx.update(|cx| {
        let app = h.app.read(cx);
        let conv = app
            .state()
            .conversation(&note)
            .expect("note to self listed");
        assert_eq!(conv.title, "Note to Self");
    });
    with_window(cx, &h, |window, cx| window.click("new-chat", cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let app = h.app.read(cx);
        let first = app.filtered_contacts()[0];
        assert_eq!(first.id, note);
        assert_eq!(first.title, "Note to Self");
    });
    with_window(cx, &h, |window, cx| {
        window.click(eid(format!("contact-{note}")), cx)
    });
    cx.run_until_parked();
    h.sink.take();
    with_window(cx, &h, |window, cx| {
        window.input("remember the milk", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    let cmds = h.sink.take();
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, BackendCommand::Typing { .. })),
        "no typing indicators to yourself: {cmds:?}"
    );
    assert!(cmds.iter().any(|c| matches!(
        c,
        BackendCommand::SendText { conversation, body, .. } if *conversation == note && body == "remember the milk"
    )));
}

#[gpui_kit::test]
fn editing_an_own_message_sends_an_edit(cx: &mut TestAppContext) {
    let h = setup(cx, true);
    let app = h.app.clone();
    // Alice's chat has one outgoing message from 18 minutes ago
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.open_conversation(alice(), window, cx))
    });
    let (own_id, own_sent_at, incoming_id) = cx.update(|cx| {
        let msgs = h.app.read(cx).state().messages().to_vec();
        let own = msgs.iter().find(|m| m.outgoing).unwrap();
        let other = msgs.iter().find(|m| !m.outgoing).unwrap();
        (own.id, own.sent_at, other.id)
    });
    // incoming messages can't be edited
    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.start_edit(incoming_id, window, cx))
    });
    cx.update(|cx| assert!(h.app.read(cx).editing().is_none()));

    with_window(cx, &h, |window, cx| {
        app.update(cx, |app, cx| app.start_edit(own_id, window, cx))
    });
    cx.run_until_parked();
    with_window(cx, &h, |window, _| {
        assert!(window.try_find("edit-banner").is_some())
    });
    cx.update(|cx| {
        let text = h.app.read(cx).composer().read(cx).value().to_string();
        assert!(text.starts_with("Absolutely"), "composer prefilled: {text}");
    });
    h.sink.take();
    with_window(cx, &h, |window, cx| {
        window.press("secondary-a", cx);
        window.input("Make it 11am instead", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    let edits: Vec<_> = h
        .sink
        .take()
        .into_iter()
        .filter(|c| matches!(c, BackendCommand::Edit { .. }))
        .collect();
    assert_eq!(
        edits,
        vec![BackendCommand::Edit {
            conversation: alice(),
            target_sent_at: own_sent_at,
            body: "Make it 11am instead".into()
        }]
    );
    cx.update(|cx| assert!(h.app.read(cx).editing().is_none()));
}
