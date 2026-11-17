# Date triggers

A tag can fire once a date passes.

```js
// @todo-by 2026-12-01 delete the new_checkout flag
# @todo-by 2026-10 drop the Python 3.10 compat shim
```

| Written as | Deadline |
|---|---|
| `2026-12-01` | that day |
| `2026-10` | last day of that month |
| `2026` | rejected, write `2026-12` for the end of the year |
| `2026-02-30` | rejected, the day does not exist |
| `2026.09.01` | rejected, dates use dashes (dotted numbers are versions and need a `v`) |

A tag is overdue once its deadline is on or before today, in UTC. Rejected spellings are reported as findings, each naming the fix, rather than interpreted, so a typo cannot silently postpone a deadline.

## Early warnings

`--warn N` (or `warn = N` in the config) reports tags due within N days as warnings, without failing the build. In `--format github` they render as `::warning` annotations.

```console
$ todo-by --warn 14
./src/checkout.ts:14: due in 6 days (2026-12-01): delete the new_checkout flag
1 warning
```

Warnings apply to dates only. A future release or issue close is not knowable ahead of time.

## Dry runs

`--today 2026-12-31` evaluates every date tag as if it were that day, which answers "what will fail after the holidays?" before it does.
