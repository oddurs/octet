// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Core hex/radix table layout engine and information-design terminal renderer.

use std::fmt::Write as _;
use std::ops::Range;

use crate::classify::{ByteClass, ByteStats, classify_slice};
use crate::magic::{HeaderInspection, inspect_header};
use crate::theme::Theme;

/// Number base / representation for the primary data pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radix {
    /// Standard hexadecimal bytes (`00`..`ff`).
    Hex,
    /// 8-bit binary digits (`00000000`..`11111111`, `xxd -b`).
    Binary,
    /// 3-digit octal bytes (`000`..`377`, `hexdump -b`).
    Octal,
    /// 5-digit unsigned decimal bytes (`  0`..`255`).
    Decimal,
}

/// Visual density and compatibility profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewStyle {
    /// Full `octet` information-design view (telemetry header, semantic glyphs, annotations).
    Smart,
    /// Compact `octet` table only (semantic colors and glyphs, no banner/annotation cards).
    Compact,
    /// Strict `xxd`-compatible plain ASCII layout (`00000000: 4865 ...  Hello`).
    Classic,
    /// Canonical BSD/GNU `hexdump -C` layout (`00000000  48 65 ...  |Hello|`).
    Canonical,
    /// Continuous unformatted hex stream (`xxd -p`).
    Plain,
}

/// Configuration for formatting a byte slice.
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Label or filename displayed in the header banner.
    pub label: Option<String>,
    /// Visual style profile.
    pub style: ViewStyle,
    /// Number base for the data column.
    pub radix: Radix,
    /// Number of bytes per row (`-c`).
    pub cols: usize,
    /// Number of bytes per space-separated group (`-g`, `0` means group entire row).
    pub group_size: usize,
    /// Format groups in little-endian byte order (`-e`).
    pub little_endian: bool,
    /// Use uppercase hexadecimal digits (`-u`).
    pub upper_hex: bool,
    /// Collapse consecutive identical rows (`-a`).
    pub autoskip: bool,
    /// Base offset added to displayed row addresses (`-o`).
    pub display_offset: u64,
    /// Inspect and annotate known file headers.
    pub annotate_headers: bool,
    /// Highlight byte ranges matching a search query (`-S`).
    pub search_spans: Vec<Range<usize>>,
    /// Active design system theme.
    pub theme: Theme,
    /// Emit ANSI escape sequences.
    pub color: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            label: None,
            style: ViewStyle::Smart,
            radix: Radix::Hex,
            cols: 16,
            group_size: 2,
            little_endian: false,
            upper_hex: false,
            autoskip: true,
            display_offset: 0,
            annotate_headers: true,
            search_spans: Vec::new(),
            theme: Theme::dark(),
            color: true,
        }
    }
}

/// Structured analysis and layout model used by both the terminal and SVG renderers.
#[derive(Debug, Clone)]
pub struct AnalyzedBuffer {
    /// Byte class per byte in the slice.
    pub classes: Vec<ByteClass>,
    /// Statistical summary and entropy metrics.
    pub stats: ByteStats,
    /// Optional recognized file header and structural field spans.
    pub header: Option<HeaderInspection>,
    /// Pre-grouped display rows (including elided runs).
    pub rows: Vec<DisplayRow>,
}

/// A single row in the formatted output table.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    /// A normal row of bytes at `offset`.
    Data {
        /// Displayed byte offset (`display_offset + slice_index`).
        offset: u64,
        /// Starting index into the analyzed buffer.
        start_index: usize,
        /// Slice of bytes in this row.
        bytes: Vec<u8>,
    },
    /// An elided run of identical rows (`autoskip`).
    Elided {
        /// Displayed starting offset of the elided range.
        start_offset: u64,
        /// Displayed ending offset (inclusive) of the elided range.
        end_offset: u64,
        /// Number of rows skipped.
        row_count: usize,
        /// Total bytes skipped.
        byte_count: usize,
        /// Representative byte if all bytes in the skipped rows are identical (e.g., `Some(0x00)`).
        uniform_byte: Option<u8>,
    },
}

/// Analyzes `data` according to `opts` and builds an [`AnalyzedBuffer`].
#[must_use]
pub fn analyze_buffer(data: &[u8], opts: &FormatOptions) -> AnalyzedBuffer {
    let classes = classify_slice(data);
    let stats = ByteStats::analyze(data, &classes);
    let header = if opts.annotate_headers && opts.display_offset == 0 {
        inspect_header(data)
    } else {
        None
    };

    let cols = opts.cols.max(1);
    let raw_chunks: Vec<(usize, &[u8])> = data
        .chunks(cols)
        .enumerate()
        .map(|(idx, slice)| (idx * cols, slice))
        .collect();

    let mut rows = Vec::new();
    let mut i = 0;
    while i < raw_chunks.len() {
        let (start_idx, chunk) = raw_chunks[i];
        rows.push(DisplayRow::Data {
            offset: opts.display_offset.saturating_add(start_idx as u64),
            start_index: start_idx,
            bytes: chunk.to_vec(),
        });

        let can_autoskip = opts.autoskip
            && chunk.len() == cols
            && (opts.style != ViewStyle::Classic || chunk.iter().all(|&b| b == 0));
        if can_autoskip {
            let mut repeat_end = i + 1;
            while repeat_end < raw_chunks.len() && raw_chunks[repeat_end].1 == chunk {
                repeat_end += 1;
            }
            let elide_end = if opts.style == ViewStyle::Classic && repeat_end == raw_chunks.len() {
                repeat_end.saturating_sub(1)
            } else {
                repeat_end
            };
            let duplicate_count = elide_end.saturating_sub(i + 1);
            let min_duplicates = if opts.style == ViewStyle::Canonical {
                1
            } else {
                2
            };
            if duplicate_count >= min_duplicates {
                let first_dup_idx = raw_chunks[i + 1].0;
                let last_dup_idx =
                    raw_chunks[elide_end - 1].0 + raw_chunks[elide_end - 1].1.len() - 1;
                let uniform_byte = if chunk.iter().all(|&b| b == chunk[0]) {
                    Some(chunk[0])
                } else {
                    None
                };
                rows.push(DisplayRow::Elided {
                    start_offset: opts.display_offset.saturating_add(first_dup_idx as u64),
                    end_offset: opts.display_offset.saturating_add(last_dup_idx as u64),
                    row_count: duplicate_count,
                    byte_count: duplicate_count * cols,
                    uniform_byte,
                });
                i = elide_end;
                continue;
            }
        }
        i += 1;
    }

    AnalyzedBuffer {
        classes,
        stats,
        header,
        rows,
    }
}

