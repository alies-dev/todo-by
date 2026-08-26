//! Runs the real binary against roots it refuses to walk.
//!
//! The unit tests cover the predicate; what they cannot cover is that the
//! predicate reaches the exit code, and the exit code is the whole point.
//! `todo-by .git` printing a notice and exiting 0 looks exactly like a
//! clean scan to the CI job reading only the status, which is the failure
//! this pins.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A tree with a `.git` holding a file worth not finding, and one real
/// source file beside it.
fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("todo-by-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git/logs")).expect("temp dir");
    // Split so the dogfood CI step, which scans this repository with the
    // binary under test, cannot mistake this line for a real overdue tag.
    // In the fixture it is contiguous and overdue: a regression that
    // walks `.git` after all would surface as a finding, not stay
    // invisible behind an empty log.
    fs::write(
        dir.join(".git/logs/HEAD"),
        concat!("0000 1111 commit: todo-", "by 2000-01-01 in metadata\n"),
    )
    .expect("git log");
    fs::write(dir.join("visible.rs"), "// nothing to find here\n").expect("source file");
    dir
}

/// A run that answers to its arguments and nothing else: `GITHUB_ACTIONS`
/// alone switches the output format, and a `todo-by.toml` above the
/// working directory would be loaded.
fn command(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_todo-by"));
    cmd.current_dir(dir)
        .env_remove("GITHUB_ACTIONS")
        .env_remove("TODO_BY_FORMAT")
        .env_remove("TODO_BY_WARN")
        .env_remove("TODO_BY_VERSION");
    cmd
}

fn todo_by(args: &[&Path], dir: &Path) -> Output {
    command(dir).args(args).output().expect("run todo-by")
}

/// The same run with `-` among its paths and `input` on stdin.
fn todo_by_with_stdin(args: &[&Path], dir: &Path, input: &str) -> Output {
    let mut child = command(dir)
        .args(args)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn todo-by");
    // A run that never reads stdin (`--files`) may exit and close the
    // pipe before the write lands; that early EPIPE is part of the
    // behavior under test, not a test failure.
    if let Err(err) = child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
    {
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::BrokenPipe,
            "write stdin: {err}"
        );
    }
    child.wait_with_output().expect("wait for todo-by")
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

#[test]
fn stdin_is_a_source_the_dropped_roots_cannot_take_away() {
    // `todo-by - .git` reads stdin and reports what it finds there, so
    // the scan ran. Failing it would break a pipeline for a path the
    // tool was never going to read anyway. The overdue tag on stdin
    // (split for the dogfood run's sake) is what proves stdin was
    // actually scanned: exit 0 alone could also mean it was never read.
    let dir = tree("stdin-kept");
    let git = dir.join(".git");
    let overdue = concat!("// todo-", "by 2000-01-01 expired on stdin\n");
    let out = todo_by_with_stdin(&[&git], &dir, overdue);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = fs::remove_dir_all(&dir);

    assert!(stderr.contains("never scanned"), "stderr: {stderr}");
    assert!(stdout.contains("<stdin>"), "stdout: {stdout}");
    assert_eq!(
        out.status.code(),
        Some(1),
        "the stdin finding decides the exit code, not the dropped root: {stderr}"
    );
}

#[test]
fn listing_files_with_only_stdin_has_nothing_to_list_and_says_so() {
    // `todo-by --files -` names a source `--files` can never use, so the
    // run lists nothing — which must not be mistakable for a clean empty
    // listing, the same contract the dropped-roots cases pin.
    let dir = tree("stdin-only-files");
    let files = Path::new("--files");
    let out = todo_by_with_stdin(&[files], &dir, "// nothing to find here\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = fs::remove_dir_all(&dir);

    assert!(stderr.contains("never reads stdin"), "stderr: {stderr}");
    assert!(out.stdout.is_empty(), "nothing was walked");
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
}

#[test]
fn listing_files_cannot_lean_on_stdin() {
    // `--files` lists walked paths and never reads stdin, so `-` leaves
    // it with nothing to list and nothing to say about it.
    let dir = tree("stdin-files");
    let git = dir.join(".git");
    let files = Path::new("--files");
    let out = todo_by_with_stdin(&[files, &git], &dir, "// nothing to find here\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = fs::remove_dir_all(&dir);

    assert!(out.stdout.is_empty(), "nothing was walked");
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
}
