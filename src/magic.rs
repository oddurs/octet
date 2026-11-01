// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Known file header recognizer and byte-level structural field annotator.

use std::ops::Range;

/// A single annotated structural field within a binary header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldAnnotation {
    /// Byte offset range (relative to the start of the analyzed buffer).
    pub range: Range<usize>,
    /// Short field identifier (e.g., `"IHDR.width"`, `"e_entry"`).
    pub name: &'static str,
    /// Decoded human-readable value and interpretation.
    pub value: String,
    /// Accent color index into the theme's annotation palette.
    pub accent_index: usize,
}

/// Complete structural recognition result for a binary buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderInspection {
    /// Concise format title (e.g., `"PNG Image"`, `"Mach-O 64-bit Executable"`).
    pub format_name: &'static str,
    /// Standard MIME type or category identifier.
    pub mime_or_category: &'static str,
    /// High-level one-line summary of the header metadata.
    pub summary: String,
    /// Ordered list of byte-range field annotations.
    pub fields: Vec<FieldAnnotation>,
}

impl HeaderInspection {
    /// Finds the first field annotation covering the given byte index, if any.
    #[must_use]
    pub fn field_at(&self, byte_index: usize) -> Option<&FieldAnnotation> {
        self.fields.iter().find(|f| f.range.contains(&byte_index))
    }
}

/// Inspects the leading bytes of `data` and returns structural annotations if a known format
/// is recognized.
#[must_use]
pub fn inspect_header(data: &[u8]) -> Option<HeaderInspection> {
    if data.len() < 2 {
        return None;
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(parse_png(data));
    }
    if data.starts_with(b"\x7fELF") {
        return Some(parse_elf(data));
    }
    if let Some(macho_or_class) = try_parse_macho_or_javaclass(data) {
        return Some(macho_or_class);
    }
    if data.starts_with(b"PK\x03\x04") {
        return Some(parse_zip(data));
    }
    if data.starts_with(b"\0asm") {
        return Some(parse_wasm(data));
    }
    if data.starts_with(b"SQLite format 3\0") {
        return Some(parse_sqlite(data));
    }
    if data.starts_with(b"RIFF") && data.len() >= 12 {
        return Some(parse_riff(data));
    }
    if data.starts_with(b"fLaC") {
        return Some(parse_flac(data));
    }
    if data.starts_with(b"OggS") {
        return Some(parse_ogg(data));
    }
    if data.len() >= 262 && &data[257..262] == b"ustar" {
        return Some(parse_tar(data));
    }
    if data.starts_with(b"MZ") {
        return Some(parse_pe_mz(data));
    }
    if data.starts_with(&[0x1f, 0x8b]) {
        return Some(parse_gzip(data));
    }
    if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Some(parse_zstd(data));
    }
    if data.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        return Some(parse_xz(data));
    }
    if data.starts_with(b"BZh") && data.len() >= 4 && data[3].is_ascii_digit() {
        return Some(parse_bzip2(data));
    }
    if data.starts_with(&[0x04, 0x22, 0x4d, 0x18]) {
        return Some(parse_lz4(data));
    }
    if data.starts_with(b"%PDF-") {
        return Some(parse_pdf(data));
    }
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(parse_jpeg(data));
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some(parse_gif(data));
    }
    if data.starts_with(b"#!") {
        return Some(parse_shebang(data));
    }
    if data.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Some(HeaderInspection {
            format_name: "UTF-8 Text with BOM",
            mime_or_category: "text/plain; charset=utf-8",
            summary: "Byte Order Mark (EF BB BF) followed by UTF-8 stream".to_owned(),
            fields: vec![FieldAnnotation {
                range: 0..3,
                name: "UTF8.bom",
                value: "U+FEFF Byte Order Mark".to_owned(),
                accent_index: 0,
            }],
        });
    }
    None
}

