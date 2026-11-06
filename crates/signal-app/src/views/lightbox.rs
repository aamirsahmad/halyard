//! Full-size image viewer with Open and Save As.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::{
    Context, InteractiveElement as _, ObjectFit, ParentElement as _, Styled as _, StyledImage as _,
    TestSupportExt as _, Window, div, img,
};

use crate::app::SignalApp;

impl SignalApp {
    pub fn open_lightbox(
        &mut self,
        path: String,
        content_type: String,
        file_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(image) = self.attachments.image(&path, &content_type) else {
            window.push_notification(Notification::error("Couldn't open this image"), cx);
            return;
        };
        let viewport = window.viewport_size();
        let (w, h) = (viewport.width * 0.8, viewport.height * 0.7);
        let access = self.attachments.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let (open_path, open_name) = (path.clone(), file_name.clone());
            let (save_path, save_name) = (path.clone(), file_name.clone());
            let open_access = access.clone();
            let save_access = access.clone();
            dialog
                .title(file_name.clone())
                .w(w)
                .child(
                    div()
                        .id("lightbox")
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(h)
                        .child(img(image.clone()).size_full().object_fit(ObjectFit::Contain))
                        .test_support(),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(Button::new("lightbox-open").outline().label("Open").on_click(move |_, _, cx| {
                            match open_access.path_for_open(&open_path, &open_name) {
                                Ok(p) => cx.open_with_system(&p),
                                Err(e) => tracing::error!(%e, "failed to open image"),
                            }
                        }))
                        .child(Button::new("lightbox-save").primary().label("Save As…").on_click(move |_, window, cx| {
                            let dir = dirs_home();
                            let rx = cx.prompt_for_new_path(&dir, Some(&save_name));
                            let access = save_access.clone();
                            let source = save_path.clone();
                            window
                                .spawn(cx, async move |_| {
                                    if let Ok(Ok(Some(dest))) = rx.await {
                                        match access.export_to(&source, &dest) {
                                            Ok(()) => tracing::info!(dest = %dest.display(), "saved attachment"),
                                            Err(e) => tracing::error!(%e, "failed to save attachment"),
                                        }
                                    }
                                })
                                .detach();
                        })),
                )
        });
    }
}

impl SignalApp {
    /// Opens view-once media a single time, then forgets it.
    pub fn open_view_once(
        &mut self,
        message_id: i64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(a) = self
            .state
            .message(message_id)
            .and_then(|m| m.attachments.get(index))
            .cloned()
        else {
            return;
        };
        let Some(path) = a.local_path.clone().filter(|_| a.view_once && !a.viewed) else {
            return;
        };
        if a.is_image() {
            match self.attachments.image(&path, &a.content_type) {
                Some(image) => {
                    let viewport = window.viewport_size();
                    let (w, h) = (viewport.width * 0.8, viewport.height * 0.7);
                    window.open_dialog(cx, move |dialog, _, _| {
                        dialog.title("View-once photo").w(w).child(
                            div()
                                .id("view-once-viewer")
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(h)
                                .child(
                                    img(image.clone())
                                        .size_full()
                                        .object_fit(ObjectFit::Contain),
                                )
                                .test_support(),
                        )
                    });
                }
                None => {
                    window.push_notification(Notification::error("Couldn't open this photo"), cx)
                }
            }
        } else if let Ok(p) = self.attachments.path_for_open(&path, &a.display_name()) {
            cx.open_with_system(&p);
        }
        self.attachments.forget_image(&path);
        self.apply_local(signal_core::Mutation::ViewOnceOpened { message_id }, cx);
    }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(Into::into)
        .unwrap_or_else(|| ".".into())
}
