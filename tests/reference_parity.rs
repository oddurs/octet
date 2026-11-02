// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Reference-oracle differential test suite comparing `octet` directly against
//! the system `/usr/bin/xxd` and `/usr/bin/hexdump` binaries.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use octet::{FormatOptions, Radix, ViewStyle, render_dump, reverse_dump};

fn octet_bin() -> &'static str {
    env!("CARGO_BIN_EXE_octet")
}

fn has_system_tool(path: &str) -> bool {
    Path::new(path).exists()
}

fn run_with_stdin(bin: &str, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {bin}: {e}"));

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(err) = stdin.write_all(input) {
            assert_eq!(
                err.kind(),
                std::io::ErrorKind::BrokenPipe,
                "failed to write input to child stdin: {err}"
            );
        }
    }

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(
        output.status.success(),
        "{bin} {:?} exited with {}: {}",
        args,
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn prng_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.max(1);
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push((state & 0xff) as u8);
    }
    out
}

fn reference_corpora() -> Vec<(&'static str, Vec<u8>)> {
    let mut all_bytes: Vec<u8> = (0u8..=255u8).collect();
    all_bytes.extend_from_slice(b"Octet Reference Parity Check\n");

    let mut interior_zeros = Vec::new();
    interior_zeros.extend_from_slice(b"HEADER_ROW_16B!!");
    interior_zeros.extend_from_slice(&[0u8; 48]);
    interior_zeros.extend_from_slice(b"FOOTER_ROW_16B!!");

    let mut interior_two_zeros = Vec::new();
    interior_two_zeros.extend_from_slice(b"HEADER_ROW_16B!!");
    interior_two_zeros.extend_from_slice(&[0u8; 32]);
    interior_two_zeros.extend_from_slice(b"FOOTER_ROW_16B!!");

    vec![
        ("empty_0B", Vec::new()),
        ("single_1B", vec![0x41]),
        ("pair_2B", vec![0x41, 0x42]),
        ("triple_3B", vec![0x41, 0x42, 0x43]),
        ("word_4B", b"ABCD".to_vec()),
        ("partial_group_5B", b"ABCDE".to_vec()),
        ("odd_7B", b"ABCDEFG".to_vec()),
        ("half_row_8B", b"ABCDEFGH".to_vec()),
        ("almost_row_15B", b"ABCDEFGHIJKLMNO".to_vec()),
        ("exact_row_16B", b"ABCDEFGHIJKLMNOP".to_vec()),
        ("row_plus_one_17B", b"ABCDEFGHIJKLMNOPQ".to_vec()),
        ("two_rows_minus_one_31B", prng_bytes(31, 0x1337)),
        ("exact_two_rows_32B", prng_bytes(32, 0x4242)),
        ("three_rows_partial_35B", prng_bytes(35, 0x9001)),
        ("full_byte_table_285B", all_bytes),
        ("interior_3_zero_rows", interior_zeros),
        ("interior_2_zero_rows", interior_two_zeros),
        ("trailing_2_zero_rows_32B", vec![0u8; 32]),
        ("trailing_3_zero_rows_48B", vec![0u8; 48]),
        ("trailing_4_zero_rows_64B", vec![0u8; 64]),
        ("trailing_5_zero_rows_80B", vec![0u8; 80]),
        ("repeated_non_zero_64B", vec![b'A'; 64]),
        ("prng_binary_512B", prng_bytes(512, 0xcafe_babe_dead_beef)),
    ]
}

#[test]
fn test_xxd_classic_default_group_cols_upper_parity() {
    if !has_system_tool("/usr/bin/xxd") {
        return;
    }

    let flag_sets: &[&[&str]] = &[
        &[],
        &["-u"],
        &["-g", "1"],
        &["-g", "2"],
        &["-g", "4"],
        &["-g", "8"],
        &["-g", "0"],
        &["-c", "1"],
        &["-c", "7"],
        &["-c", "8"],
        &["-c", "12"],
        &["-c", "16"],
        &["-c", "24"],
        &["-c", "32"],
        &["-c", "12", "-g", "4", "-u"],
    ];

    for (name, payload) in reference_corpora() {
        for &flags in flag_sets {
            let expected = run_with_stdin("/usr/bin/xxd", flags, &payload);
            let mut octet_args = vec!["--classic", "--color=never"];
            octet_args.extend_from_slice(flags);
            let actual = run_with_stdin(octet_bin(), &octet_args, &payload);

            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "mismatch vs /usr/bin/xxd on corpus '{name}' with flags {flags:?}"
            );
        }
    }
}

