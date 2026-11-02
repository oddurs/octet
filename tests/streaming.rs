// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! The streaming renderers must write exactly what the buffered ones return, and the
//! single-pass diff must lay out exactly what the original two-pass one did.

use std::io::{self, Read};

use octet::{
    CodeLanguage, Diff, DiffReport, DiffRow, FormatOptions, Radix, Theme, ViewStyle, compute_diff,
    generate_code, render_diff_ansi, render_dump, stream_code, stream_dump,
};

/// Deterministic xorshift, so a failure names a reproducible case.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

/// A reader that returns short, uneven reads, as pipes do, so that rows and UTF-8
/// sequences land across every kind of read boundary.
struct Trickle<'a> {
    data: &'a [u8],
    rng: Rng,
}

impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let max = buf.len().min(self.data.len());
        if max == 0 {
            return Ok(0);
        }
        let n = 1 + self.rng.below(max.min(70_001));
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

fn streamed_dump(data: &[u8], opts: &FormatOptions, seed: u64) -> String {
    let mut out = Vec::new();
    let reader = Trickle {
        data,
        rng: Rng(seed | 1),
    };
    stream_dump(reader, &mut out, opts).expect("stream into a Vec");
    String::from_utf8(out).expect("UTF-8 output")
}

/// Inputs named by what they exercise. The sizes straddle the 64 KiB step and the
/// 1 MiB header window.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut inputs: Vec<(String, Vec<u8>)> = Vec::new();
    for len in [0, 1, 2, 3, 7, 15, 16, 17, 31, 32, 33, 48, 64, 100] {
        inputs.push((format!("random {len}"), rng.bytes(len)));
        inputs.push((format!("zeros {len}"), vec![0; len]));
    }
    for len in [65_535, 65_536, 65_537, 65_536 * 2 + 5] {
        inputs.push((format!("random {len}"), rng.bytes(len)));
        inputs.push((format!("zeros {len}"), vec![0; len]));
    }
    // Zero runs that cross every step boundary, and end exactly at EOF or just before.
    for (run_start, run_end, len) in [
        (60_000, 140_000, 200_000),
        (65_520, 65_568, 70_000),
        (100, 131_072, 131_072),
        (100, 131_056, 131_072),
        (32, 96, 96),
        (32, 80, 96),
        (16, 64, 64),
    ] {
        let mut data = rng.bytes(len);
        data[run_start..run_end].fill(0);
        inputs.push((format!("zero run {run_start}..{run_end} of {len}"), data));
    }
    // Identical non-zero rows: hexdump collapses them, xxd does not.
    let row = rng.bytes(16);
    let mut repeats: Vec<u8> = row.iter().copied().cycle().take(16 * 9_000).collect();
    repeats.extend_from_slice(&rng.bytes(5));
    inputs.push((String::from("repeated rows"), repeats));
    // Multi-byte UTF-8 in every alignment, for the colour classes.
    let text = "héllo wörld → ∑ 𝄞 日本語 ";
    let utf8: Vec<u8> = text.bytes().cycle().take(140_000).collect();
    inputs.push((String::from("utf-8 text"), utf8));
    // Recognised headers, for colour field highlighting.
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x01\x00\x00\x00\x01\x00\x08\x06\x00\x00\x00".to_vec();
    png.extend(rng.bytes(70_000));
    inputs.push((String::from("png"), png));
    // A 65535-byte file name puts the last header field past the first 64 KiB step.
    let mut zip = b"PK\x03\x04\x14\x00\x00\x00\x00\x00".to_vec();
    zip.extend([0; 16]);
    zip.extend([0xff, 0xff, 0x00, 0x00]);
    zip.extend(std::iter::repeat_n(b'n', 65_535));
    zip.extend(rng.bytes(1_000));
    inputs.push((String::from("zip with a long name"), zip));
    let mut elf = b"\x7fELF\x02\x01\x01\x00".to_vec();
    elf.extend(rng.bytes(1024 * 1024 + 100));
    inputs.push((String::from("elf past the header window"), elf));
    inputs
}

