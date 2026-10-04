# Codex onboarding brief (one page)

For every Codex lane (X-*) at unfreeze. Source of truth for paths: `OWNERS.md`.

1. Start from current main. Your lanes own only the globs listed for them in OWNERS.md.
2. `seams/` belongs to Claude lanes; do not edit it. Crossing a lane is a `[DEP-ASK]`.
3. Public export requests to crates/core follow the G0gr export list; ask, do not widen.
4. Work offline against the frozen pack (FRZ0): schemas, goldens and traces verified by
   digest. No Podman, live Postgres or live Core is needed or assumed.
5. Vendor dir and new dependencies: a new dependency is a `[DEP-ASK]`.
6. Depth-1 PR rule: one PR targets main directly, no stacked PRs.
7. Bundle rule: a PR changes one lane's files plus its own docs and journal entry.
8. Migrations, ADR and journal numbers come from your lane's range in OWNERS.md.
9. Journal tags and the Team trailer: `docs/agents/governance.md`.

## Data-class rule (all developer agents, Claude and Codex)

Both Claude and Codex are hosted third parties. The data-class rule: developer agents
work from schemas, digests and synthetic fixtures, never from raw E0 text, the CSV
sources, original or customer data, recordings, credentials or secrets. Nothing of that
class is pasted into a prompt, committed, or placed in a journal, report or test
fixture. If a task seems to need such data, stop and raise a `[BLOCKED]` or `[ASK]`.
Local engine sensors may process E0 on the user's machine; developer agents see only
their schemas and synthetic stand-ins.
