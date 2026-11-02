// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Multi-representation byte inspector and pattern search utilities.

use std::fmt::Write as _;
use std::ops::Range;

use crate::error::{Error, Result};
use crate::theme::Theme;
use crate::width::{pad_or_cut, str_width, take_width};

/// Decoded numeric and text representations of bytes starting at a specific offset.
#[derive(Debug, Clone)]
pub struct ValueInspection {
    /// Absolute offset of the inspected position.
    pub offset: u64,
    /// Up to 8 raw bytes starting at the inspected position.
    pub raw_bytes: Vec<u8>,
    /// Unsigned and signed 8-bit values.
    pub int8: Option<(u8, i8)>,
    /// Little-endian and big-endian 16-bit integer values.
    pub int16: Option<((u16, i16), (u16, i16))>,
    /// Little-endian and big-endian 32-bit integer values.
    pub int32: Option<((u32, i32), (u32, i32))>,
    /// Little-endian and big-endian 64-bit integer values.
    pub int64: Option<((u64, i64), (u64, i64))>,
    /// Little-endian and big-endian IEEE-754 `f32` values.
    pub float32: Option<(f32, f32)>,
    /// Little-endian and big-endian IEEE-754 `f64` values.
    pub float64: Option<(f64, f64)>,
    /// Unsigned LEB128 varint value and byte length, if valid.
    pub uleb128: Option<(u64, usize)>,
    /// Signed LEB128 varint value and byte length, if valid.
    pub sleb128: Option<(i64, usize)>,
    /// Decoded UTF-8 character at the start of the slice, if valid.
    pub utf8_char: Option<(char, usize)>,
}

impl ValueInspection {
    /// Inspects the bytes starting at `index` inside `data` (with base `display_offset`).
    #[must_use]
    pub fn at(data: &[u8], index: usize, display_offset: u64) -> Option<Self> {
        if index >= data.len() {
            return None;
        }
        let slice = &data[index..];
        let raw_bytes = slice.iter().copied().take(8).collect::<Vec<_>>();
        let int8 = Some((slice[0], slice[0] as i8));

        let int16 = if slice.len() >= 2 {
            let b: [u8; 2] = [slice[0], slice[1]];
            Some((
                (u16::from_le_bytes(b), i16::from_le_bytes(b)),
                (u16::from_be_bytes(b), i16::from_be_bytes(b)),
            ))
        } else {
            None
        };

        let int32 = if slice.len() >= 4 {
            let b: [u8; 4] = [slice[0], slice[1], slice[2], slice[3]];
            Some((
                (u32::from_le_bytes(b), i32::from_le_bytes(b)),
                (u32::from_be_bytes(b), i32::from_be_bytes(b)),
            ))
        } else {
            None
        };

        let int64 = if slice.len() >= 8 {
            let b: [u8; 8] = [
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ];
            Some((
                (u64::from_le_bytes(b), i64::from_le_bytes(b)),
                (u64::from_be_bytes(b), i64::from_be_bytes(b)),
            ))
        } else {
            None
        };

        let float32 = if slice.len() >= 4 {
            let b: [u8; 4] = [slice[0], slice[1], slice[2], slice[3]];
            Some((f32::from_le_bytes(b), f32::from_be_bytes(b)))
        } else {
            None
        };

        let float64 = if slice.len() >= 8 {
            let b: [u8; 8] = [
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ];
            Some((f64::from_le_bytes(b), f64::from_be_bytes(b)))
        } else {
            None
        };

        let uleb128 = decode_uleb128(slice);
        let sleb128 = decode_sleb128(slice);
        let utf8_char = decode_first_utf8_char(slice);

        Some(Self {
            offset: display_offset.wrapping_add(index as u64),
            raw_bytes,
            int8,
            int16,
            int32,
            int64,
            float32,
            float64,
            uleb128,
            sleb128,
            utf8_char,
        })
    }