/// Renders `data` into a formatted string according to `opts`.
#[must_use]
pub fn render_dump(data: &[u8], opts: &FormatOptions) -> String {
    if opts.style == ViewStyle::Plain {
        return render_plain(data, opts.cols, opts.upper_hex);
    }

    let analyzed = analyze_buffer(data, opts);
    let off_w = compute_offset_width(data.len(), opts.display_offset);
    let card_width = compute_card_width(opts, off_w);
    let mut out = String::with_capacity(analyzed.rows.len() * (card_width + 64) + 1024);

    if opts.style == ViewStyle::Smart {
        render_header_banner(&mut out, &analyzed, opts, card_width, off_w);
    }

    if data.is_empty() {
        if opts.style == ViewStyle::Classic {
            return String::new();
        }
        if opts.style == ViewStyle::Canonical {
            return if opts.display_offset > 0 {
                let off_str =
                    opts.theme
                        .format_offset(opts.display_offset, off_w, false, opts.color);
                format!("{off_str}\n")
            } else {
                String::new()
            };
        }
        let msg = opts
            .theme
            .paint("  (empty input: 0 bytes)", opts.theme.fg_muted, opts.color);
        let _ = writeln!(out, "{msg}");
        return out;
    }

    for row in &analyzed.rows {
        match row {
            DisplayRow::Data {
                offset,
                start_index,
                bytes,
            } => {
                render_data_row(
                    &mut out,
                    *offset,
                    *start_index,
                    bytes,
                    &analyzed,
                    opts,
                    off_w,
                );
            }
            DisplayRow::Elided {
                start_offset,
                end_offset,
                row_count,
                byte_count,
                uniform_byte,
            } => {
                if matches!(opts.style, ViewStyle::Classic | ViewStyle::Canonical) {
                    let _ = writeln!(out, "*");
                } else {
                    let off_str = opts
                        .theme
                        .format_offset(*start_offset, off_w, false, opts.color);
                    let kind = match uniform_byte {
                        Some(0x00) => String::from("of 0x00 "),
                        Some(b) => format!("of 0x{b:02x} "),
                        None => String::new(),
                    };
                    let msg = format!(
                        "  ⋮   ⋯ {row_count} identical rows ({byte_count} bytes {kind}from 0x{start_offset:04x}..0x{end_offset:04x}) elided ⋯"
                    );
                    if opts.style == ViewStyle::Smart {
                        let padded_msg = pad_or_clip(&msg, card_width.saturating_sub(off_w + 1));
                        let _ = writeln!(
                            out,
                            "{off_str}{}{}",
                            opts.theme
                                .paint(&padded_msg, opts.theme.fg_muted, opts.color),
                            opts.theme.paint("│", opts.theme.border, opts.color)
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "{off_str}{}",
                            opts.theme.paint(&msg, opts.theme.fg_muted, opts.color)
                        );
                    }
                }
            }
        }
    }

    if opts.style == ViewStyle::Canonical {
        let end_off = opts.display_offset.saturating_add(data.len() as u64);
        let off_str = opts.theme.format_offset(end_off, off_w, false, opts.color);
        let _ = writeln!(out, "{off_str}");
    } else if opts.style == ViewStyle::Smart {
        if let Some(ref hdr) = analyzed.header {
            render_annotation_footer(&mut out, hdr, opts, card_width);
        } else {
            render_legend_footer(&mut out, &analyzed.stats, opts, card_width);
        }
    }

    out
}

