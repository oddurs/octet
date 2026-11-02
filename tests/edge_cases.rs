// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Edge-case, boundary-value, and adversarial input tests for `octet`.

use std::io::Write as _;
use std::process::{Command, Stdio};

use octet::{
    FormatOptions, Theme, ValueInspection, ViewStyle, compute_diff, inspect_header,
    parse_byte_count, render_ansi_to_svg, render_diff_ansi, render_diff_json, render_dump,
    render_json_report, reverse_dump, slice_buffer, strip_ansi,
};

#[test]
fn test_64bit_offsets_preserve_uniform_card_widths() {
    let data = b"0123456789abcdef0123456789abcdef";
    for &base_offset in &[
        0x1_0000_0000_u64,
        0x0001_0000_4000_u64,
        0x7fff_ffff_ffff_ffe0_u64,
        u64::MAX - 15,
    ] {
        let opts = FormatOptions {
            label: Some("vm_page.bin".to_owned()),
            style: ViewStyle::Smart,
            display_offset: base_offset,
            color: false,
            ..FormatOptions::default()
        };
        let rendered = render_dump(data, &opts);
        let widths: Vec<usize> = rendered
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| strip_ansi(l).chars().count())
            .collect();
        assert!(!widths.is_empty());
        let first_w = widths[0];
        for (idx, w) in widths.iter().enumerate() {
            assert_eq!(
                *w, first_w,
                "line {idx} width mismatch at 64-bit offset 0x{base_offset:x}:\n{rendered}"
            );
        }

        // Also verify binary diff card alignment at 64-bit offsets.
        let right = b"0123456789ABCDEF0123456789abcdef";
        let diff_rep = compute_diff(data, right, 8, base_offset, true);
        let diff_str =
            render_diff_ansi(&diff_rep, "left.bin", "right.bin", 8, &Theme::dark(), false);
        let diff_widths: Vec<usize> = diff_str
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| strip_ansi(l).chars().count())
            .collect();
        let diff_first = diff_widths[0];
        for (idx, w) in diff_widths.iter().enumerate() {
            assert_eq!(
                *w, diff_first,
                "diff line {idx} width mismatch at 64-bit offset 0x{base_offset:x}:\n{diff_str}"
            );
        }
    }
}

#[test]
fn test_reverse_sparse_offset_jumps_vs_star_elision() {
    // 1. Sparse offset jump WITHOUT `*` must pad the gap with 0x00 bytes (matching `xxd -r`).
    let sparse_dump = "\
00000000: 4142 4344                                ABCD
00000010: 4546 4748                                EFGH
";
    let reversed_sparse = reverse_dump(sparse_dump, false).expect("reverse sparse");
    let mut expected_sparse = vec![b'A', b'B', b'C', b'D'];
    expected_sparse.resize(16, 0x00);
    expected_sparse.extend_from_slice(b"EFGH");
    assert_eq!(reversed_sparse, expected_sparse);

    // 2. Elided offset jump WITH `*` must repeat the preceding row.
    let star_dump = "\
00000000: 4142 4344                                ABCD
*
00000010: 4546 4748                                EFGH
";
    let reversed_star = reverse_dump(star_dump, false).expect("reverse star");
    let mut expected_star = Vec::new();
    for _ in 0..4 {
        expected_star.extend_from_slice(b"ABCD");
    }
    expected_star.extend_from_slice(b"EFGH");
    assert_eq!(reversed_star, expected_star);

    // 3. Reversing tokens with `0x` / `0X` prefixes and in-place backward offset patching.
    let patch_dump = "\
00000000: 0x00 0x00 0x00 0x00
00000001: 0Xde 0Xad
";
    let reversed_patch = reverse_dump(patch_dump, false).expect("reverse patch");
    assert_eq!(reversed_patch, vec![0x00, 0xde, 0xad, 0x00]);
}

