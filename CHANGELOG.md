# Changelog

All notable changes to `octet` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2] - 2026-11-02

### Fixed
- **Data loss (regression in 0.1.1)**: `octet --strings 8 FILE` treated `8` as the input and overwrote `FILE` as the output. A word made only of digits after `--strings` is again the minimum length. octet now also refuses to run when OUTFILE is the same file as INFILE (hard links included), so a dump can never overwrite its own input.
- **Hang (regression in 0.1.1)**: a negative `-s` on a device such as `/dev/zero` read forever. It now fails at once with "cannot seek".
- **Memory and latency**: `--classic` (with every option), `-C`, `-p` and `-i` (all languages except Rust) stream in 64 KiB steps instead of reading the whole input and rendering the whole output first. On a 200 MB file, `--classic | head -1` went from 17 s / 1.5 GB to 0.02 s / 2 MB, and `--classic /dev/zero` no longer exhausts memory. `--diff` of two 200 MiB files went from 2.8 GB to 400 MB.
- **Linux devices**: offsets on character devices are correct (`-s 0x100 /dev/zero` showed `00000000`), and `-s` works on `/proc` files, which report size 0.
- **`-S` search**: the Smart view no longer slows quadratically with many matches. 1 MiB with `-S A` went from 5.9 s to 0.08 s.
- **Card layout**: Smart cards stay aligned for every `-c`/`-g`/radix combination, 51 of which were misaligned. Wide (CJK, emoji) and zero-width characters are measured in terminal columns everywhere: banners, annotations, inspector, `--strings` and the `--diff` header.
- **Inspector**: u64 and i64 are always shown in full, LE and BE. Extreme values such as `i64::MIN` were cut off.
- **Header recognition**: no longer disabled by `-o`. It is still skipped when `-s` starts the view part-way into the file.
- **Docs and tooling**: the `--diff` help example had old and new swapped. Bash completion no longer splits file names containing spaces. The man page passes `mandoc -T lint` and honours `SOURCE_DATE_EPOCH`. `-p` and `-i` accept more than 256 columns, as xxd does.

### Changed
- Smart and Compact views read at most 16 MiB of an input that is not a regular file (a pipe or device) unless `-l` is given, with a note on stderr. Regular files are always read in full.
- Release archives now include static musl builds for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. They run on any Linux, while the gnu build needs glibc 2.34 or newer.

### Added
- Library: `stream_dump`, `stream_code` and `can_stream` for bounded-memory dumping from a reader to a writer, and `Diff` for planning a diff's shown rows in one pass.

## [0.1.1] - 2026-11-01

### Fixed
- **Reverse (`-r`)**: `xxd -g4` / `octet -g4` groups made only of `0`/`1` digits were decoded as `-b` bits, losing bytes. An `xxd` ASCII column containing `  |` was parsed as hex. Row layout is now detected from the row's shape.
- **xxd / hexdump parity**: offsets use a per-line 8-digit minimum, as `%08x` does, instead of the widest offset in the dump. Display offsets wrap past `u64::MAX`. `-e` with `-g` larger than `-c` no longer pads to the group size (a huge `-g` hung). `-c 0` means the mode's default width. `-n 0` prints nothing in `-C`, `--classic` and `-p`.
- **Terminal safety**: control characters in filenames are shown as `·` in the Smart banner, the `--diff` header and the `--strings` card, so a hostile name can no longer drive the terminal. `--color auto` no longer writes ANSI escapes into an `OUTFILE`. `FORCE_COLOR=0`, `false` or an empty value no longer forces colour.
- **Input**: a positive `-s` on a pipe or process substitution skips by reading instead of being silently ignored. Non-UTF-8 arguments no longer panic. `--help` and `--version` no longer panic on a closed pipe.
- **Inspector & search**: `--inspect` at an offset before the `-s` start is an out-of-range error instead of silently inspecting the start. A non-ASCII hex `-S` pattern is an error instead of a panic, and `+` signs inside hex patterns are rejected.
- **Header recognition**: the WASM section size is decoded as ULEB128. Big-endian pcapng follows its byte-order magic. Text that merely starts with `MZ` is no longer labelled a PE/MZ executable.
- **Codegen (`-i`)**: names that are reserved words in C, Go or Zig get a `_` suffix so the output compiles. C names starting with a digit use the `__` prefix, as `xxd -i` does.
- **Diff**: an elided run that reaches EOF reports the real byte count.
- **Entropy sparkline**: bins now cover the whole buffer instead of dropping up to 11 trailing bytes.
- **SVG**: U+FFFE and U+FFFF are replaced, so the output is always well-formed XML.

