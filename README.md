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
todo-by [PATHS]...          # scan paths (default: current dir)
todo-by --format text       # human-readable (default)
todo-by --format github     # GitHub Actions annotations
todo-by --today 2026-12-31  # override "now" (useful for testing and CI dry runs)
todo-by --hidden            # also scan hidden files and directories
```

Exit codes: `0` no findings, `1` findings, `2` usage or I/O error.

### CI (GitHub Actions)

```yaml
- run: todo-by --format github
```

## What gets scanned

Everything git would track. `todo-by` uses ripgrep's directory walker, so `.gitignore` files are honored with full git semantics (nested files, negation, `**` globs, `.git/info/exclude`), including outside a git repository. Hidden files, binary files, and symlinks are skipped; pass `--hidden` to include hidden files. A file named explicitly on the command line is always scanned.

## Design goals

A single small static binary and boring, predictable behavior. Scanning is parallel across cores: a large monorepo (about 13k tracked files) scans in under 0.2 seconds.

## Roadmap

- JSON output format
- `todo-by.toml` config: include/exclude globs, custom tag aliases, per-path rules
- Inline suppression (`todo-by:ignore`)
- More triggers beyond dates: package versions (`todo-by >=2.0`), GitHub issues closed (`todo-by #123`), and similar
- Warn-ahead mode (report tags due within N days before they fail CI)
- Prebuilt binaries, Homebrew formula, Composer bin plugin, GitHub Action

## License

MIT.
