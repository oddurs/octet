// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Bounded-memory rendering for the `xxd`- and `hexdump`-compatible views and for
//! code generation.
//!
//! [`render_dump`](crate::render_dump) and [`generate_code`] take the whole input as a slice and
//! return the whole output as a string, which a 200 MB file turns into gigabytes and
//! `/dev/zero` into an out-of-memory kill. The functions here read and write in 64 KiB
//! chunks, carrying across each chunk boundary the little state that crosses it (the
//! `*` run, the offset, a partial line), and write the same bytes the buffered
//! functions return.

use std::io::{self, Read, Write};

use crate::classify::{ByteClass, ByteStats, classify_slice};
use crate::codegen::{self, CodeLanguage, generate_code};
use crate::format::{self, AnalyzedBuffer, FormatOptions, ViewStyle};
use crate::magic::{HeaderInspection, inspect_header};

/// Bytes read per step; rows are rendered and written once per step.
const CHUNK: usize = 64 * 1024;

/// How much of the input header recognition sees when colour needs it. Every recognised
/// format puts its header fields inside this window, except a PE file whose `e_lfanew`
/// points past it, which colour then does not mark as PE.
const HEADER_WINDOW: usize = 1024 * 1024;

const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";
const UPPER_HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Whether [`stream_dump`] renders `opts` in bounded memory.
///
/// The Smart and Compact views summarise the whole input before the first row, so they
/// cannot. Neither can a coloured Classic or Canonical dump with search highlights, whose
/// spans index the whole input.
#[must_use]
pub fn can_stream(opts: &FormatOptions) -> bool {
    match opts.style {
        ViewStyle::Plain => true,
        ViewStyle::Classic | ViewStyle::Canonical => !opts.color || opts.search_spans.is_empty(),
        ViewStyle::Smart | ViewStyle::Compact => false,
    }
}

/// Reads `reader` to its end and writes what [`render_dump`](crate::render_dump) would return
/// for those bytes.
///
/// When [`can_stream`] is true this holds one chunk of input and output at a time;
/// otherwise it reads the whole input first. Search spans in `opts` index the input from
/// its first byte read.
///
/// # Errors
///
/// Returns the first error from reading `reader` or writing `out`.
pub fn stream_dump(
    mut reader: impl Read,
    mut out: impl Write,
    opts: &FormatOptions,
) -> io::Result<()> {
    if !can_stream(opts) {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        return out.write_all(crate::render_dump(&data, opts).as_bytes());
    }
    if opts.style == ViewStyle::Plain {
        return stream_plain(reader, out, opts.cols, opts.upper_hex);
    }
    RowStream::new(opts).run(&mut reader, &mut out)
}

/// Reads `reader` to its end and writes what [`generate_code`] would return for those bytes.
///
/// Every language but Rust streams. Rust declares the array length before the first byte,
/// so it reads the whole input first.
///
/// # Errors
///
/// Returns the first error from reading `reader` or writing `out`.
pub fn stream_code(
    mut reader: impl Read,
    mut out: impl Write,
    var_name: Option<&str>,
    lang: CodeLanguage,
    cols: usize,
    upper: bool,
) -> io::Result<()> {
    if lang == CodeLanguage::Rust {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        return out.write_all(generate_code(&data, var_name, lang, cols, upper).as_bytes());
    }
    let cols = cols.max(1) as u64;
    let is_c = lang == CodeLanguage::C;
    let indent = codegen::grid_indent(lang).as_bytes();
    let prefix: &[u8] = match (upper, is_c) {
        (true, true) => b"0X",
        _ => b"0x",
    };
    let digits = if upper { UPPER_HEX } else { LOWER_HEX };
    out.write_all(codegen::declaration_open(var_name, lang, 0).as_bytes())?;

    let mut text = Vec::with_capacity(CHUNK * 7);
    let emit = |text: &mut Vec<u8>, byte: u8, index: u64, is_last: bool| {
        if index.is_multiple_of(cols) {
            text.extend_from_slice(indent);
        }
        text.extend_from_slice(prefix);
        text.push(digits[usize::from(byte >> 4)]);
        text.push(digits[usize::from(byte & 0xf)]);
        // C follows xxd and drops the comma after the last byte.
        if !(is_last && is_c) {
            text.push(b',');
        }
        text.push(if is_last || (index + 1).is_multiple_of(cols) {
            b'\n'
        } else {
            b' '
        });
    };

    // Whether a byte is the last decides its comma and line break, so each byte waits
    // until the next one (or the end of input) arrives.
    let mut pending: Option<u8> = None;
    let mut count = 0_u64;
    let mut chunk = vec![0_u8; CHUNK];
    loop {
        let n = read_some(&mut reader, &mut chunk)?;
        if n == 0 {
            break;
        }
        for &byte in &chunk[..n] {
            if let Some(prev) = pending.replace(byte) {
                emit(&mut text, prev, count, false);
                count += 1;
            }
        }
        out.write_all(&text)?;
        text.clear();
    }
    if let Some(last) = pending {
        emit(&mut text, last, count, true);
        count += 1;
    }
    out.write_all(&text)?;
    out.write_all(codegen::declaration_close(var_name, lang, count).as_bytes())
}

