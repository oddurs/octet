// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Design-system color tokens, palettes, and ANSI/SVG styling primitives.

use std::fmt::Write as _;

use crate::classify::ByteClass;

/// RGB color triplet used across TrueColor ANSI and SVG rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Formats the color as a CSS/SVG hexadecimal string (`#rrggbb`).
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Formats the color as an ANSI TrueColor foreground escape sequence.
    #[must_use]
    pub fn ansi_fg(self) -> String {
        format!("\x1b[38;2;{};{};{}m", self.0, self.1, self.2)
    }

    /// Formats the color as an ANSI TrueColor background escape sequence.
    #[must_use]
    pub fn ansi_bg(self) -> String {
        format!("\x1b[48;2;{};{};{}m", self.0, self.1, self.2)
    }
}

/// Terminal color capability mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Automatically detect terminal capabilities and `NO_COLOR`.
    Auto,
    /// Force 24-bit TrueColor ANSI output.
    TrueColor,
    /// Disable all ANSI color sequences.
    Never,
}

impl ColorMode {
    /// Resolves whether color output should be active for stdout.
    #[must_use]
    pub fn is_enabled(self, is_tty: bool) -> bool {
        match self {
            Self::TrueColor => true,
            Self::Never => false,
            Self::Auto => {
                if std::env::var_os("NO_COLOR").is_some() {
                    return false;
                }
                if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
                    return false;
                }
                is_tty || std::env::var_os("FORCE_COLOR").is_some()
            }
        }
    }
}

/// Visual theme preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeVariant {
    /// Deep carbon/obsidian dark palette (default).
    Dark,
    /// High-contrast daylight paper palette.
    Light,
}

/// Complete set of semantic design tokens for `octet`.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Theme variant identifier.
    pub variant: ThemeVariant,
    /// Primary canvas background (used in SVG export and badges).
    pub bg_canvas: Rgb,
    /// Elevated card/header background.
    pub bg_surface: Rgb,
    /// Subtle structural border / divider color.
    pub border: Rgb,
    /// Primary foreground text color.
    pub fg_primary: Rgb,
    /// Secondary label text color.
    pub fg_secondary: Rgb,
    /// Ultra-muted structural chrome color (leading zeros, dots, frame lines).
    pub fg_muted: Rgb,
    /// Significant digits of the byte offset gutter.
    pub offset_active: Rgb,
    /// Color for `ByteClass::Null` (`0x00`).
    pub byte_null: Rgb,
    /// Color for `ByteClass::AsciiGraphic` (`0x21..=0x7E`).
    pub byte_ascii: Rgb,
    /// Color for `ByteClass::AsciiWhitespace`.
    pub byte_whitespace: Rgb,
    /// Color for `ByteClass::AsciiControl`.
    pub byte_control: Rgb,
    /// Color for `ByteClass::Utf8MultiByte`.
    pub byte_utf8: Rgb,
    /// Color for `ByteClass::HighByte`.
    pub byte_high: Rgb,
    /// Color for `ByteClass::Sentinel` (`0xFF`).
    pub byte_sentinel: Rgb,
    /// Search match highlight background.
    pub search_bg: Rgb,
    /// Search match highlight foreground.
    pub search_fg: Rgb,
    /// Diff byte added / right side delta.
    pub diff_add: Rgb,
    /// Diff byte removed / left side delta.
    pub diff_del: Rgb,
    /// Rotating accent palette for structural header field annotations.
    pub annotation_accents: [Rgb; 6],
}

