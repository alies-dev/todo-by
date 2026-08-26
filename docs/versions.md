# Version triggers

A tag can fire once the project reaches a version, instead of on a date.

```js
// @todo-by v2.0 drop legacy endpoint after v2 ships
// @todo-by >v2.0 drop it only after 2.0 itself is out
```

The tag fires the moment the project's current version satisfies the constraint. A version without a comparator means `>=`, which is what almost every cleanup tag wants. A space after the comparator is allowed, so `>= v2.0` and `>=v2.0` are the same tag.

| Written as | Meaning |
|---|---|
| `v2.0`, `>=v2.0`, `>= v2.0` | fires once the current version is 2.0 or later |
| `v2026.01` | calendar version, fires once the current version is 2026.1 or later |
| `>v2.0` | fires once the current version is later than 2.0 |
| `2.0`, `>=2.0`, `2026.09.01` | rejected, with the marked spelling as the fix |
| `V2.0` | rejected, the marker is lowercase only |
| `<`, `<=`, `=`, `==`, `^`, `~` | recognized but rejected as findings, not silently ignored |

## Why the `v` is required

It is what tells a version from a date, so a tag is never guessed at: dates are the ones with dashes, versions are the ones with a `v`, and a number carrying neither marking is reported as an error naming the fix.

That matters because a tag sits in prose, where an unmarked number reads two ways at once. `2026.09.01` is both a dotted deadline and a calendar version, `12.5.2026` is both a day first date and a three component version, and `3.5` is both a constraint and the start of "3.5 hours of work". Guessing at any of those risks the worst outcome this tool has: a guess of "version" produces a constraint the project never reaches, and an unreached constraint reports nothing at all, so the chore is buried instead of surfaced.

The current version is a separate matter. `--current-version`, `TODO_BY_VERSION`, `version-cmd`, and `git describe` all accept a bare `1.2.3`, since those strings come from the project rather than from a tag author. The marker is required only where the ambiguity exists, which is inside the tag.

## Why only `>=` and `>`

Writing `<v1.0` to mean "before version 1.0" is a natural reach, and so is borrowing `^v1.0` or `~v1.0` from a package manager. But this tool has no way to fire on something it can never observe (a version that is never released, or one above a ceiling), so it reports those as invalid rather than quietly never firing.

Unlike dates, `--warn` never applies to version triggers: a future version is not knowable ahead of time the way a future date is.

## Resolving the current version

`todo-by` resolves it once per run, and only when the scan finds at least one version tag, in this order.

1. `--current-version <X>` on the command line.
2. The `TODO_BY_VERSION` environment variable.
3. The `version-cmd` config key: a shell command whose trimmed stdout is the version, for example `version-cmd = "jq -r .version package.json"`. It runs in the config file's own directory, so a relative path (like `package.json` above) resolves against the file that declared it, not against wherever `todo-by` was invoked.
4. `git describe --tags --abbrev=0`, with a leading `v` or `V` stripped. This runs in the directory `todo-by` was invoked in, not the config file's directory: git already walks upward on its own to find the repository, and a config file discovered above the actual repository (a monorepo layout, for example) would otherwise point git at the wrong one.

Whichever source wins, a resolved version keeps only its tag. The markers `git describe` appends when HEAD is past the tag (`v1.2.3-4-gabc123`) or the tree is dirty (`v1.2.3-dirty`) are stripped, so both read as `1.2.3`. Semver counts those suffixes as a pre-release, which sorts below the release it was built from, so without the strip a commit four past `v1.2.3` would compare as not having reached `1.2.3` and a `>=v1.2.3` tag would produce no finding at all. The built-in default avoids the suffix through `--abbrev=0`, but a `version-cmd` such as `git describe --tags` does not. A genuine pre-release (`1.2.3-rc.1`) keeps its suffix and still sorts below `1.2.3`, which is correct.

## `version-cmd` is a shell command

Treat it the same as any other command a repository can make CI run, and only enable it in repositories you trust. It executes only when the scan actually finds a version tag, never on every run.

Worth knowing where that command can come from: config discovery walks from the current directory upward, so the file that supplies `version-cmd` is not necessarily inside the repository being scanned. A `todo-by.toml` in a parent directory (or in your home directory) applies to everything below it. Use `--dump-config` to see which file won, and `--current-version` or `TODO_BY_VERSION` to bypass the config path entirely.

## Cookbook

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
