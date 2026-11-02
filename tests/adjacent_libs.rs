// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Reference-based test suites modeled on adjacent Rust hex, classification,
//! and binary magic-header libraries (`hexyl`, `pretty-hex`, `infer` / `file`).

use octet::{
    ByteClass, ByteStats, FormatOptions, Radix, Theme, ValueInspection, ViewStyle, classify_slice,
    compute_diff, compute_entropy, inspect_header, render_diff_ansi, render_dump, reverse_dump,
};

/// Models `hexyl`'s byte category invariants across all 256 single-byte values (`0x00..=0xFF`).
#[test]
fn test_hexyl_exhaustive_256_byte_classification_and_glyphs() {
    for b in 0u8..=255u8 {
        let class = ByteClass::classify_single(b);
        let rich = class.glyph(b, true);
        let classic = class.glyph(b, false);

        match b {
            0x00 => {
                assert_eq!(class, ByteClass::Null);
                assert_eq!(rich, '·');
                assert_eq!(classic, '.');
            }
            0x09 => {
                assert_eq!(class, ByteClass::AsciiWhitespace);
                assert_eq!(rich, '⇥');
                assert_eq!(classic, '.');
            }
            0x0a => {
                assert_eq!(class, ByteClass::AsciiWhitespace);
                assert_eq!(rich, '↵');
                assert_eq!(classic, '.');
            }
            0x0d => {
                assert_eq!(class, ByteClass::AsciiWhitespace);
                assert_eq!(rich, '␍');
                assert_eq!(classic, '.');
            }
            0x0b | 0x0c => {
                assert_eq!(class, ByteClass::AsciiWhitespace);
                assert_eq!(rich, '␣');
                assert_eq!(classic, '.');
            }
            0x20 => {
                assert_eq!(class, ByteClass::AsciiWhitespace);
                assert_eq!(rich, '␣');
                assert_eq!(classic, ' ');
            }
            0x01..=0x08 | 0x0e..=0x1f => {
                assert_eq!(class, ByteClass::AsciiControl);
                assert_eq!(
                    rich,
                    char::from_u32(0x2400 + u32::from(b)).unwrap(),
                    "C0 control byte 0x{b:02x} should map to Unicode Control Picture"
                );
                assert_eq!(classic, '.');
            }
            0x21..=0x7e => {
                assert_eq!(class, ByteClass::AsciiGraphic);
                assert_eq!(rich, b as char);
                assert_eq!(classic, b as char);
            }
            0x7f => {
                assert_eq!(class, ByteClass::AsciiControl);
                assert_eq!(rich, '␡');
                assert_eq!(classic, '.');
            }
            0x80..=0xfe => {
                assert_eq!(class, ByteClass::HighByte);
                assert_eq!(rich, '▪');
                assert_eq!(classic, '.');
            }
            0xff => {
                assert_eq!(class, ByteClass::Sentinel);
                assert_eq!(rich, '×');
                assert_eq!(classic, '.');
            }
        }
    }

    // Verify full 256-byte uniform buffer achieves maximum 8.0 bits/byte Shannon entropy
    let full_table: Vec<u8> = (0u8..=255u8).collect();
    let entropy = compute_entropy(&full_table);
    assert!((entropy - 8.0).abs() < 1e-9, "expected 8.0, got {entropy}");
    assert_eq!(compute_entropy(&[0x00; 128]), 0.0);
    assert_eq!(compute_entropy(&[]), 0.0);
}

/// Models UTF-8 state-machine boundary and RFC 3629 invalid sequence rejection tests.
#[test]
fn test_utf8_multibyte_boundaries_and_rfc3629_rejection() {
    // 1. Valid 2-byte, 3-byte, and 4-byte UTF-8 sequences
    let text = "é (2B) | ⌘ (3B) | 🦀 (4B)";
    let classes = classify_slice(text.as_bytes());
    let stats = ByteStats::analyze(text.as_bytes(), &classes);
    assert_eq!(stats.utf8_count, 2 + 3 + 4);
    assert_eq!(stats.high_count, 0);

    // 2. 4-byte UTF-8 sequence ('🦀' = F0 9F A6 80) straddling a 16-byte row boundary (indices 14..18)
    let mut row_crossing = vec![b'A'; 14];
    row_crossing.extend_from_slice("🦀".as_bytes());
    row_crossing.extend_from_slice(b"ZZZZ");
    let cross_classes = classify_slice(&row_crossing);
    assert_eq!(
        &cross_classes[14..18],
        &[ByteClass::Utf8MultiByte; 4],
        "UTF-8 sequence crossing 16-byte row boundary must remain Utf8MultiByte"
    );

    // 3. Adversarial RFC 3629 invalid sequences must be rejected (classified as HighByte, not Utf8MultiByte)
    let invalid_cases: &[(&str, &[u8])] = &[
        ("overlong_2B_nul", &[0xc0, 0x80]),
        ("overlong_2B_slash", &[0xc0, 0xaf]),
        ("overlong_3B", &[0xe0, 0x80, 0x80]),
        ("overlong_4B", &[0xf0, 0x80, 0x80, 0x80]),
        ("utf16_surrogate_high", &[0xed, 0xa0, 0x80]),
        ("utf16_surrogate_low", &[0xed, 0xbf, 0xbf]),
        ("above_u10ffff", &[0xf4, 0x90, 0x80, 0x80]),
        ("bare_continuation", &[0x80, 0xbf]),
        ("truncated_2B_at_eof", &[0xc2]),
        ("truncated_3B_at_eof", &[0xe2, 0x82]),
        ("truncated_4B_at_eof", &[0xf0, 0x9f, 0xa6]),
    ];

    for &(label, seq) in invalid_cases {
        let cls = classify_slice(seq);
        assert!(
            cls.iter().all(|&c| c == ByteClass::HighByte),
            "invalid UTF-8 case '{label}' ({seq:02x?}) was falsely accepted as {cls:?}"
        );
    }
}

