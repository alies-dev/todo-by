use std::path::Path;

use crate::date::{deadline, Date};
use crate::version::{Constraint, COMPARATORS};

/// What triggered a finding, and its outcome. `written` preserves the
/// source text exactly (not normalized) on every variant, so output can
/// quote the tag as the author wrote it. Carrying each variant's data
/// directly (rather than a parallel `Trigger` enum) makes the pairing
/// compile-time: there's no way to construct, say, an `Overdue` without a
/// `deadline`, or a `VersionReached` that also carries a `Constraint`.
pub enum Kind {
    Overdue {
        written: String,
        deadline: Date,
    },
    /// Due within the warn window, not yet overdue.
    DueSoon {
        written: String,
        deadline: Date,
    },
    /// Impossible date, e.g. 2026-02-30.
    InvalidDate {
        written: String,
    },
    /// A syntactically valid constraint whose satisfaction the scanner
    /// can't judge: it doesn't know the project's current version. main.rs
    /// resolves these once, after the scan, into `VersionReached` or drops
    /// the finding entirely when not yet satisfied. Never reaches output
    /// rendering.
    VersionPending {
        written: String,
        constraint: Constraint,
    },
    /// The current version satisfies a version constraint.
    VersionReached {
        written: String,
    },
    /// Bad version syntax, or a syntactically version-like but unsupported
    /// comparator (`<`, `<=`, `=`, `==`).
    InvalidTrigger {
        written: String,
    },
}

pub struct Finding {
    pub file: String,
    pub line: usize,
    pub kind: Kind,
    pub message: String,
}

pub struct ScanCtx<'a> {
    pub today: Date,
    /// Inclusive upper bound for DueSoon findings (today + warn window).
    /// None disables warn-ahead.
    pub warn_until: Option<Date>,
    /// Tags to match, in priority order. Never empty. Matching is
    /// case-insensitive regardless of the case stored here.
    pub tags: &'a [String],
}

/// Extracts `(date, message)` for the first matching tag in `line`,
/// case-insensitive: `@todo-by 2999-12-31 message`, `TODO-BY: 2999-09 -
/// message`, etc. A thin wrapper over [`match_line_from`] starting at
/// position 0, kept so existing single-trigger tests don't need to change
/// shape.
#[cfg(test)]
pub fn match_line<'a>(line: &'a str, tags: &[String]) -> Option<(&'a str, String)> {
    match_line_from(line, 0, tags, &tag_firsts(tags))
        .map(|(written, message, _end)| (written, message))
}

/// Lowercased first byte of each tag: the per-byte fast-reject set for
/// [`match_line_from`]. Built once per file, not per line or per byte.
fn tag_firsts(tags: &[String]) -> Vec<u8> {
    tags.iter()
        .filter_map(|t| t.as_bytes().first())
        .map(u8::to_ascii_lowercase)
        .collect()
}

