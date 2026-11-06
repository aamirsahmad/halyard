//! Turns a list of messages into renderable timeline rows: date separators,
//! sender grouping (collapsed avatars/names/timestamps) like Signal Desktop.

use chrono::NaiveDate;

use crate::model::{Message, Millis};

/// Consecutive messages from the same author within this window are grouped.
pub const GROUPING_WINDOW_MS: Millis = 3 * 60 * 1000;

#[derive(Clone, Debug, PartialEq)]
pub enum TimelineItem {
    DateSeparator(String),
    Message(TimelineMessage),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineMessage {
    /// Index into the source message slice.
    pub index: usize,
    /// First bubble of a sender group (author name shown in groups).
    pub first_in_group: bool,
    /// Last bubble of a sender group (avatar + timestamp shown).
    pub last_in_group: bool,
}

/// Builds the timeline for `messages` (sorted ascending by time).
/// `day_of` maps a timestamp to a local calendar day; `today` is used for labels.
pub fn build_timeline(
    messages: &[Message],
    today: NaiveDate,
    day_of: impl Fn(Millis) -> NaiveDate,
) -> Vec<TimelineItem> {
    let mut items = Vec::with_capacity(messages.len() + 4);
    let mut last_day: Option<NaiveDate> = None;
    for (i, m) in messages.iter().enumerate() {
        let day = day_of(m.sent_at);
        if last_day != Some(day) {
            items.push(TimelineItem::DateSeparator(crate::format::date_separator(
                day, today,
            )));
            last_day = Some(day);
        }
        let prev = i.checked_sub(1).map(|p| &messages[p]);
        let next = messages.get(i + 1);
        let groups_with = |a: &Message, b: &Message| {
            a.author == b.author
                && a.outgoing == b.outgoing
                && (b.sent_at - a.sent_at).abs() <= GROUPING_WINDOW_MS
                && day_of(a.sent_at) == day_of(b.sent_at)
        };
        items.push(TimelineItem::Message(TimelineMessage {
            index: i,
            first_in_group: !prev.is_some_and(|p| groups_with(p, m)),
            last_in_group: !next.is_some_and(|n| groups_with(m, n)),
        }));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ConversationId;
    use crate::model::MessageStatus;
    use uuid::Uuid;

    fn m(author: u128, sent_at: Millis) -> Message {
        Message {
            id: sent_at,
            conversation: ConversationId::Direct(Uuid::from_u128(9)),
            author: Uuid::from_u128(author),
            sent_at,
            received_at: sent_at,
            outgoing: author == 0,
            body: "x".into(),
            attachments: vec![],
            quote: None,
            reactions: vec![],
            status: MessageStatus::Received,
            read: true,
            edited_at: None,
            deleted: false,
            expires_in_secs: None,
        }
    }

    const DAY: Millis = 86_400_000;

    fn day_of(ts: Millis) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new((ts / DAY) as u64)
    }

    fn flags(items: &[TimelineItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                TimelineItem::DateSeparator(s) => format!("[{s}]"),
                TimelineItem::Message(t) => format!(
                    "{}{}{}",
                    t.index,
                    if t.first_in_group { "F" } else { "" },
                    if t.last_in_group { "L" } else { "" }
                ),
            })
            .collect()
    }

    #[test]
    fn empty() {
        assert!(build_timeline(&[], day_of(0), day_of).is_empty());
    }

    #[test]
    fn groups_by_author_and_window() {
        let msgs = vec![
            m(1, 1000),
            m(1, 2000),
            m(1, 3000 + GROUPING_WINDOW_MS + 1), // too far apart
            m(0, 3000 + GROUPING_WINDOW_MS + 2), // different author
            m(0, 3000 + GROUPING_WINDOW_MS + 3),
        ];
        let today = day_of(0);
        let items = build_timeline(&msgs, today, day_of);
        assert_eq!(
            flags(&items),
            vec!["[Today]", "0F", "1L", "2FL", "3F", "4L"]
        );
    }

    #[test]
    fn day_change_breaks_groups_and_adds_separator() {
        let msgs = vec![m(1, DAY - 1000), m(1, DAY + 1000)];
        let items = build_timeline(&msgs, day_of(DAY), day_of);
        assert_eq!(flags(&items), vec!["[Yesterday]", "0FL", "[Today]", "1FL"]);
    }
}
