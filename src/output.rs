//! Renders findings to stdout in one of three formats: human-readable text,
//! GitHub Actions workflow commands, or JSON Lines.

use std::io::{self, Write};

use crate::date::Date;
use crate::scanner::{Finding, Kind};
use crate::version::{missing_v_marker, unsupported_comparator};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Text,
    Github,
    Json,
}

pub struct RenderOpts {
    pub format: Format,
    /// Honored by Text only; Github and Json never emit color codes.
    pub color: bool,
    pub today: Date,
    /// The resolved current version, when the scan produced at least one
    /// version candidate; None when it didn't (in which case no finding
    /// needs it: VersionReached can't exist without a resolved version).
    pub current_version: Option<String>,
}

const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// Renders `findings` to `w` per `opts.format`. Text also prints a
/// summary line to stderr when there's at least one finding; Json prints a
/// trailing summary record to `w` instead (always, even with zero
/// findings).
///
/// Every write is fallible and every error is returned: `w` is a pipe as
/// often as it is a terminal, and a reader that stops reading must end the
/// output, not the process. See `crate::stdout`.
pub fn render(w: &mut dyn Write, findings: &[Finding], opts: &RenderOpts) -> io::Result<()> {
    for f in findings {
        writeln!(w, "{}", render_finding(f, opts))?;
    }
    match opts.format {
        Format::Text => {
            if !findings.is_empty() {
                // Flushed before the summary reaches stderr, because under
                // `2>&1` both land in the same stream and a buffered
                // stdout would otherwise put the summary above the
                // findings it counts.
                w.flush()?;
                // Best effort, for the reason `note!` is: `2>&1 | head`
                // closes stderr the same way it closes stdout, and a
                // dropped summary line beats a panic on the way out. It
                // does not go through `note!` because it is a count, not
                // a diagnostic, and carries no `todo-by: ` prefix.
                let _ = writeln!(io::stderr(), "{}", summary_text(findings));
            }
        }
        Format::Github => {}
        Format::Json => writeln!(w, "{}", summary_json(findings))?,
    }
    Ok(())
}

fn render_finding(f: &Finding, opts: &RenderOpts) -> String {
    match opts.format {
        Format::Text => render_text(f, opts),
        Format::Github => render_github(f, opts.today, opts.current_version.as_deref()),
        Format::Json => render_json_finding(f, opts.today, opts.current_version.as_deref()),
    }
}

/// Days from `today` to `deadline`. Callers only pass pairs where this is
/// non-negative (Overdue: deadline <= today; DueSoon: deadline > today).
fn days_between(from: Date, to: Date) -> i64 {
    to.to_days_since_epoch() - from.to_days_since_epoch()
}

fn plural_days(n: i64) -> String {
    format!("{n} day{}", if n == 1 { "" } else { "s" })
}

/// Message for an InvalidTrigger finding. Three causes, each named with its
/// own way out: an unsupported comparator, a version written without the
/// mandatory `v` marker, or syntax no marker would rescue. The missing
/// marker is by far the likeliest of the three, since `2.0` is how versions
/// are written everywhere else, so it quotes the exact replacement string
/// rather than describing the rule.
fn invalid_trigger_message(written: &str) -> String {
    if let Some(cmp) = unsupported_comparator(written) {
        return format!("unsupported comparator {cmp:?} (use >=vX: fires once version reaches X)");
    }
    if let Some(fixed) = missing_v_marker(written) {
        // Without a comparator the token could equally have been meant as a
        // deadline (`2026.09.01` for `2026-09-01`), and that author is
        // helped by the other half of the rule, not by being told to add a
        // `v` to what they think is a date. A comparator rules that reading
        // out, so the hint stays short there.
        let alternative = if written.starts_with(['v', 'V'])
            || written.starts_with(|c: char| c.is_ascii_digit())
        {
            ", or dashes for a deadline"
        } else {
            ""
        };
        return format!(
            "version trigger {written:?} needs a lowercase v (write {fixed:?}{alternative})"
        );
    }
    format!("invalid version constraint {written:?}")
}

