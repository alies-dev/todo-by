use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use ignore::overrides::{Override, OverrideBuilder};
use ignore::{WalkBuilder, WalkState};

mod config;
mod date;
mod output;
mod scanner;
mod version;

use date::Date;
use output::{Format, RenderOpts};
use scanner::{Finding, ScanCtx, Trigger};
use version::Version;

const USAGE: &str = "\
todo-by: flag todo-by tags whose deadline date has passed

Usage: todo-by [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...             Files or directories to scan (default: current
                         directory); \"-\" reads stdin as a single file

Options:
      --format <FORMAT>   Output format: text, github, json
                         [default: text; github auto-selected in GitHub Actions]
      --today <DATE>      Treat tags due on or before this date as overdue
                         (YYYY-MM-DD, default: today in UTC)
      --current-version <X> Current version for version-constraint triggers
                         (default: TODO_BY_VERSION env, then config
                         version-cmd, then git describe --tags --abbrev=0)
      --warn <N>           Also report tags due within N days as warnings
      --exit-zero          Always exit 0 on findings (still 2 on errors)
      --color <WHEN>       Color: auto, always, never [default: auto]
      --hidden             Also scan hidden files and directories
      --files              List files that would be scanned, then exit
      --dump-config        Print effective config, then exit
  -h, --help               Print help
  -V, --version            Print version

Exit codes: 0 no findings, 1 findings, 2 usage, config, or I/O error";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColorWhen {
    Auto,
    Always,
    Never,
}

#[derive(Debug)]
struct Cli {
    paths: Vec<PathBuf>,
    format: Option<Format>,
    today: Option<String>,
    /// Raw, unvalidated: validated lazily, only when the scan actually
    /// produces a version candidate (see the laziness contract in `main`).
    current_version: Option<String>,
    warn: Option<u32>,
    exit_zero: bool,
    color: ColorWhen,
    hidden: bool,
    files: bool,
    dump_config: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli {
        paths: Vec::new(),
        format: None,
        today: None,
        current_version: None,
        warn: None,
        exit_zero: false,
        color: ColorWhen::Auto,
        hidden: false,
        files: false,
        dump_config: false,
    };
    let mut args = args;
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
                cli.format = Some(match value("--format")?.as_str() {
                    "text" => Format::Text,
                    "github" => Format::Github,
                    "json" => Format::Json,
                    other => return Err(format!("unknown format {other:?} (text, github, json)")),
                })
            }
            "--today" => cli.today = Some(value("--today")?),
            "--current-version" => cli.current_version = Some(value("--current-version")?),
            "--warn" => {
                let raw = value("--warn")?;
                cli.warn =
                    Some(raw.parse::<u32>().map_err(|_| {
                        format!("--warn must be a non-negative integer, got {raw:?}")
                    })?);
            }
            "--exit-zero" => cli.exit_zero = true,
            "--color" => {
                cli.color = match value("--color")?.as_str() {
                    "auto" => ColorWhen::Auto,
                    "always" => ColorWhen::Always,
                    "never" => ColorWhen::Never,
                    other => {
                        return Err(format!(
                            "unknown color mode {other:?} (auto, always, never)"
                        ))
                    }
                }
            }
            "--hidden" => cli.hidden = true,
            "--files" => cli.files = true,
            "--dump-config" => cli.dump_config = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("todo-by {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-" => cli.paths.push(PathBuf::from("-")),
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

/// Resolves the output format: `--format` flag, then `TODO_BY_FORMAT` env
/// (invalid value is an error naming the env var), then `github` when
/// running in GitHub Actions, else `text`.
fn resolve_format(
    flag: Option<Format>,
    env_format: Option<&str>,
    github_actions: Option<&str>,
) -> Result<Format, String> {
    if let Some(f) = flag {
        return Ok(f);
    }
    if let Some(v) = env_format {
        return match v {
            "text" => Ok(Format::Text),
            "github" => Ok(Format::Github),
            "json" => Ok(Format::Json),
            other => Err(format!(
                "TODO_BY_FORMAT must be text, github, or json, got {other:?}"
            )),
        };
    }
    if github_actions == Some("true") {
        return Ok(Format::Github);
    }
    Ok(Format::Text)
}

/// Resolves the warn window: `--warn` flag, then
/// `TODO_BY_WARN` env (invalid value is an error naming the env
/// var), then the config file's `warn`, else off.
fn resolve_warn(
    flag: Option<u32>,
    env_warn: Option<&str>,
    config_warn: Option<u32>,
) -> Result<Option<u32>, String> {
    if let Some(n) = flag {
        return Ok(Some(n));
    }
    if let Some(v) = env_warn {
        return v
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("TODO_BY_WARN must be a non-negative integer, got {v:?}"));
    }
    Ok(config_warn)
}

