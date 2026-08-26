# Issue triggers

A tag can fire once a GitHub issue or pull request closes.

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
| `owner/repo#123` | rejected, quoting the exact URL to write instead |
| `repo#123`, `GH-123`, `#12x` | rejected, naming the accepted spelling |

## The marker

A marker is required, as with the `v` on a version: a `#` with a digit behind it, or the `GH-` form. That is what tells a trigger from a date, from a version, and from prose.

The spellings GitHub renders as links (`owner/repo#123`, `repo#123`, `GH-123`) are recognized only in order to be rejected. **None of them ever fires or reaches the network.** They are recognized at all so the tag gets an error naming the accepted spelling instead of silently disappearing, which is the outcome this tool exists to prevent:

```console
$ todo-by
src/legacy.rs:8: cross-repo reference "acme/lib#452" is not supported (write "https://github.com/acme/lib/issues/452"): drop the shim
```

A token carrying no marker stays prose. That matters because `tags` is configurable: a project matching on `todo` would otherwise have every `TODO: refactor later` in the tree reported, and `todo-by` does not police undated TODOs.

## Firing rule

**Any state other than open fires, `merged` included.** The close reason (`completed`, `not planned`) is never requested and never inspected. A tag on an issue closed as "not planned" therefore reports, which is deliberate: the author reads the finding and decides, and one visible finding beats a chore buried by a tool second guessing a tracker.

`--warn` never applies here, since a future close date is not knowable ahead of time.

## Going online

Checking an issue means a network call, so it never happens by default. Pass `--online` (or `online = true` in the config, with `--offline` to override one run). The check then runs only when the scan actually found an issue tag, so a tree of date tags never opens a socket.

```console
$ todo-by --online
src/legacy.rs:8: #123 is closed (https://github.com/acme/app/issues/123): drop the shim
1 finding
```

Without it, issue tags are left unchecked and say so on stderr, producing no findings and no change to the exit code:

```console
$ todo-by
todo-by: 2 issue tags not checked (pass --online to check GitHub)
```

## Transports

There is no HTTP client inside `todo-by`. It shells out, which is what keeps the dependency list at one crate:

1. `curl`, when `GH_TOKEN` or `GITHUB_TOKEN` is set and the target is github.com. The token travels in curl's config file on stdin, so it never reaches the process list or the disk, and the request gets a timeout of 30 seconds.
2. `gh`, otherwise, authenticating from its own keyring, so a laptop that has run `gh auth login` needs no setup.

**An environment token is only ever sent to github.com.** A tag names its own host and a tag is repository content, so a comment reading `todo-by https://evil.example/o/r/issues/1` must not be able to make a CI run hand `$GITHUB_TOKEN` to whatever host it names. Every other host goes through `gh`, which keeps credentials per host and fails closed when it has none for that one. That is also the correct behavior for GitHub Enterprise, where a github.com token is the wrong credential anyway.

The identity is the same either way, since `gh` also prefers those variables over its keyring. With neither transport usable, `--online` exits 2 rather than skipping the check quietly. Note that `gh` has no timeout flag, so a `gh`-backed run waits as long as the network does, exactly as `version-cmd` and `git describe` already do.

References are batched into one GraphQL request per host (up to 100 at a time), so a repository with two hundred issue tags makes two round trips, not two hundred.

## Access and private repositories

- On a laptop, `gh auth login` covers public and private repositories, including organizations that enforce SSO.
- In GitHub Actions, the built in `GITHUB_TOKEN` covers the workflow's own repository and needs `permissions: issues: read` (add `pull-requests: read` for PR references).
- **`GITHUB_TOKEN` cannot read a different private repository**, whatever the transport. A cross repository reference into one needs a fine grained PAT with Issues: Read there, or a GitHub App token, held as a secret and exported as `GH_TOKEN`.
- GitHub Enterprise works through the URL form: the host in the URL selects the API, and `gh` picks up that host from its own configuration.
- The token comes from the environment only. There is no config key for it, because config files get committed.

## Which repository `#123` means

The bare form reads `origin`, or `upstream` when there is no `origin`. Two remotes that disagree, the usual fork checkout, are reported rather than guessed at, since picking one silently would resolve `#123` against somebody else's tracker. Settle it with `repo = "owner/name"` in the config, which skips remote inference entirely. That key is github.com only, so on another host write the references as URLs.

## Failures

Failures are per reference: an issue nobody can see, or a number that does not exist, reports on its own line and leaves every other finding intact.

A failure that kills a whole request (no credentials, a rejected token, an unreachable host) is reported once and exits 2, and it abandons only the host it happened on. An unreachable GitHub Enterprise instance says nothing about the github.com references in the same tree, so those findings still stand.
