//! Renders findings to stdout in one of three formats: human-readable text,
//! GitHub Actions workflow commands, or JSON Lines.

use crate::date::Date;
use crate::scanner::{Finding, Kind, Trigger};
use crate::version::unsupported_comparator;

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
}

const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// Renders `findings` to stdout per `opts.format`. Text also prints a
/// summary line to stderr when there's at least one finding; Json prints a
/// trailing summary record to stdout instead (always, even with zero
/// findings).
pub fn render(findings: &[Finding], opts: &RenderOpts) {
    for f in findings {
        println!("{}", render_finding(f, opts));
    }
    match opts.format {
        Format::Text => {
            if !findings.is_empty() {
                eprintln!("{}", summary_text(findings));
            }
        }
        Format::Github => {}
        Format::Json => println!("{}", summary_json(findings)),
    }
}

fn render_finding(f: &Finding, opts: &RenderOpts) -> String {
    match opts.format {
        Format::Text => render_text(f, opts),
        Format::Github => render_github(f, opts.today),
        Format::Json => render_json_finding(f, opts.today),
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

/// The date-as-written for an Overdue/DueSoon/InvalidDate finding. Panics on
/// a Version trigger: those three kinds only ever pair with Trigger::Date,
/// enforced by `scanner::classify`.
fn date_written(f: &Finding) -> &str {
    match &f.trigger {
        Trigger::Date { written, .. } => written,
        Trigger::Version { .. } => unreachable!("date-kind finding without a Trigger::Date"),
    }
}

fn date_deadline(f: &Finding) -> Date {
    match &f.trigger {
        Trigger::Date { deadline, .. } => deadline.expect("Overdue/DueSoon always have a deadline"),
        Trigger::Version { .. } => unreachable!("date-kind finding without a Trigger::Date"),
    }
}

/// The constraint as written, and the current version that satisfied it,
/// for a VersionReached finding.
fn version_reached_parts(f: &Finding) -> (&str, &str) {
    match &f.trigger {
        Trigger::Version {
            written,
            current_version,
            ..
        } => (
            written,
            current_version
                .as_deref()
                .expect("VersionReached always carries current_version"),
        ),
        Trigger::Date { .. } => unreachable!("VersionReached finding without a Trigger::Version"),
    }
}

/// Message for an InvalidTrigger finding: names the unsupported comparator
/// with a remedy hint when that's the cause, otherwise reports the
/// constraint as generically invalid (bad version syntax).
fn invalid_trigger_message(f: &Finding) -> String {
    match &f.trigger {
        Trigger::Version { written, .. } => match unsupported_comparator(written) {
            Some(cmp) => {
                format!("unsupported comparator {cmp:?} (use >=X: fires once version reaches X)")
            }
            None => format!("invalid version constraint {written:?}"),
        },
        Trigger::Date { .. } => unreachable!("InvalidTrigger finding without a Trigger::Version"),
    }
}

fn render_text(f: &Finding, opts: &RenderOpts) -> String {
    let (phrase, color) = match f.kind {
        Kind::Overdue => (format!("overdue since {}", date_written(f)), RED),
        Kind::InvalidDate => (format!("invalid date {}", date_written(f)), RED),
        Kind::DueSoon => {
            let n = days_between(opts.today, date_deadline(f));
            (
                format!("due in {} ({})", plural_days(n), date_deadline(f)),
                YELLOW,
            )
        }
        Kind::VersionReached => {
            let (written, current) = version_reached_parts(f);
            (format!("version {current} reached ({written})"), RED)
        }
        Kind::InvalidTrigger => (invalid_trigger_message(f), RED),
        Kind::VersionPending => unreachable!("resolved into VersionReached or dropped in main"),
    };
    let phrase = if opts.color {
        format!("{color}{phrase}{RESET}")
    } else {
        phrase
    };
    format!("{}:{}: {}: {}", f.file, f.line, phrase, f.message)
}

fn render_github(f: &Finding, today: Date) -> String {
    let (command, title) = match f.kind {
        Kind::Overdue => (
            "error",
            format!("todo-by overdue since {}", date_written(f)),
        ),
        Kind::InvalidDate => ("error", format!("todo-by invalid date {}", date_written(f))),
        Kind::DueSoon => {
            let n = days_between(today, date_deadline(f));
            (
                "warning",
                format!("todo-by due in {} ({})", plural_days(n), date_deadline(f)),
            )
        }
        Kind::VersionReached => {
            let (written, current) = version_reached_parts(f);
            (
                "error",
                format!("todo-by version {current} reached ({written})"),
            )
        }
        Kind::InvalidTrigger => ("error", format!("todo-by {}", invalid_trigger_message(f))),
        Kind::VersionPending => unreachable!("resolved into VersionReached or dropped in main"),
    };
    format!(
        "::{command} file={},line={},title={}::{}",
        gh_escape_property(&f.file),
        f.line,
        gh_escape_property(&title),
        gh_escape_data(&f.message)
    )
}

fn render_json_finding(f: &Finding, today: Date) -> String {
    match f.kind {
        Kind::Overdue => {
            let deadline = date_deadline(f);
            let days = days_between(deadline, today);
            format!(
                "{{\"type\":\"finding\",\"kind\":\"overdue\",\"path\":\"{}\",\"line\":{},\
                 \"date\":\"{}\",\"deadline\":\"{deadline}\",\"days_overdue\":{days},\
                 \"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(date_written(f)),
                escape_json(&f.message)
            )
        }
        Kind::DueSoon => {
            let deadline = date_deadline(f);
            let days = days_between(today, deadline);
            format!(
                "{{\"type\":\"finding\",\"kind\":\"due-soon\",\"path\":\"{}\",\"line\":{},\
                 \"date\":\"{}\",\"deadline\":\"{deadline}\",\"days_until_due\":{days},\
                 \"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(date_written(f)),
                escape_json(&f.message)
            )
        }
        Kind::InvalidDate => format!(
            "{{\"type\":\"finding\",\"kind\":\"invalid-date\",\"path\":\"{}\",\"line\":{},\
             \"date\":\"{}\",\"deadline\":null,\"message\":\"{}\"}}",
            escape_json(&f.file),
            f.line,
            escape_json(date_written(f)),
            escape_json(&f.message)
        ),
        Kind::VersionReached => {
            let (written, current) = version_reached_parts(f);
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
        Kind::InvalidTrigger => {
            let written = match &f.trigger {
                Trigger::Version { written, .. } => written,
                Trigger::Date { .. } => unreachable!("InvalidTrigger without a Trigger::Version"),
            };
            format!(
                "{{\"type\":\"finding\",\"kind\":\"invalid-trigger\",\"path\":\"{}\",\"line\":{},\
                 \"constraint\":\"{}\",\"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(written),
                escape_json(&f.message)
            )
        }
        Kind::VersionPending => unreachable!("resolved into VersionReached or dropped in main"),
    }
}

/// Splits findings into error-level (Overdue, InvalidDate, VersionReached,
/// InvalidTrigger) and warning-level (DueSoon) counts; also drives the exit
/// code in main. VersionPending never reaches here: main.rs resolves every
/// such finding into VersionReached or drops it before rendering.
pub fn counts(findings: &[Finding]) -> (usize, usize) {
    let errors = findings
        .iter()
        .filter(|f| {
            matches!(
                f.kind,
                Kind::Overdue | Kind::InvalidDate | Kind::VersionReached | Kind::InvalidTrigger
            )
        })
        .count();
    let warnings = findings
        .iter()
        .filter(|f| matches!(f.kind, Kind::DueSoon))
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
    use crate::scanner::{Finding, Kind, Trigger};
    use crate::version::Constraint;

    fn date(s: &str) -> Date {
        Date::parse_full(s).unwrap()
    }

    fn overdue() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 12,
            kind: Kind::Overdue,
            trigger: Trigger::Date {
                written: "2000-01-01".to_string(),
                deadline: Some(date("2000-01-01")),
            },
            message: "remove workaround".to_string(),
        }
    }

    fn due_soon() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 20,
            kind: Kind::DueSoon,
            trigger: Trigger::Date {
                written: "2000-01-10".to_string(),
                deadline: Some(date("2000-01-10")),
            },
            message: "drop feature flag".to_string(),
        }
    }

    fn invalid() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 30,
            kind: Kind::InvalidDate,
            trigger: Trigger::Date {
                written: "2000-02-30".to_string(),
                deadline: None,
            },
            message: "typo'd date".to_string(),
        }
    }

    fn version_reached() -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line: 30,
            kind: Kind::VersionReached,
            trigger: Trigger::Version {
                written: ">=2.0".to_string(),
                constraint: Constraint::parse(">=2.0"),
                current_version: Some("2.1.0".to_string()),
            },
            message: "drop legacy endpoint".to_string(),
        }
    }

    fn invalid_version_syntax() -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line: 31,
            kind: Kind::InvalidTrigger,
            trigger: Trigger::Version {
                written: ">=2.x".to_string(),
                constraint: None,
                current_version: None,
            },
            message: "remove thing".to_string(),
        }
    }

    fn unsupported_comparator_finding() -> Finding {
        Finding {
            file: "src/api.rs".to_string(),
            line: 32,
            kind: Kind::InvalidTrigger,
            trigger: Trigger::Version {
                written: "<1.0".to_string(),
                constraint: None,
                current_version: None,
            },
            message: "old behavior".to_string(),
        }
    }

    fn opts(format: Format, color: bool) -> RenderOpts {
        RenderOpts {
            format,
            color,
            today: date("2000-01-05"),
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
        f.trigger = Trigger::Date {
            written: "2000-01-06".to_string(),
            deadline: Some(date("2000-01-06")),
        };
        assert_eq!(
            render_finding(&f, &o),
            "src/lib.rs:20: due in 1 day (2000-01-06): drop feature flag"
        );
    }

    #[test]
    fn text_version_reached_line() {
        let o = opts(Format::Text, false);
        assert_eq!(
            render_finding(&version_reached(), &o),
            "src/api.rs:30: version 2.1.0 reached (>=2.0): drop legacy endpoint"
        );
    }

    #[test]
    fn text_invalid_trigger_lines() {
        let o = opts(Format::Text, false);
        assert_eq!(
            render_finding(&invalid_version_syntax(), &o),
            "src/api.rs:31: invalid version constraint \">=2.x\": remove thing"
        );
        assert_eq!(
            render_finding(&unsupported_comparator_finding(), &o),
            "src/api.rs:32: unsupported comparator \"<\" (use >=X: fires once version reaches X): old behavior"
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
        let line = render_finding(&version_reached(), &opts(Format::Github, false));
        assert_eq!(
            line,
            "::error file=src/api.rs,line=30,title=todo-by version 2.1.0 reached (>=2.0)::drop legacy endpoint"
        );
        let line = render_finding(
            &unsupported_comparator_finding(),
            &opts(Format::Github, false),
        );
        assert_eq!(
            // The hint's own ':' goes through gh_escape_property like any
            // other title content, becoming %3A.
            line,
            "::error file=src/api.rs,line=32,title=todo-by unsupported comparator \"<\" (use >=X%3A fires once version reaches X)::old behavior"
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
        let line = render_finding(&version_reached(), &opts(Format::Json, false));
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"version-reached\",\"path\":\"src/api.rs\",\"line\":30,\"constraint\":\">=2.0\",\"current_version\":\"2.1.0\",\"message\":\"drop legacy endpoint\"}"
        );
    }

    #[test]
    fn json_invalid_trigger_shape_has_no_current_version() {
        let line = render_finding(&invalid_version_syntax(), &opts(Format::Json, false));
        assert_eq!(
            line,
            "{\"type\":\"finding\",\"kind\":\"invalid-trigger\",\"path\":\"src/api.rs\",\"line\":31,\"constraint\":\">=2.x\",\"message\":\"remove thing\"}"
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
}