/// Where the current version comes from, in precedence order. A pure
/// (I/O-free) choice: it just picks which tier wins given already-collected
/// string values, so precedence can be unit tested without running git or a
/// shell. Actually producing a version string from the winning tier (a
/// shell command, or `git describe`) happens in `resolve_current_version`,
/// which runs only for the tier this function picks.
#[derive(Debug, PartialEq, Eq)]
enum VersionSource {
    Flag(String),
    Env(String),
    ConfigCmd(String),
    GitDefault,
}

impl VersionSource {
    /// Human-readable origin for error messages ("current version X from
    /// Y is not valid", "could not run Y").
    fn label(&self) -> String {
        match self {
            VersionSource::Flag(_) => "--current-version".to_string(),
            VersionSource::Env(_) => "TODO_BY_VERSION".to_string(),
            VersionSource::ConfigCmd(cmd) => format!("version-cmd {cmd:?}"),
            VersionSource::GitDefault => "git describe --tags --abbrev=0".to_string(),
        }
    }
}

/// Precedence: `--current-version` flag, then `TODO_BY_VERSION` env, then
/// the config's `version-cmd`, else the git-tag default.
fn choose_version_source(
    flag: Option<&str>,
    env: Option<&str>,
    config_cmd: Option<&str>,
) -> VersionSource {
    if let Some(v) = flag {
        return VersionSource::Flag(v.to_string());
    }
    if let Some(v) = env {
        return VersionSource::Env(v.to_string());
    }
    if let Some(cmd) = config_cmd {
        return VersionSource::ConfigCmd(cmd.to_string());
    }
    VersionSource::GitDefault
}

/// Produces the raw current-version string for the chosen source, running a
/// shell command or `git` only for the tier that actually won (laziness
/// lives one level up: `main` only calls this when the scan produced a
/// version candidate at all).
fn resolve_current_version(source: VersionSource, invocation_dir: &Path) -> Result<String, String> {
    match source {
        VersionSource::Flag(v) | VersionSource::Env(v) => Ok(v),
        VersionSource::ConfigCmd(cmd) => run_version_cmd(&cmd),
        VersionSource::GitDefault => run_git_describe(invocation_dir),
    }
}

fn run_version_cmd(cmd: &str) -> Result<String, String> {
    // `sh` isn't a given on Windows runners/installs; `cmd` is.
    let output = if cfg!(windows) {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(cmd)
            .output()
    } else {
        std::process::Command::new("sh").arg("-c").arg(cmd).output()
    }
    .map_err(|err| format!("version-cmd {cmd:?} failed to run: {err}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "version-cmd {cmd:?} exited with {}: {stderr}",
            output.status
        ));
    }
    if stdout.is_empty() {
        return Err(format!("version-cmd {cmd:?} produced empty output"));
    }
    Ok(stdout)
}

