# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Issue triggers: a tag can fire when a GitHub issue or pull request closes. Write `#123` for the repository the git remote points at, or a full issue or pull request URL for any other repository or host. `owner/repo#123` is not supported, since the URL already says the same thing and can also name a host. Any state other than open fires, `merged` included; the close reason is never requested and never inspected.
- The issue spellings GitHub itself renders as links (`owner/repo#123`, `repo#123`, `GH-123`) are reported rather than ignored, each naming the accepted spelling; the cross repository form quotes the exact URL to write instead. A token carrying neither marker (a `#` with a digit behind it, or the `GH-` form) stays prose, so a project matching on `todo` does not have every undated TODO reported.
- Issue URLs may carry a fragment or a query, so a comment permalink pasted straight from GitHub (`.../issues/123#issuecomment-456`) names issue 123 rather than being ignored as prose.
- `--online` (and the `online` config key, with `--offline` to override it) gates every network call. Without it, issue tags are left unchecked and reported once on stderr, with no findings and no change to the exit code. The check runs only when the scan actually found an issue tag, so a tree of date tags stays hermetic.
- Issue lookups go through `curl` when `GH_TOKEN` or `GITHUB_TOKEN` is set and the target is github.com (the token travels in curl's config file on stdin, never in argv or on disk, with a timeout of 30 seconds), and through `gh` otherwise, which authenticates from its own keyring. An environment token is never sent to any other host, since a tag names its own host and a tag is repository content. References are batched into one GraphQL request per host, up to 100 at a time.
- `repo` config key (`owner/name`): the repository bare `#123` references resolve against. Without it the git remote decides, and two remotes that disagree (a fork checkout) are reported rather than guessed at.
- `online` and `repo` config keys, plus boolean values in the config parser.

### Changed

- Minimum supported Rust version raised from 1.85 to 1.88. The `ignore` dependency declares `rust-version = "1.88"` from 0.4.31 on, so 1.85 no longer resolves a working dependency set.

### Fixed

- A trigger written flush against an HTML comment closer, with no space before the `-->`, left a stray `->` at the head of the finding's message. The closer is now stripped from the message the way it was already kept out of the trigger span.

## [0.3.0] - 2026-07-26

### Added

- Version triggers: a tag can fire when the project reaches a version instead of on a date. Write the version with a lowercase `v` (`v2.0`, or `v2026.01` for calendar versions) to mean "that version or later", or with an explicit comparator (`>=v2.0`, `>v2.0`). `<`, `<=`, `=`, `==`, `^`, and `~` are recognized and reported as invalid rather than silently ignored, since this tool cannot fire on a version that is never released or one held below a ceiling.
- Current version resolution, run once per scan and only when a version tag is actually found: `--current-version`, then `TODO_BY_VERSION`, then the `version-cmd` config key (a shell command, run in the config file's directory), then `git describe --tags --abbrev=0`. A resolved version keeps only its tag: the markers `git describe` adds for commits past the tag (`-4-gabc123`) and for a dirty tree (`-dirty`) are stripped, since semver reads them as a pre-release sorting below the release they came from, which would make a commit past `v1.2.3` count as never having reached it.
- `version-cmd` config key, for projects whose version lives in `package.json`, `composer.json`, or anywhere else git tags do not cover.

### Changed

- **Breaking.** A year on its own (a tag reading just `2026`) is no longer a deadline meaning December 31 of that year. Deadlines now need at least a month, written with dashes (`2026-12`). A bare digit-leading token reads as a version constraint now, and a lone year cannot be told apart from a one-component version. Existing year-only tags are reported as errors naming both replacements (`2026-12` for the deadline, `v2026` for the version), so upgrading surfaces every one of them instead of quietly changing when they fire.
- **Breaking.** A version written in a tag needs a lowercase `v`. A bare number (`2.0`, `>=2.0`, `2026.09.01`) is no longer a version trigger, and is reported as an error quoting the marked spelling to write instead. The marker is what separates a version from a date, and without it a tag sitting in prose reads two ways at once: `2026.09.01` is both a dotted deadline and a calendar version, `12.5.2026` is both a day-first date and a three-component version, `3.5` is both a constraint and the start of "3.5 hours of work". Guessing wrong on any of those fails silently, since a constraint the project never reaches produces no finding at all, which would bury the chore instead of surfacing it. Dates keep their own marking, the dashes they always had.
- The current version is unaffected by that rule: `--current-version`, `TODO_BY_VERSION`, `version-cmd`, and `git describe` all still accept a bare `1.2.3`, since those strings come from the project rather than from a tag author.

## [0.2.1] - 2026-07-12

### Added

- Homebrew install: `brew install alies-dev/todo-by/todo-by`. The repository doubles as its own tap, and the formula is regenerated from the release checksums after each release.
- Declared minimum supported Rust version (`rust-version = "1.85"`), enforced in CI.

## [0.2.0] - 2026-07-10

### Added

- `--warn <days>` (or `warn = N` in the config file): tags due within N days are reported as warnings (yellow in text output, `::warning` annotations in GitHub Actions) and exit 0, so deadlines appear in CI before they start failing it.
- `--exit-zero`: report-only mode for scheduled inventory jobs and gradual adoption.
- `--format json`: JSON Lines output, one object per finding plus a trailing summary record. The schema is additive-stable.
- GitHub Actions auto-detect: when `GITHUB_ACTIONS=true` and no `--format` is given, the github format is selected automatically.
- `--color auto|always|never`, honoring `NO_COLOR`, `TERM=dumb`, and TTY detection.
- stdin scanning: `todo-by -` scans standard input (for example `git diff | todo-by -`).
- Config file `todo-by.toml` (or `.todo-by.toml`), discovered from the current directory upward. Keys: `warn`, `exclude` (gitignore-style globs on top of `.gitignore`), `tags` (replaces the default tag list). Precedence: flags, then `TODO_BY_FORMAT` / `TODO_BY_WARN`, then the config file.
- Introspection flags: `--files` lists what would be scanned, `--dump-config` prints the effective config and its source.

### Changed

- The exit code contract is now documented: 0 clean (warnings alone stay 0), 1 findings, 2 usage, config, or IO error.

## [0.1.0] - 2026-07-09

Initial release.

### Added

- Scanner for `todo-by <date>` tags in any file type: byte-level, case-insensitive, no language grammars.
- Three date precisions: `2026` (due Dec 31), `2026-09` (due last day of month), `2026-09-01`. Impossible dates such as `2026-02-30` are reported as `invalid-date` findings so typos cannot silently postpone a deadline.
- Parallel directory walking with full gitignore semantics (nested `.gitignore` files, negation, `**` globs), also outside a git repository. Hidden files, binaries, and symlinks are skipped; explicitly named files are always scanned.
- Output formats: `text` for humans, `github` for workflow annotations.
- Exit codes: 0 clean, 1 findings, 2 error.
- `--today` to override the clock for testing and dry runs.

[Unreleased]: https://github.com/alies-dev/todo-by/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/alies-dev/todo-by/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/alies-dev/todo-by/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/alies-dev/todo-by/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/alies-dev/todo-by/releases/tag/v0.1.0
