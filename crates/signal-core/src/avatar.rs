//! Avatar colours and initials, mirroring Signal Desktop's `AvatarColors`.

/// A background/foreground colour pair as 0xRRGGBB values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvatarColor {
    pub name: &'static str,
    pub bg: u32,
    pub fg: u32,
}

/// The twelve avatar colours used by Signal (A100 … A210).
pub const AVATAR_COLORS: [AvatarColor; 12] = [
    AvatarColor { name: "A100", bg: 0xE3E3FE, fg: 0x3838F5 },
    AvatarColor { name: "A110", bg: 0xDDE7FC, fg: 0x1251D3 },
    AvatarColor { name: "A120", bg: 0xD8E8F0, fg: 0x086DA0 },
    AvatarColor { name: "A130", bg: 0xCDE4CD, fg: 0x067906 },
    AvatarColor { name: "A140", bg: 0xEAE0FD, fg: 0x661AFF },
    AvatarColor { name: "A150", bg: 0xF5E3FE, fg: 0x9F00F0 },
    AvatarColor { name: "A160", bg: 0xF6D8EC, fg: 0xB8057C },
    AvatarColor { name: "A170", bg: 0xF5D7D7, fg: 0xBE0404 },
    AvatarColor { name: "A180", bg: 0xFEF5D0, fg: 0x836B01 },
    AvatarColor { name: "A190", bg: 0xEAE6D5, fg: 0x7D6F40 },
    AvatarColor { name: "A200", bg: 0xD2D2DC, fg: 0x4F4F6D },
    AvatarColor { name: "A210", bg: 0xD7D7D9, fg: 0x5C5C5C },
];

/// Picks a stable colour for an identifier (uuid bytes / group id).
pub fn color_for(seed: &[u8]) -> AvatarColor {
    // FNV-1a: tiny, stable across platforms and releases.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in seed {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    AVATAR_COLORS[(hash % AVATAR_COLORS.len() as u64) as usize]
}

/// Up to two initials for a display name, like Signal Desktop's `getInitials`.
/// Returns `None` for names without letters (e.g. phone numbers), in which
/// case the UI shows a generic person/group glyph.
pub fn initials(name: &str) -> Option<String> {
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|w| w.chars().next().is_some_and(char::is_alphabetic))
        .collect();
    let first = words.first()?.chars().next()?;
    let mut out: String = first.to_uppercase().collect();
    if words.len() > 1 {
        if let Some(last) = words.last().and_then(|w| w.chars().next()) {
            out.extend(last.to_uppercase());
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_cases() {
        assert_eq!(initials("Alice"), Some("A".into()));
        assert_eq!(initials("alice smith"), Some("AS".into()));
        assert_eq!(initials("Jean Paul Sartre"), Some("JS".into()));
        assert_eq!(initials("+1 555 0100"), None);
        assert_eq!(initials(""), None);
        assert_eq!(initials("Émile 42"), Some("É".into()));
    }

    #[test]
    fn color_is_stable_and_distributed() {
        let a = color_for(b"abc");
        assert_eq!(a, color_for(b"abc"));
        let distinct: std::collections::HashSet<_> =
            (0u32..200).map(|i| color_for(&i.to_le_bytes()).name).collect();
        assert!(distinct.len() >= 10, "poor distribution: {distinct:?}");
    }
}
