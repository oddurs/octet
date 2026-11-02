// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Side-by-side binary diff engine (`--diff <file>`).

use std::fmt::Write as _;
use std::io;
use std::path::Path;

use crate::classify::ByteClass;
use crate::theme::Theme;
use crate::width::pad_or_clip;

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
    Diff::new(left, right, cols, display_offset, autoskip).report()
}

/// A comparison of two buffers that is laid out row by row on demand, so that rendering
/// it holds one row at a time rather than a [`DiffRow`] for every row of the inputs.
#[derive(Debug, Clone, Copy)]
pub struct Diff<'a> {
    left: &'a [u8],
    right: &'a [u8],
    cols: usize,
    display_offset: u64,
    autoskip: bool,
}

/// Where a shown row comes from, before its bytes are copied out.
enum Planned {
    Data { row: usize, has_diff: bool },
    Elided { first: usize, count: usize },
}

impl<'a> Diff<'a> {
    /// Prepares a comparison with the same parameters as [`compute_diff`].
    #[must_use]
    pub fn new(
        left: &'a [u8],
        right: &'a [u8],
        cols: usize,
        display_offset: u64,
        autoskip: bool,
    ) -> Self {
        Self {
            left,
            right,
            cols: cols.max(4),
            display_offset,
            autoskip,
        }
    }

    /// The full report, as [`compute_diff`] returns it.
    #[must_use]
    pub fn report(&self) -> DiffReport {
        let mut rows = Vec::new();
        let mut report = self.plan(|p| rows.push(self.materialize(&p)));
        report.rows = rows;
        report
    }

    /// The report's totals with `rows` left empty: all that [`render_diff_json`] reads.
    #[must_use]
    pub fn summary(&self) -> DiffReport {
        self.plan(|_| {})
    }

    /// Writes what [`render_diff_ansi`] returns for [`Diff::report`], rendering one row at
    /// a time.
    ///
    /// # Errors
    ///
    /// Returns the first error from writing `out`.
    pub fn write_ansi(
        &self,
        mut out: impl io::Write,
        left_label: &str,
        right_label: &str,
        theme: &Theme,
        color: bool,
    ) -> io::Result<()> {
        // The header shows the totals and is as wide as the last row's offset, so a
        // first pass finds those and a second renders the rows.
        let mut last_offset = 0;
        let summary = self.plan(|p| last_offset = self.offset_of(&p));
        let layout = DiffLayout::new(self.cols, last_offset, theme, color);
        let mut text = String::new();
        layout.head(&mut text, &summary, left_label, right_label);
        let mut result = Ok(());
        self.plan(|p| {
            if result.is_ok() {
                layout.row(&mut text, &self.materialize(&p));
                if text.len() >= 64 * 1024 {
                    result = out.write_all(text.as_bytes());
                    text.clear();
                }
            }
        });
        result?;
        layout.foot(&mut text);
        out.write_all(text.as_bytes())
    }

