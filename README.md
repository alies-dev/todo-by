<p align="center">
  <img src="https://raw.githubusercontent.com/alies-dev/todo-by/main/assets/banner.svg" alt="todo-by" width="620">
</p>

<p align="center">
  <a href="https://github.com/alies-dev/todo-by/actions/workflows/ci.yml"><img src="https://github.com/alies-dev/todo-by/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/todo-by-cli"><img src="https://img.shields.io/crates/v/todo-by-cli.svg" alt="crates.io"></a>
  <a href="LICENSE"><img src="https://img.shields.io/crates/l/todo-by-cli.svg" alt="license"></a>
</p>

Flag `todo-by` tags whose deadline has passed, whose target version has shipped, or whose GitHub issue has closed. Works on any file type. Tiny and lightning-fast. Respects your .gitignore.

## Idea

Tag any comment with a deadline. `todo-by` scans the tree and exits non-zero once one has passed, so it gates CI. It finds the tag in any comment style (docblocks, `//`, `#`, `--`, HTML) because it works on plain text, not language grammars.

```js
// @todo-by 2026-09-01 - Remove this legacy controller once signed URLs ship
# todo-by 2026-09 drop the legacy webhook once v2 ships
```

```console
$ todo-by
config/legacy.yml:42: overdue since 2026-06-26: drop the legacy webhook once v2 ships
1 finding
```

## What it's for

Date-triggered chores rot in a backlog. "Remove once v2 ships" becomes a ticket nobody reopens, disconnected from the code it was about. `todo-by` welds the reminder to that code and lets the trigger, not a person, decide when it comes due.

Reach for a tag when the task is **small** (a minute or two, no extra context), **mechanical** (delete, revert, re-enable, not new work to design), and **triggered** (a date, a released version, a downstream change). If it needs an owner or a conversation, use a real tracker. `todo-by` is the layer beneath the tracker, for the small stuff a tracker would only bury.

## Installation

Homebrew (macOS, Linux):

```console
brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
brew install alies-dev/todo-by/todo-by
```

Cargo:

```console
cargo install todo-by-cli --locked
```

`--locked` builds against the dependency versions this project tests, which are the ones the stated minimum Rust version is verified against. Without it, cargo resolves fresh versions that may need a newer compiler.

