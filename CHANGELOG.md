# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Version triggers: a tag can fire when the project reaches a version instead of on a date. Write the version bare (a tag reading `2.0`, or `v2026.01` for calendar versions) to mean "that version or later", or with an explicit comparator (`>=2.0`, `>2.0`). `<`, `<=`, `=`, and `==` are recognized and reported as invalid rather than silently ignored, since this tool cannot fire on a version that is never released.
- Current version resolution, run once per scan and only when a version tag is actually found: `--current-version`, then `TODO_BY_VERSION`, then the `version-cmd` config key (a shell command, run in the config file's directory), then `git describe --tags --abbrev=0`.
- `version-cmd` config key, for projects whose version lives in `package.json`, `composer.json`, or anywhere else git tags do not cover.

### Changed

- **Breaking.** A year on its own (a tag reading just `2026`) is no longer a deadline meaning December 31 of that year. Deadlines now need at least a month, written with dashes (`2026-12`). A bare digit-leading token reads as a version constraint now, and a lone year cannot be told apart from a one-component version. Existing year-only tags are reported as errors naming both replacements (`2026-12` for the deadline, `v2026` for the version), so upgrading surfaces every one of them instead of quietly changing when they fire.
- Dotted dates (`2026.09.01`) are read as versions rather than as malformed dates. Dates use dashes.
- **Breaking.** More broadly, any digit-leading token containing a dot is now a version constraint, so text that used to be ignored can become a finding. A tag reading `3.5 hours of work left` is a constraint on version 3.5, and `2.5x speedup` is an invalid one. Prose belongs after the trigger, not in its place.

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

[Unreleased]: https://github.com/alies-dev/todo-by/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/alies-dev/todo-by/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/alies-dev/todo-by/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/alies-dev/todo-by/releases/tag/v0.1.0