/// Message for an InvalidDate finding. A token of nothing but digits is a
/// year-only deadline (a lone `2026`), which 0.2 accepted as Dec 31 of
/// that year and 0.3 does not: a bare digit-leading token is a version
/// now, and a lone year can't be told apart from a one-component version.
/// Those tags get a migration hint naming both replacements instead of the
/// generic wording, since the generic one would read as a typo report for
/// something that used to be valid.
fn invalid_date_message(written: &str) -> String {
    if !written.is_empty() && written.bytes().all(|b| b.is_ascii_digit()) {
        return format!(
            "year-only deadline {written} is no longer supported \
             (write {written}-12 for the end of that year, or v{written} for a version)"
        );
    }
    format!("invalid date {written}")
}

/// Phrase for a fired issue trigger. The `#123` form gets the resolved URL
/// appended, since the tag itself doesn't say which repository it landed
/// on; a tag written as a URL already carries it and isn't repeated.
fn issue_closed_phrase(written: &str, state: &str, url: &str) -> String {
    if written.starts_with('#') {
        format!("{written} is {state} ({url})")
    } else {
        format!("{written} is {state}")
    }
}

/// Phrase for an issue trigger that could not be acted on. A syntax
/// complaint already quotes the reference, but an answer from the API
/// ("not found") does not, and with two triggers on one line the reader
/// would have no way to tell which one failed.
fn issue_error_phrase(written: &str, detail: &str) -> String {
    if detail.contains(written) {
        detail.to_string()
    } else {
        format!("{written}: {detail}")
    }
}

fn render_text(f: &Finding, opts: &RenderOpts) -> String {
    let (phrase, color) = match &f.kind {
        Kind::Overdue { written, .. } => (format!("overdue since {written}"), RED),
        Kind::InvalidDate { written } => (invalid_date_message(written), RED),
        Kind::DueSoon { deadline, .. } => {
            let n = days_between(opts.today, *deadline);
            (format!("due in {} ({deadline})", plural_days(n)), YELLOW)
        }
        Kind::VersionReached { written } => {
            let current = opts
                .current_version
                .as_deref()
                .expect("VersionReached always has a resolved current_version");
            (format!("version {current} reached ({written})"), RED)
        }
        Kind::InvalidTrigger { written } => (invalid_trigger_message(written), RED),
        Kind::IssueClosed {
            written,
            state,
            url,
        } => (issue_closed_phrase(written, state, url), RED),
        Kind::IssueError { written, detail } => (issue_error_phrase(written, detail), RED),
        Kind::VersionPending { .. } | Kind::IssuePending { .. } => {
            unreachable!("resolved or dropped in main")
        }
    };
    let phrase = if opts.color {
        format!("{color}{phrase}{RESET}")
    } else {
        phrase
    };
    format!("{}:{}: {}: {}", f.file, f.line, phrase, f.message)
}

fn render_github(f: &Finding, today: Date, current_version: Option<&str>) -> String {
    let (command, title) = match &f.kind {
        Kind::Overdue { written, .. } => ("error", format!("todo-by overdue since {written}")),
        Kind::InvalidDate { written } => (
            "error",
            format!("todo-by {}", invalid_date_message(written)),
        ),
        Kind::DueSoon { deadline, .. } => {
            let n = days_between(today, *deadline);
            (
                "warning",
                format!("todo-by due in {} ({deadline})", plural_days(n)),
            )
        }
        Kind::VersionReached { written } => {
            let current =
                current_version.expect("VersionReached always has a resolved current_version");
            (
                "error",
                format!("todo-by version {current} reached ({written})"),
            )
        }
        Kind::InvalidTrigger { written } => (
            "error",
            format!("todo-by {}", invalid_trigger_message(written)),
        ),
        Kind::IssueClosed {
            written,
            state,
            url,
        } => (
            "error",
            format!("todo-by {}", issue_closed_phrase(written, state, url)),
        ),
        Kind::IssueError { written, detail } => (
            "error",
            format!("todo-by {}", issue_error_phrase(written, detail)),
        ),
        Kind::VersionPending { .. } | Kind::IssuePending { .. } => {
            unreachable!("resolved or dropped in main")
        }
    };
    format!(
        "::{command} file={},line={},title={}::{}",
        gh_escape_property(&f.file),
        f.line,
        gh_escape_property(&title),
        gh_escape_data(&f.message)
    )
}

