// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Side-by-side binary diff engine (`--diff <file>`).

use std::fmt::Write as _;
use std::path::Path;

use crate::classify::ByteClass;
use crate::theme::Theme;

/// Summary metrics for a binary comparison between two buffers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffReport {
    /// Length of the left (original) buffer in bytes.
    pub left_len: usize,
    /// Length of the right (modified) buffer in bytes.
    pub right_len: usize,
    /// Number of byte positions that differ.
    pub changed_bytes: usize,
    /// Number of rows with at least one differing byte.
    pub changed_rows: usize,
    /// Structured rows for rendering in terminal or SVG.
    pub rows: Vec<DiffRow>,
}

/// A single row or elision bridge in a side-by-side binary diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    /// Elided run of identical rows.
    Elided {
        /// Starting byte offset of the identical run.
        start_offset: u64,
        /// Number of identical rows skipped.
        row_count: usize,
        /// Total identical bytes skipped.
        byte_count: usize,
    },
    /// Data row comparing `left` and `right` slices at `offset`.
    Data {
        /// Byte offset of this row.
        offset: u64,
        /// Left buffer bytes (`None` if beyond EOF).
        left: Vec<Option<u8>>,
        /// Right buffer bytes (`None` if beyond EOF).
        right: Vec<Option<u8>>,
        /// Whether any byte in this row differs.
        has_diff: bool,
    },
}

/// Compares `left` and `right` byte slices into a structured [`DiffReport`].
#[must_use]
pub fn compute_diff(
    left: &[u8],
    right: &[u8],
    cols: usize,
    display_offset: u64,
    autoskip: bool,
) -> DiffReport {
    let cols = cols.max(4);
    let max_len = left.len().max(right.len());
    let total_rows = max_len.div_ceil(cols);
    let mut changed_bytes = 0;
    let mut changed_rows = 0;
    let mut raw_rows = Vec::with_capacity(total_rows);

    for row_idx in 0..total_rows {
        let start = row_idx * cols;
        let offset = display_offset.saturating_add(start as u64);
        let mut l_vec = Vec::with_capacity(cols);
        let mut r_vec = Vec::with_capacity(cols);
        let mut row_diff = false;

        for col in 0..cols {
            let idx = start + col;
            let lb = left.get(idx).copied();
            let rb = right.get(idx).copied();
            if lb != rb && (lb.is_some() || rb.is_some()) {
                changed_bytes += 1;
                row_diff = true;
            }
            l_vec.push(lb);
            r_vec.push(rb);
        }
        if row_diff {
            changed_rows += 1;
        }
        raw_rows.push((offset, l_vec, r_vec, row_diff));
    }

    let mut rows = Vec::new();
    let mut i = 0;
    while i < raw_rows.len() {
        if autoskip && !raw_rows[i].3 {
            // Keep 1 context row immediately adjacent to a changed row.
            let prev_changed = i > 0 && raw_rows[i - 1].3;
            if !prev_changed {
                let mut run_end = i;
                while run_end < raw_rows.len() && !raw_rows[run_end].3 {
                    run_end += 1;
                }
                // Leave 1 context row before the next changed row if there is one.
                let elide_until = if run_end < raw_rows.len() && run_end > i + 1 {
                    run_end - 1
                } else {
                    run_end
                };
                let count = elide_until.saturating_sub(i);
                if count >= 2 {
                    rows.push(DiffRow::Elided {
                        start_offset: raw_rows[i].0,
                        row_count: count,
                        // The final row of the longer buffer may be short.
                        byte_count: (elide_until * cols).min(max_len) - i * cols,
                    });
                    i = elide_until;
                    continue;
                }
            }
        }
        let (offset, left_b, right_b, has_diff) = raw_rows[i].clone();
        rows.push(DiffRow::Data {
            offset,
            left: left_b,
            right: right_b,
            has_diff,
        });
        i += 1;
    }

    DiffReport {
        left_len: left.len(),
        right_len: right.len(),
        changed_bytes,
        changed_rows,
        rows,
    }
}