/// Errors only when actually called: `main` only reaches this when the
/// scan produced a version candidate, so a repo with no version tags in
/// comments never runs git and never fails because it has no git tags.
fn run_git_describe(invocation_dir: &Path) -> Result<String, String> {
    const REMEDY: &str = "set version-cmd in todo-by.toml or pass --current-version";
    let output = std::process::Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .current_dir(invocation_dir)
        .output()
        .map_err(|err| {
            format!(
                "could not determine current version: git describe failed to run ({err}); {REMEDY}"
            )
        })?;
    if !output.status.success() {
        // git's own stderr distinguishes "no tags" from e.g. "not a git
        // repository"; hardcoding one cause here would misreport the others.
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "found no tags".to_string()
        } else {
            stderr
        };
        return Err(format!(
            "could not determine current version: git describe --tags --abbrev=0 failed ({detail}); {REMEDY}"
        ));
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Err(format!(
            "could not determine current version: git describe --tags --abbrev=0 produced no output; {REMEDY}"
        ));
    }
    Ok(raw)
}

/// Resolves every `VersionPending` finding the scanner couldn't classify on
/// its own: promotes satisfied ones to `VersionReached`, recording the
/// current version for display, and drops the rest. Skips (rather than
/// panics on) a `VersionPending` finding whose constraint is somehow
/// missing, since that combination should be unreachable from `scanner`.
fn resolve_version_candidates(
    findings: &mut Vec<Finding>,
    current: &Version,
    current_display: &str,
) {
    for f in findings.iter_mut() {
        if !matches!(f.kind, scanner::Kind::VersionPending) {
            continue;
        }
        let Trigger::Version {
            constraint: Some(c),
            current_version,
            ..
        } = &mut f.trigger
        else {
            continue;
        };
        if c.satisfied_by(current) {
            f.kind = scanner::Kind::VersionReached;
            *current_version = Some(current_display.to_string());
        }
    }
    findings.retain(|f| !matches!(f.kind, scanner::Kind::VersionPending));
}

/// Resolves whether Text output should be colored. `auto` requires a TTY
/// stdout, an unset-or-empty `NO_COLOR`, and `TERM` other than "dumb".
fn resolve_color(
    when: ColorWhen,
    stdout_is_tty: bool,
    no_color_set: bool,
    term_is_dumb: bool,
) -> bool {
    match when {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => stdout_is_tty && !no_color_set && !term_is_dumb,
    }
}

/// Builds the exclude overrides from `patterns` (config `exclude`),
/// rooted at `root`. Each pattern is added as a `!`-prefixed glob, which in
/// override-builder semantics means "exclude" rather than "whitelist".
///
/// `root` must be the invocation directory: the walker hands the matcher
/// paths in the same (usually relative) form the scan roots were given in,
/// so anchored globs only match against the right base when both share the
/// current directory as their basis. Rooting at the config file's directory
/// instead would silently anchor patterns wrongly whenever the config lives in
/// an ancestor of the invocation directory.
fn build_overrides(root: &Path, patterns: &[String]) -> Result<Option<Override>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = OverrideBuilder::new(root);
    for pattern in patterns {
        builder
            .add(&format!("!{pattern}"))
            .map_err(|err| format!("invalid exclude pattern {pattern:?}: {err}"))?;
    }
    builder
        .build()
        .map(Some)
        .map_err(|err| format!("invalid exclude patterns: {err}"))
}

