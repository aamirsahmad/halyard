//! UI integration tests for Signal Desktop parity features.

use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use signal_app::views::conversation_dom_id;
use signal_app::{RecordingSink, SignalApp};
use signal_backend::demo::{DEMO_ALICE, DEMO_BOB, DEMO_GROUP, demo_account, seed};
use signal_core::{
    AccountState, BackendCommand, BackendEvent, Conversation, ConversationId, Database, Mutation,
    NewMessage, now_millis,
};

struct H {
    window: AnyWindowHandle,
    app: Entity<SignalApp>,
    sink: RecordingSink,
    _dir: tempfile::TempDir,
}

fn eid(s: impl Into<String>) -> ElementId {
    ElementId::Name(s.into().into())
}

fn alice() -> ConversationId {
    ConversationId::Direct(DEMO_ALICE)
}

/// Seeds the demo data plus `extra` mutations, opens a registered app.
fn setup_with(cx: &mut TestAppContext, extra: Vec<Mutation>) -> H {
    cx.update(gpui_kit::init);
    cx.update(signal_app::actions::bind_keys);
    let dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(dir.path().join("m.db"), None).unwrap();
    seed(&mut db, now_millis()).unwrap();
    db.apply_all(extra).unwrap();
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
        .unwrap()
    });
    let h = H {
        window,
        app,
        sink,
        _dir: dir,
    };
    h.app
        .update(cx, |app, _| app.set_focus_override(Some(true)));
    emit(
        cx,
        &h,
        BackendEvent::Account(AccountState::Registered(demo_account())),
    );
    h
}

fn setup(cx: &mut TestAppContext) -> H {
    setup_with(cx, vec![])
}

