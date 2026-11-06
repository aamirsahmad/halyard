//! Conversation details: group members or contact info.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::dialog::Dialog;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    ElementId, FontWeight, Image, InteractiveElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use signal_core::ConversationId;
use uuid::Uuid;

use crate::theme::Palette;
use crate::views::avatar::Avatar;
use crate::views::rem_px;

/// Photos and about text shown in the details panel.
#[derive(Clone, Default)]
pub struct DetailsExtras {
    pub photo: Option<Arc<Image>>,
    pub about: Option<String>,
    pub member_photos: HashMap<Uuid, Arc<Image>>,
}

#[allow(clippy::too_many_arguments)]
pub fn details_dialog(
    dialog: Dialog,
    title: &str,
    id: ConversationId,
    members: &[(Uuid, String)],
    phones: &HashMap<Uuid, String>,
    me: Option<Uuid>,
    extras: &DetailsExtras,
    p: &Palette,
) -> Dialog {
    let header = div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .pb_3()
        .child(
            Avatar::new(id.color_seed(), title.to_string())
                .photo(extras.photo.clone())
                .note_to_self(me.is_some_and(|me| id == ConversationId::Direct(me)))
                .group(id.is_group())
                .size(rem_px(72.)),
        )
        .child(
            div()
                .text_size(rem_px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .when_some(extras.about.clone(), |d, about| {
            d.child(
                div()
                    .id("details-about")
                    .text_size(rem_px(13.))
                    .text_center()
                    .child(about)
                    .test_support(),
            )
        })
        .when(id.is_group(), |d| {
            d.child(
                div()
                    .text_size(rem_px(13.))
                    .text_color(p.text_secondary)
                    .child(format!("Group · {} members", members.len())),
            )
        });

    let rows = members.iter().map(|(uuid, name)| {
        let phone = phones.get(uuid).cloned().filter(|_| Some(*uuid) != me);
        div()
            .id(ElementId::Name(format!("member-{uuid}").into()))
            .flex()
            .items_center()
            .gap_3()
            .py_2()
            .child(
                Avatar::new(uuid.as_bytes().to_vec(), name.clone())
                    .photo(extras.member_photos.get(uuid).cloned())
                    .size(rem_px(36.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().child(name.clone()))
                    .when_some(phone, |d, phone| {
                        d.child(
                            div()
                                .text_size(rem_px(12.))
                                .text_color(p.text_secondary)
                                .child(phone),
                        )
                    }),
            )
            .test_support()
    });

    dialog
        .title(if id.is_group() {
            "Group members"
        } else {
            "Chat details"
        })
        .w(gpui_kit::px(380.))
        .child(
            div()
                .id("conversation-details-panel")
                .flex()
                .flex_col()
                .child(header)
                .when(id.is_group() && members.is_empty(), |d| {
                    d.child(
                        div()
                            .text_color(p.text_secondary)
                            .child("Member list not available yet. It syncs from your phone."),
                    )
                })
                .child(
                    div()
                        .id("member-list")
                        .flex()
                        .flex_col()
                        .max_h(rem_px(360.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .test_support(),
        )
}
