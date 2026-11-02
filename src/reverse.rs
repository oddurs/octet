// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Reverse hexadecimal or binary dump to raw binary conversion (`-r` / `--reverse`).

use crate::error::{Error, Result};

/// Reverses a hexadecimal (or binary `-b`) dump string back into raw binary bytes.
///
/// When `plain` is `true` (`-r -p`), all whitespace is ignored and pairs of hex digits are
/// decoded sequentially. When `plain` is `false`, each data row's leading offset and data column
/// area are parsed (supporting `octet` smart dumps with elided rows, classic `xxd` dumps, and
/// `-b` binary dumps).
pub fn reverse_dump(input: &str, plain: bool) -> Result<Vec<u8>> {
    if plain {
        return reverse_plain(input);
    }
    reverse_structured(input)
}

fn reverse_plain(input: &str) -> Result<Vec<u8>> {
    let clean = strip_ansi(input);
    let mut out = Vec::with_capacity(clean.len() / 2);
    let mut high_nybble: Option<u8> = None;

    for (line_idx, line) in clean.lines().enumerate() {
        for ch in line.chars() {
            if ch.is_whitespace() {
                continue;
            }
            let Some(val) = ch.to_digit(16) else {
                return Err(Error::ParseHex {
                    line: line_idx + 1,
                    message: format!("unexpected non-hex character {ch:?} in plain hex stream"),
                });
            };
            let nybble = val as u8;
            if let Some(hi) = high_nybble.take() {
                out.push((hi << 4) | nybble);
            } else {
                high_nybble = Some(nybble);
            }
        }
    }

    if high_nybble.is_some() {
        return Err(Error::ParseHex {
            line: clean.lines().count().max(1),
            message: "odd number of hex digits in plain hex stream".to_owned(),
        });
    }
    Ok(out)
}

fn reverse_structured(input: &str) -> Result<Vec<u8>> {
    let clean = strip_ansi(input);
    let mut out: Vec<u8> = Vec::new();
    let mut base_offset: Option<u64> = None;
    let mut last_row_bytes: Vec<u8> = Vec::new();
    let mut pending_star = false;

    for line in clean.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.trim_end() == "*" {
            pending_star = true;
            continue;
        }
        let Some(first_char) = trimmed.chars().next() else {
            continue;
        };
        if !first_char.is_ascii_hexdigit() {
            continue;
        }

        let offset_end = trimmed
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(trimmed.len());
        if offset_end < 4 {
            continue;
        }
        let Ok(row_offset) = u64::from_str_radix(&trimmed[..offset_end], 16) else {
            continue;
        };

        let rest = &trimmed[offset_end..];
        let colon_row = rest.starts_with(':');
        let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();

        // If this is an `octet` elision bridge (`⋮   ⋯ N identical rows ...`), immediately
        // materialize the elided rows so even trailing elided runs at EOF are preserved.
        if rest.starts_with('⋮') {
            if let Some(elided_rows) = parse_elided_row_count(rest)
                && !last_row_bytes.is_empty()
            {
                for _ in 0..elided_rows {
                    out.extend_from_slice(&last_row_bytes);
                }
            }
            continue;
        }
        if rest.starts_with('*') {
            pending_star = true;
            continue;
        }

        // The ASCII column may itself contain `|` or runs of spaces, so the separator is chosen
        // from the row's shape rather than from whichever marker appears first.
        let (data_zone, glyphs) = if let Some((data, pane)) = rest.split_once('│') {
            let pane = pane.split('│').next().unwrap_or(pane);
            (data, Glyphs::Octet(pane))
        } else if colon_row {
            let data = rest.split_once("  ").map_or(rest, |(data, _)| data);
            (data, Glyphs::Xxd(rest))
        } else if rest.trim_end().ends_with('|')
            && let Some((data, _)) = rest.split_once("  |")
        {
            (data, Glyphs::Unknown)
        } else {
            let data = rest.split_once("  ").map_or(rest, |(data, _)| data);
            (data, Glyphs::Unknown)
        };

        let row_bytes = parse_data_tokens(data_zone, &glyphs);
        if row_bytes.is_empty() {
            // In `hexdump -C`, a trailing `*` elision is followed by an offset-only EOF line.
            if pending_star
                && rest.is_empty()
                && let Some(base) = base_offset
                && row_offset >= base
            {
                let rel_target = (row_offset - base) as usize;
                while out.len() < rel_target {
                    if !last_row_bytes.is_empty()
                        && (rel_target - out.len()).is_multiple_of(last_row_bytes.len())
                    {
                        out.extend_from_slice(&last_row_bytes);
                    } else {
                        out.push(0x00);
                    }
                }
                pending_star = false;
            }
            continue;
        }
        let had_star = pending_star;
        pending_star = false;

        let base = *base_offset.get_or_insert(row_offset);
        if row_offset >= base {
            let rel_target = (row_offset - base) as usize;
            while out.len() < rel_target {
                if had_star
                    && !last_row_bytes.is_empty()
                    && (rel_target - out.len()).is_multiple_of(last_row_bytes.len())
                {
                    out.extend_from_slice(&last_row_bytes);
                } else {
                    out.push(0x00);
                }
            }
            if rel_target < out.len() {
                for (i, &b) in row_bytes.iter().enumerate() {
                    if rel_target + i < out.len() {
                        out[rel_target + i] = b;
                    } else {
                        out.push(b);
                    }
                }
            } else {
                out.extend_from_slice(&row_bytes);
            }
        } else {
            out.extend_from_slice(&row_bytes);
        }
        last_row_bytes = row_bytes;
    }

    Ok(out)
}

