use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use ignore::{WalkBuilder, WalkState};

mod date;
mod scanner;

use date::Date;
use scanner::{Finding, Kind};

const USAGE: &str = "\
todo-by: flag todo-by tags whose deadline date has passed

Usage: todo-by [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Files or directories to scan (default: current directory)

Options:
      --format <FORMAT>  Output format: text, github [default: text]
      --today <DATE>     Treat tags due on or before this date as overdue
                         (YYYY-MM-DD, default: today in UTC)
      --hidden           Also scan hidden files and directories
  -h, --help             Print help
  -V, --version          Print version

Exit codes: 0 no findings, 1 findings, 2 usage or I/O error";

#[derive(Clone, Copy)]
enum Format {
    Text,
    Github,
}

struct Cli {
    paths: Vec<PathBuf>,
    format: Format,
    today: Option<String>,
    hidden: bool,
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        paths: Vec::new(),
        format: Format::Text,
        today: None,
        hidden: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline_value
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match flag.as_str() {
            "--format" => {
                cli.format = match value("--format")?.as_str() {
                    "text" => Format::Text,
                    "github" => Format::Github,
                    other => return Err(format!("unknown format {other:?} (text, github)")),
                }
            }
            "--today" => cli.today = Some(value("--today")?),
            "--hidden" => cli.hidden = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("todo-by {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg:?}")),
            _ => cli.paths.push(PathBuf::from(arg)),
        }
    }
    if cli.paths.is_empty() {
        cli.paths.push(PathBuf::from("."));
    }
    Ok(cli)
}

fn today_utc() -> Date {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Date::from_days_since_epoch(secs.div_euclid(86_400))
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

fn render(findings: &[Finding], format: Format) {
    match format {
        Format::Text => {
            for f in findings {
                match f.kind {
                    Kind::Overdue => {
                        println!(
                            "{}:{}: overdue since {}: {}",
                            f.file, f.line, f.date, f.message
                        )
                    }
                    Kind::InvalidDate => {
                        println!(
                            "{}:{}: invalid date {}: {}",
                            f.file, f.line, f.date, f.message
                        )
                    }
                }
            }
            if !findings.is_empty() {
                let n = findings.len();
                eprintln!("{n} finding{}", if n == 1 { "" } else { "s" });
            }
        }
        Format::Github => {
            for f in findings {
                let label = match f.kind {
                    Kind::Overdue => format!("todo-by overdue since {}", f.date),
                    Kind::InvalidDate => format!("todo-by invalid date {}", f.date),
                };
                println!(
                    "::error file={},line={},title={}::{}",
                    gh_escape_property(&f.file),
                    f.line,
                    gh_escape_property(&label),
                    gh_escape_data(&f.message)
                );
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("todo-by: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let today = match &cli.today {
        Some(s) => match Date::parse_full(s) {
            Some(d) => d,
            None => {
                eprintln!("todo-by: --today must be a valid YYYY-MM-DD date, got {s:?}");
                return ExitCode::from(2);
            }
        },
        None => today_utc(),
    };

    let mut had_error = false;
    let roots: Vec<&PathBuf> = cli
        .paths
        .iter()
        .filter(|root| {
            let exists = root.exists();
            if !exists {
                eprintln!("todo-by: path does not exist: {}", root.display());
                had_error = true;
            }
            exists
        })
        .collect();

    let mut findings = Vec::new();
    if let Some((first, rest)) = roots.split_first() {
        let mut builder = WalkBuilder::new(first);
        for root in rest {
            builder.add(root);
        }
        builder
            .hidden(!cli.hidden)
            // Respect .gitignore even outside a git repository.
            .require_git(false);

        let io_error = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel::<Finding>();
        builder.build_parallel().run(|| {
            let tx = tx.clone();
            let io_error = &io_error;
            Box::new(move |entry| {
                match entry {
                    Ok(entry) => {
                        if entry.file_type().is_some_and(|t| t.is_file()) {
                            let mut local = Vec::new();
                            if let Err(err) = scanner::scan_file(entry.path(), today, &mut local) {
                                eprintln!("todo-by: {}: {err}", entry.path().display());
                                io_error.store(true, Ordering::Relaxed);
                            }
                            for finding in local {
                                let _ = tx.send(finding);
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("todo-by: {err}");
                        io_error.store(true, Ordering::Relaxed);
                    }
                }
                WalkState::Continue
            })
        });
        drop(tx);
        findings = rx.into_iter().collect();
        had_error = had_error || io_error.load(Ordering::Relaxed);
    }

    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    render(&findings, cli.format);

    if had_error {
        ExitCode::from(2)
    } else if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_escaping_neutralizes_command_syntax() {
        assert_eq!(gh_escape_property("a,b:c.txt"), "a%2Cb%3Ac.txt");
        assert_eq!(gh_escape_property("50%,done"), "50%25%2Cdone");
        assert_eq!(gh_escape_data("line1\nline2, 50%"), "line1%0Aline2, 50%25");
        assert_eq!(gh_escape_data("cr\rlf"), "cr%0Dlf");
    }
}
