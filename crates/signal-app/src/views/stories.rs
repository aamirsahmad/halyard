//! Stories: list of contacts with recent stories, and the story viewer.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    TestSupportExt as _, Window, div, img, rgba,
};
use signal_core::format::conversation_list_time;
use signal_core::{Mutation, Story, now_millis};
use uuid::Uuid;

use crate::app::SignalApp;
use crate::theme::Palette;
use crate::views::avatar::Avatar;
use crate::views::rem_px;
use signal_backend::interpret::STORY_COLORS;

/// Stories grouped by author, newest author first; each group oldest first.
pub fn group_by_author(stories: &[Story]) -> Vec<(Uuid, Vec<Story>)> {
    let mut groups: Vec<(Uuid, Vec<Story>)> = Vec::new();
    for s in stories {
        match groups.iter_mut().find(|(a, _)| *a == s.author) {
            Some((_, v)) => v.push(s.clone()),
            None => groups.push((s.author, vec![s.clone()])),
        }
    }
    for (_, v) in &mut groups {
        v.sort_by_key(|s| s.sent_at);
    }
    // authors with unviewed stories first, then most recent
    groups.sort_by_key(|(_, v)| {
        (
            v.iter().all(|s| s.viewed),
            std::cmp::Reverse(v.iter().map(|s| s.sent_at).max().unwrap_or(0)),
        )
    });
    groups
}

fn argb(c: u32) -> Hsla {
    // Signal colours are 0xAARRGGBB; gpui's rgba takes 0xRRGGBBAA
    rgba(c.rotate_left(8)).into()
}

impl SignalApp {
    pub(crate) fn reload_stories(&mut self) {
        self.stories = self.db.stories().unwrap_or_default();
    }

    pub fn stories(&self) -> &[Story] {
        &self.stories
    }

    pub fn unviewed_story_count(&self) -> usize {
        self.stories.iter().filter(|s| !s.viewed).count()
    }

    pub(super) fn render_story_list(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let me = self.me();
        let groups: Vec<(Uuid, Vec<Story>)> = group_by_author(&self.stories)
            .into_iter()
            .filter(|(a, _)| Some(*a) != me)
            .collect();
        let now = now_millis();
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(me) = me {
            rows.push(self.render_my_stories_row(me, now, p, cx));
        }
        if groups.is_empty() {
            rows.push(
                div()
                    .id("no-stories")
                    .p_6()
                    .text_center()
                    .text_color(p.text_secondary)
                    .child(
                        "No recent stories. Stories from your contacts appear here for 24 hours.",
                    )
                    .test_support()
                    .into_any_element(),
            );
        }
        for (author, stories) in groups {
            let name = self.display_name(&author);
            let unviewed = stories.iter().any(|s| !s.viewed);
            let latest = stories.iter().map(|s| s.sent_at).max().unwrap_or(0);
            let count = stories.len();
            rows.push(
                div()
                    .id(ElementId::Name(format!("story-author-{author}").into()))
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(rem_px(64.))
                    .px_3()
                    .rounded(rem_px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_story_viewer(author, window, cx)
                    }))
                    .child(
                        div()
                            .p(rem_px(2.))
                            .rounded_full()
                            .border_2()
                            .border_color(if unviewed { p.accent } else { p.border })
                            .child(
                                Avatar::new(author.as_bytes().to_vec(), name.clone())
                                    .photo(
                                        self.photo_for(&signal_core::ConversationId::Direct(
                                            author,
                                        )),
                                    )
                                    .size(rem_px(40.)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(if unviewed {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::SEMIBOLD
                                    })
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(rem_px(13.))
                                    .text_color(p.text_secondary)
                                    .child(format!(
                                        "{} · {}",
                                        if count == 1 {
                                            "1 story".to_string()
                                        } else {
                                            format!("{count} stories")
                                        },
                                        conversation_list_time(latest, now)
                                    )),
                            ),
                    )
                    .test_support()
                    .into_any_element(),
            );
        }
        div()
            .id("story-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .children(rows)
            .test_support()
            .into_any_element()
    }

