# todo-by

Flag `todo-by` tags whose deadline date has passed. Works on any file type.

## Idea

Tag any comment with a deadline date:

```yaml
# todo-by 2026-09-01 drop the legacy webhook once v2 ships
```

`todo-by` scans the tree (respecting `.gitignore`), validates each date, and exits non-zero when a date is in the past, so it gates CI.

```console
$ todo-by
config/legacy.yml:42: overdue since 2026-09-01: drop the legacy webhook once v2 ships
```

## Usage

```console
todo-by [PATHS]...        # scan paths (default: current dir)
todo-by --format text     # human-readable (default)
todo-by --format json     # machine-readable
todo-by --format github   # GitHub Actions annotations
todo-by --today 2026-12-31  # override "now"
```

## Status

Early scaffold. Scanner not implemented yet.

## License

MIT.