### Changed
- `--strings` takes a minimum length only as `--strings=<N>`. A following argument is always the input file.
- Conflicting modes (`-j` with `-i` or `-r`, `--svg` with `-j` or `-r`, and others) and `=value` on boolean flags are errors instead of being silently ignored.
- `--help` now lists `--completions`, `--man`, `-V`/`--version`, `-h`/`--help`, `--name` and `--svg-title`.
- The minimum supported Rust version is 1.88. 0.1.0 declared 1.87 but did not build on it.
- Licence texts ship as `LICENSE-MIT` and `LICENSE-APACHE`, matching the declared `MIT OR Apache-2.0`.

## [0.1.0] - 2026-11-01

### Added
- **Information-Design Terminal & Vector Renderer**:
  - Unified 106-column Smart view with Shannon entropy telemetry, 8-bin block entropy sparkline, proportional byte-class bar, dynamic column index ruler (`OFFSET │ 0001 ...`), inline row context rail, and structural header annotation footer.
  - 7 semantic byte classes (`Null`, `AsciiGraphic`, `AsciiWhitespace`, `AsciiControl`, `Utf8MultiByte`, `HighByte`, `Sentinel`) with calibrated dark (`#0D1117`) and light (`#FAF8F5`) truecolor palettes.
  - Native grid-locked SVG terminal window renderer (`--svg <PATH>`) with custom vector geometry for box-drawing borders, control badges (`LF`, `HT`, `NL`, `SH`, `DE`), sparklines, and markers.
- **31+ Automatic File Header Recognizers**:
  - Executables & Bytecode: **ELF** (32/64-bit, LE/BE), **Mach-O** (32/64-bit & Universal Fat binaries), **Java `.class`** (`0xCAFEBABE` disambiguated from Mach-O Fat), **PE / MZ**, **WebAssembly** (`\0asm`), **Android DEX**, **LLVM IR Bitcode**, and **Shebang (`#!`)** scripts.
  - Archives & Compression: **ZIP / JAR / Wheel**, **POSIX `ustar` TAR**, **7-Zip**, **GZIP**, **Zstandard**, **XZ**, **bzip2**, and **LZ4**.
  - Media, Captures, Databases & Fonts: **PNG** (`IHDR` dimensions, color type, CRC-32), **JPEG**, **GIF**, **WebP**, **MP4 / MOV / HEIC / AVIF** (`ftyp`), **RIFF WAVE / AVI**, **FLAC**, **Ogg**, **PCAP** (microsecond & nanosecond LE/BE) & **PCAPNG**, **SQLite 3**, **PDF**, **WOFF / WOFF2 / OpenType**, and **UTF-8 BOM**.
- **Full `xxd` & `hexdump -C` Compatibility + Modern Extensions**:
  - View styles: `Smart` (default), `Compact` (`-q`), `Classic` (`--classic` / `--xxd`), `Canonical` (`-C` / `--canonical` / `--hexdump`), and `Plain` (`-p`).
  - Radices & grouping: Hexadecimal, Binary (`-b`), Octal (`-O`), and Decimal (`-d`), with configurable `--cols` (`-c`), `--groupsize` (`-g`), and little-endian grouping (`-e`).
  - Side-by-side binary diffing (`--diff <FILE>`) with context row elision and similarity percentage telemetry.
  - Multi-type value inspector (`--inspect <[+-]OFFSET>`, repeatable) decoding `u8`/`i8`, `u16`/`i16`, `u32`/`i32`, `u64`/`i64`, `f32`, `f64`, `ULEB128` & `SLEB128` varints, and `UTF-8` codepoints.
  - Printable ASCII & graphic UTF-8 string literal extractor (`--strings[=<MIN_LEN>]`) with encoding (`ASCII`/`UTF-8`), null-termination (`\0`), and JSON (`-j`) reporting.
  - Multi-language byte array code generation (`-i` / `--lang <rust|c|go|python|ts|zig|csharp>`).
  - Round-trip hex-to-binary reversal (`-r` / `--reverse`) across `octet`, `xxd`, `hexdump -C`, `-b` binary, and `-r -p` plain streams.
  - Structured RFC 8259 JSON report export (`-j` / `--json`), shell completions (`--completions <bash|zsh|fish>`), and troff manual page generation (`--man`).

[Unreleased]: https://github.com/oddurs/octet/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/oddurs/octet/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/oddurs/octet/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/oddurs/octet/releases/tag/v0.1.0
