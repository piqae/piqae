# Local-only node

**Status:** headless agent and loopback API implemented; platform packaging
tiers vary.

Local-only mode keeps jobs, content, printer discovery, and queue state on one
computer. It does not turn the loopback API root into a web application.

```sh
SPOOL_AGENT_MODE=local \
SPOOL_DATA_DIR="$PWD/.spool" \
cargo run -p spool-agent
```

The authenticated API binds to `127.0.0.1:39100` by default and creates
`local.token` in the data directory. Keep that file private. Native shells read
it to display status and initiate profile capture; they do not open SQLite.

Use this mode for development and direct local printing. It does not provide
remote submission, multi-node routing, hosted identity, or off-device disaster
recovery. For exact routes and security boundaries, read
[`architecture/local-agent-control.md`](../architecture/local-agent-control.md).

Continue with the relevant node guide:
[macOS](../nodes/macos.md), [Windows](../nodes/windows.md), or
[headless Linux](../nodes/linux-headless.md).
