use std::path::Path;

use crate::date::{deadline, Date};

const TAG: &[u8] = b"todo-by";

pub enum Kind {
    Overdue,
    /// Impossible date, e.g. 2026-02-30.
    InvalidDate,
}

pub struct Finding {
    pub file: String,
    pub line: usize,
    pub kind: Kind,
    /// Date as written in the tag, not normalized.
    pub date: String,
    pub message: String,
}

/// Extracts `(date, message)` from a line with a todo-by tag, case-insensitive:
/// `@todo-by 2026-12-31 message`, `TODO-BY: 2026-09 - message`, etc.
pub fn match_line(line: &str) -> Option<(&str, String)> {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + TAG.len() < n {
        if !bytes[i..i + TAG.len()].eq_ignore_ascii_case(TAG) {
            i += 1;
            continue;
        }
        // word boundary: don't match inside identifiers
        if i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
                i += 1;
                continue;
            }
        }
        let mut j = i + TAG.len();
        if j < n && bytes[j] == b':' {
            j += 1;
        }
        let ws_start = j;
        while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if j == ws_start {
            i += TAG.len();
            continue;
        }
        if let Some(end) = parse_date_span(bytes, j) {
            return Some((&line[j..end], clean_message(&line[end..])));
        }
        i += TAG.len();
    }
    None
}

/// Parses `YYYY(-M+(-D+)?)?` at `start`, returns its end. Rejects a fifth
/// year digit. Consumes sloppy components (`2026-1-5`, `2026-123`) in full so
/// `date::deadline` can judge them numerically; truncating to `2026` here
/// would silently postpone the deadline to Dec 31.
fn parse_date_span(bytes: &[u8], start: usize) -> Option<usize> {
    let mut j = start;
    for _ in 0..4 {
        if !bytes.get(j).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        j += 1;
    }
    if bytes.get(j).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    for _ in 0..2 {
        if bytes.get(j) == Some(&b'-') && bytes.get(j + 1).is_some_and(u8::is_ascii_digit) {
            j += 1;
            while bytes.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
        } else {
            break;
        }
    }
    Some(j)
}

fn clean_message(rest: &str) -> String {
    let mut msg = rest.trim_start();
    if let Some(stripped) = msg.strip_prefix('-').or_else(|| msg.strip_prefix(':')) {
        msg = stripped.trim_start();
    }
    for closer in ["*/", "-->", "#}", "}}"] {
        if let Some(stripped) = msg.strip_suffix(closer) {
            msg = stripped;
        }
    }
    msg.trim().to_string()
}

pub fn scan_file(path: &Path, today: Date, findings: &mut Vec<Finding>) -> std::io::Result<()> {
    let content = std::fs::read(path)?;
    // binary heuristic: NUL byte in the first 8 KiB
    if content.iter().take(8192).any(|&b| b == 0) {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&content);

    for (idx, line) in text.lines().enumerate() {
        let Some((written, message)) = match_line(line) else {
            continue;
        };
        let (kind, is_finding) = match deadline(written) {
            None => (Kind::InvalidDate, true),
            Some(due) => (Kind::Overdue, due <= today),
        };
        if is_finding {
            findings.push(Finding {
                file: path.display().to_string(),
                line: idx + 1,
                kind,
                date: written.to_string(),
                message,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_common_comment_styles() {
        let cases = [
            (
                "// @todo-by 2999-12-31 remove flag",
                "2999-12-31",
                "remove flag",
            ),
            (
                "# todo-by 2999-09-01 drop webhook",
                "2999-09-01",
                "drop webhook",
            ),
            (
                "* @todo-by 2999-10-01 - Kennedy, check this",
                "2999-10-01",
                "Kennedy, check this",
            ),
            (
                "/** @todo-by 2999-04-20 - Alies, Optimize it */",
                "2999-04-20",
                "Alies, Optimize it",
            ),
            ("<!-- TODO-BY: 2999-01 clean up -->", "2999-01", "clean up"),
            (
                "@todo-by 2999-08 - month precision",
                "2999-08",
                "month precision",
            ),
            ("-- todo-by 2999 rewrite in SQL", "2999", "rewrite in SQL"),
            (
                "{# todo-by: 2999-05-05 twig comment #}",
                "2999-05-05",
                "twig comment",
            ),
        ];
        for (line, want_date, want_msg) in cases {
            let (date, msg) = match_line(line).unwrap_or_else(|| panic!("no match: {line}"));
            assert_eq!(date, want_date, "date in {line:?}");
            assert_eq!(msg, want_msg, "message in {line:?}");
        }
    }

    #[test]
    fn ignores_lines_without_a_date() {
        assert_eq!(match_line("todo-by [PATHS]..."), None);
        assert_eq!(match_line("plain TODO: fix later"), None);
        assert_eq!(match_line("todo-by"), None);
        assert_eq!(match_line("todo-by 20261 five digits"), None);
        assert_eq!(match_line("autodo-by 2999-01-01 not a word boundary"), None);
    }

    #[test]
    fn impossible_dates_still_match_for_reporting() {
        // built at runtime so scanning this repo doesn't flag the fixture
        let line = format!("// todo-by {} bad", "2999-13-45");
        let (date, _) = match_line(&line).unwrap();
        assert_eq!(date, "2999-13-45");
    }

    #[test]
    fn sloppy_dates_are_consumed_in_full_not_truncated() {
        let (date, msg) = match_line("// todo-by 2999-1-5 sloppy").unwrap();
        assert_eq!(date, "2999-1-5");
        assert_eq!(msg, "sloppy");

        // Consumed whole so deadline() reports it invalid instead of the
        // tag silently meaning "2026", i.e. a later deadline.
        let line = format!("// todo-by {} overlong month", "2026-123");
        let (date, _) = match_line(&line).unwrap();
        assert_eq!(date, "2026-123");
    }
}