fn render_json_finding(f: &Finding, today: Date, current_version: Option<&str>) -> String {
    match &f.kind {
        Kind::Overdue { written, deadline } => {
            let days = days_between(*deadline, today);
            format!(
                "{{\"type\":\"finding\",\"kind\":\"overdue\",\"path\":\"{}\",\"line\":{},\
                 \"date\":\"{}\",\"deadline\":\"{deadline}\",\"days_overdue\":{days},\
                 \"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(written),
                escape_json(&f.message)
            )
        }
        Kind::DueSoon { written, deadline } => {
            let days = days_between(today, *deadline);
            format!(
                "{{\"type\":\"finding\",\"kind\":\"due-soon\",\"path\":\"{}\",\"line\":{},\
                 \"date\":\"{}\",\"deadline\":\"{deadline}\",\"days_until_due\":{days},\
                 \"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(written),
                escape_json(&f.message)
            )
        }
        Kind::InvalidDate { written } => format!(
            "{{\"type\":\"finding\",\"kind\":\"invalid-date\",\"path\":\"{}\",\"line\":{},\
             \"date\":\"{}\",\"deadline\":null,\"message\":\"{}\"}}",
            escape_json(&f.file),
            f.line,
            escape_json(written),
            escape_json(&f.message)
        ),
        Kind::VersionReached { written } => {
            let current =
                current_version.expect("VersionReached always has a resolved current_version");
            format!(
                "{{\"type\":\"finding\",\"kind\":\"version-reached\",\"path\":\"{}\",\"line\":{},\
                 \"constraint\":\"{}\",\"current_version\":\"{}\",\"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(written),
                escape_json(current),
                escape_json(&f.message)
            )
        }
        Kind::InvalidTrigger { written } => {
            format!(
                "{{\"type\":\"finding\",\"kind\":\"invalid-trigger\",\"path\":\"{}\",\"line\":{},\
                 \"constraint\":\"{}\",\"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(written),
                escape_json(&f.message)
            )
        }
        Kind::IssueClosed {
            written,
            state,
            url,
        } => format!(
            "{{\"type\":\"finding\",\"kind\":\"issue-closed\",\"path\":\"{}\",\"line\":{},\
             \"reference\":\"{}\",\"state\":\"{state}\",\"url\":\"{}\",\"message\":\"{}\"}}",
            escape_json(&f.file),
            f.line,
            escape_json(written),
            escape_json(url),
            escape_json(&f.message)
        ),
        Kind::IssueError { written, detail } => format!(
            "{{\"type\":\"finding\",\"kind\":\"issue-error\",\"path\":\"{}\",\"line\":{},\
             \"reference\":\"{}\",\"detail\":\"{}\",\"message\":\"{}\"}}",
            escape_json(&f.file),
            f.line,
            escape_json(written),
            escape_json(detail),
            escape_json(&f.message)
        ),
        Kind::VersionPending { .. } | Kind::IssuePending { .. } => {
            unreachable!("resolved or dropped in main")
        }
    }
}

