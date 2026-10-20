<p align="center">
  <img src="https://raw.githubusercontent.com/alies-dev/todo-by/main/assets/banner.svg" alt="todo-by" width="620">
</p>

<p align="center">
  <a href="https://github.com/alies-dev/todo-by/actions/workflows/ci.yml"><img src="https://github.com/alies-dev/todo-by/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/todo-by-cli"><img src="https://img.shields.io/crates/v/todo-by-cli.svg" alt="crates.io"></a>
  <a href="LICENSE"><img src="https://img.shields.io/crates/l/todo-by-cli.svg" alt="license"></a>
</p>

**Fail CI when a TODO comes due.**

Tag a comment with a date, a version, or a GitHub issue. `todo-by` scans your repository and exits non-zero once the date passes, the version ships, or the issue closes. It reads plain text, so it works in any language and any comment style.

```text
src/billing.ts:14   // @todo-by 2026-09-01 remove the legacy checkout once signed URLs ship
src/api.py:3        # @todo-by v2.0 drop the /v1 endpoints
src/client.rs:8     // @todo-by #123 drop the shim once the upstream bug closes
```

```console
$ todo-by --online
./src/api.py:3: version 2.0 reached (v2.0): drop the /v1 endpoints
./src/billing.ts:14: overdue since 2026-09-01: remove the legacy checkout once signed URLs ship
./src/client.rs:8: #123 is closed (https://github.com/acme/app/issues/123): drop the shim once the upstream bug closes
3 findings
```

## Install

Homebrew (macOS, Linux):

```console
brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
brew install alies-dev/todo-by/todo-by
```

Cargo:

```console
cargo install todo-by-cli --locked
```

Or grab a prebuilt binary from [Releases](https://github.com/alies-dev/todo-by/releases).

Building from source needs Rust 1.88 or later. `--locked` builds against the dependency versions that floor is tested with; the floor tracks the `ignore` crate and can rise in any release, including a patch.

## Usage

```console
todo-by                         # scan the current directory
todo-by src tests               # scan specific paths
git diff | todo-by -            # scan stdin as a single file
todo-by --warn 14               # also warn about tags due within 14 days
todo-by --online                # also check issue tags against GitHub
todo-by --format github         # GitHub Actions annotations (auto-selected in Actions)
todo-by --format json           # JSON Lines, one object per finding
todo-by --today 2026-12-31      # pretend it is another day (dry runs)
todo-by --exit-zero             # report, but never fail on findings
todo-by --files                 # list files that would be scanned, then exit
```

Exit codes: `0` nothing due (warnings alone still exit 0), `1` findings, `2` usage, config, or I/O error. `todo-by --help` lists every option.

## When to use it

For chores that are **small** (a minute or two), **mechanical** (delete, revert, re-enable), and **triggered** by something observable (a date, a release, an upstream fix). The reminder stays next to the code it is about, and the trigger, not a person, decides when it comes due.

If the work needs an owner, a design, or a conversation, use an issue tracker instead.

## Triggers

Each kind has a marker, so a tag is never guessed at: dates have dashes, versions a lowercase `v`, issues a `#`. A trigger missing its marker is reported as an error naming the fix.

### Dates

| Written as | Deadline |
|---|---|
| `2026-09-01` | that day |
| `2026-09` | last day of that month |

A bare year (`2026`), an impossible date (`2026-02-30`), or a dotted one (`2026.09.01`) is reported rather than interpreted, so a typo cannot silently postpone a deadline.

`--warn N` reports tags due within N days as warnings, rendered as `::warning` in `--format github`, without failing the build:

```console
$ todo-by --warn 14
./src/billing.ts:14: due in 7 days (2026-09-01): remove the legacy checkout once signed URLs ship
1 warning
```

### Versions

```js
// @todo-by v2.0 drop the legacy endpoint
// @todo-by >v2.0 drop it only after 2.0 itself is out
```

Fires once the project's current version reaches the constraint. The current version comes from `--current-version`, then `TODO_BY_VERSION`, then the `version-cmd` config key, then the latest git tag.

More in [docs/versions.md](https://github.com/alies-dev/todo-by/blob/main/docs/versions.md): accepted spellings, how each source resolves, and a `version-cmd` cookbook.

### Issues

```js
// @todo-by #123 drop the shim once the upstream bug closes
// @todo-by https://github.com/acme/lib/issues/45 revert when their fix lands
```

Fires once the issue or pull request is no longer open, merged included. `#123` means the repository your git remote points at; use a full URL for any other.

Checking needs the network, so it only happens with `--online` (or `online = true`), through `gh` or `curl`. More in [docs/issues.md](https://github.com/alies-dev/todo-by/blob/main/docs/issues.md): tokens, private repositories, and failure handling.

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

Every text file git would track: `.gitignore` is honored with full git semantics, dotfiles are included, and binaries and version control metadata are skipped. A path named on the command line is scanned even if it is ignored.

Details in [docs/scanning.md](https://github.com/alies-dev/todo-by/blob/main/docs/scanning.md).

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
- `version-cmd` (string): shell command whose trimmed stdout is the current version, run in the config file's directory. See [docs/versions.md](https://github.com/alies-dev/todo-by/blob/main/docs/versions.md).
- `online` (boolean): check issue triggers against GitHub. `--online` and `--offline` both override it.
- `repo` (string, `owner/name`): repository that bare `#123` references resolve against, instead of the git remote. github.com only; elsewhere write URLs.

Precedence: command line flags win, then the `TODO_BY_FORMAT` / `TODO_BY_WARN` / `TODO_BY_VERSION` environment variables, then the config file.

Use `--dump-config` to see the effective config and where it came from.

## Prior art

Inspired by [phpstan/phpstan-todo-by](https://github.com/staabm/phpstan-todo-by) by Markus Staab.

## License

MIT.
