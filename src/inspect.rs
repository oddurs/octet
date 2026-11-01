// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Multi-representation byte inspector and pattern search utilities.

use std::fmt::Write as _;
use std::ops::Range;

use crate::error::{Error, Result};
use crate::theme::Theme;

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
        let utf8_char = decode_first_utf8_char(slice);

        Some(Self {
            offset: display_offset + index as u64,
            raw_bytes,
            int8,
            int16,
            int32,
            int64,
            float32,
            float64,
            uleb128,
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
        if let Some(((ule, _ile), (ube, _ibe))) = self.int64 {
            let line = format!(
                "   {:<16} : {:<26} {:<16} : 0x{ube:016x}",
                "u64 (LE)",
                format!("{ule}"),
                "u64 (BE hex)",
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
        if let Some((val, len)) = self.uleb128 {
            let utf8_info = match self.utf8_char {
                Some((ch, clen)) => format!("U+{:04X} {:?} ({clen} B)", ch as u32, ch),
                None => String::from("Invalid UTF-8 lead"),
            };
            let line = format!(
                "   {:<16} : {:<26} {:<16} : {}",
                "ULEB128 Varint",
                format!("{val} ({len} B)"),
                "UTF-8 Codepoint",
                utf8_info
            );
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
    let count = text.chars().count();
    let padded = if count >= inner_w {
        text.chars().take(inner_w).collect::<String>()
    } else {
        format!("{text}{}", " ".repeat(inner_w - count))
    };
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

fn decode_uleb128(slice: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    for (i, &byte) in slice.iter().take(10).enumerate() {
        let low = u64::from(byte & 0x7f);
        result |= low.checked_shl(shift)?;
        if (byte & 0x80) == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
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
        let clean: String = hex_str.chars().filter(|c| !c.is_whitespace()).collect();
        if clean.is_empty() || !clean.len().is_multiple_of(2) {
            return Err(Error::InvalidArgument(format!(
                "hex search pattern '{query}' must have an even number of hex digits"
            )));
        }
        let mut bytes = Vec::with_capacity(clean.len() / 2);
        for i in (0..clean.len()).step_by(2) {
            let b = u8::from_str_radix(&clean[i..i + 2], 16).map_err(|_| {
                Error::InvalidArgument(format!(
                    "invalid hex byte '{}' in search pattern",
                    &clean[i..i + 2]
                ))
            })?;
            bytes.push(b);
        }
        Ok(bytes)
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