    /// Renders the value inspection card as an ANSI-styled terminal string.
    #[must_use]
    pub fn render_ansi(&self, theme: &Theme, color: bool) -> String {
        let mut out = String::new();
        let inner_w = 104usize;
        let top = format!("╭{}╮", "─".repeat(inner_w));
        let bot = format!("╰{}╯", "─".repeat(inner_w));
        let _ = writeln!(out, "{}", theme.paint(&top, theme.border, color));

        let hex_bytes = self
            .raw_bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let title_str = format!(
            " ◈ DATA INSPECTOR @ 0x{:08x}   │   Raw Bytes: [ {hex_bytes} ]",
            self.offset
        );
        write_card_row(
            &mut out,
            &title_str,
            inner_w,
            theme,
            theme.offset_active,
            true,
            color,
        );

        if let Some((u, i)) = self.int8 {
            let line = format!(
                "   {:<16} : {:<26} {:<16} : {:08b} (0o{:03o})",
                "u8 / i8",
                format!("{u} / {i}"),
                "Bits / Octal",
                u,
                u
            );
            write_card_row(
                &mut out,
                &line,
                inner_w,
                theme,
                theme.fg_primary,
                false,
                color,
            );
        }
        if let Some(((ule, ile), (ube, ibe))) = self.int16 {
            let line = format!(
                "   {:<16} : {:<26} {:<16} : {} / {}",
                "u16 / i16 (LE)",
                format!("{ule} / {ile}"),
                "u16 / i16 (BE)",
                ube,
                ibe
            );
            write_card_row(
                &mut out,
                &line,
                inner_w,
                theme,
                theme.fg_primary,
                false,
                color,
            );
        }
        if let Some(((ule, ile), (ube, ibe))) = self.int32 {
            let line = format!(
                "   {:<16} : {:<26} {:<16} : {} / {}",
                "u32 / i32 (LE)",
                format!("{ule} / {ile}"),
                "u32 / i32 (BE)",
                ube,
                ibe
            );
            write_card_row(
                &mut out,
                &line,
                inner_w,
                theme,
                theme.fg_primary,
                false,
                color,
            );
        }
        if let Some(((ule, ile), (ube, ibe))) = self.int64 {
            // One row per signedness: "u64 / i64" pairs reach 43 columns at the extremes,
            // wider than either value column.
            let rows = [
                ("u64 (LE)", ule.to_string(), "u64 (BE)", ube.to_string()),
                ("i64 (LE)", ile.to_string(), "i64 (BE)", ibe.to_string()),
            ];
            for (le_label, le_val, be_label, be_val) in rows {
                let line = format!("   {le_label:<16} : {le_val:<26} {be_label:<16} : {be_val}");
                write_card_row(
                    &mut out,
                    &line,
                    inner_w,
                    theme,
                    theme.fg_primary,
                    false,
                    color,
                );
            }
        }
        if let Some((fle, fbe)) = self.float32 {
            let line = format!(
                "   {:<16} : {:<26.6e} {:<16} : {:.6e}",
                "f32 (LE)", fle, "f32 (BE)", fbe
            );
            write_card_row(
                &mut out,
                &line,
                inner_w,
                theme,
                theme.fg_primary,
                false,
                color,
            );
        }
        if let Some((fle, fbe)) = self.float64 {
            let line = format!(
                "   {:<16} : {:<26.9e} {:<16} : {:.9e}",
                "f64 (LE)", fle, "f64 (BE)", fbe
            );
            write_card_row(
                &mut out,
                &line,
                inner_w,
                theme,
                theme.byte_ascii,
                true,
                color,
            );
        }
        let leb_info = match (self.uleb128, self.sleb128) {
            (Some((uval, ulen)), Some((sval, _))) => format!("{uval} / {sval:+} ({ulen} B)"),
            (Some((uval, ulen)), None) => format!("{uval} / overflow ({ulen} B)"),
            (None, Some((sval, slen))) => format!("overflow / {sval:+} ({slen} B)"),
            (None, None) => String::from("n/a (unterminated/overflow)"),
        };
        let utf8_info = match self.utf8_char {
            Some((ch, clen)) => format!("U+{:04X} {:?} ({clen} B)", ch as u32, ch),
            None => String::from("Invalid UTF-8 lead"),
        };
        // Each gets a full row: a ten-byte varint pair runs to 50 columns, and the codepoint
        // is drawn at its terminal width, so neither fits a half-row column.
        for line in [
            format!("   {:<16} : {leb_info}", "ULEB / SLEB128"),
            format!("   {:<16} : {utf8_info}", "UTF-8 Codepoint"),
        ] {
            write_card_row(
                &mut out,
                &line,
                inner_w,
                theme,
                theme.fg_secondary,
                false,
                color,
            );
        }
        let _ = writeln!(out, "{}", theme.paint(&bot, theme.border, color));
        out
    }
}

