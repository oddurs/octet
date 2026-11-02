// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Multi-language source code array generator (`-i` / `--include`).

use std::fmt::Write as _;

use crate::error::{Error, Result};

/// Target programming language for `-i` / `--include` array generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLanguage {
    /// C `unsigned char[]` and `unsigned int` length (`xxd -i` default).
    C,
    /// Rust `pub const DATA: &[u8; N] = &[...];`.
    Rust,
    /// Go `var data = []byte{...}`.
    Go,
    /// Python `DATA = bytes([...])`.
    Python,
    /// TypeScript / JavaScript `export const DATA = new Uint8Array([...]);`.
    TypeScript,
    /// Zig `pub const octet_data = [_]u8{ ... };`.
    Zig,
    /// C# `public static readonly byte[] OctetData = new byte[] { ... };`.
    CSharp,
}

impl CodeLanguage {
    /// Parses a language identifier string (`c`, `rust`, `rs`, `go`, `py`, `python`, `ts`, `js`,
    /// `zig`, `cs`, `csharp`).
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "c" | "h" | "cpp" => Ok(Self::C),
            "rust" | "rs" => Ok(Self::Rust),
            "go" | "golang" => Ok(Self::Go),
            "py" | "python" => Ok(Self::Python),
            "ts" | "typescript" | "js" | "javascript" => Ok(Self::TypeScript),
            "zig" => Ok(Self::Zig),
            "cs" | "csharp" | "c#" => Ok(Self::CSharp),
            other => Err(Error::InvalidArgument(format!(
                "unsupported codegen language '{other}' (expected: rust, c, go, python, ts, zig, csharp)"
            ))),
        }
    }
}

/// Generates a formatted byte-array literal in the specified [`CodeLanguage`].
#[must_use]
pub fn generate_code(
    data: &[u8],
    var_name: Option<&str>,
    lang: CodeLanguage,
    cols: usize,
    upper: bool,
) -> String {
    let cols = cols.max(1);
    let is_c = lang == CodeLanguage::C;
    let mut out = declaration_open(var_name, lang, data.len() as u64);
    write_hex_grid(&mut out, data, cols, upper, is_c, grid_indent(lang), is_c);
    out.push_str(&declaration_close(var_name, lang, data.len() as u64));
    out
}

/// The lines before the byte grid. Bare C output (`xxd -i` reading stdin) has none.
pub(crate) fn declaration_open(var_name: Option<&str>, lang: CodeLanguage, len: u64) -> String {
    let ident = identifier_for(var_name.unwrap_or("octet_data"), lang);
    match lang {
        CodeLanguage::C if var_name.is_none() => String::new(),
        CodeLanguage::C => format!("unsigned char {ident}[] = {{\n"),
        CodeLanguage::Rust => format!("pub const {ident}: &[u8; {len}] = &[\n"),
        CodeLanguage::Go => format!("var {ident} = []byte{{\n"),
        CodeLanguage::Python => format!("{ident} = bytes([\n"),
        CodeLanguage::TypeScript => format!("export const {ident} = new Uint8Array([\n"),
        CodeLanguage::Zig => format!("pub const {ident} = [_]u8{{\n"),
        CodeLanguage::CSharp => format!("public static readonly byte[] {ident} = new byte[] {{\n"),
    }
}

/// The lines after the byte grid; C declares the length here, as xxd does.
pub(crate) fn declaration_close(var_name: Option<&str>, lang: CodeLanguage, len: u64) -> String {
    match lang {
        CodeLanguage::C if var_name.is_none() => String::new(),
        CodeLanguage::C => {
            let ident = identifier_for(var_name.unwrap_or("octet_data"), lang);
            format!("}};\nunsigned int {ident}_len = {len};\n")
        }
        CodeLanguage::Rust => String::from("];\n"),
        CodeLanguage::Go => String::from("}\n"),
        CodeLanguage::Python => String::from("])\n"),
        CodeLanguage::TypeScript => String::from("]);\n"),
        CodeLanguage::Zig | CodeLanguage::CSharp => String::from("};\n"),
    }
}

/// The indentation of each grid row, in the language's customary style.
pub(crate) fn grid_indent(lang: CodeLanguage) -> &'static str {
    match lang {
        CodeLanguage::C | CodeLanguage::TypeScript => "  ",
        CodeLanguage::Go => "\t",
        CodeLanguage::Rust | CodeLanguage::Python | CodeLanguage::Zig | CodeLanguage::CSharp => {
            "    "
        }
    }
}

