// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Command-line interface for `octet`.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, IsTerminal as _, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use octet::{
    CodeLanguage, ColorMode, Error, FormatOptions, Radix, Result, Theme, ValueInspection,
    ViewStyle, compute_diff, extract_strings, find_pattern_spans, generate_code, parse_byte_count,
    parse_search_pattern, render_ansi_to_svg, render_diff_ansi, render_diff_json, render_dump,
    render_json_report, render_strings_ansi, render_strings_json, reverse_dump,
};

fn main() -> ExitCode {
    // `args_os`, because file names on Unix need not be UTF-8.
    match run_cli(std::env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let err_tty = io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
            let theme = Theme::dark();
            if err_tty {
                let badge = theme.paint_bold("◈ octet error:", theme.diff_del, true);
                let hint = theme.paint(
                    "Run 'octet --help' for usage and examples.",
                    theme.fg_muted,
                    true,
                );
                let _ = writeln!(io::stderr(), "{badge} {err}\n  {hint}");
            } else {
                let _ = writeln!(io::stderr(), "octet error: {err}");
            }
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug)]
struct CliConfig {
    help: bool,
    version: bool,
    man: bool,
    completions: Option<String>,
    reverse: bool,
    json: bool,
    include_codegen: bool,
    codegen_lang: CodeLanguage,
    codegen_name: Option<String>,
    radix: Radix,
    style: ViewStyle,
    cols: Option<usize>,
    group_size: Option<usize>,
    little_endian: bool,
    upper_hex: bool,
    autoskip: Option<bool>,
    annotate_headers: bool,
    seek: Option<String>,
    length: Option<u64>,
    display_offset: Option<u64>,
    search_patterns: Vec<String>,
    inspect_offsets: Vec<String>,
    strings_min_len: Option<usize>,
    diff_path: Option<PathBuf>,
    svg_out: Option<PathBuf>,
    svg_title: Option<String>,
    theme: Theme,
    color_mode: ColorMode,
    input_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
}

impl Default for CliConfig {
    fn default() -> Self {
        let theme = match std::env::var("OCTET_THEME")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "light" => Theme::light(),
            _ => Theme::dark(),
        };
        let color_mode = match std::env::var("OCTET_COLOR")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "always" | "truecolor" | "yes" => ColorMode::TrueColor,
            "never" | "no" | "none" => ColorMode::Never,
            _ => ColorMode::Auto,
        };
        Self {
            help: false,
            version: false,
            man: false,
            completions: None,
            reverse: false,
            json: false,
            include_codegen: false,
            codegen_lang: CodeLanguage::C,
            codegen_name: None,
            radix: Radix::Hex,
            style: ViewStyle::Smart,
            cols: None,
            group_size: None,
            little_endian: false,
            upper_hex: false,
            autoskip: None,
            annotate_headers: true,
            seek: None,
            length: None,
            display_offset: None,
            search_patterns: Vec::new(),
            inspect_offsets: Vec::new(),
            strings_min_len: None,
            diff_path: None,
            svg_out: None,
            svg_title: None,
            theme,
            color_mode,
            input_path: None,
            output_path: None,
        }
    }
}

impl CliConfig {
    /// The row width, where `-c 0` means the mode's default, as in xxd.
    fn cols_or(&self, default: usize) -> usize {
        self.cols.filter(|&c| c != 0).unwrap_or(default)
    }

