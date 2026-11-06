//! Turns a message body with Signal body ranges into display text with flat,
//! non-overlapping style spans and clickable links.

use std::ops::Range;

use uuid::Uuid;

use crate::model::{BodyRange, MENTION_PLACEHOLDER, RangeKind};

/// Combined style of a span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub monospace: bool,
    pub spoiler: bool,
    pub mention: bool,
    pub link: bool,
}

impl SpanStyle {
    pub fn is_plain(&self) -> bool {
        *self == SpanStyle::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichText {
    pub text: String,
    /// Sorted, non-overlapping styled byte ranges of `text`.
    pub spans: Vec<(Range<usize>, SpanStyle)>,
    /// Byte ranges of links and their URLs.
    pub links: Vec<(Range<usize>, String)>,
}

impl RichText {
    pub fn is_plain(&self) -> bool {
        self.spans.is_empty() && self.links.is_empty()
    }
}

/// Byte offset of each UTF-16 code unit index (plus the end).
fn utf16_to_byte(s: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(s.len() + 1);
    for (byte, ch) in s.char_indices() {
        for _ in 0..ch.len_utf16() {
            map.push(byte);
        }
    }
    map.push(s.len());
    map
}

fn byte_range(map: &[usize], start: u32, length: u32) -> Option<Range<usize>> {
    let s = *map.get(start as usize)?;
    let e = *map.get(start.checked_add(length)? as usize)?;
    (s < e).then_some(s..e)
}

/// Finds http(s) and www. links, trimming trailing punctuation.
pub fn find_links(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let starts =
            rest.starts_with("https://") || rest.starts_with("http://") || rest.starts_with("www.");
        let at_word_start = i == 0
            || text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || "([<\"'".contains(c));
        if starts && at_word_start {
            let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let mut end = i + len;
            while end > i
                && text[..end].ends_with(['.', ',', '!', '?', ')', ']', '>', '"', '\'', ':', ';'])
            {
                end -= 1;
            }
            let raw = &text[i..end];
            if raw.len() > "https://".len().min(raw.len()) && raw.contains('.') {
                let url = if raw.starts_with("www.") {
                    format!("https://{raw}")
                } else {
                    raw.to_string()
                };
                out.push((i..end, url));
            }
            i = end.max(i + 1);
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Builds display text: mentions become `@Name`, styles are flattened.
pub fn layout(body: &str, ranges: &[BodyRange], name_of: impl Fn(&Uuid) -> String) -> RichText {
    let map = utf16_to_byte(body);
    // mention replacements, by original byte range
    let mut mentions: Vec<(Range<usize>, String)> = ranges
        .iter()
        .filter_map(|r| match &r.kind {
            RangeKind::Mention(u) => byte_range(&map, r.start, r.length)
                .filter(|br| body[br.clone()].contains(MENTION_PLACEHOLDER))
                .map(|br| (br, format!("@{}", name_of(u)))),
            _ => None,
        })
        .collect();
    mentions.sort_by_key(|(r, _)| r.start);
    mentions.dedup_by(|b, a| b.0.start < a.0.end); // drop overlaps

    // build text and an offset mapper from original → new byte positions
    let mut text = String::with_capacity(body.len() + mentions.len() * 8);
    let mut shifts: Vec<(usize, isize)> = Vec::new(); // (orig position after replacement, cumulative delta)
    let mut last = 0;
    let mut delta: isize = 0;
    let mut mention_spans = Vec::new();
    for (r, label) in &mentions {
        text.push_str(&body[last..r.start]);
        let start = text.len();
        text.push_str(label);
        mention_spans.push(start..text.len());
        delta += label.len() as isize - r.len() as isize;
        shifts.push((r.end, delta));
        last = r.end;
    }
    text.push_str(&body[last..]);
    let map_pos = |orig: usize| -> usize {
        let d = shifts
            .iter()
            .rev()
            .find(|(p, _)| *p <= orig)
            .map_or(0, |(_, d)| *d);
        (orig as isize + d) as usize
    };

    // style intervals in new coordinates
    let mut styled: Vec<(Range<usize>, SpanStyle)> = Vec::new();
    for r in ranges {
        let Some(br) = byte_range(&map, r.start, r.length) else {
            continue;
        };
        let mut st = SpanStyle::default();
        match r.kind {
            RangeKind::Bold => st.bold = true,
            RangeKind::Italic => st.italic = true,
            RangeKind::Strikethrough => st.strikethrough = true,
            RangeKind::Monospace => st.monospace = true,
            RangeKind::Spoiler => st.spoiler = true,
            RangeKind::Mention(_) => continue,
        }
        let nr = map_pos(br.start)..map_pos(br.end);
        if nr.start < nr.end && text.is_char_boundary(nr.start) && text.is_char_boundary(nr.end) {
            styled.push((nr, st));
        }
    }
    for m in &mention_spans {
        styled.push((
            m.clone(),
            SpanStyle {
                mention: true,
                ..Default::default()
            },
        ));
    }
    let links = find_links(&text);
    for (r, _) in &links {
        styled.push((
            r.clone(),
            SpanStyle {
                link: true,
                ..Default::default()
            },
        ));
    }

    // flatten overlapping intervals
    let mut points: Vec<usize> = styled.iter().flat_map(|(r, _)| [r.start, r.end]).collect();
    points.sort_unstable();
    points.dedup();
    let mut spans: Vec<(Range<usize>, SpanStyle)> = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut st = SpanStyle::default();
        for (r, s) in &styled {
            if r.start <= a && b <= r.end {
                st.bold |= s.bold;
                st.italic |= s.italic;
                st.strikethrough |= s.strikethrough;
                st.monospace |= s.monospace;
                st.spoiler |= s.spoiler;
                st.mention |= s.mention;
                st.link |= s.link;
            }
        }
        if !st.is_plain() {
            match spans.last_mut() {
                Some((r, s)) if r.end == a && *s == st => r.end = b,
                _ => spans.push((a..b, st)),
            }
        }
    }
    RichText { text, spans, links }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOB: Uuid = Uuid::from_u128(0xB0B);

    fn name(_: &Uuid) -> String {
        "Bob".into()
    }

    #[test]
    fn plain_text_has_no_spans() {
        let r = layout("hello there", &[], name);
        assert_eq!(r.text, "hello there");
        assert!(r.is_plain());
    }

    #[test]
    fn mentions_are_substituted_and_shift_styles() {
        // "hi ￼ bold" with mention at utf16 3 and bold over "bold" (utf16 5..9)
        let body = "hi \u{FFFC} bold";
        let ranges = vec![
            BodyRange {
                start: 3,
                length: 1,
                kind: RangeKind::Mention(BOB),
            },
            BodyRange {
                start: 5,
                length: 4,
                kind: RangeKind::Bold,
            },
        ];
        let r = layout(body, &ranges, name);
        assert_eq!(r.text, "hi @Bob bold");
        let bold = r.spans.iter().find(|(_, s)| s.bold).unwrap();
        assert_eq!(&r.text[bold.0.clone()], "bold");
        let mention = r.spans.iter().find(|(_, s)| s.mention).unwrap();
        assert_eq!(&r.text[mention.0.clone()], "@Bob");
    }

    #[test]
    fn overlapping_styles_are_flattened() {
        let ranges = vec![
            BodyRange {
                start: 0,
                length: 6,
                kind: RangeKind::Bold,
            },
            BodyRange {
                start: 3,
                length: 6,
                kind: RangeKind::Italic,
            },
        ];
        let r = layout("abcdefghi", &ranges, name);
        let spans: Vec<(&str, bool, bool)> = r
            .spans
            .iter()
            .map(|(rg, s)| (&r.text[rg.clone()], s.bold, s.italic))
            .collect();
        assert_eq!(
            spans,
            vec![
                ("abc", true, false),
                ("def", true, true),
                ("ghi", false, true)
            ]
        );
    }

    #[test]
    fn utf16_offsets_with_emoji() {
        // 😀 is two UTF-16 units and four bytes
        let ranges = vec![BodyRange {
            start: 3,
            length: 2,
            kind: RangeKind::Italic,
        }];
        let r = layout("😀 ok!", &ranges, name);
        let (rg, _) = &r.spans[0];
        assert_eq!(&r.text[rg.clone()], "ok");
    }

    #[test]
    fn out_of_range_ranges_are_ignored() {
        let ranges = vec![BodyRange {
            start: 10,
            length: 5,
            kind: RangeKind::Bold,
        }];
        assert!(layout("short", &ranges, name).is_plain());
    }

    #[test]
    fn links_are_detected() {
        let r = layout(
            "see https://signal.org/download, or www.example.com. (http://a.b/c)",
            &[],
            name,
        );
        let urls: Vec<&str> = r.links.iter().map(|(_, u)| u.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://signal.org/download",
                "https://www.example.com",
                "http://a.b/c"
            ]
        );
        for (rg, _) in &r.links {
            assert!(r.spans.iter().any(|(s, st)| s == rg && st.link));
        }
        assert!(
            find_links("email me at foo@www.bar").is_empty(),
            "not at a word start"
        );
        assert!(find_links("https://").is_empty());
    }
}