/// Splits findings into error-level (everything except DueSoon) and
/// warning-level (DueSoon) counts; also drives the exit code in main. The
/// pending kinds never reach here: main.rs resolves each one or drops it
/// before rendering.
pub fn counts(findings: &[Finding]) -> (usize, usize) {
    let errors = findings
        .iter()
        .filter(|f| {
            matches!(
                f.kind,
                Kind::Overdue { .. }
                    | Kind::InvalidDate { .. }
                    | Kind::VersionReached { .. }
                    | Kind::InvalidTrigger { .. }
                    | Kind::IssueClosed { .. }
                    | Kind::IssueError { .. }
            )
        })
        .count();
    let warnings = findings
        .iter()
        .filter(|f| matches!(f.kind, Kind::DueSoon { .. }))
        .count();
    (errors, warnings)
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn summary_text(findings: &[Finding]) -> String {
    let (errors, warnings) = counts(findings);
    match (errors, warnings) {
        (0, w) => plural(w, "warning"),
        (e, 0) => plural(e, "finding"),
        (e, w) => format!("{}, {}", plural(e, "finding"), plural(w, "warning")),
    }
}

fn summary_json(findings: &[Finding]) -> String {
    let (errors, warnings) = counts(findings);
    format!("{{\"type\":\"summary\",\"findings\":{errors},\"warnings\":{warnings}}}")
}

// Workflow-command escaping:
// https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands

fn gh_escape_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn gh_escape_property(s: &str) -> String {
    gh_escape_data(s).replace(':', "%3A").replace(',', "%2C")
}

/// Escapes a string for embedding in a JSON string literal.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{Finding, Kind};

    fn date(s: &str) -> Date {
        Date::parse_full(s).unwrap()
    }

    fn overdue() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 12,
            kind: Kind::Overdue {
                written: "2000-01-01".to_string(),
                deadline: date("2000-01-01"),
            },
            message: "remove workaround".to_string(),
        }
    }

    fn due_soon() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 20,
            kind: Kind::DueSoon {
                written: "2000-01-10".to_string(),
                deadline: date("2000-01-10"),
            },
            message: "drop feature flag".to_string(),
        }
    }

    fn invalid() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 30,
            kind: Kind::InvalidDate {
                written: "2000-02-30".to_string(),
            },
            message: "typo'd date".to_string(),
        }
    }

    fn version_reached() -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line: 30,
            kind: Kind::VersionReached {
                written: ">=v2.0".to_string(),
            },
            message: "drop legacy endpoint".to_string(),
        }
    }

    fn invalid_version_syntax() -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line: 31,
            kind: Kind::InvalidTrigger {
                written: ">=v2.x".to_string(),
            },
            message: "remove thing".to_string(),
        }
    }

    fn unsupported_comparator_finding() -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line: 32,
            kind: Kind::InvalidTrigger {
                written: "<v1.0".to_string(),
            },
            message: "old behavior".to_string(),
        }
    }

    fn missing_marker(line: usize, written: &str, message: &str) -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line,
            kind: Kind::InvalidTrigger {
                written: written.to_string(),
            },
            message: message.to_string(),
        }
    }

    fn opts(format: Format, color: bool) -> RenderOpts {
        RenderOpts {
            format,
            color,
            today: date("2000-01-05"),
            current_version: None,
        }
    }

    /// Like `opts`, but with a resolved current version, for the
    /// VersionReached render tests: main.rs only ever supplies a
    /// current_version when it actually resolved one.
    fn opts_with_version(format: Format, current_version: &str) -> RenderOpts {
        RenderOpts {
            format,
            color: false,
            today: date("2000-01-05"),
            current_version: Some(current_version.to_string()),
        }
    }

    #[test]
    fn text_overdue_line() {
        let o = opts(Format::Text, false);
        assert_eq!(
            render_finding(&overdue(), &o),
            "src/lib.rs:12: overdue since 2000-01-01: remove workaround"
        );
    }

    #[test]
    fn text_invalid_date_line() {
        let o = opts(Format::Text, false);
        assert_eq!(
            render_finding(&invalid(), &o),
            "src/lib.rs:30: invalid date 2000-02-30: typo'd date"
        );
    }

    #[test]
    fn text_due_soon_line_singular_and_plural() {
        let o = opts(Format::Text, false);
        assert_eq!(
            render_finding(&due_soon(), &o),
            "src/lib.rs:20: due in 5 days (2000-01-10): drop feature flag"
        );

        let mut f = due_soon();
        f.kind = Kind::DueSoon {
            written: "2000-01-06".to_string(),
            deadline: date("2000-01-06"),
        };
        assert_eq!(
            render_finding(&f, &o),
            "src/lib.rs:20: due in 1 day (2000-01-06): drop feature flag"
        );
    }

    #[test]
    fn text_version_reached_line() {
        let o = opts_with_version(Format::Text, "2.1.0");
        assert_eq!(
            render_finding(&version_reached(), &o),
            "src/api.rs:30: version 2.1.0 reached (>=v2.0): drop legacy endpoint"
        );
    }

    #[test]
    fn text_invalid_trigger_lines() {
        let o = opts(Format::Text, false);
        assert_eq!(
            render_finding(&invalid_version_syntax(), &o),
            "src/api.rs:31: invalid version constraint \">=v2.x\": remove thing"
        );
        assert_eq!(
            render_finding(&unsupported_comparator_finding(), &o),
            "src/api.rs:32: unsupported comparator \"<\" (use >=vX: fires once version reaches X): old behavior"
        );
    }

    #[test]
    fn text_missing_marker_line_quotes_the_exact_replacement() {
        let o = opts(Format::Text, false);
        // No comparator: could have been meant as a deadline, so the hint
        // names that way out too.
        assert_eq!(
            render_finding(&missing_marker(33, "2026.09.01", "drop the adapter"), &o),
            "src/api.rs:33: version trigger \"2026.09.01\" needs a lowercase v \
             (write \"v2026.09.01\", or dashes for a deadline): drop the adapter"
        );
        // A comparator rules out the date reading, so the hint stays short.
        assert_eq!(
            render_finding(&missing_marker(34, ">=2.0", "drop legacy"), &o),
            "src/api.rs:34: version trigger \">=2.0\" needs a lowercase v \
             (write \">=v2.0\"): drop legacy"
        );
        // An uppercase V failed on case alone; same remedy shape.
        assert_eq!(
            render_finding(&missing_marker(35, "V2.0", "casing"), &o),
            "src/api.rs:35: version trigger \"V2.0\" needs a lowercase v \
             (write \"v2.0\", or dashes for a deadline): casing"
        );
    }

    #[test]
    fn text_color_wraps_kind_phrase_only() {
        let o = opts(Format::Text, true);
        assert_eq!(
            render_finding(&overdue(), &o),
            "src/lib.rs:12: \x1b[31moverdue since 2000-01-01\x1b[0m: remove workaround"
        );
        assert_eq!(
            render_finding(&invalid(), &o),
            "src/lib.rs:30: \x1b[31minvalid date 2000-02-30\x1b[0m: typo'd date"
        );
        assert_eq!(
            render_finding(&due_soon(), &o),
            "src/lib.rs:20: \x1b[33mdue in 5 days (2000-01-10)\x1b[0m: drop feature flag"
        );
    }

    #[test]
    fn text_no_color_has_no_escape_codes() {
        let o = opts(Format::Text, false);
        assert!(!render_finding(&overdue(), &o).contains('\x1b'));
    }

    #[test]
    fn summary_text_variants() {
        assert_eq!(summary_text(&[overdue()]), "1 finding");
        assert_eq!(summary_text(&[overdue(), invalid()]), "2 findings");
        assert_eq!(summary_text(&[due_soon()]), "1 warning");
        assert_eq!(summary_text(&[due_soon(), due_soon()]), "2 warnings");
        assert_eq!(
            summary_text(&[overdue(), due_soon()]),
            "1 finding, 1 warning"
        );
        assert_eq!(
            summary_text(&[overdue(), invalid(), due_soon(), due_soon()]),
            "2 findings, 2 warnings"
        );
        assert_eq!(
            summary_text(&[version_reached(), invalid_version_syntax()]),
            "2 findings"
        );
    }

    #[test]
    fn github_overdue_and_invalid_emit_error() {
        let line = render_finding(&overdue(), &opts(Format::Github, false));
        assert_eq!(
            line,
            "::error file=src/lib.rs,line=12,title=todo-by overdue since 2000-01-01::remove workaround"
        );
        let line = render_finding(&invalid(), &opts(Format::Github, false));
        assert_eq!(
            line,
            "::error file=src/lib.rs,line=30,title=todo-by invalid date 2000-02-30::typo'd date"
        );
    }

    #[test]
    fn github_due_soon_emits_warning() {
        let line = render_finding(&due_soon(), &opts(Format::Github, false));
        assert_eq!(
            line,
            "::warning file=src/lib.rs,line=20,title=todo-by due in 5 days (2000-01-10)::drop feature flag"
        );
    }

    #[test]
    fn github_version_reached_and_invalid_trigger_emit_error() {
        let line = render_finding(
            &version_reached(),
            &opts_with_version(Format::Github, "2.1.0"),
        );
        assert_eq!(
            line,
            "::error file=src/api.rs,line=30,title=todo-by version 2.1.0 reached (>=v2.0)::drop legacy endpoint"
        );
        let line = render_finding(
            &unsupported_comparator_finding(),
            &opts(Format::Github, false),
        );
        assert_eq!(
            // The hint's own ':' goes through gh_escape_property like any
            // other title content, becoming %3A.
            line,
            "::error file=src/api.rs,line=32,title=todo-by unsupported comparator \"<\" (use >=vX%3A fires once version reaches X)::old behavior"
        );
    }

    #[test]
    fn json_overdue_shape() {
        let line = render_finding(&overdue(), &opts(Format::Json, false));
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"overdue\",\"path\":\"src/lib.rs\",\"line\":12,\"date\":\"2000-01-01\",\"deadline\":\"2000-01-01\",\"days_overdue\":4,\"message\":\"remove workaround\"}"
        );
    }

    #[test]
    fn json_due_soon_shape() {
        let line = render_finding(&due_soon(), &opts(Format::Json, false));
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"due-soon\",\"path\":\"src/lib.rs\",\"line\":20,\"date\":\"2000-01-10\",\"deadline\":\"2000-01-10\",\"days_until_due\":5,\"message\":\"drop feature flag\"}"
        );
    }

    #[test]
    fn json_invalid_date_shape() {
        let line = render_finding(&invalid(), &opts(Format::Json, false));
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"invalid-date\",\"path\":\"src/lib.rs\",\"line\":30,\"date\":\"2000-02-30\",\"deadline\":null,\"message\":\"typo'd date\"}"
        );
    }

    #[test]
    fn json_version_reached_shape() {
        let line = render_finding(
            &version_reached(),
            &opts_with_version(Format::Json, "2.1.0"),
        );
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"version-reached\",\"path\":\"src/api.rs\",\"line\":30,\"constraint\":\">=v2.0\",\"current_version\":\"2.1.0\",\"message\":\"drop legacy endpoint\"}"
        );
    }

    #[test]
    fn json_invalid_trigger_shape_has_no_current_version() {
        let line = render_finding(&invalid_version_syntax(), &opts(Format::Json, false));
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"invalid-trigger\",\"path\":\"src/api.rs\",\"line\":31,\"constraint\":\">=v2.x\",\"message\":\"remove thing\"}"
        );
    }

    #[test]
    fn json_summary_counts_errors_and_warnings_separately() {
        assert_eq!(
            summary_json(&[overdue(), invalid(), due_soon()]),
            "{\"type\":\"summary\",\"findings\":2,\"warnings\":1}"
        );
        assert_eq!(
            summary_json(&[
                version_reached(),
                unsupported_comparator_finding(),
                due_soon()
            ]),
            "{\"type\":\"summary\",\"findings\":2,\"warnings\":1}"
        );
        assert_eq!(
            summary_json(&[]),
            "{\"type\":\"summary\",\"findings\":0,\"warnings\":0}"
        );
    }

    #[test]
    fn escape_json_handles_control_chars_and_passthrough() {
        assert_eq!(escape_json("a\"b"), "a\\\"b");
        assert_eq!(escape_json("a\\b"), "a\\\\b");
        assert_eq!(escape_json("a\nb"), "a\\nb");
        assert_eq!(escape_json("a\u{1}b"), "a\\u0001b");
        assert_eq!(escape_json("café"), "café");
    }

    #[test]
    fn github_escaping_neutralizes_command_syntax() {
        assert_eq!(gh_escape_property("a,b:c.txt"), "a%2Cb%3Ac.txt");
        assert_eq!(gh_escape_property("50%,done"), "50%25%2Cdone");
        assert_eq!(gh_escape_data("line1\nline2, 50%"), "line1%0Aline2, 50%25");
        assert_eq!(gh_escape_data("cr\rlf"), "cr%0Dlf");
    }

    fn issue_finding(kind: Kind) -> Finding {
        Finding {
            file: "src/a.rs".to_string(),
            line: 4,
            kind,
            message: "drop the shim".to_string(),
        }
    }

    fn closed(written: &str, state: &'static str) -> Kind {
        Kind::IssueClosed {
            written: written.to_string(),
            state,
            url: "https://github.com/o/r/issues/9".to_string(),
        }
    }

    #[test]
    fn issue_findings_render_in_every_format() {
        let opts = RenderOpts {
            format: Format::Text,
            color: false,
            today: Date::new(2026, 8, 26).unwrap(),
            current_version: None,
        };
        let f = issue_finding(closed("#9", "closed"));
        assert_eq!(
            render_finding(&f, &opts),
            "src/a.rs:4: #9 is closed (https://github.com/o/r/issues/9): drop the shim"
        );
        let gh = render_github(&f, opts.today, None);
        assert!(gh.starts_with("::error file=src/a.rs,line=4,"), "{gh}");
        assert!(gh.contains("#9 is closed"), "{gh}");
        let json = render_json_finding(&f, opts.today, None);
        assert!(json.contains(r#""kind":"issue-closed""#), "{json}");
        assert!(json.contains(r##""reference":"#9""##), "{json}");
        assert!(json.contains(r#""state":"closed""#), "{json}");
        assert!(
            json.contains(r#""url":"https://github.com/o/r/issues/9""#),
            "{json}"
        );
    }

    #[test]
    fn a_url_reference_does_not_repeat_itself() {
        let written = "https://github.com/o/r/issues/9";
        assert_eq!(
            issue_closed_phrase(written, "merged", "https://github.com/o/r/issues/9"),
            "https://github.com/o/r/issues/9 is merged"
        );
        assert_eq!(
            issue_closed_phrase("#9", "merged", "https://github.com/o/r/issues/9"),
            "#9 is merged (https://github.com/o/r/issues/9)"
        );
    }

    #[test]
    fn issue_errors_render_their_own_detail() {
        let f = issue_finding(Kind::IssueError {
            written: "#12x".to_string(),
            detail: "invalid issue reference".to_string(),
        });
        let opts = RenderOpts {
            format: Format::Text,
            color: false,
            today: Date::new(2026, 8, 26).unwrap(),
            current_version: None,
        };
        // A bare answer from the API says nothing about which reference it
        // belongs to, so the reference is prefixed.
        assert_eq!(
            render_finding(&f, &opts),
            "src/a.rs:4: #12x: invalid issue reference: drop the shim"
        );
        // A syntax complaint already quotes it, so it is not repeated.
        assert_eq!(
            issue_error_phrase("#12x", "invalid issue reference \"#12x\" (write #123)"),
            "invalid issue reference \"#12x\" (write #123)"
        );
        let json = render_json_finding(&f, opts.today, None);
        assert!(json.contains(r#""kind":"issue-error""#), "{json}");
        assert!(
            json.contains(r#""detail":"invalid issue reference""#),
            "{json}"
        );
    }

    /// A writer that accepts `limit` bytes and then fails the way a pipe
    /// whose reader went away does. Byte-counted rather than
    /// line-counted, and unrelated to any real pipe buffer, so the test
    /// does not depend on how much a kernel happens to buffer.
    struct ClosedAfter {
        limit: usize,
        written: Vec<u8>,
        kind: io::ErrorKind,
        flushes: usize,
    }

    impl ClosedAfter {
        fn new(limit: usize, kind: io::ErrorKind) -> Self {
            ClosedAfter {
                limit,
                written: Vec::new(),
                kind,
                flushes: 0,
            }
        }

        /// Accepts everything, so the same writer doubles as a plain sink
        /// that counts flushes.
        fn open() -> Self {
            ClosedAfter::new(usize::MAX, io::ErrorKind::BrokenPipe)
        }
    }

    impl Write for ClosedAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.written.len() >= self.limit {
                return Err(io::Error::new(self.kind, "reader is gone"));
            }
            let take = buf.len().min(self.limit - self.written.len());
            self.written.extend_from_slice(&buf[..take]);
            Ok(take)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn render_writes_every_finding_then_the_summary() {
        let mut buf = Vec::new();
        render(
            &mut buf,
            &[overdue(), due_soon()],
            &opts(Format::Json, false),
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert_eq!(out.lines().count(), 3, "{out}");
        assert!(
            out.lines().last().unwrap().contains(r#""type":"summary""#),
            "{out}"
        );
    }

    #[test]
    fn text_flushes_stdout_before_the_stderr_summary() {
        // Under `2>&1` the two streams merge, so a buffered stdout that
        // was not flushed here would print the count above the findings
        // it counts.
        let mut w = ClosedAfter::open();
        render(&mut w, &[overdue()], &opts(Format::Text, false)).unwrap();
        assert_eq!(w.flushes, 1);

        // Nothing to summarize, nothing to order: no flush is forced.
        let mut w = ClosedAfter::open();
        render(&mut w, &[], &opts(Format::Text, false)).unwrap();
        assert_eq!(w.flushes, 0);
    }

    #[test]
    fn render_reports_a_closed_reader_instead_of_panicking() {
        // The bug this guards: `todo-by | head -1` used to panic inside
        // `println!` and exit 101, a code the CLI never documented.
        let mut w = ClosedAfter::new(10, io::ErrorKind::BrokenPipe);
        let findings = vec![overdue(), due_soon(), invalid()];
        let err = render(&mut w, &findings, &opts(Format::Text, false)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        // Stops at the break rather than carrying on into a stream nobody
        // is reading.
        assert_eq!(w.written.len(), 10);
    }

    #[test]
    fn render_reports_a_real_write_failure_too() {
        // Same path, different verdict in main: this one is an error, and
        // telling them apart is `stdout::write`'s job, not `render`'s.
        let mut w = ClosedAfter::new(0, io::ErrorKind::StorageFull);
        let err = render(&mut w, &[overdue()], &opts(Format::Json, false)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
    }

    #[test]
    fn json_summary_is_written_even_with_no_findings() {
        let mut buf = Vec::new();
        render(&mut buf, &[], &opts(Format::Json, false)).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap().trim(),
            r#"{"type":"summary","findings":0,"warnings":0}"#
        );
    }

    #[test]
    fn github_format_writes_no_summary() {
        let mut buf = Vec::new();
        render(&mut buf, &[overdue()], &opts(Format::Github, false)).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert_eq!(out.lines().count(), 1);
        assert!(out.starts_with("::error "), "{out}");
    }

    #[test]
    fn issue_findings_count_as_errors() {
        let findings = vec![
            issue_finding(closed("#9", "merged")),
            issue_finding(Kind::IssueError {
                written: "#1".to_string(),
                detail: "no access".to_string(),
            }),
        ];
        assert_eq!(counts(&findings), (2, 0));
    }
}
