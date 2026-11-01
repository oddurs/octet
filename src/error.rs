// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Error types and result aliases for the `octet` library.

use std::fmt;
use std::io;

/// Alias for a `Result` with the library's [`Error`] type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by `octet` parsing, formatting, and I/O operations.
#[derive(Debug)]
pub enum Error {
    /// Underlying standard I/O error.
    Io(io::Error),
    /// Invalid command-line or configuration argument.
    InvalidArgument(String),
    /// Malformed hexadecimal input encountered during reverse (`-r`) conversion.
    ParseHex {
        /// One-based line number where the error occurred.
        line: usize,
        /// Human-readable description of the parse error.
        message: String,
    },
    /// Offset or slice range is invalid for the given input.
    InvalidOffset(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::ParseHex { line, message } => {
                write!(f, "hex parse error on line {line}: {message}")
            }
            Self::InvalidOffset(msg) => write!(f, "invalid offset: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
