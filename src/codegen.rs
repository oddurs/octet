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
}

impl CodeLanguage {
    /// Parses a language identifier string (`c`, `rust`, `rs`, `go`, `py`, `python`, `ts`, `js`).
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "c" | "h" | "cpp" => Ok(Self::C),
            "rust" | "rs" => Ok(Self::Rust),
            "go" | "golang" => Ok(Self::Go),
            "py" | "python" => Ok(Self::Python),
            "ts" | "typescript" | "js" | "javascript" => Ok(Self::TypeScript),
            other => Err(Error::InvalidArgument(format!(
                "unsupported codegen language '{other}' (expected: rust, c, go, python, ts)"
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
    let base_ident = sanitize_identifier(var_name.unwrap_or("octet_data"));
    let upper_ident = base_ident.to_ascii_uppercase();
    let mut out = String::new();

    match lang {
        CodeLanguage::C => {
            let _ = writeln!(out, "unsigned char {base_ident}[] = {{");
            write_hex_grid(&mut out, data, cols, upper, "  ", true);
            let _ = writeln!(out, "}};");
            let _ = writeln!(out, "unsigned int {base_ident}_len = {};", data.len());
        }
        CodeLanguage::Rust => {
            let _ = writeln!(out, "pub const {upper_ident}: &[u8; {}] = &[", data.len());
            write_hex_grid(&mut out, data, cols, upper, "    ", false);
            let _ = writeln!(out, "];");
        }
        CodeLanguage::Go => {
            let _ = writeln!(out, "var {base_ident} = []byte{{");
            write_hex_grid(&mut out, data, cols, upper, "\t", false);
            let _ = writeln!(out, "}}");
        }
        CodeLanguage::Python => {
            let _ = writeln!(out, "{upper_ident} = bytes([");
            write_hex_grid(&mut out, data, cols, upper, "    ", false);
            let _ = writeln!(out, "])");
        }
        CodeLanguage::TypeScript => {
            let _ = writeln!(out, "export const {upper_ident} = new Uint8Array([");
            write_hex_grid(&mut out, data, cols, upper, "  ", false);
            let _ = writeln!(out, "]);");
        }
    }
    out
}

fn write_hex_grid(
    out: &mut String,
    data: &[u8],
    cols: usize,
    upper: bool,
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
                let _ = write!(out, "0x{byte:02X}");
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

fn sanitize_identifier(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    for (i, ch) in raw.chars().enumerate() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if i == 0 && ch.is_ascii_digit() {
                s.push('_');
            }
            s.push(ch);
        } else {
            s.push('_');
        }
    }
    let trimmed = s.trim_matches('_');
    if trimmed.is_empty() {
        String::from("octet_data")
    } else {
        trimmed.to_owned()
    }
}
