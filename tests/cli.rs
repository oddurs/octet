// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Command-line behavior tests that run the built `octet` binary.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// The binary with every color-related environment variable cleared, so the
/// developer's or CI's environment cannot change what a test observes.
fn octet() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_octet"));
    for var in [
        "NO_COLOR",
        "FORCE_COLOR",
        "OCTET_COLOR",
        "OCTET_THEME",
        "TERM",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// A per-test scratch directory; tests run in parallel, so each gets its own.
fn scratch_dir(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(test);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn scratch_file(test: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let path = scratch_dir(test).join(name);
    fs::write(&path, bytes).expect("write scratch file");
    path
}

fn has_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

fn stdout_of(out: &Output) -> String {
    assert!(
        out.status.success(),
        "octet exited with {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).expect("stdout is UTF-8")
}

/// Asserts a clean usage failure: exit code 1 (not a panic's 101) and an error
/// message containing `needle`.
fn assert_fails_with(out: &Output, needle: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains(needle), "expected '{needle}' in: {stderr}");
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}

#[test]
fn color_always_still_colors_an_outfile() {
    let input = scratch_file("color_always_outfile", "in.bin", b"hello world");
    let output = input.with_file_name("out.txt");
    let out = octet()
        .args(["--classic", "--color", "always"])
        .arg(&input)
        .arg(&output)
        .output()
        .expect("run octet");
    stdout_of(&out);
    assert!(has_ansi(&fs::read(&output).expect("read outfile")));
}

/// Runs `octet <args>` under `script(1)` so its stdout is a pseudo-terminal.
#[cfg(unix)]
fn run_on_pty(args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_octet");
    let mut cmd = Command::new("script");
    if cfg!(target_os = "linux") {
        // util-linux takes the command as one shell string.
        let line = std::iter::once(bin)
            .chain(args.iter().copied())
            .map(|a| format!("'{a}'"))
            .collect::<Vec<_>>()
            .join(" ");
        cmd.args(["-qec", &line, "/dev/null"]);
    } else {
        cmd.args(["-q", "/dev/null", bin]).args(args);
    }
    for var in ["NO_COLOR", "FORCE_COLOR", "OCTET_COLOR", "OCTET_THEME"] {
        cmd.env_remove(var);
    }
    cmd.env("TERM", "xterm-256color")
        .stdin(Stdio::null())
        .output()
        .expect("run script(1)")
}

#[cfg(unix)]
#[test]
fn auto_color_is_off_for_an_outfile_even_when_stdout_is_a_terminal() {
    let input = scratch_file("auto_color_outfile", "in.bin", b"hello world");
    let output = input.with_file_name("out.txt");
    let input_s = input.to_str().expect("UTF-8 temp path");
    let output_s = output.to_str().expect("UTF-8 temp path");

    // Without this control the test would pass vacuously if script(1) failed
    // to give the child a terminal.
    let on_tty = run_on_pty(&["--classic", input_s]);
    assert!(on_tty.status.success(), "{on_tty:?}");
    assert!(has_ansi(&on_tty.stdout), "pty stdout should be colored");

    let to_file = run_on_pty(&["--classic", input_s, output_s]);
    assert!(to_file.status.success(), "{to_file:?}");
    assert!(!has_ansi(&fs::read(&output).expect("read outfile")));
}

#[test]
fn force_color_zero_false_or_empty_does_not_force_color() {
    let input = scratch_file("force_color", "in.bin", b"hello world");
    for value in ["0", "false", "FALSE", ""] {
        let out = octet()
            .arg("--classic")
            .arg(&input)
            .env("FORCE_COLOR", value)
            .output()
            .expect("run octet");
        assert!(
            !has_ansi(stdout_of(&out).as_bytes()),
            "FORCE_COLOR={value:?} forced color"
        );
    }
    let forced = octet()
        .arg("--classic")
        .arg(&input)
        .env("FORCE_COLOR", "1")
        .output()
        .expect("run octet");
    assert!(has_ansi(stdout_of(&forced).as_bytes()));
}

#[cfg(unix)]
#[test]
fn non_utf8_arguments_do_not_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let missing = OsString::from_vec(vec![b'n', 0xff]);
    let out = octet().arg("-p").arg(&missing).output().expect("run octet");
    assert_fails_with(&out, "I/O error");

    let bad_value = OsString::from_vec(vec![b'1', 0xff]);
    let out = octet()
        .arg("-s")
        .arg(&bad_value)
        .arg("Cargo.toml")
        .output()
        .expect("run octet");
    assert_fails_with(&out, "not valid UTF-8");

    let bad_flag = OsString::from_vec(vec![b'-', b'-', 0xff]);
    let out = octet()
        .arg(&bad_flag)
        .arg("Cargo.toml")
        .output()
        .expect("run octet");
    assert_fails_with(&out, "not valid UTF-8");
}

// macOS file systems reject non-UTF-8 names, so only Linux can create one.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_file_names_are_read() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let dir = scratch_dir("non_utf8_file_names");
    let path = dir.join(OsString::from_vec(vec![b'f', 0xff]));
    fs::write(&path, b"AB").expect("write non-UTF-8 file");
    let out = octet().arg("-p").arg(&path).output().expect("run octet");
    assert_eq!(stdout_of(&out), "4142\n");

    let diff = octet()
        .args(["--color", "never", "--diff"])
        .arg(&path)
        .arg(&path)
        .output()
        .expect("run octet --diff");
    assert!(stdout_of(&diff).contains("BINARY DIFF"));
}