    /// Rejects flag combinations where one flag would silently be ignored.
    fn check_conflicts(&self) -> Result<()> {
        let conflict =
            |a: &str, b: &str| Error::InvalidArgument(format!("{a} cannot be combined with {b}"));
        let modes: Vec<&str> = [
            ("-r/--reverse", self.reverse),
            ("-i/--include", self.include_codegen),
            ("--diff", self.diff_path.is_some()),
            ("--strings", self.strings_min_len.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect();
        if let [first, second, ..] = modes[..] {
            return Err(conflict(first, second));
        }
        let mode = modes.first().copied();
        // xxd caps its row layouts at 256 octets but not the plain stream or
        // the C include array, whose rows have no layout to break.
        if let Some(c) = self.cols
            && c > 256
            && self.style != ViewStyle::Plain
            && !self.include_codegen
        {
            return Err(Error::InvalidArgument(format!(
                "column count {c} exceeds maximum supported columns (256)"
            )));
        }
        // Reverse and codegen output are not reports, so there is no JSON form.
        if self.json && (self.reverse || self.include_codegen) {
            return Err(conflict("-j/--json", mode.unwrap_or_default()));
        }
        if self.svg_out.is_some() {
            if self.json {
                return Err(conflict("--svg", "-j/--json"));
            }
            if self.reverse {
                return Err(conflict("--svg", "-r/--reverse"));
            }
        }
        if let Some(m) = mode {
            if !self.inspect_offsets.is_empty() {
                return Err(conflict("--inspect", m));
            }
            if !self.search_patterns.is_empty() {
                return Err(conflict("-S/--search", m));
            }
        }
        Ok(())
    }
}

/// The file name of `path` for titles and labels, lossily decoded.
fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn run_cli(args: Vec<OsString>) -> Result<()> {
    let mut cfg = parse_args(&args)?;
    let stdout_is_tty = io::stdout().is_terminal();

    // If invoked with no arguments in an interactive terminal, show help instead of hanging on stdin.
    if args.is_empty() && io::stdin().is_terminal() {
        cfg.help = true;
    }

    if cfg.help {
        let color = cfg.color_mode.is_enabled(stdout_is_tty) || cfg.svg_out.is_some();
        let help_text = render_help(&cfg.theme, color);
        if let Some(ref svg_path) = cfg.svg_out {
            let title = cfg.svg_title.as_deref().unwrap_or("octet --help");
            let svg_doc = render_ansi_to_svg(&help_text, title, &cfg.theme);
            fs::write(svg_path, svg_doc)?;
        }
        return write_output(None, help_text.as_bytes());
    }

    if cfg.version {
        let version = format!("octet {}\n", env!("CARGO_PKG_VERSION"));
        return write_output(None, version.as_bytes());
    }

    if cfg.man {
        write_output(cfg.output_path.as_deref(), render_manpage().as_bytes())?;
        return Ok(());
    }

    if let Some(ref shell) = cfg.completions {
        let script = render_completions(shell)?;
        write_output(cfg.output_path.as_deref(), script.as_bytes())?;
        return Ok(());
    }

    cfg.check_conflicts()?;
    if let Some(out) = cfg.output_path.as_deref() {
        for input in [cfg.input_path.as_deref(), cfg.diff_path.as_deref()]
            .into_iter()
            .flatten()
        {
            if overwrites(input, out) {
                return Err(Error::InvalidArgument(format!(
                    "OUTFILE '{}' is the same file as INFILE '{}'; refusing to overwrite it",
                    out.display(),
                    input.display()
                )));
            }
        }
    }

    // An OUTFILE is never a terminal, so `auto` leaves it plain even when stdout is one.
    let output_is_tty = cfg.output_path.is_none() && stdout_is_tty;
    let color = cfg.color_mode.is_enabled(output_is_tty) || cfg.svg_out.is_some();

    if cfg.reverse {
        let raw_input = read_input_window(cfg.input_path.as_deref(), None, None)?.0;
        let text = String::from_utf8_lossy(&raw_input);
        let bytes = reverse_dump(&text, cfg.style == ViewStyle::Plain)?;
        write_output(cfg.output_path.as_deref(), &bytes)?;
        return Ok(());
    }

    let (sliced_vec, start_off) =
        read_input_window(cfg.input_path.as_deref(), cfg.seek.as_deref(), cfg.length)?;
    let sliced = sliced_vec.as_slice();
    let base_disp_offset = start_off.wrapping_add(cfg.display_offset.unwrap_or(0));
    let input_label = cfg.input_path.as_deref().map(display_name);

    // Code generation mode (`-i` / `--include` or `--lang`)
    if cfg.include_codegen {
        let default_cols = if cfg.radix == Radix::Binary { 6 } else { 12 };
        let cols = cfg.cols_or(default_cols);
        let input_name = cfg.input_path.as_ref().map(|p| p.to_string_lossy());
        let var_name = cfg.codegen_name.as_deref().or(input_name.as_deref());
        let code = generate_code(sliced, var_name, cfg.codegen_lang, cols, cfg.upper_hex);
        if let Some(ref svg_path) = cfg.svg_out {
            let styled = style_code_for_svg(&code, &cfg.theme, color);
            let title = cfg.svg_title.as_deref().unwrap_or("octet -i");
            let svg_doc = render_ansi_to_svg(&styled, title, &cfg.theme);
            fs::write(svg_path, svg_doc)?;
        }
        write_output(cfg.output_path.as_deref(), code.as_bytes())?;
        return Ok(());
    }

    // Binary diff mode (`--diff <other>`)
    if let Some(ref right_path) = cfg.diff_path {
        let (right_sliced, _) =
            read_input_window(Some(right_path), cfg.seek.as_deref(), cfg.length)?;
        let cols = cfg.cols_or(8);
        let autoskip = cfg.autoskip.unwrap_or(true);
        let report = compute_diff(sliced, &right_sliced, cols, base_disp_offset, autoskip);
        let left_label = cfg.input_path.as_ref().map_or_else(
            || String::from("stdin"),
            |p| p.to_string_lossy().into_owned(),
        );
        let right_label = right_path.to_string_lossy();
        if cfg.json {
            let json_out = render_diff_json(&report, &left_label, &right_label);
            write_output(cfg.output_path.as_deref(), json_out.as_bytes())?;
            return Ok(());
        }
        let rendered =
            render_diff_ansi(&report, &left_label, &right_label, cols, &cfg.theme, color);
        if let Some(ref svg_path) = cfg.svg_out {
            let default_title = format!("octet --diff {left_label} {right_label}");
            let title = cfg.svg_title.as_deref().unwrap_or(&default_title);
            let svg_doc = render_ansi_to_svg(&rendered, title, &cfg.theme);
            fs::write(svg_path, svg_doc)?;
        }
        write_output(cfg.output_path.as_deref(), rendered.as_bytes())?;
        return Ok(());
    }

    // String literal extraction mode (`--strings`)
    if let Some(min_chars) = cfg.strings_min_len {
        let label = input_label.as_deref();
        let extracted = extract_strings(sliced, min_chars, base_disp_offset);
        if cfg.json {
            let json_out = render_strings_json(&extracted, sliced.len(), min_chars, label);
            write_output(cfg.output_path.as_deref(), json_out.as_bytes())?;
            return Ok(());
        }
        let rendered = render_strings_ansi(
            &extracted,
            sliced.len(),
            min_chars,
            label,
            &cfg.theme,
            color,
        );
        if let Some(ref svg_path) = cfg.svg_out {
            let default_title = match label {
                Some(l) => format!("octet --strings {l}"),
                None => String::from("octet --strings"),
            };
            let title = cfg.svg_title.as_deref().unwrap_or(&default_title);
            let svg_doc = render_ansi_to_svg(&rendered, title, &cfg.theme);
            fs::write(svg_path, svg_doc)?;
        }
        write_output(cfg.output_path.as_deref(), rendered.as_bytes())?;
        return Ok(());
    }

    // Standard / Smart / Classic / Plain hex or radix dump
    let cols = if cfg.style == ViewStyle::Plain {
        // Plain mode keeps 0: like `xxd -p -c 0`, it prints one unbroken line.
        cfg.cols.unwrap_or(30)
    } else {
        cfg.cols_or(match cfg.radix {
            Radix::Binary => 6,
            Radix::Hex | Radix::Octal | Radix::Decimal => 16,
        })
    };
    let default_group = match cfg.radix {
        Radix::Binary | Radix::Octal | Radix::Decimal => 1,
        Radix::Hex => {
            if cfg.little_endian {
                4
            } else if cfg.style == ViewStyle::Canonical {
                1
            } else {
                2
            }
        }
    };
    let group_size = cfg.group_size.unwrap_or(default_group);
    let autoskip = cfg.autoskip.unwrap_or(matches!(
        cfg.style,
        ViewStyle::Smart | ViewStyle::Compact | ViewStyle::Canonical
    ));

    let mut search_spans = Vec::new();
    for pat_str in &cfg.search_patterns {
        let needle = parse_search_pattern(pat_str)?;
        search_spans.extend(find_pattern_spans(sliced, &needle));
    }
    search_spans.sort_by_key(|r| r.start);

    let opts = FormatOptions {
        label: input_label,
        style: cfg.style,
        radix: cfg.radix,
        cols,
        group_size,
        little_endian: cfg.little_endian,
        upper_hex: cfg.upper_hex,
        autoskip,
        display_offset: base_disp_offset,
        // A header is only recognisable at byte 0 of the input; `-o` moves
        // the displayed addresses, not the data.
        annotate_headers: cfg.annotate_headers && start_off == 0,
        search_spans,
        theme: cfg.theme.clone(),
        color,
    };

    let mut inspections = Vec::with_capacity(cfg.inspect_offsets.len());
    for insp_str in &cfg.inspect_offsets {
        let trimmed = insp_str.trim();
        let rel_idx = if let Some(neg) = trimmed.strip_prefix('-') {
            let back = usize::try_from(parse_byte_count(neg)?).unwrap_or(usize::MAX);
            if back == 0 || back > sliced.len() {
                return Err(Error::InvalidOffset(format!(
                    "inspect offset -{neg} is outside buffer range (0..0x{:x})",
                    sliced.len()
                )));
            }
            sliced.len() - back
        } else {
            // An offset before the window is as out of range as one past
            // its end; usize::MAX makes `ValueInspection::at` reject it.
            parse_byte_count(trimmed)?
                .checked_sub(base_disp_offset)
                .and_then(|rel| usize::try_from(rel).ok())
                .unwrap_or(usize::MAX)
        };
        if let Some(insp) = ValueInspection::at(sliced, rel_idx, base_disp_offset) {
            inspections.push(insp);
        } else {
            return Err(Error::InvalidOffset(format!(
                "inspect offset '{insp_str}' is outside buffer range (0..0x{:x})",
                sliced.len()
            )));
        }
    }

    if cfg.json {
        let json_out = render_json_report(sliced, &opts, inspections.first());
        write_output(cfg.output_path.as_deref(), json_out.as_bytes())?;
        return Ok(());
    }

    // Asked for zero bytes, hexdump and xxd print nothing, not even the final
    // offset line that a seek past EOF still gets. render_dump sees an empty
    // slice either way, so only the requested length can tell them apart.
    let compatible_style = matches!(
        cfg.style,
        ViewStyle::Canonical | ViewStyle::Classic | ViewStyle::Plain
    );
    let mut rendered = if cfg.length == Some(0) && compatible_style {
        String::new()
    } else {
        render_dump(sliced, &opts)
    };
    for insp in &inspections {
        rendered.push_str(&insp.render_ansi(&cfg.theme, color));
    }

    if let Some(ref svg_path) = cfg.svg_out {
        let default_title = match cfg.input_path.as_deref() {
            Some(p) => format!("octet {}", p.to_string_lossy()),
            None => String::from("octet"),
        };
        let title = cfg.svg_title.as_deref().unwrap_or(&default_title);
        let svg_doc = render_ansi_to_svg(&rendered, title, &cfg.theme);
        fs::write(svg_path, svg_doc)?;
    }

    write_output(cfg.output_path.as_deref(), rendered.as_bytes())?;
    Ok(())
}

/// Whether writing OUTFILE `out` would destroy the regular file `input`. A
/// device such as `/dev/stdout` may be both, but writing it destroys nothing,
/// and an OUTFILE that does not exist yet cannot be the input.
fn overwrites(input: &Path, out: &Path) -> bool {
    let Ok(out_meta) = fs::metadata(out) else {
        return false;
    };
    if !out_meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    if let Ok(in_meta) = fs::metadata(input) {
        use std::os::unix::fs::MetadataExt as _;
        // Catches hard links, which have different canonical paths.
        if (in_meta.dev(), in_meta.ino()) == (out_meta.dev(), out_meta.ino()) {
            return true;
        }
    }
    matches!(
        (fs::canonicalize(input), fs::canonicalize(out)),
        (Ok(a), Ok(b)) if a == b
    )
}

/// Takes the value of option `flag`: its attached part when there is one,
/// otherwise the next argument.
fn flag_value(
    args: &[OsString],
    i: &mut usize,
    inline: Option<&str>,
    flag: &str,
) -> Result<OsString> {
    if let Some(v) = inline {
        return Ok(v.into());
    }
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| Error::InvalidArgument(format!("option '{flag}' requires a value")))
}

/// Like [`flag_value`], for options whose value is text rather than a path.
fn flag_text(args: &[OsString], i: &mut usize, inline: Option<&str>, flag: &str) -> Result<String> {
    flag_value(args, i, inline, flag)?
        .into_string()
        .map_err(|v| {
            Error::InvalidArgument(format!(
                "value {v:?} for option '{flag}' is not valid UTF-8"
            ))
        })
}

fn parse_cols(raw: &str) -> Result<usize> {
    Ok(usize::try_from(parse_byte_count(raw)?).unwrap_or(usize::MAX))
}

/// Applies a long flag that takes no value; returns false if `key` is not one.
fn apply_long_switch(cfg: &mut CliConfig, key: &str) -> bool {
    match key {
        "help" => cfg.help = true,
        "version" => cfg.version = true,
        "man" => cfg.man = true,
        "autoskip" => cfg.autoskip = Some(true),
        "no-autoskip" => cfg.autoskip = Some(false),
        "hex" | "hexadecimal" => cfg.radix = Radix::Hex,
        "bits" | "binary" => cfg.radix = Radix::Binary,
        "octal" => cfg.radix = Radix::Octal,
        "decimal" => cfg.radix = Radix::Decimal,
        "little-endian" | "endian" => cfg.little_endian = true,
        "include" => cfg.include_codegen = true,
        "plain" | "ps" | "postscript" => cfg.style = ViewStyle::Plain,
        "compact" | "quiet" => cfg.style = ViewStyle::Compact,
        "canonical" | "hexdump" => cfg.style = ViewStyle::Canonical,
        "classic" | "xxd" => {
            cfg.style = ViewStyle::Classic;
            if cfg.autoskip.is_none() {
                cfg.autoskip = Some(false);
            }
        }
        "reverse" => cfg.reverse = true,
        "json" => cfg.json = true,
        "upper" | "uppercase" => cfg.upper_hex = true,
        "no-annotate" => cfg.annotate_headers = false,
        _ => return false,
    }
    true
}

fn parse_args(args: &[OsString]) -> Result<CliConfig> {
    let mut cfg = CliConfig::default();
    let mut positional: Vec<OsString> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let Some(arg) = args[i].to_str() else {
            // A non-UTF-8 argument can only be a path; options are ASCII.
            if args[i].as_encoded_bytes().starts_with(b"-") {
                return Err(Error::InvalidArgument(format!(
                    "option {:?} is not valid UTF-8",
                    args[i]
                )));
            }
            positional.push(args[i].clone());
            i += 1;
            continue;
        };
        if arg == "--" {
            positional.extend_from_slice(&args[i + 1..]);
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (key, inline_val) = match long.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (long, None),
            };
            let flag = format!("--{key}");

            match key {
                "completions" => {
                    cfg.completions = Some(flag_text(args, &mut i, inline_val, &flag)?);
                }
                "cols" | "columns" => {
                    cfg.cols = Some(parse_cols(&flag_text(args, &mut i, inline_val, &flag)?)?);
                }
                "groupsize" | "group-size" | "group" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    cfg.group_size =
                        Some(usize::try_from(parse_byte_count(&v)?).unwrap_or(usize::MAX));
                }
                "lang" | "language" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    cfg.codegen_lang = CodeLanguage::parse(&v)?;
                    cfg.include_codegen = true;
                }
                "name" => {
                    cfg.codegen_name = Some(flag_text(args, &mut i, inline_val, &flag)?);
                }
                "length" | "len" | "bytes" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    cfg.length = Some(parse_byte_count(&v)?);
                }
                "seek" => {
                    cfg.seek = Some(flag_text(args, &mut i, inline_val, &flag)?);
                }
                "display-offset" | "offset" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    cfg.display_offset = Some(parse_byte_count(&v)?);
                }
                "search" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    cfg.search_patterns.push(v);
                }
                "inspect" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    cfg.inspect_offsets.push(v);
                }
                "strings" => {
                    // The minimum length is optional. A separate word is taken
                    // only when it is all digits, so `--strings 8 fw.bin`
                    // reads fw.bin while `--strings 1.bin` keeps its file; a
                    // file named `8` is still reachable as `./8`.
                    let separate = args
                        .get(i + 1)
                        .and_then(|a| a.to_str())
                        .filter(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()));
                    let raw = match (inline_val, separate) {
                        (Some(v), _) => Some(v),
                        (None, Some(v)) => {
                            i += 1;
                            Some(v)
                        }
                        (None, None) => None,
                    };
                    let min_len = match raw {
                        Some(v) => usize::try_from(parse_byte_count(v)?).unwrap_or(4).max(1),
                        None => 4,
                    };
                    cfg.strings_min_len = Some(min_len);
                }
                "diff" => {
                    cfg.diff_path = Some(flag_value(args, &mut i, inline_val, &flag)?.into());
                }
                "svg" => {
                    cfg.svg_out = Some(flag_value(args, &mut i, inline_val, &flag)?.into());
                }
                "svg-title" => {
                    cfg.svg_title = Some(flag_text(args, &mut i, inline_val, &flag)?);
                }
                "theme" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    match v.to_ascii_lowercase().as_str() {
                        "dark" => cfg.theme = Theme::dark(),
                        "light" => cfg.theme = Theme::light(),
                        other => {
                            return Err(Error::InvalidArgument(format!(
                                "unknown theme '{other}' (expected 'dark' or 'light')"
                            )));
                        }
                    }
                }
                "color" | "colour" => {
                    let v = flag_text(args, &mut i, inline_val, &flag)?;
                    match v.to_ascii_lowercase().as_str() {
                        "auto" => cfg.color_mode = ColorMode::Auto,
                        "always" | "truecolor" | "yes" => cfg.color_mode = ColorMode::TrueColor,
                        "never" | "no" | "none" => cfg.color_mode = ColorMode::Never,
                        other => {
                            return Err(Error::InvalidArgument(format!(
                                "unknown color mode '{other}' (expected auto, always, never)"
                            )));
                        }
                    }
                }
                switch => {
                    if !apply_long_switch(&mut cfg, switch) {
                        let suffix = match suggest_long_flag(switch) {
                            Some(s) => format!(" (did you mean '--{s}'?)"),
                            None => String::from(" (run 'octet --help' for usage)"),
                        };
                        return Err(Error::InvalidArgument(format!(
                            "unrecognized option '{flag}'{suffix}"
                        )));
                    }
                    // `--json=no` reads as a request to turn JSON off; doing
                    // the opposite of what was written is worse than failing.
                    if let Some(v) = inline_val {
                        return Err(Error::InvalidArgument(format!(
                            "option '{flag}' does not take a value (got '{v}')"
                        )));
                    }
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            // Short flags (supporting combined flags like `-rp` or attached values like `-g4`)
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut c_idx = 0;
            while c_idx < chars.len() {
                let ch = chars[c_idx];
                let rest: String = chars[c_idx + 1..].iter().collect();
                let inline = (!rest.is_empty()).then_some(rest.as_str());
                let flag = format!("-{ch}");

                match ch {
                    'h' => cfg.help = true,
                    'V' => cfg.version = true,
                    'v' => {
                        if args.len() == 1 && chars.len() == 1 {
                            cfg.version = true;
                        } else {
                            cfg.autoskip = Some(false);
                        }
                    }
                    'a' => cfg.autoskip = Some(true),
                    'b' => cfg.radix = Radix::Binary,
                    'd' => cfg.radix = Radix::Decimal,
                    'O' => cfg.radix = Radix::Octal,
                    'e' => cfg.little_endian = true,
                    'i' => cfg.include_codegen = true,
                    'p' => cfg.style = ViewStyle::Plain,
                    'q' => cfg.style = ViewStyle::Compact,
                    'C' => cfg.style = ViewStyle::Canonical,
                    'r' => cfg.reverse = true,
                    'j' => cfg.json = true,
                    'u' => cfg.upper_hex = true,
                    'c' => {
                        cfg.cols = Some(parse_cols(&flag_text(args, &mut i, inline, &flag)?)?);
                        break;
                    }
                    'g' => {
                        let v = flag_text(args, &mut i, inline, &flag)?;
                        cfg.group_size =
                            Some(usize::try_from(parse_byte_count(&v)?).unwrap_or(usize::MAX));
                        break;
                    }
                    'l' | 'n' => {
                        let v = flag_text(args, &mut i, inline, &flag)?;
                        cfg.length = Some(parse_byte_count(&v)?);
                        break;
                    }
                    'o' => {
                        let v = flag_text(args, &mut i, inline, &flag)?;
                        cfg.display_offset = Some(parse_byte_count(&v)?);
                        break;
                    }
                    's' => {
                        cfg.seek = Some(flag_text(args, &mut i, inline, &flag)?);
                        break;
                    }
                    'S' => {
                        let v = flag_text(args, &mut i, inline, &flag)?;
                        cfg.search_patterns.push(v);
                        break;
                    }
                    other => {
                        return Err(Error::InvalidArgument(format!(
                            "unrecognized flag '-{other}' (run 'octet --help' for usage)"
                        )));
                    }
                }
                c_idx += 1;
            }
        } else {
            positional.push(args[i].clone());
        }
        i += 1;
    }

    if positional.len() > 2 {
        return Err(Error::InvalidArgument(
            "expected at most 2 positional arguments: [infile [outfile]]".to_owned(),
        ));
    }
    let mut positional = positional.into_iter().map(|p| (p != "-").then(|| p.into()));
    cfg.input_path = positional.next().flatten();
    cfg.output_path = positional.next().flatten();

    Ok(cfg)
}