/// Finds the next matching tag in `line` at or after absolute byte offset
/// `start`, returning `(written span, message, end)` where `end` is the
/// absolute offset just past the consumed trigger span. Tries `tags` in
/// order at each scan position; the first tag that yields a full match
/// (tag text, word boundary, and a date or version span) wins.
///
/// `start` is always an offset into the ORIGINAL `line`, never into a
/// re-sliced suffix: `scan_text` resumes a line by calling this again with
/// a later `start`, not by slicing `line` down to `&line[start..]`. That
/// distinction matters because the word-boundary check below reads
/// `bytes[i - 1]`, the byte immediately before a candidate match; slicing
/// would lose that left-context at the slice boundary and could let a
/// second trigger match mid-identifier right after the first one ends.
fn match_line_from<'a>(
    line: &'a str,
    start: usize,
    tags: &[String],
    firsts: &[u8],
) -> Option<(&'a str, String, usize)> {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = start;
    while i < n {
        // Fast reject first: this loop runs for every byte of every scanned
        // file, so nothing heavier than a first-byte comparison may sit on
        // the common path (v0.1 had this shape; losing it cost ~25% wall
        // time on real corpora).
        if !firsts.contains(&bytes[i].to_ascii_lowercase()) {
            i += 1;
            continue;
        }
        // word boundary: don't match inside identifiers. Independent of
        // which tag is tried, so check once per position.
        if i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
                i += 1;
                continue;
            }
        }
        // Try every tag at this position, not just the first textual match:
        // with tags = ["fixme", "fixme-by"], "fixme" matches textually on a
        // "fixme-by 2026-..." line but fails to extend, and only "fixme-by"
        // yields the full match.
        for tag in tags {
            let tag_bytes = tag.as_bytes();
            if i + tag_bytes.len() > n
                || !bytes[i..i + tag_bytes.len()].eq_ignore_ascii_case(tag_bytes)
            {
                continue;
            }
            let mut j = i + tag_bytes.len();
            if j < n && bytes[j] == b':' {
                j += 1;
            }
            let ws_start = j;
            while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j == ws_start {
                continue;
            }
            if let Some(end) = parse_date_span(bytes, j) {
                return Some((&line[j..end], clean_message(&line[end..]), end));
            }
            if let Some(end) = parse_version_span(bytes, j) {
                return Some((&line[j..end], clean_message(&line[end..]), end));
            }
        }
        // Advancing by one byte is safe: positions inside a just-rejected
        // token fail the word-boundary check above.
        i += 1;
    }
    None
}

/// Returns the end of the date-like token at `start`, or None when the tag
/// has no date. Requires exactly four leading year digits (a fifth digit
/// disqualifies the tag), then consumes the whole contiguous token (ASCII
/// alphanumerics, '-', '/', '.') so malformed dates like `2026/01/05`,
/// `2026-`, or `2026-09x` reach `date::deadline` intact and are reported
/// invalid; truncating to a valid prefix would silently postpone the
/// deadline. `trim_trailing_html_comment_dashes` then excludes an
/// immediately-following HTML comment closer's two hyphens from the token.
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
    while bytes
        .get(j)
        .is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'/' | b'.'))
    {
        j += 1;
    }
    Some(trim_trailing_html_comment_dashes(bytes, j))
}

/// Returns the end of the `comparator + version` token at `start`, or None
/// when there's no recognized comparator here, or it isn't immediately
/// (no space) followed by a version-like token: the byte after the
/// comparator, and its optional `v`/`V` prefix, must be an ASCII digit.
/// That guards prose like `todo-by > out.txt` or `todo-by <PATHS>` from
/// matching at all, the same way `parse_date_span` requires four leading
/// digits before committing to "this is a date".
///
/// Once a comparator commits, the version part is consumed whole (same
/// rationale as dates): `>=2.x` must reach `version::Constraint::parse`
/// intact and be reported invalid, not truncated to a valid-looking `>=2`.
/// `_` is included alongside `.`, `-`, `+` in the consumed charset for the
/// same reason: `>=2.0_rc.1` must reach the validator whole, not get cut
/// to a valid-looking `>=2.0`.
fn parse_version_span(bytes: &[u8], start: usize) -> Option<usize> {
    let cmp_len = COMPARATORS
        .iter()
        .find(|c| bytes[start..].starts_with(c.as_bytes()))?
        .len();
    let mut j = start + cmp_len;
    if bytes.get(j).is_some_and(|&b| matches!(b, b'v' | b'V')) {
        j += 1;
    }
    if !bytes.get(j).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    while bytes
        .get(j)
        .is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'_'))
    {
        j += 1;
    }
    Some(trim_trailing_html_comment_dashes(bytes, j))
}