fn write_card_row(
    out: &mut String,
    text: &str,
    inner_w: usize,
    theme: &Theme,
    fg: crate::theme::Rgb,
    bold: bool,
    color: bool,
) {
    let padded = pad_or_cut(text, inner_w);
    let styled = if bold {
        theme.paint_bold(&padded, fg, color)
    } else {
        theme.paint(&padded, fg, color)
    };
    let _ = writeln!(
        out,
        "{}{styled}{}",
        theme.paint("│", theme.border, color),
        theme.paint("│", theme.border, color)
    );
}

pub(crate) fn decode_uleb128(slice: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    for (i, &byte) in slice.iter().take(10).enumerate() {
        let low = u64::from(byte & 0x7f);
        if shift == 63 && low > 1 {
            return None;
        }
        result |= low.checked_shl(shift)?;
        if (byte & 0x80) == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None
}

fn decode_sleb128(slice: &[u8]) -> Option<(i64, usize)> {
    let mut result = 0i64;
    let mut shift = 0u32;
    for (i, &byte) in slice.iter().take(10).enumerate() {
        let low = byte & 0x7f;
        if shift == 63 && low != 0x00 && low != 0x7f {
            return None;
        }
        result |= i64::from(low).wrapping_shl(shift);
        shift += 7;
        if (byte & 0x80) == 0 {
            if shift < 64 && (byte & 0x40) != 0 {
                result |= (!0i64).wrapping_shl(shift);
            }
            return Some((result, i + 1));
        }
    }
    None
}

fn decode_first_utf8_char(slice: &[u8]) -> Option<(char, usize)> {
    for len in 1..=slice.len().min(4) {
        if let Ok(s) = std::str::from_utf8(&slice[..len])
            && let Some(ch) = s.chars().next()
        {
            return Some((ch, ch.len_utf8()));
        }
    }
    None
}

/// Parses a search query (either `0x...` hex bytes or an ASCII/UTF-8 string) into raw bytes.
pub fn parse_search_pattern(query: &str) -> Result<Vec<u8>> {
    if query.is_empty() {
        return Err(Error::InvalidArgument(
            "search pattern cannot be empty".to_owned(),
        ));
    }
    if let Some(hex_str) = query
        .strip_prefix("0x")
        .or_else(|| query.strip_prefix("0X"))
    {
        // Decoding per char (rather than slicing the string in 2-byte steps) keeps non-ASCII
        // input from splitting a UTF-8 sequence, and `to_digit` rejects the `+` sign that
        // `u8::from_str_radix` would accept.
        let mut nibbles = Vec::with_capacity(hex_str.len());
        for c in hex_str.chars().filter(|c| !c.is_whitespace()) {
            let digit = c.to_digit(16).ok_or_else(|| {
                Error::InvalidArgument(format!(
                    "invalid hex digit '{c}' in search pattern '{query}'"
                ))
            })?;
            nibbles.push(digit as u8);
        }
        if nibbles.is_empty() || !nibbles.len().is_multiple_of(2) {
            return Err(Error::InvalidArgument(format!(
                "hex search pattern '{query}' must have an even number of hex digits"
            )));
        }
        Ok(nibbles
            .chunks(2)
            .map(|pair| (pair[0] << 4) | pair[1])
            .collect())
    } else {
        Ok(query.as_bytes().to_vec())
    }
}

/// Finds all non-overlapping byte ranges in `data` that match `needle`.
#[must_use]
pub fn find_pattern_spans(data: &[u8], needle: &[u8]) -> Vec<Range<usize>> {
    if needle.is_empty() || needle.len() > data.len() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut i = 0;
    while i + needle.len() <= data.len() {
        if &data[i..i + needle.len()] == needle {
            spans.push(i..i + needle.len());
            i += needle.len();
        } else {
            i += 1;
        }
    }
    spans
}

/// A contiguous printable ASCII or graphic UTF-8 string literal extracted from a binary buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedString {
    /// Displayed starting byte offset of the string.
    pub offset: u64,
    /// Length of the string in raw bytes.
    pub byte_len: usize,
    /// Length of the string in Unicode scalar values (`char`s).
    pub char_len: usize,
    /// Whether the string contains multi-byte UTF-8 characters.
    pub is_utf8_multibyte: bool,
    /// Whether the string is immediately followed by a `0x00` NUL terminator byte.
    pub null_terminated: bool,
    /// Sanitized single-line representation of the extracted string.
    pub text: String,
}

