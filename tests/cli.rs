// Copyright 2026 The Octet Authors.
// Dual-licensed under MIT or Apache-2.0.

//! Command-line behavior tests that run the built `octet` binary.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

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