/// Both `-` and `.` sit in the date and version charsets above, so a
/// trigger written just before an HTML comment closer (`<!-- todo-by
/// 2026-09-01--> ` or `<!-- todo-by >=2.0-->`, no space before `-->`) would
/// otherwise eat the closer's two hyphens into the span itself, producing
/// a bogus trailing `--` (an InvalidDate false positive, or a version
/// pre-release of `-`). If the just-consumed span ends with `--` and the
/// very next byte is `>`, back `end` off by 2 so those hyphens stay
/// outside the trigger; a genuine trailing `--` NOT followed by `>` (real
/// content, not a comment closer) is left untouched.
fn trim_trailing_html_comment_dashes(bytes: &[u8], end: usize) -> usize {
    if end >= 2 && &bytes[end - 2..end] == b"--" && bytes.get(end) == Some(&b'>') {
        end - 2
    } else {
        end
    }
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

pub fn scan_file(path: &Path, ctx: &ScanCtx, findings: &mut Vec<Finding>) -> std::io::Result<()> {
    let content = std::fs::read(path)?;
    scan_bytes(&path.display().to_string(), &content, ctx, findings);
    Ok(())
}

/// Scans raw bytes (file contents or stdin): skips binary content (NUL byte
/// in the first 8 KiB) and decodes the rest lossily, so invalid UTF-8 never
/// aborts a scan.
pub fn scan_bytes(file_label: &str, content: &[u8], ctx: &ScanCtx, findings: &mut Vec<Finding>) {
    if content.iter().take(8192).any(|&b| b == 0) {
        return;
    }
    let text = String::from_utf8_lossy(content);
    scan_text(file_label, &text, ctx, findings);
}

pub fn scan_text(file_label: &str, text: &str, ctx: &ScanCtx, findings: &mut Vec<Finding>) {
    let firsts = tag_firsts(ctx.tags);
    for (idx, line) in text.lines().enumerate() {
        // Every trigger on the line is reported, not just the first: resume
        // right after each match's span rather than stopping there. An
        // earlier trigger's message is the untouched rest of the line, so
        // it may include a later trigger's text verbatim; that's fine, the
        // later trigger still gets its own finding.
        let mut pos = 0;
        while let Some((written, message, end)) = match_line_from(line, pos, ctx.tags, &firsts) {
            if let Some(kind) = classify(written, ctx) {
                findings.push(Finding {
                    file: file_label.to_string(),
                    line: idx + 1,
                    kind,
                    message,
                });
            }
            pos = end;
        }
    }
}

/// Classifies a matched trigger span, or returns None when there's nothing
/// to report (a valid date outside today and the warn window). A date span
/// always starts with a digit (`parse_date_span` requires four leading
/// digits); a version span always starts with a comparator character, so
/// the leading byte alone tells the two apart.
fn classify(written: &str, ctx: &ScanCtx) -> Option<Kind> {
    // Non-empty by construction (both span parsers return >=1 byte spans);
    // indexing stays deliberate so a broken invariant panics loudly instead
    // of silently reclassifying an empty span.
    debug_assert!(!written.is_empty());
    if written.as_bytes()[0].is_ascii_digit() {
        match deadline(written) {
            None => Some(Kind::InvalidDate {
                written: written.to_string(),
            }),
            Some(due) if due <= ctx.today => Some(Kind::Overdue {
                written: written.to_string(),
                deadline: due,
            }),
            Some(due) => match ctx.warn_until {
                Some(w) if due <= w => Some(Kind::DueSoon {
                    written: written.to_string(),
                    deadline: due,
                }),
                _ => None,
            },
        }
    } else {
        // Warn-ahead never applies here: a future version isn't knowable at
        // scan time, so there's no "due soon" analog. The scanner can't
        // even tell Overdue from not-yet-reached without the current
        // version (which it doesn't have); that's why every valid
        // constraint becomes a VersionPending candidate for main.rs to
        // resolve, unconditionally.
        Some(match Constraint::parse(written) {
            Some(constraint) => Kind::VersionPending {
                written: written.to_string(),
                constraint,
            },
            None => Kind::InvalidTrigger {
                written: written.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn todo_by() -> Vec<String> {
        tags(&["todo-by"])
    }

    #[test]
    fn matches_common_comment_styles() {
        let todo_by_tags = todo_by();
        let cases = [
            (
                "// @todo-by 2999-12-31 remove feature flag after rollout",
                "2999-12-31",
                "remove feature flag after rollout",
            ),
            (
                "# todo-by 2999-09-01 drop legacy webhook handler",
                "2999-09-01",
                "drop legacy webhook handler",
            ),
            (
                "* @todo-by 2999-10-01 - Alies, remove workaround once upstream fix ships",
                "2999-10-01",
                "Alies, remove workaround once upstream fix ships",
            ),
            (
                "/** @todo-by 2999-04-20 - migrate to v2 API */",
                "2999-04-20",
                "migrate to v2 API",
            ),
            (
                "<!-- TODO-BY: 2999-01 remove IE11 polyfill -->",
                "2999-01",
                "remove IE11 polyfill",
            ),
            (
                "@todo-by 2999-08 - delete deprecated endpoint",
                "2999-08",
                "delete deprecated endpoint",
            ),
            (
                "-- todo-by 2999 drop unused index",
                "2999",
                "drop unused index",
            ),
            (
                "{# todo-by: 2999-05-05 remove banner after campaign ends #}",
                "2999-05-05",
                "remove banner after campaign ends",
            ),
        ];
        for (line, want_date, want_msg) in cases {
            let (date, msg) =
                match_line(line, &todo_by_tags).unwrap_or_else(|| panic!("no match: {line}"));
            assert_eq!(date, want_date, "date in {line:?}");
            assert_eq!(msg, want_msg, "message in {line:?}");
        }
    }

    #[test]
    fn ignores_lines_without_a_date() {
        let todo_by_tags = todo_by();
        assert_eq!(match_line("todo-by [PATHS]...", &todo_by_tags), None);
        assert_eq!(match_line("plain TODO: fix later", &todo_by_tags), None);
        assert_eq!(match_line("todo-by", &todo_by_tags), None);
        assert_eq!(match_line("todo-by 20261 five digits", &todo_by_tags), None);
        assert_eq!(
            match_line("autodo-by 2999-01-01 not a word boundary", &todo_by_tags),
            None
        );
    }

    #[test]
    fn impossible_dates_still_match_for_reporting() {
        let todo_by_tags = todo_by();
        // built at runtime so scanning this repo doesn't flag the fixture
        let line = format!("// todo-by {} bad", "2999-13-45");
        let (date, _) = match_line(&line, &todo_by_tags).unwrap();
        assert_eq!(date, "2999-13-45");
    }

    #[test]
    fn sloppy_dates_are_consumed_in_full_not_truncated() {
        let todo_by_tags = todo_by();
        let (date, msg) = match_line("// todo-by 2999-1-5 sloppy", &todo_by_tags).unwrap();
        assert_eq!(date, "2999-1-5");
        assert_eq!(msg, "sloppy");

        // Consumed whole so deadline() reports it invalid instead of the
        // tag silently meaning "2026", i.e. a later deadline.
        let line = format!("// todo-by {} overlong month", "2026-123");
        let (date, _) = match_line(&line, &todo_by_tags).unwrap();
        assert_eq!(date, "2026-123");
    }

    #[test]
    fn malformed_dates_are_consumed_whole_not_truncated() {
        let todo_by_tags = todo_by();
        // built at runtime so a future dogfood scan doesn't flag the fixtures
        for bad in [
            "2026/01/05",
            "2026.01.05",
            "2026-",
            "2026-09x",
            "2026-1-2-3",
        ] {
            let line = format!("// todo-by {bad} typo");
            let (date, msg) =
                match_line(&line, &todo_by_tags).unwrap_or_else(|| panic!("no match: {line}"));
            assert_eq!(date, bad, "date in {line:?}");
            assert_eq!(msg, "typo", "message in {line:?}");
            assert_eq!(
                crate::date::deadline(date),
                None,
                "{bad:?} must be reported invalid, not truncated to a later deadline"
            );
        }
    }

    #[test]
    fn alias_tags_all_match_alongside_todo_by() {
        let both = tags(&["todo-by", "fixme-by"]);
        let line = format!("// {} 2999-01-01 fix this", "fixme-by");
        let (date, msg) = match_line(&line, &both).unwrap();
        assert_eq!(date, "2999-01-01");
        assert_eq!(msg, "fix this");

        let line = format!("// {} 2999-01-01 fix that", "todo-by");
        let (date, msg) = match_line(&line, &both).unwrap();
        assert_eq!(date, "2999-01-01");
        assert_eq!(msg, "fix that");
    }

    #[test]
    fn tags_without_todo_by_do_not_match_todo_by_lines() {
        let fixme_only = tags(&["fixme-by"]);
        let line = format!("// {} 2999-01-01 not tracked", "todo-by");
        assert_eq!(match_line(&line, &fixme_only), None);
    }

    #[test]
    fn prefix_tag_does_not_shadow_longer_tag() {
        // "fixme" matches textually at the start of "fixme-by" but cannot
        // extend to a date; the longer tag must still win at that position.
        let both = tags(&["fixme", "fixme-by"]);
        let line = format!("// {} 2999-01-01 do it", "fixme-by");
        let (date, msg) = match_line(&line, &both).unwrap();
        assert_eq!(date, "2999-01-01");
        assert_eq!(msg, "do it");

        // And the shorter tag still works on its own lines.
        let line = format!("// {} 2999-01-01 short tag", "fixme");
        let (date, _) = match_line(&line, &both).unwrap();
        assert_eq!(date, "2999-01-01");
    }

    #[test]
    fn alias_tags_stay_word_boundary_safe() {
        let both = tags(&["todo-by", "fixme-by"]);
        let line = format!("// prefix-{} 2999-01-01 not a boundary", "fixme-by");
        assert_eq!(match_line(&line, &both), None);
    }

    fn ctx<'a>(today: Date, warn_until: Option<Date>, tags: &'a [String]) -> ScanCtx<'a> {
        ScanCtx {
            today,
            warn_until,
            tags,
        }
    }

    fn deadline_of(f: &Finding) -> Option<Date> {
        match &f.kind {
            Kind::Overdue { deadline, .. } | Kind::DueSoon { deadline, .. } => Some(*deadline),
            _ => panic!("expected an Overdue or DueSoon finding"),
        }
    }

    #[test]
    fn due_soon_within_warn_window_overdue_beyond_and_before() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let warn_until = Date::new(2999, 1, 15).unwrap();
        let c = ctx(today, Some(warn_until), &todo_by_tags);

        // within warn window: DueSoon
        let mut findings = Vec::new();
        scan_text("f", "// todo-by 2999-01-10 in window", &c, &mut findings);
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].kind, Kind::DueSoon { .. }));
        assert_eq!(deadline_of(&findings[0]), Date::new(2999, 1, 10));

        // beyond warn window: no finding
        let mut findings = Vec::new();
        scan_text(
            "f",
            "// todo-by 2999-02-01 beyond window",
            &c,
            &mut findings,
        );
        assert!(findings.is_empty());

        // already overdue: Overdue, not DueSoon
        let mut findings = Vec::new();
        scan_text(
            "f",
            "// todo-by 2998-12-31 already overdue",
            &c,
            &mut findings,
        );
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].kind, Kind::Overdue { .. }));
        assert_eq!(deadline_of(&findings[0]), Date::new(2998, 12, 31));
    }

    #[test]
    fn warn_until_none_disables_due_soon() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let mut findings = Vec::new();
        scan_text("f", "// todo-by 2999-01-10 near future", &c, &mut findings);
        assert!(findings.is_empty());
    }

    #[test]
    fn scan_bytes_skips_binary_and_decodes_invalid_utf8_lossily() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);

        // NUL in the first 8 KiB: treated as binary, no findings.
        let mut findings = Vec::new();
        let binary = b"\x00// todo-by 2998-01-01 hidden in binary";
        scan_bytes("bin", binary, &c, &mut findings);
        assert!(findings.is_empty());

        // Invalid UTF-8 elsewhere must not abort the scan of a valid tag.
        let mut findings = Vec::new();
        let mut content = b"\xff\xfe garbage\n".to_vec();
        content.extend_from_slice(b"// todo-by 2998-01-01 still found\n");
        scan_bytes("mixed", &content, &c, &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
    }

    #[test]
    fn scan_text_reports_one_based_line_numbers_and_label() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let text = "line one\n// todo-by 2998-01-01 overdue here\nline three\n// todo-by 2998-06-06 also overdue";
        let mut findings = Vec::new();
        scan_text("some/file.rs", text, &c, &mut findings);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].file, "some/file.rs");
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[1].file, "some/file.rs");
        assert_eq!(findings[1].line, 4);
    }

    // Fixtures below build the tag word and the trigger value in separate
    // format! arguments, same convention as the date fixtures above: the
    // repo's own dogfood scan reads this file as plain text too, and a
    // low-threshold constraint written directly next to the tag word would
    // fire for real now that this feature ships (the repo's version is
    // well past 0.1).

    #[test]
    fn version_triggers_match_across_comment_styles() {
        let todo_by_tags = todo_by();
        let ge = ">=2.0";
        let gt_pre = ">1.4.0-rc.1";
        let ge_v = ">=v3.0";
        let cases = [
            (
                format!("// @todo-by {ge} drop legacy endpoint after v2 ships"),
                ge,
                "drop legacy endpoint after v2 ships",
            ),
            (
                format!("# todo-by: {gt_pre} - remove polyfill"),
                gt_pre,
                "remove polyfill",
            ),
            (
                format!("<!-- todo-by {ge_v} delete migration shims -->"),
                ge_v,
                "delete migration shims",
            ),
        ];
        for (line, want_written, want_msg) in cases {
            let (written, msg) =
                match_line(&line, &todo_by_tags).unwrap_or_else(|| panic!("no match: {line}"));
            assert_eq!(written, want_written, "written in {line:?}");
            assert_eq!(msg, want_msg, "message in {line:?}");
        }
    }

    #[test]
    fn unsupported_comparators_become_invalid_trigger() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        for cmp in ["<", "<=", "=", "=="] {
            let written = format!("{cmp}2.0");
            let line = format!("// todo-by {written} old behavior");
            let mut findings = Vec::new();
            scan_text("f", &line, &c, &mut findings);
            assert_eq!(findings.len(), 1, "{line:?}");
            match &findings[0].kind {
                Kind::InvalidTrigger { written: w } => assert_eq!(w, &written, "{line:?}"),
                _ => panic!("expected InvalidTrigger for {line:?}"),
            }
        }
    }

    #[test]
    fn comparator_followed_by_space_does_not_match() {
        let todo_by_tags = todo_by();
        let line = format!("// todo-by {} 2.0 drop it", ">=");
        assert_eq!(match_line(&line, &todo_by_tags), None);
    }

    #[test]
    fn prose_after_tag_does_not_match_as_version() {
        let todo_by_tags = todo_by();
        assert_eq!(match_line("todo-by <PATHS>...", &todo_by_tags), None);
        assert_eq!(match_line("todo-by > out.txt", &todo_by_tags), None);
    }

    #[test]
    fn malformed_version_is_consumed_whole_not_truncated() {
        let todo_by_tags = todo_by();
        let bad = ">=2.x";
        let line = format!("// todo-by {bad} typo");
        let (written, msg) = match_line(&line, &todo_by_tags).unwrap();
        assert_eq!(written, bad);
        assert_eq!(msg, "typo");
        assert!(
            Constraint::parse(written).is_none(),
            "{bad:?} must be reported invalid, not truncated to a valid-looking prefix"
        );
    }

    #[test]
    fn version_candidates_ignore_the_warn_window() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let warn_until = Date::new(2999, 1, 15).unwrap();
        let written = ">=2.0";
        let line = format!("// todo-by {written} drop it");

        let mut findings = Vec::new();
        scan_text("f", &line, &ctx(today, None, &todo_by_tags), &mut findings);
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].kind, Kind::VersionPending { .. }));

        let mut findings = Vec::new();
        scan_text(
            "f",
            &line,
            &ctx(today, Some(warn_until), &todo_by_tags),
            &mut findings,
        );
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].kind, Kind::VersionPending { .. }));
    }

    #[test]
    fn a_line_with_a_date_trigger_then_a_version_trigger_reports_both() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let ge = ">=2.0";
        let line = format!("// todo-by 2998-01-01 overdue, todo-by {ge} drop legacy");
        let mut findings = Vec::new();
        scan_text("f", &line, &c, &mut findings);
        assert_eq!(findings.len(), 2, "{line:?}");
        assert!(matches!(findings[0].kind, Kind::Overdue { .. }));
        assert!(matches!(findings[1].kind, Kind::VersionPending { .. }));
    }

    #[test]
    fn version_trigger_does_not_shadow_a_later_overdue_date_on_the_same_line() {
        // Regression: scanning used to stop after a line's first trigger,
        // silently dropping everything after it. A version candidate
        // "shadowed" a later overdue date this way; both must be reported.
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let ge = ">=999.0"; // "unsatisfied" once main.rs resolves it; the scanner just emits VersionPending
        let line = format!("// todo-by {ge} not yet, todo-by 2998-01-01 also overdue");
        let mut findings = Vec::new();
        scan_text("f", &line, &c, &mut findings);
        assert_eq!(findings.len(), 2, "{line:?}");
        assert!(matches!(findings[0].kind, Kind::VersionPending { .. }));
        assert!(matches!(findings[1].kind, Kind::Overdue { .. }));
        // Acceptable: the earlier trigger's message is the untouched rest
        // of the line, so it includes the later trigger's text verbatim.
        assert_eq!(
            findings[0].message,
            "not yet, todo-by 2998-01-01 also overdue"
        );
    }

    #[test]
    fn underscore_in_version_span_is_consumed_whole_and_reported_invalid() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let bad = ">=2.0_rc.1";
        let line = format!("// todo-by {bad} typo");
        let mut findings = Vec::new();
        scan_text("f", &line, &c, &mut findings);
        assert_eq!(findings.len(), 1, "{line:?}");
        match &findings[0].kind {
            Kind::InvalidTrigger { written } => assert_eq!(written, bad),
            _ => panic!("expected InvalidTrigger for {line:?}"),
        }
    }

    #[test]
    fn html_comment_closer_does_not_corrupt_a_date_span() {
        // Before the backoff, the closer's two hyphens got swallowed into
        // the date span ("2998-09-01--"), which failed to parse and
        // misreported as InvalidDate instead of the real (overdue) date.
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let mut findings = Vec::new();
        scan_text("f", "<!-- todo-by 2998-09-01-->", &c, &mut findings);
        assert_eq!(findings.len(), 1);
        match &findings[0].kind {
            Kind::Overdue { written, deadline } => {
                assert_eq!(written, "2998-09-01");
                assert_eq!(*deadline, Date::new(2998, 9, 1).unwrap());
            }
            _ => panic!("expected Overdue, the closer must not corrupt the date span"),
        }
    }

    #[test]
    fn html_comment_closer_does_not_corrupt_a_version_span() {
        let todo_by_tags = todo_by();
        let today = Date::new(2999, 1, 1).unwrap();
        let c = ctx(today, None, &todo_by_tags);
        let mut findings = Vec::new();
        scan_text("f", "<!-- todo-by >=2.0-->", &c, &mut findings);
        assert_eq!(findings.len(), 1);
        match &findings[0].kind {
            Kind::VersionPending { written, .. } => assert_eq!(written, ">=2.0"),
            _ => panic!("expected VersionPending, the closer must not corrupt the version span"),
        }
    }

    #[test]
    fn trailing_double_hyphen_without_a_close_angle_is_still_consumed() {
        // Genuine content, not a comment closer (no '>' right after): must
        // stay part of the span and be rejected as malformed, not silently
        // trimmed the way a real "-->" closer is. Built at runtime: this
        // is an InvalidDate regardless of "today", so a literal here would
        // flag the repo's own dogfood scan.
        let todo_by_tags = todo_by();
        let bad = "2026-09-01--";
        let line = format!("// todo-by {bad} typo");
        let (date, msg) = match_line(&line, &todo_by_tags).unwrap();
        assert_eq!(date, bad);
        assert_eq!(msg, "typo");
        assert_eq!(crate::date::deadline(date), None);
    }

    #[test]
    fn trailing_double_hyphen_without_a_close_angle_is_still_consumed_in_version_span() {
        let todo_by_tags = todo_by();
        let bad = ">=2.0--";
        let line = format!("// todo-by {bad} typo");
        let (written, msg) = match_line(&line, &todo_by_tags).unwrap();
        assert_eq!(written, bad);
        assert_eq!(msg, "typo");
    }
}