/// Scans `data` for contiguous graphic UTF-8 / printable ASCII strings of at least `min_chars`
/// characters (`--strings`).
#[must_use]
pub fn extract_strings(data: &[u8], min_chars: usize, display_offset: u64) -> Vec<ExtractedString> {
    let min_chars = min_chars.max(1);
    let mut results = Vec::new();
    let mut i = 0;

    while i < data.len() {
        let start = i;
        let mut text = String::new();
        let mut char_len = 0usize;
        let mut has_multibyte = false;

        while i < data.len() {
            let b = data[i];
            if (0x20..=0x7e).contains(&b) {
                text.push(b as char);
                char_len += 1;
                i += 1;
            } else if b == b'\t' {
                text.push(' ');
                char_len += 1;
                i += 1;
            } else if b >= 0xc2
                && let Some((ch, clen)) = decode_first_utf8_char(&data[i..])
                && clen > 1
                && !ch.is_control()
            {
                text.push(ch);
                char_len += 1;
                has_multibyte = true;
                i += clen;
            } else {
                break;
            }
        }

        if char_len >= min_chars && text.chars().any(|c| !c.is_whitespace()) {
            let null_terminated = data.get(i) == Some(&0x00);
            results.push(ExtractedString {
                offset: display_offset.saturating_add(start as u64),
                byte_len: i - start,
                char_len,
                is_utf8_multibyte: has_multibyte,
                null_terminated,
                text,
            });
        }

        if i == start {
            i += 1;
        }
    }

    results
}