fn render_plain(data: &[u8], cols: usize, upper: bool) -> String {
    if data.is_empty() {
        return if cols == 0 {
            String::from("\n")
        } else {
            String::new()
        };
    }
    let wrap = if cols == 0 { usize::MAX } else { cols };
    let mut out = String::with_capacity(data.len() * 2 + data.len() / wrap.max(1) + 2);
    for (i, &b) in data.iter().enumerate() {
        if upper {
            let _ = write!(out, "{b:02X}");
        } else {
            let _ = write!(out, "{b:02x}");
        }
        if wrap != usize::MAX && (i + 1) % wrap == 0 && i + 1 < data.len() {
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

fn effective_group_size(opts: &FormatOptions) -> usize {
    let cols = opts.cols.max(1);
    if opts.group_size == 0 {
        cols
    } else {
        opts.group_size
    }
}

fn compute_offset_width(data_len: usize, display_offset: u64) -> usize {
    let max_off = display_offset.saturating_add(data_len.saturating_sub(1) as u64);
    format!("{max_off:08x}").len()
}

fn compute_table_core_width(opts: &FormatOptions, off_w: usize) -> usize {
    let cols = opts.cols.max(1);
    let cell_w = match opts.radix {
        Radix::Hex => 2,
        Radix::Binary => 8,
        Radix::Octal => 3,
        Radix::Decimal => 3,
    };
    let group_size = effective_group_size(opts);
    let groups = cols.div_ceil(group_size);
    let spaces = groups.saturating_sub(1);
    let mid_extra =
        if cols >= 8 && opts.style != ViewStyle::Classic && (cols / 2).is_multiple_of(group_size) {
            2
        } else {
            0
        };
    let data_pane_w = cols * cell_w + spaces + mid_extra;
    off_w + 2 + data_pane_w + 2 + 1 + cols + 1
}

fn compute_card_width(opts: &FormatOptions, off_w: usize) -> usize {
    // Ensure smart cards have at least 106 columns (plus any extra width for 64-bit offsets)
    // so telemetry, inline rails, and header summaries form a unified grid.
    let min_smart_w = 106 + off_w.saturating_sub(8);
    compute_table_core_width(opts, off_w).max(min_smart_w)
}

fn render_header_banner(
    out: &mut String,
    analyzed: &AnalyzedBuffer,
    opts: &FormatOptions,
    width: usize,
    off_w: usize,
) {
    let inner_w = width.saturating_sub(2);
    let t = &opts.theme;
    let c = opts.color;
    let top_bar = format!("╭{}╮", "─".repeat(inner_w));

    let _ = writeln!(out, "{}", t.paint(&top_bar, t.border, c));

    let label = opts.label.as_deref().unwrap_or("stdin");
    let size_str = format!(
        "{} B (0x{:x})",
        analyzed.stats.total_bytes, analyzed.stats.total_bytes
    );
    let format_badge = match &analyzed.header {
        Some(hdr) => format!("{} · {}", hdr.format_name, hdr.summary),
        None => format!("Binary Stream · {}", analyzed.stats.entropy_profile()),
    };

    let line1_left = format!(" ◈ {label}  │  {size_str}  │  {format_badge}");
    let line1_string = pad_or_clip(&line1_left, inner_w);
    let _ = writeln!(
        out,
        "{}{}{}",
        t.paint("│", t.border, c),
        style_banner_line1(&line1_string, t, c),
        t.paint("│", t.border, c)
    );

    let spark = analyzed.stats.sparkline_string();
    let spark_chars = spark.chars().count();
    let bar_w = 12usize;
    let dist_bar = render_distribution_bar(&analyzed.stats, bar_w, t, c);
    let pct_summary = format_class_percentages(&analyzed.stats);
    let entropy_val_str = format!("{:4.2} b/B", analyzed.stats.entropy);
    let line2_plain_len = 9
        + entropy_val_str.chars().count()
        + 2
        + spark_chars
        + 15
        + bar_w
        + 3
        + pct_summary.chars().count();
    let pad_needed = inner_w.saturating_sub(line2_plain_len);
    let _ = writeln!(
        out,
        "{} {} {} [{}]  {}  Classes [{}]  {}{}{}",
        t.paint("│", t.border, c),
        t.paint("Entropy", t.fg_secondary, c),
        t.paint_bold(&entropy_val_str, t.fg_primary, c),
        t.paint(&spark, t.byte_high, c),
        t.paint("│", t.border, c),
        dist_bar,
        t.paint(&pct_summary, t.fg_secondary, c),
        " ".repeat(pad_needed),
        t.paint("│", t.border, c),
    );

    let core_w = compute_table_core_width(opts, off_w);
    let cols = opts.cols.max(1);
    let j1 = core_w.saturating_sub(cols + 3);
    let j2 = core_w.saturating_sub(2);
    let mid_bar = build_junction_bar('├', '┤', '┬', inner_w, &[j1, j2]);
    let ruler_bot_bar = build_junction_bar('╰', '╯', '┴', inner_w, &[j1, j2]);
    let _ = writeln!(out, "{}", t.paint(&mid_bar, t.border, c));
    render_column_ruler_row(out, opts, width, off_w);
    let _ = writeln!(out, "{}", t.paint(&ruler_bot_bar, t.border, c));
}

fn build_junction_bar(
    left: char,
    right: char,
    junction: char,
    inner_w: usize,
    positions: &[usize],
) -> String {
    let mut s = String::with_capacity(inner_w * 3 + 6);
    s.push(left);
    for idx in 0..inner_w {
        if positions.contains(&idx) {
            s.push(junction);
        } else {
            s.push('─');
        }
    }
    s.push(right);
    s
}

fn render_column_ruler_row(
    out: &mut String,
    opts: &FormatOptions,
    card_width: usize,
    off_w: usize,
) {
    let t = &opts.theme;
    let c = opts.color;
    let cols = opts.cols.max(1);
    let group_size = effective_group_size(opts);
    let mid_col = cols / 2;
    let show_mid_divider =
        cols >= 8 && opts.style != ViewStyle::Classic && mid_col.is_multiple_of(group_size);

    let off_hdr = format!(" {:<width$}", "OFFSET", width = off_w);
    let _ = write!(
        out,
        "{}{}",
        t.paint("│", t.border, c),
        t.paint_bold(&off_hdr, t.fg_muted, c)
    );

    if opts.little_endian && opts.radix == Radix::Hex && group_size > 1 {
        let groups = cols.div_ceil(group_size);
        for g in 0..groups {
            let g_start = g * group_size;
            let g_end = (g_start + group_size).min(cols);
            if g > 0 {
                if show_mid_divider && g_start == mid_col {
                    let _ = write!(out, " {} ", t.paint("⋮", t.border, c));
                } else {
                    out.push(' ');
                }
            }
            for col in (g_start..g_end).rev() {
                let tok = if opts.upper_hex {
                    format!("{col:02X}")
                } else {
                    format!("{col:02x}")
                };
                let _ = write!(out, "{}", t.paint(&tok, t.fg_muted, c));
            }
        }
    } else {
        for col in 0..cols {
            if col > 0 {
                if show_mid_divider && col == mid_col {
                    let _ = write!(out, " {} ", t.paint("⋮", t.border, c));
                } else if col % group_size == 0 {
                    out.push(' ');
                }
            }
            let tok = match opts.radix {
                Radix::Hex => {
                    if opts.upper_hex {
                        format!("{col:02X}")
                    } else {
                        format!("{col:02x}")
                    }
                }
                Radix::Binary => format!("  +{col:02x}   "),
                Radix::Octal | Radix::Decimal => format!("+{col:02x}"),
            };
            let _ = write!(out, "{}", t.paint(&tok, t.fg_muted, c));
        }
    }

    let _ = write!(out, "  {}", t.paint("│", t.border, c));
    let mut glyph_ruler = String::with_capacity(cols);
    for col in 0..cols {
        let ch = char::from_digit((col & 0xf) as u32, 16).unwrap_or('0');
        glyph_ruler.push(if opts.upper_hex {
            ch.to_ascii_uppercase()
        } else {
            ch
        });
    }
    let _ = write!(
        out,
        "{}{}",
        t.paint(&glyph_ruler, t.fg_muted, c),
        t.paint("│", t.border, c)
    );

    let core_w = compute_table_core_width(opts, off_w);
    if card_width > core_w + 4 {
        let rail_budget = card_width - core_w - 3;
        let padded = pad_or_clip("CONTEXT / ANNOTATIONS", rail_budget);
        let _ = write!(
            out,
            "  {}{}",
            t.paint_bold(&padded, t.fg_muted, c),
            t.paint("│", t.border, c)
        );
    }
    let _ = writeln!(out);
}

fn style_banner_line1(padded: &str, t: &Theme, color: bool) -> String {
    if !color {
        return padded.to_owned();
    }
    let parts: Vec<&str> = padded.splitn(3, '│').collect();
    if parts.len() == 3 {
        format!(
            "{}{}{}{}{}",
            t.paint_bold(parts[0], t.fg_primary, true),
            t.paint("│", t.border, true),
            t.paint(parts[1], t.offset_active, true),
            t.paint("│", t.border, true),
            t.paint_bold(parts[2], t.byte_ascii, true),
        )
    } else {
        t.paint_bold(padded, t.fg_primary, true)
    }
}

fn render_distribution_bar(stats: &ByteStats, width: usize, t: &Theme, color: bool) -> String {
    if stats.total_bytes == 0 {
        return t.paint(&"·".repeat(width), t.fg_muted, color);
    }
    let total = stats.total_bytes as f64;
    let segments = [
        (stats.ascii_count, t.byte_ascii, '█'),
        (stats.utf8_count, t.byte_utf8, '▓'),
        (stats.control_count, t.byte_control, '▒'),
        (stats.high_count, t.byte_high, '▒'),
        (stats.sentinel_count, t.byte_sentinel, '▪'),
        (stats.null_count, t.byte_null, '░'),
    ];
    let mut out = String::new();
    let mut used = 0usize;
    for (i, (count, col, ch)) in segments.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        let mut cells = (((*count as f64) / total) * (width as f64)).round() as usize;
        if cells == 0 && *count > 0 && used < width {
            cells = 1;
        }
        if i + 1 == segments.len() || used + cells > width {
            cells = width.saturating_sub(used);
        }
        if cells > 0 {
            let s: String = std::iter::repeat_n(*ch, cells).collect();
            let _ = write!(out, "{}", t.paint(&s, *col, color));
            used += cells;
        }
    }
    if used < width {
        let rem: String = std::iter::repeat_n('░', width - used).collect();
        let _ = write!(out, "{}", t.paint(&rem, t.byte_null, color));
    }
    out
}

fn format_class_percentages(stats: &ByteStats) -> String {
    if stats.total_bytes == 0 {
        return String::from("Empty");
    }
    let pct = |n: usize| ((n as f64 / stats.total_bytes as f64) * 100.0).round() as usize;
    format!(
        "ASCII {}% · Null {}% · High {}% · Ctrl {}%",
        pct(stats.ascii_count + stats.utf8_count),
        pct(stats.null_count),
        pct(stats.high_count + stats.sentinel_count),
        pct(stats.control_count),
    )
}

fn pad_or_clip(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count > width {
        if width >= 4 {
            let mut clipped: String = s.chars().take(width - 2).collect();
            clipped.push('…');
            clipped.push(' ');
            clipped
        } else {
            let mut clipped: String = s.chars().take(width.saturating_sub(1)).collect();
            clipped.push('…');
            clipped
        }
    } else {
        format!("{s}{}", " ".repeat(width - count))
    }
}

fn render_data_row(
    out: &mut String,
    offset: u64,
    start_index: usize,
    bytes: &[u8],
    analyzed: &AnalyzedBuffer,
    opts: &FormatOptions,
    off_w: usize,
) {
    let card_width = compute_card_width(opts, off_w);
    let t = &opts.theme;
    let c = opts.color;
    let cols = opts.cols.max(1);
    let group_size = effective_group_size(opts);
    let mid_col = cols / 2;
    let show_mid_divider = cols >= 8
        && !matches!(opts.style, ViewStyle::Classic | ViewStyle::Canonical)
        && mid_col.is_multiple_of(group_size);
    let canonical_mid_gap =
        cols >= 8 && opts.style == ViewStyle::Canonical && mid_col.is_multiple_of(group_size);

    let off_str = t.format_offset(offset, off_w, false, c);
    if opts.style == ViewStyle::Classic {
        let _ = write!(out, "{off_str}: ");
    } else {
        let _ = write!(out, "{off_str}  ");
    }

    // Format primary radix column
    if opts.little_endian && opts.radix == Radix::Hex && group_size > 1 {
        render_little_endian_hex(
            out,
            start_index,
            bytes,
            analyzed,
            opts,
            group_size,
            show_mid_divider,
        );
    } else {
        for col in 0..cols {
            if col > 0 {
                if show_mid_divider && col == mid_col {
                    let _ = write!(out, " {} ", t.paint("⋮", t.border, c));
                } else if canonical_mid_gap && col == mid_col {
                    let _ = write!(out, "  ");
                } else if col % group_size == 0 {
                    out.push(' ');
                }
            }
            if let Some(&byte) = bytes.get(col) {
                let abs_idx = start_index + col;
                let class = analyzed
                    .classes
                    .get(abs_idx)
                    .copied()
                    .unwrap_or_else(|| ByteClass::classify_single(byte));
                let tok = format_byte_radix(byte, opts.radix, opts.upper_hex);
                let styled = style_byte_token(
                    &tok,
                    abs_idx,
                    class,
                    analyzed.header.as_ref(),
                    &opts.search_spans,
                    t,
                    c,
                );
                let _ = write!(out, "{styled}");
            } else {
                let pad_w = match opts.radix {
                    Radix::Hex => 2,
                    Radix::Binary => 8,
                    Radix::Octal | Radix::Decimal => 3,
                };
                for _ in 0..pad_w {
                    out.push(' ');
                }
            }
        }
    }

    // Format right-hand glyph pane
    let rich_glyphs = !matches!(opts.style, ViewStyle::Classic | ViewStyle::Canonical);
    if opts.style == ViewStyle::Classic {
        let _ = write!(out, "  ");
    } else if opts.style == ViewStyle::Canonical {
        let _ = write!(out, "  {}", t.paint("|", t.border, c));
    } else {
        let _ = write!(out, "  {}", t.paint("│", t.border, c));
    }

    for (col, &byte) in bytes.iter().enumerate() {
        let abs_idx = start_index + col;
        let class = analyzed
            .classes
            .get(abs_idx)
            .copied()
            .unwrap_or_else(|| ByteClass::classify_single(byte));
        let ch = class.glyph(byte, rich_glyphs);
        let ch_str = ch.to_string();
        let styled = style_byte_token(
            &ch_str,
            abs_idx,
            class,
            analyzed.header.as_ref(),
            &opts.search_spans,
            t,
            c,
        );
        let _ = write!(out, "{styled}");
    }

    if opts.style == ViewStyle::Canonical {
        let _ = write!(out, "{}", t.paint("|", t.border, c));
    } else if opts.style != ViewStyle::Classic {
        for _ in bytes.len()..cols {
            out.push(' ');
        }
        let _ = write!(out, "{}", t.paint("│", t.border, c));
    }

    // In Smart mode, render the inline row context rail so the table aligns with the 106-col cards.
    if opts.style == ViewStyle::Smart {
        let core_w = compute_table_core_width(opts, off_w);
        if card_width > core_w + 4 {
            let rail_budget = card_width - core_w - 3; // "  " + content + "│"
            let (note_text, note_color, is_bold) =
                build_row_context_note(start_index, bytes, analyzed, opts);
            let padded_note = pad_or_clip(&note_text, rail_budget);
            let styled_note = if is_bold {
                t.paint_bold(&padded_note, note_color, c)
            } else {
                t.paint(&padded_note, note_color, c)
            };
            let _ = write!(out, "  {styled_note}{}", t.paint("│", t.border, c));
        }
    }

    let _ = writeln!(out);
}

fn build_row_context_note(
    start_index: usize,
    bytes: &[u8],
    analyzed: &AnalyzedBuffer,
    opts: &FormatOptions,
) -> (String, crate::theme::Rgb, bool) {
    let t = &opts.theme;
    let row_range = start_index..start_index + bytes.len();

    // 1. Search match in this row (using partition_point when sorted, with linear fallback)
    let first_cand = opts
        .search_spans
        .partition_point(|r| r.end <= row_range.start);
    if let Some(span) = opts.search_spans.get(first_cand)
        && span.start < row_range.end
        && span.end > row_range.start
    {
        let match_off = opts.display_offset.saturating_add(span.start as u64);
        return (format!("★ match @ 0x{match_off:02x}"), t.byte_control, true);
    }
    for span in &opts.search_spans {
        if span.start < row_range.end && span.end > row_range.start {
            let match_off = opts.display_offset.saturating_add(span.start as u64);
            return (format!("★ match @ 0x{match_off:02x}"), t.byte_control, true);
        }
    }

    // 2. Overlapping structural header annotations
    if let Some(ref hdr) = analyzed.header {
        let mut names = Vec::new();
        let mut primary_accent = None;
        for field in &hdr.fields {
            if field.range.start < row_range.end && field.range.end > row_range.start {
                if primary_accent.is_none() {
                    primary_accent = Some(t.accent(field.accent_index));
                }
                if !names.contains(&field.name) {
                    names.push(field.name);
                }
            }
        }
        if !names.is_empty() {
            let joined = names.into_iter().take(3).collect::<Vec<_>>().join(" · ");
            return (
                format!("◂ {joined}"),
                primary_accent.unwrap_or(t.offset_active),
                false,
            );
        }
    }

    // 3. Multi-byte UTF-8 graphic sequence in this row
    for (i, &b) in bytes.iter().enumerate() {
        let abs = start_index + i;
        if analyzed.classes.get(abs) == Some(&ByteClass::Utf8MultiByte)
            && b >= 0xc0
            && let Some(ch) = decode_utf8_at(&bytes[i..])
            && !ch.is_control()
            && !ch.is_whitespace()
        {
            return (
                format!("◈ UTF-8 {:?} (U+{:04X})", ch, ch as u32),
                t.byte_utf8,
                false,
            );
        }
    }

    // 4. All-zero row
    if !bytes.is_empty() && bytes.iter().all(|&b| b == 0) {
        return (String::from("· zero block (0x00)"), t.fg_muted, false);
    }

    // 5. Sentinel 0xFF run (>= 4 bytes)
    if bytes.windows(4).any(|w| w == [0xff, 0xff, 0xff, 0xff]) {
        return (
            String::from("× sentinel (0xffffffff)"),
            t.byte_sentinel,
            false,
        );
    }

    // 6. Contiguous printable ASCII run (>= 4 chars)
    if let Some(ascii_snippet) = extract_longest_ascii_run(bytes, 4) {
        return (format!("▸ \"{ascii_snippet}\""), t.byte_ascii, false);
    }

    // 7. High-byte / binary data summary
    let high_cnt = bytes.iter().filter(|&&b| b >= 0x80).count();
    if high_cnt * 2 >= bytes.len() && !bytes.is_empty() {
        return (String::from("▪ binary / high-entropy"), t.byte_high, false);
    }

    (String::new(), t.fg_muted, false)
}

fn decode_utf8_at(slice: &[u8]) -> Option<char> {
    for len in 2..=slice.len().min(4) {
        if let Ok(s) = std::str::from_utf8(&slice[..len])
            && let Some(ch) = s.chars().next()
        {
            return Some(ch);
        }
    }
    None
}

fn extract_longest_ascii_run(bytes: &[u8], min_len: usize) -> Option<String> {
    let mut best: &[u8] = &[];
    let mut start = None;
    for (i, &b) in bytes.iter().enumerate() {
        if (0x20..=0x7e).contains(&b) {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take()
            && i - s > best.len()
        {
            best = &bytes[s..i];
        }
    }
    if let Some(s) = start
        && bytes.len() - s > best.len()
    {
        best = &bytes[s..];
    }
    if best.len() >= min_len {
        Some(String::from_utf8_lossy(best).trim().to_owned())
    } else {
        None
    }
}

fn render_little_endian_hex(
    out: &mut String,
    start_index: usize,
    bytes: &[u8],
    analyzed: &AnalyzedBuffer,
    opts: &FormatOptions,
    group_size: usize,
    show_mid_divider: bool,
) {
    let cols = opts.cols.max(1);
    let mid_col = cols / 2;
    let groups = cols.div_ceil(group_size);
    for g in 0..groups {
        let g_start = g * group_size;
        let g_end = if opts.style == ViewStyle::Classic {
            g_start + group_size
        } else {
            (g_start + group_size).min(cols)
        };
        if g > 0 {
            if show_mid_divider && g_start == mid_col {
                let _ = write!(
                    out,
                    " {} ",
                    opts.theme.paint("⋮", opts.theme.border, opts.color)
                );
            } else {
                out.push(' ');
            }
        }
        for col in (g_start..g_end).rev() {
            if let Some(&byte) = bytes.get(col) {
                let abs_idx = start_index + col;
                let class = analyzed
                    .classes
                    .get(abs_idx)
                    .copied()
                    .unwrap_or_else(|| ByteClass::classify_single(byte));
                let tok = format_byte_radix(byte, Radix::Hex, opts.upper_hex);
                let styled = style_byte_token(
                    &tok,
                    abs_idx,
                    class,
                    analyzed.header.as_ref(),
                    &opts.search_spans,
                    &opts.theme,
                    opts.color,
                );
                let _ = write!(out, "{styled}");
            } else {
                let _ = write!(out, "  ");
            }
        }
    }
}

fn format_byte_radix(byte: u8, radix: Radix, upper: bool) -> String {
    match radix {
        Radix::Hex => {
            if upper {
                format!("{byte:02X}")
            } else {
                format!("{byte:02x}")
            }
        }
        Radix::Binary => format!("{byte:08b}"),
        Radix::Octal => format!("{byte:03o}"),
        Radix::Decimal => format!("{byte:3}"),
    }
}

fn style_byte_token(
    text: &str,
    abs_idx: usize,
    class: ByteClass,
    header: Option<&HeaderInspection>,
    search_spans: &[Range<usize>],
    theme: &Theme,
    color: bool,
) -> String {
    if !color {
        return text.to_owned();
    }
    // Search highlight takes visual priority (fast binary search when sorted).
    let cand_idx = search_spans.partition_point(|r| r.end <= abs_idx);
    let is_search_hit = search_spans
        .get(cand_idx)
        .is_some_and(|r| r.contains(&abs_idx))
        || (search_spans.len() <= 8 && search_spans.iter().any(|r| r.contains(&abs_idx)));
    if is_search_hit {
        return format!(
            "\x1b[1m{}{}{text}\x1b[0m",
            theme.search_bg.ansi_bg(),
            theme.search_fg.ansi_fg()
        );
    }
    // Check if byte belongs to an annotated header field.
    if let Some(hdr) = header
        && let Some(field) = hdr.field_at(abs_idx)
    {
        let accent = theme.accent(field.accent_index);
        return format!("\x1b[4m{}{text}\x1b[0m", accent.ansi_fg());
    }
    let fg = theme.class_color(class);
    format!("{}{text}\x1b[0m", fg.ansi_fg())
}

fn render_annotation_footer(
    out: &mut String,
    hdr: &HeaderInspection,
    opts: &FormatOptions,
    width: usize,
) {
    let t = &opts.theme;
    let c = opts.color;
    let inner_w = width.saturating_sub(2);
    let top = format!("╭{}╮", "─".repeat(inner_w));
    let bot = format!("╰{}╯", "─".repeat(inner_w));
    let _ = writeln!(out, "{}", t.paint(&top, t.border, c));

    let title = format!(
        " ◈ STRUCTURAL HEADER ANNOTATIONS · {} ({})",
        hdr.format_name, hdr.mime_or_category
    );
    let padded_title = pad_or_clip(&title, inner_w);
    let _ = writeln!(
        out,
        "{}{}{}",
        t.paint("│", t.border, c),
        t.paint_bold(&padded_title, t.fg_primary, c),
        t.paint("│", t.border, c)
    );

    for field in &hdr.fields {
        let accent = t.accent(field.accent_index);
        let range_str = format!("[0x{:02x}..0x{:02x}]", field.range.start, field.range.end);
        let name_padded = format!("{:<16}", field.name);
        let prefix_len = 3 + range_str.chars().count() + 1 + name_padded.chars().count() + 1;
        let val_budget = inner_w.saturating_sub(prefix_len);
        let val_padded = pad_or_clip(&field.value, val_budget);

        let _ = writeln!(
            out,
            "{} {} {} {} {}{}",
            t.paint("│", t.border, c),
            t.paint("■", accent, c),
            t.paint(&range_str, t.fg_muted, c),
            t.paint_bold(&name_padded, accent, c),
            t.paint(&val_padded, t.fg_primary, c),
            t.paint("│", t.border, c),
        );
    }
    let mid = format!("├{}┤", "─".repeat(inner_w));
    let _ = writeln!(out, "{}", t.paint(&mid, t.border, c));
    render_legend_row(out, opts, inner_w);
    let _ = writeln!(out, "{}", t.paint(&bot, t.border, c));
}

fn render_legend_footer(out: &mut String, _stats: &ByteStats, opts: &FormatOptions, width: usize) {
    let t = &opts.theme;
    let c = opts.color;
    let inner_w = width.saturating_sub(2);
    let top = format!("╭{}╮", "─".repeat(inner_w));
    let bot = format!("╰{}╯", "─".repeat(inner_w));
    let _ = writeln!(out, "{}", t.paint(&top, t.border, c));
    render_legend_row(out, opts, inner_w);
    let _ = writeln!(out, "{}", t.paint(&bot, t.border, c));
}

fn render_legend_row(out: &mut String, opts: &FormatOptions, inner_w: usize) {
    let t = &opts.theme;
    let c = opts.color;
    let items: &[(&str, crate::theme::Rgb, &str)] = &[
        ("·", t.byte_null, "Null (00)"),
        ("A", t.byte_ascii, "ASCII (21..7E)"),
        ("␣↵⇥", t.byte_whitespace, "Space"),
        ("␁␡", t.byte_control, "Control"),
        ("◈", t.byte_utf8, "UTF-8"),
        ("▪", t.byte_high, "High (80..FE)"),
        ("×", t.byte_sentinel, "Sentinel (FF)"),
    ];
    let mut plain_len = 1usize; // leading space
    let mut styled = String::from(" ");
    for (idx, &(glyph, color_rgb, label)) in items.iter().enumerate() {
        if idx > 0 {
            styled.push_str("   ");
            plain_len += 3;
        }
        let g_chars = glyph.chars().count();
        let l_chars = label.chars().count();
        if plain_len + g_chars + 1 + l_chars > inner_w {
            break;
        }
        let _ = write!(
            styled,
            "{} {}",
            t.paint_bold(glyph, color_rgb, c),
            t.paint(label, t.fg_muted, c)
        );
        plain_len += g_chars + 1 + l_chars;
    }
    let pad = inner_w.saturating_sub(plain_len);
    styled.push_str(&" ".repeat(pad));
    let _ = writeln!(
        out,
        "{}{styled}{}",
        t.paint("│", t.border, c),
        t.paint("│", t.border, c)
    );
}

/// Renders a structured RFC 8259 JSON analysis report for `data`.
#[must_use]
pub fn render_json_report(
    data: &[u8],
    opts: &FormatOptions,
    inspection: Option<&crate::inspect::ValueInspection>,
) -> String {
    let analyzed = analyze_buffer(data, opts);
    let st = &analyzed.stats;
    let label = opts.label.as_deref().unwrap_or("stdin");

    let mut out = String::from("{\n");
    let _ = writeln!(out, "  \"label\": \"{}\",", escape_json(label));
    let _ = writeln!(out, "  \"total_bytes\": {},", st.total_bytes);
    let _ = writeln!(out, "  \"display_offset\": {},", opts.display_offset);
    let _ = writeln!(out, "  \"entropy_bits_per_byte\": {:.4},", st.entropy);
    let _ = writeln!(
        out,
        "  \"entropy_profile\": \"{}\",",
        escape_json(st.entropy_profile())
    );
    let _ = writeln!(
        out,
        "  \"classes\": {{ \"null\": {}, \"ascii\": {}, \"control\": {}, \"utf8\": {}, \"high\": {}, \"sentinel\": {} }},",
        st.null_count,
        st.ascii_count,
        st.control_count,
        st.utf8_count,
        st.high_count,
        st.sentinel_count
    );

    if let Some(ref hdr) = analyzed.header {
        let _ = writeln!(out, "  \"header\": {{");
        let _ = writeln!(
            out,
            "    \"format_name\": \"{}\",",
            escape_json(hdr.format_name)
        );
        let _ = writeln!(
            out,
            "    \"mime_type\": \"{}\",",
            escape_json(hdr.mime_or_category)
        );
        let _ = writeln!(out, "    \"summary\": \"{}\",", escape_json(&hdr.summary));
        let _ = writeln!(out, "    \"fields\": [");
        for (i, f) in hdr.fields.iter().enumerate() {
            let comma = if i + 1 < hdr.fields.len() { "," } else { "" };
            let _ = writeln!(
                out,
                "      {{ \"name\": \"{}\", \"start\": {}, \"end\": {}, \"value\": \"{}\" }}{comma}",
                escape_json(f.name),
                f.range.start,
                f.range.end,
                escape_json(&f.value)
            );
        }
        let _ = writeln!(out, "    ]");
        let _ = writeln!(out, "  }},");
    } else {
        let _ = writeln!(out, "  \"header\": null,");
    }

    let match_offsets: Vec<String> = opts
        .search_spans
        .iter()
        .map(|r| (opts.display_offset.saturating_add(r.start as u64)).to_string())
        .collect();
    let _ = writeln!(out, "  \"search_matches\": [{}],", match_offsets.join(", "));

    if let Some(insp) = inspection {
        let _ = writeln!(out, "  \"inspection\": {{");
        let _ = writeln!(out, "    \"offset\": {},", insp.offset);
        if let Some((u, i)) = insp.int8 {
            let _ = writeln!(out, "    \"u8\": {u}, \"i8\": {i},");
        }
        if let Some(((ule, ile), (ube, ibe))) = insp.int16 {
            let _ = writeln!(
                out,
                "    \"u16_le\": {ule}, \"i16_le\": {ile}, \"u16_be\": {ube}, \"i16_be\": {ibe},"
            );
        }
        if let Some(((ule, ile), (ube, ibe))) = insp.int32 {
            let _ = writeln!(
                out,
                "    \"u32_le\": {ule}, \"i32_le\": {ile}, \"u32_be\": {ube}, \"i32_be\": {ibe},"
            );
        }
        if let Some(((ule, ile), (ube, ibe))) = insp.int64 {
            let _ = writeln!(
                out,
                "    \"u64_le\": {ule}, \"i64_le\": {ile}, \"u64_be\": {ube}, \"i64_be\": {ibe},"
            );
        }
        if let Some((val, len)) = insp.uleb128 {
            let _ = writeln!(
                out,
                "    \"uleb128\": {{ \"value\": {val}, \"bytes\": {len} }},"
            );
        } else {
            let _ = writeln!(out, "    \"uleb128\": null,");
        }
        if let Some((val, len)) = insp.sleb128 {
            let _ = writeln!(
                out,
                "    \"sleb128\": {{ \"value\": {val}, \"bytes\": {len} }},"
            );
        } else {
            let _ = writeln!(out, "    \"sleb128\": null,");
        }
        if let Some((ch, len)) = insp.utf8_char {
            let _ = writeln!(
                out,
                "    \"utf8_char\": {{ \"char\": \"{}\", \"codepoint\": {}, \"bytes\": {len} }}",
                escape_json(&ch.to_string()),
                ch as u32
            );
        } else {
            let _ = writeln!(out, "    \"utf8_char\": null");
        }
        let _ = writeln!(out, "  }}");
    } else {
        let _ = writeln!(out, "  \"inspection\": null");
    }

    out.push_str("}\n");
    out
}

fn escape_json(s: &str) -> String {
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