impl Theme {
    /// Returns the default dark obsidian design system theme.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            variant: ThemeVariant::Dark,
            bg_canvas: Rgb(13, 17, 23),         // #0d1117
            bg_surface: Rgb(22, 27, 34),        // #161b22
            border: Rgb(48, 54, 61),            // #30363d
            fg_primary: Rgb(230, 237, 243),     // #e6edf3
            fg_secondary: Rgb(139, 148, 158),   // #8b949e
            fg_muted: Rgb(72, 79, 88),          // #484f58
            offset_active: Rgb(121, 192, 255),  // #79c0ff
            byte_null: Rgb(68, 76, 86),         // #444c56 (recedes into background)
            byte_ascii: Rgb(86, 211, 100),      // #56d364 (crisp emerald)
            byte_whitespace: Rgb(88, 166, 255), // #58a6ff (sky blue)
            byte_control: Rgb(227, 179, 65),    // #e3b341 (warm amber)
            byte_utf8: Rgb(210, 168, 255),      // #d2a8ff (soft lavender)
            byte_high: Rgb(57, 197, 207),       // #39c5cf (electric teal)
            byte_sentinel: Rgb(255, 123, 114),  // #ff7b72 (coral red-orange)
            search_bg: Rgb(187, 128, 9),        // #bb8009
            search_fg: Rgb(255, 255, 255),      // #ffffff
            diff_add: Rgb(63, 185, 80),         // #3fb950
            diff_del: Rgb(248, 81, 73),         // #f85149
            annotation_accents: [
                Rgb(121, 192, 255), // Sky Blue
                Rgb(210, 168, 255), // Lavender
                Rgb(255, 166, 87),  // Tangerine
                Rgb(86, 211, 100),  // Emerald
                Rgb(255, 123, 114), // Coral
                Rgb(57, 197, 207),  // Cyan
            ],
        }
    }

    /// Returns the daylight paper design system theme.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            variant: ThemeVariant::Light,
            bg_canvas: Rgb(255, 255, 255),     // #ffffff
            bg_surface: Rgb(246, 248, 250),    // #f6f8fa
            border: Rgb(208, 215, 222),        // #d0d7de
            fg_primary: Rgb(31, 35, 40),       // #1f2328
            fg_secondary: Rgb(89, 99, 110),    // #59636e
            fg_muted: Rgb(175, 184, 193),      // #afb8c1
            offset_active: Rgb(9, 105, 218),   // #0969da
            byte_null: Rgb(185, 194, 204),     // #b9c2cc
            byte_ascii: Rgb(17, 99, 41),       // #116329
            byte_whitespace: Rgb(9, 105, 218), // #0969da
            byte_control: Rgb(154, 103, 0),    // #9a6700
            byte_utf8: Rgb(130, 80, 223),      // #8250df
            byte_high: Rgb(13, 122, 138),      // #0d7a8a
            byte_sentinel: Rgb(207, 34, 46),   // #cf222e
            search_bg: Rgb(255, 248, 197),     // #fff8c5
            search_fg: Rgb(31, 35, 40),        // #1f2328
            diff_add: Rgb(26, 127, 55),        // #1a7f37
            diff_del: Rgb(207, 34, 46),        // #cf222e
            annotation_accents: [
                Rgb(9, 105, 218),
                Rgb(130, 80, 223),
                Rgb(188, 76, 0),
                Rgb(26, 127, 55),
                Rgb(207, 34, 46),
                Rgb(13, 122, 138),
            ],
        }
    }

    /// Returns the semantic foreground [`Rgb`] token for a given [`ByteClass`].
    #[must_use]
    pub const fn class_color(&self, class: ByteClass) -> Rgb {
        match class {
            ByteClass::Null => self.byte_null,
            ByteClass::AsciiGraphic => self.byte_ascii,
            ByteClass::AsciiWhitespace => self.byte_whitespace,
            ByteClass::AsciiControl => self.byte_control,
            ByteClass::Utf8MultiByte => self.byte_utf8,
            ByteClass::HighByte => self.byte_high,
            ByteClass::Sentinel => self.byte_sentinel,
        }
    }

    /// Returns the rotating accent color for an annotation index.
    #[must_use]
    pub const fn accent(&self, index: usize) -> Rgb {
        self.annotation_accents[index % self.annotation_accents.len()]
    }

    /// Formats a text fragment with an optional foreground color.
    #[must_use]
    pub fn paint(&self, text: &str, color: Rgb, enabled: bool) -> String {
        if !enabled {
            return text.to_owned();
        }
        format!("{}{text}\x1b[0m", color.ansi_fg())
    }

    /// Formats a bold text fragment with an optional foreground color.
    #[must_use]
    pub fn paint_bold(&self, text: &str, color: Rgb, enabled: bool) -> String {
        if !enabled {
            return text.to_owned();
        }
        format!("\x1b[1m{}{text}\x1b[0m", color.ansi_fg())
    }

    /// Formats a 64-bit or 32-bit hex offset with muted leading zeros and highlighted significant
    /// digits.
    #[must_use]
    pub fn format_offset(&self, offset: u64, width: usize, upper: bool, color: bool) -> String {
        let raw = if upper {
            format!("{offset:0width$X}")
        } else {
            format!("{offset:0width$x}")
        };
        if !color {
            return raw;
        }
        // Split into leading zeros and significant suffix (keep at least last digit active).
        let trimmed = raw.trim_start_matches('0');
        let sig_len = trimmed.len().max(1);
        let split_idx = raw.len() - sig_len;
        let (leading, active) = raw.split_at(split_idx);
        let mut out = String::with_capacity(raw.len() + 32);
        if !leading.is_empty() {
            let _ = write!(out, "{}{leading}", self.fg_muted.ansi_fg());
        }
        let _ = write!(out, "{}{active}\x1b[0m", self.offset_active.ansi_fg());
        out
    }
}