/// Renders a [`DiffReport`] as an ANSI-formatted side-by-side comparison.
#[must_use]
pub fn render_diff_ansi(
    report: &DiffReport,
    left_label: &str,
    right_label: &str,
    cols: usize,
    theme: &Theme,
    color: bool,
) -> String {
    let cols = cols.max(4);
    let mut out = String::new();
    let left_short = basename(left_label);
    let right_short = basename(right_label);
    let max_off = report
        .rows
        .iter()
        .map(|r| match r {
            DiffRow::Elided { start_offset, .. } => *start_offset,
            DiffRow::Data { offset, .. } => *offset,
        })
        .max()
        .unwrap_or(0);
    let off_w = format!("{max_off:08x}").len();
    let side_w = 4 * cols + 2;
    let table_w = (off_w + 8) + 2 * side_w;
    let inner_w = table_w.saturating_sub(2);
    let top_bar = format!("╭{}╮", "─".repeat(inner_w));
    let bot_bar = format!("╰{}╯", "─".repeat(inner_w));

    let _ = writeln!(out, "{}", theme.paint(&top_bar, theme.border, color));
    let max_len = report.left_len.max(report.right_len);
    let similarity = if max_len == 0 {
        100.0
    } else {
        ((max_len.saturating_sub(report.changed_bytes) as f64) / (max_len as f64)) * 100.0
    };
    let summary = format!(
        " ◈ BINARY DIFF  │  {} Δ bytes across {} changed rows  │  {similarity:.1}% identical",
        report.changed_bytes, report.changed_rows
    );
    let padded = pad_diff_cell(&summary, inner_w);
    let _ = writeln!(
        out,
        "{}{}{}",
        theme.paint("│", theme.border, color),
        theme.paint_bold(&padded, theme.fg_primary, color),
        theme.paint("│", theme.border, color),
    );
    let off_hdr = format!(" {:<width$}", "OFFSET", width = off_w.saturating_sub(2));
    let left_hdr = pad_diff_cell(&format!("⊖ {left_short} ({} B)", report.left_len), side_w);
    let right_hdr = pad_diff_cell(
        &format!("⊕ {right_short} ({} B)", report.right_len),
        side_w.saturating_sub(1),
    );
    let j1 = off_hdr.chars().count() + 1;
    let j2 = j1 + 1 + 1 + side_w + 2;
    let hdr_mid_bar = build_diff_junction_bar('├', '┤', '┬', inner_w, &[j1, j2]);
    let hdr_bot_bar = build_diff_junction_bar('╰', '╯', '┴', inner_w, &[j1, j2]);
    let _ = writeln!(out, "{}", theme.paint(&hdr_mid_bar, theme.border, color));
    let _ = writeln!(
        out,
        "{}{} {} {}  {}  {}{}",
        theme.paint("│", theme.border, color),
        theme.paint_bold(&off_hdr, theme.fg_muted, color),
        theme.paint("│", theme.border, color),
        theme.paint_bold(&left_hdr, theme.diff_del, color),
        theme.paint("│", theme.border, color),
        theme.paint_bold(&right_hdr, theme.diff_add, color),
        theme.paint("│", theme.border, color),
    );
    let _ = writeln!(out, "{}", theme.paint(&hdr_bot_bar, theme.border, color));

    for row in &report.rows {
        match row {
            DiffRow::Elided {
                start_offset,
                row_count,
                byte_count,
            } => {
                let off_str = theme.format_offset(*start_offset, off_w, false, color);
                let msg = pad_diff_cell(
                    &format!(" ⋮ ⋯ {row_count} identical rows ({byte_count} bytes) unchanged ⋯"),
                    table_w.saturating_sub(off_w + 1),
                );
                let _ = writeln!(
                    out,
                    "{off_str}{}{}",
                    theme.paint(&msg, theme.fg_muted, color),
                    theme.paint("│", theme.border, color)
                );
            }
            DiffRow::Data {
                offset,
                left,
                right,
                has_diff,
            } => {
                let off_str = theme.format_offset(*offset, off_w, false, color);
                let marker = if *has_diff {
                    theme.paint_bold("Δ", theme.byte_control, color)
                } else {
                    theme.paint("│", theme.border, color)
                };
                let left_side = format_diff_side(left, right, true, cols, theme, color);
                let right_side = format_diff_side(right, left, false, cols, theme, color);
                let _ = writeln!(
                    out,
                    "{off_str} {marker} {left_side}  {}  {right_side}",
                    theme.paint("│", theme.border, color)
                );
            }
        }
    }

    let _ = writeln!(out, "{}", theme.paint(&top_bar, theme.border, color));
    let footer_plain =
        " Δ Modified Row   ⊖ Left / Removed Byte   ⊕ Right / Added Byte   · Unchanged";
    let footer_padded = pad_diff_cell(footer_plain, inner_w);
    let _ = writeln!(
        out,
        "{}{}{}",
        theme.paint("│", theme.border, color),
        theme.paint(&footer_padded, theme.fg_muted, color),
        theme.paint("│", theme.border, color),
    );
    let _ = writeln!(out, "{}", theme.paint(&bot_bar, theme.border, color));
    out
}

