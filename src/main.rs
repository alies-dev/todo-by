// The whole point of `crate::stdout` is that it is the only way out. A
// `println!` anywhere else reintroduces the panic it exists to remove, so
// the compiler, not a reviewer, is what keeps them from coming back.
#![deny(clippy::print_stdout)]

use std::ffi::OsStr;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use ignore::overrides::{Override, OverrideBuilder};
use ignore::{WalkBuilder, WalkState};

mod config;
mod date;
mod issue;
mod json;
mod output;
mod scanner;
mod stdout;
mod version;

use date::Date;
use output::{Format, RenderOpts};
use scanner::{Finding, ScanCtx};
use version::Version;

const USAGE: &str = "\
todo-by: flag todo-by tags whose deadline has passed, whose version has shipped,
        or whose GitHub issue has closed

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
      --online             Check #123 issue triggers against GitHub
                         (needs gh, or curl with GH_TOKEN set)
      --offline            Never check issue triggers, overriding config
      --exit-zero          Always exit 0 on findings (still 2 on errors)
      --color <WHEN>       Color: auto, always, never [default: auto]
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
    /// Some(true) from `--online`, Some(false) from `--offline`, None when
    /// neither was passed and the config decides.
    online: Option<bool>,
    exit_zero: bool,
    color: ColorWhen,
    files: bool,
    dump_config: bool,
    /// Notices for flags kept only so an existing invocation keeps
    /// working. `main` prints them once, to stderr, before it does
    /// anything else. Empty on almost every run.
    deprecated: Vec<&'static str>,
}