/// Reads the `-s`/`-l` window of `path` (stdin when `None`) and returns it
/// with the absolute offset it starts at.
fn read_input_window(
    path: Option<&Path>,
    seek_spec: Option<&str>,
    length: Option<u64>,
) -> Result<(Vec<u8>, u64)> {
    let Some(path) = path else {
        return read_stream_window(io::stdin().lock(), seek_spec, length);
    };
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    let seek_spec = seek_spec.map(str::trim);

    // Only a regular file has a length worth trusting; a pipe or FIFO
    // (`/dev/stdin`, `<(…)`) reports what happens to be buffered, often 0.
    if !meta.is_file() {
        if let Some(spec) = seek_spec.filter(|s| !s.starts_with('-')) {
            let pos = parse_byte_count(spec)?;
            // Block devices can seek; when a pipe refuses, fall through and
            // skip by reading, as xxd does.
            if let Ok(at) = file.seek(SeekFrom::Start(pos)) {
                return Ok((read_limited(file, length)?, at));
            }
        }
        return read_stream_window(file, seek_spec, length);
    }

    let start = match seek_spec {
        None => 0,
        Some(spec) => {
            let target = match spec.strip_prefix('-') {
                Some(back) => meta.len().saturating_sub(parse_byte_count(back)?),
                None => parse_byte_count(spec)?.min(meta.len()),
            };
            file.seek(SeekFrom::Start(target))?
        }
    };
    Ok((read_limited(file, length)?, start))
}