fn options(
    style: ViewStyle,
    radix: Radix,
    cols: usize,
    group_size: usize,
    flags: u8,
    display_offset: u64,
) -> FormatOptions {
    FormatOptions {
        label: Some(String::from("in.bin")),
        style,
        radix,
        cols,
        group_size,
        little_endian: flags & 1 != 0,
        upper_hex: flags & 2 != 0,
        autoskip: flags & 4 != 0,
        display_offset,
        annotate_headers: flags & 8 != 0,
        search_spans: Vec::new(),
        theme: Theme::dark(),
        color: flags & 16 != 0,
    }
}

/// Every combination of the options that change compatible output.
fn option_matrix() -> Vec<FormatOptions> {
    let mut all = Vec::new();
    for style in [ViewStyle::Classic, ViewStyle::Canonical, ViewStyle::Plain] {
        for radix in [Radix::Hex, Radix::Binary, Radix::Octal, Radix::Decimal] {
            for cols in [1, 3, 16, 20] {
                for group_size in [0, 2, 5] {
                    for flags in 0..32 {
                        for display_offset in [0, u64::MAX - 20] {
                            all.push(options(
                                style,
                                radix,
                                cols,
                                group_size,
                                flags,
                                display_offset,
                            ));
                        }
                    }
                }
            }
        }
    }
    // Plain's `-c 0` is one unbroken line.
    all.push(options(ViewStyle::Plain, Radix::Hex, 0, 2, 0, 0));
    all
}

fn assert_streams_identically(name: &str, data: &[u8], opts: &FormatOptions, seed: u64) {
    let expected = render_dump(data, opts);
    let actual = streamed_dump(data, opts, seed);
    if actual != expected {
        let (a, e) = (actual.as_bytes(), expected.as_bytes());
        let at = a
            .iter()
            .zip(e)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(e.len()));
        let lo = at.saturating_sub(80);
        panic!(
            "{name}, {opts:?}: first difference at byte {at}\n  streamed: {:?}\n  buffered: {:?}",
            String::from_utf8_lossy(&a[lo..(at + 80).min(a.len())]),
            String::from_utf8_lossy(&e[lo..(at + 80).min(e.len())]),
        );
    }
}

#[test]
fn small_streamed_dumps_match_render_dump_for_every_option() {
    let matrix = option_matrix();
    let mut rng = Rng(0x1234_5678);
    let mut checked = 0;
    for (name, data) in corpus().into_iter().filter(|(_, d)| d.len() <= 100) {
        for opts in &matrix {
            assert_streams_identically(&name, &data, opts, rng.next());
            checked += 1;
        }
    }
    assert!(checked > 50_000, "only {checked} cases ran");
}

#[test]
fn large_streamed_dumps_match_render_dump() {
    let matrix = option_matrix();
    let defaults = [
        options(ViewStyle::Classic, Radix::Hex, 16, 2, 0, 0),
        options(ViewStyle::Classic, Radix::Hex, 16, 2, 4, 0),
        options(ViewStyle::Canonical, Radix::Hex, 16, 1, 4, 0),
        options(ViewStyle::Canonical, Radix::Hex, 16, 1, 4 | 8 | 16, 0),
        options(ViewStyle::Classic, Radix::Hex, 16, 2, 4 | 8 | 16, 0),
        // One- and two-byte rows put a UTF-8 sequence across nearly every cut.
        options(ViewStyle::Classic, Radix::Hex, 1, 1, 16, 0),
        options(ViewStyle::Canonical, Radix::Binary, 2, 1, 4 | 16, 0),
        options(ViewStyle::Plain, Radix::Hex, 30, 2, 0, 0),
        options(ViewStyle::Plain, Radix::Hex, 0, 2, 2, 0),
    ];
    let mut rng = Rng(0x8765_4321);
    for (name, data) in corpus().into_iter().filter(|(_, d)| d.len() > 100) {
        let sample: Vec<&FormatOptions> =
            (0..3).map(|_| &matrix[rng.below(matrix.len())]).collect();
        for opts in defaults.iter().chain(sample) {
            assert_streams_identically(&name, &data, opts, rng.next());
        }
    }
}

