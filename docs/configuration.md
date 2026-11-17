# Configuration

`todo-by.toml` (or `.todo-by.toml`) is discovered by searching from the current directory upward; the first file found wins.

```toml
warn = 14
exclude = ["vendor/**", "*.gen.go"]
tags = ["todo-by", "fixme-by"]
version-cmd = "jq -r .version package.json"
online = true
repo = "acme/app"
```

| Key | Type | Meaning |
|---|---|---|
| `warn` | integer | same as `--warn` |
| `exclude` | array | gitignore style globs excluded on top of `.gitignore`, matched relative to where `todo-by` runs |
| `tags` | array | tags to match, case insensitive; replaces the default (`todo-by`) rather than adding to it |
| `version-cmd` | string | shell command whose trimmed stdout is the current version, run in the config file's directory; see [versions.md](versions.md) |
| `online` | boolean | check issue triggers against GitHub; `--online` and `--offline` both override it |
| `repo` | string, `owner/name` | repository that bare `#123` references resolve against, instead of the git remote; github.com only, elsewhere write URLs |

## Precedence

Command line flags win, then the `TODO_BY_FORMAT`, `TODO_BY_WARN`, and `TODO_BY_VERSION` environment variables, then the config file.

`todo-by --dump-config` prints the effective config and the file it came from.