#[test]
fn test_uleb128_boundaries_and_eof_utf8_inspection() {
    // Valid ULEB128 maximum: u64::MAX = 9 bytes of 0xFF followed by 0x01.
    let mut u64_max_leb = [0xff_u8; 10];
    u64_max_leb[9] = 0x01;
    let insp_max = ValueInspection::at(&u64_max_leb, 0, 0).expect("inspect u64::MAX");
    assert_eq!(insp_max.uleb128, Some((u64::MAX, 10)));

    // Overflowing 10th byte: 9 bytes of 0xFF followed by 0x02 exceeds u64::MAX.
    let mut overflow_leb = [0xff_u8; 10];
    overflow_leb[9] = 0x02;
    let insp_over = ValueInspection::at(&overflow_leb, 0, 0).expect("inspect overflow");
    assert_eq!(insp_over.uleb128, None);

    // Unterminated 10-byte varint.
    let unterminated = [0x80_u8; 11];
    let insp_unterm = ValueInspection::at(&unterminated, 0, 0).expect("inspect unterminated");
    assert_eq!(insp_unterm.uleb128, None);

    // 4-byte UTF-8 emoji ('🦀' = F0 9F A6 80) at the very end of a buffer has all high bits set,
    // so `uleb128` is `None` (unterminated), yet `utf8_char` is `Some(('🦀', 4))` and MUST appear
    // in the rendered inspection card.
    let crab = "🦀".as_bytes();
    let insp_crab = ValueInspection::at(crab, 0, u64::MAX).expect("inspect crab");
    assert_eq!(insp_crab.utf8_char, Some(('🦀', 4)));
    assert_eq!(insp_crab.uleb128, None);
    let card = insp_crab.render_ansi(&Theme::dark(), false);
    assert!(
        card.contains("U+1F980") && card.contains("4 B"),
        "expected UTF-8 codepoint in inspector card even when ULEB128 is None:\n{card}"
    );
}

#[test]
fn test_adversarial_and_corrupted_magic_headers() {
    // 0-byte and 1-byte slices.
    assert!(inspect_header(&[]).is_none());
    assert!(inspect_header(&[0x89]).is_none());

    // Corrupted MZ/PE header with `e_lfanew = 0xFFFF_FFFF` and `0xFFFF_FFFE`.
    let mut mz = vec![0u8; 64];
    mz[0] = b'M';
    mz[1] = b'Z';
    mz[60..64].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());
    let pe_hdr = inspect_header(&mz).expect("MZ header should still parse");
    assert_eq!(pe_hdr.format_name, "PE / MZ Executable");

    // ZIP header with malicious control characters (`\n`, `\r`, `\x1b`) in filename.
    let mut zip = vec![0u8; 42];
    zip[0..4].copy_from_slice(b"PK\x03\x04");
    zip[26..28].copy_from_slice(&12u16.to_le_bytes());
    zip[30..42].copy_from_slice(b"evil\n\x1b[31m.c");
    let zip_hdr = inspect_header(&zip).expect("ZIP header");
    assert!(!zip_hdr.summary.contains('\n'));
    assert!(!zip_hdr.summary.contains('\x1b'));

    // Verify rendering Smart view on the malicious ZIP preserves uniform 106-column width.
    let rendered = render_dump(
        &zip,
        &FormatOptions {
            color: false,
            ..FormatOptions::default()
        },
    );
    for line in rendered.lines() {
        assert_eq!(line.chars().count(), 106);
    }
}

#[test]
fn test_svg_escapes_xml_control_characters() {
    let svg = render_ansi_to_svg("00000000  41\n", "title\x00\x07\x1b<>&\"'", &Theme::dark());
    assert!(!svg.contains('\x00'));
    assert!(!svg.contains('\x07'));
    assert!(!svg.contains('\x1b'));
    assert!(svg.contains("&lt;&gt;&amp;&quot;&apos;"));
}

