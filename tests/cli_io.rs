// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! How the `octet` binary reads its input and writes its output: endless and
//! huge inputs, seeking on devices, pipes and pseudo-files.

use std::io::Read as _;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

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

/// Waits for `child`, killing it and failing the test if it outlives
/// `limit`: the bugs these tests pin show up as hangs, not wrong output.
fn wait_within(mut child: Child, limit: Duration, what: &str) -> Output {
    let start = Instant::now();
    loop {
        if child.try_wait().expect("poll child").is_some() {
            return child.wait_with_output().expect("collect output");
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{what} still running after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_within(args: &[&str], limit: Duration) -> Output {
    let child = octet()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn octet");
    wait_within(child, limit, &format!("octet {}", args.join(" ")))
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

/// `octet --classic /dev/zero | head -c 100`: the dump must start at once and
/// stop quietly when the reader goes away, as `xxd /dev/zero | head` does.
#[cfg(unix)]
#[test]
fn an_endless_input_streams_into_a_closing_pipe() {
    for args in [&["--classic"][..], &["-C", "-v"], &["-p"], &["-i"]] {
        let mut child = octet()
            .args(args)
            .arg("/dev/zero")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn octet");
        let mut stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        // Reading on a thread lets a dump that never starts fail the test
        // instead of blocking it; dropping the pipe is what `head` exiting does.
        std::thread::spawn(move || {
            let mut head = [0_u8; 100];
            let _ = tx.send(stdout.read_exact(&mut head).map(|()| head));
        });
        let Ok(Ok(head)) = rx.recv_timeout(Duration::from_secs(5)) else {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{args:?}: no 100 bytes of output within 5 s");
        };
        let out = wait_within(child, Duration::from_secs(5), &format!("{args:?}"));
        assert!(out.status.success(), "{args:?}: {}", out.status);
        assert!(
            out.stderr.is_empty(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if args == ["--classic"] {
            assert!(head.starts_with(b"00000000: 0000 0000 0000 0000"));
        }
    }
}

/// The whole-buffer views cannot stream, but an endless input must not hang
/// them: they show a bounded prefix and say so on stderr.
#[cfg(unix)]
#[test]
fn whole_buffer_views_bound_an_endless_input() {
    for args in [&[][..], &["--compact"], &["--classic", "--json"]] {
        let mut argv = args.to_vec();
        argv.push("/dev/zero");
        let out = run_within(&argv, Duration::from_secs(20));
        let stdout = stdout_of(&out);
        assert!(!stdout.is_empty(), "{argv:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("first 16 MiB"), "{argv:?}: {stderr}");
        assert!(stderr.contains("-l"), "{argv:?}: {stderr}");
    }
    // With -l the reader knows how much to take, so there is nothing to say.
    let out = run_within(&["-l", "64", "/dev/zero"], Duration::from_secs(5));
    stdout_of(&out);
    assert!(out.stderr.is_empty());
}

/// A character device has no end to count back from: xxd says "Sorry,
/// cannot seek" at once, where v0.1.1 read `/dev/zero` forever.
#[cfg(unix)]
#[test]
fn a_seek_from_the_end_of_a_device_fails_at_once() {
    for args in [
        &["-C", "-s", "-16", "-l", "16", "/dev/zero"][..],
        &["--classic", "-s", "-1", "/dev/zero"],
        &["-s", "-16", "/dev/zero"],
    ] {
        let out = run_within(args, Duration::from_secs(5));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(stderr.contains("cannot seek"), "{args:?}: {stderr}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

/// A seek from the end of a pipe reads it to EOF but keeps only the tail.
#[cfg(unix)]
#[test]
fn a_seek_from_the_end_of_a_pipe_shows_the_tail() {
    use std::io::Write as _;
    let mut child = octet()
        .args(["-C", "-s", "-20", "-l", "4"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn octet");
    let mut stdin = child.stdin.take().expect("stdin");
    let writer = std::thread::spawn(move || {
        let block: Vec<u8> = (0..=255_u8).cycle().take(1 << 20).collect();
        for _ in 0..8 {
            stdin.write_all(&block).expect("feed octet");
        }
    });
    let out = wait_within(child, Duration::from_secs(20), "tail of a pipe");
    writer.join().expect("writer thread");
    // 8 MiB of a 256-byte cycle; the last 20 bytes start at 0x7fffec, byte 0xec.
    assert_eq!(
        stdout_of(&out),
        "007fffec  ec ed ee ef                                       |....|\n007ffff0\n"
    );
}

/// Linux answers `lseek` on `/dev/zero` with 0 instead of the requested
/// position, so a device is skipped by reading; xxd shows 00000100 here.
#[cfg(target_os = "linux")]
#[test]
fn a_seek_into_a_character_device_keeps_its_offset() {
    let out = run_within(
        &["--classic", "-s", "0x100", "-l", "4", "/dev/zero"],
        Duration::from_secs(5),
    );
    let dump = stdout_of(&out);
    assert!(dump.starts_with("00000100: 0000 0000 "), "{dump}");
    assert!(dump.ends_with("  ....\n"), "{dump}");
}

/// Files under /proc report a size of 0, so clamping a seek to the reported
/// size silently started the dump at 0.
#[cfg(target_os = "linux")]
#[test]
fn a_seek_into_a_proc_file_is_not_clamped_to_its_reported_size() {
    let version = std::fs::read("/proc/version").expect("read /proc/version");
    assert!(version.len() > 26, "/proc/version is unexpectedly short");
    let out = run_within(
        &["-p", "-s", "10", "-l", "16", "/proc/version"],
        Duration::from_secs(5),
    );
    let expected: String = version[10..26].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(stdout_of(&out), format!("{expected}\n"));
    let out = run_within(
        &["--classic", "-s", "10", "-l", "1", "/proc/version"],
        Duration::from_secs(5),
    );
    assert!(stdout_of(&out).starts_with("0000000a: "));
}
