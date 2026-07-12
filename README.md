# todo-by

[![CI](https://github.com/alies-dev/todo-by/actions/workflows/ci.yml/badge.svg)](https://github.com/alies-dev/todo-by/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/todo-by-cli.svg)](https://crates.io/crates/todo-by-cli)
[![license](https://img.shields.io/crates/l/todo-by-cli.svg)](LICENSE)

Flag `todo-by` tags whose deadline date has passed. Works on any file type. Tiny and lightning-fast. Respects your .gitignore.

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

## What it's for

Date-triggered chores rot in a backlog. "Remove once v2 ships" becomes a ticket nobody reopens, disconnected from the code it was about. `todo-by` welds the reminder to that code and lets the date, not a person, decide when it comes due.

Reach for a tag when the task is:

- **Small.** Anyone can finish it in a minute or two with zero extra context.
- **Mechanical.** A cleanup (delete, revert, re-enable), not new work to design.
- **Triggered.** It comes due on a date, a released version, or a downstream change.

If it needs an owner or a conversation, use a real tracker instead. `todo-by` is the layer beneath the tracker, for the small stuff a tracker would only bury.

## Installation

Homebrew (macOS, Linux):

```console
brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
brew install alies-dev/todo-by/todo-by
```

Cargo:

```console
cargo install todo-by-cli
```

Or grab a prebuilt binary from [Releases](https://github.com/alies-dev/todo-by/releases).

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

## Triggers

### Dates

Three precisions are supported. A tag becomes overdue the day its deadline is reached.

| Written as | Deadline |
|---|---|
| `2026-09-01` | that day |
| `2026-09` | last day of that month |
| `2026` | December 31 of that year |

Impossible dates (for example `2026-02-30`) are reported as findings too, so typos cannot silently postpone a deadline forever.

#### Warn ahead

`--warn N` reports tags due within N days as warnings rather than errors, so a deadline surfaces in CI before it starts failing the build. It still exits 0.

```console
$ todo-by --warn 14
src/legacy.rs:8: due in 5 days (2026-07-14): drop the feature flag
1 warning
```

In `--format github`, warnings render as `::warning` annotations instead of `::error`.

## CI (GitHub Actions)

Download the prebuilt static (musl) binary, verify its checksum, and run it. No Rust toolchain and no compile step, so the job finishes in about a second. Pin the version and its checksum with the two variables; both come from the release's `sha256.sum`.

```yaml
name: todo-by
on: [push, pull_request]

jobs:
  todo-by:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      - name: Check overdue todo-by tags
        env:
          TODOBY_VERSION: v0.2.1
          TODOBY_SHA256: 2a2d0396a592a16ab211604fdb1e860586676a1a0785a9c89cbfb377fe9d9234
        run: |
          ASSET="todo-by-cli-x86_64-unknown-linux-musl.tar.xz"
          curl --proto '=https' --tlsv1.2 -sSfL \
            "https://github.com/alies-dev/todo-by/releases/download/${TODOBY_VERSION}/${ASSET}" -o /tmp/todo-by.tar.xz
          echo "${TODOBY_SHA256}  /tmp/todo-by.tar.xz" | sha256sum -c -
          tar -xJf /tmp/todo-by.tar.xz -C /tmp
          /tmp/todo-by-cli-x86_64-unknown-linux-musl/todo-by
```

On a codebase with existing overdue tags, phase it in with `continue-on-error: true` on the step, or `todo-by --warn N --exit-zero` so deadlines surface without failing the build. Shorter but less strict: the release also ships an installer script (`curl ... todo-by-cli-installer.sh | sh`). Other methods (`cargo install todo-by-cli --locked`, Homebrew) work too. See [Installation](#installation).

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

## Roadmap

- Package version trigger (`todo-by >=2.0`)
- GitHub issue closed trigger (`todo-by #123`)

## Prior art

Inspired by [phpstan/phpstan-todo-by](https://github.com/staabm/phpstan-todo-by) by Markus Staab, which does this (and more: package version and issue triggers) for PHP files as a PHPStan extension. `todo-by` trades those triggers for working on any file type with no runtime.

## License

MIT.
