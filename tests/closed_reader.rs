//! Runs the real binary against a reader that stops reading.
//!
//! The unit tests cover the pieces: `stdout::finish` classifies a
//! `BrokenPipe`, `output::render` returns one instead of panicking. What
//! they cannot cover is the wiring, and the wiring is where the bug lived:
//! `println!` unwrapping a write error is invisible to any test that hands
//! the renderer its own writer. So this spawns `todo-by`, reads one line,
//! closes the pipe, and asks what the process did about it.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Builds a tree whose output cannot fit in a pipe buffer.
///
/// That size is the whole point. A write only fails once it has to wait
/// for a reader, so an output small enough to sit in the buffer succeeds
/// no matter who is listening: scanning this repository, `--files` prints
/// 27 lines, `| head -4` never broke, and the bug went unnoticed until
/// someone pointed the tool at something large. 3000 files put a few
/// hundred KB through a buffer that holds 64.
fn tree(name: &str, files: usize, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("todo-by-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    for i in 0..files {
        // Long enough that the paths alone outgrow the buffer.
        let path = dir.join(format!("a-file-with-a-realistic-length-of-name-{i:05}.rs"));
        fs::write(&path, contents).expect("fixture file");
    }
    dir
}

/// Spawns `todo-by` over `dir`, reads one line, then closes the pipe the
/// way `head` does, and returns the exit code and everything on stderr.
fn run_and_stop_reading(args: &[&str], dir: &Path) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_todo-by"))
        .args(args)
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn todo-by");

    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut first = String::new();
    stdout.read_line(&mut first).expect("read first line");
    assert!(!first.trim().is_empty(), "expected output to read");
    drop(stdout);

    let done = child.wait_with_output().expect("wait for todo-by");
    (
        done.status.code(),
        String::from_utf8_lossy(&done.stderr).into_owned(),
    )
}

#[test]
fn listing_files_into_a_closed_pipe_exits_clean() {
    let dir = tree("files", 3000, "nothing to find here\n");
    let (code, stderr) = run_and_stop_reading(&["--files"], &dir);
    let _ = fs::remove_dir_all(&dir);

    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    // Nothing was wrong with the run, and nothing is wrong with a reader
    // that had enough.
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn findings_into_a_closed_pipe_keep_their_exit_code() {
    // The claim this pins: `todo-by | head -1` under `set -o pipefail`
    // still fails a job whose tree has overdue tags. A blanket exit 0,
    // which is what ripgrep and bat return for a broken pipe, would pass
    // it silently.
    let dir = tree(
        "findings",
        3000,
        "// todo-by 2000-01-01: expired long ago\n",
    );
    let (code, stderr) = run_and_stop_reading(&[], &dir);
    let _ = fs::remove_dir_all(&dir);

    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert_eq!(code, Some(1), "stderr: {stderr}");
    // The count still arrives: the reader closed stdout, not stderr.
    assert!(stderr.contains("3000 findings"), "stderr: {stderr}");
}

#[test]
fn a_closed_pipe_does_not_hide_a_real_error() {
    // An unreadable path is an error whatever the reader does, so the
    // broken pipe must not talk the run down from 2 to 0.
    let dir = tree("error", 3000, "nothing to find here\n");
    let mut child = Command::new(env!("CARGO_BIN_EXE_todo-by"))
        .arg("--files")
        .arg(&dir)
        .arg(dir.join("no-such-path"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn todo-by");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut first = String::new();
    stdout.read_line(&mut first).expect("read first line");
    drop(stdout);
    let done = child.wait_with_output().expect("wait for todo-by");
    let _ = fs::remove_dir_all(&dir);

    let stderr = String::from_utf8_lossy(&done.stderr);
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert_eq!(done.status.code(), Some(2), "stderr: {stderr}");
}