fn emit(cx: &mut TestAppContext, h: &H, event: BackendEvent) {
    let app = h.app.clone();
    cx.update_window(h.window, |_, window, cx| {
        app.update(cx, |a, cx| a.handle_event(event, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn with_app<R>(
    cx: &mut TestAppContext,
    h: &H,
    f: impl FnOnce(&mut SignalApp, &mut gpui_kit::Window, &mut gpui_kit::Context<SignalApp>) -> R,
) -> R {
    let app = h.app.clone();
    let r = cx
        .update_window(h.window, |_, window, cx| {
            app.update(cx, |a, cx| f(a, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    r
}

fn with_window<R>(
    cx: &mut TestAppContext,
    h: &H,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    let r = cx
        .update_window(h.window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    r
}

fn conv(cx: &mut TestAppContext, h: &H, id: ConversationId) -> Option<Conversation> {
    cx.update(|cx| h.app.read(cx).state().conversation(&id).cloned())
}

#[gpui_kit::test]
fn mute_for_a_duration_and_unmute(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, _, cx| a.mute_for(alice(), Some(3_600_000), cx));
    let c = conv(cx, &h, alice()).unwrap();
    assert!(c.muted);
    let until = c.muted_until.unwrap();
    assert!((until - now_millis() - 3_600_000).abs() < 5_000);
    with_app(cx, &h, |a, _, cx| a.mute_for(alice(), None, cx));
    assert_eq!(conv(cx, &h, alice()).unwrap().muted_until, Some(i64::MAX));
    with_app(cx, &h, |a, _, cx| a.unmute(alice(), cx));
    assert!(!conv(cx, &h, alice()).unwrap().muted);
}

#[gpui_kit::test]
fn mark_unread_shows_a_dot_and_opening_clears_it(cx: &mut TestAppContext) {
    let h = setup(cx);
    let bob = ConversationId::Direct(DEMO_BOB);
    with_app(cx, &h, |a, _, cx| a.set_marked_unread(bob, true, cx));
    with_window(cx, &h, |w, _| {
        assert!(w.try_find(eid(format!("marked-unread-{bob}"))).is_some())
    });
    // opening the chat marks it read
    with_window(cx, &h, |w, cx| w.click(eid(conversation_dom_id(&bob)), cx));
    // the backend would echo the read state; apply the local mutation as it does
    with_app(cx, &h, |a, _, cx| a.set_marked_unread(bob, false, cx));
    assert!(!conv(cx, &h, bob).unwrap().marked_unread);
    assert!(
        h.sink
            .take()
            .contains(&BackendCommand::MarkRead { conversation: bob })
    );
}

#[gpui_kit::test]
fn delete_chat_removes_it_from_the_list(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    with_app(cx, &h, |a, _, cx| a.delete_conversation(alice(), cx));
    assert!(conv(cx, &h, alice()).is_none());
    with_window(cx, &h, |w, _| {
        assert!(w.try_find(eid(conversation_dom_id(&alice()))).is_none());
        assert!(w.try_find("welcome").is_some());
    });
}

#[gpui_kit::test]
fn message_request_banner_accept_and_block(cx: &mut TestAppContext) {
    let stranger = uuid::Uuid::from_u128(0x5e);
    let id = ConversationId::Direct(stranger);
    let h = setup_with(
        cx,
        vec![Mutation::InsertMessage(NewMessage::text(
            id,
            stranger,
            now_millis(),
            false,
            "hi, is this Sam?",
        ))],
    );
    assert!(!conv(cx, &h, id).unwrap().accepted);
    with_app(cx, &h, |a, w, cx| a.open_conversation(id, w, cx));
    h.sink.take();
    with_window(cx, &h, |w, cx| {
        assert!(w.try_find("request-banner").is_some());
        assert!(
            w.try_find("composer").is_none(),
            "no composer before accepting"
        );
        w.click("request-accept", cx);
    });
    assert!(conv(cx, &h, id).unwrap().accepted);
    assert!(
        h.sink
            .take()
            .contains(&BackendCommand::MarkRead { conversation: id })
    );
    with_window(cx, &h, |w, _| assert!(w.try_find("composer").is_some()));

    // blocking from the header menu path
    with_app(cx, &h, |a, _, cx| a.set_blocked(id, true, cx));
    with_window(cx, &h, |w, cx| {
        assert!(w.try_find("request-banner").is_some(), "blocked banner");
        w.click("unblock", cx);
    });
    assert!(!conv(cx, &h, id).unwrap().blocked);
}

#[gpui_kit::test]
fn disappearing_timer_command_notice_and_header(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    h.sink.take();
    with_app(cx, &h, |a, _, cx| {
        a.set_expire_timer(alice(), Some(3600), cx)
    });
    assert_eq!(
        h.sink.take(),
        vec![BackendCommand::SetExpireTimer {
            conversation: alice(),
            secs: Some(3600)
        }]
    );
    // simulate the backend's echo
    let mut db = Database::open_in_memory().unwrap();
    let ch = db
        .apply(Mutation::SetExpireTimer {
            conversation: alice(),
            secs: Some(3600),
            author: signal_backend::demo::DEMO_ME,
            outgoing: true,
            at: now_millis(),
        })
        .unwrap();
    let mut c = conv(cx, &h, alice()).unwrap();
    c.expire_timer = Some(3600);
    emit(cx, &h, BackendEvent::ConversationUpdated(c));
    let mut notice = ch.messages[0].clone();
    notice.id = 10_000;
    emit(cx, &h, BackendEvent::MessageUpserted(notice));
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("header-timer").is_some());
        assert!(
            w.try_find("row-10000").is_some(),
            "timer notice in the timeline"
        );
    });
}

#[gpui_kit::test]
fn delete_for_me_removes_the_bubble(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    let id = cx.update(|cx| h.app.read(cx).state().messages()[0].id);
    with_window(cx, &h, |w, _| {
        assert!(w.try_find(eid(format!("row-{id}"))).is_some())
    });
    with_app(cx, &h, |a, _, cx| a.delete_for_me(id, cx));
    cx.update(|cx| assert!(h.app.read(cx).state().message(id).is_none()));
    with_window(cx, &h, |w, _| {
        assert!(w.try_find(eid(format!("row-{id}"))).is_none())
    });
}

#[gpui_kit::test]
fn forward_to_selected_chats(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    let id = cx.update(|cx| h.app.read(cx).state().messages()[0].id);
    with_app(cx, &h, |a, w, cx| a.open_forward(id, w, cx));
    let bob = ConversationId::Direct(DEMO_BOB);
    let group = ConversationId::Group(DEMO_GROUP);
    h.sink.take();
    with_window(cx, &h, |w, cx| {
        w.click(eid(format!("forward-check-{bob}")), cx);
        w.click(eid(format!("forward-check-{group}")), cx);
        w.click("forward-send", cx);
    });
    let cmds = h.sink.take();
    let fwd = cmds.iter().find_map(|c| match c {
        BackendCommand::Forward { targets, body, .. } => Some((targets.clone(), body.clone())),
        _ => None,
    });
    let (targets, body) = fwd.expect("forward command");
    assert!(targets.contains(&bob) && targets.contains(&group));
    assert!(body.contains("coffee"));
}

#[gpui_kit::test]
fn hover_reaction_bar_reacts(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    let id = cx.update(|cx| h.app.read(cx).state().messages()[0].id);
    h.sink.take();
    with_window(cx, &h, |w, cx| {
        w.hover(eid(format!("row-{id}")), cx);
        w.click(eid(format!("react-{id}")), cx);
    });
    with_window(cx, &h, |w, cx| {
        w.click(eid(format!("quick-react-{id}-👍")), cx)
    });
    let cmds = h.sink.take();
    assert!(
        cmds.iter().any(
            |c| matches!(c, BackendCommand::React { emoji, remove: false, .. } if emoji == "👍")
        ),
        "{cmds:?}"
    );
}

#[gpui_kit::test]
fn composer_emoji_picker_inserts_emoji(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    with_window(cx, &h, |w, cx| w.click("emoji", cx));
    with_window(cx, &h, |w, cx| {
        w.within("composer-emoji").click("input-state", cx);
        w.input("party", cx);
    });
    with_window(cx, &h, |w, cx| w.click(eid("composer-emoji-🎉"), cx));
    cx.update(|cx| assert_eq!(h.app.read(cx).composer().read(cx).value().as_ref(), "🎉"));
}

#[gpui_kit::test]
fn preferences_toggle_read_receipts_and_enter_to_send(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_preferences(w, cx));
    h.sink.take();
    with_window(cx, &h, |w, cx| {
        assert!(w.try_find("preferences").is_some());
        w.click("pref-read-receipts", cx);
    });
    assert!(
        h.sink
            .take()
            .contains(&BackendCommand::SetReadReceipts(false))
    );
    cx.update(|cx| assert!(!h.app.read(cx).prefs().read_receipts));
    with_window(cx, &h, |w, cx| w.click("pref-enter-sends", cx));
    cx.update(|cx| assert!(!h.app.read(cx).prefs().enter_sends));

    // with Enter-to-send off, Enter adds a newline and ⌘/Ctrl+Enter sends
    with_app(cx, &h, |a, w, cx| {
        w.close_dialog(cx);
        a.open_conversation(alice(), w, cx);
    });
    h.sink.take();
    with_window(cx, &h, |w, cx| {
        w.input("line one", cx);
        w.press("enter", cx);
    });
    assert!(
        !h.sink
            .take()
            .iter()
            .any(|c| matches!(c, BackendCommand::SendText { .. }))
    );
    with_window(cx, &h, |w, cx| w.press("secondary-enter", cx));
    let sent = h.sink.take().into_iter().find_map(|c| match c {
        BackendCommand::SendText { body, .. } => Some(body),
        _ => None,
    });
    assert_eq!(sent.as_deref(), Some("line one"));
}

#[gpui_kit::test]
fn search_within_a_conversation_jumps(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    with_window(cx, &h, |w, cx| {
        w.click("conversation-search", cx);
        w.input("table", cx);
    });
    let result = cx.update(|cx| h.app.read(cx).conv_search_result_ids());
    assert_eq!(result.len(), 1);
    with_window(cx, &h, |w, cx| {
        w.click(eid(format!("search-result-{}", result[0])), cx)
    });
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("conversation-search-bar").is_none())
    });
}

#[gpui_kit::test]
fn unread_divider_marks_first_unread(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| {
        a.open_conversation(ConversationId::Group(DEMO_GROUP), w, cx)
    });
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("unread-marker").is_some())
    });
    // chats with nothing unread have no divider
    with_app(cx, &h, |a, w, cx| {
        a.open_conversation(ConversationId::Direct(DEMO_BOB), w, cx)
    });
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("unread-marker").is_none())
    });
}

