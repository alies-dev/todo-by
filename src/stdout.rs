//! The one path everything that reaches stdout goes through.
//!
//! A reader is allowed to stop reading. `| head -4`, `less` quit on the
//! first page, `grep -q` that already matched: each closes the pipe, and
//! the next write fails with `BrokenPipe`. `println!` answers that by
//! panicking, so an ordinary pipeline turned into a crash report and an
//! exit code (101, or an abort under `panic = "abort"`) that `--help`
//! never documented. Writing through a handle whose errors are returned
//! rather than unwrapped is what makes that impossible, and keeping it to
//! one place is what keeps it that way; `clippy::print_stdout`, denied at
//! the crate root, is the guard that a later `println!` cannot slip past.
//!
//! Buffering comes along for free and is worth having on its own: the raw
//! `Stdout` handle is line buffered, so a `--files` listing paid a write
//! syscall per path.

use std::io::{self, BufWriter, ErrorKind, Write};

/// Runs `f` against a buffered, locked stdout, then flushes.
///
/// Returns `Ok(())` when the output either completed or was cut short by a
/// reader that stopped reading, and the error otherwise. Callers report
/// that error and exit 2, the code the contract already gives an I/O
/// failure.
pub fn write(f: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    // Flushed here rather than left to `BufWriter`'s `Drop`, which
    // discards the error: a report truncated by a full disk that still
    // exits 0 is the same silent failure in a different costume.
    finish(f(&mut out).and_then(|()| out.flush()))
}

/// Decides which stdout failures are failures.
///
/// `BrokenPipe` is not one. The reader got what it asked for and went
/// away, so output stops where it stopped and the run keeps the exit code
/// it had already earned: `0` no findings, `1` findings, `2` error. That
/// is deliberately not the blanket `0` that ripgrep and bat return, and
/// this tool can afford the difference because it collects every finding
/// before it writes the first line. The verdict is already known when the
/// pipe breaks, so `todo-by | head -1` under `set -o pipefail` still fails
/// a CI job that has overdue tags, and still passes one that does not.
fn finish(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_output_is_success() {
        assert!(finish(Ok(())).is_ok());
    }

    #[test]
    fn closed_reader_is_not_a_failure() {
        let err = io::Error::new(ErrorKind::BrokenPipe, "Broken pipe (os error 32)");
        assert!(finish(Err(err)).is_ok());
    }

    #[test]
    fn other_errors_survive_unchanged() {
        let err = io::Error::new(ErrorKind::StorageFull, "No space left on device");
        let got = finish(Err(err)).unwrap_err();
        assert_eq!(got.kind(), ErrorKind::StorageFull);
        assert_eq!(got.to_string(), "No space left on device");
    }
}
