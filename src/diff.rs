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
        let offset = display_offset + start as u64;
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
                        byte_count: count * cols,
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
    let mut out = String::new();
    let left_short = basename(left_label);
    let right_short = basename(right_label);
    let inner_w = 88usize;
    let top_bar = format!("╭{}╮", "─".repeat(inner_w));
    let bot_bar = format!("╰{}╯", "─".repeat(inner_w));

    let _ = writeln!(out, "{}", theme.paint(&top_bar, theme.border, color));
    let summary = format!(
        " ◈ BINARY DIFF · {left_short} ({} B) vs {right_short} ({} B) · {} Δ bytes in {} rows",
        report.left_len, report.right_len, report.changed_bytes, report.changed_rows
    );
    let count = summary.chars().count();
    let padded = if count >= inner_w {
        summary.chars().take(inner_w).collect::<String>()
    } else {
        format!("{summary}{}", " ".repeat(inner_w - count))
    };
    let _ = writeln!(
        out,
        "{}{}{}",
        theme.paint("│", theme.border, color),
        theme.paint_bold(&padded, theme.fg_primary, color),
        theme.paint("│", theme.border, color),
    );
    let _ = writeln!(out, "{}", theme.paint(&bot_bar, theme.border, color));

    for row in &report.rows {
        match row {
            DiffRow::Elided {
                start_offset,
                row_count,
                byte_count,
            } => {
                let off_str = theme.format_offset(*start_offset, 8, false, color);
                let msg =
                    format!("  ⋮   ⋯ {row_count} identical rows ({byte_count} bytes) unchanged ⋯");
                let _ = writeln!(out, "{off_str}{}", theme.paint(&msg, theme.fg_muted, color));
            }
            DiffRow::Data {
                offset,
                left,
                right,
                has_diff,
            } => {
                let off_str = theme.format_offset(*offset, 8, false, color);
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
