//! Renders message bodies with Signal formatting, mentions and links.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText,
    IntoElement, ParentElement as _, StrikethroughStyle, Styled as _, StyledText, UnderlineStyle,
    div, hsla,
};
use signal_core::BodyRange;

use crate::views::rem_px;
use signal_core::richtext::{SpanStyle, layout};
use uuid::Uuid;

fn highlight(s: &SpanStyle, fg: Hsla, outgoing: bool) -> HighlightStyle {
    let mut h = HighlightStyle::default();
    if s.bold || s.mention {
        h.font_weight = Some(FontWeight::BOLD);
    }
    if s.italic {
        h.font_style = Some(FontStyle::Italic);
    }
    if s.strikethrough {
        h.strikethrough = Some(StrikethroughStyle {
            thickness: gpui_kit::px(1.),
            color: None,
        });
    }
    if s.monospace {
        h.background_color = Some(if outgoing {
            hsla(0., 0., 1., 0.18)
        } else {
            hsla(0., 0., 0., 0.08)
        });
    }
    if s.mention {
        h.background_color = Some(if outgoing {
            hsla(0., 0., 1., 0.2)
        } else {
            hsla(0., 0., 0., 0.06)
        });
    }
    if s.link {
        h.underline = Some(UnderlineStyle {
            thickness: gpui_kit::px(1.),
            color: None,
            wavy: false,
        });
    }
    if s.spoiler {
        // hidden like Signal's spoilers
        h.color = Some(fg);
        h.background_color = Some(fg);
    }
    h
}

pub fn render_body(
    message_id: i64,
    body: &str,
    ranges: &[BodyRange],
    names: &HashMap<Uuid, String>,
    me: Option<Uuid>,
    fg: Hsla,
    outgoing: bool,
) -> AnyElement {
    let rich = layout(body, ranges, |u| {
        if Some(*u) == me {
            "You".into()
        } else {
            names.get(u).cloned().unwrap_or_else(|| "Unknown".into())
        }
    });
    let base = div().text_size(rem_px(14.)).line_height(rem_px(20.));
    if rich.is_plain() {
        return base.child(rich.text).into_any_element();
    }
    let highlights: Vec<_> = rich
        .spans
        .iter()
        .map(|(r, s)| (r.clone(), highlight(s, fg, outgoing)))
        .collect();
    let styled = StyledText::new(rich.text.clone()).with_highlights(highlights);
    if rich.links.is_empty() {
        return base.child(styled).into_any_element();
    }
    let urls: Rc<Vec<String>> = Rc::new(rich.links.iter().map(|(_, u)| u.clone()).collect());
    let ranges = rich.links.iter().map(|(r, _)| r.clone()).collect();
    base.child(
        InteractiveText::new(ElementId::Name(format!("body-{message_id}").into()), styled)
            .on_click(ranges, move |ix, _, cx| {
                if let Some(url) = urls.get(ix) {
                    cx.open_url(url);
                }
            }),
    )
    .into_any_element()
}