#[test]
fn test_xxd_little_endian_and_binary_bits_parity() {
    if !has_system_tool("/usr/bin/xxd") {
        return;
    }

    let flag_sets: &[&[&str]] = &[
        &["-e"],
        &["-e", "-u"],
        &["-e", "-g", "2"],
        &["-e", "-g", "4"],
        &["-e", "-g", "8"],
        &["-e", "-c", "6"],
        &["-e", "-c", "8", "-g", "4"],
        &["-b"],
        &["-b", "-c", "4"],
        &["-b", "-g", "2"],
    ];

    let vim_le_probe =
        String::from_utf8_lossy(&run_with_stdin("/usr/bin/xxd", &["-e"], b"A")).into_owned();
    // GNU/Linux Vim `xxd -e` emits 3 spaces before the ASCII pane whereas macOS `tinyxxd`
    // and `octet` emit the standard 2 spaces.
    let is_vim_le_extra_space = vim_le_probe.contains("41                              A");

    for (name, payload) in reference_corpora() {
        for &flags in flag_sets {
            let expected_raw = run_with_stdin("/usr/bin/xxd", flags, &payload);
            let expected_str = String::from_utf8_lossy(&expected_raw);
            let expected_norm = if is_vim_le_extra_space && flags.contains(&"-e") {
                normalize_vim_le_extra_space(&expected_str, flags)
            } else {
                expected_str.into_owned()
            };

            let mut octet_args = vec!["--classic", "--color=never"];
            octet_args.extend_from_slice(flags);
            let actual = run_with_stdin(octet_bin(), &octet_args, &payload);

            assert_eq!(
                String::from_utf8_lossy(&actual),
                expected_norm,
                "mismatch vs /usr/bin/xxd on corpus '{name}' with flags {flags:?}"
            );
        }
    }
}

fn normalize_vim_le_extra_space(s: &str, flags: &[&str]) -> String {
    let mut cols = 16usize;
    let mut group_size = 4usize;
    let mut i = 0;
    while i < flags.len() {
        if flags[i] == "-c" && i + 1 < flags.len() {
            cols = flags[i + 1].parse().unwrap_or(16);
        } else if flags[i] == "-g" && i + 1 < flags.len() {
            group_size = flags[i + 1].parse().unwrap_or(4);
        }
        i += 1;
    }
    let groups = cols.div_ceil(group_size);
    let gap_idx = 10 + groups * (group_size * 2) + groups.saturating_sub(1);

    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        if line.len() > gap_idx + 2 && line.as_bytes().get(gap_idx) == Some(&b' ') {
            out.push_str(&line[..gap_idx]);
            out.push_str(&line[gap_idx + 1..]);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn test_xxd_autoskip_and_plain_mode_parity() {
    if !has_system_tool("/usr/bin/xxd") {
        return;
    }

    let flag_sets: &[&[&str]] = &[
        &["-a"],
        &["-a", "-c", "8"],
        &["-a", "-g", "4", "-u"],
        &["-p"],
        &["-p", "-u"],
        &["-p", "-c", "16"],
        &["-p", "-c", "0"],
    ];

    for (name, payload) in reference_corpora() {
        for &flags in flag_sets {
            let expected = run_with_stdin("/usr/bin/xxd", flags, &payload);
            let mut octet_args = vec!["--classic", "--color=never"];
            octet_args.extend_from_slice(flags);
            let actual = run_with_stdin(octet_bin(), &octet_args, &payload);

            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "mismatch vs /usr/bin/xxd on corpus '{name}' with flags {flags:?}"
            );
        }
    }
}

#[test]
fn test_xxd_seek_length_offset_and_c_include_file_parity() {
    if !has_system_tool("/usr/bin/xxd") {
        return;
    }

    let tmp_dir = std::env::temp_dir();
    let sample_path = tmp_dir.join("octet_xxd_parity_sample.bin");
    let data = prng_bytes(140, 0x2026_1101);
    std::fs::write(&sample_path, &data).expect("write temp sample");
    let path_str = sample_path.to_str().unwrap();

    let file_flag_sets: &[&[&str]] = &[
        &["-s", "16", "-l", "48"],
        &["-s", "0x20", "-l", "0x25", "-o", "0x1000"],
        &["-s", "-32"],
        &["-i"],
        &["-i", "-u"],
        &["-i", "-c", "8"],
    ];

    for &flags in file_flag_sets {
        let mut xxd_args = flags.to_vec();
        xxd_args.push(path_str);
        let expected = run_with_stdin("/usr/bin/xxd", &xxd_args, &[]);

        let mut octet_args = vec!["--classic", "--color=never"];
        octet_args.extend_from_slice(flags);
        octet_args.push(path_str);
        let actual = run_with_stdin(octet_bin(), &octet_args, &[]);

        let expected_str = String::from_utf8_lossy(&expected);
        let actual_str = String::from_utf8_lossy(&actual);
        assert_eq!(
            actual_str, expected_str,
            "file parity mismatch vs /usr/bin/xxd with flags {flags:?}"
        );
    }

    // Also verify stdin-mode `xxd -i` and `xxd -i -u` parity across corpora
    for (name, payload) in reference_corpora() {
        for flags in [&["-i"][..], &["-i", "-u"][..], &["-i", "-c", "8"][..]] {
            let expected = run_with_stdin("/usr/bin/xxd", flags, &payload);
            let mut octet_args = vec!["--classic", "--color=never"];
            octet_args.extend_from_slice(flags);
            let actual = run_with_stdin(octet_bin(), &octet_args, &payload);
            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "stdin -i parity mismatch vs /usr/bin/xxd on corpus '{name}' with flags {flags:?}"
            );
        }
    }

    let _ = std::fs::remove_file(&sample_path);
}