/// Reads the `-s`/`-l` window of a stream that cannot seek: a positive seek
/// is skipped by reading, and a seek from the end needs the whole stream.
fn read_stream_window(
    mut reader: impl io::Read,
    seek_spec: Option<&str>,
    length: Option<u64>,
) -> Result<(Vec<u8>, u64)> {
    let skipped = match seek_spec.map(str::trim) {
        None => 0,
        Some(spec) if spec.starts_with('-') => {
            let mut full = Vec::new();
            reader.read_to_end(&mut full)?;
            let (sliced, off) = octet::slice_buffer(&full, Some(spec), length)?;
            return Ok((sliced.to_vec(), off));
        }
        Some(spec) => {
            let pos = parse_byte_count(spec)?;
            io::copy(&mut reader.by_ref().take(pos), &mut io::sink())?
        }
    };
    Ok((read_limited(reader, length)?, skipped))
}

fn read_limited(reader: impl io::Read, length: Option<u64>) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader
        .take(length.unwrap_or(u64::MAX))
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Writes to `path`, or to stdout when `None`. A closed stdout (`octet f |
/// head`) ends output quietly, as in other Unix filters, rather than failing.
fn write_output(path: Option<&Path>, bytes: &[u8]) -> Result<()> {
    if let Some(p) = path {
        fs::write(p, bytes)?;
    } else {
        let stdout = io::stdout();
        let mut writer = io::BufWriter::with_capacity(64 * 1024, stdout.lock());
        if let Err(err) = writer.write_all(bytes).and_then(|()| writer.flush()) {
            if err.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(Error::Io(err));
        }
    }
    Ok(())
}

