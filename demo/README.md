# demo/ - the 'demo magica' on our side (Codex stand-in, honestly labelled)

Plan section 2 (10 steps) made visible end to end **without** the Rust engine: the e2e-core Codex stand-in drives the REAL `real_local` Core
stack (scripted LLM double, bank/lab fixtures), a data-derived analysis supplies the findings, and translators turn the REAL outputs into the
debug-console fixture-server world.

## One command

```powershell
pwsh demo/run.ps1             # unit tests -> real_local stack on pulso-dev -> scenario -> teardown (try/finally) -> demo/out/*
pwsh demo/run.ps1 -Offline    # no stack: SYNTHETIC Core values (labelled offline_core in doubles[]) for a quick console preview
pwsh demo/run.ps1 -Serve      # additionally serve the produced world on :4010 (Ctrl+C to stop)
```

`-Keep` leaves the stack up; `-Namespace`/`-BaseImage` as in `e2e-core/run.ps1`. Needs the pinned venv `%TEMP%\pulso-wire-venv-<pin7>`.

## Watch it in the console

The console itself is untouched. `demo/serve/serve-demo.mjs` runs a rewritten COPY of `debug-console/fixture-server/server.mjs`
(3 anchored replacements, it fails loudly if the console drifts) that serves `demo/out/world.json` as scenario `demo`.

```powershell
node demo/serve/serve-demo.mjs demo/out/world.json demo/out/replay.json   # fixture API/SSE on :4010
cd debug-console; npm ci; npm run dev                                      # Vite on :5173 proxies to :4010
# or the built image: build debug-console, run it with PULSO_CONTROL_API_URL=http://host.containers.internal:4010
node demo/serve/play.mjs demo/out/world.json                               # optional: replay the run live over SSE
```

Open run `run-demo` (main path), `run-demo-refuted` (refuted hypothesis kept visible, ends in "Do nothing") and `run-demo-successor`
(planned only). The profile banner lists every double.

## What is real and what is not (`out/demo-report.json` -> `doubles[]`)

| Piece | Real | Double |
|---|---|---|
| Core runtime, registry, native evaluation, writer receipts, audit chain, exporter, chain verification | yes (real_local stack) | |
| Engine (Scout/Verifier/Builder orchestration) | | Codex stand-in (`e2e-core`), until the Rust engine HTTP API exists |
| LLM | | scripted gateway; its tool calls/outputs are built from the data-derived analysis |
| `lab_query` rows | | the e2e-fixtures lab double returns fixed rows; the SQL that produces the findings runs on the demo's sqlite dataset (SQL text + digests recorded and sent through the real tool) |
| Improvement gate / revision | | demo's mechanism_proxy judge (stand-in for the Codex judge) |
| Console API | | fixture API/SSE fed from `world.json` |
| Human approve/publish | | hook only (`src/pulso_demo/decision_hook.py`, `PendingHook`), never approves |

Nothing is precomputed as the winning finding: `analysis.scout` ranks measured excess abandonment, `analysis.verify` re-tests per week and
refutes the decoy, `analysis.judge/revise` simulate candidates on the data (candidate 1, a global retry raise, breaches the guard; the
bounded revision narrows the scope and passes). `tests/unit/test_analysis_first_red.py` proves that a dataset without the latent mechanism
yields no such hypothesis. Mechanism proxy only: no causal SLA or bank-saving claim.

Plan steps 8 (human) and 9 (staging/exposure) are reported `hook_pending` / `not_run`; step 10 reports the exporter delivery and chain
verification, and the successor run is only planned (not started by the stand-in).

## Connecting local-identity

Implement `DecisionHook.decide(DecisionRequest) -> DecisionResult` (signed approval receipt in `receipt_ref`) and pass it to
`driver.main`'s hook (one line in `driver.py`); the translator must then read approve/publish nodes from REAL receipts only.

## Layout and tests

`src/pulso_demo/{dataset,analysis,bundle,translate,driver,decision_hook}.py`, `serve/{serve-demo,play}.mjs`, `tests/golden/{results,world}.json`
(regenerate with `python tests/make_golden.py`). Tests: `pytest demo/tests` (runs automatically at the start of `run.ps1`).

## Console changes that would help (none are required to watch the demo)

See the list in the delivery report: alternatives panel, per-attempt gate history, multi-hypothesis investigation, `world.demo` driven profile,
`/__fixture/load`, per-run diff/gates routes, decision hook states.