#[gpui_kit::test]
fn shortcuts_and_profile_dialogs(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_shortcuts(w, cx));
    with_window(cx, &h, |w, _| assert!(w.try_find("shortcuts").is_some()));
    with_app(cx, &h, |_, w, cx| w.close_dialog(cx));
    with_window(cx, &h, |w, cx| w.click("my-avatar", cx));
    with_window(cx, &h, |w, _| assert!(w.try_find("profile").is_some()));
}

#[gpui_kit::test]
fn message_info_shows_edit_history(cx: &mut TestAppContext) {
    let target_ts = now_millis() - 60_000;
    let h = setup_with(
        cx,
        vec![
            Mutation::InsertMessage(NewMessage::text(
                alice(),
                DEMO_ALICE,
                target_ts,
                false,
                "first",
            )),
            Mutation::Edit {
                conversation: alice(),
                author: DEMO_ALICE,
                target_sent_at: target_ts,
                body: "second".into(),
                edited_at: target_ts + 1,
            },
        ],
    );
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    let id = cx.update(|cx| {
        h.app
            .read(cx)
            .state()
            .messages()
            .iter()
            .find(|m| m.body == "second")
            .unwrap()
            .id
    });
    with_app(cx, &h, |a, w, cx| a.show_message_info(id, w, cx));
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("message-info").is_some());
        assert!(w.try_find("edit-history").is_some());
    });
}