fn read_u16_le(data: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let bytes: [u8; 2] = data.get(offset..end)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn read_u16_be(data: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let bytes: [u8; 2] = data.get(offset..end)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

fn read_u32_le(data: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn read_u64_le(data: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let bytes: [u8; 8] = data.get(offset..end)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn read_u64_be(data: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let bytes: [u8; 8] = data.get(offset..end)?.try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

fn sanitize_header_str(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect()
}

fn parse_png(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..8,
        name: "PNG.magic",
        value: "\\x89PNG\\r\\n\\x1a\\n (8-byte signature)".to_owned(),
        accent_index: 0,
    }];
    let mut summary = String::from("Portable Network Graphics");

    if let Some(len) = read_u32_be(data, 8) {
        fields.push(FieldAnnotation {
            range: 8..12,
            name: "IHDR.length",
            value: format!("{len} bytes"),
            accent_index: 1,
        });
    }
    if data.len() >= 16 && &data[12..16] == b"IHDR" {
        fields.push(FieldAnnotation {
            range: 12..16,
            name: "IHDR.type",
            value: "\"IHDR\" (Image Header Chunk)".to_owned(),
            accent_index: 2,
        });
        if let Some(width) = read_u32_be(data, 16) {
            fields.push(FieldAnnotation {
                range: 16..20,
                name: "IHDR.width",
                value: format!("{width} px"),
                accent_index: 3,
            });
            if let Some(height) = read_u32_be(data, 20) {
                summary = format!("{width}×{height} px");
                fields.push(FieldAnnotation {
                    range: 20..24,
                    name: "IHDR.height",
                    value: format!("{height} px"),
                    accent_index: 4,
                });
                if data.len() >= 26 {
                    let bit_depth = data[24];
                    let color_type = data[25];
                    let color_desc = match color_type {
                        0 => "Grayscale",
                        2 => "Truecolor RGB",
                        3 => "Indexed Palette",
                        4 => "Grayscale + Alpha",
                        6 => "Truecolor RGBA",
                        _ => "Unknown Color",
                    };
                    summary = format!("{width}×{height} px, {bit_depth}-bit {color_desc}");
                    fields.push(FieldAnnotation {
                        range: 24..26,
                        name: "IHDR.pixel_fmt",
                        value: format!("{bit_depth}-bit {color_desc} (type={color_type})"),
                        accent_index: 5,
                    });
                    if data.len() >= 29 {
                        let interlace = if data[28] == 1 {
                            "Adam7 Interlaced"
                        } else {
                            "Non-interlaced"
                        };
                        summary = format!(
                            "{width}×{height} px, {bit_depth}-bit {color_desc}, {interlace}"
                        );
                        fields.push(FieldAnnotation {
                            range: 26..29,
                            name: "IHDR.method",
                            value: format!("Deflate / Adaptive / {interlace}"),
                            accent_index: 0,
                        });
                    }
                    if let Some(crc) = read_u32_be(data, 29) {
                        fields.push(FieldAnnotation {
                            range: 29..33,
                            name: "IHDR.crc32",
                            value: format!("0x{crc:08x}"),
                            accent_index: 1,
                        });
                    }
                }
            }
        }
    }

    HeaderInspection {
        format_name: "PNG Image",
        mime_or_category: "image/png",
        summary,
        fields,
    }
}

fn parse_elf(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "e_ident.magic",
        value: "\\x7fELF".to_owned(),
        accent_index: 0,
    }];

    let is_64 = data.get(4).copied() == Some(2);
    let is_le = data.get(5).copied() != Some(2);
    let class_str = match data.get(4) {
        Some(1) => "ELF32 (32-bit)",
        Some(2) => "ELF64 (64-bit)",
        _ => "Unknown Class",
    };
    let endian_str = match data.get(5) {
        Some(1) => "Little-Endian",
        Some(2) => "Big-Endian",
        _ => "Unknown Endian",
    };
    let osabi_str = match data.get(7) {
        Some(0) => "System V",
        Some(3) => "GNU/Linux",
        Some(9) => "FreeBSD",
        Some(255) => "Standalone",
        _ => "UNIX ABI",
    };

    if data.len() >= 8 {
        fields.push(FieldAnnotation {
            range: 4..8,
            name: "e_ident.info",
            value: format!("{class_str}, {endian_str}, {osabi_str}"),
            accent_index: 1,
        });
    }

    let read16 = |off| {
        if is_le {
            read_u16_le(data, off)
        } else {
            read_u16_be(data, off)
        }
    };
    let read32 = |off| {
        if is_le {
            read_u32_le(data, off)
        } else {
            read_u32_be(data, off)
        }
    };
    let read64 = |off| {
        if is_le {
            read_u64_le(data, off)
        } else {
            read_u64_be(data, off)
        }
    };

    let mut obj_type = "Binary";
    let mut arch_name = "Unknown Arch";

    if let Some(et) = read16(16) {
        obj_type = match et {
            1 => "ET_REL (Relocatable)",
            2 => "ET_EXEC (Executable)",
            3 => "ET_DYN (Shared / PIE)",
            4 => "ET_CORE (Core Dump)",
            _ => "ET_OTHER",
        };
        fields.push(FieldAnnotation {
            range: 16..18,
            name: "e_type",
            value: format!("{obj_type} (0x{et:04x})"),
            accent_index: 2,
        });
    }

    if let Some(em) = read16(18) {
        arch_name = match em {
            0x03 => "x86 (i386)",
            0x28 => "ARM32",
            0x3e => "x86_64 (AMD64)",
            0xb7 => "AArch64 (ARM64)",
            0xf3 => "RISC-V",
            _ => "Custom ISA",
        };
        fields.push(FieldAnnotation {
            range: 18..20,
            name: "e_machine",
            value: format!("{arch_name} (0x{em:04x})"),
            accent_index: 3,
        });
    }

    if is_64 {
        if let Some(entry) = read64(24) {
            fields.push(FieldAnnotation {
                range: 24..32,
                name: "e_entry",
                value: format!("0x{entry:08x} (Entry Point)"),
                accent_index: 4,
            });
        }
        if let Some(phoff) = read64(32) {
            fields.push(FieldAnnotation {
                range: 32..40,
                name: "e_phoff",
                value: format!("0x{phoff:x} ({phoff} B offset)"),
                accent_index: 5,
            });
        }
        if let Some(shoff) = read64(40) {
            fields.push(FieldAnnotation {
                range: 40..48,
                name: "e_shoff",
                value: format!("0x{shoff:x} ({shoff} B offset)"),
                accent_index: 0,
            });
        }
        if let (Some(phnum), Some(shnum)) = (read16(56), read16(60)) {
            fields.push(FieldAnnotation {
                range: 56..62,
                name: "e_phnum/shnum",
                value: format!("{phnum} program hdrs, {shnum} section hdrs"),
                accent_index: 1,
            });
        }
    } else if let Some(entry) = read32(24) {
        fields.push(FieldAnnotation {
            range: 24..28,
            name: "e_entry",
            value: format!("0x{entry:08x} (Entry Point)"),
            accent_index: 4,
        });
    }

    HeaderInspection {
        format_name: "ELF Binary",
        mime_or_category: "application/x-executable",
        summary: format!("{class_str} {obj_type} for {arch_name} ({endian_str})"),
        fields,
    }
}

fn try_parse_macho_or_javaclass(data: &[u8]) -> Option<HeaderInspection> {
    let magic_le = read_u32_le(data, 0)?;
    let magic_be = read_u32_be(data, 0)?;

    if magic_be == 0xcafebabe {
        let nfat_or_ver = read_u32_be(data, 4).unwrap_or(0);
        let major_ver = read_u16_be(data, 6).unwrap_or(0);
        // Disambiguate Java `.class` (major_version >= 45 in bytes 6..8) from Mach-O Fat Binary
        // (`nfat_arch` in bytes 4..8 is a small count 1..=32).
        if (1..=32).contains(&nfat_or_ver) {
            let mut fields = vec![
                FieldAnnotation {
                    range: 0..4,
                    name: "fat_magic",
                    value: "0xcafebabe (Mach-O Universal / Fat Binary)".to_owned(),
                    accent_index: 0,
                },
                FieldAnnotation {
                    range: 4..8,
                    name: "nfat_arch",
                    value: format!("{nfat_or_ver} architecture slices"),
                    accent_index: 1,
                },
            ];
            if data.len() >= 28 {
                let cputype = read_u32_be(data, 8).unwrap_or(0);
                let offset = read_u32_be(data, 16).unwrap_or(0);
                let size = read_u32_be(data, 20).unwrap_or(0);
                fields.push(FieldAnnotation {
                    range: 8..28,
                    name: "fat_arch[0]",
                    value: format!(
                        "cpu={} offset=0x{offset:x} size={size} B",
                        macho_cpu_name(cputype)
                    ),
                    accent_index: 2,
                });
            }
            return Some(HeaderInspection {
                format_name: "Mach-O Universal Binary",
                mime_or_category: "application/x-mach-binary",
                summary: format!(
                    "Universal fat binary containing {nfat_or_ver} architecture slice(s)"
                ),
                fields,
            });
        } else if major_ver >= 45 {
            let minor_ver = read_u16_be(data, 4).unwrap_or(0);
            let jdk = major_ver.saturating_sub(44);
            let mut fields = vec![
                FieldAnnotation {
                    range: 0..4,
                    name: "Class.magic",
                    value: "0xcafebabe (JVM Class File)".to_owned(),
                    accent_index: 0,
                },
                FieldAnnotation {
                    range: 4..8,
                    name: "Class.version",
                    value: format!("v{major_ver}.{minor_ver} (Java SE {jdk})"),
                    accent_index: 1,
                },
            ];
            if let Some(cp_count) = read_u16_be(data, 8) {
                fields.push(FieldAnnotation {
                    range: 8..10,
                    name: "constant_pool",
                    value: format!("{} entries", cp_count.saturating_sub(1)),
                    accent_index: 2,
                });
            }
            return Some(HeaderInspection {
                format_name: "Java Class Bytecode",
                mime_or_category: "application/java-vm",
                summary: format!("JVM Classfile v{major_ver}.{minor_ver} (Java SE {jdk})"),
                fields,
            });
        }
    }

    let (is_64, is_le) = match magic_le {
        0xfeedfacf => (true, true),
        0xfeedface => (false, true),
        0xcffaedfe => (true, false),
        0xcefaedfe => (false, false),
        _ => return None,
    };

    let read32 = |off| {
        if is_le {
            read_u32_le(data, off)
        } else {
            read_u32_be(data, off)
        }
    };

    let cputype = read32(4).unwrap_or(0);
    let cpusubtype = read32(8).unwrap_or(0);
    let filetype = read32(12).unwrap_or(0);
    let ncmds = read32(16).unwrap_or(0);
    let sizeofcmds = read32(20).unwrap_or(0);
    let flags = read32(24).unwrap_or(0);

    let cpu_str = macho_cpu_name(cputype);
    let ftype_str = match filetype {
        1 => "MH_OBJECT (Relocatable)",
        2 => "MH_EXECUTE (Executable)",
        6 => "MH_DYLIB (Dynamic Library)",
        8 => "MH_BUNDLE (Bundle)",
        10 => "MH_DSYM (Debug Symbols)",
        _ => "Mach-O Image",
    };

    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "magic",
        value: format!(
            "0x{magic_le:08x} ({})",
            if is_64 { "MH_MAGIC_64" } else { "MH_MAGIC" }
        ),
        accent_index: 0,
    }];
    if data.len() >= 8 {
        fields.push(FieldAnnotation {
            range: 4..8,
            name: "cputype",
            value: format!("{cpu_str} (0x{cputype:08x})"),
            accent_index: 1,
        });
    }
    if data.len() >= 12 {
        fields.push(FieldAnnotation {
            range: 8..12,
            name: "cpusubtype",
            value: format!("0x{cpusubtype:08x}"),
            accent_index: 2,
        });
    }
    if data.len() >= 16 {
        fields.push(FieldAnnotation {
            range: 12..16,
            name: "filetype",
            value: format!("{ftype_str} ({filetype})"),
            accent_index: 3,
        });
    }
    if data.len() >= 24 {
        fields.push(FieldAnnotation {
            range: 16..24,
            name: "load_cmds",
            value: format!("{ncmds} cmds ({sizeofcmds} bytes total)"),
            accent_index: 4,
        });
    }
    if data.len() >= 28 {
        fields.push(FieldAnnotation {
            range: 24..28,
            name: "flags",
            value: format!(
                "0x{flags:08x}{}",
                if flags & 0x200000 != 0 { " (PIE)" } else { "" }
            ),
            accent_index: 5,
        });
    }

    let hdr_size = if is_64 { 32 } else { 28 };
    if let (Some(cmd), Some(cmdsize)) = (read32(hdr_size), read32(hdr_size + 4)) {
        let cmd_name = match cmd {
            0x19 => "LC_SEGMENT_64",
            0x01 => "LC_SEGMENT",
            0x02 => "LC_SYMTAB",
            0x0c => "LC_LOAD_DYLIB",
            0x8000_0028 => "LC_MAIN",
            0x32 => "LC_BUILD_VERSION",
            _ => "LC_COMMAND",
        };
        fields.push(FieldAnnotation {
            range: hdr_size..hdr_size + 8,
            name: "cmd[0].hdr",
            value: format!("{cmd_name} (size={cmdsize} B)"),
            accent_index: 0,
        });
        if (cmd == 0x19 || cmd == 0x01) && data.len() >= hdr_size + 24 {
            let seg_bytes = &data[hdr_size + 8..hdr_size + 24];
            let seg_len = seg_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(seg_bytes.len());
            if seg_len > 0 {
                let seg_name = sanitize_header_str(&seg_bytes[..seg_len]);
                fields.push(FieldAnnotation {
                    range: hdr_size + 8..hdr_size + 24,
                    name: "cmd[0].segname",
                    value: format!("\"{seg_name}\""),
                    accent_index: 1,
                });
            }
        }
    }

    Some(HeaderInspection {
        format_name: if is_64 {
            "Mach-O 64-bit Binary"
        } else {
            "Mach-O 32-bit Binary"
        },
        mime_or_category: "application/x-mach-binary",
        summary: format!("{cpu_str} {ftype_str}, {ncmds} load commands"),
        fields,
    })
}