fn write_hex_grid(
    out: &mut String,
    data: &[u8],
    cols: usize,
    upper: bool,
    upper_prefix_x: bool,
    indent: &str,
    omit_last_comma: bool,
) {
    let total = data.len();
    for (row_idx, chunk) in data.chunks(cols).enumerate() {
        let _ = write!(out, "{indent}");
        for (col_idx, &byte) in chunk.iter().enumerate() {
            let abs_idx = row_idx * cols + col_idx;
            let is_last = abs_idx + 1 == total;
            if upper {
                if upper_prefix_x {
                    let _ = write!(out, "0X{byte:02X}");
                } else {
                    let _ = write!(out, "0x{byte:02X}");
                }
            } else {
                let _ = write!(out, "0x{byte:02x}");
            }
            if !(is_last && omit_last_comma) {
                let _ = write!(out, ",");
            }
            if col_idx + 1 < chunk.len() {
                let _ = write!(out, " ");
            }
        }
        let _ = writeln!(out);
    }
}

/// Derives the declared name for `lang`: sanitized, upper-cased where the language convention
/// is a constant, and suffixed with `_` when it would collide with a reserved word.
fn identifier_for(raw: &str, lang: CodeLanguage) -> String {
    // xxd prefixes a leading digit with "__", so C output keeps that for `-i` parity.
    let digit_prefix = if lang == CodeLanguage::C { "__" } else { "_" };
    let mut ident = sanitize_identifier(raw, digit_prefix);
    let reserved = match lang {
        CodeLanguage::C => C_RESERVED.contains(&ident.as_str()),
        CodeLanguage::Go => GO_RESERVED.contains(&ident.as_str()),
        CodeLanguage::Zig => is_zig_reserved(&ident),
        // Every keyword in these languages contains a lower-case letter, so the upper-cased
        // name cannot collide with one.
        CodeLanguage::Rust
        | CodeLanguage::Python
        | CodeLanguage::TypeScript
        | CodeLanguage::CSharp => {
            ident.make_ascii_uppercase();
            false
        }
    };
    if reserved {
        ident.push('_');
    }
    ident
}

fn sanitize_identifier(raw: &str, digit_prefix: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    for (i, ch) in raw.chars().enumerate() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if i == 0 && ch.is_ascii_digit() {
                s.push_str(digit_prefix);
            }
            s.push(ch);
        } else {
            s.push('_');
        }
    }
    if s.chars().all(|c| c == '_') {
        String::from("octet_data")
    } else {
        s
    }
}

/// C and C++ keywords (C23 and C++23), since `-i c` output is also pasted into C++ sources,
/// plus `main`, which C++ rejects and C makes undefined behavior as a variable.
const C_RESERVED: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "co_await",
    "co_return",
    "co_yield",
    "compl",
    "concept",
    "const",
    "const_cast",
    "consteval",
    "constexpr",
    "constinit",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "main",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "nullptr",
    "operator",
    "or",
    "or_eq",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "typeof",
    "typeof_unqual",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
    "xor_eq",
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_BitInt",
    "_Bool",
    "_Complex",
    "_Decimal128",
    "_Decimal32",
    "_Decimal64",
    "_Generic",
    "_Imaginary",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
];

/// Go keywords plus predeclared identifiers: shadowing `byte` breaks the `[]byte` literal
/// itself, and shadowing the rest breaks any code in the same package that uses them. `init`
/// and `main` cannot be declared as package-level variables.
const GO_RESERVED: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "any",
    "bool",
    "byte",
    "comparable",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "true",
    "false",
    "iota",
    "nil",
    "append",
    "cap",
    "clear",
    "close",
    "complex",
    "copy",
    "delete",
    "imag",
    "len",
    "make",
    "max",
    "min",
    "new",
    "panic",
    "print",
    "println",
    "real",
    "recover",
    "init",
    "main",
];

/// Zig keywords and primitive names; Zig rejects any declaration that shadows a primitive.
const ZIG_RESERVED: &[&str] = &[
    "addrspace",
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "linksection",
    "noalias",
    "noinline",
    "nosuspend",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
    "while",
    "anyerror",
    "anyopaque",
    "bool",
    "c_char",
    "c_int",
    "c_long",
    "c_longdouble",
    "c_longlong",
    "c_short",
    "c_uint",
    "c_ulong",
    "c_ulonglong",
    "c_ushort",
    "comptime_float",
    "comptime_int",
    "f16",
    "f32",
    "f64",
    "f80",
    "f128",
    "false",
    "isize",
    "noreturn",
    "null",
    "true",
    "type",
    "undefined",
    "usize",
    "void",
];

fn is_zig_reserved(s: &str) -> bool {
    // `u0`..`u65535` and `i0`..`i65535` are all primitive integer types.
    let arbitrary_int = s
        .strip_prefix(['u', 'i'])
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    arbitrary_int || ZIG_RESERVED.contains(&s)
}
