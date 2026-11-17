# Installing todo-by

## Homebrew (macOS, Linux)

```console
brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
brew install alies-dev/todo-by/todo-by
```

This repository doubles as the tap, which is why `brew tap` needs the URL.

## Cargo

```console
cargo install todo-by-cli --locked
```

Building from source needs Rust 1.88 or later. `--locked` builds against the dependency versions that floor is tested with; the floor tracks the `ignore` crate and can rise in any release, including a patch.

## Prebuilt binaries

Every [release](https://github.com/alies-dev/todo-by/releases) ships binaries for macOS, Linux, and Windows, a `sha256.sum`, and an installer script. In CI, the static musl Linux binary needs no toolchain; see [ci.md](ci.md).
