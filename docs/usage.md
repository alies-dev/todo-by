# Usage

```console
todo-by                         # scan the current directory
todo-by src tests               # scan specific paths
git diff | todo-by -            # scan stdin as a single file
todo-by --warn 14               # also warn about tags due within 14 days
todo-by --online                # also check issue tags against GitHub
todo-by --offline               # never check issue tags, overriding the config
todo-by --current-version 2.1.0 # set the version that version tags compare against
todo-by --today 2026-12-31      # pretend it is another day (dry runs)
todo-by --exit-zero             # report, but never fail on findings
todo-by --files                 # list files that would be scanned, then exit
todo-by --dump-config           # print the effective config and its sources, then exit
```

`todo-by --help` lists every option.

## Output formats

| `--format` | Output |
|---|---|
| `text` | `path:line: reason: message`, the default outside GitHub Actions |
| `github` | `::error` and `::warning` annotations, auto-selected inside GitHub Actions |
| `json` | JSON Lines, one object per finding |

`--color auto|always|never` controls color in text output.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | nothing due (warnings alone still exit 0) |
| `1` | at least one finding |
| `2` | usage, config, or I/O error, or nothing left that the scan was allowed to read |

`--exit-zero` turns `1` into `0` but leaves `2` alone, so a broken run still fails.
