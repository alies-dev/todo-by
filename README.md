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
todo-by --current-version 2.1.0 # override the project's current version, for version triggers
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

Dates are written with dashes, to a month or to a day. A tag becomes overdue the day its deadline is reached.

| Written as | Deadline |
|---|---|
| `2026-09-01` | that day |
| `2026-09` | last day of that month |

A month is the coarsest precision. A year on its own (`2026`) is not a deadline, because it cannot be told apart from a version constraint (see [Versions](#versions)); such a tag is reported as an error naming both replacements, `2026-12` or `v2026`. Impossible dates (for example `2026-02-30`) are reported as findings too, so typos cannot silently postpone a deadline forever.

Dates use dashes. A dotted date (`2026.09.01`) is reported as an error, because it reads equally as a calendar version (see [Versions](#versions)); the message names both spellings so you can pick one.

#### Warn ahead

`--warn N` reports tags due within N days as warnings rather than errors, so a deadline surfaces in CI before it starts failing the build. It still exits 0.

```console
$ todo-by --warn 14
src/legacy.rs:8: due in 5 days (2026-07-14): drop the feature flag
1 warning
```

In `--format github`, warnings render as `::warning` annotations instead of `::error`.

### Versions

A tag can also fire once the project reaches a version, instead of a date. Write the version with a lowercase `v`, on its own or after a comparator.

```js
// @todo-by v2.0 drop legacy endpoint after v2 ships
// @todo-by >v2.0 drop it only after 2.0 itself is out
```

The tag fires the moment the project's current version satisfies the constraint. A version without a comparator means `>=`, which is what almost every cleanup tag wants.

**The `v` is required.** It is what tells a version from a date, so a tag is never guessed at: dates are the ones with dashes, versions are the ones with a `v`, and a number carrying neither marking is reported as an error naming the fix. That matters because a tag sits in prose, where an unmarked number reads two ways at once. `2026.09.01` is both a dotted deadline and a calendar version, `12.5.2026` is both a day first date and a three component version, and `3.5` is both a constraint and the start of "3.5 hours of work". Guessing at any of those risks the worst outcome this tool has: a guess of "version" produces a constraint the project never reaches, and an unreached constraint reports nothing at all, so the chore is buried instead of surfaced.

| Written as | Meaning |
|---|---|
| `v2.0`, `>=v2.0`, `>= v2.0` | fires once the current version is 2.0 or later |
| `v2026.01` | calendar version, fires once the current version is 2026.1 or later |
| `>v2.0` | fires once the current version is later than 2.0 |
| `2.0`, `>=2.0`, `2026.09.01` | rejected, with the marked spelling as the fix |
| `V2.0` | rejected, the marker is lowercase only |
| `<`, `<=`, `=`, `==`, `^`, `~` | recognized but rejected as findings, not silently ignored |

A space after the comparator is allowed, so `>= v2.0` and `>=v2.0` are the same tag.

The current version is a separate matter: `--current-version`, `TODO_BY_VERSION`, `version-cmd`, and `git describe` all accept a bare `1.2.3`, since those strings come from the project rather than from a tag author. The marker is required only where the ambiguity exists, which is inside the tag.

Only `>=` and `>` are supported. Writing `<v1.0` to mean "before version 1.0" is a natural reach, and so is borrowing `^v1.0` or `~v1.0` from a package manager, but this tool has no way to fire on something it can never observe (a version that is never released, or one above a ceiling), so it reports those as invalid rather than quietly never firing.

Unlike dates, `--warn` never applies to version triggers: a future version isn't knowable ahead of time the way a future date is.

`todo-by` resolves the current version once, only when the scan finds at least one version tag, in this order.

1. `--current-version <X>` on the command line.
2. The `TODO_BY_VERSION` environment variable.
3. The `version-cmd` config key: a shell command whose trimmed stdout is the version, for example `version-cmd = "jq -r .version package.json"`. It runs in the config file's own directory, so a relative path (like `package.json` above) resolves against the file that declared it, not against wherever `todo-by` was invoked.
4. `git describe --tags --abbrev=0`, with a leading `v` or `V` stripped. This runs in the directory `todo-by` was invoked in, not the config file's directory: git already walks upward on its own to find the repository, and a config file discovered above the actual repository (a monorepo layout, for example) would otherwise point git at the wrong one.

Whichever source wins, a resolved version keeps only its tag. The markers `git describe` appends when HEAD is past the tag (`v1.2.3-4-gabc123`) or the tree is dirty (`v1.2.3-dirty`) are stripped, so both read as `1.2.3`. Semver counts those suffixes as a pre-release, which sorts below the release it was built from, so without the strip a commit four past `v1.2.3` would compare as not having reached `1.2.3` and a `>=v1.2.3` tag would produce no finding at all. The built-in default avoids the suffix through `--abbrev=0`, but a `version-cmd` such as `git describe --tags` does not. A genuine pre-release (`1.2.3-rc.1`) keeps its suffix and still sorts below `1.2.3`, which is correct.

`version-cmd` runs a shell command taken from the config file, so treat it the same as any other command a repository can make CI run, and only enable it in repositories you trust. It executes only when the scan actually finds a version tag, never on every run.

Worth knowing where that command can come from: config discovery walks from the current directory upward, so the file that supplies `version-cmd` is not necessarily inside the repository being scanned. A `todo-by.toml` in a parent directory (or in your home directory) applies to everything below it. Use `--dump-config` to see which file won, and `--current-version` or `TODO_BY_VERSION` to bypass the config path entirely.

#### version-cmd cookbook

`todo-by` deliberately does not parse build manifests. Inferring a version from a build system is how a linter starts lying to you: a missing version fails loudly (exit 2), a wrongly guessed one silently changes when your tags fire. So the extraction stays a command you write and can verify.

Anything printing the version on stdout works. Output is trimmed, and a leading `v` is stripped. These recipes assume a POSIX shell, since `version-cmd` runs through `sh -c`; on Windows it runs through `cmd /C`, so `cat` becomes `type` and the sourcing trick at the bottom needs a `for /f` loop instead.

| Version lives in | `version-cmd` |
|---|---|
| `package.json` | `jq -r .version package.json` |
| `package.json`, without jq | `node -p "require('./package.json').version"` |
| `Cargo.toml` | `cargo metadata --no-deps --format-version 1 \| jq -r .packages[0].version` |
| `pyproject.toml` | `python -c "import tomllib;print(tomllib.load(open('pyproject.toml','rb'))['project']['version'])"` |
| `pom.xml` | `mvn -q -DforceStdout help:evaluate -Dexpression=project.version` |
| `gradle.properties` | `gradle -q properties \| awk '/^version:/ {print $2}'` |
| `Chart.yaml` (Helm) | `yq -r .version Chart.yaml` |
| A plain `VERSION` file | `cat VERSION` |
| A key and value file (OpenSSL style) | `. ./VERSION.dat && echo "$MAJOR.$MINOR.$PATCH"` |

Go and PHP projects usually keep no version in a manifest at all, so the git tag default already covers them. Nothing else needed.

Two things worth checking when a recipe misbehaves: the command runs in the config file's directory (so relative paths resolve against the config file, not the invocation directory), and it must print the version alone, since the whole trimmed stdout is the version.

In GitHub Actions, `actions/checkout` fetches no tags by default, so the git based default finds nothing to describe. Set `fetch-depth: 0` on the checkout step, or skip git entirely with `--current-version` or `TODO_BY_VERSION`. Note that `fetch-tags: true` alone is not enough: it fetches the tag objects but leaves the clone shallow, so `git describe` still reports `No tags can describe` unless HEAD happens to be the tagged commit itself.

### Issues

A tag can also fire once a GitHub issue or pull request closes. Two spellings, one job each.

```js
// @todo-by #123 drop the shim once the upstream bug closes
// @todo-by https://github.com/acme/lib/issues/45 revert when their fix lands
```

| Written as | Meaning |
|---|---|
| `#123` | issue or PR in the repository the git remote points at |
| `https://github.com/o/r/issues/123` | explicit repository, any host, the only cross repository form |
| `https://github.com/o/r/pull/123` | the same, spelled as a pull request |
| `https://github.com/o/r/issues/123#issuecomment-456` | a comment permalink; the fragment and any query are ignored |
| `owner/repo#123` | not supported, since the URL already says it and can also name a host |
| `GH-123`, `gh#123`, `#12x` | reported as errors, with `#123` named as the fix |

**The `#` is required**, for the same reason the `v` is on a version: it is the marker that tells this trigger from a date, a version, and from prose. A `#` not followed by an alphanumeric (`todo-by # note`) is prose and is skipped.

**Any state other than open fires, `merged` included.** The close reason (`completed`, `not planned`) is never requested and never inspected. A tag on an issue closed as "not planned" therefore reports, which is deliberate: the author reads the finding and decides, and one visible finding beats a chore buried by a tool second guessing a tracker.

Like versions, `--warn` never applies to issue triggers, since a future close date is not knowable ahead of time.

#### Going online

Checking an issue means a network call, so it never happens by default. Pass `--online` (or set `online = true` in the config, with `--offline` to override it for one run), and the check runs only when the scan actually found an issue tag.

```console
$ todo-by --online
src/legacy.rs:8: #123 is closed (https://github.com/acme/app/issues/123): drop the shim
1 finding
```

Without `--online`, issue tags are left unchecked and say so on stderr, without producing findings and without changing the exit code:

```console
$ todo-by
todo-by: 2 issue tags not checked (pass --online to check GitHub)
```

There is no HTTP client inside `todo-by`. It shells out, which is what keeps the dependency list at one crate:

1. `curl`, when `GH_TOKEN` or `GITHUB_TOKEN` is set and the target is github.com. The token is passed in curl's config file on stdin, so it never appears in the process list or on disk, and the request gets a 30 second timeout.
2. `gh`, otherwise. It authenticates from its own keyring, so a laptop with `gh auth login` already done needs no setup at all.

**An environment token is only ever sent to github.com.** A tag can name any host, and a tag is repository content, so a comment reading `todo-by https://evil.example/o/r/issues/1` must not be able to make a CI run hand `$GITHUB_TOKEN` to whatever host it names. Every other host goes through `gh`, which keeps credentials per host and fails closed when it has none for that one. That is also the correct behavior for GitHub Enterprise, where a github.com token is the wrong credential anyway.

The identity is the same either way, since `gh` also prefers those environment variables over its keyring. If neither transport can be used, `--online` fails with exit 2 rather than skipping the check quietly. Note that `gh` has no timeout flag, so a `gh`-backed run can wait as long as the network does; the same is already true of `version-cmd` and `git describe`.

All references are batched into one GraphQL request per host (up to 100 at a time), so a repository with two hundred issue tags makes two round trips, not two hundred.

#### Access and private repositories

- On a laptop, `gh auth login` covers public and private repositories, including organizations that enforce SSO.
- In GitHub Actions, the built in `GITHUB_TOKEN` works for the workflow's own repository and needs `permissions: issues: read` (add `pull-requests: read` for PR references).
- **`GITHUB_TOKEN` cannot read a different private repository**, whatever the transport. A cross repository reference into a private repo needs a fine grained PAT with Issues: Read on that repository, or a GitHub App token, held as a secret and exported as `GH_TOKEN`.
- GitHub Enterprise works through the URL form: the host in the URL selects the API, and `gh` picks up the matching host from its own configuration.
- The token comes from the environment only. There is no config key for it, because config files get committed.

#### Which repository `#123` means

The bare form resolves against the git remote, reading `origin` (or `upstream` when there is no `origin`). Two remotes that disagree, the usual fork checkout, are reported rather than guessed at, since picking one silently would resolve `#123` against somebody else's tracker. Settle it with `repo = "owner/name"` in the config, which skips remote inference entirely.

A run that reaches more than one host abandons only the host that fails. An unreachable GitHub Enterprise instance says nothing about the github.com references in the same tree, so those findings are still reported (and the run still exits 2).

Failures are per reference, not per run: an issue nobody can see, or a number that does not exist, reports on its own line and leaves every other finding intact. Only a failure that makes the whole batch meaningless (no credentials, a rejected token, an unreachable host) is reported once and exits 2.

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
version-cmd = "jq -r .version package.json"
online = true
repo = "acme/app"
```

- `warn` (integer): same as `--warn`.
- `exclude` (array of strings): gitignore-style globs excluded in addition to `.gitignore`. Globs are matched relative to the directory where `todo-by` runs, like ripgrep's `--glob`.
- `tags` (array of strings): tags to match, case-insensitive. Setting this replaces the default (`todo-by`) entirely rather than adding to it.
- `version-cmd` (string): a shell command whose trimmed stdout is the current version, used to resolve version triggers (see [Versions](#versions)). It runs via `sh -c` (on Windows, `cmd /C`) in the config file's directory, so relative paths keep working when `todo-by` is invoked from a subdirectory.
- `online` (boolean): check issue triggers against GitHub (see [Issues](#issues)). `--online` and `--offline` both override it.
- `repo` (string, `owner/name`): the repository bare `#123` references resolve against, instead of the git remote. github.com only; on another host, write the references as URLs.

Precedence: command line flags win, then the `TODO_BY_FORMAT` / `TODO_BY_WARN` / `TODO_BY_VERSION` environment variables, then the config file.

Use `--dump-config` to see the effective config and where it came from, and `--files` to see which files would be scanned.

## Prior art

Inspired by [phpstan/phpstan-todo-by](https://github.com/staabm/phpstan-todo-by) by Markus Staab, which does this for PHP files as a PHPStan extension, with package version triggers `todo-by` does not have. `todo-by` trades those for working on any file type with no runtime.

## License

MIT.