    /// Walks both inputs once, passing each row that will be shown to `emit` and
    /// returning the totals. The rules are those of `autoskip`: an unchanged run of rows
    /// collapses when at least two of its rows can go, keeping the row on either side of
    /// a changed row as context.
    fn plan(&self, mut emit: impl FnMut(Planned)) -> DiffReport {
        let cols = self.cols;
        let total_rows = self.left.len().max(self.right.len()).div_ceil(cols);
        let mut changed_bytes = 0;
        let mut changed_rows = 0;
        // The first row of an unchanged run whose rows are not shown yet.
        let mut run_start = None;
        let mut prev_changed = false;

        let close_run = |emit: &mut dyn FnMut(Planned), first: usize, end: usize| {
            let len = end - first;
            let data = |row| Planned::Data {
                row,
                has_diff: false,
            };
            if end == total_rows {
                if len >= 2 {
                    emit(Planned::Elided { first, count: len });
                } else {
                    emit(data(first));
                }
            } else if len >= 3 {
                emit(Planned::Elided {
                    first,
                    count: len - 1,
                });
                emit(data(end - 1));
            } else {
                for row in first..end {
                    emit(data(row));
                }
            }
        };

        for row in 0..total_rows {
            let changes = self.changes_in(row);
            if changes > 0 {
                changed_bytes += changes;
                changed_rows += 1;
                if let Some(first) = run_start.take() {
                    close_run(&mut emit, first, row);
                }
                emit(Planned::Data {
                    row,
                    has_diff: true,
                });
                prev_changed = true;
            } else if !self.autoskip || prev_changed {
                emit(Planned::Data {
                    row,
                    has_diff: false,
                });
                prev_changed = false;
            } else if run_start.is_none() {
                run_start = Some(row);
            }
        }
        if let Some(first) = run_start {
            close_run(&mut emit, first, total_rows);
        }

        DiffReport {
            left_len: self.left.len(),
            right_len: self.right.len(),
            changed_bytes,
            changed_rows,
            rows: Vec::new(),
        }
    }

    /// The bytes of `row` on one side; shorter than a row past that side's end.
    fn side(data: &[u8], start: usize, cols: usize) -> &[u8] {
        let from = start.min(data.len());
        &data[from..(start + cols).min(data.len())]
    }

    /// Positions in `row` where the sides differ, a byte present on only one side included.
    fn changes_in(&self, row: usize) -> usize {
        let start = row * self.cols;
        let l = Self::side(self.left, start, self.cols);
        let r = Self::side(self.right, start, self.cols);
        if l == r {
            return 0;
        }
        let common = l.len().min(r.len());
        let differing = l[..common]
            .iter()
            .zip(&r[..common])
            .filter(|(a, b)| a != b)
            .count();
        differing + l.len().max(r.len()) - common
    }

    fn row_offset(&self, row: usize) -> u64 {
        self.display_offset.saturating_add((row * self.cols) as u64)
    }

    fn offset_of(&self, planned: &Planned) -> u64 {
        match *planned {
            Planned::Data { row, .. } => self.row_offset(row),
            Planned::Elided { first, .. } => self.row_offset(first),
        }
    }

    fn materialize(&self, planned: &Planned) -> DiffRow {
        let cols = self.cols;
        match *planned {
            Planned::Data { row, has_diff } => {
                let start = row * cols;
                let bytes = |data: &[u8]| {
                    (start..start + cols)
                        .map(|i| data.get(i).copied())
                        .collect()
                };
                DiffRow::Data {
                    offset: self.row_offset(row),
                    left: bytes(self.left),
                    right: bytes(self.right),
                    has_diff,
                }
            }
            Planned::Elided { first, count } => {
                let max_len = self.left.len().max(self.right.len());
                DiffRow::Elided {
                    start_offset: self.row_offset(first),
                    row_count: count,
                    // The final row of the longer buffer may be short.
                    byte_count: ((first + count) * cols).min(max_len) - first * cols,
                }
            }
        }
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
    let max_off = report
        .rows
        .iter()
        .map(|r| match r {
            DiffRow::Elided { start_offset, .. } => *start_offset,
            DiffRow::Data { offset, .. } => *offset,
        })
        .max()
        .unwrap_or(0);
    let layout = DiffLayout::new(cols.max(4), max_off, theme, color);
    let mut out = String::new();
    layout.head(&mut out, report, left_label, right_label);
    for row in &report.rows {
        layout.row(&mut out, row);
    }
    layout.foot(&mut out);
    out
}

/// The table geometry shared by the head, every row and the foot of a rendered diff.
struct DiffLayout<'a> {
    cols: usize,
    off_w: usize,
    side_w: usize,
    table_w: usize,
    inner_w: usize,
    theme: &'a Theme,
    color: bool,
}