fn build_diff_junction_bar(
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

fn pad_diff_cell(s: &str, width: usize) -> String {
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

/// Renders a [`DiffReport`] as a standalone RFC 8259 JSON object.
#[must_use]
pub fn render_diff_json(report: &DiffReport, left_label: &str, right_label: &str) -> String {
    let max_len = report.left_len.max(report.right_len);
    let similarity = if max_len == 0 {
        100.0
    } else {
        ((max_len.saturating_sub(report.changed_bytes) as f64) / (max_len as f64)) * 100.0
    };
    let mut out = String::from("{\n");
    let _ = writeln!(out, "  \"mode\": \"diff\",");
    let _ = writeln!(out, "  \"left_label\": \"{}\",", escape_json(left_label));
    let _ = writeln!(out, "  \"right_label\": \"{}\",", escape_json(right_label));
    let _ = writeln!(out, "  \"left_bytes\": {},", report.left_len);
    let _ = writeln!(out, "  \"right_bytes\": {},", report.right_len);
    let _ = writeln!(out, "  \"changed_bytes\": {},", report.changed_bytes);
    let _ = writeln!(out, "  \"changed_rows\": {},", report.changed_rows);
    let _ = writeln!(out, "  \"similarity_pct\": {similarity:.2}");
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

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
}

fn format_diff_side(
    primary: &[Option<u8>],
    other: &[Option<u8>],
    is_left: bool,
    cols: usize,
    theme: &Theme,
    color: bool,
) -> String {
    let mut hex_part = String::new();
    let mut glyph_part = String::new();
    let delta_color = if is_left {
        theme.diff_del
    } else {
        theme.diff_add
    };

    for i in 0..cols {
        let pb = primary.get(i).copied().flatten();
        let ob = other.get(i).copied().flatten();
        let changed = pb != ob;

        match pb {
            Some(b) => {
                let class = ByteClass::classify_single(b);
                let g = class.glyph(b, true);
                let hex_tok = format!("{b:02x}");
                if changed {
                    let _ = write!(
                        hex_part,
                        "{}",
                        theme.paint_bold(&hex_tok, delta_color, color)
                    );
                    let _ = write!(
                        glyph_part,
                        "{}",
                        theme.paint_bold(&g.to_string(), delta_color, color)
                    );
                } else {
                    let c = if b == 0 {
                        theme.byte_null
                    } else {
                        theme.fg_secondary
                    };
                    let _ = write!(hex_part, "{}", theme.paint(&hex_tok, c, color));
                    let _ = write!(glyph_part, "{}", theme.paint(&g.to_string(), c, color));
                }
            }
            None => {
                let _ = write!(hex_part, "{}", theme.paint("--", theme.fg_muted, color));
                let _ = write!(glyph_part, "{}", theme.paint(" ", theme.fg_muted, color));
            }
        }
        if i + 1 < cols {
            hex_part.push(' ');
        }
    }
    format!(
        "{hex_part} {}{glyph_part}{}",
        theme.paint("│", theme.border, color),
        theme.paint("│", theme.border, color)
    )
}
