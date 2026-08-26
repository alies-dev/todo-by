//! Runs the real binary against roots that overlap, proving a file
//! covered by two of them is reported once rather than once per root,
//! without an explicitly named root ever losing coverage to an
//! ancestor's ignore rules.
//!
//! The unit tests cover `collapse_duplicate_roots` and the walk's
//! containment filter directly; what they cannot cover is that `main`
//! wires them together correctly for the real binary, in either
//! argument order, which is what these pin.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A tree with one real file two directories down, so the root and its
/// own subdirectory can be passed together as overlapping arguments.
///
/// The tag dates to 2998: the dogfood CI step scans this very repository
/// with the binary under test, and an earlier date would make it mistake
/// this fixture's source line for a real overdue tag.
fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("todo-by-overlap-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("temp dir");
    fs::write(
        dir.join("src/lib.rs"),
        "// todo-by 2998-01-01 covered by two roots\n",
    )
    .expect("source file");
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

fn todo_by(args: &[&str], dir: &Path) -> Output {
    command(dir).args(args).output().expect("run todo-by")
}

#[test]
fn a_root_and_its_own_subdirectory_are_each_covered_once_by_the_deepest_root() {
    let dir = tree("files-nested");
    let forward = todo_by(&["--files", ".", "src"], &dir);
    let reversed = todo_by(&["--files", "src", "."], &dir);
    let _ = fs::remove_dir_all(&dir);

    let forward_out = String::from_utf8_lossy(&forward.stdout);
    let reversed_out = String::from_utf8_lossy(&reversed.stdout);
    assert_eq!(
        forward_out.lines().filter(|l| l.contains("lib.rs")).count(),
        1,
        "stdout: {forward_out}"
    );
    assert!(
        forward_out
            .lines()
            .any(|l| l.replace('\\', "/") == "src/lib.rs"),
        "the file under src must carry src's spelling, the deepest root \
         that covers it, not the ancestor's: {forward_out}"
    );
    assert_eq!(
        forward_out, reversed_out,
        "argument order must not change what --files reports, even though \
         both roots survive and each keeps its own subtree"
    );
}

#[test]
fn an_exact_duplicate_root_lists_each_file_once() {
    let dir = tree("files-duplicate");
    let out = todo_by(&["--files", "src", "src"], &dir);
    let _ = fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().filter(|l| l.contains("lib.rs")).count(),
        1,
        "stdout: {stdout}"
    );
}

#[test]
fn an_explicit_root_gitignored_by_its_parent_keeps_full_coverage() {
    // README: a path named on the command line is walked with root
    // semantics; ignore rules do not apply to it. Naming `.` alongside
    // it must not take that away.
    let dir = tree("gitignored-nested-root");
    fs::write(dir.join(".gitignore"), "generated/\n").expect(".gitignore");
    fs::create_dir_all(dir.join("generated")).expect("generated dir");
    fs::write(
        dir.join("generated/output.rs"),
        "// build artifact, not source\n",
    )
    .expect("generated file");

    let out = todo_by(&["--files", ".", "generated"], &dir);
    let _ = fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches("output.rs").count(),
        1,
        "an explicitly named root must keep its root semantics even when \
         another root covers it too, and must not be covered twice \
         either: {stdout}"
    );
}

#[test]
fn a_metadata_dir_named_alongside_another_root_still_gets_its_notice() {
    // AC4: nesting no longer removes a root, so `.git` named beside `.`
    // must still be refused with its usual notice, exactly as it was
    // before overlapping roots were handled at all. This is the one
    // behavior the nested-root-dropping approach broke: `.git` dedupped
    // away under `.` before the version-control check ever saw it.
    let dir = tree("git-notice");
    fs::create_dir_all(dir.join(".git/logs")).expect(".git dir");
    fs::write(dir.join(".git/logs/HEAD"), "0000 1111 commit: message\n").expect("git log");

    let out = todo_by(&[".", ".git"], &dir);
    let _ = fs::remove_dir_all(&dir);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("never scanned"), "stderr: {stderr}");
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
}

#[test]
fn a_root_and_its_own_subdirectory_report_the_finding_once() {
    let dir = tree("findings-nested");
    // Wide enough to bring a 2998 tag inside the warn window without
    // moving the fixture's own tag into the current era.
    let out = todo_by(&["--warn", "400000", ".", "src"], &dir);
    let _ = fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches("covered by two roots").count(),
        1,
        "stdout: {stdout}"
    );
}
