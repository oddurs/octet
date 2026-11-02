// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Terminal display width of text, for padding and clipping card cells.
//!
//! Cards draw their right border after padding a cell to a fixed width, so the padding must
//! count terminal columns rather than `char`s: a CJK ideograph or an emoji takes two columns
//! and a combining accent takes none.

/// Ranges with East Asian Width `W` or `F` (Unicode 15.1), which terminals draw two columns
/// wide. This covers CJK, Hangul, kana, fullwidth forms, and emoji with default emoji
/// presentation.
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x303E),
    (0x3041, 0x3247),
    (0x3250, 0x4DBF),
    (0x4E00, 0xA4C6),
    (0xA960, 0xA97C),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAD9),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6B),
    (0xFF01, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x16FE0, 0x1B2FB),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F200, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FAF8),
    (0x20000, 0x323AF),
];

/// Combining marks, joiners, variation selectors, and invisible format characters that draw
/// onto the previous cell instead of taking one of their own. Common blocks only; a rare mark
/// outside them costs one column of misalignment, not a garbled card.
const ZERO: &[(u32, u32)] = &[
    (0x0300, 0x036F),
    (0x0483, 0x0489),
    (0x0591, 0x05BD),
    (0x05BF, 0x05BF),
    (0x05C1, 0x05C2),
    (0x05C4, 0x05C5),
    (0x05C7, 0x05C7),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x06DF, 0x06E4),
    (0x06E7, 0x06E8),
    (0x06EA, 0x06ED),
    (0x0E31, 0x0E31),
    (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x20D0, 0x20FF),
    (0x302A, 0x302D),
    (0x3099, 0x309A),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xFEFF, 0xFEFF),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
    (0xE0100, 0xE01EF),
];

fn in_table(table: &[(u32, u32)], cp: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Columns a terminal gives `c`: 2 for wide, 0 for combining or invisible, else 1.
pub(crate) fn char_width(c: char) -> usize {
    let cp = u32::from(c);
    // Everything below U+0300 is one column; skipping the tables keeps ASCII cheap.
    if cp < 0x300 {
        1
    } else if in_table(ZERO, cp) {
        0
    } else if in_table(WIDE, cp) {
        2
    } else {
        1
    }
}

/// Columns a terminal gives `s`.
pub(crate) fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// The longest prefix of `s` that fits in `max` columns, and its width.
///
/// A wide character that would straddle the limit is left out whole, so the caller pads the
/// remaining column with a space rather than splitting the glyph across a border.
pub(crate) fn take_width(s: &str, max: usize) -> (&str, usize) {
    let mut used = 0;
    for (idx, c) in s.char_indices() {
        let w = char_width(c);
        if used + w > max {
            return (&s[..idx], used);
        }
        used += w;
    }
    (s, used)
}

/// Pads `s` with spaces to exactly `width` columns, or clips it with an ellipsis when it
/// does not fit.
pub(crate) fn pad_or_clip(s: &str, width: usize) -> String {
    let w = str_width(s);
    if w <= width {
        return format!("{s}{}", " ".repeat(width - w));
    }
    if width == 0 {
        return String::new();
    }
    // Wide cells keep a space between the ellipsis and the border so the clip reads as one.
    let suffix = if width >= 4 { "… " } else { "…" };
    let budget = width - str_width(suffix);
    let (head, used) = take_width(s, budget);
    format!("{head}{}{suffix}", " ".repeat(budget - used))
}

/// Pads `s` with spaces to exactly `width` columns, or cuts it at the last character that
/// fits.
pub(crate) fn pad_or_cut(s: &str, width: usize) -> String {
    let (head, used) = take_width(s, width);
    format!("{head}{}", " ".repeat(width - used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_disjoint_for_binary_search() {
        for table in [WIDE, ZERO] {
            assert!(table.iter().all(|&(lo, hi)| lo <= hi));
            assert!(table.windows(2).all(|w| w[0].1 < w[1].0));
        }
    }

    #[test]
    fn char_width_follows_terminal_columns() {
        for c in ['a', ' ', '~', 'é', '◈', '│', '…', '★', '⋮', '▪', 'Ω'] {
            assert_eq!(char_width(c), 1, "{c:?}");
        }
        for c in ['漢', 'テ', '한', 'Ａ', '😀', '👍', '🏽', '⭐', '\u{20000}'] {
            assert_eq!(char_width(c), 2, "{c:?}");
        }
        for c in ['\u{301}', '\u{200d}', '\u{fe0f}', '\u{200b}', '\u{e0100}'] {
            assert_eq!(char_width(c), 0, "{c:?}");
        }
    }

    #[test]
    fn str_width_sums_columns() {
        assert_eq!(str_width(""), 0);
        assert_eq!(str_width("abc"), 3);
        assert_eq!(str_width("漢字.png"), 8);
        assert_eq!(str_width("e\u{301}"), 1);
        assert_eq!(str_width("👨\u{200d}💻"), 4);
    }

    #[test]
    fn take_width_never_splits_a_wide_char() {
        assert_eq!(take_width("漢字", 3), ("漢", 2));
        assert_eq!(take_width("漢字", 4), ("漢字", 4));
        assert_eq!(take_width("a漢", 2), ("a", 1));
        assert_eq!(take_width("e\u{301}x", 1), ("e\u{301}", 1));
        assert_eq!(take_width("abc", 0), ("", 0));
    }

    #[test]
    fn pad_or_clip_fills_exactly_the_requested_columns() {
        assert_eq!(pad_or_clip("ab", 4), "ab  ");
        assert_eq!(pad_or_clip("abcdef", 5), "abc… ");
        assert_eq!(pad_or_clip("abcdef", 3), "ab…");
        assert_eq!(pad_or_clip("abcdef", 0), "");
        assert_eq!(pad_or_clip("漢字", 4), "漢字");
        assert_eq!(pad_or_clip("漢字", 5), "漢字 ");
        // A wide char that would straddle the clip point becomes a space.
        assert_eq!(pad_or_clip("a漢字漢字", 6), "a漢 … ");
        for s in [
            "漢字漢字.png",
            "😀👍🏽 é 한국어",
            "e\u{301}e\u{301}e\u{301}",
            "plain",
        ] {
            for width in 0..16 {
                assert_eq!(str_width(&pad_or_clip(s, width)), width, "{s:?} @ {width}");
                assert_eq!(str_width(&pad_or_cut(s, width)), width, "{s:?} @ {width}");
            }
        }
    }

    #[test]
    fn pad_or_cut_keeps_ascii_behaviour() {
        assert_eq!(pad_or_cut("abcdef", 4), "abcd");
        assert_eq!(pad_or_cut("ab", 4), "ab  ");
        assert_eq!(pad_or_cut("a漢", 2), "a ");
    }
}