const fn macho_cpu_name(cputype: u32) -> &'static str {
    match cputype {
        0x0100_0007 => "x86_64",
        0x0000_0007 => "x86 (i386)",
        0x0100_000c => "ARM64 (AArch64)",
        0x0000_000c => "ARM32",
        _ => "Unknown CPU",
    }
}

fn parse_zip(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "ZIP.magic",
        value: "PK\\x03\\x04 (Local File Header)".to_owned(),
        accent_index: 0,
    }];
    let mut summary = String::from("ZIP Archive");

    if data.len() >= 30 {
        let ver = read_u16_le(data, 4).unwrap_or(0);
        let method = read_u16_le(data, 8).unwrap_or(0);
        let crc = read_u32_le(data, 14).unwrap_or(0);
        let comp_size = read_u32_le(data, 18).unwrap_or(0);
        let uncomp_size = read_u32_le(data, 22).unwrap_or(0);
        let name_len = usize::from(read_u16_le(data, 26).unwrap_or(0));
        let extra_len = usize::from(read_u16_le(data, 28).unwrap_or(0));

        let method_str = match method {
            0 => "Stored (Uncompressed)",
            8 => "Deflate",
            12 => "BZIP2",
            14 => "LZMA",
            93 => "Zstandard",
            _ => "Compressed",
        };

        fields.push(FieldAnnotation {
            range: 4..10,
            name: "ZIP.method",
            value: format!("v{}.{}, {method_str} (method={method})", ver / 10, ver % 10),
            accent_index: 1,
        });
        fields.push(FieldAnnotation {
            range: 14..18,
            name: "ZIP.crc32",
            value: format!("0x{crc:08x}"),
            accent_index: 2,
        });
        fields.push(FieldAnnotation {
            range: 18..26,
            name: "ZIP.sizes",
            value: format!("{comp_size} B compressed -> {uncomp_size} B raw"),
            accent_index: 3,
        });
        fields.push(FieldAnnotation {
            range: 26..30,
            name: "ZIP.lengths",
            value: format!("filename={name_len} B, extra={extra_len} B"),
            accent_index: 4,
        });

        if name_len > 0 && data.len() >= 30 + name_len {
            let name = sanitize_header_str(&data[30..30 + name_len]);
            summary = format!("First entry \"{name}\" ({method_str}, {uncomp_size} B)");
            fields.push(FieldAnnotation {
                range: 30..30 + name_len,
                name: "ZIP.filename",
                value: format!("\"{name}\""),
                accent_index: 5,
            });
        }
    }

    HeaderInspection {
        format_name: "ZIP Archive",
        mime_or_category: "application/zip",
        summary,
        fields,
    }
}

