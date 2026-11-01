// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Command-line interface for `octet`.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, IsTerminal as _, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;
use std::process::ExitCode;

use octet::{
    CodeLanguage, ColorMode, Error, FormatOptions, Radix, Result, Theme, ValueInspection,
    ViewStyle, compute_diff, find_pattern_spans, generate_code, parse_byte_count,
    parse_search_pattern, render_ansi_to_svg, render_diff_ansi, render_dump, reverse_dump,
};

fn main() -> ExitCode {
    match run_cli(std::env::args().skip(1).collect()) {
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
    reverse: bool,
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
    inspect_offset: Option<String>,
    diff_path: Option<String>,
    svg_out: Option<String>,
    svg_title: Option<String>,
    theme: Theme,
    color_mode: ColorMode,
    input_path: Option<String>,
    output_path: Option<String>,
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
            reverse: false,
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
            inspect_offset: None,
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

fn run_cli(args: Vec<String>) -> Result<()> {
    let mut cfg = parse_args(&args)?;
    let is_tty = io::stdout().is_terminal();
    let color = cfg.color_mode.is_enabled(is_tty) || cfg.svg_out.is_some();

    // If invoked with no arguments in an interactive terminal, show help instead of hanging on stdin.
    if args.is_empty() && io::stdin().is_terminal() {
        cfg.help = true;
    }

    if cfg.help {
        let help_text = render_help(&cfg.theme, color);
        if let Some(ref svg_path) = cfg.svg_out {
            let title = cfg.svg_title.as_deref().unwrap_or("octet --help");
            let svg_doc = render_ansi_to_svg(&help_text, title, &cfg.theme);
            fs::write(svg_path, svg_doc)?;
        }
        print!("{help_text}");
        return Ok(());
    }

    if cfg.version {
        println!("octet {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

    // Code generation mode (`-i` / `--include` or `--lang`)
    if cfg.include_codegen {
        let default_cols = if cfg.radix == Radix::Binary { 6 } else { 12 };
        let cols = cfg.cols.unwrap_or(default_cols);
        let var_name = cfg.codegen_name.as_deref().or(cfg.input_path.as_deref());
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
        let cols = cfg.cols.unwrap_or(8);
        let autoskip = cfg.autoskip.unwrap_or(true);
        let report = compute_diff(sliced, &right_sliced, cols, base_disp_offset, autoskip);
        let left_label = cfg.input_path.as_deref().unwrap_or("stdin");
        let rendered = render_diff_ansi(&report, left_label, right_path, cols, &cfg.theme, color);
        if let Some(ref svg_path) = cfg.svg_out {
            let default_title = format!("octet --diff {left_label} {right_path}");
            let title = cfg.svg_title.as_deref().unwrap_or(&default_title);
            let svg_doc = render_ansi_to_svg(&rendered, title, &cfg.theme);
            fs::write(svg_path, svg_doc)?;
        }
        write_output(cfg.output_path.as_deref(), rendered.as_bytes())?;
        return Ok(());
    }

    // Standard / Smart / Classic / Plain hex or radix dump
    let default_cols = if cfg.style == ViewStyle::Plain {
        30
    } else {
        match cfg.radix {
            Radix::Binary => 6,
            Radix::Hex | Radix::Octal | Radix::Decimal => 16,
        }
    };
    let cols = cfg.cols.unwrap_or(default_cols);
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

    let label = cfg.input_path.as_ref().map(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(p)
            .to_owned()
    });

    let opts = FormatOptions {
        label,
        style: cfg.style,
        radix: cfg.radix,
        cols,
        group_size,
        little_endian: cfg.little_endian,
        upper_hex: cfg.upper_hex,
        autoskip,
        display_offset: base_disp_offset,
        annotate_headers: cfg.annotate_headers,
        search_spans,
        theme: cfg.theme.clone(),
        color,
    };

    let mut rendered = render_dump(sliced, &opts);

    if let Some(ref insp_str) = cfg.inspect_offset {
        let target_off = parse_byte_count(insp_str)?;
        let rel_idx = target_off.saturating_sub(base_disp_offset) as usize;
        if let Some(insp) = ValueInspection::at(sliced, rel_idx, base_disp_offset) {
            rendered.push_str(&insp.render_ansi(&cfg.theme, color));
        } else {
            return Err(Error::InvalidOffset(format!(
                "inspect offset 0x{target_off:x} is outside buffer range (0..0x{:x})",
                sliced.len()
            )));
        }
    }

    if let Some(ref svg_path) = cfg.svg_out {
        let default_title = match cfg.input_path.as_deref() {
            Some(p) => format!("octet {p}"),
            None => String::from("octet"),
        };
        let title = cfg.svg_title.as_deref().unwrap_or(&default_title);
        let svg_doc = render_ansi_to_svg(&rendered, title, &cfg.theme);
        fs::write(svg_path, svg_doc)?;
    }

    write_output(cfg.output_path.as_deref(), rendered.as_bytes())?;
    Ok(())
}

fn parse_args(args: &[String]) -> Result<CliConfig> {
    let mut cfg = CliConfig::default();
    let mut positional = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            positional.extend_from_slice(&args[i + 1..]);
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (key, inline_val) = match long.split_once('=') {
                Some((k, v)) => (k, Some(v.to_owned())),
                None => (long, None),
            };
            let mut next_val = |flag_name: &str| -> Result<String> {
                if let Some(v) = inline_val.clone() {
                    Ok(v)
                } else if i + 1 < args.len() {
                    i += 1;
                    Ok(args[i].clone())
                } else {
                    Err(Error::InvalidArgument(format!(
                        "option '--{flag_name}' requires a value"
                    )))
                }
            };

            match key {
                "help" => cfg.help = true,
                "version" => cfg.version = true,
                "autoskip" => cfg.autoskip = Some(true),
                "no-autoskip" => cfg.autoskip = Some(false),
                "hex" | "hexadecimal" => cfg.radix = Radix::Hex,
                "bits" | "binary" => cfg.radix = Radix::Binary,
                "octal" => cfg.radix = Radix::Octal,
                "decimal" => cfg.radix = Radix::Decimal,
                "cols" | "columns" => {
                    let v = next_val("cols")?;
                    cfg.cols = Some(parse_byte_count(&v)? as usize);
                }
                "groupsize" | "group-size" | "group" => {
                    let v = next_val("groupsize")?;
                    cfg.group_size = Some(parse_byte_count(&v)? as usize);
                }
                "little-endian" | "endian" => cfg.little_endian = true,
                "include" => cfg.include_codegen = true,
                "lang" | "language" => {
                    let v = next_val("lang")?;
                    cfg.codegen_lang = CodeLanguage::parse(&v)?;
                    cfg.include_codegen = true;
                }
                "name" => {
                    cfg.codegen_name = Some(next_val("name")?);
                }
                "length" | "len" | "bytes" => {
                    let v = next_val("length")?;
                    cfg.length = Some(parse_byte_count(&v)?);
                }
                "seek" => {
                    cfg.seek = Some(next_val("seek")?);
                }
                "display-offset" | "offset" => {
                    let v = next_val("display-offset")?;
                    cfg.display_offset = Some(parse_byte_count(&v)?);
                }
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
                "upper" | "uppercase" => cfg.upper_hex = true,
                "no-annotate" => cfg.annotate_headers = false,
                "search" => {
                    cfg.search_patterns.push(next_val("search")?);
                }
                "inspect" => {
                    cfg.inspect_offset = Some(next_val("inspect")?);
                }
                "diff" => {
                    cfg.diff_path = Some(next_val("diff")?);
                }
                "svg" => {
                    cfg.svg_out = Some(next_val("svg")?);
                }
                "svg-title" => {
                    cfg.svg_title = Some(next_val("svg-title")?);
                }
                "theme" => {
                    let v = next_val("theme")?;
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
                    let v = next_val("color")?;
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
                other => {
                    let suggestion = suggest_long_flag(other);
                    let suffix = match suggestion {
                        Some(s) => format!(" (did you mean '--{s}'?)"),
                        None => String::from(" (run 'octet --help' for usage)"),
                    };
                    return Err(Error::InvalidArgument(format!(
                        "unrecognized option '--{other}'{suffix}"
                    )));
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            // Short flags (supporting combined flags like `-rp` or attached values like `-g4`)
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut c_idx = 0;
            while c_idx < chars.len() {
                let ch = chars[c_idx];
                let inline_rest: String = chars[c_idx + 1..].iter().collect();
                let mut take_short_val = |flag: char| -> Result<String> {
                    if !inline_rest.is_empty() {
                        Ok(inline_rest.clone())
                    } else if i + 1 < args.len() {
                        i += 1;
                        Ok(args[i].clone())
                    } else {
                        Err(Error::InvalidArgument(format!(
                            "option '-{flag}' requires an argument"
                        )))
                    }
                };

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
                    'u' => cfg.upper_hex = true,
                    'c' => {
                        let v = take_short_val('c')?;
                        cfg.cols = Some(parse_byte_count(&v)? as usize);
                        break;
                    }
                    'g' => {
                        let v = take_short_val('g')?;
                        cfg.group_size = Some(parse_byte_count(&v)? as usize);
                        break;
                    }
                    'l' | 'n' => {
                        let v = take_short_val(ch)?;
                        cfg.length = Some(parse_byte_count(&v)?);
                        break;
                    }
                    'o' => {
                        let v = take_short_val('o')?;
                        cfg.display_offset = Some(parse_byte_count(&v)?);
                        break;
                    }
                    's' => {
                        let v = take_short_val('s')?;
                        cfg.seek = Some(v);
                        break;
                    }
                    'S' => {
                        let v = take_short_val('S')?;
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
            positional.push(arg.clone());
        }
        i += 1;
    }

    if positional.len() > 2 {
        return Err(Error::InvalidArgument(
            "expected at most 2 positional arguments: [infile [outfile]]".to_owned(),
        ));
    }
    if let Some(first) = positional.first()
        && first != "-"
    {
        cfg.input_path = Some(first.clone());
    }
    if let Some(second) = positional.get(1)
        && second != "-"
    {
        cfg.output_path = Some(second.clone());
    }

    Ok(cfg)
}

fn read_input_window(
    path: Option<&str>,
    seek_spec: Option<&str>,
    length: Option<u64>,
) -> Result<(Vec<u8>, u64)> {
    if let Some(p) = path {
        let mut file = File::open(p)?;
        let start_off = if let Some(spec) = seek_spec {
            let trimmed = spec.trim();
            if let Some(neg) = trimmed.strip_prefix('-') {
                let back = parse_byte_count(neg)?;
                let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                let target = len.saturating_sub(back);
                file.seek(SeekFrom::Start(target))?
            } else {
                let pos = parse_byte_count(trimmed)?;
                match file.seek(SeekFrom::Start(pos)) {
                    Ok(actual) => actual,
                    Err(_) => {
                        io::copy(&mut io::Read::by_ref(&mut file).take(pos), &mut io::sink())?;
                        pos
                    }
                }
            }
        } else {
            0
        };

        let mut buf = Vec::new();
        if let Some(limit) = length {
            file.take(limit).read_to_end(&mut buf)?;
        } else {
            file.read_to_end(&mut buf)?;
        }
        Ok((buf, start_off))
    } else {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        if let Some(spec) = seek_spec {
            let trimmed = spec.trim();
            if trimmed.starts_with('-') {
                let mut full = Vec::new();
                handle.read_to_end(&mut full)?;
                let (sliced, off) = octet::slice_buffer(&full, Some(trimmed), length)?;
                return Ok((sliced.to_vec(), off));
            }
            let pos = parse_byte_count(trimmed)?;
            io::copy(&mut handle.by_ref().take(pos), &mut io::sink())?;
            let mut buf = Vec::new();
            if let Some(limit) = length {
                handle.take(limit).read_to_end(&mut buf)?;
            } else {
                handle.read_to_end(&mut buf)?;
            }
            Ok((buf, pos))
        } else {
            let mut buf = Vec::new();
            if let Some(limit) = length {
                handle.take(limit).read_to_end(&mut buf)?;
            } else {
                handle.read_to_end(&mut buf)?;
            }
            Ok((buf, 0))
        }
    }
}

fn write_output(path: Option<&str>, bytes: &[u8]) -> Result<()> {
    if let Some(p) = path {
        fs::write(p, bytes)?;
    } else {
        let mut stdout = io::stdout().lock();
        if let Err(err) = stdout.write_all(bytes) {
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
        "upper",
        "no-annotate",
        "search",
        "inspect",
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
            "--inspect <OFFSET>",
            "Decode u8/i8/u16/u32/u64/f32/f64/ULEB128/UTF-8 at <OFFSET>"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-i, --include [--lang <L>]",
            "Emit source array in rust, c (default), go, python, or ts"
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
            "octet --diff v1.bin v2.bin",
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