fn style_code_for_svg(code: &str, theme: &Theme, color: bool) -> String {
    if !color {
        return code.to_owned();
    }
    let mut out = String::new();
    for line in code.lines() {
        if line.contains("0x") {
            let _ = writeln!(out, "{}", theme.paint(line, theme.byte_ascii, true));
        } else {
            let _ = writeln!(out, "{}", theme.paint_bold(line, theme.offset_active, true));
        }
    }
    out
}

fn suggest_long_flag(unknown: &str) -> Option<&'static str> {
    const KNOWN_FLAGS: &[&str] = &[
        "help",
        "version",
        "man",
        "completions",
        "autoskip",
        "no-autoskip",
        "hex",
        "bits",
        "binary",
        "octal",
        "decimal",
        "cols",
        "columns",
        "groupsize",
        "group-size",
        "little-endian",
        "include",
        "lang",
        "name",
        "length",
        "seek",
        "display-offset",
        "offset",
        "plain",
        "compact",
        "canonical",
        "hexdump",
        "classic",
        "xxd",
        "reverse",
        "json",
        "upper",
        "no-annotate",
        "search",
        "inspect",
        "strings",
        "diff",
        "svg",
        "svg-title",
        "theme",
        "color",
    ];
    let mut best: Option<(&'static str, usize)> = None;
    for &candidate in KNOWN_FLAGS {
        let dist = edit_distance(unknown, candidate);
        if dist <= 3 && best.is_none_or(|(_, b_dist)| dist < b_dist) {
            best = Some((candidate, dist));
        }
    }
    best.map(|(s, _)| s)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0usize; b_chars.len() + 1];
    for (i, &ca) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

