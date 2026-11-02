// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Regression tests for header recognition, search-pattern parsing, and codegen identifiers.

use octet::{CodeLanguage, Error, HeaderInspection, generate_code, inspect_header};
use octet::{FieldAnnotation, parse_search_pattern};

fn field<'a>(hdr: &'a HeaderInspection, name: &str) -> &'a FieldAnnotation {
    hdr.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("missing field {name} in {hdr:?}"))
}

#[test]
fn search_pattern_rejects_non_ascii_hex_without_panicking() {
    for query in ["0x0é0", "0xé0", "0x00é", "0x€€"] {
        match parse_search_pattern(query) {
            Err(Error::InvalidArgument(_)) => {}
            other => panic!("{query:?}: expected InvalidArgument, got {other:?}"),
        }
    }
}

#[test]
fn search_pattern_rejects_signs_and_accepts_whitespace() {
    for query in ["0x+1+2", "0x-1", "0x+f", "0x0g"] {
        assert!(
            matches!(parse_search_pattern(query), Err(Error::InvalidArgument(_))),
            "{query:?} should be rejected"
        );
    }
    assert_eq!(
        parse_search_pattern("0x de AD\tbe ef").unwrap(),
        [0xde, 0xad, 0xbe, 0xef]
    );
}

#[test]
fn wasm_section_size_is_uleb128() {
    let mut wasm = b"\0asm\x01\0\0\0\x01\xac\x02".to_vec();
    wasm.resize(wasm.len() + 300, 0);
    let hdr = inspect_header(&wasm).expect("WASM");
    let sec = field(&hdr, "WASM.section[0]");
    assert!(sec.value.contains("payload=300 B"), "{}", sec.value);
    assert_eq!(sec.range, 8..11);
}

#[test]
fn wasm_truncated_section_size_is_not_misread() {
    let hdr = inspect_header(b"\0asm\x01\0\0\0\x01\xac").expect("WASM");
    let sec = field(&hdr, "WASM.section[0]");
    assert!(!sec.value.contains("payload=172"), "{}", sec.value);
    assert_eq!(sec.range, 8..9);
}

#[test]
fn pcapng_byte_order_magic_selects_endianness() {
    let be = [
        0x0a, 0x0d, 0x0d, 0x0a, 0x00, 0x00, 0x00, 0x1c, 0x1a, 0x2b, 0x3c, 0x4d,
    ];
    let le = [
        0x0a, 0x0d, 0x0d, 0x0a, 0x1c, 0x00, 0x00, 0x00, 0x4d, 0x3c, 0x2b, 0x1a,
    ];
    for (bytes, order) in [(&be, "big-endian"), (&le, "little-endian")] {
        let hdr = inspect_header(bytes).expect("PCAPNG");
        assert_eq!(field(&hdr, "SHB.total_len").value, "28 bytes");
        let bom = &field(&hdr, "SHB.byte_order").value;
        assert!(bom.starts_with("0x1a2b3c4d"), "{bom}");
        assert!(bom.contains(order), "{bom}");
    }
}

#[test]
fn pcapng_unknown_byte_order_does_not_guess_lengths() {
    let bad = [
        0x0a, 0x0d, 0x0d, 0x0a, 0x1c, 0x00, 0x00, 0x00, 0xde, 0xad, 0xbe, 0xef,
    ];
    let hdr = inspect_header(&bad).expect("PCAPNG");
    assert!(hdr.fields.iter().all(|f| f.name != "SHB.total_len"));
}

/// A DOS header as linkers emit it, with `e_lfanew` pointing at `pe_offset`.
fn dos_header(len: usize, pe_offset: u32) -> Vec<u8> {
    let mut mz = vec![0u8; len];
    mz[0..2].copy_from_slice(b"MZ");
    mz[2..4].copy_from_slice(&0x90u16.to_le_bytes());
    mz[4..6].copy_from_slice(&3u16.to_le_bytes());
    mz[8..10].copy_from_slice(&4u16.to_le_bytes());
    mz[0x3c..0x40].copy_from_slice(&pe_offset.to_le_bytes());
    mz
}

#[test]
fn mz_prose_is_not_an_executable() {
    let prose = b"MZ is a valid prefix for many words, and this file is plain prose text \
                  that happens to start with those two capital letters.\n";
    assert!(prose.len() >= 0x40);
    assert!(inspect_header(prose).is_none());
}

#[test]
fn mz_requires_full_dos_header() {
    let full = dos_header(0x40, 0x40);
    assert!(inspect_header(&full).is_some());
    assert!(inspect_header(&full[..0x3f]).is_none());
}

