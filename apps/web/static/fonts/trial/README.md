# Local trial fonts

This directory supports local typography evaluation only. Font binaries are
ignored by Git and must not be published with a commercial build.

For the current marketing prototype, copy the Signal Type Foundry Exact trial
webfont to:

```text
exact-test-regular.woff
```

`src/marketing.css` exposes the typography through three tokens:

- `--m-font-editorial`: Exact for major editorial headings.
- `--m-font-display`: Instrument Sans for interface and secondary headings.
- `--m-font-body`: Inter for body copy.

Before production deployment, replace the trial file and `@font-face` source
with the properly licensed Exact webfont. Keep the licensed filename and
format private to the deployment workflow if its licence does not permit
redistribution in the public source repository.
