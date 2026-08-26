# Running todo-by in CI

Download the prebuilt musl binary, verify its checksum, run it. No toolchain, no compile step, about a second per job. Both variables come from the release's `sha256.sum`.

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
          TODOBY_VERSION: v0.4.0
          TODOBY_SHA256: 0f9cb2bc7b5544e64c3f511bd3ed090558016e9477f1b8086667963c83320b5c
        run: |
          ASSET="todo-by-cli-x86_64-unknown-linux-musl.tar.xz"
          curl --proto '=https' --tlsv1.2 -sSfL \
            "https://github.com/alies-dev/todo-by/releases/download/${TODOBY_VERSION}/${ASSET}" -o /tmp/todo-by.tar.xz
          echo "${TODOBY_SHA256}  /tmp/todo-by.tar.xz" | sha256sum -c -
          tar -xJf /tmp/todo-by.tar.xz -C /tmp
          /tmp/todo-by-cli-x86_64-unknown-linux-musl/todo-by
```

On a codebase with existing overdue tags, phase it in with `continue-on-error: true`, or `todo-by --warn N --exit-zero` so deadlines surface without failing the build. The release also ships an installer script if you prefer it to the checksum dance.