/// `xxd -p`: hex pairs, a newline every `cols` bytes (`0` for none) and one at the end.
fn stream_plain(
    mut reader: impl Read,
    mut out: impl Write,
    cols: usize,
    upper: bool,
) -> io::Result<()> {
    let digits = if upper { UPPER_HEX } else { LOWER_HEX };
    let wrap = cols as u64;
    let mut total = 0_u64;
    let mut chunk = vec![0_u8; CHUNK];
    let mut text = Vec::with_capacity(CHUNK * 3);
    loop {
        let n = read_some(&mut reader, &mut chunk)?;
        if n == 0 {
            break;
        }
        for &byte in &chunk[..n] {
            // The break goes before a byte rather than after one, so the last line
            // needs no lookahead to avoid an extra newline.
            if wrap != 0 && total != 0 && total.is_multiple_of(wrap) {
                text.push(b'\n');
            }
            text.push(digits[usize::from(byte >> 4)]);
            text.push(digits[usize::from(byte & 0xf)]);
            total += 1;
        }
        out.write_all(&text)?;
        text.clear();
    }
    if total > 0 || cols == 0 {
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// The Classic (`xxd`) and Canonical (`hexdump -C`) row renderer.
struct RowStream<'a> {
    opts: &'a FormatOptions,
    cols: usize,
    /// Byte classes and header fields of the bytes in `buf`, indexed from `buf[0]`; empty
    /// without colour, where neither changes a byte of output.
    analyzed: AnalyzedBuffer,
    /// The recognised header with field ranges counted from the first byte of input.
    header: Option<HeaderInspection>,
    buf: Vec<u8>,
    /// Absolute index of `buf[0]`.
    base: u64,
    /// Leading bytes of `buf` that finish a UTF-8 sequence begun in an earlier chunk.
    continuation: usize,
    text: String,
    /// The last row printed, while identical rows after it may still collapse into `*`.
    anchor: Option<Vec<u8>>,
    /// Rows identical to `anchor` seen since it was printed.
    dups: u64,
    /// The first two of those rows, rendered, for a run too short to collapse.
    held: String,
    spans_sorted: bool,
}

impl<'a> RowStream<'a> {
    fn new(opts: &'a FormatOptions) -> Self {
        Self {
            opts,
            cols: opts.cols.max(1),
            analyzed: AnalyzedBuffer {
                classes: Vec::new(),
                stats: ByteStats::analyze(&[], &[]),
                header: None,
                rows: Vec::new(),
            },
            header: None,
            buf: Vec::new(),
            base: 0,
            continuation: 0,
            text: String::new(),
            anchor: None,
            dups: 0,
            held: String::new(),
            spans_sorted: format::spans_are_sorted(&opts.search_spans),
        }
    }

    fn run(mut self, reader: &mut impl Read, out: &mut impl Write) -> io::Result<()> {
        let color = self.opts.color;
        // A step keeps back its last row and at least 3 bytes: whether a row is the last
        // one decides xxd's trailing `*`, and a UTF-8 sequence can straddle the cut.
        let holdback = self.cols.max(3);
        let target = (CHUNK / self.cols).max(1) * self.cols + holdback;
        let want_header = color && self.opts.annotate_headers;
        let first_target = if want_header {
            target.max(HEADER_WINDOW)
        } else {
            target
        };
        let mut eof = fill(reader, &mut self.buf, first_target)?;
        if want_header {
            self.header = inspect_header(&self.buf);
        }
        loop {
            let ready = if eof {
                self.buf.len()
            } else {
                (self.buf.len() - holdback) / self.cols * self.cols
            };
            if color {
                self.prepare_colour();
            }
            let mut start = 0;
            while start < ready {
                let end = (start + self.cols).min(ready);
                self.row(start, end);
                start = end;
            }
            if eof {
                self.finish();
                return out.write_all(self.text.as_bytes());
            }
            if color {
                self.continuation = self.buf[ready..]
                    .iter()
                    .zip(&self.analyzed.classes[ready..])
                    .take_while(|&(&b, &c)| c == ByteClass::Utf8MultiByte && b & 0xc0 == 0x80)
                    .count();
            }
            self.buf.drain(..ready);
            self.base += ready as u64;
            out.write_all(self.text.as_bytes())?;
            self.text.clear();
            eof = fill(reader, &mut self.buf, target)?;
        }
    }

    /// Classifies `buf` as `classify_slice` would see it inside the whole input and moves
    /// the header field ranges to index it. Classes of the last 3 bytes may change once
    /// more input arrives, which is why `run` holds them back.
    fn prepare_colour(&mut self) {
        let classes = &mut self.analyzed.classes;
        classes.clear();
        classes.resize(
            self.continuation.min(self.buf.len()),
            ByteClass::Utf8MultiByte,
        );
        classes.extend(classify_slice(&self.buf[classes.len()..]));

        let base = usize::try_from(self.base).unwrap_or(usize::MAX);
        self.analyzed.header = self.header.as_ref().and_then(|hdr| {
            let fields: Vec<_> = hdr
                .fields
                .iter()
                .filter(|f| f.range.end > base)
                .map(|f| {
                    let mut field = f.clone();
                    field.range = f.range.start.saturating_sub(base)..f.range.end - base;
                    field
                })
                .collect();
            (!fields.is_empty()).then(|| HeaderInspection {
                fields,
                ..hdr.clone()
            })
        });
        if self.analyzed.header.is_none() {
            self.header = None;
        }
    }

    /// Feeds the row `buf[start..end]` through the same `*` rules as `analyze_buffer`.
    fn row(&mut self, start: usize, end: usize) {
        let bytes = &self.buf[start..end];
        if self.anchor.as_deref() == Some(bytes) {
            self.dups += 1;
            if self.dups <= 2 {
                let mut held = std::mem::take(&mut self.held);
                self.render(&mut held, start, end);
                self.held = held;
            }
            return;
        }
        self.close_run();
        let mut text = std::mem::take(&mut self.text);
        self.render(&mut text, start, end);
        self.text = text;

        let bytes = &self.buf[start..end];
        let collapsible = self.opts.autoskip
            && bytes.len() == self.cols
            && (self.opts.style != ViewStyle::Classic || bytes.iter().all(|&b| b == 0));
        if collapsible {
            let anchor = self.anchor.get_or_insert_with(Vec::new);
            anchor.clear();
            anchor.extend_from_slice(bytes);
        }
    }

    /// Ends a run of identical rows at a row that differs: `*` if it is long enough to
    /// collapse (hexdump: one repeat, xxd: two), else the repeats as rows.
    fn close_run(&mut self) {
        if self.anchor.take().is_some() && self.dups > 0 {
            if self.dups >= self.min_dups() {
                self.text.push_str("*\n");
            } else {
                self.text.push_str(&self.held);
            }
        }
        self.dups = 0;
        self.held.clear();
    }

    fn min_dups(&self) -> u64 {
        if self.opts.style == ViewStyle::Canonical {
            1
        } else {
            2
        }
    }

    fn finish(&mut self) {
        let total = self.base + self.buf.len() as u64;
        if self.anchor.is_some() && self.dups > 0 && self.opts.style == ViewStyle::Classic {
            // xxd prints the last row of a run that reaches the end of input, so the
            // run collapses only with two repeats besides that row.
            if self.dups > 2 {
                self.text.push_str("*\n");
                let end = self.buf.len();
                let mut text = std::mem::take(&mut self.text);
                self.render(&mut text, end - self.cols, end);
                self.text = text;
            } else {
                self.text.push_str(&self.held);
            }
            self.anchor = None;
        } else {
            self.close_run();
        }
        if self.opts.style == ViewStyle::Canonical && (total > 0 || self.opts.display_offset > 0) {
            let end = self.opts.display_offset.wrapping_add(total);
            let theme = &self.opts.theme;
            self.text
                .push_str(&theme.format_offset(end, 8, false, self.opts.color));
            self.text.push('\n');
        }
    }

    fn render(&self, text: &mut String, start: usize, end: usize) {
        let offset = self
            .opts
            .display_offset
            .wrapping_add(self.base + start as u64);
        format::render_data_row(
            text,
            offset,
            start,
            &self.buf[start..end],
            &self.analyzed,
            self.opts,
            format::RowLayout {
                off_w: 8,
                spans_sorted: self.spans_sorted,
            },
        );
    }
}

/// Reads until `buf` holds `target` bytes or the input ends, and returns whether it ended.
fn fill(reader: &mut impl Read, buf: &mut Vec<u8>, target: usize) -> io::Result<bool> {
    let want = target.saturating_sub(buf.len());
    let got = reader.by_ref().take(want as u64).read_to_end(buf)?;
    Ok(got < want)
}

/// One `read`, retried on `Interrupted`; 0 means the end of input.
fn read_some(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            res => return res,
        }
    }
}
