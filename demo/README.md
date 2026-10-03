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

The console itself is untouched and loads the produced world directly (the old `serve/serve-demo.mjs` wrapper is DEPRECATED):

```powershell
$env:FIXTURE_WORLD_FILE = "demo/out/world.json"; node debug-console/fixture-server/server.mjs   # fixture API/SSE (or POST /__fixture/load)
cd debug-console; npm ci; npm run dev                                                           # Vite proxies to the fixture server
node demo/serve/play.mjs demo/out/world.json                                                    # optional: replay the run live over SSE
```

`world.json` carries, besides the console fixture shape, `investigation[run].hypotheses` (hypothesis_id, statement, verdict, evidence_refs),
`gates_by_run`, `demo.human`, `demo.decision_timeline` and `demo.successor`.

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
| Human approve/publish | Core registry approve/publish/alias reads, JWS verified by Core | the human (scripted = simulated) and the sandbox human issuer (IdP double) |

Nothing is precomputed as the winning finding: `analysis.scout` ranks measured excess abandonment, `analysis.verify` re-tests per week and
refutes the decoy, `analysis.judge/revise` simulate candidates on the data (candidate 1, a global retry raise, breaches the guard; the
bounded revision narrows the scope and passes). `tests/unit/test_analysis_first_red.py` proves that a dataset without the latent mechanism
yields no such hypothesiapproved is not published) -> `publish` -> staging confirmed by an
alias READ. Prod is never touched unless `--promote` is given. The console world shows decision requested (`waiting_dependency` /
`human_decision_pending`) -> approved -> published -> staging confirmed, with receipt refs (intention/command ids, proposal rev, release id, aliases).

* `--human-mode scripted` (default): a labelled SIMULATED human supervisor approves; the report lists the human as simulated.
* `--human-mode manual`: the driver pauses (`--human-timeout`, default 900 s; then the decision stays pending) until a person runs
  `python -m pulso_demo.decide --out demo/out approve|reject` (bound to the pending proposal id and candidate hash, consumed on read).
* `--offline` runs the same flow on labelled synthetic ports (`offline-*` ids).

Step 10: after staging is confirmed, a second batch of scripted observations (post-change dataset) goes through a real Core scout run and the real
exporter -> ingest fixture (second batch + chain verification); the stand-in engine re-measures memory claims by SQL (the transfer_limit claim is
CONTRADICTED, no causal attribution) and starts a successor investigation. `demo-report.json` marks every step `real|stand-in|simulated` plus an outcome.

## Layout and tests

`src/pulso_demo/{dataset,analysis,bundle,translate,driver,decision_hook,human_flow,human_live,offline_ports,decide}.py`, `serve/{serve-demo,play}.mjs`, `tests/golden/{results,world}.json`
(regenerate with `python tests/make_golden.py`). Tests: `pytest demo/tests` (runs automatically at the start of `run.ps1`).

## Console changes that would help (none are required to watch the demo)

See the list in the delivery report: alternatives panel, per-attempt gate history, multi-hypothesis investigation, `world.demo` driven profile,
`/__fixture/load`, per-run diff/gates routes, decision hook states.