#[test]
fn test_hexdump_canonical_mode_reference_parity() {
    if !has_system_tool("/usr/bin/hexdump") {
        return;
    }

    let flag_sets: &[&[&str]] = &[
        &["-C"],
        &["-C", "-v"],
        &["-C", "-n", "24"],
        &["-C", "-s", "8", "-n", "19"],
    ];

    let tmp_path = std::env::temp_dir().join("octet_hexdump_parity_sample.bin");
    let path_str = tmp_path.to_str().unwrap();

    for (name, payload) in reference_corpora() {
        std::fs::write(&tmp_path, &payload).expect("write temp hexdump sample");
        for &flags in flag_sets {
            if flags.contains(&"-s") && payload.len() < 8 {
                continue;
            }
            let (expected, actual) = if flags.contains(&"-s") {
                let mut hd_args = flags.to_vec();
                hd_args.push(path_str);
                let exp = run_with_stdin("/usr/bin/hexdump", &hd_args, &[]);

                let mut oc_args = vec!["--color=never"];
                oc_args.extend_from_slice(flags);
                oc_args.push(path_str);
                let act = run_with_stdin(octet_bin(), &oc_args, &[]);
                (exp, act)
            } else {
                let exp = run_with_stdin("/usr/bin/hexdump", flags, &payload);
                let mut oc_args = vec!["--color=never"];
                oc_args.extend_from_slice(flags);
                let act = run_with_stdin(octet_bin(), &oc_args, &payload);
                (exp, act)
            };

            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "canonical parity mismatch vs /usr/bin/hexdump on corpus '{name}' with flags {flags:?}"
            );
        }
    }
    let _ = std::fs::remove_file(&tmp_path);
}

#[test]
fn test_cross_tool_bidirectional_reverse_interoperability() {
    if !has_system_tool("/usr/bin/xxd") {
        return;
    }

    for (name, payload) in reference_corpora() {
        // 1. /usr/bin/xxd -> octet -r
        let xxd_dump = run_with_stdin("/usr/bin/xxd", &[], &payload);
        let octet_rev = run_with_stdin(octet_bin(), &["-r"], &xxd_dump);
        assert_eq!(
            octet_rev, payload,
            "xxd -> octet -r failed for corpus '{name}'"
        );

        // 2. octet --classic -> /usr/bin/xxd -r
        let octet_dump = run_with_stdin(octet_bin(), &["--classic", "--color=never"], &payload);
        let xxd_rev = run_with_stdin("/usr/bin/xxd", &["-r"], &octet_dump);
        assert_eq!(
            xxd_rev, payload,
            "octet --classic -> xxd -r failed for corpus '{name}'"
        );

        // 3. /usr/bin/xxd -a -> octet -r
        let xxd_auto = run_with_stdin("/usr/bin/xxd", &["-a"], &payload);
        let octet_auto_rev = run_with_stdin(octet_bin(), &["-r"], &xxd_auto);
        assert_eq!(
            octet_auto_rev, payload,
            "xxd -a -> octet -r failed for corpus '{name}'"
        );

        // 4. /usr/bin/xxd -p -> octet -r -p
        let xxd_plain = run_with_stdin("/usr/bin/xxd", &["-p"], &payload);
        let octet_plain_rev = run_with_stdin(octet_bin(), &["-r", "-p"], &xxd_plain);
        assert_eq!(
            octet_plain_rev, payload,
            "xxd -p -> octet -r -p failed for corpus '{name}'"
        );

        // 5. octet -p -> /usr/bin/xxd -r -p
        let octet_plain = run_with_stdin(octet_bin(), &["-p"], &payload);
        let xxd_plain_rev = run_with_stdin("/usr/bin/xxd", &["-r", "-p"], &octet_plain);
        assert_eq!(
            xxd_plain_rev, payload,
            "octet -p -> xxd -r -p failed for corpus '{name}'"
        );

        // 6. /usr/bin/hexdump -C -> octet -r (including `*` elided runs at interior and EOF!)
        if has_system_tool("/usr/bin/hexdump") {
            let hd_out = run_with_stdin("/usr/bin/hexdump", &["-C"], &payload);
            let recovered = run_with_stdin(octet_bin(), &["-r"], &hd_out);
            assert_eq!(
                recovered, payload,
                "hexdump -C -> octet -r failed for corpus '{name}'"
            );
        }
    }

    // 7. Library-level Smart, Compact, Classic, & Canonical dumps round-trip cleanly
    let payload = prng_bytes(200, 0x55aa_55aa);
    for style in [
        ViewStyle::Smart,
        ViewStyle::Compact,
        ViewStyle::Classic,
        ViewStyle::Canonical,
    ] {
        let opts = FormatOptions {
            style,
            radix: Radix::Hex,
            color: true,
            ..FormatOptions::default()
        };
        let dump = render_dump(&payload, &opts);
        let recovered = reverse_dump(&dump, false).expect("library round-trip");
        assert_eq!(recovered, payload);
    }
}
