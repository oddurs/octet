// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Command-line interface for `octet`.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::path::Path;
use std::process::ExitCode;

use octet::{
    CodeLanguage, ColorMode, Error, FormatOptions, Radix, Result, Theme, ValueInspection,
    ViewStyle, compute_diff, find_pattern_spans, generate_code, parse_byte_count,
    parse_search_pattern, render_ansi_to_svg, render_diff_ansi, render_dump, reverse_dump,
    slice_buffer,
};

fn main() -> ExitCode {
    match run_cli(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "\x1b[1;31moctet error:\x1b[0m {err}");
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
            theme: Theme::dark(),
            color_mode: ColorMode::Auto,
            input_path: None,
            output_path: None,
        }
    }
}

fn run_cli(args: Vec<String>) -> Result<()> {
    let cfg = parse_args(&args)?;
    let is_tty = io::stdout().is_terminal();
    let color = cfg.color_mode.is_enabled(is_tty) || cfg.svg_out.is_some();

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

    let raw_input = read_input(cfg.input_path.as_deref())?;

    if cfg.reverse {
        let text = String::from_utf8_lossy(&raw_input);
        let bytes = reverse_dump(&text, cfg.style == ViewStyle::Plain)?;
        write_output(cfg.output_path.as_deref(), &bytes)?;
        return Ok(());
    }

    let (sliced, start_off) = slice_buffer(&raw_input, cfg.seek.as_deref(), cfg.length)?;
    let base_disp_offset = cfg.display_offset.unwrap_or(start_off);

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
        let right_raw = fs::read(right_path)?;
        let (right_sliced, _) = slice_buffer(&right_raw, cfg.seek.as_deref(), cfg.length)?;
        let cols = cfg.cols.unwrap_or(8);
        let autoskip = cfg.autoskip.unwrap_or(true);
        let report = compute_diff(sliced, right_sliced, cols, base_disp_offset, autoskip);
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

    // Standard / Smart / Classic hex or radix dump
    let default_cols = match cfg.radix {
        Radix::Binary => 6,
        Radix::Hex | Radix::Octal | Radix::Decimal => 16,
    };
    let cols = cfg.cols.unwrap_or(default_cols);
    let default_group = match cfg.radix {
        Radix::Binary | Radix::Octal | Radix::Decimal => 1,
        Radix::Hex => {
            if cfg.little_endian {
                4
            } else {
                2
            }
        }
    };
    let group_size = cfg.group_size.unwrap_or(default_group);
    let autoskip = cfg
        .autoskip
        .unwrap_or(cfg.style == ViewStyle::Smart || cfg.style == ViewStyle::Compact);

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
                "bits" | "binary" => cfg.radix = Radix::Binary,
                "octal" => cfg.radix = Radix::Octal,
                "decimal" => cfg.radix = Radix::Decimal,
                "cols" | "columns" => {
                    let v = next_val("cols")?;
                    cfg.cols = Some(parse_byte_count(&v)? as usize);
                }
                "groupsize" | "group" => {
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
                "length" | "len" => {
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
                    return Err(Error::InvalidArgument(format!(
                        "unrecognized option '--{other}' (run 'octet --help' for usage)"
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
                    'v' | 'V' => cfg.version = true,
                    'a' => cfg.autoskip = Some(true),
                    'b' => cfg.radix = Radix::Binary,
                    'd' => cfg.radix = Radix::Decimal,
                    'e' => cfg.little_endian = true,
                    'i' => cfg.include_codegen = true,
                    'p' => cfg.style = ViewStyle::Plain,
                    'q' => cfg.style = ViewStyle::Compact,
                    'C' => cfg.style = ViewStyle::Smart,
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

fn read_input(path: Option<&str>) -> Result<Vec<u8>> {
    if let Some(p) = path {
        Ok(fs::read(p)?)
    } else {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf)?;
        Ok(buf)
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

fn render_help(t: &Theme, c: bool) -> String {
    let mut out = String::new();
    let bar = t.paint(&"─".repeat(86), t.border, c);
    let _ = writeln!(
        out,
        "{}{bar}{}",
        t.paint("╭", t.border, c),
        t.paint("╮", t.border, c)
    );
    let _ = writeln!(
        out,
        "{}  {} — {}          {}",
        t.paint("│", t.border, c),
        t.paint_bold("◈ OCTET", t.offset_active, c),
        t.paint(
            "Information-Design Hex Viewer, Binary Inspector & xxd Replacement",
            t.fg_primary,
            c
        ),
        t.paint("│", t.border, c)
    );
    let _ = writeln!(
        out,
        "{}{bar}{}",
        t.paint("╰", t.border, c),
        t.paint("╯", t.border, c)
    );

    let section =
        |title: &str| -> String { format!("\n{}", t.paint_bold(title, t.byte_control, c)) };
    let flag = |f: &str, desc: &str| -> String {
        format!(
            "  {:<28} {}",
            t.paint_bold(f, t.byte_ascii, c),
            t.paint(desc, t.fg_primary, c)
        )
    };

    let _ = writeln!(out, "{}", section("USAGE"));
    let _ = writeln!(
        out,
        "  {} {}",
        t.paint_bold("octet", t.offset_active, c),
        t.paint("[OPTIONS] [INFILE [OUTFILE]]", t.fg_secondary, c)
    );

    let _ = writeln!(
        out,
        "{}",
        section("LAYOUT & RADIX OPTIONS (xxd / hexdump compatible)")
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-c, --cols <N>",
            "Format <N> octets per row (default: 16, binary: 6)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-g, --groupsize <N>",
            "Group <N> octets per space-separated block (default: 2)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-e, --little-endian",
            "Little-endian byte grouping (default group size: 4)"
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
            "-d, --decimal / --octal",
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
            "-p, --plain",
            "Continuous plain hexadecimal stream (xxd -p)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-q, --compact / --classic",
            "Compact table without cards (-q) or strict xxd output (--classic)"
        )
    );

    let _ = writeln!(out, "{}", section("SEEKING, SLICING & ELISION"));
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-s, --seek <[+-]OFFSET>",
            "Start at <OFFSET> (hex 0x100, KiB, or -0x40 from EOF)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "-l, --length <LEN>",
            "Read at most <LEN> bytes (also -n <LEN>)"
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
            "-a, --autoskip / --no-autoskip",
            "Collapse consecutive identical/zero rows (default in smart mode)"
        )
    );

    let _ = writeln!(out, "{}", section("INSPECTION, DIFFING, CODEGEN & REVERSE"));
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
            "Reverse hex dump (octet, xxd, or -r -p plain) back to raw binary"
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

    let _ = writeln!(out, "{}", section("DESIGN SYSTEM & EXPORT"));
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--theme <dark|light>",
            "Select calibrated color palette (default: dark)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--color <auto|always|never>",
            "Control ANSI TrueColor output (respects NO_COLOR)"
        )
    );
    let _ = writeln!(
        out,
        "{}",
        flag(
            "--svg <PATH>",
            "Render vector terminal window screenshot to <PATH>"
        )
    );
    out
}
