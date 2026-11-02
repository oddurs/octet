// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! `octet` — Information-design-first hex viewer, binary inspector, and `xxd`/`hexdump`
//! replacement.
//!
//! This crate provides both the core library primitives and the backing engine for the `octet`
//! command-line utility:
//! - Semantic byte classification and Shannon entropy analysis ([`classify`])
//! - Calibrated dark/light design system palettes ([`theme`])
//! - Automatic file header recognition and byte-span field annotations ([`magic`])
//! - Multi-radix terminal formatting with zero-run elision ([`mod@format`])
//! - Multi-language byte array code generation ([`codegen`])
//! - Round-trip reverse hex-to-binary conversion ([`reverse`])
//! - Side-by-side binary diffing ([`diff`])
//! - Multi-type value inspection and pattern search ([`inspect`])
//! - Grid-locked vector terminal SVG rendering ([`svg`])

pub mod classify;
pub mod codegen;
pub mod diff;
pub mod error;
pub mod format;
pub mod inspect;
pub mod magic;
pub mod reverse;
pub mod svg;
pub mod theme;

pub use crate::classify::{ByteClass, ByteStats, classify_slice, compute_entropy};
pub use crate::codegen::{CodeLanguage, generate_code};
pub use crate::diff::{DiffReport, DiffRow, compute_diff, render_diff_ansi, render_diff_json};
pub use crate::error::{Error, Result};
pub use crate::format::{
    AnalyzedBuffer, DisplayRow, FormatOptions, Radix, ViewStyle, analyze_buffer, render_dump,
    render_json_report,
};
pub use crate::inspect::{ValueInspection, find_pattern_spans, parse_search_pattern};
pub use crate::magic::{FieldAnnotation, HeaderInspection, inspect_header};
pub use crate::reverse::{reverse_dump, strip_ansi};
pub use crate::svg::render_ansi_to_svg;
pub use crate::theme::{ColorMode, Rgb, Theme, ThemeVariant};

/// Parses a human-friendly byte count or offset string.
///
/// Supports decimal (`256`), hexadecimal (`0x100`), octal (`0o400`), binary (`0b1010`),
/// and unit suffixes (`k`/`kb`/`kib` = 1024, `m`/`mb`/`mib` = 1024²).
pub fn parse_byte_count(raw: &str) -> Result<u64> {
    let s = raw.trim().replace('_', "");
    if s.is_empty() {
        return Err(Error::InvalidOffset(
            "empty offset/length string".to_owned(),
        ));
    }
    let s = s.strip_prefix('+').unwrap_or(&s);

    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16)
            .map_err(|_| Error::InvalidOffset(format!("invalid hex number '{raw}'")));
    }
    if let Some(oct) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
        return u64::from_str_radix(oct, 8)
            .map_err(|_| Error::InvalidOffset(format!("invalid octal number '{raw}'")));
    }
    if let Some(bin) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        return u64::from_str_radix(bin, 2)
            .map_err(|_| Error::InvalidOffset(format!("invalid binary number '{raw}'")));
    }

    let lower = s.to_ascii_lowercase();
    let (num_part, multiplier) = if let Some(n) = lower
        .strip_suffix("tib")
        .or_else(|| lower.strip_suffix("tb"))
        .or_else(|| lower.strip_suffix('t'))
    {
        (n, 1024_u64 * 1024 * 1024 * 1024)
    } else if let Some(n) = lower
        .strip_suffix("gib")
        .or_else(|| lower.strip_suffix("gb"))
        .or_else(|| lower.strip_suffix('g'))
    {
        (n, 1024_u64 * 1024 * 1024)
    } else if let Some(n) = lower
        .strip_suffix("mib")
        .or_else(|| lower.strip_suffix("mb"))
        .or_else(|| lower.strip_suffix('m'))
    {
        (n, 1024_u64 * 1024)
    } else if let Some(n) = lower
        .strip_suffix("kib")
        .or_else(|| lower.strip_suffix("kb"))
        .or_else(|| lower.strip_suffix('k'))
    {
        (n, 1024_u64)
    } else if let Some(n) = lower.strip_suffix('b') {
        (n, 1_u64)
    } else {
        (lower.as_str(), 1_u64)
    };

    let base: u64 = num_part
        .parse()
        .map_err(|_| Error::InvalidOffset(format!("invalid numeric value '{raw}'")))?;
    base.checked_mul(multiplier)
        .ok_or_else(|| Error::InvalidOffset(format!("numeric overflow in '{raw}'")))
}

/// Resolves a seek specification (`+offset`, `offset`, or `-offset` from end of buffer) and
/// optional length limit into a subslice and its starting offset.
pub fn slice_buffer<'a>(
    data: &'a [u8],
    seek_spec: Option<&str>,
    length: Option<u64>,
) -> Result<(&'a [u8], u64)> {
    let start_idx = if let Some(spec) = seek_spec {
        let trimmed = spec.trim();
        if let Some(neg) = trimmed.strip_prefix('-') {
            let back = usize::try_from(parse_byte_count(neg)?).unwrap_or(usize::MAX);
            data.len().saturating_sub(back)
        } else {
            let pos = usize::try_from(parse_byte_count(trimmed)?).unwrap_or(usize::MAX);
            pos.min(data.len())
        }
    } else {
        0
    };

    let remaining = &data[start_idx..];
    let sliced = if let Some(len) = length {
        let take = usize::try_from(len)
            .unwrap_or(usize::MAX)
            .min(remaining.len());
        &remaining[..take]
    } else {
        remaining
    };
    Ok((sliced, start_idx as u64))
}