fn parse_wasm(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "WASM.magic",
        value: "\\0asm (WebAssembly Binary Module)".to_owned(),
        accent_index: 0,
    }];
    let ver = read_u32_le(data, 4).unwrap_or(1);
    if data.len() >= 8 {
        fields.push(FieldAnnotation {
            range: 4..8,
            name: "WASM.version",
            value: format!("Version {ver}"),
            accent_index: 1,
        });
    }
    if data.len() >= 10 {
        let sec_id = data[8];
        let sec_name = match sec_id {
            0 => "Custom Section (0)",
            1 => "Type Section (1)",
            2 => "Import Section (2)",
            3 => "Function Section (3)",
            4 => "Table Section (4)",
            5 => "Memory Section (5)",
            6 => "Global Section (6)",
            7 => "Export Section (7)",
            10 => "Code Section (10)",
            11 => "Data Section (11)",
            _ => "Section",
        };
        fields.push(FieldAnnotation {
            range: 8..10,
            name: "WASM.section[0]",
            value: format!("{sec_name}, payload={} B", data[9]),
            accent_index: 2,
        });
    }

    HeaderInspection {
        format_name: "WebAssembly Module",
        mime_or_category: "application/wasm",
        summary: format!("WebAssembly binary module v{ver}"),
        fields,
    }
}

