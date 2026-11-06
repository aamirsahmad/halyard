//! Composing @mentions: detecting the query being typed and turning the
//! chosen `@Name`s into Signal mention ranges when sending.

use uuid::Uuid;

use crate::model::{BodyRange, MENTION_PLACEHOLDER, RangeKind};

/// The partial name after a trailing `@`, if the text ends while typing a
/// mention ("hi @al" → `Some("al")`). The `@` must start a word.
pub fn query(text: &str) -> Option<&str> {
    let at = text.rfind('@')?;
    let before_ok = at == 0
        || text[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
    let partial = &text[at + 1..];
    (before_ok && !partial.contains(char::is_whitespace) && partial.chars().count() <= 30)
        .then_some(partial)
}

/// Replaces the trailing `@partial` with `@Name ` (what the composer shows).
pub fn complete(text: &str, name: &str) -> String {
    match text.rfind('@') {
        Some(at) => format!("{}@{} ", &text[..at], name),
        None => format!("{text}@{name} "),
    }
}

/// Converts the `@Name` occurrences of chosen mentions into placeholders
/// with mention ranges, as Signal sends them.
pub fn encode(body: &str, mentions: &[(String, Uuid)]) -> (String, Vec<BodyRange>) {
    // longest names first so "@Ann Lee" wins over "@Ann"
    let mut chosen: Vec<&(String, Uuid)> = mentions.iter().collect();
    chosen.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    let mut out = String::with_capacity(body.len());
    let mut ranges = Vec::new();
    let mut utf16 = 0u32;
    let mut rest = body;
    'outer: while !rest.is_empty() {
        if rest.starts_with('@') {
            for (name, uuid) in &chosen {
                let token = format!("@{name}");
                if rest.starts_with(&token) {
                    ranges.push(BodyRange {
                        start: utf16,
                        length: 1,
                        kind: RangeKind::Mention(*uuid),
                    });
                    out.push(MENTION_PLACEHOLDER);
                    utf16 += 1;
                    rest = &rest[token.len()..];
                    continue 'outer;
                }
            }
        }
        let ch = rest.chars().next().expect("non-empty");
        out.push(ch);
        utf16 += ch.len_utf16() as u32;
        rest = &rest[ch.len_utf8()..];
    }
    (out, ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANN: Uuid = Uuid::from_u128(1);
    const ANN_LEE: Uuid = Uuid::from_u128(2);

    #[test]
    fn detects_query() {
        assert_eq!(query("hi @al"), Some("al"));
        assert_eq!(query("@"), Some(""));
        assert_eq!(query("mail me@home"), None, "not at a word start");
        assert_eq!(query("hi @al there"), None, "finished");
        assert_eq!(query("no mention"), None);
    }

    #[test]
    fn completes() {
        assert_eq!(complete("hi @al", "Alice Johnson"), "hi @Alice Johnson ");
        assert_eq!(complete("@", "Bob"), "@Bob ");
    }

    #[test]
    fn encodes_mentions_with_utf16_offsets() {
        let (body, ranges) = encode(
            "😀 @Ann Lee and @Ann!",
            &[("Ann".into(), ANN), ("Ann Lee".into(), ANN_LEE)],
        );
        assert_eq!(body, "😀 \u{FFFC} and \u{FFFC}!");
        assert_eq!(
            ranges,
            vec![
                BodyRange {
                    start: 3,
                    length: 1,
                    kind: RangeKind::Mention(ANN_LEE)
                },
                BodyRange {
                    start: 9,
                    length: 1,
                    kind: RangeKind::Mention(ANN)
                },
            ]
        );
        // round-trips through the renderer's layout
        let rich = crate::richtext::layout(&body, &ranges, |u| {
            if *u == ANN {
                "Ann".into()
            } else {
                "Ann Lee".into()
            }
        });
        assert_eq!(rich.text, "😀 @Ann Lee and @Ann!");
    }

    #[test]
    fn untouched_without_mentions() {
        let (body, ranges) = encode("email @someone", &[]);
        assert_eq!(body, "email @someone");
        assert!(ranges.is_empty());
    }
}
