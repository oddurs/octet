// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Grid-locked vector terminal window renderer (`--svg <path>`).

use std::fmt::Write as _;

use crate::theme::{Rgb, Theme};

/// Renders an ANSI-styled terminal output string into a standalone, grid-aligned SVG document.
#[must_use]
pub fn render_ansi_to_svg(ansi_output: &str, title: &str, theme: &Theme) -> String {
    let lines: Vec<Vec<StyledCell>> = ansi_output
        .lines()
        .map(|l| parse_ansi_line(l, theme))
        .collect();
    let max_cols = lines
        .iter()
        .map(std::vec::Vec::len)
        .max()
        .unwrap_or(80)
        .max(60);
    let row_count = lines.len().max(1);

    // 14px Menlo / SF Mono has an exact 8.4px character advance width (0.6 * 14 = 8.4).
    let cell_w = 8.4_f64;
    let cell_h = 20.0_f64;
    let pad_x = 24.0_f64;
    let titlebar_h = 42.0_f64;
    let pad_top = titlebar_h + 18.0;
    let pad_bottom = 24.0_f64;

    let width = (pad_x * 2.0 + (max_cols as f64) * cell_w).ceil();
    let height = (pad_top + (row_count as f64) * cell_h + pad_bottom).ceil();

    let bg_hex = theme.bg_canvas.to_hex();
    let surface_hex = theme.bg_surface.to_hex();
    let border_hex = theme.border.to_hex();
    let title_hex = theme.fg_secondary.to_hex();

    let mut svg = String::with_capacity(lines.len() * 640 + 2048);
    let _ = writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">"#
    );
    let _ = writeln!(
        svg,
        r#"  <style>
    .term-text {{
      font-family: 'Menlo', 'SF Mono', 'JetBrains Mono', 'Monaco', 'Consolas', monospace;
      font-size: 14px;
      font-variant-ligatures: none;
      xml:space: preserve;
    }}
    .ctrl-badge {{
      font-family: 'Menlo', 'SF Mono', 'JetBrains Mono', monospace;
      font-size: 6.2px;
      font-weight: 700;
      letter-spacing: -0.2px;
    }}
    .title-text {{
      font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Inter', sans-serif;
      font-size: 12.5px;
      font-weight: 600;
      fill: {title_hex};
    }}
  </style>"#
    );

    // Outer window background and border
    let _ = writeln!(
        svg,
        r#"  <rect x="1" y="1" width="{}" height="{}" rx="12" ry="12" fill="{bg_hex}" stroke="{border_hex}" stroke-width="1.5"/>"#,
        width - 2.0,
        height - 2.0
    );
    // Titlebar surface
    let _ = writeln!(
        svg,
        r#"  <path d="M 1 13 A 12 12 0 0 1 13 1 L {} 1 A 12 12 0 0 1 {} 13 L {} {titlebar_h} L 1 {titlebar_h} Z" fill="{surface_hex}"/>"#,
        width - 13.0,
        width - 1.0,
        width - 1.0
    );
    let _ = writeln!(
        svg,
        r#"  <line x1="1" y1="{titlebar_h}" x2="{}" y2="{titlebar_h}" stroke="{border_hex}" stroke-width="1"/>"#,
        width - 1.0
    );

    // macOS window control dots
    let _ = writeln!(svg, r##"  <circle cx="22" cy="21" r="6" fill="#ff5f56"/>"##);
    let _ = writeln!(svg, r##"  <circle cx="42" cy="21" r="6" fill="#ffbd2e"/>"##);
    let _ = writeln!(svg, r##"  <circle cx="62" cy="21" r="6" fill="#27c93f"/>"##);

    // Centered command title
    let _ = writeln!(
        svg,
        r#"  <text x="{:.1}" y="25.5" text-anchor="middle" class="title-text">{}</text>"#,
        width / 2.0,
        escape_xml(title)
    );

    // Render each terminal row
    for (row_idx, row_cells) in lines.iter().enumerate() {
        let row_top = pad_top + (row_idx as f64) * cell_h;
        let baseline_y = row_top + 14.5;

        // Pass 1: Background rectangles, underline accents, and vector geometric primitives
        for (col_idx, cell) in row_cells.iter().enumerate() {
            let cell_x = pad_x + (col_idx as f64) * cell_w;
            if let Some(bg) = cell.bg {
                let _ = writeln!(
                    svg,
                    r#"  <rect x="{cell_x:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="2" fill="{}"/>"#,
                    row_top + 1.0,
                    cell_w + 0.4,
                    cell_h - 2.0,
                    bg.to_hex()
                );
            }
            if cell.underline {
                let _ = writeln!(
                    svg,
                    r#"  <rect x="{cell_x:.1}" y="{:.1}" width="{:.1}" height="2" fill="{}" opacity="0.9"/>"#,
                    row_top + cell_h - 2.5,
                    cell_w - 0.5,
                    cell.fg.to_hex()
                );
            }
            let _ = try_render_vector_glyph(
                &mut svg, cell.ch, cell.fg, cell_x, row_top, cell_w, cell_h,
            );
        }

        // Pass 2: Render text characters (skipping spaces and vector-drawn glyphs)
        let mut col_idx = 0;
        while col_idx < row_cells.len() {
            let cell = &row_cells[col_idx];
            if cell.ch == ' ' || is_vector_glyph(cell.ch) {
                col_idx += 1;
                continue;
            }
            let x = pad_x + (col_idx as f64) * cell_w;
            let weight = if cell.bold {
                r#" font-weight="700""#
            } else {
                ""
            };
            let fill = cell.fg.to_hex();

            if cell.ch.is_ascii() {
                let mut run_end = col_idx + 1;
                while run_end < row_cells.len() {
                    let next = &row_cells[run_end];
                    if next.ch.is_ascii()
                        && next.fg == cell.fg
                        && next.bold == cell.bold
                        && next.ch != ' '
                        && !is_vector_glyph(next.ch)
                    {
                        run_end += 1;
                    } else {
                        break;
                    }
                }
                let run_text: String = row_cells[col_idx..run_end].iter().map(|c| c.ch).collect();
                let _ = writeln!(
                    svg,
                    r#"  <text x="{x:.1}" y="{baseline_y:.1}" class="term-text" fill="{fill}"{weight}>{}</text>"#,
                    escape_xml(&run_text)
                );
                col_idx = run_end;
            } else {
                let _ = writeln!(
                    svg,
                    r#"  <text x="{x:.1}" y="{baseline_y:.1}" class="term-text" fill="{fill}"{weight}>{}</text>"#,
                    escape_xml(&cell.ch.to_string())
                );
                col_idx += 1;
            }
        }
    }

    let _ = writeln!(svg, "</svg>");
    svg
}

const fn is_vector_glyph(ch: char) -> bool {
    matches!(
        ch,
        '─' | '│'
            | '╭'
            | '╮'
            | '╰'
            | '╯'
            | '├'
            | '┤'
            | '┬'
            | '┴'
            | '┼'
            | '┌'
            | '└'
            | '⋮'
            | '⋯'
            | '◈'
            | '◂'
            | '▸'
            | '▁'
            | '▂'
            | '▃'
            | '▄'
            | '▅'
            | '▆'
            | '▇'
            | '█'
            | '▓'
            | '▒'
            | '░'
            | '■'
            | '▪'
            | '·'
            | '␣'
            | '↵'
            | '⇥'
            | '\u{2400}'..='\u{2421}'
    )
}

fn control_picture_mnemonic(ch: char) -> Option<&'static str> {
    match ch {
        '↵' => Some("LF"),
        '⇥' => Some("HT"),
        _ => match ch as u32 {
            0x2400 => Some("NL"),
            0x2401 => Some("SH"),
            0x2402 => Some("SX"),
            0x2403 => Some("EX"),
            0x2404 => Some("ET"),
            0x2405 => Some("EQ"),
            0x2406 => Some("AK"),
            0x2407 => Some("BL"),
            0x2408 => Some("BS"),
            0x2409 => Some("HT"),
            0x240A => Some("LF"),
            0x240B => Some("VT"),
            0x240C => Some("FF"),
            0x240D => Some("CR"),
            0x240E => Some("SO"),
            0x240F => Some("SI"),
            0x2410 => Some("DL"),
            0x2411 => Some("D1"),
            0x2412 => Some("D2"),
            0x2413 => Some("D3"),
            0x2414 => Some("D4"),
            0x2415 => Some("NK"),
            0x2416 => Some("SY"),
            0x2417 => Some("EB"),
            0x2418 => Some("CN"),
            0x2419 => Some("EM"),
            0x241A => Some("SB"),
            0x241B => Some("ES"),
            0x241C => Some("FS"),
            0x241D => Some("GS"),
            0x241E => Some("RS"),
            0x241F => Some("US"),
            0x2420 => Some("SP"),
            0x2421 => Some("DE"),
            _ => None,
        },
    }
}

fn try_render_vector_glyph(
    svg: &mut String,
    ch: char,
    fg: Rgb,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> bool {
    let hex = fg.to_hex();
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let x_end = x + w;
    let y_end = y + h;

    if let Some(mnem) = control_picture_mnemonic(ch) {
        let _ = writeln!(
            svg,
            r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="10.5" rx="1.6" fill="{hex}" opacity="0.16"/>"#,
            x + 0.4,
            cy - 5.2,
            w - 0.8
        );
        let _ = writeln!(
            svg,
            r#"  <text x="{cx:.2}" y="{:.2}" text-anchor="middle" class="ctrl-badge" fill="{hex}">{mnem}</text>"#,
            cy + 2.2
        );
        return true;
    }

    match ch {
        '␣' => {
            // Crisp U-shaped open-box space indicator inside cell bounds
            let _ = writeln!(
                svg,
                r#"  <path d="M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}" fill="none" stroke="{hex}" stroke-width="1.2"/>"#,
                x + 1.6,
                cy + 1.8,
                x + 1.6,
                cy + 4.4,
                x_end - 1.6,
                cy + 4.4,
                x_end - 1.6,
                cy + 1.8
            );
            true
        }
        '─' => {
            let _ = writeln!(
                svg,
                r#"  <line x1="{:.2}" y1="{cy:.2}" x2="{:.2}" y2="{cy:.2}" stroke="{hex}" stroke-width="1.3"/>"#,
                x - 0.3,
                x_end + 0.3
            );
            true
        }
        '│' => {
            let _ = writeln!(
                svg,
                r#"  <line x1="{cx:.2}" y1="{:.2}" x2="{cx:.2}" y2="{:.2}" stroke="{hex}" stroke-width="1.3"/>"#,
                y - 0.3,
                y_end + 0.3
            );
            true
        }
        '╭' | '┌' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {cx:.2} {:.2} L {cx:.2} {cy:.2} L {:.2} {cy:.2}" fill="none" stroke="{hex}" stroke-width="1.3" stroke-linejoin="round"/>"#,
                y_end + 0.3,
                x_end + 0.3
            );
            true
        }
        '╮' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {:.2} {cy:.2} L {cx:.2} {cy:.2} L {cx:.2} {:.2}" fill="none" stroke="{hex}" stroke-width="1.3" stroke-linejoin="round"/>"#,
                x - 0.3,
                y_end + 0.3
            );
            true
        }
        '╰' | '└' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {cx:.2} {:.2} L {cx:.2} {cy:.2} L {:.2} {cy:.2}" fill="none" stroke="{hex}" stroke-width="1.3" stroke-linejoin="round"/>"#,
                y - 0.3,
                x_end + 0.3
            );
            true
        }
        '╯' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {:.2} {cy:.2} L {cx:.2} {cy:.2} L {cx:.2} {:.2}" fill="none" stroke="{hex}" stroke-width="1.3" stroke-linejoin="round"/>"#,
                x - 0.3,
                y - 0.3
            );
            true
        }
        '├' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {cx:.2} {:.2} L {cx:.2} {:.2} M {cx:.2} {cy:.2} L {:.2} {cy:.2}" fill="none" stroke="{hex}" stroke-width="1.3"/>"#,
                y - 0.3,
                y_end + 0.3,
                x_end + 0.3
            );
            true
        }
        '┤' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {cx:.2} {:.2} L {cx:.2} {:.2} M {:.2} {cy:.2} L {cx:.2} {cy:.2}" fill="none" stroke="{hex}" stroke-width="1.3"/>"#,
                y - 0.3,
                y_end + 0.3,
                x - 0.3
            );
            true
        }
        '┬' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {:.2} {cy:.2} L {:.2} {cy:.2} M {cx:.2} {cy:.2} L {cx:.2} {:.2}" fill="none" stroke="{hex}" stroke-width="1.3"/>"#,
                x - 0.3,
                x_end + 0.3,
                y_end + 0.3
            );
            true
        }
        '┴' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {:.2} {cy:.2} L {:.2} {cy:.2} M {cx:.2} {:.2} L {cx:.2} {cy:.2}" fill="none" stroke="{hex}" stroke-width="1.3"/>"#,
                x - 0.3,
                x_end + 0.3,
                y - 0.3
            );
            true
        }
        '┼' => {
            let _ = writeln!(
                svg,
                r#"  <path d="M {:.2} {cy:.2} L {:.2} {cy:.2} M {cx:.2} {:.2} L {cx:.2} {:.2}" fill="none" stroke="{hex}" stroke-width="1.3"/>"#,
                x - 0.3,
                x_end + 0.3,
                y - 0.3,
                y_end + 0.3
            );
            true
        }
        '⋮' => {
            for dy in [-4.5, 0.0, 4.5] {
                let _ = writeln!(
                    svg,
                    r#"  <circle cx="{cx:.2}" cy="{:.2}" r="1.15" fill="{hex}"/>"#,
                    cy + dy
                );
            }
            true
        }
        '⋯' => {
            for dx in [-2.6, 0.0, 2.6] {
                let _ = writeln!(
                    svg,
                    r#"  <circle cx="{:.2}" cy="{cy:.2}" r="0.95" fill="{hex}"/>"#,
                    cx + dx
                );
            }
            true
        }
        '◈' => {
            let r = 3.6;
            let _ = writeln!(
                svg,
                r#"  <polygon points="{cx:.2},{:.2} {:.2},{cy:.2} {cx:.2},{:.2} {:.2},{cy:.2}" fill="none" stroke="{hex}" stroke-width="1.2"/>"#,
                cy - r,
                cx + r,
                cy + r,
                cx - r
            );
            let _ = writeln!(
                svg,
                r#"  <circle cx="{cx:.2}" cy="{cy:.2}" r="1.2" fill="{hex}"/>"#
            );
            true
        }
        '◂' => {
            let _ = writeln!(
                svg,
                r#"  <polygon points="{:.2},{cy:.2} {:.2},{:.2} {:.2},{:.2}" fill="{hex}"/>"#,
                cx - 2.6,
                cx + 2.2,
                cy - 3.2,
                cx + 2.2,
                cy + 3.2
            );
            true
        }
        '▸' => {
            let _ = writeln!(
                svg,
                r#"  <polygon points="{:.2},{cy:.2} {:.2},{:.2} {:.2},{:.2}" fill="{hex}"/>"#,
                cx + 2.6,
                cx - 2.2,
                cy - 3.2,
                cx - 2.2,
                cy + 3.2
            );
            true
        }
        '·' => {
            let _ = writeln!(
                svg,
                r#"  <circle cx="{cx:.2}" cy="{cy:.2}" r="1.3" fill="{hex}"/>"#
            );
            true
        }
        '■' => {
            let size = 7.0;
            let _ = writeln!(
                svg,
                r#"  <rect x="{:.2}" y="{:.2}" width="{size}" height="{size}" rx="1.5" fill="{hex}"/>"#,
                cx - size * 0.5,
                cy - size * 0.5
            );
            true
        }
        '▪' => {
            let size = 4.6;
            let _ = writeln!(
                svg,
                r#"  <rect x="{:.2}" y="{:.2}" width="{size}" height="{size}" rx="1" fill="{hex}"/>"#,
                cx - size * 0.5,
                cy - size * 0.5
            );
            true
        }
        '▁' | '▂' | '▃' | '▄' | '▅' | '▆' | '▇' | '█' => {
            let level = match ch {
                '▁' => 1,
                '▂' => 2,
                '▃' => 3,
                '▄' => 4,
                '▅' => 5,
                '▆' => 6,
                '▇' => 7,
                _ => 8,
            };
            let max_bar_h = 13.0;
            let bar_h = (f64::from(level) / 8.0) * max_bar_h;
            let bar_y = y + 3.5 + (max_bar_h - bar_h);
            let _ = writeln!(
                svg,
                r#"  <rect x="{:.2}" y="{bar_y:.2}" width="{:.2}" height="{bar_h:.2}" fill="{hex}"/>"#,
                x + 0.4,
                w - 0.8
            );
            true
        }
        '▓' | '▒' | '░' => {
            let opacity = match ch {
                '▓' => "0.85",
                '▒' => "0.65",
                _ => "0.35",
            };
            let _ = writeln!(
                svg,
                r#"  <rect x="{:.2}" y="{:.2}" width="{:.2}" height="13.0" fill="{hex}" opacity="{opacity}"/>"#,
                x + 0.3,
                y + 3.5,
                w - 0.6
            );
            true
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
struct StyledCell {
    ch: char,
    fg: Rgb,
    bg: Option<Rgb>,
    bold: bool,
    underline: bool,
}

fn parse_ansi_line(line: &str, theme: &Theme) -> Vec<StyledCell> {
    let mut cells = Vec::with_capacity(line.len());
    let mut fg = theme.fg_primary;
    let mut bg: Option<Rgb> = None;
    let mut bold = false;
    let mut underline = false;

    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            let mut seq = String::new();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
                seq.push(c);
            }
            apply_sgr(&seq, theme, &mut fg, &mut bg, &mut bold, &mut underline);
        } else {
            cells.push(StyledCell {
                ch,
                fg,
                bg,
                bold,
                underline,
            });
        }
    }
    cells
}