fn parse_sqlite(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..16,
        name: "SQLite.magic",
        value: "\"SQLite format 3\\0\"".to_owned(),
        accent_index: 0,
    }];
    let mut summary = String::from("SQLite 3 Database");

    if data.len() >= 18 {
        let raw_page = read_u16_be(data, 16).unwrap_or(0);
        let page_size = if raw_page == 1 {
            65536
        } else {
            u32::from(raw_page)
        };
        fields.push(FieldAnnotation {
            range: 16..18,
            name: "SQLite.page_size",
            value: format!("{page_size} bytes"),
            accent_index: 1,
        });
        summary = format!("SQLite 3 Database ({page_size} B pages)");
        if let Some(db_pages) = read_u32_be(data, 28) {
            fields.push(FieldAnnotation {
                range: 28..32,
                name: "SQLite.db_pages",
                value: format!("{db_pages} pages"),
                accent_index: 2,
            });
            summary = format!("SQLite 3 Database ({page_size} B pages, {db_pages} total pages)");
        }
    }

    HeaderInspection {
        format_name: "SQLite 3 Database",
        mime_or_category: "application/vnd.sqlite3",
        summary,
        fields,
    }
}

fn parse_riff(data: &[u8]) -> HeaderInspection {
    let chunk_size = read_u32_le(data, 4).unwrap_or(0);
    let form_type = sanitize_header_str(&data[8..12]);
    let (format_name, mime) = match form_type.as_str() {
        "WAVE" => ("RIFF WAVE Audio", "audio/wav"),
        "WEBP" => ("WebP Image", "image/webp"),
        "AVI " => ("RIFF AVI Video", "video/x-msvideo"),
        _ => ("RIFF Container", "application/octet-stream"),
    };
    HeaderInspection {
        format_name,
        mime_or_category: mime,
        summary: format!("{format_name} (form=\"{form_type}\", payload={chunk_size} B)"),
        fields: vec![
            FieldAnnotation {
                range: 0..4,
                name: "RIFF.magic",
                value: "\"RIFF\" (Resource Interchange File Format)".to_owned(),
                accent_index: 0,
            },
            FieldAnnotation {
                range: 4..8,
                name: "RIFF.size",
                value: format!("{chunk_size} bytes"),
                accent_index: 1,
            },
            FieldAnnotation {
                range: 8..12,
                name: "RIFF.form",
                value: format!("\"{form_type}\""),
                accent_index: 2,
            },
        ],
    }
}

