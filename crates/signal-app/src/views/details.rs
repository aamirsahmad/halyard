//! Conversation details: group members or contact info.

use std::collections::HashMap;

use gpui_kit::component::dialog::Dialog;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    ElementId, FontWeight, InteractiveElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
};
use signal_core::ConversationId;
use uuid::Uuid;

use crate::theme::Palette;
use crate::views::avatar::Avatar;

pub fn details_dialog(
    dialog: Dialog,
    title: &str,
    id: ConversationId,
    members: &[(Uuid, String)],
    phones: &HashMap<Uuid, String>,
    me: Option<Uuid>,
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
                .group(id.is_group())
                .size(px(72.)),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .when(id.is_group(), |d| {
            d.child(
                div()
                    .text_size(px(13.))
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
            .child(Avatar::new(uuid.as_bytes().to_vec(), name.clone()).size(px(36.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().child(name.clone()))
                    .when_some(phone, |d, phone| {
                        d.child(
                            div()
                                .text_size(px(12.))
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
        .w(px(380.))
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
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .test_support(),
        )
}
