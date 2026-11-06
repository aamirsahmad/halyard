//! Signal-style circular avatar with initials on a stable colour.

use gpui_kit::component::Icon;
use gpui_kit::{
    App, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString, Styled as _, Window,
    div, px,
};
use signal_core::avatar;

use crate::assets::Icons;
use crate::theme::hex;

#[derive(IntoElement)]
pub struct Avatar {
    seed: Vec<u8>,
    name: SharedString,
    size: Pixels,
    group: bool,
}

impl Avatar {
    pub fn new(seed: impl Into<Vec<u8>>, name: impl Into<SharedString>) -> Self {
        Self {
            seed: seed.into(),
            name: name.into(),
            size: px(48.),
            group: false,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn group(mut self, group: bool) -> Self {
        self.group = group;
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let color = avatar::color_for(&self.seed);
        let size = self.size;
        let base = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(size)
            .rounded_full()
            .bg(hex(color.bg))
            .text_color(hex(color.fg))
            .overflow_hidden();
        match (self.group, avatar::initials(&self.name)) {
            (false, Some(initials)) => base
                .text_size(size * 0.4)
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .child(initials),
            (group, _) => {
                let icon = if group { Icons::Users } else { Icons::User };
                base.child(Icon::new(icon).size(size * 0.5).text_color(hex(color.fg)))
            }
        }
    }
}
