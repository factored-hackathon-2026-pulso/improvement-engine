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

## Scripted vs derived (read this before believing the demo)

Derived from the data (mutation-tested: swapping the planted mechanism to another flow moves the finding; no mechanism, noise and a second
mechanism change hypotheses, verdicts and the candidate; planting the guard breach in another segment changes the revision, no breach means no
revision, an unfixable breach is a bounded stop; see `tests/unit/test_driver.py`, `tests/unit/test_revision.py`): scout hypotheses (measured excess
abandonment, lower confidence bound), verifier verdicts (per-week stability + pooled test; the device-mix decoy is refuted by that test, not by
name), **candidate 1** (scope = the flow of the top SUPPORTED otp_verify hypothesis, `max_retries` = current + the max extra attempts exhausted users
needed in that flow, capped by the policy ceiling; no supported hypothesis means no candidate, outcome `no_opportunity`, gate `hold`; a candidate on
a flow without a supported hypothesis is refused by `check_supported`), the gate verdicts on each candidate, the **structured guard breach** (metric,
direction, observed vs limit, magnitude, per-device exposure, affected segment), and the **revision**, which is steered by that breach: exclude the
affected segment, then (if nothing is left to exclude) lower the retry ceiling by one, never leaving the supported flow, at most `--max-revisions`
(default 2). `world.demo.attempts[]` records failure (`failure`) -> rationale (`revision.rationale`) -> ChangeSpec delta (`revision.delta`), so the
console shows WHY candidate 2 differs. Only a `guard_breach` is steerable; `insufficient_lift` or an exhausted bound stops with step 7 `failed`
and nobody is asked to approve. The second-batch contradiction and the successor target are also derived.

Scripted / canned (honest limits):
* The dataset is synthetic and its generator PLANTS the mechanism (OTP retry exhaustion in one flow, a decoy incident) and the second-batch change
  (`dataset.py`). The pipeline finds what was planted; it is not evidence that the method works on real data.
* The candidate SHAPE is fixed (a retry policy on `otp_verify`: scope, excluded segments, `max_retries`), the policy ceiling (5) and the guard limit
  are constants, and the revision menu is two rule-based moves (exclude segment, lower ceiling), not an LLM reviser. The judge's mechanism_proxy
  reads the planted `retry_exhausted/extra_needed/risky` columns; the breach is segmented by device and cohort only, and the planted default
  breach is spread over devices (mobile carries slightly more), so which segment gets excluded is a measurement, not a story. The `risk_hot`
  dataset option exists only for the mutation tests.
* Revision is triggered only by a failing improvement gate with a steerable (guard_breach) failure; if the bounded revision does not clear it the
  report says `no_candidate_passed_gates` and nobody is asked to approve. No-human / pending / rejected outcomes are reported as such
  (`outcome` = `awaiting_human_decision`, `rejected_by_human`, `no_opportunity`, ...), never as ok.
* The LLM is scripted (answers built from the analysis); `lab_query` rows from the e2e double are fixed, the SQL that matters runs on the sqlite dataset.
* Step 5 and 9 are `real` only because the Core registry receipts / staging alias READ come from the real stack; with `--offline` they are `simulated`.
  Step 8 is `simulated` in scripted mode (a labelled simulated human), `real` only for `--human-mode manual` (identity is still the sandbox issuer).
* A manual-mode decision file is bound to proposal id + candidate hash, consumed on read, and a decision file predating the request is discarded.

Human step (plan: authority only): the human decides ONLY `approve` (operation and candidate hash fixed by a durable single-use intention; approved is
not published) -> `publish` -> staging confirmed by an
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
