// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Regression tests for `-r` row-format detection and diff elision accounting.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use octet::{DiffRow, FormatOptions, ViewStyle, compute_diff, render_dump, reverse_dump};

/// Bytes whose `-g4` hex groups read as valid 8-bit binary tokens.
const BIT_LOOKALIKE: &[u8] = b"\x00\x00\x00\x00\x01\x10\x11\x01";

#[test]
fn test_reverse_xxd_g4_groups_that_look_like_bits() {
    let dump = "00000000: 00000000 01101101                    ........\n";
    assert_eq!(reverse_dump(dump, false).unwrap(), BIT_LOOKALIKE);
}

#[test]
fn test_reverse_octet_g4_groups_that_look_like_bits() {
    for style in [ViewStyle::Smart, ViewStyle::Compact] {
        let opts = FormatOptions {
            style,
            group_size: 4,
            color: false,
            ..FormatOptions::default()
        };
        let dump = render_dump(BIT_LOOKALIKE, &opts);
        assert_eq!(
            reverse_dump(&dump, false).unwrap(),
            BIT_LOOKALIKE,
            "{style:?}"
        );
    }
}

#[test]
fn test_reverse_xxd_bits_with_nonprintable_ascii_column() {
    let dump = "\
00000000: 00000000 00000001 00010000 00010001 00000001 00000000  ......
00000006: 00000000 00000001 00010000                             ...
";
    assert_eq!(
        reverse_dump(dump, false).unwrap(),
        b"\x00\x01\x10\x11\x01\x00\x00\x01\x10"
    );
}

#[test]
fn test_reverse_xxd_ascii_column_containing_pipe() {
    let hex = "00000000: 6162 2020 7c                                ab  |\n";
    assert_eq!(reverse_dump(hex, false).unwrap(), b"ab  |");

    let bits = "\
00000000: 01100001 01100010 00100000 00100000 01111100 00000000  ab  |.
00000006: 00000001                                               .
";
    assert_eq!(reverse_dump(bits, false).unwrap(), b"ab  |\x00\x01");
}

#[test]
fn test_reverse_hexdump_canonical_ascii_column_containing_pipe() {
    let dump = "\
00000000  61 62 20 20 7c                                    |ab  ||
00000005
";
    assert_eq!(reverse_dump(dump, false).unwrap(), b"ab  |");
}

#[test]
fn test_diff_elided_run_counts_short_final_row() {
    let data = [0x5a_u8; 17];
    let report = compute_diff(&data, &data, 8, 0, true);
    let elided: Vec<(usize, usize)> = report
        .rows
        .iter()
        .filter_map(|row| match row {
            DiffRow::Elided {
                row_count,
                byte_count,
                ..
            } => Some((*row_count, *byte_count)),
            DiffRow::Data { .. } => None,
        })
        .collect();
    assert_eq!(elided, vec![(3, 17)]);
}

fn run_with_stdin(bin: &str, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {bin}: {e}"));
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input)
        .expect("write child stdin");
    let output = child.wait_with_output().expect("wait on child");
    assert!(output.status.success(), "{bin} {args:?} failed");
    output.stdout
}

fn prng_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.max(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state & 0xff) as u8
        })
        .collect()
}

/// Random and printable-heavy payloads put `|`, double spaces and bit-like groups into
/// every column, which is where format detection used to guess wrong.
#[test]
fn test_reverse_round_trips_xxd_and_octet_layouts() {
    let octet = env!("CARGO_BIN_EXE_octet");
    let has_xxd = Path::new("/usr/bin/xxd").exists();
    let mut payloads = vec![BIT_LOOKALIKE.to_vec(), b"ab  |".to_vec()];
    for seed in 1..=12_u64 {
        let raw = prng_bytes(97 + seed as usize * 13, seed);
        payloads.push(raw.clone());
        payloads.push(
            raw.iter()
                .map(|b| b" |01a\x00\x01\x10\x11"[(*b % 9) as usize])
                .collect(),
        );
    }

    for payload in &payloads {
        let mut dumps = vec![
            run_with_stdin(octet, &["--color=never", "-g", "4"], payload),
            run_with_stdin(octet, &["--color=never", "-b"], payload),
        ];
        if has_xxd {
            for flags in [&[][..], &["-g4"], &["-b"]] {
                dumps.push(run_with_stdin("/usr/bin/xxd", flags, payload));
            }
        }
        for dump in dumps {
            let text = String::from_utf8(dump).expect("dump is UTF-8");
            assert_eq!(
                &reverse_dump(&text, false).unwrap(),
                payload,
                "round trip failed for dump:\n{text}"
            );
        }
    }
}
