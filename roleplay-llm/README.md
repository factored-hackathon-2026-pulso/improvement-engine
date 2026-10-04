# roleplay-llm

Purpose: the `agent_roleplay` shim, an OpenAI-compatible chat endpoint that the real llm-gateway alias
`pulso-evolution-llm` points at. A fresh-context responder (a human or subagent) answers queued requests; the
shim only serves recorded or queued answers. Plumbing only: `quality_claims: forbidden`.

- Layout: `roleplay_llm/{shim,scanner,protocol}.py` (server, treated-payload scanner TPS, queue protocol), `tests/`.
- Full operating procedure: [`RUNBOOK.md`](RUNBOOK.md).
- Run: `python -m roleplay_llm --queue <dir> --port 8640` (add `--replay-only` to serve only recorded responses).

Tests (verified, stdlib only):

    cd roleplay-llm && uv run --python 3.12 python -m unittest discover -s tests

Data-class rules: only treated or synthetic payloads reach the shim. TPS scans every request (system prompt
included) before it is queued and rejects raw-E0-shaped content. Known open risk: opaque ids that look like
allowed tokens pass the allow-list until a registry of expected ids exists. Raw-E0-derived queues stay in a
git-ignored directory; no E0 rows are committed.

Honesty labels: every response carries `provenance: agent_roleplay`; any run that uses the shim lists it in
`doubles[]` and never calls it a real model.

Owner lane: L-MODEL (see `OWNERS.md`).