Or grab a prebuilt binary from [Releases](https://github.com/alies-dev/todo-by/releases).

The minimum supported Rust version is 1.88. It tracks the floor of the one dependency (`ignore`, ripgrep's directory walker) and can rise in any release, including a patch.

## Usage

```console
todo-by [PATHS]...              # scan paths (default: current dir)
todo-by -                       # scan stdin as a single file (e.g. git diff | todo-by -)
todo-by --format text           # human-readable (default)
todo-by --format github         # GitHub Actions annotations
todo-by --format json           # JSON Lines, one object per finding
todo-by --today 2026-12-31      # override "now" (useful for testing and CI dry runs)
todo-by --current-version 2.1.0 # override the project's current version, for version triggers
todo-by --warn 14               # also report tags due within 14 days, as warnings
todo-by --exit-zero             # always exit 0 on findings (still 2 on errors)
todo-by --color always          # auto, always, never (default: auto)
todo-by --files                 # list files that would be scanned, then exit
todo-by --dump-config           # print effective config, then exit
```

Exit codes: `0` no findings (warnings alone still exit 0), `1` findings, `2` usage, config, or I/O error.
A reader that stops reading is not one of them: `todo-by | head -4` ends the output where `head` stopped and reports on the scan it ran, so `set -o pipefail` still fails a job whose tree has overdue tags.

## Triggers

Three kinds, each with a marker so a tag is never guessed at: dates carry dashes, versions a lowercase `v`, issues a `#`. A number with no marking is reported as an error naming the fix, never interpreted.

### Dates

| Written as | Deadline |
|---|---|
| `2026-09-01` | that day |
| `2026-09` | last day of that month |

A month is the coarsest precision. A year on its own (`2026`) is not a deadline, since it cannot be told apart from a version; such a tag is reported with both replacements named, `2026-12` or `v2026`. Impossible dates (`2026-02-30`) and dotted ones (`2026.09.01`, which reads equally as a calendar version) are reported too, so a typo cannot silently postpone a deadline forever.

`--warn N` reports tags due within N days as warnings instead of errors, and still exits 0, so a deadline surfaces in CI before it starts failing the build. In `--format github` those render as `::warning`.

```console
$ todo-by --warn 14
src/legacy.rs:8: due in 5 days (2026-07-14): drop the feature flag
1 warning
```

### Versions

```js
// @todo-by v2.0 drop legacy endpoint after v2 ships
// @todo-by >v2.0 drop it only after 2.0 itself is out
```

| Written as | Meaning |
|---|---|
| `v2.0`, `>=v2.0`, `>= v2.0` | fires once the current version is 2.0 or later |
| `v2026.01` | calendar version, fires once the current version is 2026.1 or later |
| `>v2.0` | fires once the current version is later than 2.0 |
| `2.0`, `>=2.0`, `V2.0`, `<v1.0`, `^v1.0` | rejected as findings, never silently ignored |

Only `>=` and `>` exist, because this tool cannot fire on something it never observes. `--warn` does not apply. The current version comes from `--current-version`, then `TODO_BY_VERSION`, then the `version-cmd` config key, then `git describe --tags --abbrev=0`, resolved once and only when a version tag is actually found.

See [docs/versions.md](https://github.com/alies-dev/todo-by/blob/main/docs/versions.md) for the resolution ladder, the `version-cmd` cookbook, and why the marker is mandatory.

### Issues

```js
// @todo-by #123 drop the shim once the upstream bug closes
// @todo-by https://github.com/acme/lib/issues/45 revert when their fix lands
```

| Written as | Meaning |
|---|---|
| `#123` | issue or PR in the repository the git remote points at |
| `https://github.com/o/r/issues/123` | explicit repository, any host, the only cross repository form |
| `https://github.com/o/r/pull/123` | the same, spelled as a pull request |
| `.../issues/123#issuecomment-456` | a comment permalink; fragment and query are ignored |
| `owner/repo#123`, `repo#123`, `GH-123` | rejected, naming the spelling to use instead |

Any state other than open fires, `merged` included; the close reason is never inspected. `--warn` does not apply.

Checking an issue means a network call, so it never happens unless `--online` (or `online = true`) is set, and even then only when the scan found an issue tag. Lookups shell out to `curl` or `gh`, and an environment token is only ever sent to github.com.

See [docs/issues.md](https://github.com/alies-dev/todo-by/blob/main/docs/issues.md) for transports, private repositories, and failure semantics.

## CI (GitHub Actions)

Download the prebuilt musl binary, verify its checksum, run it. No toolchain, no compile step, about a second per job.

```yaml
- name: Check overdue todo-by tags
  run: |
    curl --proto '=https' --tlsv1.2 -sSfL "$URL" -o /tmp/todo-by.tar.xz
    echo "$SHA256  /tmp/todo-by.tar.xz" | sha256sum -c -
    tar -xJf /tmp/todo-by.tar.xz -C /tmp && /tmp/*/todo-by
```

Full workflow, checksum pinning, and how to phase it in on a codebase that already has overdue tags: [docs/ci.md](https://github.com/alies-dev/todo-by/blob/main/docs/ci.md).

## What gets scanned

Everything git would track. `todo-by` uses ripgrep's directory walker, so `.gitignore` is honored with full git semantics (nested files, negation, `**` globs, `.git/info/exclude`), even outside a repository. Dotfiles and dotted directories are scanned like any other, `.github/workflows` among them, because git tracks them. Version control metadata (`.git`, `.hg`, `.svn`, `.jj`) is never walked, at no depth, and neither it nor anything inside it can be reached by naming a path either. Binary and symlinked files are skipped. Any other file named on the command line is scanned even when `.gitignore` covers it, and the `exclude` config key covers whatever `.gitignore` does not.

## Configuration

`todo-by.toml` (or `.todo-by.toml`) is discovered by searching from the current directory upward; the first file found wins.

```toml
warn = 14
exclude = ["vendor/**", "*.gen.go"]
tags = ["todo-by", "fixme-by"]
version-cmd = "jq -r .version package.json"
online = true
repo = "acme/app"
```

- `warn` (integer): same as `--warn`.
- `exclude` (array): gitignore-style globs excluded on top of `.gitignore`, matched relative to where `todo-by` runs.
- `tags` (array): tags to match, case-insensitive. Replaces the default (`todo-by`) rather than adding to it.
- `version-cmd` (string): shell command whose trimmed stdout is the current version. Runs via `sh -c` (`cmd /C` on Windows) in the config file's directory, so relative paths survive being invoked from a subdirectory. See [docs/versions.md](https://github.com/alies-dev/todo-by/blob/main/docs/versions.md).
- `online` (boolean): check issue triggers against GitHub. `--online` and `--offline` both override it. See [docs/issues.md](https://github.com/alies-dev/todo-by/blob/main/docs/issues.md).
- `repo` (string, `owner/name`): repository that bare `#123` references resolve against, instead of the git remote. github.com only; elsewhere write URLs.

Precedence: command line flags win, then the `TODO_BY_FORMAT` / `TODO_BY_WARN` / `TODO_BY_VERSION` environment variables, then the config file.

Use `--dump-config` to see the effective config and where it came from, and `--files` to see which files would be scanned.

## Prior art

Inspired by [phpstan/phpstan-todo-by](https://github.com/staabm/phpstan-todo-by) by Markus Staab.

## License

MIT.