#[gpui_kit::test]
fn safety_number_request_and_dialog(cx: &mut TestAppContext) {
    let h = setup(cx);
    h.sink.take();
    with_app(cx, &h, |a, _, cx| a.request_safety_number(DEMO_ALICE, cx));
    assert_eq!(
        h.sink.take(),
        vec![BackendCommand::RequestSafetyNumber {
            contact: DEMO_ALICE
        }]
    );
    let number = signal_backend::demo::demo_safety_number(&DEMO_ALICE);
    assert_eq!(number.len(), 60);
    emit(
        cx,
        &h,
        BackendEvent::SafetyNumber {
            contact: DEMO_ALICE,
            number: Some(number),
        },
    );
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("safety-number").is_some())
    });
}

#[gpui_kit::test]
fn link_preview_card_renders(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| {
        a.open_conversation(
            ConversationId::Direct(signal_backend::demo::DEMO_CAROL),
            w,
            cx,
        )
    });
    let id = cx.update(|cx| {
        h.app
            .read(cx)
            .state()
            .messages()
            .iter()
            .find(|m| m.link_preview.is_some())
            .unwrap()
            .id
    });
    with_window(cx, &h, |w, _| {
        assert!(w.try_find(eid(format!("link-preview-{id}"))).is_some())
    });
}