fn apply_sgr(
    seq: &str,
    theme: &Theme,
    fg: &mut Rgb,
    bg: &mut Option<Rgb>,
    bold: &mut bool,
    underline: &mut bool,
) {
    if seq.is_empty() || seq == "0" {
        *fg = theme.fg_primary;
        *bg = None;
        *bold = false;
        *underline = false;
        return;
    }
    let parts: Vec<&str> = seq.split(';').collect();
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "0" => {
                *fg = theme.fg_primary;
                *bg = None;
                *bold = false;
                *underline = false;
                i += 1;
            }
            "1" => {
                *bold = true;
                i += 1;
            }
            "4" => {
                *underline = true;
                i += 1;
            }
            "38" if i + 4 < parts.len() && parts[i + 1] == "2" => {
                let r = parts[i + 2].parse::<u8>().unwrap_or(255);
                let g = parts[i + 3].parse::<u8>().unwrap_or(255);
                let b = parts[i + 4].parse::<u8>().unwrap_or(255);
                *fg = Rgb(r, g, b);
                i += 5;
            }
            "48" if i + 4 < parts.len() && parts[i + 1] == "2" => {
                let r = parts[i + 2].parse::<u8>().unwrap_or(0);
                let g = parts[i + 3].parse::<u8>().unwrap_or(0);
                let b = parts[i + 4].parse::<u8>().unwrap_or(0);
                *bg = Some(Rgb(r, g, b));
                i += 5;
            }
            _ => {
                i += 1;
            }
        }
    }
}

fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // U+FFFE/U+FFFF are outside the XML 1.0 `Char` production (C0 controls too) and
            // make the document ill-formed; C1 controls are legal but unprintable.
            c if (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
                || matches!(c, '\u{fffe}' | '\u{ffff}') =>
            {
                out.push('·');
            }
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// XML 1.0 `Char` production; anything outside it makes the document ill-formed.
    fn is_xml_char(c: char) -> bool {
        matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..)
    }

    #[test]
    fn svg_output_contains_only_xml_chars() {
        let hostile = "abc\u{fffe}\u{ffff}\u{1}\u{1b}\u{7f}\u{9b}def";
        let svg = render_ansi_to_svg(hostile, hostile, &Theme::dark());
        let bad: Vec<char> = svg.chars().filter(|&c| !is_xml_char(c)).collect();
        assert!(bad.is_empty(), "XML-forbidden characters emitted: {bad:?}");
        assert_eq!(escape_xml("a\u{fffe}b\u{ffff}c"), "a·b·c");
    }
}