#[test]
fn streamed_codegen_matches_generate_code() {
    let languages = [
        CodeLanguage::C,
        CodeLanguage::Rust,
        CodeLanguage::Go,
        CodeLanguage::Python,
        CodeLanguage::TypeScript,
        CodeLanguage::Zig,
        CodeLanguage::CSharp,
    ];
    let mut rng = Rng(77);
    for len in [0, 1, 11, 12, 13, 65_535, 65_536, 65_537, 200_003] {
        let data = rng.bytes(len);
        for lang in languages {
            for cols in [1, 12, 16] {
                for upper in [false, true] {
                    for name in [None, Some("dir/9 file.bin")] {
                        let expected = generate_code(&data, name, lang, cols, upper);
                        let mut out = Vec::new();
                        let reader = Trickle {
                            data: &data,
                            rng: Rng(rng.next() | 1),
                        };
                        stream_code(reader, &mut out, name, lang, cols, upper)
                            .expect("stream into a Vec");
                        assert!(
                            out == expected.as_bytes(),
                            "{len} bytes, {lang:?}, cols {cols}, upper {upper}, name {name:?}"
                        );
                    }
                }
            }
        }
    }
}

/// The diff engine as it was before it learned to skip unshown rows: every row built,
/// then elided. Kept verbatim as the reference the new one must match.
fn reference_diff(
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
            let prev_changed = i > 0 && raw_rows[i - 1].3;
            if !prev_changed {
                let mut run_end = i;
                while run_end < raw_rows.len() && !raw_rows[run_end].3 {
                    run_end += 1;
                }
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

#[test]
fn single_pass_diff_matches_the_reference() {
    let mut rng = Rng(4242);
    let theme = Theme::dark();
    for case in 0..3_000 {
        let len = match case % 4 {
            0 => rng.below(40),
            1 => rng.below(300),
            2 => rng.below(5_000),
            _ => 8 * rng.below(64),
        };
        let left = rng.bytes(len);
        let mut right = left.clone();
        // Sparse edits leave unchanged runs of every length between them.
        for _ in 0..rng.below(8) {
            if !right.is_empty() {
                let at = rng.below(right.len());
                right[at] ^= 1 + rng.below(255) as u8;
            }
        }
        match rng.below(4) {
            0 => right.truncate(rng.below(len + 1)),
            1 => {
                let extra = rng.below(40);
                right.extend(rng.bytes(extra));
            }
            _ => {}
        }
        let cols = [1, 4, 5, 8, 16][rng.below(5)];
        let offset = [0, 0x100, u64::MAX - 64][rng.below(3)];
        let autoskip = rng.below(4) != 0;

        let expected = reference_diff(&left, &right, cols, offset, autoskip);
        let actual = compute_diff(&left, &right, cols, offset, autoskip);
        assert_eq!(actual, expected, "case {case}");

        let diff = Diff::new(&left, &right, cols, offset, autoskip);
        let summary = diff.summary();
        assert_eq!(
            (
                summary.changed_bytes,
                summary.changed_rows,
                summary.rows.len()
            ),
            (expected.changed_bytes, expected.changed_rows, 0),
            "case {case}"
        );

        let color = case % 2 == 0;
        let rendered = render_diff_ansi(&expected, "a.bin", "b.bin", cols, &theme, color);
        let mut written = Vec::new();
        diff.write_ansi(&mut written, "a.bin", "b.bin", &theme, color)
            .expect("write into a Vec");
        assert!(written == rendered.as_bytes(), "case {case}");
    }
}