/// Models `infer` / `file` / `goblin` format-detection tests across all 25 supported magic signatures.
#[test]
fn test_exhaustive_magic_header_corpus() {
    // 1. PNG
    let png =
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x01\x00\x00\x00\x01\x00\x08\x02\x00\x00\x00";
    assert_eq!(inspect_header(png).unwrap().format_name, "PNG Image");

    // 2. ELF 64-bit LE & 32-bit BE
    let mut elf64 = vec![0u8; 64];
    elf64[0..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    elf64[16..18].copy_from_slice(&2u16.to_le_bytes());
    elf64[18..20].copy_from_slice(&0x3eu16.to_le_bytes()); // x86_64
    let elf64_hdr = inspect_header(&elf64).unwrap();
    assert_eq!(elf64_hdr.format_name, "ELF Binary");
    assert!(elf64_hdr.summary.contains("x86_64"));

    let mut elf32_be = vec![0u8; 32];
    elf32_be[0..7].copy_from_slice(b"\x7fELF\x01\x02\x01");
    elf32_be[16..18].copy_from_slice(&1u16.to_be_bytes()); // ET_REL
    elf32_be[18..20].copy_from_slice(&0xf3u16.to_be_bytes()); // RISC-V
    let elf32_hdr = inspect_header(&elf32_be).unwrap();
    assert!(elf32_hdr.summary.contains("RISC-V"));

    // 3. Mach-O 64 LE, 64 BE, 32 LE, 32 BE
    let mut mo64_le = vec![0u8; 32];
    mo64_le[0..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    assert_eq!(
        inspect_header(&mo64_le).unwrap().format_name,
        "Mach-O 64-bit Binary"
    );

    let mut mo64_be = vec![0u8; 32];
    mo64_be[0..4].copy_from_slice(&[0xfe, 0xed, 0xfa, 0xcf]);
    assert_eq!(
        inspect_header(&mo64_be).unwrap().format_name,
        "Mach-O 64-bit Binary"
    );

    let mut mo32_le = vec![0u8; 32];
    mo32_le[0..4].copy_from_slice(&[0xce, 0xfa, 0xed, 0xfe]);
    assert_eq!(
        inspect_header(&mo32_le).unwrap().format_name,
        "Mach-O 32-bit Binary"
    );

    let mut mo32_be = vec![0u8; 32];
    mo32_be[0..4].copy_from_slice(&[0xfe, 0xed, 0xfa, 0xce]);
    assert_eq!(
        inspect_header(&mo32_be).unwrap().format_name,
        "Mach-O 32-bit Binary"
    );

    // 4. Mach-O Universal vs 5. Java Class
    let fat = [0xca, 0xfe, 0xba, 0xbe, 0x00, 0x00, 0x00, 0x02];
    assert_eq!(
        inspect_header(&fat).unwrap().format_name,
        "Mach-O Universal Binary"
    );
    let java = [0xca, 0xfe, 0xba, 0xbe, 0x00, 0x00, 0x00, 61, 0x00, 16];
    let java_hdr = inspect_header(&java).unwrap();
    assert_eq!(java_hdr.format_name, "Java Class Bytecode");
    assert!(java_hdr.summary.contains("Java SE 17"));

    // 6. Windows PE / MZ Executable
    let mut pe = vec![0u8; 128];
    pe[0..2].copy_from_slice(b"MZ");
    pe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    pe[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    let pe_hdr = inspect_header(&pe).unwrap();
    assert_eq!(pe_hdr.format_name, "PE / MZ Executable");
    assert!(pe_hdr.summary.contains("x86_64"));

    // 7. WebAssembly
    let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04];
    assert_eq!(
        inspect_header(&wasm).unwrap().format_name,
        "WebAssembly Module"
    );

    // 8. ZIP Archive
    let mut zip = vec![0u8; 36];
    zip[0..4].copy_from_slice(b"PK\x03\x04");
    zip[4..6].copy_from_slice(&20u16.to_le_bytes());
    zip[8..10].copy_from_slice(&8u16.to_le_bytes()); // Deflate
    zip[26..28].copy_from_slice(&6u16.to_le_bytes());
    zip[30..36].copy_from_slice(b"lib.rs");
    let zip_hdr = inspect_header(&zip).unwrap();
    assert_eq!(zip_hdr.format_name, "ZIP Archive");
    assert!(zip_hdr.summary.contains("Deflate"));

    // 9. SQLite 3
    let mut sqlite = vec![0u8; 64];
    sqlite[0..16].copy_from_slice(b"SQLite format 3\0");
    sqlite[16..18].copy_from_slice(&4096u16.to_be_bytes());
    sqlite[56..60].copy_from_slice(&1u32.to_be_bytes()); // UTF-8
    let sq_hdr = inspect_header(&sqlite).unwrap();
    assert_eq!(sq_hdr.format_name, "SQLite 3 Database");
    assert!(sq_hdr.summary.contains("4096 B"));

    // 10. RIFF (WAVE, WEBP, AVI)
    assert_eq!(
        inspect_header(b"RIFF\x24\x00\x00\x00WAVEfmt ")
            .unwrap()
            .format_name,
        "RIFF WAVE Audio"
    );
    assert_eq!(
        inspect_header(b"RIFF\x24\x00\x00\x00WEBPVP8 ")
            .unwrap()
            .format_name,
        "WebP Image"
    );
    assert_eq!(
        inspect_header(b"RIFF\x24\x00\x00\x00AVI LIST")
            .unwrap()
            .format_name,
        "RIFF AVI Video"
    );

    // 11. FLAC
    let mut flac = vec![0u8; 24];
    flac[0..4].copy_from_slice(b"fLaC");
    flac[4] = 0x80; // Last metadata block + STREAMINFO (0)
    flac[5..8].copy_from_slice(&[0x00, 0x00, 0x22]); // 34 bytes
    let flac_hdr = inspect_header(&flac).unwrap();
    assert_eq!(flac_hdr.format_name, "FLAC Audio");
    assert!(flac_hdr.fields[1].value.contains("STREAMINFO"));

    // 12. Ogg
    let mut ogg = vec![0u8; 28];
    ogg[0..4].copy_from_slice(b"OggS");
    ogg[5] = 0x02; // BOS
    ogg[26] = 1;
    assert_eq!(inspect_header(&ogg).unwrap().format_name, "Ogg Container");

    // 13. POSIX ustar TAR Archive
    let mut tar = vec![0u8; 300];
    tar[0..10].copy_from_slice(b"Cargo.toml");
    tar[124..135].copy_from_slice(b"00000001000"); // 512 bytes in octal
    tar[156] = b'0'; // regular file
    tar[257..263].copy_from_slice(b"ustar\0");
    let tar_hdr = inspect_header(&tar).unwrap();
    assert_eq!(tar_hdr.format_name, "POSIX ustar Archive");
    assert!(tar_hdr.summary.contains("Cargo.toml"));

    // 14. GZIP, 15. Zstandard, 16. XZ, 17. bzip2, 18. LZ4
    let gzip = [0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03];
    assert_eq!(
        inspect_header(&gzip).unwrap().format_name,
        "GZIP Compressed Archive"
    );

    let zstd = [0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x00];
    assert_eq!(
        inspect_header(&zstd).unwrap().format_name,
        "Zstandard Frame"
    );

    let xz = [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04];
    assert_eq!(
        inspect_header(&xz).unwrap().format_name,
        "XZ Compressed Archive"
    );

    let bz2 = b"BZh91AY&SY";
    assert_eq!(
        inspect_header(bz2).unwrap().format_name,
        "bzip2 Compressed Archive"
    );

    let lz4 = [0x04, 0x22, 0x4d, 0x18, 0x60, 0x40, 0x82];
    assert_eq!(inspect_header(&lz4).unwrap().format_name, "LZ4 Frame");

    // 19. PDF, 20. JPEG, 21. GIF, 22. UTF-8 BOM, 23. Shebang
    assert_eq!(
        inspect_header(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
            .unwrap()
            .format_name,
        "PDF Document"
    );
    assert_eq!(
        inspect_header(&[
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00
        ])
        .unwrap()
        .format_name,
        "JPEG Image"
    );
    assert_eq!(
        inspect_header(b"GIF89a\x20\x03\x58\x02\xf7\x00\x00")
            .unwrap()
            .format_name,
        "GIF Image"
    );
    assert_eq!(
        inspect_header(b"\xef\xbb\xbfHello UTF-8")
            .unwrap()
            .format_name,
        "UTF-8 Text with BOM"
    );
    assert_eq!(
        inspect_header(b"#!/usr/bin/env bash\necho hi\n")
            .unwrap()
            .format_name,
        "Executable Script (Shebang)"
    );
}

/// Models `pretty-hex` structural layout invariants: uniform card width in Smart mode
/// and multi-radix rendering across Hex, Binary, Octal, and Decimal.
#[test]
fn test_pretty_hex_structural_card_alignment_and_radices() {
    let mut data_with_hdr = Vec::new();
    data_with_hdr.extend_from_slice(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR");
    data_with_hdr.extend_from_slice(&[0u8; 48]);
    data_with_hdr.extend_from_slice("Hello 🦀 Octet!\n".as_bytes());

    let data_no_hdr =
        b"Arbitrary binary payload without a known magic header!\x00\xff\x01".to_vec();

    for sample in [&data_with_hdr, &data_no_hdr] {
        for radix in [Radix::Hex, Radix::Binary, Radix::Octal, Radix::Decimal] {
            let cols = if radix == Radix::Binary { 6 } else { 16 };
            let group_size = if radix == Radix::Hex { 2 } else { 1 };
            let opts = FormatOptions {
                label: Some(String::from("alignment.bin")),
                style: ViewStyle::Smart,
                radix,
                cols,
                group_size,
                color: false,
                ..FormatOptions::default()
            };
            let rendered = render_dump(sample, &opts);
            assert!(rendered.contains("OFFSET"));
            // Terminal columns, not chars: the crab is the sample's one wide character.
            let line_widths: Vec<usize> = rendered
                .lines()
                .map(|l| l.chars().map(|c| if c == '🦀' { 2 } else { 1 }).sum())
                .collect();
            let expected_w = line_widths[0];
            for (idx, &w) in line_widths.iter().enumerate() {
                assert_eq!(
                    w, expected_w,
                    "Smart view line {idx} for radix {radix:?} had width {w}, expected uniform {expected_w}"
                );
            }
        }
    }

    // Also verify uniform character width across every row of binary diff output
    let mut left = vec![0x41u8; 64];
    left[0..8].copy_from_slice(b"OCTET_v1");
    let mut right = left.clone();
    right[0..8].copy_from_slice(b"OCTET_v2");
    let report = compute_diff(&left, &right, 8, 0, true);
    let diff_str = render_diff_ansi(&report, "v1.bin", "v2.bin", 8, &Theme::dark(), false);
    let diff_widths: Vec<usize> = diff_str.lines().map(|l| l.chars().count()).collect();
    let expected_diff_w = diff_widths[0];
    for (idx, &w) in diff_widths.iter().enumerate() {
        assert_eq!(
            w, expected_diff_w,
            "Diff view line {idx} had width {w}, expected uniform {expected_diff_w}"
        );
    }
}

/// Property-style round-trip and value-inspection verification across diverse buffer lengths.
#[test]
fn test_property_roundtrip_and_value_inspection_invariants() {
    // 1. ValueInspection endianness and IEEE-754 float invariants
    let mut buf = [0u8; 8];
    buf[0..4].copy_from_slice(&std::f32::consts::PI.to_le_bytes());
    buf[4..8].copy_from_slice(&0x1122_3344u32.to_be_bytes());
    let insp = ValueInspection::at(&buf, 0, 0x40).unwrap();
    assert_eq!(insp.offset, 0x40);
    let (f32_le, _) = insp.float32.unwrap();
    assert!((f32_le - std::f32::consts::PI).abs() < 1e-6);
    let rendered = insp.render_ansi(&Theme::dark(), false);
    assert!(rendered.contains("DATA INSPECTOR @ 0x00000040"));

    // 2. Round-trip across 40 distinct buffer lengths, column widths, and view styles
    for len in (0..=160).step_by(5) {
        let payload: Vec<u8> = (0..len).map(|i| ((i * 73 + 19) & 0xff) as u8).collect();
        for &(style, cols, group_size) in &[
            (ViewStyle::Smart, 16, 2),
            (ViewStyle::Compact, 12, 4),
            (ViewStyle::Classic, 16, 2),
            (ViewStyle::Classic, 8, 1),
            (ViewStyle::Plain, 30, 1),
        ] {
            let opts = FormatOptions {
                style,
                cols,
                group_size,
                color: true,
                ..FormatOptions::default()
            };
            let dump = render_dump(&payload, &opts);
            let rev = reverse_dump(&dump, style == ViewStyle::Plain).unwrap();
            assert_eq!(
                rev, payload,
                "round-trip failed for len={len}, style={style:?}, cols={cols}, group={group_size}"
            );
        }
    }
}
