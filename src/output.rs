//! Renders findings to stdout in one of three formats: human-readable text,
//! GitHub Actions workflow commands, or JSON Lines.

use crate::date::Date;
use crate::scanner::{Finding, Kind};

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

fn render_text(f: &Finding, opts: &RenderOpts) -> String {
    let (phrase, color) = match f.kind {
        Kind::Overdue => (format!("overdue since {}", f.date), RED),
        Kind::InvalidDate => (format!("invalid date {}", f.date), RED),
        Kind::DueSoon => {
            let deadline = f.deadline.expect("DueSoon always carries a deadline");
            let n = days_between(opts.today, deadline);
            (format!("due in {} ({deadline})", plural_days(n)), YELLOW)
        }
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
        Kind::Overdue => ("error", format!("todo-by overdue since {}", f.date)),
        Kind::InvalidDate => ("error", format!("todo-by invalid date {}", f.date)),
        Kind::DueSoon => {
            let deadline = f.deadline.expect("DueSoon always carries a deadline");
            let n = days_between(today, deadline);
            (
                "warning",
                format!("todo-by due in {} ({deadline})", plural_days(n)),
            )
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

fn render_json_finding(f: &Finding, today: Date) -> String {
    match f.kind {
        Kind::Overdue => {
            let deadline = f.deadline.expect("Overdue always carries a deadline");
            let days = days_between(deadline, today);
            format!(
                "{{\"type\":\"finding\",\"kind\":\"overdue\",\"path\":\"{}\",\"line\":{},\
                 \"date\":\"{}\",\"deadline\":\"{deadline}\",\"days_overdue\":{days},\
                 \"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(&f.date),
                escape_json(&f.message)
            )
        }
        Kind::DueSoon => {
            let deadline = f.deadline.expect("DueSoon always carries a deadline");
            let days = days_between(today, deadline);
            format!(
                "{{\"type\":\"finding\",\"kind\":\"due-soon\",\"path\":\"{}\",\"line\":{},\
                 \"date\":\"{}\",\"deadline\":\"{deadline}\",\"days_until_due\":{days},\
                 \"message\":\"{}\"}}",
                escape_json(&f.file),
                f.line,
                escape_json(&f.date),
                escape_json(&f.message)
            )
        }
        Kind::InvalidDate => format!(
            "{{\"type\":\"finding\",\"kind\":\"invalid-date\",\"path\":\"{}\",\"line\":{},\
             \"date\":\"{}\",\"deadline\":null,\"message\":\"{}\"}}",
            escape_json(&f.file),
            f.line,
            escape_json(&f.date),
            escape_json(&f.message)
        ),
    }
}

/// Splits findings into error-level (Overdue, InvalidDate) and warning-level
/// (DueSoon) counts; also drives the exit code in main.
pub fn counts(findings: &[Finding]) -> (usize, usize) {
    let errors = findings
        .iter()
        .filter(|f| matches!(f.kind, Kind::Overdue | Kind::InvalidDate))
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
    use crate::scanner::{Finding, Kind};

    fn date(s: &str) -> Date {
        Date::parse_full(s).unwrap()
    }

    fn overdue() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 12,
            kind: Kind::Overdue,
            date: "2000-01-01".to_string(),
            deadline: Some(date("2000-01-01")),
            message: "remove workaround".to_string(),
        }
    }

    fn due_soon() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 20,
            kind: Kind::DueSoon,
            date: "2000-01-10".to_string(),
            deadline: Some(date("2000-01-10")),
            message: "drop feature flag".to_string(),
        }
    }

    fn invalid() -> Finding {
        Finding {
            file: "src/lib.rs".to_string(),
            line: 30,
            kind: Kind::InvalidDate,
            date: "2000-02-30".to_string(),
            deadline: None,
            message: "typo'd date".to_string(),
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
        f.deadline = Some(date("2000-01-06"));
        f.date = "2000-01-06".to_string();
        assert_eq!(
            render_finding(&f, &o),
            "src/lib.rs:20: due in 1 day (2000-01-06): drop feature flag"
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
    fn json_summary_counts_errors_and_warnings_separately() {
        assert_eq!(
            summary_json(&[overdue(), invalid(), due_soon()]),
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
