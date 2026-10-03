# pin-watch

Automated, repeatable check of upstream drift against our committed agent-core pin. Run it once per orchestrator
cycle. It also tracks `pulso-factored/llm-gateway` main (no pin of its own: tracked against the last-seen SHA).

Read-only by design: only `gh api` GET calls (never remote-tracking branches), a scratch checkout under
`%TEMP%\pin-watch-scratch` (never `references/agent-core*`), no pushes, no repo edits, no secrets (gh holds its auth).
Outputs go to `scripts/core/pin-watch/out/` and `.state/` (both git-ignored).

## Run

```powershell
pwsh scripts/core/pin-watch/pin-watch.ps1 -- --since-last --json   # the orchestrator cycle call
pwsh scripts/core/pin-watch/pin-watch.ps1 -- --no-checks           # API-only, ~2 s
pwsh scripts/core/pin-watch/pin-watch.ps1 -- --run-tests           # also our non-PG core-bridge tests vs new SHA
```

Flags: `--json` (stdout JSON; default markdown), `--since-last` (skip when pin, agent-core main, open-PR heads and
llm-gateway main are all unchanged since the last run; stores them in `.state/last-seen.json`), `--no-checks`,
`--run-tests`, `--include-prs` (open-PR hints raise the overall verdict), `--python`, `--no-sync`, `--llm-baseline SHA`,
`--clone-source`, `--scratch-root`, `--out-dir`, `--state-file`, `--gh-fixtures DIR` (offline replay).

What it does:

1. Reads the CURRENT pin (`contracts/agent_core/pin.json` if present, else the newest
   `core-bridge/wire/*/MANIFEST.json`). SHA-agnostic.
2. Queries agent-core `main`, open PRs (advisory) and llm-gateway `main` via `gh api`.
3. If `main != pin`: commits since pin, files grouped by area (contracts, registry, composition, api, identity, cli,
   dockerfile, migrations, docs, tests, ...) and signals: `contracts_version`, `openapi_changed`, `schema_changed`,
   `signature_change` (ports / `ServePorts` / `build_api_deps` / `resolve_ports` / `create_app` / ...),
   `migration_added`, `problem_code_change`, `new_env_var`, `new_route`, `route_removed`.
4. Unless `--no-checks`: scratch checkout of the new SHA, then (venv for the new SHA is reused or built with
   `uv sync --locked` under `%TEMP%\pulso-wire-venv-<sha7>`, same convention as `gen-wire.ps1`):
   - `compat`: our `assert_compat()` with `PYTHONPATH=<scratch>;core-bridge/src`;
   - `gen_wire`: the real `gen_wire.py` (run from a shadow copy pinned to the new SHA) into a temp dir;
   - `wire_diff`: byte diff vs the committed `core-bridge/wire/agent_core@<pin7>` per category
     (schemas, registry, events, derived, golden, openapi) plus the route table (added/removed `METHOD /path`);
   - `tests` (`--run-tests`): `pytest core-bridge/tests -k "not pg and not postgres and not integration"`.
5. Writes `out/pin-watch-report.json` + `out/pin-watch-report.md`, with `our_files_likely_to_change`.

## Verdicts and exit codes

| exit | verdict | meaning |
|---|---|---|
| 0 | `no-change` | upstream == pin (or `--since-last` and nothing new) |
| 10 | `additive-safe` | upstream moved, compat + gen_wire pass, wire identical, no interface signals (docs/tests/impl only) |
| 20 | `needs-bump-work` | wire drift, or new routes / env vars / migrations / problem codes / schema changes, or non-PG tests fail; compat passes |
| 30 | `breaking` | `contracts/VERSION` changed, route removed, `compat` or `gen_wire` fails, or a pinned signature changed while compat was not proven |
| 1 | error | no pin, gh failure, bad args (message on stderr) |

Overall verdict is the worst of agent-core main and llm-gateway (open PRs only with `--include-prs`).

## Orchestrator use each cycle

1. Run `pin-watch.ps1 -- --since-last --json`; branch on the exit code (or `verdict` in the JSON).
2. Act per verdict:
   - **0**: nothing. (With `--since-last` the report says `previous_verdict`; if that was 20/30 and nobody acted, the
     skip does NOT re-alert: track the open bump task, or force a fresh look with a run without `--since-last`.)
   - **10 additive-safe**: no code change needed. Optionally bump the pin when convenient (regenerate wire with
     `core-bridge/scripts/gen-wire.ps1`, add `contracts/agent_core/pin.json`, ADR); otherwise note and continue.
   - **20 needs-bump-work**: open a pin-bump task. Hand the implementer the report's `our_files_likely_to_change`,
     the signals, and `check_details.wire_diff`. Follow ADR 0008's recipe: update pin, `gen-wire.ps1`, fix
     `compat.py` symbols, factories/app wiring, env in Dockerfile and e2e stack, platform-sim/registry_mock, then
     full tests. New env vars requiring `local/compose.yaml` changes go to Codex.
   - **30 breaking**: do NOT auto-bump. Escalate: read `check_details.compat.output` / `gen_wire.output`, contracts
     VERSION change and removed routes; agree the contract change with the agent-core owners (Codex) before work.
   - **1**: fix the environment (`gh auth status`, network) and retry; never treat as "no change".
3. Open-PR hints (`agent_core.open_prs[*].verdict_hint`) are an early warning: raise them with agent-core owners
   before they merge when a hint is 20/30.
4. llm-gateway: first run only records the baseline (use `--since-last` or `--llm-baseline SHA`). Later, changes to
   its API/contracts/env give `needs-bump-work`, other changes `additive-safe`; adapt `HttpLLMGateway` usage /
   `AGENTCORE_LLM_GATEWAY_*` and the e2e llm double.

## Tests

```powershell
uv run --python 3.12 --no-project --with pytest python -m pytest scripts/core/pin-watch/tests -q
pwsh -NoProfile -Command "Invoke-Pester -Path scripts/core/pin-watch/tests/pin-watch.Tests.ps1"   # Pester 3.x ok
```

Pytest uses fake `gh` output (golden JSON in `tests/golden`), a local fixture git repo for the scratch checkout, and
the exit-code contract. Heuristics are regex on GitHub patch text: they flag, a human confirms.
