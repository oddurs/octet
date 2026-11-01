// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Integration test suite for the `octet` library.

use octet::{
    ByteClass, CodeLanguage, FormatOptions, Radix, Theme, ValueInspection, ViewStyle,
    classify_slice, compute_diff, compute_entropy, find_pattern_spans, generate_code,
    inspect_header, parse_byte_count, parse_search_pattern, render_ansi_to_svg, render_diff_ansi,
    render_dump, reverse_dump, slice_buffer,
};

#[test]
fn test_semantic_byte_classification_and_utf8() {
    let mut payload = Vec::new();
    payload.push(0x00); // Null
    payload.extend_from_slice(b"A "); // AsciiGraphic + AsciiWhitespace
    payload.push(0x07); // AsciiControl (BEL)
    payload.extend_from_slice("λ".as_bytes()); // 2-byte UTF-8 (0xCE, 0xBB)
    payload.push(0xfe); // HighByte (invalid standalone UTF-8)
    payload.push(0xff); // Sentinel

    let classes = classify_slice(&payload);
    assert_eq!(
        classes,
        vec![
            ByteClass::Null,
            ByteClass::AsciiGraphic,
            ByteClass::AsciiWhitespace,
            ByteClass::AsciiControl,
            ByteClass::Utf8MultiByte,
            ByteClass::Utf8MultiByte,
            ByteClass::HighByte,
            ByteClass::Sentinel,
        ]
    );

    let entropy = compute_entropy(&payload);
    assert!(entropy > 2.5 && entropy <= 3.0);
}

#[test]
fn test_magic_header_recognition_and_cafebabe_disambiguation() {
    // 1. PNG with IHDR (1920x1080, 8-bit RGBA)
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&1920u32.to_be_bytes());
    png.extend_from_slice(&1080u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&0x9a7c_1122u32.to_be_bytes());

    let png_hdr = inspect_header(&png).expect("PNG should be recognized");
    assert_eq!(png_hdr.format_name, "PNG Image");
    assert!(png_hdr.summary.contains("1920×1080 px"));
    assert!(png_hdr.summary.contains("Truecolor RGBA"));
    assert!(png_hdr.fields.len() >= 7);

    // Progressive PNG header recognition on a 24-byte slice
    let short_png_hdr = inspect_header(&png[..24]).expect("short PNG should be recognized");
    assert_eq!(short_png_hdr.fields.len(), 5);

    // 2. ELF 64-bit Little-Endian AArch64 PIE
    let mut elf = vec![0u8; 64];
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2; // 64-bit
    elf[5] = 1; // Little-Endian
    elf[6] = 1; // Version
    elf[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
    elf[18..20].copy_from_slice(&0xb7u16.to_le_bytes()); // AArch64
    elf[24..32].copy_from_slice(&0x0040_1000u64.to_le_bytes()); // e_entry

    let elf_hdr = inspect_header(&elf).expect("ELF should be recognized");
    assert_eq!(elf_hdr.format_name, "ELF Binary");
    assert!(elf_hdr.summary.contains("AArch64"));
    assert!(elf_hdr.summary.contains("ET_DYN"));

    // 3. Mach-O 64-bit ARM64 Executable
    let mut macho = vec![0u8; 64];
    macho[0..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
    macho[4..8].copy_from_slice(&0x0100_000cu32.to_le_bytes()); // ARM64
    macho[12..16].copy_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
    macho[16..20].copy_from_slice(&16u32.to_le_bytes()); // ncmds = 16
    macho[20..24].copy_from_slice(&1280u32.to_le_bytes()); // sizeofcmds
    macho[24..28].copy_from_slice(&0x0020_0085u32.to_le_bytes()); // PIE flags
    macho[32..36].copy_from_slice(&0x19u32.to_le_bytes()); // LC_SEGMENT_64
    macho[36..40].copy_from_slice(&72u32.to_le_bytes());
    macho[40..50].copy_from_slice(b"__PAGEZERO");

    let macho_hdr = inspect_header(&macho).expect("Mach-O should be recognized");
    assert_eq!(macho_hdr.format_name, "Mach-O 64-bit Binary");
    assert!(macho_hdr.summary.contains("ARM64"));

    // 4. Disambiguate 0xCAFEBABE: Mach-O Universal vs Java Class Bytecode
    let fat_bin = [0xca, 0xfe, 0xba, 0xbe, 0x00, 0x00, 0x00, 0x02];
    let fat_hdr = inspect_header(&fat_bin).expect("Fat Mach-O");
    assert_eq!(fat_hdr.format_name, "Mach-O Universal Binary");

    let java_class = [0xca, 0xfe, 0xba, 0xbe, 0x00, 0x00, 0x00, 65, 0x00, 32];
    let java_hdr = inspect_header(&java_class).expect("Java class");
    assert_eq!(java_hdr.format_name, "Java Class Bytecode");
    assert!(java_hdr.summary.contains("Java SE 21"));

    // 5. RIFF WAVE & WebAssembly
    let wave = b"RIFF\x24\x08\x00\x00WAVEfmt ";
    let wave_hdr = inspect_header(wave).expect("WAVE should be recognized");
    assert_eq!(wave_hdr.format_name, "RIFF WAVE Audio");

    let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07];
    let wasm_hdr = inspect_header(&wasm).expect("WASM should be recognized");
    assert_eq!(wasm_hdr.format_name, "WebAssembly Module");
}

