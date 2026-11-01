// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Semantic byte classification, UTF-8 sequence detection, and statistical analysis.

/// Semantic classification of a byte for visual hierarchy and typography.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteClass {
    /// Zero byte (`0x00`), rendered with low visual weight as negative space.
    Null,
    /// Printable ASCII graphic character (`0x21..=0x7E`).
    AsciiGraphic,
    /// ASCII whitespace (` `, `\t`, `\n`, `\r`, `\x0b`, `\x0c`).
    AsciiWhitespace,
    /// ASCII control character (`0x01..=0x1F` excluding whitespace, and `0x7F`).
    AsciiControl,
    /// Part of a valid multi-byte UTF-8 codepoint sequence (`0x80..=0xF4`).
    Utf8MultiByte,
    /// Non-ASCII, non-UTF-8 high byte (`0x80..=0xFE`).
    HighByte,
    /// All-ones sentinel byte (`0xFF`).
    Sentinel,
}

impl ByteClass {
    /// Returns a short human-readable label for this byte class.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Null => "Null (0x00)",
            Self::AsciiGraphic => "ASCII Text",
            Self::AsciiWhitespace => "Whitespace",
            Self::AsciiControl => "Control",
            Self::Utf8MultiByte => "UTF-8 Multi",
            Self::HighByte => "High Byte",
            Self::Sentinel => "Sentinel (0xFF)",
        }
    }

    /// Classifies a single byte without surrounding UTF-8 context.
    #[must_use]
    pub const fn classify_single(byte: u8) -> Self {
        match byte {
            0x00 => Self::Null,
            0x20 | 0x09..=0x0d => Self::AsciiWhitespace,
            0x01..=0x08 | 0x0e..=0x1f | 0x7f => Self::AsciiControl,
            0x21..=0x7e => Self::AsciiGraphic,
            0xff => Self::Sentinel,
            0x80..=0xfe => Self::HighByte,
        }
    }

    /// Returns the glyph representation for the right-hand character pane.
    ///
    /// When `rich_glyphs` is `true`, semantic Unicode symbols (`·`, `␣`, `↵`, `⇥`, etc.)
    /// are used to preserve grid alignment while revealing hidden structure.
    #[must_use]
    pub const fn glyph(self, byte: u8, rich_glyphs: bool) -> char {
        if !rich_glyphs {
            return match byte {
                0x20..=0x7e => byte as char,
                _ => '.',
            };
        }
        match self {
            Self::Null => '·',
            Self::AsciiGraphic => byte as char,
            Self::AsciiWhitespace => match byte {
                b' ' => '␣',
                b'\n' => '↵',
                b'\r' => '␍',
                b'\t' => '⇥',
                _ => '␣',
            },
            Self::AsciiControl => {
                if byte == 0x7f {
                    '␡'
                } else {
                    // Unicode Control Pictures block starts at U+2400.
                    match char::from_u32(0x2400 + byte as u32) {
                        Some(c) => c,
                        None => '•',
                    }
                }
            }
            Self::Utf8MultiByte => '◈',
            Self::HighByte => '▪',
            Self::Sentinel => '×',
        }
    }
}

/// Classifies an entire slice of bytes, detecting valid multi-byte UTF-8 sequences.
#[must_use]
pub fn classify_slice(data: &[u8]) -> Vec<ByteClass> {
    let mut classes = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b < 0x80 {
            classes.push(ByteClass::classify_single(b));
            i += 1;
            continue;
        }
        if b == 0xff {
            classes.push(ByteClass::Sentinel);
            i += 1;
            continue;
        }
        // Check for valid 2-, 3-, or 4-byte UTF-8 sequence starting at `i`.
        let seq_len = if (b & 0b1110_0000) == 0b1100_0000 {
            2
        } else if (b & 0b1111_0000) == 0b1110_0000 {
            3
        } else if (b & 0b1111_1000) == 0b1111_0000 {
            4
        } else {
            0
        };

        if seq_len >= 2
            && i + seq_len <= data.len()
            && std::str::from_utf8(&data[i..i + seq_len]).is_ok()
        {
            for _ in 0..seq_len {
                classes.push(ByteClass::Utf8MultiByte);
            }
            i += seq_len;
        } else {
            classes.push(ByteClass::HighByte);
            i += 1;
        }
    }
    classes
}

