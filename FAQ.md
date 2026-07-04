# FAQ

Short answers to common architecture questions. See [docs/architecture/den-runtime.md](docs/architecture/den-runtime.md) and [docs/roadmap/PLAN.md](docs/roadmap/PLAN.md) for detail.

## Why does web chat go through Den?

The browser is untrusted, so **Den is the gate**: it authenticates the user, checks bear membership, resolves the Bear stance/session context, executes the native agent loop, and enforces Den-hosted tool policy. Channels bring their own app identity and signing, but they should reuse Den run services rather than bypass Den authorization.