    fn render_my_stories_row(
        &self,
        me: Uuid,
        now: i64,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mine: Vec<&Story> = self.stories.iter().filter(|s| s.author == me).collect();
        let latest = mine.iter().map(|s| s.sent_at).max();
        let subtitle = match (mine.len(), latest) {
            (0, _) | (_, None) => "Add a story".to_string(),
            (1, Some(t)) => format!("1 story · {}", conversation_list_time(t, now)),
            (n, Some(t)) => format!("{n} stories · {}", conversation_list_time(t, now)),
        };
        let has_stories = !mine.is_empty();
        div()
            .id("my-stories")
            .flex()
            .items_center()
            .gap_3()
            .h(rem_px(64.))
            .px_3()
            .mb_1()
            .rounded(rem_px(10.))
            .cursor_pointer()
            .hover(|s| s.bg(p.hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                if has_stories {
                    this.open_story_viewer(me, window, cx)
                } else {
                    this.open_story_composer(window, cx)
                }
            }))
            .child(
                Avatar::new(me.as_bytes().to_vec(), "You")
                    .photo(self.photo_for(&signal_core::ConversationId::Direct(me)))
                    .size(rem_px(44.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("My Stories"))
                    .child(
                        div()
                            .text_size(rem_px(13.))
                            .text_color(p.text_secondary)
                            .child(subtitle),
                    ),
            )
            .child(
                Button::new("add-story")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("Add a story")
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_story_composer(window, cx)
                    })),
            )
            .test_support()
            .into_any_element()
    }

    /// The "new story" dialog: a text story on a chosen background colour,
    /// or a photo/video picked from disk.
    pub fn open_story_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.story_text.clone();
        input.update(cx, |s, cx| s.set_value("", window, cx));
        self.story_background = STORY_COLORS[0];
        // after the dialog has taken focus for itself
        let focus = gpui_kit::Focusable::focus_handle(input.read(cx), cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        let app = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let selected = app
                .upgrade()
                .map(|a| a.read(cx).story_background)
                .unwrap_or(STORY_COLORS[0]);
            let swatches = STORY_COLORS.iter().enumerate().map(|(i, &color)| {
                let app = app.clone();
                div()
                    .id(ElementId::Name(format!("story-color-{i}").into()))
                    .size(rem_px(28.))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(argb(color))
                    .border_2()
                    .border_color(if color == selected {
                        gpui_kit::white()
                    } else {
                        argb(color)
                    })
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.story_background = color;
                            cx.notify();
                        });
                        window.refresh();
                    })
                    .test_support()
            });
            let post = {
                let app = app.clone();
                move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                    let _ = app.update(cx, |this, cx| this.post_text_story(window, cx));
                }
            };
            let media = {
                let app = app.clone();
                move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                    let _ = app.update(cx, |this, cx| this.pick_story_media(window, cx));
                }
            };
            dialog.title("New story").w(gpui_kit::px(420.)).child(
                div()
                    .id("story-composer")
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(rem_px(280.))
                            .p_6()
                            .rounded(rem_px(16.))
                            .bg(argb(selected))
                            .child(
                                gpui_kit::component::input::Input::new(&input)
                                    .id("story-text-input")
                                    .appearance(false)
                                    .text_color(gpui_kit::white())
                                    .text_size(rem_px(22.)),
                            ),
                    )
                    .child(div().flex().gap_2().justify_center().children(swatches))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("story-add-media")
                                    .outline()
                                    .label("Photo or video…")
                                    .on_click(media),
                            )
                            .child(
                                Button::new("story-post")
                                    .primary()
                                    .label("Post to My Story")
                                    .on_click(post),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rem_px(12.))
                            .text_color(gpui_kit::hsla(0., 0., 0.5, 1.))
                            .child("Visible for 24 hours to everyone you chat with."),
                    )
                    .test_support(),
            )
        });
    }

    pub fn story_text_value(&self, cx: &gpui_kit::App) -> String {
        self.story_text.read(cx).value().to_string()
    }

    pub fn post_text_story(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.story_text.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.commands.send(signal_core::BackendCommand::PostStory {
            text: Some(text),
            background: Some(self.story_background),
            attachment: None,
        });
        window.close_dialog(cx);
        cx.notify();
    }

    /// Picks a photo or video and posts it as a story.
    pub fn pick_story_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Post".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.post_story_media(path, window, cx)
                });
            }
        })
        .detach();
    }

    pub fn post_story_media(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commands.send(signal_core::BackendCommand::PostStory {
            text: None,
            background: None,
            attachment: Some(path),
        });
        window.close_dialog(cx);
        cx.notify();
    }

    /// Opens an author's stories, starting at the first unviewed one.
    pub fn open_story_viewer(&mut self, author: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let stories: Vec<Story> = group_by_author(&self.stories)
            .into_iter()
            .find(|(a, _)| *a == author)
            .map(|(_, v)| v)
            .unwrap_or_default();
        if stories.is_empty() {
            return;
        }
        let start = stories.iter().position(|s| !s.viewed).unwrap_or(0);
        let index = Rc::new(Cell::new(start));
        self.mark_story_viewed(stories[start].id, cx);
        let own = self.me() == Some(author);
        let name = if own {
            "My Story".to_string()
        } else {
            self.display_name(&author)
        };
        let app = cx.entity().downgrade();
        let access = self.attachments.clone();
        let reply_input = self.story_reply.clone();
        reply_input.update(cx, |s, cx| s.set_value("", window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            let ix = index.get().min(stories.len() - 1);
            let story = &stories[ix];
            let content: AnyElement = match (&story.text, &story.attachment) {
                (_, Some(a)) => match a
                    .local_path
                    .as_ref()
                    .and_then(|p| access.image(p, &a.content_type))
                {
                    Some(image) => img(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => div()
                        .text_color(gpui_kit::white())
                        .child("Loading…")
                        .into_any_element(),
                },
                (Some(text), None) => div()
                    .p_6()
                    .text_center()
                    .text_size(rem_px(26.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(story.foreground.map(argb).unwrap_or(gpui_kit::white()))
                    .child(text.clone())
                    .into_any_element(),
                (None, None) => div().into_any_element(),
            };
            let nav = |dir: isize, label: &'static str, enabled: bool| {
                let index = index.clone();
                let app = app.clone();
                let ids: Vec<i64> = stories.iter().map(|s| s.id).collect();
                Button::new(ElementId::Name(format!("story-{label}").into()))
                    .ghost()
                    .small()
                    .icon(if dir < 0 {
                        IconName::ChevronLeft
                    } else {
                        IconName::ChevronRight
                    })
                    .tooltip(label)
                    .when(!enabled, |b| b.opacity(0.3))
                    .on_click(move |_, window, cx| {
                        if !enabled {
                            return;
                        }
                        let next =
                            (index.get() as isize + dir).clamp(0, ids.len() as isize - 1) as usize;
                        index.set(next);
                        let _ = app.update(cx, |this, cx| this.mark_story_viewed(ids[next], cx));
                        window.refresh();
                    })
            };
            let bg = story
                .background
                .map(argb)
                .unwrap_or(rgba(0x1B1B1BFF).into());
            dialog
                .title(format!(
                    "{name} · {}",
                    conversation_list_time(story.sent_at, now_millis())
                ))
                .w(gpui_kit::px(440.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(nav(-1, "Previous", ix > 0))
                        .child(
                            div()
                                .id("story-viewer")
                                .flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .h(rem_px(520.))
                                .rounded(rem_px(16.))
                                .overflow_hidden()
                                .bg(bg)
                                .child(content)
                                .test_support(),
                        )
                        .child(nav(1, "Next", ix + 1 < stories.len())),
                )
                .child(
                    div()
                        .id("story-progress")
                        .flex()
                        .gap_1()
                        .pt_2()
                        .children((0..stories.len()).map(|i| {
                            div().flex_1().h(rem_px(3.)).rounded_full().bg(if i <= ix {
                                rgba(0x2C6BEDFF)
                            } else {
                                rgba(0x8888884D)
                            })
                        }))
                        .test_support(),
                )
                .children((!own).then(|| {
                    let sent_at = story.sent_at;
                    let send = {
                        let app = app.clone();
                        let input = reply_input.clone();
                        move |body: Option<String>, window: &mut Window, cx: &mut gpui_kit::App| {
                            let body =
                                body.unwrap_or_else(|| input.read(cx).value().trim().to_string());
                            if body.is_empty() {
                                return;
                            }
                            input.update(cx, |s, cx| s.set_value("", window, cx));
                            let _ = app.update(cx, |this, cx| {
                                this.reply_to_story(author, sent_at, body, window, cx)
                            });
                        }
                    };
                    let send = Rc::new(send);
                    div()
                        .id("story-reply")
                        .flex()
                        .items_center()
                        .gap_2()
                        .pt_3()
                        .children(["❤️", "😂", "😮"].map(|emoji| {
                            let send = send.clone();
                            div()
                                .id(ElementId::Name(format!("story-react-{emoji}").into()))
                                .text_size(rem_px(20.))
                                .cursor_pointer()
                                .child(emoji)
                                .on_click(move |_, window, cx| {
                                    send(Some(emoji.to_string()), window, cx)
                                })
                                .test_support()
                        }))
                        .child(
                            div().flex_1().child(
                                gpui_kit::component::input::Input::new(&reply_input)
                                    .id("story-reply-input")
                                    .small(),
                            ),
                        )
                        .child({
                            let send = send.clone();
                            Button::new("story-reply-send")
                                .primary()
                                .small()
                                .label("Reply")
                                .on_click(move |_, window, cx| send(None, window, cx))
                        })
                        .test_support()
                }))
        });
    }

    pub fn reply_to_story(
        &mut self,
        author: Uuid,
        story_sent_at: i64,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commands
            .send(signal_core::BackendCommand::ReplyToStory {
                author,
                story_sent_at,
                body,
            });
        window.push_notification(
            gpui_kit::component::notification::Notification::success(format!(
                "Reply sent to {}",
                self.display_name(&author)
            )),
            cx,
        );
    }

    pub fn mark_story_viewed(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.stories.iter().any(|s| s.id == id && !s.viewed) {
            self.apply_local(Mutation::MarkStoryViewed { id }, cx);
            self.reload_stories();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: i64, author: u128, at: i64, viewed: bool) -> Story {
        Story {
            id,
            author: Uuid::from_u128(author),
            sent_at: at,
            text: None,
            background: None,
            foreground: None,
            attachment: None,
            viewed,
        }
    }

    #[test]
    fn grouping_puts_unviewed_first_and_orders_stories() {
        let stories = vec![
            s(1, 1, 100, true),
            s(2, 2, 50, false),
            s(3, 1, 200, true),
            s(4, 2, 40, false),
        ];
        let g = group_by_author(&stories);
        assert_eq!(g[0].0, Uuid::from_u128(2), "unviewed author first");
        assert_eq!(g[0].1.iter().map(|s| s.id).collect::<Vec<_>>(), vec![4, 2]);
        assert_eq!(g[1].1.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn argb_conversion() {
        let c = argb(0xFF2C6BED);
        let expected: Hsla = rgba(0x2C6BEDFF).into();
        assert_eq!(c, expected);
    }
}