#[gpui_kit::test]
fn audio_attachment_play_button(cx: &mut TestAppContext) {
    // a 0.5 s silent WAV
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("voice.wav");
    let samples = 4000u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    for v in [1u16, 1] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    for v in [8000u32, 16000] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    for v in [2u16, 16] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    wav.resize(wav.len() + (samples * 2) as usize, 0);
    std::fs::write(&path, wav).unwrap();

    let mut m = NewMessage::text(alice(), DEMO_ALICE, now_millis() - 1000, false, "");
    m.attachments = vec![signal_core::Attachment {
        content_type: "audio/wav".into(),
        file_name: Some("voice.wav".into()),
        size: 8044,
        local_path: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    }];
    let h = setup_with(cx, vec![Mutation::InsertMessage(m)]);
    with_app(cx, &h, |a, w, cx| a.open_conversation(alice(), w, cx));
    let id = cx.update(|cx| {
        h.app
            .read(cx)
            .state()
            .messages()
            .iter()
            .find(|m| !m.attachments.is_empty())
            .unwrap()
            .id
    });
    with_window(cx, &h, |w, cx| {
        w.click(eid(format!("audio-play-{id}-0")), cx)
    });
    // the audio thread either starts playing or reports a missing device
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let status = loop {
        let st = cx.update(|cx| h.app.read(cx).audio_status());
        if st.current.is_some() || st.error.is_some() || std::time::Instant::now() > deadline {
            break st;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        status.current == Some((id, 0)) || status.error.is_some(),
        "{status:?}"
    );
}

#[gpui_kit::test]
fn stories_list_and_viewer(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_window(cx, &h, |w, cx| {
        assert!(
            w.try_find("stories-badge").is_some(),
            "unviewed stories badge"
        );
        w.click("stories", cx);
    });
    with_window(cx, &h, |w, cx| {
        assert!(w.try_find("story-list").is_some());
        w.click(eid(format!("story-author-{DEMO_ALICE}")), cx);
    });
    with_window(cx, &h, |w, cx| {
        assert!(w.try_find("story-viewer").is_some());
        w.click("story-Next", cx);
    });
    cx.update(|cx| {
        let app = h.app.read(cx);
        let alice: Vec<_> = app
            .stories()
            .iter()
            .filter(|s| s.author == DEMO_ALICE)
            .collect();
        assert_eq!(alice.len(), 2);
        assert!(
            alice.iter().all(|s| s.viewed),
            "both of Alice's stories viewed"
        );
        assert_eq!(app.unviewed_story_count(), 1, "Carol's story is still new");
    });
}

#[gpui_kit::test]
fn zoom_shortcuts_scale_the_ui(cx: &mut TestAppContext) {
    let h = setup(cx);
    let rem = |cx: &mut TestAppContext| cx.update_window(h.window, |_, w, _| w.rem_size()).unwrap();
    assert_eq!(rem(cx), px(16.));
    with_app(cx, &h, |_, w, cx| w.focus(&cx.focus_handle(), cx));
    with_app(cx, &h, |a, w, cx| a.zoom(1, w, cx));
    cx.update(|cx| assert_eq!(h.app.read(cx).prefs().zoom, 1.1));
    assert_eq!(rem(cx), px(16. * 1.1));
    with_app(cx, &h, |a, w, cx| {
        a.zoom(-1, w, cx);
        a.zoom(-1, w, cx);
    });
    cx.update(|cx| assert_eq!(h.app.read(cx).prefs().zoom, 0.9));
    with_app(cx, &h, |a, w, cx| a.zoom(0, w, cx));
    assert_eq!(rem(cx), px(16.));
}

#[gpui_kit::test]
fn reply_to_a_story(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_story_viewer(DEMO_ALICE, w, cx));
    h.sink.take();
    with_window(cx, &h, |w, cx| w.click(eid("story-react-❤️"), cx));
    with_window(cx, &h, |w, cx| {
        w.click("story-reply-input", cx);
        w.input("See you there!", cx);
        w.click("story-reply-send", cx);
    });
    let replies: Vec<String> = h
        .sink
        .take()
        .into_iter()
        .filter_map(|c| match c {
            BackendCommand::ReplyToStory { author, body, .. } if author == DEMO_ALICE => Some(body),
            _ => None,
        })
        .collect();
    assert_eq!(
        replies,
        vec!["❤️".to_string(), "See you there!".to_string()]
    );
}

#[gpui_kit::test]
fn mic_button_when_composer_is_empty(cx: &mut TestAppContext) {
    let h = setup(cx);
    with_app(cx, &h, |a, w, cx| a.open_conversation(ConversationId::Direct(DEMO_BOB), w, cx));
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("record").is_some(), "mic shown for an empty composer");
        assert!(w.try_find("send").is_none());
    });
    with_window(cx, &h, |w, cx| w.input("hi", cx));
    with_window(cx, &h, |w, _| {
        assert!(w.try_find("send").is_some());
        assert!(w.try_find("record").is_none());
    });
    cx.update(|cx| assert!(!h.app.read(cx).is_recording()));
}