/// Statistical summary of a byte buffer for the information-design header bar.
#[derive(Debug, Clone)]
pub struct ByteStats {
    /// Total number of bytes analyzed.
    pub total_bytes: usize,
    /// Count of `0x00` bytes.
    pub null_count: usize,
    /// Count of printable ASCII + whitespace bytes.
    pub ascii_count: usize,
    /// Count of control bytes.
    pub control_count: usize,
    /// Count of valid UTF-8 multi-byte bytes.
    pub utf8_count: usize,
    /// Count of non-UTF-8 high bytes (`0x80..=0xFE`).
    pub high_count: usize,
    /// Count of `0xFF` sentinel bytes.
    pub sentinel_count: usize,
    /// Overall Shannon entropy in bits per byte (`0.0..=8.0`).
    pub entropy: f64,
    /// 16-bucket entropy sparkline values (`0..=7`).
    pub sparkline_buckets: Vec<u8>,
}

impl ByteStats {
    /// Computes byte class distributions, Shannon entropy, and a spatial entropy sparkline.
    #[must_use]
    pub fn analyze(data: &[u8], classes: &[ByteClass]) -> Self {
        let mut null_count = 0;
        let mut ascii_count = 0;
        let mut control_count = 0;
        let mut utf8_count = 0;
        let mut high_count = 0;
        let mut sentinel_count = 0;

        for class in classes {
            match class {
                ByteClass::Null => null_count += 1,
                ByteClass::AsciiGraphic | ByteClass::AsciiWhitespace => ascii_count += 1,
                ByteClass::AsciiControl => control_count += 1,
                ByteClass::Utf8MultiByte => utf8_count += 1,
                ByteClass::HighByte => high_count += 1,
                ByteClass::Sentinel => sentinel_count += 1,
            }
        }

        let entropy = compute_entropy(data);
        let bucket_count = 12.min(data.len().max(1));
        let chunk_size = (data.len() / bucket_count).max(1);
        let mut sparkline_buckets = Vec::with_capacity(bucket_count);
        if !data.is_empty() {
            for chunk in data.chunks(chunk_size).take(bucket_count) {
                let e = compute_entropy(chunk);
                let level = ((e / 8.0) * 7.0).round().clamp(0.0, 7.0) as u8;
                sparkline_buckets.push(level);
            }
        }

        Self {
            total_bytes: data.len(),
            null_count,
            ascii_count,
            control_count,
            utf8_count,
            high_count,
            sentinel_count,
            entropy,
            sparkline_buckets,
        }
    }

    /// Returns a unicode sparkline string representing local entropy across the file.
    #[must_use]
    pub fn sparkline_string(&self) -> String {
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        self.sparkline_buckets
            .iter()
            .map(|&idx| BARS[usize::from(idx.min(7))])
            .collect()
    }

    /// Returns a concise interpretation of the Shannon entropy value.
    #[must_use]
    pub fn entropy_profile(&self) -> &'static str {
        if self.total_bytes == 0 {
            "Empty"
        } else if self.entropy < 1.5 {
            "Sparse / Zero-heavy"
        } else if self.entropy < 4.8 {
            "Structured / Text"
        } else if self.entropy < 6.8 {
            "Mixed / Machine Code"
        } else {
            "Dense / Compressed"
        }
    }
}

/// Computes the Shannon entropy (`0.0..=8.0` bits/byte) of a byte slice.
#[must_use]
pub fn compute_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in data {
        counts[usize::from(b)] += 1;
    }
    let len = data.len() as f64;
    let mut entropy = 0.0;
    for &count in &counts {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}