#[test]
fn test_roundtrip_reverse_smart_trailing_zeros_bits_classic_and_plain() {
    let mut original = Vec::new();
    original.extend_from_slice(b"\x89PNG\r\n\x1a\nHello Octet World!\n");
    // Add 64 zero bytes in the middle
    original.extend_from_slice(&[0u8; 64]);
    original.extend_from_slice(b"Middle payload 1234567890ABCDEF!");
    // End with 64 zero bytes at EOF to test trailing elided rows!
    original.extend_from_slice(&[0u8; 64]);

    // 1. Smart mode (with ANSI colors, inline context rail, header/footer cards, and trailing elision)
    let smart_opts = FormatOptions::default();
    let smart_dump = render_dump(&original, &smart_opts);
    assert!(smart_dump.contains("elided"));
    let recovered_smart = reverse_dump(&smart_dump, false).expect("reverse smart dump");
    assert_eq!(recovered_smart, original);

    // 2. Binary (-b) mode round-trip
    let bits_opts = FormatOptions {
        style: ViewStyle::Compact,
        radix: Radix::Binary,
        cols: 6,
        group_size: 1,
        autoskip: false,
        ..FormatOptions::default()
    };
    let bits_dump = render_dump(&original[..24], &bits_opts);
    let recovered_bits = reverse_dump(&bits_dump, false).expect("reverse binary dump");
    assert_eq!(recovered_bits, &original[..24]);

    // 3. Classic xxd mode
    let classic_opts = FormatOptions {
        style: ViewStyle::Classic,
        autoskip: false,
        color: false,
        ..FormatOptions::default()
    };
    let classic_dump = render_dump(&original, &classic_opts);
    let recovered_classic = reverse_dump(&classic_dump, false).expect("reverse classic dump");
    assert_eq!(recovered_classic, original);

    // 4. Plain mode (-p / -r -p)
    let plain_opts = FormatOptions {
        style: ViewStyle::Plain,
        cols: 30,
        ..FormatOptions::default()
    };
    let plain_dump = render_dump(&original, &plain_opts);
    let recovered_plain = reverse_dump(&plain_dump, true).expect("reverse plain dump");
    assert_eq!(recovered_plain, original);
}

#[test]
fn test_codegen_languages() {
    let data = [0xde, 0xad, 0xbe, 0xef];
    let rs = generate_code(&data, Some("magic.bin"), CodeLanguage::Rust, 8, false);
    assert!(rs.contains("pub const MAGIC_BIN: &[u8; 4] = &["));
    assert!(rs.contains("0xde, 0xad, 0xbe, 0xef,"));

    let c = generate_code(&data, Some("magic.bin"), CodeLanguage::C, 8, true);
    assert!(c.contains("unsigned char magic_bin[]"));
    assert!(c.contains("0xDE, 0xAD, 0xBE, 0xEF"));
    assert!(c.contains("unsigned int magic_bin_len = 4;"));
}

#[test]
fn test_diff_and_value_inspector() {
    let left = b"OCTET_v1\x00\x00\x00\x00\x01\x02\x03\x04";
    let right = b"OCTET_v2\x00\x00\x00\x00\x01\x02\xff\x04";
    let report = compute_diff(left, right, 8, 0, true);
    assert_eq!(report.changed_bytes, 2);
    assert_eq!(report.changed_rows, 2);
    let diff_ansi = render_diff_ansi(&report, "v1.bin", "v2.bin", 8, &Theme::dark(), true);
    assert!(diff_ansi.contains("BINARY DIFF"));

    let insp = ValueInspection::at(&[0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0, 0x100)
        .expect("valid inspection");
    assert_eq!(insp.offset, 0x100);
    assert_eq!(insp.int32.unwrap().0.0, 42);
}

#[test]
fn test_search_slicing_and_svg_export() {
    let data = b"prefix_CAFE_middle_CAFE_suffix";
    let pat = parse_search_pattern("0x43414645").expect("hex pattern");
    let spans = find_pattern_spans(data, &pat);
    assert_eq!(spans, vec![7..11, 19..23]);

    assert_eq!(parse_byte_count("4KiB").unwrap(), 4096);
    assert_eq!(parse_byte_count("0x20").unwrap(), 32);

    let (sliced, off) = slice_buffer(data, Some("-6"), Some(6)).unwrap();
    assert_eq!(sliced, b"suffix");
    assert_eq!(off, 24);

    let opts = FormatOptions {
        radix: Radix::Hex,
        search_spans: spans,
        ..FormatOptions::default()
    };
    let ansi = render_dump(data, &opts);
    let svg = render_ansi_to_svg(&ansi, "octet test", &Theme::dark());
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("</svg>"));
}
