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

    for line in clean.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
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
            continue;
        }

        // Isolate the data column before the glyph pane (`│` in octet, or two spaces in xxd).
        let data_zone = if let Some((before_pipe, _)) = rest.split_once('│') {
            before_pipe
        } else if let Some((before_gap, _)) = rest.split_once("  ") {
            before_gap
        } else {
            rest
        };

        let row_bytes = parse_data_tokens(data_zone);
        if row_bytes.is_empty() {
            continue;
        }

        let base = *base_offset.get_or_insert(row_offset);
        if row_offset >= base {
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

fn parse_data_tokens(data_zone: &str) -> Vec<u8> {
    let tokens: Vec<&str> = data_zone
        .split_whitespace()
        .filter(|&t| t != "⋮" && t != "│")
        .collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    // Check if all tokens are 8-bit binary strings (`00` / `01` etc. of length 8).
    let is_binary_row = tokens
        .iter()
        .all(|t| t.len() == 8 && t.chars().all(|c| c == '0' || c == '1'));
    if is_binary_row {
        return tokens
            .iter()
            .filter_map(|t| u8::from_str_radix(t, 2).ok())
            .collect();
    }

    let mut bytes = Vec::new();
    for token in tokens {
        if token.chars().all(|c| c.is_ascii_hexdigit()) && token.len().is_multiple_of(2) {
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