/// Renders extracted strings (`--strings`) as an ANSI-styled 106-column table card.
#[must_use]
pub fn render_strings_ansi(
    strings: &[ExtractedString],
    total_bytes: usize,
    min_chars: usize,
    label: Option<&str>,
    theme: &Theme,
    color: bool,
) -> String {
    let max_off = strings.last().map_or(0, |s| s.offset);
    let off_w = format!("{max_off:08x}").len();
    let card_w = 106 + off_w.saturating_sub(8);
    let inner_w = card_w - 2;
    let mut out = String::with_capacity(strings.len() * (card_w + 32) + 512);

    let top = format!("╭{}╮", "─".repeat(inner_w));
    let mid = format!("├{}┤", "─".repeat(inner_w));
    let bot = format!("╰{}╯", "─".repeat(inner_w));

    let _ = writeln!(out, "{}", theme.paint(&top, theme.border, color));
    let lbl = crate::format::neutralise_controls(label.unwrap_or("stdin"));
    let hdr_plain = format!(
        " ◈ STRING TABLE · {lbl}  │  {} strings (min {min_chars} chars) in {total_bytes} B",
        strings.len()
    );
    write_card_row(
        &mut out,
        &hdr_plain,
        inner_w,
        theme,
        theme.offset_active,
        true,
        color,
    );
    let _ = writeln!(out, "{}", theme.paint(&mid, theme.border, color));

    let col_hdr = format!(
        " {:<off_w$}  {:>6}  {:<8}  STRING LITERAL",
        "OFFSET", "BYTES", "ENCODING"
    );
    write_card_row(
        &mut out,
        &col_hdr,
        inner_w,
        theme,
        theme.fg_muted,
        true,
        color,
    );
    let _ = writeln!(out, "{}", theme.paint(&bot, theme.border, color));

    if strings.is_empty() {
        let msg = format!("  (no printable strings of length >= {min_chars} found)");
        let _ = writeln!(out, "{}", theme.paint(&msg, theme.fg_muted, color));
        return out;
    }

    for item in strings {
        let off_str = theme.format_offset(item.offset, off_w, false, color);
        let len_str = format!("{:>5}B", item.byte_len);
        let enc_tag = match (item.is_utf8_multibyte, item.null_terminated) {
            (true, true) => "UTF-8\\0 ",
            (true, false) => "UTF-8   ",
            (false, true) => "ASCII\\0 ",
            (false, false) => "ASCII   ",
        };
        let enc_color = if item.is_utf8_multibyte {
            theme.byte_utf8
        } else {
            theme.byte_ascii
        };
        // Offset, length, encoding, and the "▸ " marker; the length column widens past
        // 99999 B, so measure rather than assume.
        let prefix_w = off_w + 2 + str_width(&len_str) + 2 + str_width(enc_tag) + 2 + 2;
        let clipped_lit = clip_string_literal(&item.text, card_w.saturating_sub(prefix_w));
        let _ = writeln!(
            out,
            "{off_str}  {}  {}  {} {}",
            theme.paint(&len_str, theme.fg_secondary, color),
            theme.paint_bold(enc_tag, enc_color, color),
            theme.paint("▸", enc_color, color),
            theme.paint(&clipped_lit, theme.fg_primary, color),
        );
    }

    out
}

/// Renders extracted strings (`--strings -j`) as a standalone RFC 8259 JSON report.
#[must_use]
pub fn render_strings_json(
    strings: &[ExtractedString],
    total_bytes: usize,
    min_chars: usize,
    label: Option<&str>,
) -> String {
    let lbl = label.unwrap_or("stdin");
    let mut out = String::from("{\n");
    let _ = writeln!(out, "  \"mode\": \"strings\",");
    let _ = writeln!(out, "  \"label\": \"{}\",", escape_json_str(lbl));
    let _ = writeln!(out, "  \"total_bytes\": {total_bytes},");
    let _ = writeln!(out, "  \"min_chars\": {min_chars},");
    let _ = writeln!(out, "  \"count\": {},", strings.len());
    let _ = writeln!(out, "  \"strings\": [");
    for (idx, s) in strings.iter().enumerate() {
        let comma = if idx + 1 < strings.len() { "," } else { "" };
        let enc = if s.is_utf8_multibyte { "utf8" } else { "ascii" };
        let _ = writeln!(
            out,
            "    {{ \"offset\": {}, \"byte_len\": {}, \"char_len\": {}, \"encoding\": \"{enc}\", \"null_terminated\": {}, \"text\": \"{}\" }}{comma}",
            s.offset,
            s.byte_len,
            s.char_len,
            s.null_terminated,
            escape_json_str(&s.text)
        );
    }
    let _ = writeln!(out, "  ]");
    out.push_str("}\n");
    out
}

/// Quotes `s`, clipping it with a trailing ellipsis so the result spans at most `max_cols`
/// terminal columns.
fn clip_string_literal(s: &str, max_cols: usize) -> String {
    if str_width(s) + 2 <= max_cols {
        format!("\"{s}\"")
    } else {
        let (prefix, _) = take_width(s, max_cols.saturating_sub(3));
        format!("\"{prefix}\"…")
    }
}

fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::width::str_width;

    fn line_widths(rendered: &str) -> Vec<usize> {
        rendered.lines().map(str_width).collect()
    }

    #[test]
    fn inspector_fits_the_widest_values_and_shows_u64_and_i64() {
        // Display column of the second " : " in a two-column row: border, indent, label,
        // separator, left value, gap, right label.
        const RIGHT_SEPARATOR_COL: usize = 1 + 3 + 16 + 3 + 26 + 1 + 16;
        let patterns: [[u8; 8]; 7] = [
            [0x80, 0, 0, 0, 0, 0, 0, 0x80],
            [0xff; 8],
            [0x80, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0x80],
            (-f64::MAX).to_be_bytes(),
            f64::NAN.to_le_bytes(),
            f64::NEG_INFINITY.to_be_bytes(),
        ];
        for bytes in patterns {
            let insp = ValueInspection::at(&bytes, 0, 0xffff_ffff_ffff).expect("in range");
            let rendered = insp.render_ansi(&Theme::dark(), false);
            assert!(
                line_widths(&rendered).iter().all(|&w| w == 106),
                "{bytes:02x?}\n{rendered}"
            );
            let ((ule, ile), (ube, ibe)) = insp.int64.expect("eight bytes");
            for value in [ule.to_string(), ube.to_string()] {
                assert!(rendered.contains(&value), "u64 {value} missing\n{rendered}");
            }
            for value in [ile.to_string(), ibe.to_string()] {
                assert!(rendered.contains(&value), "i64 {value} missing\n{rendered}");
            }
            for line in rendered.lines() {
                if let Some((idx, _)) = line.match_indices(" : ").nth(1) {
                    assert_eq!(
                        str_width(&line[..idx]),
                        RIGHT_SEPARATOR_COL,
                        "left column overran in {line:?}"
                    );
                }
            }
        }
        let rendered = ValueInspection::at(&(-f64::MAX).to_be_bytes(), 0, 0)
            .expect("in range")
            .render_ansi(&Theme::dark(), false);
        assert!(rendered.contains("-1.797693135e308"), "{rendered}");
        for (bytes, text) in [
            (f64::NAN.to_le_bytes(), "NaN"),
            (f64::NEG_INFINITY.to_be_bytes(), "-inf"),
        ] {
            let rendered = ValueInspection::at(&bytes, 0, 0)
                .expect("in range")
                .render_ansi(&Theme::dark(), false);
            assert!(rendered.contains(text), "{rendered}");
        }
    }

    #[test]
    fn inspector_card_keeps_its_border_after_a_wide_codepoint() {
        for text in ["漢xyz", "😀abcdefgh", "e\u{301}"] {
            let rendered = ValueInspection::at(text.as_bytes(), 0, 0)
                .expect("in range")
                .render_ansi(&Theme::dark(), false);
            assert!(
                line_widths(&rendered).iter().all(|&w| w == 106),
                "{text}\n{rendered}"
            );
        }
    }

    #[test]
    fn strings_table_stays_within_its_card() {
        let mut data = "漢字テキスト 😀👍🏽 e\u{301} 한국어 ".repeat(8).into_bytes();
        data.push(0);
        for len in [70, 74, 75, 76, 77, 120] {
            data.extend(std::iter::repeat_n(b'x', len));
            data.push(0);
        }
        for (label, offset) in [("漢字漢字.bin", 0), ("plain.bin", 0x1_0000_0000)] {
            let strings = extract_strings(&data, 4, offset);
            let rendered =
                render_strings_ansi(&strings, data.len(), 4, Some(label), &Theme::dark(), false);
            let widths = line_widths(&rendered);
            let card_w = widths[0];
            assert!(widths[..5].iter().all(|&w| w == card_w), "{rendered}");
            assert!(widths.iter().all(|&w| w <= card_w), "{rendered}");
        }
    }
}