/// Walks `roots` (already filtered to existing, non-stdin paths) in
/// parallel, scanning every file. Returns findings and whether any I/O
/// error occurred.
fn scan_roots(
    roots: &[PathBuf],
    hidden: bool,
    overrides: Option<Override>,
    today: Date,
    warn_until: Option<Date>,
    tags: &[String],
) -> (Vec<Finding>, bool) {
    let Some((first, rest)) = roots.split_first() else {
        return (Vec::new(), false);
    };
    let mut builder = WalkBuilder::new(first);
    for root in rest {
        builder.add(root);
    }
    builder.hidden(!hidden).require_git(false);
    if let Some(ov) = overrides {
        builder.overrides(ov);
    }

    let io_error = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<Finding>();
    builder.build_parallel().run(|| {
        let tx = tx.clone();
        let io_error = &io_error;
        Box::new(move |entry| {
            match entry {
                Ok(entry) => {
                    if entry.file_type().is_some_and(|t| t.is_file()) {
                        let ctx = ScanCtx {
                            today,
                            warn_until,
                            tags,
                        };
                        let mut local = Vec::new();
                        if let Err(err) = scanner::scan_file(entry.path(), &ctx, &mut local) {
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
    let findings = rx.into_iter().collect();
    (findings, io_error.load(Ordering::Relaxed))
}

/// Walks `roots` single-threaded, collecting file paths for `--files`.
fn list_file_paths(
    roots: &[PathBuf],
    hidden: bool,
    overrides: Option<Override>,
) -> (Vec<String>, bool) {
    let Some((first, rest)) = roots.split_first() else {
        return (Vec::new(), false);
    };
    let mut builder = WalkBuilder::new(first);
    for root in rest {
        builder.add(root);
    }
    builder.hidden(!hidden).require_git(false);
    if let Some(ov) = overrides {
        builder.overrides(ov);
    }

    let mut had_error = false;
    let mut paths = Vec::new();
    for entry in builder.build() {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    paths.push(entry.path().display().to_string());
                }
            }
            Err(err) => {
                eprintln!("todo-by: {err}");
                had_error = true;
            }
        }
    }
    paths.sort();
    (paths, had_error)
}

fn main() -> ExitCode {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("todo-by: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let format = match resolve_format(
        cli.format,
        std::env::var("TODO_BY_FORMAT").ok().as_deref(),
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
    ) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("todo-by: {err}");
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

    let start_dir = match std::env::current_dir() {
        Ok(d) => d,
        Err(err) => {
            eprintln!("todo-by: {err}");
            return ExitCode::from(2);
        }
    };
    let cfg = match config::load(&start_dir) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("todo-by: {err}");
            return ExitCode::from(2);
        }
    };

    let warn = match resolve_warn(
        cli.warn,
        std::env::var("TODO_BY_WARN").ok().as_deref(),
        cfg.warn,
    ) {
        Ok(w) => w,
        Err(err) => {
            eprintln!("todo-by: {err}");
            return ExitCode::from(2);
        }
    };

    if cli.dump_config {
        let effective = config::Config { warn, ..cfg };
        print!("{}", config::dump(&effective));
        return ExitCode::SUCCESS;
    }

    let color = resolve_color(
        cli.color,
        std::io::stdout().is_terminal(),
        // var_os, not var: a non-UTF-8 value is still "present and not an
        // empty string" per the NO_COLOR spec, so it must disable color.
        std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        std::env::var("TERM").map(|v| v == "dumb").unwrap_or(false),
    );

    let warn_until =
        warn.map(|n| Date::from_days_since_epoch(today.to_days_since_epoch() + n as i64));

    let overrides = match build_overrides(&start_dir, &cfg.exclude) {
        Ok(ov) => ov,
        Err(err) => {
            eprintln!("todo-by: {err}");
            return ExitCode::from(2);
        }
    };

    let mut had_error = false;
    let mut has_stdin = false;
    let mut fs_paths = Vec::new();
    for p in &cli.paths {
        if p.as_os_str() == "-" {
            has_stdin = true;
            continue;
        }
        if p.exists() {
            fs_paths.push(p.clone());
        } else {
            eprintln!("todo-by: path does not exist: {}", p.display());
            had_error = true;
        }
    }

    if cli.files {
        let (paths, walk_error) = list_file_paths(&fs_paths, cli.hidden, overrides);
        for p in &paths {
            println!("{p}");
        }
        return if had_error || walk_error {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }

    let (mut findings, walk_error) = scan_roots(
        &fs_paths, cli.hidden, overrides, today, warn_until, &cfg.tags,
    );
    had_error = had_error || walk_error;

    if has_stdin {
        // Raw bytes, not read_to_string: invalid UTF-8 on stdin must scan
        // lossily like file contents do, not abort with an I/O error.
        let mut input = Vec::new();
        match Read::read_to_end(&mut std::io::stdin(), &mut input) {
            Ok(_) => {
                let ctx = ScanCtx {
                    today,
                    warn_until,
                    tags: &cfg.tags,
                };
                scanner::scan_bytes("<stdin>", &input, &ctx, &mut findings);
            }
            Err(err) => {
                eprintln!("todo-by: <stdin>: {err}");
                had_error = true;
            }
        }
    }

    // Laziness is a hard requirement: resolving the current version can run
    // git or a config-defined shell command, so it must happen only when
    // the scan actually produced a version candidate. A repo with no
    // version tags in comments never runs git and never fails over missing
    // tags; invalid-trigger findings alone (already fully classified) don't
    // count as a candidate either.
    if findings
        .iter()
        .any(|f| matches!(f.kind, scanner::Kind::VersionPending))
    {
        let source = choose_version_source(
            cli.current_version.as_deref(),
            std::env::var("TODO_BY_VERSION").ok().as_deref(),
            cfg.version_cmd.as_deref(),
        );
        let label = source.label();
        let raw = match resolve_current_version(source, &start_dir) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("todo-by: {err}");
                return ExitCode::from(2);
            }
        };
        let display = raw.strip_prefix(['v', 'V']).unwrap_or(&raw).to_string();
        let current = match Version::parse(&display) {
            Some(v) => v,
            None => {
                eprintln!("todo-by: current version {raw:?} from {label} is not a valid version");
                return ExitCode::from(2);
            }
        };
        resolve_version_candidates(&mut findings, &current, &display);
    }

    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));

    let opts = RenderOpts {
        format,
        color,
        today,
    };
    output::render(&findings, &opts);

    if had_error {
        return ExitCode::from(2);
    }
    let (errors, _warnings) = output::counts(&findings);
    if errors > 0 && !cli.exit_zero {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> impl Iterator<Item = String> {
        items
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn default_paths_to_current_dir() {
        let cli = parse_args(args(&[])).unwrap();
        assert_eq!(cli.paths, vec![PathBuf::from(".")]);
    }

    #[test]
    fn dash_is_a_path_not_an_unknown_flag() {
        let cli = parse_args(args(&["-", "src"])).unwrap();
        assert_eq!(cli.paths, vec![PathBuf::from("-"), PathBuf::from("src")]);
    }

    #[test]
    fn dash_alone_does_not_trigger_default_path() {
        let cli = parse_args(args(&["-"])).unwrap();
        assert_eq!(cli.paths, vec![PathBuf::from("-")]);
    }

    #[test]
    fn format_flag_parses_all_three_values() {
        assert_eq!(
            parse_args(args(&["--format", "text"])).unwrap().format,
            Some(Format::Text)
        );
        assert_eq!(
            parse_args(args(&["--format", "github"])).unwrap().format,
            Some(Format::Github)
        );
        assert_eq!(
            parse_args(args(&["--format", "json"])).unwrap().format,
            Some(Format::Json)
        );
    }

    #[test]
    fn unknown_format_value_is_rejected() {
        let err = parse_args(args(&["--format", "xml"])).unwrap_err();
        assert!(err.contains("xml"), "{err}");
    }

    #[test]
    fn warn_inline_and_split_forms() {
        let cli = parse_args(args(&["--warn=14"])).unwrap();
        assert_eq!(cli.warn, Some(14));
        let cli = parse_args(args(&["--warn", "7"])).unwrap();
        assert_eq!(cli.warn, Some(7));
    }

    #[test]
    fn warn_rejects_non_integer() {
        let err = parse_args(args(&["--warn", "soon"])).unwrap_err();
        assert!(err.contains("soon"), "{err}");
    }

    #[test]
    fn exit_zero_flag() {
        assert!(parse_args(args(&["--exit-zero"])).unwrap().exit_zero);
        assert!(!parse_args(args(&[])).unwrap().exit_zero);
    }

    #[test]
    fn color_flag_parses_all_three_values() {
        assert_eq!(
            parse_args(args(&["--color", "auto"])).unwrap().color,
            ColorWhen::Auto
        );
        assert_eq!(
            parse_args(args(&["--color", "always"])).unwrap().color,
            ColorWhen::Always
        );
        assert_eq!(
            parse_args(args(&["--color", "never"])).unwrap().color,
            ColorWhen::Never
        );
    }

    #[test]
    fn unknown_color_value_is_rejected() {
        assert!(parse_args(args(&["--color", "rainbow"])).is_err());
    }

    #[test]
    fn files_and_dump_config_flags() {
        let cli = parse_args(args(&["--files"])).unwrap();
        assert!(cli.files);
        assert!(!cli.dump_config);
        let cli = parse_args(args(&["--dump-config"])).unwrap();
        assert!(cli.dump_config);
        assert!(!cli.files);
    }

    #[test]
    fn unknown_flag_is_rejected() {
        assert!(parse_args(args(&["--bogus"])).is_err());
    }

    #[test]
    fn missing_value_is_rejected() {
        assert!(parse_args(args(&["--format"])).is_err());
        assert!(parse_args(args(&["--warn"])).is_err());
    }

    #[test]
    fn format_resolution_precedence() {
        // flag beats env beats GITHUB_ACTIONS beats default
        assert_eq!(
            resolve_format(Some(Format::Json), Some("github"), Some("true")),
            Ok(Format::Json)
        );
        assert_eq!(
            resolve_format(None, Some("github"), Some("true")),
            Ok(Format::Github)
        );
        assert_eq!(resolve_format(None, None, Some("true")), Ok(Format::Github));
        assert_eq!(resolve_format(None, None, None), Ok(Format::Text));
        assert_eq!(resolve_format(None, None, Some("false")), Ok(Format::Text));
    }

    #[test]
    fn format_resolution_rejects_invalid_env_value() {
        let err = resolve_format(None, Some("xml"), None).unwrap_err();
        assert!(err.contains("TODO_BY_FORMAT"), "{err}");
        assert!(err.contains("xml"), "{err}");
    }

    #[test]
    fn warn_resolution_precedence() {
        // flag beats env beats config beats None
        assert_eq!(resolve_warn(Some(3), Some("5"), Some(7)), Ok(Some(3)));
        assert_eq!(resolve_warn(None, Some("5"), Some(7)), Ok(Some(5)));
        assert_eq!(resolve_warn(None, None, Some(7)), Ok(Some(7)));
        assert_eq!(resolve_warn(None, None, None), Ok(None));
    }

    #[test]
    fn warn_resolution_rejects_invalid_env_value() {
        let err = resolve_warn(None, Some("soon"), None).unwrap_err();
        assert!(err.contains("TODO_BY_WARN"), "{err}");
        assert!(err.contains("soon"), "{err}");
    }

    #[test]
    fn build_overrides_excludes_relative_paths_against_the_given_root() {
        // The walker hands the matcher cwd-relative paths; anchored globs
        // must match against that same basis.
        let ov = build_overrides(Path::new("/some/root"), &["vendor/**".to_string()])
            .unwrap()
            .unwrap();
        assert!(ov
            .matched(Path::new("vendor/generated.go"), false)
            .is_ignore());
        assert!(!ov.matched(Path::new("src/lib.rs"), false).is_ignore());
    }

    #[test]
    fn build_overrides_rejects_bad_pattern_and_skips_empty_list() {
        assert!(build_overrides(Path::new("."), &["{bad".to_string()]).is_err());
        assert!(build_overrides(Path::new("."), &[]).unwrap().is_none());
    }

    #[test]
    fn color_resolution_matrix() {
        assert!(resolve_color(ColorWhen::Always, false, true, true));
        assert!(!resolve_color(ColorWhen::Never, true, false, false));
        assert!(resolve_color(ColorWhen::Auto, true, false, false));
        assert!(!resolve_color(ColorWhen::Auto, false, false, false));
        // NO_COLOR set and non-empty disables auto color
        assert!(!resolve_color(ColorWhen::Auto, true, true, false));
        // TERM=dumb disables auto color
        assert!(!resolve_color(ColorWhen::Auto, true, false, true));
    }

    #[test]
    fn current_version_flag_inline_and_split_forms() {
        let cli = parse_args(args(&["--current-version=2.1.0"])).unwrap();
        assert_eq!(cli.current_version, Some("2.1.0".to_string()));
        let cli = parse_args(args(&["--current-version", "2.1.0"])).unwrap();
        assert_eq!(cli.current_version, Some("2.1.0".to_string()));
    }

    #[test]
    fn current_version_flag_defers_validation() {
        // Unlike --today, an unparsable value here is not rejected at parse
        // time: laziness means it's only validated if the scan produces a
        // version candidate, which parse_args can't know about.
        let cli = parse_args(args(&["--current-version", "not-a-version"])).unwrap();
        assert_eq!(cli.current_version, Some("not-a-version".to_string()));
    }

    #[test]
    fn version_source_precedence() {
        // flag beats env beats config's version-cmd beats the git default
        assert_eq!(
            choose_version_source(Some("2.0.0"), Some("3.0.0"), Some("cmd")),
            VersionSource::Flag("2.0.0".to_string())
        );
        assert_eq!(
            choose_version_source(None, Some("3.0.0"), Some("cmd")),
            VersionSource::Env("3.0.0".to_string())
        );
        assert_eq!(
            choose_version_source(None, None, Some("cmd")),
            VersionSource::ConfigCmd("cmd".to_string())
        );
        assert_eq!(
            choose_version_source(None, None, None),
            VersionSource::GitDefault
        );
    }

    #[test]
    fn version_source_labels_name_their_origin() {
        assert_eq!(
            VersionSource::Flag("2.0".to_string()).label(),
            "--current-version"
        );
        assert_eq!(
            VersionSource::Env("2.0".to_string()).label(),
            "TODO_BY_VERSION"
        );
        assert_eq!(
            VersionSource::ConfigCmd("jq -r .version".to_string()).label(),
            "version-cmd \"jq -r .version\""
        );
        assert_eq!(
            VersionSource::GitDefault.label(),
            "git describe --tags --abbrev=0"
        );
    }

    fn version_pending(written: &str, message: &str) -> Finding {
        Finding {
            file: "a.rs".to_string(),
            line: 1,
            kind: scanner::Kind::VersionPending,
            trigger: Trigger::Version {
                written: written.to_string(),
                constraint: version::Constraint::parse(written),
                current_version: None,
            },
            message: message.to_string(),
        }
    }

    #[test]
    fn resolve_version_candidates_promotes_satisfied_and_drops_unsatisfied() {
        let mut findings = vec![
            version_pending(">=2.0", "satisfied"),
            version_pending(">=999.0", "not yet"),
        ];
        let current = Version::parse("2.1.0").unwrap();
        resolve_version_candidates(&mut findings, &current, "2.1.0");

        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].kind, scanner::Kind::VersionReached));
        assert_eq!(findings[0].message, "satisfied");
        match &findings[0].trigger {
            Trigger::Version {
                current_version, ..
            } => assert_eq!(current_version.as_deref(), Some("2.1.0")),
            Trigger::Date { .. } => panic!("expected Trigger::Version"),
        }
    }

    #[test]
    fn resolution_is_skipped_when_no_candidates_are_present() {
        // Documents the laziness contract at the point it's enforced: main()
        // only resolves the current version behind an
        // any(kind == VersionPending) guard. InvalidTrigger findings are
        // already fully classified and must not count as a candidate.
        let findings = [Finding {
            file: "a.rs".to_string(),
            line: 1,
            kind: scanner::Kind::InvalidTrigger,
            trigger: Trigger::Version {
                written: "<1.0".to_string(),
                constraint: None,
                current_version: None,
            },
            message: "old".to_string(),
        }];
        assert!(!findings
            .iter()
            .any(|f| matches!(f.kind, scanner::Kind::VersionPending)));
    }
}