fn parse_flac(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "FLAC.magic",
        value: "\"fLaC\" (Free Lossless Audio Codec)".to_owned(),
        accent_index: 0,
    }];
    if data.len() >= 8 {
        let is_last = (data[4] & 0x80) != 0;
        let block_type = data[4] & 0x7f;
        let len = (u32::from(data[5]) << 16) | (u32::from(data[6]) << 8) | u32::from(data[7]);
        fields.push(FieldAnnotation {
            range: 4..8,
            name: "FLAC.block[0]",
            value: format!("type={block_type} (STREAMINFO), last={is_last}, len={len} B"),
            accent_index: 1,
        });
    }
    HeaderInspection {
        format_name: "FLAC Audio",
        mime_or_category: "audio/flac",
        summary: "Free Lossless Audio Codec stream".to_owned(),
        fields,
    }
}

fn parse_ogg(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "Ogg.capture",
        value: "\"OggS\" (Ogg Page Header)".to_owned(),
        accent_index: 0,
    }];
    if data.len() >= 18 {
        let ver = data[4];
        let hdr_type = data[5];
        let serial = read_u32_le(data, 14).unwrap_or(0);
        fields.push(FieldAnnotation {
            range: 4..6,
            name: "Ogg.ver_flags",
            value: format!("version={ver}, flags=0x{hdr_type:02x}"),
            accent_index: 1,
        });
        fields.push(FieldAnnotation {
            range: 14..18,
            name: "Ogg.serial",
            value: format!("0x{serial:08x}"),
            accent_index: 2,
        });
    }
    HeaderInspection {
        format_name: "Ogg Container",
        mime_or_category: "application/ogg",
        summary: "Ogg bitstream container page".to_owned(),
        fields,
    }
}