#[test]
fn test_byte_count_units_and_cli_edge_cases() {
    assert_eq!(parse_byte_count("1GiB").unwrap(), 1024 * 1024 * 1024);
    assert_eq!(parse_byte_count("2TB").unwrap(), 2 * 1024_u64.pow(4));
    assert!(parse_byte_count("999999999999999999999").is_err());

    let data = b"hello world";
    let (sliced, off) = slice_buffer(data, Some("9999"), None).unwrap();
    assert!(sliced.is_empty());
    assert_eq!(off, data.len() as u64);

    let bin = env!("CARGO_BIN_EXE_octet");

    // Seeking past EOF on stdin in Canonical mode (-C -s 100) should clamp to actual bytes read (5).
    let mut child = Command::new(bin)
        .args(["-C", "-s", "100"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn octet");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"hello")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait octet");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "00000005\n");

    // Excessive column count (> 256) returns a clean error.
    let bad_cols = Command::new(bin)
        .args(["-c", "500", "Cargo.toml"])
        .output()
        .expect("run octet -c 500");
    assert!(!bad_cols.status.success());
    let stderr = String::from_utf8_lossy(&bad_cols.stderr);
    assert!(stderr.contains("exceeds maximum supported columns"));

    // Typo flag suggestion (`--col` -> `did you mean '--cols'?`).
    let typo = Command::new(bin)
        .args(["--col", "16"])
        .output()
        .expect("run octet --col");
    assert!(!typo.status.success());
    let typo_err = String::from_utf8_lossy(&typo.stderr);
    assert!(typo_err.contains("did you mean '--cols'?"));
}

#[test]
fn test_extended_magic_headers_and_json_report() {
    // 1. ISO BMFF (MP4 / HEIC / AVIF)
    let mp4 = b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x01";
    let mp4_hdr = inspect_header(mp4).expect("MP4 header");
    assert_eq!(mp4_hdr.format_name, "MP4 / ISO Base Media");

    let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00";
    let heic_hdr = inspect_header(heic).expect("HEIC header");
    assert_eq!(heic_hdr.format_name, "HEIC / HEIF Image");

    // 2. 7-Zip Archive
    let seven_z = b"7z\xbc\xaf\x27\x1c\x00\x04\x12\x34\x56\x78";
    let sz_hdr = inspect_header(seven_z).expect("7z header");
    assert_eq!(sz_hdr.format_name, "7-Zip Archive");

    // 3. PCAP & PCAPNG
    let pcap = [
        0xd4, 0xc3, 0xb2, 0xa1, 0x02, 0x00, 0x04, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00, 0x04,
        0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    let pcap_hdr = inspect_header(&pcap).expect("PCAP header");
    assert_eq!(pcap_hdr.format_name, "PCAP Packet Capture");
    assert!(pcap_hdr.summary.contains("v2.4"));

    let pcapng = [
        0x0a, 0x0d, 0x0d, 0x0a, 0x1c, 0x00, 0x00, 0x00, 0x4d, 0x3c, 0x2b, 0x1a,
    ];
    let pcapng_hdr = inspect_header(&pcapng).expect("PCAPNG header");
    assert_eq!(pcapng_hdr.format_name, "PCAPNG Next-Gen Capture");

    // 4. WOFF2 Font, Android DEX, and LLVM Bitcode
    let woff2 = b"wOF2\x00\x01\x00\x00\x00\x00\x10\x00";
    assert_eq!(inspect_header(woff2).unwrap().format_name, "WOFF2 Web Font");

    let dex = b"dex\n039\0\x12\x34\x56\x78";
    assert_eq!(
        inspect_header(dex).unwrap().format_name,
        "Android DEX Bytecode"
    );

    let llvm = b"BC\xc0\xde\x21\x0c\x00\x00";
    assert_eq!(inspect_header(llvm).unwrap().format_name, "LLVM IR Bitcode");

    // 5. JSON report generation (library & CLI `-j`)
    let opts = FormatOptions {
        label: Some("capture.pcap".to_owned()),
        ..FormatOptions::default()
    };
    let insp = ValueInspection::at(&pcap, 0, 0);
    let json = render_json_report(&pcap, &opts, insp.as_ref());
    assert!(json.contains("\"format_name\": \"PCAP Packet Capture\""));
    assert!(json.contains("\"u32_le\": 2712847316"));

    let diff_rep = compute_diff(&pcap, &pcapng, 8, 0, true);
    let diff_json = render_diff_json(&diff_rep, "a.pcap", "b.pcapng");
    assert!(diff_json.contains("\"mode\": \"diff\""));
    assert!(diff_json.contains("\"changed_bytes\":"));
}