fn render_help(t: &Theme, c: bool) -> String {
    let mut out = String::new();
    let inner_w = 104usize;
    let top = t.paint(&format!("╭{}╮", "─".repeat(inner_w)), t.border, c);
    let mid = t.paint(&format!("├{}┤", "─".repeat(inner_w)), t.border, c);
    let bot = t.paint(&format!("╰{}╯", "─".repeat(inner_w)), t.border, c);

    let _ = writeln!(out, "{top}");
    let title_plain = format!(
        " ◈ OCTET v{}  │  Information-Design Hex Viewer, Binary Inspector & xxd / hexdump Replacement",
        env!("CARGO_PKG_VERSION")
    );
    let title_pad = " ".repeat(inner_w.saturating_sub(title_plain.chars().count()));
    let _ = writeln!(
        out,
        "{} {}  {}  {}{title_pad}{}",
        t.paint("│", t.border, c),
        t.paint_bold(
            &format!("◈ OCTET v{}", env!("CARGO_PKG_VERSION")),
            t.offset_active,
            c
        ),
        t.paint("│", t.border, c),
        t.paint_bold(
            "Information-Design Hex Viewer, Binary Inspector & xxd / hexdump Replacement",
            t.fg_primary,
            c
        ),
        t.paint("│", t.border, c),
    );
    let _ = writeln!(out, "{mid}");
    let usage_plain = " USAGE:  octet [OPTIONS] [INFILE [OUTFILE]]      (reads stdin when INFILE is omitted or '-')";
    let usage_pad = " ".repeat(inner_w.saturating_sub(usage_plain.chars().count()));
    let _ = writeln!(
        out,
        "{} {}  {} {}{usage_pad}{}",
        t.paint("│", t.border, c),
        t.paint_bold("USAGE:", t.byte_control, c),
        t.paint_bold("octet", t.offset_active, c),
        t.paint(
            "[OPTIONS] [INFILE [OUTFILE]]      (reads stdin when INFILE is omitted or '-')",
            t.fg_secondary,
            c
        ),
        t.paint("│", t.border, c),
    );
    let _ = writeln!(out, "{bot}");

    let section =
        |title: &str| -> String { format!("\n{}", t.paint_bold(title, t.byte_control, c)) };
    let flag = |f: &str, desc: &str| -> String {
        let f_padded = format!("{f:<34}");
        format!(
            "  {} {}",
            t.paint_bold(&f_padded, t.byte_ascii, c),
            t.paint(desc, t.fg_primary, c)
        )
    };

    let _ = writeln!(
        out,
        "{}",
        section("◈ VIEW PROFILES & RADIX OPTIONS (xxd & hexdump -C compatible)")
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-c, --cols <N>",
            "Format <N> octets per row (default: 16, binary: 6, plain: 30)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-g, --groupsize <N>",
            "Group <N> octets per space-separated block (default: 2, -e: 4)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-e, --little-endian",
            "Little-endian byte grouping (100% compatible with xxd -e)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-b, --bits",
            "Binary digit dump (01000001) instead of hexadecimal"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-d, --decimal / -O, --octal",
            "Render bytes as 3-digit unsigned decimal or octal"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag("-u, --upper", "Use uppercase hexadecimal digits (00..FF)")
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-C, --canonical",
            "Canonical BSD/GNU 'hexdump -C' layout (00000000  48 65 ... |He|)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--classic / -q, --compact",
            "Strict 'xxd' layout (--classic) or cardless compact table (-q)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-p, --plain",
            "Continuous plain hexadecimal stream (xxd -p)"
        )
    );

    let _ = writeln!(out, "{}", section("◈ SEEKING, SLICING & ELISION"));
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-s, --seek <[+-]OFFSET>",
            "Start at <OFFSET> (hex 0x100, 4KiB, or -0x40 from EOF)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-l, -n, --length <LEN>",
            "Read at most <LEN> bytes (supports KiB, MiB, 0x...)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-o, --display-offset <OFF>",
            "Add <OFF> to displayed row addresses"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-a, --autoskip / -v, --no-autoskip",
            "Collapse or expand consecutive identical/zero rows"
        )
    );

    let _ = writeln!(
        out,
        "{}",
        section("◈ INSPECTION, BINARY DIFFING, CODEGEN & REVERSE")
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--diff <FILE>",
            "Side-by-side binary delta comparison against <FILE>"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-S, --search <PATTERN>",
            "Highlight ASCII text or hex sequence (e.g. -S 'ELF' or -S 0xfeedfacf)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--inspect <[+-]OFFSET>",
            "Decode u8..i64/f32/f64/ULEB128/SLEB128/UTF-8 at <OFFSET>"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--strings [N], --strings=N",
            "Extract printable ASCII & UTF-8 strings of >= N chars (default: 4)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-i, --include [--lang <L>]",
            "Emit source array in rust, c (default), go, python, ts, zig, or csharp"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--name <IDENT>",
            "Identifier for the -i array (default: derived from INFILE)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-r, --reverse",
            "Reverse hex dump (octet, xxd, hexdump -C, or -r -p) to binary"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-j, --json",
            "Emit structured RFC 8259 JSON report (telemetry, headers, diff, inspect)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--no-annotate",
            "Disable automatic magic header structural annotations"
        )
    );

    let _ = writeln!(out, "{}", section("◈ DESIGN SYSTEM & VECTOR EXPORT"));
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--theme <dark|light>",
            "Select calibrated color palette (or set OCTET_THEME)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--color <auto|always|never>",
            "Control ANSI TrueColor output (respects NO_COLOR / OCTET_COLOR)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--svg <PATH>",
            "Render vector macOS terminal window screenshot to <PATH>"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--svg-title <TITLE>",
            "Window title for the --svg screenshot"
        )
    );

    let _ = writeln!(out, "{}", section("◈ SHELL INTEGRATION & META"));
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--completions <bash|zsh|fish>",
            "Print a shell completion script"
        )
    );
    let _ = writeln!(out, "{}", flag("--man", "Print the troff manual page"));
    let _ = writeln!(out, "{}", flag("-V, --version", "Print the version"));
    let _ = writeln!(out, "{}", flag("-h, --help", "Print this help"));

    let _ = writeln!(out, "\n{top}");
    let ex_hdr = " ◈ QUICK EXAMPLES";
    let ex_hdr_pad = " ".repeat(inner_w.saturating_sub(ex_hdr.chars().count()));
    let _ = writeln!(
        out,
        "{}{}{ex_hdr_pad}{}",
        t.paint("│", t.border, c),
        t.paint_bold(ex_hdr, t.offset_active, c),
        t.paint("│", t.border, c),
    );
    let examples = [
        (
            "octet firmware.bin",
            "Smart view with entropy telemetry & header annotations",
        ),
        (
            "octet v1.bin --diff v2.bin",
            "Side-by-side binary delta comparison with context elision",
        ),
        (
            "octet --inspect 0x10 -S 0xfeedfacf a.out",
            "Multi-type data inspector at 0x10 + highlight Mach-O magic",
        ),
        (
            "octet -i --lang rust -l 64 payload.bin",
            "Generate Rust 'pub const PAYLOAD_BIN: &[u8; 64]' array",
        ),
    ];
    for (cmd, note) in examples {
        let row_plain_len = 3 + 40 + 2 + note.chars().count();
        let pad = " ".repeat(inner_w.saturating_sub(row_plain_len));
        let cmd_padded = format!("{cmd:<40}");
        let _ = writeln!(
            out,
            "{}   {}  {}{pad}{}",
            t.paint("│", t.border, c),
            t.paint_bold(&cmd_padded, t.byte_ascii, c),
            t.paint(note, t.fg_secondary, c),
            t.paint("│", t.border, c),
        );
    }
    let _ = writeln!(out, "{bot}");
    out
}

