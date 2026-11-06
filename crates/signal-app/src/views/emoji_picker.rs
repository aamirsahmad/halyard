//! Searchable emoji picker, used by the composer and for reactions.

use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
};

use crate::emoji::{CATEGORIES, search};
use crate::theme::Palette;
use crate::views::rem_px;

pub type OnPick = Rc<dyn Fn(&str, &mut Window, &mut App)>;

pub struct EmojiPicker {
    id: SharedString,
    search: Entity<InputState>,
    query: String,
    on_pick: OnPick,
    _subscription: Subscription,
}

impl EmojiPicker {
    pub fn new(
        id: impl Into<SharedString>,
        on_pick: OnPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search emoji"));
        let sub = cx.subscribe(&search, |this, state, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        Self {
            id: id.into(),
            search,
            query: String::new(),
            on_pick,
            _subscription: sub,
        }
    }

    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.query.clear();
        cx.notify();
    }

    fn cell(&self, emoji: &'static str, p: &Palette) -> AnyElement {
        let on_pick = self.on_pick.clone();
        div()
            .id(ElementId::Name(format!("{}-{emoji}", self.id).into()))
            .size(rem_px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rem_px(8.))
            .text_size(rem_px(22.))
            .cursor_pointer()
            .hover(|s| s.bg(p.hover))
            .on_click(move |_, window, cx| on_pick(emoji, window, cx))
            .child(emoji)
            .test_support()
            .into_any_element()
    }
}

impl Focusable for EmojiPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

impl Render for EmojiPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        let mut body: Vec<AnyElement> = Vec::new();
        if self.query.trim().is_empty() {
            for cat in CATEGORIES {
                body.push(
                    div()
                        .pt_2()
                        .pb_1()
                        .text_size(rem_px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text_secondary)
                        .child(cat.name)
                        .into_any_element(),
                );
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .children(cat.emoji.iter().map(|(e, _)| self.cell(e, &p)))
                        .into_any_element(),
                );
            }
        } else {
            let found = search(&self.query);
            if found.is_empty() {
                body.push(
                    div()
                        .p_4()
                        .text_color(p.text_secondary)
                        .child("No emoji found")
                        .into_any_element(),
                );
            } else {
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .children(found.into_iter().map(|e| self.cell(e, &p)))
                        .into_any_element(),
                );
            }
        }
        div()
            .id(ElementId::Name(self.id.clone()))
            .flex()
            .flex_col()
            .w(rem_px(340.))
            .h(rem_px(340.))
            .gap_2()
            .child(
                Input::new(&self.search).small().prefix(
                    Icon::new(IconName::Search)
                        .small()
                        .text_color(p.text_secondary),
                ),
            )
            .child(
                div()
                    .id(ElementId::Name(format!("{}-grid", self.id).into()))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(body),
            )
            .test_support()
    }
}