fn parse_tar(data: &[u8]) -> HeaderInspection {
    let name_end = data[..100].iter().position(|&b| b == 0).unwrap_or(100);
    let first_entry = sanitize_header_str(&data[..name_end]);
    HeaderInspection {
        format_name: "POSIX ustar Archive",
        mime_or_category: "application/x-tar",
        summary: format!("POSIX tar archive (first entry: \"{first_entry}\")"),
        fields: vec![
            FieldAnnotation {
                range: 0..name_end.max(1),
                name: "tar.name",
                value: format!("\"{first_entry}\""),
                accent_index: 0,
            },
            FieldAnnotation {
                range: 257..262,
                name: "tar.magic",
                value: "\"ustar\" (POSIX IEEE P1003.1)".to_owned(),
                accent_index: 1,
            },
        ],
    }
}

fn parse_pe_mz(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..2,
        name: "DOS.e_magic",
        value: "\"MZ\" (DOS / PE Executable Header)".to_owned(),
        accent_index: 0,
    }];
    let mut summary = String::from("MS-DOS / PE Executable");

    if let Some(lfanew) = read_u32_le(data, 60) {
        fields.push(FieldAnnotation {
            range: 60..64,
            name: "DOS.e_lfanew",
            value: format!("0x{lfanew:08x} (PE header offset)"),
            accent_index: 1,
        });
        if let Ok(pe_off) = usize::try_from(lfanew)
            && let Some(pe_sig_end) = pe_off.checked_add(4)
            && data.get(pe_off..pe_sig_end) == Some(b"PE\0\0")
        {
            fields.push(FieldAnnotation {
                range: pe_off..pe_sig_end,
                name: "PE.signature",
                value: "\"PE\\0\\0\" (Portable Executable)".to_owned(),
                accent_index: 2,
            });
            if let Some(coff_end) = pe_sig_end.checked_add(2)
                && let Some(machine) = read_u16_le(data, pe_sig_end)
            {
                let arch = match machine {
                    0x8664 => "x86_64 (AMD64)",
                    0xaa64 => "ARM64",
                    0x014c => "x86 (i386)",
                    _ => "COFF Machine",
                };
                summary = format!("Windows Portable Executable ({arch})");
                fields.push(FieldAnnotation {
                    range: pe_sig_end..coff_end,
                    name: "COFF.Machine",
                    value: format!("{arch} (0x{machine:04x})"),
                    accent_index: 3,
                });
            }
        }
    }

    HeaderInspection {
        format_name: "PE / MZ Executable",
        mime_or_category: "application/vnd.microsoft.portable-executable",
        summary,
        fields,
    }
}

fn parse_gzip(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..2,
        name: "GZIP.magic",
        value: "0x1f8b (GZIP Stream)".to_owned(),
        accent_index: 0,
    }];
    if data.len() >= 10 {
        let method = if data[2] == 8 { "Deflate" } else { "Unknown" };
        let os = match data[9] {
            0 => "FAT",
            3 => "Unix",
            7 => "macOS",
            11 => "NTFS",
            255 => "Unknown OS",
            _ => "Other OS",
        };
        fields.push(FieldAnnotation {
            range: 2..4,
            name: "GZIP.method_flg",
            value: format!("{method} (flags=0x{:02x})", data[3]),
            accent_index: 1,
        });
        fields.push(FieldAnnotation {
            range: 4..10,
            name: "GZIP.mtime_os",
            value: format!("OS={os} (0x{:02x})", data[9]),
            accent_index: 2,
        });
    }
    HeaderInspection {
        format_name: "GZIP Compressed Archive",
        mime_or_category: "application/gzip",
        summary: "GZIP Deflate compressed data stream".to_owned(),
        fields,
    }
}

fn parse_zstd(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..4,
        name: "ZSTD.magic",
        value: "0xfd2fb528 (Zstandard Frame)".to_owned(),
        accent_index: 0,
    }];
    if data.len() >= 5 {
        fields.push(FieldAnnotation {
            range: 4..5,
            name: "ZSTD.fhd",
            value: format!("Frame Header Descriptor (0x{:02x})", data[4]),
            accent_index: 1,
        });
    }
    HeaderInspection {
        format_name: "Zstandard Frame",
        mime_or_category: "application/zstd",
        summary: "RFC 8878 Zstandard compressed frame".to_owned(),
        fields,
    }
}

