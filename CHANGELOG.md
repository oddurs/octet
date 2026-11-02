# Changelog

All notable changes to `octet` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
  - Multi-type value inspector (`--inspect <OFFSET>`) decoding `u8`/`i8`, `u16`/`i16`, `u32`/`i32`, `u64`/`i64`, `f32`, `f64`, `ULEB128` varints, and `UTF-8` codepoints.
  - Multi-language byte array code generation (`-i` / `--lang <rust|c|go|python|ts>`).
  - Round-trip hex-to-binary reversal (`-r` / `--reverse`) across `octet`, `xxd`, `hexdump -C`, `-b` binary, and `-r -p` plain streams.
  - Structured RFC 8259 JSON report export (`-j` / `--json`), shell completions (`--completions <bash|zsh|fish>`), and troff manual page generation (`--man`).