fn render_completions(shell: &str) -> Result<String> {
    match shell.to_ascii_lowercase().as_str() {
        "bash" => Ok(String::from(
            r#"# Fills COMPREPLY one line per candidate, so a file name with spaces stays
# one word; a while-read loop also runs on the bash 3.2 that macOS ships.
_octet_reply() {
    local word
    COMPREPLY=()
    while IFS='' read -r word; do
        COMPREPLY+=("${word}")
    done < <(compgen "$@")
}
_octet() {
    local cur prev opts
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    opts="-h --help -V --version -c --cols -g --groupsize -e --little-endian -b --bits -d --decimal -O --octal -u --upper -C --canonical --classic -q --compact -p --plain -s --seek -l -n --length -o --display-offset -a --autoskip -v --no-autoskip --diff -S --search --inspect --strings -i --include --lang --name -r --reverse -j --json --no-annotate --theme --color --svg --svg-title --completions --man"
    case "${prev}" in
        --theme) _octet_reply -W "dark light" -- "${cur}"; return 0 ;;
        --color) _octet_reply -W "auto always never" -- "${cur}"; return 0 ;;
        --lang) _octet_reply -W "rust c go python ts zig csharp" -- "${cur}"; return 0 ;;
        --completions) _octet_reply -W "bash zsh fish" -- "${cur}"; return 0 ;;
    esac
    if [[ ${cur} == -* ]]; then
        _octet_reply -W "${opts}" -- "${cur}"
        return 0
    fi
    _octet_reply -f -- "${cur}"
}
# -o filenames quotes the inserted name and marks directories with a slash.
complete -o filenames -F _octet octet
"#,
        )),
        "zsh" => Ok(String::from(
            r#"#compdef octet
_octet() {
    _arguments -s \
        '(-h --help)'{-h,--help}'[Show help information]' \
        '(-V --version)'{-V,--version}'[Show version]' \
        '(-c --cols)'{-c,--cols}'[Octets per row]:cols:' \
        '(-g --groupsize)'{-g,--groupsize}'[Octets per group]:groupsize:' \
        '(-e --little-endian)'{-e,--little-endian}'[Little-endian byte grouping]' \
        '(-b --bits)'{-b,--bits}'[Binary digit dump]' \
        '(-d --decimal)'{-d,--decimal}'[Unsigned decimal byte dump]' \
        '(-O --octal)'{-O,--octal}'[Octal byte dump]' \
        '(-u --upper)'{-u,--upper}'[Uppercase hex digits]' \
        '(-C --canonical)'{-C,--canonical}'[Canonical hexdump -C layout]' \
        '--classic[Strict xxd layout]' \
        '(-q --compact)'{-q,--compact}'[Compact cardless table]' \
        '(-p --plain)'{-p,--plain}'[Plain continuous hex stream]' \
        '(-s --seek)'{-s,--seek}'[Seek offset]:offset:' \
        '(-l -n --length)'{-l,-n,--length}'[Read length limit]:length:' \
        '(-o --display-offset)'{-o,--display-offset}'[Add offset to displayed addresses]:offset:' \
        '(-a --autoskip)'{-a,--autoskip}'[Collapse identical rows]' \
        '(-v --no-autoskip)'{-v,--no-autoskip}'[Expand all rows]' \
        '--diff[Side-by-side binary diff]:file:_files' \
        '(-S --search)'{-S,--search}'[Highlight pattern]:pattern:' \
        '--inspect[Inspect multi-type values at offset]:offset:' \
        '--strings=[Extract printable ASCII and UTF-8 strings]::min_len:' \
        '(-i --include)'{-i,--include}'[Generate source array]' \
        '--lang[Target language for codegen]:lang:(rust c go python ts zig csharp)' \
        '--name[Variable identifier for codegen]:name:' \
        '(-r --reverse)'{-r,--reverse}'[Reverse hex dump to binary]' \
        '(-j --json)'{-j,--json}'[Emit structured JSON report]' \
        '--no-annotate[Disable magic header annotations]' \
        '--theme[Select color palette]:theme:(dark light)' \
        '--color[Control ANSI color]:mode:(auto always never)' \
        '--svg[Render vector terminal SVG]:path:_files' \
        '--svg-title[Window title for the SVG]:title:' \
        '--completions[Generate shell completions]:shell:(bash zsh fish)' \
        '--man[Render troff manual page]' \
        '*:file:_files'
}
_octet "$@"
"#,
        )),
        "fish" => Ok(String::from(
            r#"complete -c octet -s h -l help -d "Show help"
complete -c octet -s V -l version -d "Show version"
complete -c octet -s c -l cols -r -d "Octets per row"
complete -c octet -s g -l groupsize -r -d "Octets per group"
complete -c octet -s e -l little-endian -d "Little-endian byte grouping"
complete -c octet -s b -l bits -d "Binary digit dump"
complete -c octet -s d -l decimal -d "Unsigned decimal byte dump"
complete -c octet -s O -l octal -d "Octal byte dump"
complete -c octet -s u -l upper -d "Uppercase hex digits"
complete -c octet -s C -l canonical -d "Canonical hexdump -C layout"
complete -c octet -l classic -d "Strict xxd layout"
complete -c octet -s q -l compact -d "Compact cardless table"
complete -c octet -s p -l plain -d "Plain continuous hex stream"
complete -c octet -s s -l seek -r -d "Start at offset"
complete -c octet -s l -s n -l length -r -d "Read at most N bytes"
complete -c octet -s o -l display-offset -r -d "Add offset to displayed addresses"
complete -c octet -s a -l autoskip -d "Collapse identical rows"
complete -c octet -s v -l no-autoskip -d "Expand all rows"
complete -c octet -l diff -r -F -d "Side-by-side binary diff"
complete -c octet -s S -l search -r -d "Highlight pattern"
complete -c octet -l inspect -r -d "Inspect values at offset"
complete -c octet -l strings -d "Extract strings of at least N chars (default 4)"
complete -c octet -s i -l include -d "Generate source code array"
complete -c octet -l lang -r -a "rust c go python ts zig csharp" -d "Codegen language"
complete -c octet -l name -r -d "Variable identifier for codegen"
complete -c octet -s r -l reverse -d "Reverse hex dump to binary"
complete -c octet -s j -l json -d "Emit structured JSON report"
complete -c octet -l no-annotate -d "Disable magic header annotations"
complete -c octet -l theme -r -a "dark light" -d "Color palette"
complete -c octet -l color -r -a "auto always never" -d "Color mode"
complete -c octet -l svg -r -F -d "Render vector SVG screenshot"
complete -c octet -l svg-title -r -d "Window title for the SVG"
complete -c octet -l completions -r -a "bash zsh fish" -d "Generate shell completions"
complete -c octet -l man -d "Render troff manual page"
"#,
        )),
        other => Err(Error::InvalidArgument(format!(
            "unsupported shell '{other}' for --completions (expected bash, zsh, or fish)"
        ))),
    }
}

