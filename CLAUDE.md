# Claude repository instructions

Read and follow `AGENTS.md` before making changes. It is the canonical
repository-wide instruction file for architecture, safety, testing, licensing,
and support claims.

Start with:

```console
cargo xtask doctor
cargo xtask test changed
```

Physical printing is never part of an ordinary development or test command. It
requires an explicit human-authorized printer and fixture.
