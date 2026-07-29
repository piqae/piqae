# DCO sign-off

Every contribution is made under the repository license and must certify the
[`DCO`](../DCO). Add a `Signed-off-by` trailer to every commit:

```console
git commit --signoff
```

The trailer must use a name and email that identify the contributor:

```text
Signed-off-by: Example Contributor <contributor@example.com>
```

To sign an existing local commit, amend it and force-push only your own branch:

```console
git commit --amend --no-edit --signoff
```

For several local commits, use an interactive rebase and amend each commit.
Do not add another person's sign-off, and do not confuse a DCO sign-off with a
GPG or SSH commit signature. Bots and automated contributors must also use an
accountable identity and sign-off.