/// The page is dated the day it is generated, as help2man does, so the date
/// needs no upkeep; `SOURCE_DATE_EPOCH` pins it for reproducible packages.
fn render_manpage() -> String {
    let epoch_secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        });
    let (year, month, day) = civil_date(epoch_secs / 86_400);
    format!(
        r#".TH OCTET 1 "{year:04}-{month:02}-{day:02}" "octet {version}" "User Commands"
.SH NAME
octet \- information-design-first hex viewer, binary inspector,
and xxd / hexdump replacement
.SH SYNOPSIS
.B octet
[\fIOPTIONS\fR] [\fIINFILE\fR [\fIOUTFILE\fR]]
.SH DESCRIPTION
.B octet
is a modern, zero-dependency hexadecimal and multi-radix binary viewer
engineered around visual hierarchy, semantic 7-class byte typography,
automatic structural header recognition (31+ binary formats),
side-by-side binary diffing, multi-type value inspection,
multi-language code generation, and round-trip hex-to-binary reversal.
.SH OPTIONS
.TP
\fB\-c\fR, \fB\-\-cols\fR \fIN\fR
Format \fIN\fR octets per row (default: 16, binary: 6, plain: 30).
At most 256, except with \fB\-p\fR and \fB\-i\fR, as in \fBxxd\fR.
.TP
\fB\-g\fR, \fB\-\-groupsize\fR \fIN\fR
Group \fIN\fR octets per space-separated block (default: 2, \-e: 4).
.TP
\fB\-e\fR, \fB\-\-little\-endian\fR
Little-endian byte grouping (compatible with \fBxxd \-e\fR).
.TP
\fB\-b\fR, \fB\-\-bits\fR
Binary digit dump (01000001) instead of hexadecimal.
.TP
\fB\-d\fR, \fB\-\-decimal\fR / \fB\-O\fR, \fB\-\-octal\fR
Render bytes as 3-digit unsigned decimal or octal.
.TP
\fB\-u\fR, \fB\-\-upper\fR
Use uppercase hexadecimal digits (00..FF).
.TP
\fB\-C\fR, \fB\-\-canonical\fR
Canonical BSD/GNU \fBhexdump \-C\fR layout.
.TP
\fB\-\-classic\fR / \fB\-q\fR, \fB\-\-compact\fR
Strict \fBxxd\fR layout (\fB\-\-classic\fR)
or cardless compact table (\fB\-q\fR).
.TP
\fB\-p\fR, \fB\-\-plain\fR
Continuous plain hexadecimal stream (\fBxxd \-p\fR).
.TP
\fB\-s\fR, \fB\-\-seek\fR [\fI+-\fR]\fIOFFSET\fR
Start at \fIOFFSET\fR (supports hex 0x100, 4KiB, or \-0x40 from EOF).
.TP
\fB\-l\fR, \fB\-n\fR, \fB\-\-length\fR \fILEN\fR
Read at most \fILEN\fR bytes.
.TP
\fB\-o\fR, \fB\-\-display\-offset\fR \fIOFF\fR
Add \fIOFF\fR to displayed row addresses.
.TP
\fB\-a\fR, \fB\-\-autoskip\fR / \fB\-v\fR, \fB\-\-no\-autoskip\fR
Collapse or expand consecutive identical/zero rows.
.TP
\fB\-\-diff\fR \fIFILE\fR
Side-by-side binary delta comparison against \fIFILE\fR.
.TP
\fB\-S\fR, \fB\-\-search\fR \fIPATTERN\fR
Highlight ASCII text or hex sequence
(e.g. \fB\-S 'ELF'\fR or \fB\-S 0xfeedfacf\fR).
.TP
\fB\-\-inspect\fR [\fI+-\fR]\fIOFFSET\fR
Decode u8/i8/u16/u32/u64/i64/f32/f64/ULEB128/SLEB128/UTF-8
at \fIOFFSET\fR (repeatable).
.TP
\fB\-\-strings\fR [\fIN\fR], \fB\-\-strings=\fR\fIN\fR
Extract printable ASCII and graphic UTF-8 string literals
of at least \fIN\fR characters (default: 4).
A separate \fIN\fR is taken only when it is all digits;
name a file called \fB8\fR as \fB./8\fR.
.TP
\fB\-i\fR, \fB\-\-include\fR [\fB\-\-lang\fR \fIL\fR]
Emit source array in rust, c (default), go, python, ts, zig, or csharp.
.TP
\fB\-\-name\fR \fIIDENT\fR
Identifier for the generated array (default: derived from \fIINFILE\fR).
.TP
\fB\-r\fR, \fB\-\-reverse\fR
Reverse hex dump (octet, xxd, hexdump \-C, or \-r \-p) back to binary.
.TP
\fB\-j\fR, \fB\-\-json\fR
Emit structured RFC 8259 JSON report.
.TP
\fB\-\-no\-annotate\fR
Disable automatic magic header structural annotations.
.TP
\fB\-\-theme\fR \fIdark|light\fR
Select calibrated color palette (or set \fBOCTET_THEME\fR).
.TP
\fB\-\-color\fR \fIauto|always|never\fR
Control ANSI TrueColor output (respects \fBNO_COLOR\fR / \fBOCTET_COLOR\fR).
.TP
\fB\-\-svg\fR \fIPATH\fR
Render vector macOS terminal window screenshot to \fIPATH\fR.
.TP
\fB\-\-svg\-title\fR \fITITLE\fR
Window title for the \fB\-\-svg\fR screenshot.
.TP
\fB\-\-completions\fR \fIbash|zsh|fish\fR
Generate shell completion script.
.TP
\fB\-\-man\fR
Emit this manual page in troff format.
.TP
\fB\-V\fR, \fB\-\-version\fR
Print the version.
.TP
\fB\-h\fR, \fB\-\-help\fR
Print usage help.
"#,
        version = env!("CARGO_PKG_VERSION")
    )
}

/// The proleptic Gregorian (year, month, day) of `days` since 1970-01-01,
/// by Howard Hinnant's `civil_from_days`, shifted so March starts the year.
fn civil_date(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + u64::from(month <= 2), month, day)
}