fn parse_elided_row_count(rest: &str) -> Option<usize> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    for window in tokens.windows(3) {
        if window[1] == "identical" && window[2].starts_with("row") {
            return window[0].parse::<usize>().ok();
        }
    }
    None
}

/// The glyph column of a row, which says how many bytes the row holds.
enum Glyphs<'a> {
    /// The pane between `│` rails: one glyph per byte, padded with spaces after the last.
    Octet(&'a str),
    /// The row after the `xxd` address colon; it ends with the printable-or-`.` ASCII column.
    Xxd(&'a str),
    Unknown,
}

fn parse_data_tokens(data_zone: &str, glyphs: &Glyphs<'_>) -> Vec<u8> {
    let tokens: Vec<&str> = data_zone
        .split_whitespace()
        .filter(|&t| t != "⋮" && t != "│")
        .collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let all_bit_tokens = tokens
        .iter()
        .all(|t| t.len() == 8 && t.chars().all(|c| c == '0' || c == '1'));
    if all_bit_tokens {
        let bits: Vec<u8> = tokens
            .iter()
            .filter_map(|t| u8::from_str_radix(t, 2).ok())
            .collect();
        if !glyphs_contradict_bits(&bits, &tokens, glyphs) {
            return bits;
        }
    }

    parse_hex_tokens(&tokens)
}

/// Decides whether 8-character `0`/`1` tokens are `-g4` hex groups rather than `-b` bits.
///
/// A bit row holds one byte per token and a hex row four, so the glyph column tells them apart.
/// Without a usable glyph column the row is read as bits, which is what `-b` produces.
fn glyphs_contradict_bits(bits: &[u8], tokens: &[&str], glyphs: &Glyphs<'_>) -> bool {
    match glyphs {
        Glyphs::Octet(pane) => pane.trim_end().chars().count() != bits.len(),
        Glyphs::Xxd(row) => {
            // Requiring a space before the column stops a short bit rendering from matching
            // the tail of a longer hex rendering made only of `.`.
            let ends_with_column = |bytes: &[u8]| {
                let column: String = bytes
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7f).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                row.strip_suffix(column.as_str())
                    .is_some_and(|head| head.ends_with(' '))
            };
            !ends_with_column(bits) && ends_with_column(&parse_hex_tokens(tokens))
        }
        Glyphs::Unknown => false,
    }
}

fn parse_hex_tokens(tokens: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &raw_token in tokens {
        let token = raw_token
            .strip_prefix("0x")
            .or_else(|| raw_token.strip_prefix("0X"))
            .unwrap_or(raw_token);
        if !token.is_empty()
            && token.chars().all(|c| c.is_ascii_hexdigit())
            && token.len().is_multiple_of(2)
        {
            for i in (0..token.len()).step_by(2) {
                if let Ok(b) = u8::from_str_radix(&token[i..i + 2], 16) {
                    bytes.push(b);
                }
            }
        } else {
            break;
        }
    }
    bytes
}

/// Strips ANSI CSI escape sequences (`\x1b[...m`) from a string.
#[must_use]
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}