fn parse_xz(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..6,
        name: "XZ.magic",
        value: "\\xfd7zXZ\\0 (XZ Compressed Stream)".to_owned(),
        accent_index: 0,
    }];
    if data.len() >= 8 {
        fields.push(FieldAnnotation {
            range: 6..8,
            name: "XZ.stream_flags",
            value: format!("0x{:02x}{:02x}", data[6], data[7]),
            accent_index: 1,
        });
    }
    HeaderInspection {
        format_name: "XZ Compressed Archive",
        mime_or_category: "application/x-xz",
        summary: "LZMA2 XZ compressed stream".to_owned(),
        fields,
    }
}

fn parse_bzip2(data: &[u8]) -> HeaderInspection {
    let level = data[3] as char;
    HeaderInspection {
        format_name: "bzip2 Compressed Archive",
        mime_or_category: "application/x-bzip2",
        summary: format!("bzip2 stream (block size = {level}00 kB)"),
        fields: vec![FieldAnnotation {
            range: 0..4,
            name: "BZIP2.header",
            value: format!("\"BZh{level}\" ({level}00 kB block size)"),
            accent_index: 0,
        }],
    }
}

fn parse_lz4(data: &[u8]) -> HeaderInspection {
    let _ = data;
    HeaderInspection {
        format_name: "LZ4 Frame",
        mime_or_category: "application/x-lz4",
        summary: "LZ4 compressed frame (0x184D2204)".to_owned(),
        fields: vec![FieldAnnotation {
            range: 0..4,
            name: "LZ4.magic",
            value: "0x184d2204 (LZ4 Frame Magic)".to_owned(),
            accent_index: 0,
        }],
    }
}

fn parse_pdf(data: &[u8]) -> HeaderInspection {
    let end = data
        .iter()
        .take(16)
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or_else(|| data.len().min(8));
    let ver = sanitize_header_str(&data[..end]);
    HeaderInspection {
        format_name: "PDF Document",
        mime_or_category: "application/pdf",
        summary: format!("Portable Document Format ({ver})"),
        fields: vec![FieldAnnotation {
            range: 0..end,
            name: "PDF.header",
            value: format!("\"{ver}\""),
            accent_index: 0,
        }],
    }
}

fn parse_jpeg(data: &[u8]) -> HeaderInspection {
    let mut fields = vec![FieldAnnotation {
        range: 0..2,
        name: "JPEG.soi",
        value: "0xffd8 (Start of Image)".to_owned(),
        accent_index: 0,
    }];
    if data.len() >= 4 {
        fields.push(FieldAnnotation {
            range: 2..4,
            name: "JPEG.marker",
            value: format!("0xff{:02x} (APP/SOF Marker)", data[3]),
            accent_index: 1,
        });
    }
    HeaderInspection {
        format_name: "JPEG Image",
        mime_or_category: "image/jpeg",
        summary: "JPEG bitstream (SOI + APP markers)".to_owned(),
        fields,
    }
}

fn parse_gif(data: &[u8]) -> HeaderInspection {
    let sig = sanitize_header_str(&data[..6]);
    let mut fields = vec![FieldAnnotation {
        range: 0..6,
        name: "GIF.signature",
        value: format!("\"{sig}\""),
        accent_index: 0,
    }];
    let mut summary = format!("GIF Image ({sig})");
    if data.len() >= 10 {
        let w = read_u16_le(data, 6).unwrap_or(0);
        let h = read_u16_le(data, 8).unwrap_or(0);
        summary = format!("{sig} {w}×{h} px Logical Screen");
        fields.push(FieldAnnotation {
            range: 6..10,
            name: "GIF.dimensions",
            value: format!("{w}×{h} px"),
            accent_index: 1,
        });
    }
    HeaderInspection {
        format_name: "GIF Image",
        mime_or_category: "image/gif",
        summary,
        fields,
    }
}

fn parse_shebang(data: &[u8]) -> HeaderInspection {
    let end = data
        .iter()
        .take(80)
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or_else(|| data.len().min(32));
    let line = sanitize_header_str(&data[..end]);
    HeaderInspection {
        format_name: "Executable Script (Shebang)",
        mime_or_category: "text/x-script",
        summary: format!("Interpreter directive: {line}"),
        fields: vec![FieldAnnotation {
            range: 0..end,
            name: "Script.shebang",
            value: format!("\"{line}\""),
            accent_index: 0,
        }],
    }
}