#[test]
fn minimal_pe_is_annotated() {
    let mut pe = dos_header(0x100, 0x80);
    pe[0x80..0x84].copy_from_slice(b"PE\0\0");
    pe[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    let hdr = inspect_header(&pe).expect("PE");
    assert_eq!(hdr.format_name, "PE / MZ Executable");
    assert!(hdr.summary.contains("x86_64"), "{}", hdr.summary);
    assert_eq!(field(&hdr, "PE.signature").range, 0x80..0x84);
    assert_eq!(field(&hdr, "COFF.Machine").range, 0x84..0x86);
}

#[test]
fn pe_signature_wins_over_odd_dos_fields() {
    // Packed and hand-crafted PEs leave the DOS fields as garbage; the loader only reads
    // `e_magic` and `e_lfanew`.
    let mut pe = vec![0xffu8; 0x48];
    pe[0..2].copy_from_slice(b"MZ");
    pe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    pe[0x44..0x46].copy_from_slice(&0xaa64u16.to_le_bytes());
    let hdr = inspect_header(&pe).expect("PE");
    assert!(hdr.summary.contains("ARM64"), "{}", hdr.summary);
}

#[test]
fn truncated_pe_view_is_labelled_dos() {
    // `octet -l 64 app.exe` sees only the DOS header; `e_lfanew` points past the view.
    let hdr = inspect_header(&dos_header(0x40, 0xf8)).expect("MS-DOS");
    assert_eq!(hdr.summary, "MS-DOS / PE Executable");
    assert!(hdr.fields.iter().all(|f| f.name != "PE.signature"));
}

#[test]
fn mz_with_misaligned_or_missing_pe_and_garbage_dos_fields_is_rejected() {
    let mut garbage = vec![b'x'; 0x100];
    garbage[0..2].copy_from_slice(b"MZ");
    garbage[0x3c..0x40].copy_from_slice(&0x41u32.to_le_bytes());
    garbage[0x41..0x45].copy_from_slice(b"PE\0\0");
    assert!(inspect_header(&garbage).is_none());
}

/// Returns the first identifier declared by the generated snippet.
fn declared_ident(code: &str, lang: CodeLanguage) -> &str {
    let first = code.lines().next().expect("declaration line");
    let rest = match lang {
        CodeLanguage::C => first.strip_prefix("unsigned char "),
        CodeLanguage::Rust | CodeLanguage::TypeScript => first
            .strip_prefix("pub const ")
            .or_else(|| first.strip_prefix("export const ")),
        CodeLanguage::Go => first.strip_prefix("var "),
        CodeLanguage::Python => Some(first),
        CodeLanguage::Zig => first.strip_prefix("pub const "),
        CodeLanguage::CSharp => first.strip_prefix("public static readonly byte[] "),
    }
    .unwrap_or_else(|| panic!("unexpected declaration {first:?}"));
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn codegen_renames_reserved_words() {
    let cases: &[(CodeLanguage, &str, &str)] = &[
        (CodeLanguage::C, "const", "const_"),
        (CodeLanguage::C, "int", "int_"),
        (CodeLanguage::C, "class", "class_"),
        (CodeLanguage::C, "main", "main_"),
        (CodeLanguage::Go, "const", "const_"),
        (CodeLanguage::Go, "type", "type_"),
        (CodeLanguage::Go, "byte", "byte_"),
        (CodeLanguage::Go, "init", "init_"),
        (CodeLanguage::Zig, "const", "const_"),
        (CodeLanguage::Zig, "type", "type_"),
        (CodeLanguage::Zig, "u8", "u8_"),
        (CodeLanguage::Zig, "i1024", "i1024_"),
        (CodeLanguage::Zig, "null", "null_"),
        (CodeLanguage::Rust, "fn", "FN"),
        (CodeLanguage::Python, "def", "DEF"),
        (CodeLanguage::TypeScript, "class", "CLASS"),
        (CodeLanguage::CSharp, "class", "CLASS"),
    ];
    for &(lang, name, want) in cases {
        let code = generate_code(&[1, 2], Some(name), lang, 8, false);
        assert_eq!(
            declared_ident(&code, lang),
            want,
            "{lang:?} {name:?}:\n{code}"
        );
    }
    let c = generate_code(&[1], Some("const"), CodeLanguage::C, 8, false);
    assert!(c.contains("unsigned int const__len = 1;"), "{c}");
}

#[test]
fn codegen_leading_digit_matches_xxd_in_c() {
    let c = generate_code(&[1], Some("9lives"), CodeLanguage::C, 8, false);
    assert!(c.contains("unsigned char __9lives[]"), "{c}");
    assert!(c.contains("unsigned int __9lives_len = 1;"), "{c}");
    let rs = generate_code(&[1], Some("9lives"), CodeLanguage::Rust, 8, false);
    assert_eq!(declared_ident(&rs, CodeLanguage::Rust), "_9LIVES");
}
