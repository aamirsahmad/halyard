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
    use signal_core::{AccountState, BackendEvent, ConversationId, Database, now_millis};

    fn context() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(gpui_kit::init);
        cx
    }

    fn capture(
        name: &str,
        mode: ThemeMode,
        setup: impl FnOnce(&mut SignalApp, &mut gpui_kit::Window, &mut gpui_kit::Context<SignalApp>) + 'static,
    ) -> image::RgbaImage {
        let mut cx = context();
        cx.update(|cx| Theme::change(mode, None, cx));
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("m.db"), None).unwrap();
        seed(&mut db, now_millis()).unwrap();
        let sink = RecordingSink::default();
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                            origin: Default::default(),
                            size: size(px(1120.), px(760.)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    move |window, cx| {
                        cx.new(|cx| {
                            let mut app = SignalApp::new(db, Rc::new(sink.clone()), Rc::new(sink), window, cx);
                            setup(&mut app, window, cx);
                            app
                        })
                    },
                )
            })
            .unwrap();
        for _ in 0..3 {
            cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx)).unwrap();
            cx.run_until_parked();
        }
        let image = cx.capture_screenshot(handle.into()).expect("offscreen renderer");
        if let Ok(out) = std::env::var("SCREENSHOT_DIR") {
            std::fs::create_dir_all(&out).unwrap();
            let path = std::path::Path::new(&out).join(format!("{name}.png"));
            image.save(&path).unwrap();
            println!("saved {}", path.display());
        }
        image
    }

    fn registered(app: &mut SignalApp, window: &mut gpui_kit::Window, cx: &mut gpui_kit::Context<SignalApp>) {
        app.handle_event(BackendEvent::Account(AccountState::Registered(demo_account())), window, cx);
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
        assert_eq!(light.dimensions(), dark.dimensions());
        assert_ne!(light.as_raw(), dark.as_raw(), "themes must render differently");
        assert_ne!(link.as_raw(), light.as_raw());
        // dark mode: the top-left pixel (sidebar background) is dark
        let px = dark.get_pixel(5, 5).0;
        assert!(px[0] < 80 && px[1] < 80 && px[2] < 80, "expected a dark sidebar, got {px:?}");
        let px = light.get_pixel(5, 5).0;
        assert!(px[0] > 200, "expected a light sidebar, got {px:?}");
        println!("rendering: ok");
    }
}