impl<'a> DiffLayout<'a> {
    fn new(cols: usize, max_off: u64, theme: &'a Theme, color: bool) -> Self {
        let off_w = format!("{max_off:08x}").len();
        let side_w = 4 * cols + 2;
        let table_w = (off_w + 8) + 2 * side_w;
        Self {
            cols,
            off_w,
            side_w,
            table_w,
            inner_w: table_w.saturating_sub(2),
            theme,
            color,
        }
    }

    fn head(&self, out: &mut String, report: &DiffReport, left_label: &str, right_label: &str) {
        let (theme, color, inner_w, side_w) = (self.theme, self.color, self.inner_w, self.side_w);
        let left_short = crate::format::neutralise_controls(basename(left_label));
        let right_short = crate::format::neutralise_controls(basename(right_label));
        let top_bar = format!("╭{}╮", "─".repeat(inner_w));

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
        let padded = pad_or_clip(&summary, inner_w);
        let _ = writeln!(
            out,
            "{}{}{}",
            theme.paint("│", theme.border, color),
            theme.paint_bold(&padded, theme.fg_primary, color),
            theme.paint("│", theme.border, color),
        );
        let off_hdr = format!(
            " {:<width$}",
            "OFFSET",
            width = self.off_w.saturating_sub(2)
        );
        let left_hdr = pad_or_clip(&format!("⊖ {left_short} ({} B)", report.left_len), side_w);
        let right_hdr = pad_or_clip(
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
    }

    fn row(&self, out: &mut String, row: &DiffRow) {
        let (theme, color, off_w) = (self.theme, self.color, self.off_w);
        match row {
            DiffRow::Elided {
                start_offset,
                row_count,
                byte_count,
            } => {
                let off_str = theme.format_offset(*start_offset, off_w, false, color);
                let msg = pad_or_clip(
                    &format!(" ⋮ ⋯ {row_count} identical rows ({byte_count} bytes) unchanged ⋯"),
                    self.table_w.saturating_sub(off_w + 1),
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
                let left_side = format_diff_side(left, right, true, self.cols, theme, color);
                let right_side = format_diff_side(right, left, false, self.cols, theme, color);
                let _ = writeln!(
                    out,
                    "{off_str} {marker} {left_side}  {}  {right_side}",
                    theme.paint("│", theme.border, color)
                );
            }
        }
    }

    fn foot(&self, out: &mut String) {
        let (theme, color, inner_w) = (self.theme, self.color, self.inner_w);
        let top_bar = format!("╭{}╮", "─".repeat(inner_w));
        let bot_bar = format!("╰{}╯", "─".repeat(inner_w));
        let _ = writeln!(out, "{}", theme.paint(&top_bar, theme.border, color));
        let footer_plain =
            " Δ Modified Row   ⊖ Left / Removed Byte   ⊕ Right / Added Byte   · Unchanged";
        let footer_padded = pad_or_clip(footer_plain, inner_w);
        let _ = writeln!(
            out,
            "{}{}{}",
            theme.paint("│", theme.border, color),
            theme.paint(&footer_padded, theme.fg_muted, color),
            theme.paint("│", theme.border, color),
        );
        let _ = writeln!(out, "{}", theme.paint(&bot_bar, theme.border, color));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::width::str_width;

    /// Cells pad by terminal columns, so a CJK file name, two columns per
    /// ideograph, leaves the card's right border straight.
    #[test]
    fn a_wide_file_name_keeps_every_card_line_the_same_width() {
        let left = b"hello, world".to_vec();
        let mut right = left.clone();
        right[3] = b'L';
        let report = compute_diff(&left, &right, 8, 0, true);
        let card = render_diff_ansi(
            &report,
            "数据/旧版本.bin",
            "新版本.bin",
            8,
            &Theme::dark(),
            false,
        );
        let widths: Vec<usize> = card.lines().map(str_width).collect();
        assert!(
            widths.iter().all(|&w| w == widths[0]),
            "line widths {widths:?} in:\n{card}"
        );
    }
}