#[test]
fn closed_stdout_is_not_a_panic() {
    let input = scratch_file("closed_stdout", "in.bin", b"hello world");
    let cases: [&[&str]; 5] = [
        &["-V"],
        &["--help"],
        &["--man"],
        &["--completions", "bash"],
        &["--classic"],
    ];
    for args in cases {
        // A pipe whose read end is already closed makes every write fail
        // with EPIPE, deterministically.
        let (reader, writer) = std::io::pipe().expect("create pipe");
        drop(reader);
        let out = octet()
            .args(args)
            .arg(&input)
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .expect("run octet");
        assert!(
            out.status.success() && out.stderr.is_empty(),
            "{args:?}: {} {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn strings_does_not_swallow_a_file_name_starting_with_a_digit() {
    let dir = scratch_dir("strings_digit_name");
    fs::write(dir.join("1.bin"), b"\x00\x00hello world\x00abc\x00").expect("write");
    let out = octet()
        .current_dir(&dir)
        .args(["--strings", "1.bin", "-j"])
        .output()
        .expect("run octet");
    let json = stdout_of(&out);
    assert!(json.contains("\"text\": \"hello world\""), "{json}");
    assert!(!json.contains("\"text\": \"abc\""), "{json}");

    let out = octet()
        .current_dir(&dir)
        .args(["--strings=3", "1.bin", "-j"])
        .output()
        .expect("run octet");
    assert!(stdout_of(&out).contains("\"text\": \"abc\""));
}

#[test]
fn incompatible_modes_are_rejected() {
    let input = scratch_file("incompatible_modes", "in.bin", b"hello world");
    let input_s = input.to_str().expect("UTF-8 temp path");
    let cases: [(&[&str], &str); 6] = [
        (
            &["-j", "-i"],
            "-j/--json cannot be combined with -i/--include",
        ),
        (
            &["-r", "-j"],
            "-j/--json cannot be combined with -r/--reverse",
        ),
        (
            &["-i", "--strings"],
            "-i/--include cannot be combined with --strings",
        ),
        (
            &["--diff", input_s, "-r"],
            "-r/--reverse cannot be combined with --diff",
        ),
        (
            &["-j", "--svg", "x.svg"],
            "--svg cannot be combined with -j/--json",
        ),
        (
            &["--strings", "--inspect", "0"],
            "--inspect cannot be combined with --strings",
        ),
    ];
    for (args, message) in cases {
        let out = octet().args(args).arg(&input).output().expect("run octet");
        assert_fails_with(&out, message);
        assert!(out.stdout.is_empty(), "{args:?} wrote output");
    }
}

#[test]
fn boolean_long_flags_reject_a_value() {
    let input = scratch_file("boolean_flag_value", "in.bin", b"hello world");
    for flag in ["--json=no", "--classic=1", "--reverse=false", "--help=yes"] {
        let out = octet().arg(flag).arg(&input).output().expect("run octet");
        assert_fails_with(&out, "does not take a value");
    }
    let out = octet()
        .arg("--jsn=1")
        .arg(&input)
        .output()
        .expect("run octet");
    assert_fails_with(&out, "unrecognized option '--jsn'");
}

#[cfg(unix)]
#[test]
fn seek_on_a_pipe_path_skips_by_reading() {
    for (seek, expected) in [("2", "636465666768\n"), ("-3", "666768\n")] {
        let mut child = octet()
            .args(["-p", "-s", seek, "/dev/stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn octet");
        // macOS reports the bytes already buffered in a pipe as its size, so
        // write only after octet has had time to look; the fixed code reads
        // the same window whatever the timing.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(b"abcdefgh").expect("write stdin");
        drop(stdin);
        let out = child.wait_with_output().expect("wait octet");
        assert_eq!(stdout_of(&out), expected, "-s {seek}");
    }
}

#[test]
fn inspect_before_the_seek_start_is_out_of_range() {
    let data: Vec<u8> = (0..=255u8).cycle().take(0x200).collect();
    let input = scratch_file("inspect_before_start", "in.bin", &data);
    let out = octet()
        .args([
            "--color",
            "never",
            "-s",
            "0x100",
            "-l",
            "16",
            "--inspect",
            "0x10",
        ])
        .arg(&input)
        .output()
        .expect("run octet");
    assert_fails_with(&out, "outside buffer range");

    let inside = octet()
        .args(["--color", "never", "-s", "0x100", "-l", "16", "--inspect"])
        .arg("0x104")
        .arg(&input)
        .output()
        .expect("run octet");
    assert!(stdout_of(&inside).contains("0x00000104"));
}

#[test]
fn zero_columns_means_the_default_width() {
    let data: Vec<u8> = (0..100u8).collect();
    let input = scratch_file("zero_columns", "in.bin", &data);
    let render = |args: &[&str]| {
        let out = octet()
            .args(["--color", "never"])
            .args(args)
            .arg(&input)
            .output()
            .expect("run octet");
        stdout_of(&out)
    };
    for mode in [
        &["--classic"][..],
        &["--classic", "-b"],
        &["-C"],
        &[],
        &["-i"],
        &["--diff", input.to_str().expect("UTF-8 temp path")],
    ] {
        let with_zero: Vec<&str> = mode.iter().copied().chain(["-c", "0"]).collect();
        assert_eq!(render(&with_zero), render(mode), "{mode:?} -c 0");
    }
    // As in xxd, plain mode with zero columns is one unbroken line.
    assert_eq!(render(&["-p", "-c", "0"]).lines().count(), 1);
}
