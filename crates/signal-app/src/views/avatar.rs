//! Signal-style circular avatar with initials on a stable colour.

use gpui_kit::component::Icon;
use gpui_kit::{
    App, IntoElement, ParentElement as _, Rems, RenderOnce, SharedString, Styled as _,
    StyledImage as _, Window, div,
};
use signal_core::avatar;

use crate::assets::Icons;
use crate::theme::hex;
use crate::views::rem_px;

#[derive(IntoElement)]
pub struct Avatar {
    seed: Vec<u8>,
    name: SharedString,
    size: Rems,
    group: bool,
    note: bool,
    photo: Option<std::sync::Arc<gpui_kit::Image>>,
}

impl Avatar {
    pub fn new(seed: impl Into<Vec<u8>>, name: impl Into<SharedString>) -> Self {
        Self {
            seed: seed.into(),
            name: name.into(),
            size: rem_px(48.),
            group: false,
            note: false,
            photo: None,
        }
    }

    pub fn size(mut self, size: Rems) -> Self {
        self.size = size;
        self
    }

    /// Signal's "Note to Self" avatar (accent circle with a note icon).
    pub fn note_to_self(mut self, note: bool) -> Self {
        self.note = note;
        self
    }

    /// Shows the contact's or group's photo instead of initials.
    pub fn photo(mut self, photo: Option<std::sync::Arc<gpui_kit::Image>>) -> Self {
        self.photo = photo;
        self
    }

    pub fn group(mut self, group: bool) -> Self {
        self.group = group;
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // Note to Self uses a fixed blue, like Signal
        let color = if self.note {
            avatar::AVATAR_COLORS[1]
        } else {
            avatar::color_for(&self.seed)
        };
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
        if let Some(photo) = self.photo {
            return base.child(
                gpui_kit::img(photo)
                    .size_full()
                    .object_fit(gpui_kit::ObjectFit::Cover),
            );
        }
        if self.note {
            return base.child(
                Icon::new(Icons::NotebookPen)
                    .size(size * 0.5)
                    .text_color(hex(color.fg)),
            );
        }
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
