//! Offscreen rendering of the real UI (Metal on macOS, WGPU on Linux).
//! Verifies screens paint without panicking, differ by theme, and optionally
//! writes PNG screenshots to `$SCREENSHOT_DIR`.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    rendering::run();
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod rendering {
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, HeadlessAppContext, WindowBounds, WindowOptions, px, size};
    use signal_app::assets::Assets;
    use signal_app::{RecordingSink, SignalApp};
    use signal_backend::demo::{DEMO_ALICE, DEMO_GROUP, demo_account, seed};
    use signal_core::{
        AccountState, BackendEvent, ConversationId, Database, Mutation, NewMessage, now_millis,
    };

    fn context() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(gpui_kit::init);
        // deterministic captures: skip entrance animations
        cx.update(|cx| cx.set_reduce_motion(true));
        cx
    }

    fn capture(
        name: &str,
        mode: ThemeMode,
        setup: impl FnOnce(&mut SignalApp, &mut gpui_kit::Window, &mut gpui_kit::Context<SignalApp>)
        + 'static,
    ) -> image::RgbaImage {
        capture_sized(name, mode, 1120., |_| {}, setup)
    }

    fn capture_sized(
        name: &str,
        mode: ThemeMode,
        width: f32,
        prepare: impl FnOnce(&mut Database),
        setup: impl FnOnce(&mut SignalApp, &mut gpui_kit::Window, &mut gpui_kit::Context<SignalApp>)
        + 'static,
    ) -> image::RgbaImage {
        let mut cx = context();
        cx.update(|cx| {
            Theme::change(mode, None, cx);
            signal_app::theme::apply_signal_accent(cx);
        });
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("m.db"), None).unwrap();
        seed(&mut db, now_millis()).unwrap();
        prepare(&mut db);
        let sink = RecordingSink::default();
        let (handle, app) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                            origin: Default::default(),
                            size: size(px(width), px(760.)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    move |window, cx| {
                        cx.new(|cx| {
                            SignalApp::new(db, Rc::new(sink.clone()), Rc::new(sink), window, cx)
                        })
                    },
                )
            })
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| setup(app, window, cx))
        })
        .unwrap();
        for _ in 0..3 {
            cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.run_until_parked();
        }
        let image = cx
            .capture_screenshot(handle.into())
            .expect("offscreen renderer");
        if let Ok(out) = std::env::var("SCREENSHOT_DIR") {
            std::fs::create_dir_all(&out).unwrap();
            let path = std::path::Path::new(&out).join(format!("{name}.png"));
            image.save(&path).unwrap();
            println!("saved {}", path.display());
        }
        image
    }

    fn registered(
        app: &mut SignalApp,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<SignalApp>,
    ) {
        app.handle_event(
            BackendEvent::Account(AccountState::Registered(demo_account())),
            window,
            cx,
        );
    }

    pub fn run() {
        let link = capture("link", ThemeMode::Light, |app, window, cx| {
            app.handle_event(
                BackendEvent::Account(AccountState::Linking {
                    url: Some("sgnl://linkdevice?uuid=AAAAAAAAAAAAAAAAAAAAAA&pub_key=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".into()),
                }),
                window,
                cx,
            );
        });
        let light = capture("chat-light", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_conversation(ConversationId::Group(DEMO_GROUP), window, cx);
        });
        let dark = capture("chat-dark", ThemeMode::Dark, |app, window, cx| {
            app.set_appearance(signal_app::theme::Appearance::Dark, window, cx);
            registered(app, window, cx);
            app.open_conversation(ConversationId::Direct(DEMO_ALICE), window, cx);
        });
        let _welcome = capture("welcome", ThemeMode::Light, registered);
        let _note = capture("note-to-self", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_conversation(
                ConversationId::Direct(signal_backend::demo::DEMO_ME),
                window,
                cx,
            );
        });
        let _rich = capture("formatting", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_conversation(
                ConversationId::Direct(signal_backend::demo::DEMO_CAROL),
                window,
                cx,
            );
        });
        let _timer = capture("timer-notice", ThemeMode::Dark, |app, window, cx| {
            app.set_appearance(signal_app::theme::Appearance::Dark, window, cx);
            registered(app, window, cx);
            app.open_conversation(
                ConversationId::Direct(signal_backend::demo::DEMO_BOB),
                window,
                cx,
            );
        });
        let _request = capture("message-request", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_conversation(
                ConversationId::Direct(signal_backend::demo::DEMO_STRANGER),
                window,
                cx,
            );
        });
        let _prefs = capture("preferences", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_preferences(window, cx);
        });
        let _stories = capture("stories", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.set_list_mode(signal_app::app::ListMode::Stories, window, cx);
            app.open_story_viewer(signal_backend::demo::DEMO_ALICE, window, cx);
        });
        let zoomed = capture("zoom-125", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.update_prefs(|p| p.zoom = 1.25, window, cx);
            app.open_conversation(ConversationId::Direct(DEMO_ALICE), window, cx);
        });
        assert_ne!(zoomed.as_raw(), dark.as_raw());
        let _members = capture("group-members", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_conversation(ConversationId::Group(DEMO_GROUP), window, cx);
            app.show_details(ConversationId::Group(DEMO_GROUP), window, cx);
        });
        // a long message in a narrow window must wrap inside the timeline
        let narrow = capture_sized(
            "narrow-long-message",
            ThemeMode::Dark,
            640.,
            |db| {
                let body = "lorem ipsum dolor sit amet consectetur ".repeat(30);
                db.apply(Mutation::InsertMessage(NewMessage::text(
                    ConversationId::Direct(DEMO_ALICE),
                    signal_backend::demo::DEMO_ME,
                    now_millis(),
                    true,
                    body,
                )))
                .unwrap();
            },
            |app, window, cx| {
                app.set_appearance(signal_app::theme::Appearance::Dark, window, cx);
                registered(app, window, cx);
                app.open_conversation(ConversationId::Direct(DEMO_ALICE), window, cx);
            },
        );
        assert!(narrow.width() < light.width());
        let _statuses = capture_sized(
            "message-statuses-dark",
            ThemeMode::Dark,
            1120.,
            |db| {
                use signal_core::MessageStatus;
                let carol = ConversationId::Direct(signal_backend::demo::DEMO_CAROL);
                let now = now_millis();
                for (i, (status, body)) in [
                    (MessageStatus::Sending, "Sending…"),
                    (MessageStatus::Sent, "Sent"),
                    (MessageStatus::Delivered, "Delivered"),
                    (MessageStatus::Read, "Read"),
                    (MessageStatus::Failed, "Failed"),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut m = NewMessage::text(
                        carol,
                        signal_backend::demo::DEMO_ME,
                        now + i as i64 * 60_000,
                        true,
                        body,
                    );
                    m.status = status;
                    m.read = true;
                    db.apply(Mutation::InsertMessage(m)).unwrap();
                }
            },
            |app, window, cx| {
                app.set_appearance(signal_app::theme::Appearance::Dark, window, cx);
                registered(app, window, cx);
                app.open_conversation(
                    ConversationId::Direct(signal_backend::demo::DEMO_CAROL),
                    window,
                    cx,
                );
                app.composer()
                    .update(cx, |s, cx| s.set_value("On my way!", window, cx));
            },
        );
        let _send = capture("composer-send", ThemeMode::Light, |app, window, cx| {
            registered(app, window, cx);
            app.open_conversation(ConversationId::Direct(DEMO_ALICE), window, cx);
            app.composer()
                .update(cx, |s, cx| s.set_value("On my way!", window, cx));
        });
        let _composer = capture("story-composer", ThemeMode::Dark, |app, window, cx| {
            app.set_appearance(signal_app::theme::Appearance::Dark, window, cx);
            registered(app, window, cx);
            app.set_list_mode(signal_app::app::ListMode::Stories, window, cx);
            app.open_story_composer(window, cx);
        });
        let _mine = capture_sized(
            "my-stories",
            ThemeMode::Light,
            1120.,
            |db| {
                db.apply(Mutation::InsertStory(signal_core::NewStory {
                    author: signal_backend::demo::DEMO_ME,
                    sent_at: now_millis(),
                    text: Some("Off hiking!".into()),
                    background: Some(signal_backend::interpret::STORY_COLORS[3]),
                    foreground: Some(0xFFFF_FFFF),
                    attachment: None,
                    viewed: true,
                }))
                .unwrap();
            },
            |app, window, cx| {
                registered(app, window, cx);
                app.set_list_mode(signal_app::app::ListMode::Stories, window, cx);
                app.open_story_viewer(signal_backend::demo::DEMO_ME, window, cx);
            },
        );
        let _left = capture_sized(
            "left-group",
            ThemeMode::Light,
            1120.,
            |db| {
                let me = signal_backend::demo::DEMO_ME;
                db.apply(Mutation::GroupMember {
                    conversation: ConversationId::Group(DEMO_GROUP),
                    actor: me,
                    member: me,
                    change: signal_core::MemberChange::Left,
                    outgoing: true,
                    member_is_me: true,
                    at: now_millis(),
                })
                .unwrap();
            },
            |app, window, cx| {
                registered(app, window, cx);
                app.open_conversation(ConversationId::Group(DEMO_GROUP), window, cx);
            },
        );
        assert_eq!(light.dimensions(), dark.dimensions());
        assert_ne!(
            light.as_raw(),
            dark.as_raw(),
            "themes must render differently"
        );
        assert_ne!(link.as_raw(), light.as_raw());
        // dark mode: the top-left pixel (sidebar background) is dark
        let px = dark.get_pixel(5, 5).0;
        assert!(
            px[0] < 80 && px[1] < 80 && px[2] < 80,
            "expected a dark sidebar, got {px:?}"
        );
        let px = light.get_pixel(5, 5).0;
        assert!(px[0] > 200, "expected a light sidebar, got {px:?}");
        println!("rendering: ok");
    }
}
