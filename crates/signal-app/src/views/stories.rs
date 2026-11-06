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
        let groups = group_by_author(&self.stories);
        let now = now_millis();
        let mut rows: Vec<AnyElement> = Vec::new();
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
        let name = self.display_name(&author);
        let app = cx.entity().downgrade();
        let access = self.attachments.clone();
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
        });
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
