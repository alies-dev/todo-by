# todo-by

[![CI](https://github.com/alies-dev/todo-by/actions/workflows/ci.yml/badge.svg)](https://github.com/alies-dev/todo-by/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/todo-by-cli.svg)](https://crates.io/crates/todo-by-cli)
[![license](https://img.shields.io/crates/l/todo-by-cli.svg)](LICENSE)

Flag `todo-by` tags whose deadline date has passed. Works on any file type.

## Idea

Tag any comment with a deadline date:

```js
// @todo-by 2026-09-01 - Remove this legacy controller once signed URLs ship
```

```yaml
# todo-by 2026-09 drop the legacy webhook once v2 ships
```

`todo-by` scans the tree, validates each date, and exits non-zero when a deadline has passed, so it gates CI. It recognizes the tag in any comment style (docblocks, `//`, `#`, `--`, HTML, and so on) because it works on plain text, not language grammars.

```console
$ todo-by
config/legacy.yml:42: overdue since 2026-06-26: drop the legacy webhook once v2 ships
1 finding
```

## Dates

Three precisions are supported. A tag becomes overdue the day its deadline is reached.

| Written as | Deadline |
|---|---|
| `2026-09-01` | that day |
| `2026-09` | last day of that month |
| `2026` | December 31 of that year |

Impossible dates (for example `2026-02-30`) are reported as findings too, so typos cannot silently postpone a deadline forever.

## Usage

```console
todo-by [PATHS]...              # scan paths (default: current dir)
todo-by -                       # scan stdin as a single file (e.g. git diff | todo-by -)
todo-by --format text           # human-readable (default)
todo-by --format github         # GitHub Actions annotations
todo-by --format json           # JSON Lines, one object per finding
todo-by --today 2026-12-31      # override "now" (useful for testing and CI dry runs)
todo-by --warn 14               # also report tags due within 14 days, as warnings
todo-by --exit-zero             # always exit 0 on findings (still 2 on errors)
todo-by --color always          # auto, always, never (default: auto)
todo-by --hidden                # also scan hidden files and directories
todo-by --files                 # list files that would be scanned, then exit
todo-by --dump-config           # print effective config, then exit
```

Exit codes: `0` no findings (warnings alone still exit 0), `1` findings, `2` usage, config, or I/O error.

### Warn ahead

`--warn N` reports tags due within N days as warnings rather than errors, so a deadline surfaces in CI before it starts failing the build. It still exits 0.

```console
$ todo-by --warn 14
src/legacy.rs:8: due in 5 days (2026-07-14): drop the feature flag
1 warning
```

In `--format github`, warnings render as `::warning` annotations instead of `::error`.

### JSON output

`--format json` prints JSON Lines: one object per finding, followed by a trailing summary record.

```console
$ todo-by --format json
{"type":"finding","kind":"overdue","path":"src/lib.rs","line":12,"date":"2000-01-01","deadline":"2000-01-01","days_overdue":4,"message":"remove workaround"}
...
{"type":"summary","findings":2,"warnings":1}
```

The schema is additive-stable: fields keep their meaning across releases, and new ones may be added, but none are removed within a major version.

### CI (GitHub Actions)

`GITHUB_ACTIONS=true` is set automatically by GitHub Actions. When it is set and no `--format` flag is given, `todo-by` auto-selects `--format github`, so a bare invocation is enough.

```yaml
- run: todo-by
```

## What gets scanned

Everything git would track. `todo-by` uses ripgrep's directory walker, so `.gitignore` files are honored with full git semantics (nested files, negation, `**` globs, `.git/info/exclude`), including outside a git repository. Hidden files, binary files, and symlinks are skipped; pass `--hidden` to include hidden files. A file named explicitly on the command line is always scanned. The config file's `exclude` patterns are applied on top of `.gitignore`, using the same glob syntax.

## Configuration

`todo-by.toml` (or `.todo-by.toml`) is discovered by searching from the current directory upward; the first file found wins.

```toml
warn = 14
exclude = ["vendor/**", "*.gen.go"]
tags = ["todo-by", "fixme-by"]
```

- `warn` (integer): same as `--warn`.
- `exclude` (array of strings): gitignore-style globs excluded in addition to `.gitignore`. Globs are matched relative to the directory where `todo-by` runs, like ripgrep's `--glob`.
- `tags` (array of strings): tags to match, case-insensitive. Setting this replaces the default (`todo-by`) entirely rather than adding to it.

Precedence: command line flags win, then the `TODO_BY_FORMAT` / `TODO_BY_WARN` environment variables, then the config file.

Use `--dump-config` to see the effective config and where it came from, and `--files` to see which files would be scanned.

## Design goals

A single small static binary and boring, predictable behavior. Scanning is parallel across cores: a real-world repo like `angular/angular` (about 10,000 files) scans in roughly 0.2 seconds on an Apple M4 Max.

## Roadmap

- More triggers beyond dates: package versions (`todo-by >=2.0`), GitHub issues closed (`todo-by #123`), and similar
- Prebuilt binaries, Homebrew formula, Composer bin plugin, GitHub Action

## Prior art

Inspired by [phpstan/phpstan-todo-by](https://github.com/staabm/phpstan-todo-by) by Markus Staab, which does this (and more: package version and issue triggers) for PHP files as a PHPStan extension. `todo-by` trades those triggers for working on any file type with no runtime.

## License

MIT.