/// Records that a retired flag was passed. Deliberately not an error: the
/// CLI surface is frozen once a version ships, so a CI job carrying the
/// flag has to keep running until the next major release removes it. Each
/// notice says what the flag does now and when it goes, and repeats of the
/// same flag collapse into one line.
fn deprecate(cli: &mut Cli, notice: &'static str) {
    if !cli.deprecated.contains(&notice) {
        cli.deprecated.push(notice);
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli {
        paths: Vec::new(),
        format: None,
        today: None,
        current_version: None,
        warn: None,
        online: None,
        exit_zero: false,
        color: ColorWhen::Auto,
        files: false,
        dump_config: false,
        deprecated: Vec::new(),
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
        // A flag that takes no value must refuse one rather than drop it:
        // `--online=false` parses as the flag plus a discarded "false", so
        // silently ignoring it does the exact opposite of what was written.
        // Same for every other switch here.
        const VALUELESS: [&str; 10] = [
            "--online",
            "--offline",
            "--exit-zero",
            "--hidden",
            "--files",
            "--dump-config",
            "-h",
            "--help",
            "-V",
            "--version",
        ];
        if inline_value.is_some() && VALUELESS.contains(&flag.as_str()) {
            return Err(format!("{flag} takes no value"));
        }
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
            "--online" => cli.online = Some(true),
            "--offline" => cli.online = Some(false),
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
            // Hidden files are scanned unconditionally now, so this flag
            // asks for what already happens. Out of --help, since there is
            // nothing left to choose, but still accepted, and now saying so
            // rather than going quiet until the day it disappears.
            // todo-by v1.0 delete this arm, its entry in VALUELESS, and the
            // notice below. A major bump is the point at which dropping an
            // accepted flag stops being a breaking change made by surprise.
            "--hidden" => deprecate(
                &mut cli,
                "--hidden does nothing: hidden files are scanned by default. \
                 The flag is accepted until 1.0 and will be removed there.",
            ),
            "--files" => cli.files = true,
            "--dump-config" => cli.dump_config = true,
            "-h" | "--help" => print_and_exit(&format!("{USAGE}\n")),
            "-V" | "--version" => {
                print_and_exit(&format!("todo-by {}\n", env!("CARGO_PKG_VERSION")))
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

/// Whether this run may reach the network for issue triggers. The flags
/// win over the config file in both directions, so `--offline` can switch
/// off an `online = true` that a shared config turned on.
fn resolve_online(flag: Option<bool>, config_online: Option<bool>) -> bool {
    flag.or(config_online).unwrap_or(false)
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
///
/// The two directories are NOT interchangeable: `config_run_dir` (see
/// [`version_run_dir`]) is the config file's directory, so a relative path
/// inside `version-cmd` resolves against the file that declared it. But
/// `git describe`'s default MUST run in `invocation_dir` (where `todo-by`
/// was actually invoked), not the config directory: config discovery walks
/// upward from the invocation directory looking for `todo-by.toml`, so the
/// config file can legitimately live above the repository itself (e.g. a
/// monorepo config at `/work/todo-by.toml` with the repo at
/// `/work/project`). Anchoring git there would make it describe the wrong
/// repository, or fail outright if `/work` isn't a repository at all.
fn resolve_current_version(
    source: VersionSource,
    config_run_dir: &Path,
    invocation_dir: &Path,
) -> Result<String, String> {
    match source {
        VersionSource::Flag(v) | VersionSource::Env(v) => Ok(v),
        VersionSource::ConfigCmd(cmd) => run_version_cmd(&cmd, config_run_dir),
        VersionSource::GitDefault => run_git_describe(invocation_dir),
    }
}

/// Whether the scan produced anything that needs a current version. This
/// is the laziness guard: no subprocess runs, and no missing-tag failure
/// is possible, for a tree whose tags are all dates. An invalid-trigger
/// finding doesn't count, being already fully classified.
fn needs_version_resolution(findings: &[Finding]) -> bool {
    findings
        .iter()
        .any(|f| matches!(f.kind, scanner::Kind::VersionPending { .. }))
}

/// Parses a resolved current version, which unlike trigger text is a
/// string this tool did not ask anyone to write: a git tag name, a
/// command's stdout, a CI variable. A `V` prefix is therefore accepted
/// here even though `V2.0` is invalid inside a tag, since failing an
/// entire scan over someone else's tag capitalization trades a real
/// report for a cosmetic objection.
fn parse_current_version(raw: &str, label: &str) -> Result<Version, String> {
    let invalid = || format!("current version {raw:?} from {label} is not a valid version");
    let normalized = raw.strip_prefix(['v', 'V']).unwrap_or(raw);
    // One prefix, not two: `Version::parse` strips a lowercase `v` of its
    // own, so without this guard `vv2.0` and `Vv2.0` would each lose both
    // and be accepted as 2.0.
    if normalized.starts_with(['v', 'V']) {
        return Err(invalid());
    }
    Version::parse(strip_git_describe_markers(normalized)).ok_or_else(invalid)
}

/// Strips the markers `git describe` appends to a tag name when HEAD isn't
/// the tagged commit (`-<n>-g<sha>`) or the tree is dirty (`-dirty`).
///
/// Semver reads either one as a pre-release, and a pre-release sorts BELOW
/// the release it is built from, so `v1.2.3-4-gabc123` compares less than
/// `1.2.3` and a `>=v1.2.3` trigger goes unreported: four commits PAST the
/// tag would count as never having reached it. Nothing is printed when that
/// happens, because an unsatisfied constraint has no finding to print, so
/// the tag is simply buried. That silent drop is the outcome this tool
/// exists to prevent, and it is reachable from any `version-cmd` or
/// `--current-version` carrying describe's default output (the built-in
/// `git describe --tags --abbrev=0` avoids the suffix, but a config saying
/// `version-cmd = "git describe --tags"` does not). So the markers come off
/// and the constraint is measured against the tag itself. A tag several
/// commits behind still reads as reached, which under-reports `>` by one
/// tag at worst and never hides a finding.
///
/// Deliberately narrow: only a trailing `-<digits>-g<hex>`, and only a
/// literal trailing `-dirty`. A real pre-release keeps its suffix
/// (`1.2.3-rc.1`), and describe run on a pre-release tag
/// (`1.2.3-rc.1-4-gabc123`) falls back to that pre-release rather than to
/// the release above it.
fn strip_git_describe_markers(s: &str) -> &str {
    let s = s.strip_suffix("-dirty").unwrap_or(s);
    let Some((head, sha)) = s.rsplit_once('-') else {
        return s;
    };
    let Some(hex) = sha.strip_prefix('g') else {
        return s;
    };
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return s;
    }
    match head.rsplit_once('-') {
        Some((rest, count)) if !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()) => {
            rest
        }
        _ => s,
    }
}

/// Directory `version-cmd` runs in: the loaded config file's directory,
/// falling back to the invocation directory when no config file exists.
/// Anchoring at the config file keeps a relative path inside `version-cmd`
/// working from any subdirectory (npm-script semantics). This is deliberately
/// NOT used for the git-describe default (see [`resolve_current_version`]):
/// unlike a shell command, git already walks upward from wherever it runs
/// to find the enclosing repository, so anchoring it at the config
/// directory buys nothing and risks pointing it at the wrong repository
/// when the config lives above the actual repo.
fn version_run_dir<'a>(config_source: Option<&'a Path>, start_dir: &'a Path) -> &'a Path {
    config_source.and_then(Path::parent).unwrap_or(start_dir)
}

fn run_version_cmd(cmd: &str, run_dir: &Path) -> Result<String, String> {
    // `sh` isn't a given on Windows runners/installs; `cmd` is. stdin is
    // closed rather than inherited: a command that decides to prompt (a
    // credential helper, say) would otherwise block a CI job forever with
    // no output, instead of failing at once on a closed stdin.
    let output = if cfg!(windows) {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(cmd)
            .current_dir(run_dir)
            .stdin(std::process::Stdio::null())
            .output()
    } else {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(run_dir)
            .stdin(std::process::Stdio::null())
            .output()
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
fn run_git_describe(run_dir: &Path) -> Result<String, String> {
    const REMEDY: &str = "set version-cmd in todo-by.toml or pass --current-version";
    let output = std::process::Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .current_dir(run_dir)
        .stdin(std::process::Stdio::null())
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
/// its own: promotes satisfied ones to `VersionReached { written }`, and
/// drops the rest. The current version itself isn't stored on the
/// finding: it's the same for every finding in a run, so it travels once
/// via `RenderOpts` instead (set by the caller after this returns).
fn resolve_version_candidates(findings: &mut Vec<Finding>, current: &Version) {
    findings.retain_mut(|f| {
        let scanner::Kind::VersionPending {
            written,
            constraint,
        } = &f.kind
        else {
            return true; // not a version candidate, keep as-is
        };
        if !constraint.satisfied_by(current) {
            return false; // not yet satisfied, drop
        }
        let written = written.clone();
        f.kind = scanner::Kind::VersionReached { written };
        true
    });
}

/// Applies resolved issue outcomes to the `IssuePending` findings that
/// produced them, in the same order they were collected: fired references
/// become `IssueClosed`, unresolvable ones `IssueError`, and open ones are
/// dropped. Mirrors [`resolve_version_candidates`], which does the same for
/// version constraints.
fn resolve_issue_candidates(findings: &mut Vec<Finding>, outcomes: Vec<issue::Outcome>) {
    let mut outcomes = outcomes.into_iter();
    findings.retain_mut(|f| {
        let scanner::Kind::IssuePending { written, .. } = &f.kind else {
            return true; // not an issue candidate, keep as-is
        };
        let written = written.clone();
        match outcomes.next() {
            Some(issue::Outcome::Fired { state, url }) => {
                f.kind = scanner::Kind::IssueClosed {
                    written,
                    state,
                    url,
                };
                true
            }
            Some(issue::Outcome::Failed(detail)) => {
                f.kind = scanner::Kind::IssueError { written, detail };
                true
            }
            // Open, or (unreachable) a short outcome list: nothing to say.
            _ => false,
        }
    });
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

/// Metadata directories no scan should ever enter. Each one stores commit
/// messages, and the last two store whole copies of tracked files as well,
/// so walking them turns one tag into a phantom finding at a path nobody
/// can edit, or into a duplicate of a finding already reported.
const VCS_DIRS: [&str; 4] = [".git", ".hg", ".svn", ".jj"];

fn is_vcs_dir(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| VCS_DIRS.contains(&name))
}

/// True when `path` names one of [`VCS_DIRS`] or sits inside one, whether
/// it is the usual directory or the plain file a git worktree gets.
/// Resolved first, so that running from within one is caught as well as
/// naming it: a relative root carries no such component of its own.
fn inside_vcs_dir(path: &Path) -> bool {
    let resolved = std::fs::canonicalize(path);
    let probe = resolved.as_deref().unwrap_or(path);
    probe.components().any(|c| is_vcs_dir(c.as_os_str()))
}

/// The roots `walk_builder` will drop, phrased for the user. Dropping
/// them quietly would make `todo-by .git/hooks` exit 0 with no output,
/// which is what a clean scan looks like, and the neighbouring check
/// already speaks up about a path that does not exist. Whether a path is
/// scanned stays `walk_builder`'s decision alone; this only describes it.
fn skipped_root_notices(roots: &[PathBuf]) -> Vec<String> {
    roots
        .iter()
        .filter(|root| inside_vcs_dir(root))
        .map(|root| {
            format!(
                "skipping {}: version control metadata is never scanned",
                root.display()
            )
        })
        .collect()
}

/// The walker configuration both scanning modes share, so `--files` and
/// the scan can never walk a different tree. The sets they report still
/// differ by the binary files the scan drops after the walk yields them.
///
/// `.gitignore` decides what is scanned, hidden entries included, with one
/// exception: the [`VCS_DIRS`] are never walked.
///
/// The exclusion matches on the entry name rather than on a path, so it
/// catches a submodule's or a nested checkout's metadata as well. Roots
/// are filtered separately because `ignore` applies `filter_entry` only
/// from depth 1 down, which would leave `todo-by .git` walking it anyway.
///
/// Returns `None` when no root survives, since a walker needs at least one
/// path to start from; the callers report an empty result instead.
fn walk_builder(roots: &[PathBuf], overrides: Option<Override>) -> Option<WalkBuilder> {
    let mut kept = roots.iter().filter(|root| !inside_vcs_dir(root));
    let mut builder = WalkBuilder::new(kept.next()?);
    for root in kept {
        builder.add(root);
    }
    builder
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| !is_vcs_dir(entry.file_name()));
    if let Some(ov) = overrides {
        builder.overrides(ov);
    }
    Some(builder)
}

/// Walks `roots` (already filtered to existing, non-stdin paths) in
/// parallel, scanning every file. Returns findings and whether any I/O
/// error occurred.
fn scan_roots(
    roots: &[PathBuf],
    overrides: Option<Override>,
    today: Date,
    warn_until: Option<Date>,
    tags: &[String],
) -> (Vec<Finding>, bool) {
    let Some(builder) = walk_builder(roots, overrides) else {
        return (Vec::new(), false);
    };

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
fn list_file_paths(roots: &[PathBuf], overrides: Option<Override>) -> (Vec<String>, bool) {
    let Some(builder) = walk_builder(roots, overrides) else {
        return (Vec::new(), false);
    };

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

/// Writes to stdout, turning the failures that are real failures into the
/// exit code the contract gives them. `Ok(())` means the output either
/// completed or was cut short by a reader that stopped reading; see
/// `stdout::write` for why the second one is not an error.
fn write_stdout(f: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> Result<(), ExitCode> {
    stdout::write(f).map_err(|err| {
        eprintln!("todo-by: {err}");
        ExitCode::from(2)
    })
}

/// Prints `text` and ends the process, the way `--help` and `--version`
/// do: nothing after them depends on the rest of parsing. Exits 2 if the
/// text could not be written for a reason other than a closed reader,
/// since a caller reading `--version` into a variable must not be handed
/// an empty string and a success.
fn print_and_exit(text: &str) -> ! {
    let code = match write_stdout(|w| write!(w, "{text}")) {
        Ok(()) => 0,
        Err(_) => 2,
    };
    std::process::exit(code);
}

fn main() -> ExitCode {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("todo-by: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    // Before anything that can fail or find something, so the notice is
    // not buried under findings, and on stderr so it never lands in
    // --files output or a JSON stream.
    for notice in &cli.deprecated {
        eprintln!("todo-by: {notice}");
    }

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

    let online = resolve_online(cli.online, cfg.online);

    if cli.dump_config {
        let effective = config::Config {
            warn,
            online: Some(online),
            ..cfg
        };
        if let Err(code) = write_stdout(|w| write!(w, "{}", config::dump(&effective))) {
            return code;
        }
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
    for notice in skipped_root_notices(&fs_paths) {
        eprintln!("todo-by: {notice}");
    }

    if cli.files {
        let (paths, walk_error) = list_file_paths(&fs_paths, overrides);
        if let Err(code) = write_stdout(|w| {
            for p in &paths {
                writeln!(w, "{p}")?;
            }
            Ok(())
        }) {
            return code;
        }
        return if had_error || walk_error {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }

    let (mut findings, walk_error) = scan_roots(&fs_paths, overrides, today, warn_until, &cfg.tags);
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
    let mut current_version: Option<String> = None;
    if needs_version_resolution(&findings) {
        // An empty value is treated as unset rather than as a version:
        // `TODO_BY_VERSION: ${{ inputs.version }}` with no input expands to
        // the empty string, and honoring that would outrank `version-cmd`
        // and git and then fail the run outright, so the documented
        // fallback ladder would never be reached.
        let env_version = std::env::var("TODO_BY_VERSION")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let source = choose_version_source(
            cli.current_version.as_deref(),
            env_version.as_deref(),
            cfg.version_cmd.as_deref(),
        );
        let label = source.label();
        let config_run_dir = version_run_dir(cfg.source.as_deref(), &start_dir);
        match resolve_current_version(source, config_run_dir, &start_dir)
            .and_then(|raw| parse_current_version(&raw, &label))
        {
            Ok(current) => {
                resolve_version_candidates(&mut findings, &current);
                current_version = Some(current.to_string());
            }
            // A failure here must not swallow the findings that don't
            // depend on it. An overdue date is already fully classified,
            // and hiding it because `git describe` found no tag would let
            // an unrelated setup problem mask a real deadline. Report the
            // error, drop only the candidates that can't be judged without
            // a version, and render the rest; the run still exits 2.
            Err(err) => {
                eprintln!("todo-by: {err}");
                had_error = true;
                findings.retain(|f| !matches!(f.kind, scanner::Kind::VersionPending { .. }));
            }
        }
    }

    // Sorted BEFORE issue resolution, not just before rendering: findings
    // arrive in whatever order the parallel walk produced them, and issue
    // resolution batches references in that order. Sorting first makes the
    // batches, and therefore which answers survive a failure, the same on
    // every run over the same tree. Version resolution is order-independent
    // and does not care either way.
    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));

    // Same laziness contract as versions, with a stronger reason: this is
    // the only code path in the tool that touches the network, so it runs
    // only when the scan found an issue tag AND the run is online.
    let pending_issues = findings
        .iter()
        .filter(|f| matches!(f.kind, scanner::Kind::IssuePending { .. }))
        .count();
    if pending_issues > 0 {
        if online {
            let refs: Vec<issue::Reference> = findings
                .iter()
                .filter_map(|f| match &f.kind {
                    scanner::Kind::IssuePending { reference, .. } => Some(reference.clone()),
                    _ => None,
                })
                .collect();
            // A request-level failure says nothing about any single
            // reference, so it is reported once per host rather than once
            // per tag. Whatever was answered before it is still applied:
            // those findings are true, and the run exits 2 either way.
            let (outcomes, failures) = issue::resolve(&refs, cfg.repo.as_ref(), &start_dir);
            for err in &failures {
                eprintln!("todo-by: {err}");
            }
            had_error = had_error || !failures.is_empty();
            resolve_issue_candidates(&mut findings, outcomes);
        } else {
            // Not an error: the trigger is opt-in, so "not checked" is the
            // configured state. It is still said out loud, because a tag
            // that silently never fires is the failure this tool exists to
            // prevent.
            eprintln!(
                "todo-by: {pending_issues} issue tag{} not checked (pass --online to check GitHub)",
                if pending_issues == 1 { "" } else { "s" }
            );
            findings.retain(|f| !matches!(f.kind, scanner::Kind::IssuePending { .. }));
        }
    }

    let opts = RenderOpts {
        format,
        color,
        today,
        current_version,
    };
    if let Err(code) = write_stdout(|w| output::render(w, &findings, &opts)) {
        return code;
    }

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

    #[test]
    fn version_run_dir_prefers_config_dir_over_start_dir() {
        // This directory feeds version-cmd only (see
        // resolve_current_version); git-describe's default deliberately
        // does not use it, covered separately below.
        let start = Path::new("/work/repo/src");
        assert_eq!(
            version_run_dir(Some(Path::new("/work/repo/todo-by.toml")), start),
            Path::new("/work/repo")
        );
        assert_eq!(version_run_dir(None, start), start);
    }

    /// Initializes a throwaway git repo at `dir` with one commit and one
    /// tag, so `git describe --tags --abbrev=0` run there has something
    /// deterministic to find.
    fn init_git_repo_with_tag(dir: &Path, tag: &str) {
        let run = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "todo-by-test")
                .env("GIT_AUTHOR_EMAIL", "todo-by-test@example.com")
                .env("GIT_COMMITTER_NAME", "todo-by-test")
                .env("GIT_COMMITTER_EMAIL", "todo-by-test@example.com")
                .output()
                .expect("git must be installed to run this test");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["commit", "-q", "--allow-empty", "-m", "init"]);
        // Explicit -a -m rather than a bare `git tag <name>`: some global
        // git configs default a bare tag to annotated and then fail
        // without a message, or vice versa. Being explicit sidesteps both.
        run(&["tag", "-a", tag, "-m", tag]);
    }

    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("todo-by-main-test-{nanos}-{n}-{tag}"));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    #[test]
    fn git_default_resolves_against_the_invocation_dir_not_the_config_dir() {
        // Regression for a bug where GitDefault inherited version-cmd's
        // config-dir anchoring: config discovery walks upward from the
        // invocation directory, so the config file can legitimately live
        // above the actual repository (a monorepo layout). `config_dir`
        // here stands in for exactly that: it is NOT a git repository at
        // all, so if git described anchored there by mistake, this would
        // fail instead of returning the repo's real tag.
        let config_dir = unique_temp_dir("git-default-config-dir");
        let repo_dir = config_dir.join("project");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_git_repo_with_tag(&repo_dir, "v9.9.9");

        let raw = resolve_current_version(VersionSource::GitDefault, &config_dir, &repo_dir)
            .expect("git describe must succeed in the invocation dir's own repository");
        assert_eq!(raw, "v9.9.9");

        std::fs::remove_dir_all(&config_dir).ok();
    }

    fn version_pending(written: &str, message: &str) -> Finding {
        Finding {
            file: "a.rs".to_string(),
            line: 1,
            kind: scanner::Kind::VersionPending {
                written: written.to_string(),
                constraint: version::Constraint::parse(written).unwrap(),
            },
            message: message.to_string(),
        }
    }

    #[test]
    fn resolve_version_candidates_promotes_satisfied_and_drops_unsatisfied() {
        let mut findings = vec![
            version_pending(">=v2.0", "satisfied"),
            version_pending(">=v999.0", "not yet"),
        ];
        let current = Version::parse("2.1.0").unwrap();
        resolve_version_candidates(&mut findings, &current);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].message, "satisfied");
        match &findings[0].kind {
            scanner::Kind::VersionReached { written } => assert_eq!(written, ">=v2.0"),
            _ => panic!("expected VersionReached"),
        }
    }

    #[test]
    fn resolution_is_skipped_when_no_candidates_are_present() {
        // Calls the guard main() actually calls, rather than restating its
        // condition here: an inlined copy would keep passing if the real
        // guard were deleted, which is exactly the regression that would
        // make a date-only tree start running git.
        let invalid_only = [Finding {
            file: "a.rs".to_string(),
            line: 1,
            kind: scanner::Kind::InvalidTrigger {
                written: "<v1.0".to_string(),
            },
            message: "old".to_string(),
        }];
        // An InvalidTrigger finding is already fully classified, so it must
        // not drag a subprocess in behind it.
        assert!(!needs_version_resolution(&invalid_only));
        assert!(!needs_version_resolution(&[]));
        assert!(needs_version_resolution(&[version_pending(
            ">=v2.0",
            "candidate"
        )]));
    }

    #[test]
    fn resolved_versions_take_one_optional_v_prefix() {
        // Lenient about case, since the string comes from a git tag or a
        // command rather than from a tag someone wrote here.
        for raw in ["2.0", "v2.0", "V2.0"] {
            assert_eq!(
                parse_current_version(raw, "--current-version")
                    .unwrap()
                    .to_string(),
                "2.0",
                "{raw:?}"
            );
        }
        // But one prefix only: Version::parse strips a lowercase v too, so
        // these would otherwise each shed both and be accepted.
        for raw in ["vv2.0", "Vv2.0", "vV2.0"] {
            assert!(
                parse_current_version(raw, "--current-version").is_err(),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn git_describe_markers_do_not_sink_the_version_below_its_tag() {
        // Being past the tag must not read as not having reached it. Semver
        // sorts a pre-release below its release, so without the strip these
        // would each compare LESS than 1.2.3 and a `>=v1.2.3` tag would
        // produce no finding at all.
        for raw in [
            "v1.2.3-4-gabc123",
            "1.2.3-4-gabc123",
            "v1.2.3-4-gabc123-dirty",
            "v1.2.3-dirty",
        ] {
            let parsed = parse_current_version(raw, "--current-version").unwrap();
            assert_eq!(parsed.to_string(), "1.2.3", "{raw:?}");
            assert!(
                crate::version::Constraint::parse(">=v1.2.3")
                    .unwrap()
                    .satisfied_by(&parsed),
                "{raw:?}"
            );
        }

        // A real pre-release keeps its suffix, including when describe ran
        // on a pre-release tag: falling back to 1.2.3 there would claim a
        // release the project hasn't cut.
        for (raw, expected) in [
            ("1.2.3-rc.1", "1.2.3-rc.1"),
            ("v1.2.3-rc.1-4-gabc123", "1.2.3-rc.1"),
            // Not describe output: no `g`, and a non-hex "sha".
            ("1.2.3-4-abc123", "1.2.3-4-abc123"),
            ("1.2.3-4-gxyz", "1.2.3-4-gxyz"),
        ] {
            assert_eq!(
                parse_current_version(raw, "--current-version")
                    .unwrap()
                    .to_string(),
                expected,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn version_cmd_runs_in_the_given_directory_and_reports_failures() {
        // The directory argument is the whole point of the config-anchoring
        // fix: a relative path inside version-cmd must resolve against the
        // config file, not the invocation directory. Without a test, that
        // argument can be swapped back with every other test still green.
        let dir = std::env::temp_dir().join("todo-by-version-cmd-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("VERSION"), "4.2.0\n").unwrap();

        let cmd = if cfg!(windows) {
            "type VERSION"
        } else {
            "cat VERSION"
        };
        assert_eq!(run_version_cmd(cmd, &dir).unwrap(), "4.2.0");

        // Same command one level up, where the file doesn't exist: the
        // failure surfaces instead of silently yielding an empty version.
        let parent = dir.parent().unwrap();
        assert!(run_version_cmd("cat todo-by-no-such-version-file", parent).is_err());

        // Non-zero exit and empty stdout are both errors, not versions.
        let err = run_version_cmd("exit 3", &dir).unwrap_err();
        assert!(err.contains("exit"), "unexpected message: {err}");
        assert!(run_version_cmd("true", &dir).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn issue_pending(written: &str) -> Finding {
        Finding {
            file: "f".to_string(),
            line: 1,
            kind: scanner::Kind::IssuePending {
                written: written.to_string(),
                reference: issue::Reference::parse(written).expect("valid reference"),
            },
            message: "msg".to_string(),
        }
    }

    #[test]
    fn a_valueless_flag_rejects_an_inline_value() {
        for arg in [
            "--online=false",
            "--offline=true",
            "--exit-zero=0",
            "--hidden=no",
            "--files=x",
            "--dump-config=x",
        ] {
            let err = parse_args(args(&[arg])).expect_err("rejected");
            assert!(err.contains("takes no value"), "{arg}: {err}");
        }
        // The bare forms still work. `--hidden` no longer sets anything,
        // but it must still parse: an existing CI invocation carrying it
        // has to keep running, and it has to keep being a valueless flag.
        let cli = parse_args(args(&["--online", "--exit-zero", "--hidden"])).expect("valid");
        assert_eq!(cli.online, Some(true));
        assert!(cli.exit_zero);
        assert_eq!(cli.paths, vec![PathBuf::from(".")]);
    }

    #[test]
    fn a_retired_flag_is_accepted_and_reported_once() {
        // Accepted, so a pinned CI job keeps running; reported, so the
        // removal at 1.0 is not the first thing anyone hears about it.
        let quiet = parse_args(args(&["--exit-zero"])).expect("valid");
        assert!(quiet.deprecated.is_empty(), "a normal run must stay silent");

        let cli = parse_args(args(&["--hidden", "--hidden"])).expect("valid");
        assert_eq!(cli.deprecated.len(), 1, "repeats collapse into one notice");
        let notice = cli.deprecated[0];
        assert!(notice.contains("--hidden"), "{notice}");
        assert!(notice.contains("1.0"), "must say when it goes: {notice}");
    }

    #[test]
    fn online_flags_win_over_the_config_in_both_directions() {
        assert!(!resolve_online(None, None));
        assert!(resolve_online(None, Some(true)));
        assert!(resolve_online(Some(true), None));
        assert!(resolve_online(Some(true), Some(false)));
        assert!(!resolve_online(Some(false), Some(true)));
    }

    #[test]
    fn issue_outcomes_apply_in_collection_order() {
        let mut findings = vec![
            issue_pending("#1"),
            version_pending(">=v9.0", "kept"),
            issue_pending("#2"),
            issue_pending("#3"),
        ];
        resolve_issue_candidates(
            &mut findings,
            vec![
                issue::Outcome::Fired {
                    state: "closed",
                    url: "https://github.com/o/r/issues/1".to_string(),
                },
                // The version finding in between must not consume an
                // outcome: the two candidate kinds are resolved separately.
                issue::Outcome::Open,
                issue::Outcome::Failed("no access".to_string()),
            ],
        );
        assert_eq!(findings.len(), 3);
        assert!(matches!(
            &findings[0].kind,
            scanner::Kind::IssueClosed { written, state, url }
                if written == "#1" && *state == "closed" && url.ends_with("/issues/1")
        ));
        assert!(matches!(
            &findings[1].kind,
            scanner::Kind::VersionPending { .. }
        ));
        assert!(matches!(
            &findings[2].kind,
            scanner::Kind::IssueError { written, detail } if written == "#3" && detail == "no access"
        ));
    }

    #[test]
    fn a_short_outcome_list_drops_the_unanswered_candidates() {
        let mut findings = vec![issue_pending("#1"), issue_pending("#2")];
        resolve_issue_candidates(&mut findings, vec![issue::Outcome::Open]);
        assert!(findings.is_empty());
    }

    #[test]
    fn answers_collected_before_a_failure_survive_it() {
        // A request that failed after an earlier one succeeded must not take
        // the earlier answer down with it: that finding is true, and the run
        // already reports the failure and exits 2.
        let mut findings = vec![issue_pending("#1"), issue_pending("#2")];
        resolve_issue_candidates(
            &mut findings,
            vec![
                issue::Outcome::Fired {
                    state: "closed",
                    url: "https://github.com/o/r/issues/1".to_string(),
                },
                issue::Outcome::Unanswered,
            ],
        );
        assert_eq!(findings.len(), 1);
        assert!(matches!(
            &findings[0].kind,
            scanner::Kind::IssueClosed { written, .. } if written == "#1"
        ));
    }

    /// Every file the walk should reach carries a tag, so a findings set
    /// and a file list can be compared directly. Every file it should not
    /// reach carries one too, so a leak announces itself instead of just
    /// widening a count.
    fn walk_fixture(tag: &str) -> PathBuf {
        let root = unique_temp_dir(tag);
        let write = |rel: &str, body: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        write("visible.txt", "// todo-by 2998-01-01 plain file\n");
        write(
            ".github/workflows/ci.yml",
            "# todo-by 2998-01-01 unpin the action\n",
        );
        write(
            ".gitignore",
            "# todo-by 2998-01-01 drop this\nignored.txt\n*.bak\n",
        );
        write(
            "worktree/kept.txt",
            "// todo-by 2998-01-01 beside a .git file\n",
        );
        write("worktree/.git", "gitdir: ../.git/worktrees/wt\n");
        write("ignored.txt", "# todo-by 2998-01-01 never read\n");
        // Hidden *and* gitignored: being hidden stopped mattering, being
        // ignored did not.
        write(".gitignore.bak", "# todo-by 2998-01-01 ignored by glob\n");
        write(
            ".git/COMMIT_EDITMSG",
            "fix: drop the todo-by 2998-01-01 tag\n",
        );
        write(".git/logs/HEAD", "0000 1111 commit: todo-by 2998-01-01\n");
        // The same hazard in the layouts git is not the only one to have:
        // a stored commit message, and a whole copy of a tracked file.
        write(
            ".hg/last-message.txt",
            "fix: drop the todo-by 2998-01-01 tag\n",
        );
        write(
            ".svn/pristine/aa/aabbcc.svn-base",
            "// todo-by 2998-01-01 plain file\n",
        );
        write(".jj/repo/store/op_heads", "op: todo-by 2998-01-01 stored\n");
        root
    }

    /// The fixture files left once the gitignored ones and everything
    /// named as, or held by, a metadata directory are gone.
    const WALK_FIXTURE_FILES: [&str; 4] = [
        ".github/workflows/ci.yml",
        ".gitignore",
        "visible.txt",
        "worktree/kept.txt",
    ];

    fn relative_to(root: &Path, paths: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut out: Vec<String> = paths
            .into_iter()
            .map(|p| {
                Path::new(&p)
                    .strip_prefix(root)
                    .unwrap_or(Path::new(&p))
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        out.sort();
        out
    }

    fn walked(roots: &[PathBuf], root: &Path, overrides: Option<Override>) -> Vec<String> {
        let (paths, had_error) = list_file_paths(roots, overrides);
        assert!(!had_error, "walk reported an I/O error");
        relative_to(root, paths)
    }

    fn scanned(roots: &[PathBuf], root: &Path, overrides: Option<Override>) -> Vec<String> {
        let today = Date::parse_full("2999-01-01").unwrap();
        let tags = vec!["todo-by".to_string()];
        let (findings, had_error) = scan_roots(roots, overrides, today, None, &tags);
        assert!(!had_error, "scan reported an I/O error");
        relative_to(root, findings.into_iter().map(|f| f.file))
    }

    #[test]
    fn the_walk_covers_hidden_files_and_never_enters_a_git_dir() {
        let root = walk_fixture("walk");
        let roots = vec![root.clone()];
        assert_eq!(
            walked(&roots, &root, None),
            WALK_FIXTURE_FILES,
            "unexpected file set"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scanning_reaches_exactly_the_files_that_files_advertises() {
        // `--files` exists to answer "what will be scanned", so the two
        // walks have to be the same walk. They were separate builders
        // once, and a filter added to one would not reach the other. Every
        // fixture file carries a tag, so the findings name every file the
        // scan actually read.
        let root = walk_fixture("scan");
        let roots = vec![root.clone()];
        assert_eq!(scanned(&roots, &root, None), walked(&roots, &root, None));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn every_metadata_dir_is_dropped_as_a_root_too() {
        // Each of these stores commit text, and .svn/pristine stores whole
        // copies of tracked files, so walking one turns a single tag into
        // a phantom finding or a duplicate of a real one.
        let root = walk_fixture("vcs-roots");
        for dir in VCS_DIRS {
            let path = root.join(dir);
            if !path.exists() {
                continue;
            }
            assert!(
                walked(std::slice::from_ref(&path), &root, None).is_empty(),
                "{dir} was walked when named as a root"
            );
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_pointing_into_a_metadata_dir_is_dropped_too() {
        // The root filter resolves before it matches, which no other test
        // needs: deleting the canonicalize call leaves the rest green.
        let root = walk_fixture("symlink");
        let link = root.join("shortcut");
        std::os::unix::fs::symlink(root.join(".git"), &link).unwrap();
        assert!(walked(std::slice::from_ref(&link), &root, None).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_file_named_on_the_command_line_is_scanned_even_when_gitignored() {
        // The walker never filters a root it was handed directly, which is
        // what lets an explicitly named path defeat .gitignore. Only the
        // metadata directories are exempt from that.
        let root = walk_fixture("named-file");
        for name in ["ignored.txt", ".gitignore.bak"] {
            let file = root.join(name);
            assert_eq!(
                walked(std::slice::from_ref(&file), &root, None),
                [name],
                "{name} named directly should still be scanned"
            );
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_dropped_root_is_reported_rather_than_silently_ignored() {
        let root = walk_fixture("skip-notice");
        assert!(
            skipped_root_notices(std::slice::from_ref(&root)).is_empty(),
            "an ordinary root must not be announced"
        );

        let git = root.join(".git");
        let notices = skipped_root_notices(&[root.clone(), git.clone(), git.join("hooks")]);
        assert_eq!(notices.len(), 2, "one per dropped root: {notices:?}");
        for notice in &notices {
            assert!(notice.starts_with("skipping "), "{notice}");
            assert!(notice.contains("never scanned"), "{notice}");
        }
        assert!(notices[0].contains(&git.display().to_string()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_git_dir_named_as_a_root_is_dropped_rather_than_walked() {
        // `ignore` applies filter_entry only from depth 1 down, so without
        // the separate root filter this is how .git gets walked anyway.
        let root = walk_fixture("git-root");
        let git = root.join(".git");
        assert!(walked(std::slice::from_ref(&git), &root, None).is_empty());
        assert!(walked(&[git.join("logs")], &root, None).is_empty());
        // Alongside a real root, the good root still produces its files
        // and the .git one contributes nothing.
        let both = vec![root.clone(), git];
        assert_eq!(walked(&both, &root, None), WALK_FIXTURE_FILES);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn every_root_past_the_first_is_walked_too() {
        // The roots after the first go through `WalkBuilder::add`, which a
        // single-root test leaves entirely unexercised.
        let root = walk_fixture("roots");
        let roots = vec![root.join("worktree"), root.join(".github")];
        assert_eq!(
            walked(&roots, &root, None),
            [".github/workflows/ci.yml", "worktree/kept.txt"]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn config_excludes_reach_the_walker_and_apply_to_hidden_files() {
        // Building the matcher is not the same as handing it to the
        // walker, and a hidden path is the case the exclude globs never
        // had to cover before.
        let root = walk_fixture("excludes");
        let roots = vec![root.clone()];
        let overrides = build_overrides(&root, &[".github/**".to_string()])
            .expect("valid pattern")
            .expect("some overrides");
        assert_eq!(
            walked(&roots, &root, Some(overrides)),
            [".gitignore", "visible.txt", "worktree/kept.txt"]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gitignore_applies_outside_a_repository() {
        // `require_git(false)` is what makes the walker usable on an
        // extracted tarball or a vendored tree. The fixture above has a
        // real `.git`, so it would pass either way.
        let root = unique_temp_dir("no-repo");
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "x\n").unwrap();
        std::fs::write(root.join("kept.txt"), "x\n").unwrap();
        assert_eq!(
            walked(std::slice::from_ref(&root), &root, None),
            [".gitignore", "kept.txt"]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_walk_with_no_usable_root_yields_nothing_instead_of_panicking() {
        assert_eq!(list_file_paths(&[], None), (Vec::new(), false));
    }

    #[test]
    fn hidden_is_absent_from_the_help_text() {
        // The flag is accepted but deliberately undocumented. Re-adding
        // the line would advertise a switch that changes nothing.
        assert!(!USAGE.contains("--hidden"));
    }
}
