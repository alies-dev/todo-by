//! Runs the real binary against roots it refuses to walk.
//!
//! The unit tests cover the predicate; what they cannot cover is that the
//! predicate reaches the exit code, and the exit code is the whole point.
//! `todo-by .git` printing a notice and exiting 0 looks exactly like a
//! clean scan to the CI job reading only the status, which is the failure
//! this pins.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A tree with a `.git` holding a file worth not finding, and one real
/// source file beside it.
fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("todo-by-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git/logs")).expect("temp dir");
    fs::write(dir.join(".git/logs/HEAD"), "0000 1111 commit: message\n").expect("git log");
    fs::write(dir.join("visible.rs"), "// nothing to find here\n").expect("source file");
    dir
}

/// A run that answers to its arguments and nothing else: `GITHUB_ACTIONS`
/// alone switches the output format, and a `todo-by.toml` above the
/// working directory would be loaded.
fn todo_by(args: &[&Path], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_todo-by"))
        .args(args)
        .current_dir(dir)
        .env_remove("GITHUB_ACTIONS")
        .env_remove("TODO_BY_FORMAT")
        .env_remove("TODO_BY_WARN")
        .env_remove("TODO_BY_VERSION")
        .output()
        .expect("run todo-by")
}

#[test]
fn a_scan_left_with_no_roots_at_all_exits_2() {
    let dir = tree("all-dropped");
    let git = dir.join(".git");
    let out = todo_by(&[&git], &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = fs::remove_dir_all(&dir);

    assert!(stderr.contains("never scanned"), "stderr: {stderr}");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a scan that could not run must not look like one that found nothing: {stderr}"
    );
    assert!(out.stdout.is_empty(), "nothing was scanned");
}

#[test]
fn a_scan_that_kept_a_root_still_exits_on_what_it_found() {
    // The other half of the contract: dropping *some* roots is not a
    // failure, since the scan ran over everything else it was given.
    let dir = tree("some-dropped");
    let git = dir.join(".git");
    let visible = dir.join("visible.rs");
    let out = todo_by(&[&git, &visible], &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = fs::remove_dir_all(&dir);

    assert!(stderr.contains("never scanned"), "stderr: {stderr}");
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
}
