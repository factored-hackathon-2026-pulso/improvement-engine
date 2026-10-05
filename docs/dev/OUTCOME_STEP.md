# OUT1: the outcome step (plan item W2-1, "did it work?")

The last beat of the demo. When a release of an agent happens, the engine says whether the problem rate it was announced for moved, honestly: `improved`, `no_detectable_change`, `worsened` or `inconclusive` (underpowered included). It never says "caused", and it never says success without `improved`.

Code: `seams/crates/pulso/src/run/outcome.rs` (+ `engine_job.rs`, `mod.rs` wiring). Tests: `seams/crates/pulso/tests/outcome_step.rs` (16, offline, a scripted estimator double as a subprocess), `scripts/out1/test_outcome_cli.py`, live (ignored) `tests/live_out1.rs`.

## Flow

1. A `release.published|promoted|revoked` trigger (`kind = outcome`) is admitted by `POST /internal/v1/automation/triggers` (see `seams/crates/debug-api/TRIGGERS.md`). The whitelisted subject keeps `release_id`, `proposal_id`, `agent`; `event.at` gives the release month.
2. The worker runs the keyed job `trigger:<key>`. The runner reads the trigger back from the in-process automation audit run (`find_trigger`). A `release.*` outcome trigger runs the OUTCOME step and not the value loop (any other trigger behaves as before). Without the step configured the job records `outcome: {skipped: ...}`.
3. Proposal to finding: the value loop's readable record `<work>/value-loop/<job>.json` keeps, per finding, `delivery.proposal_id`, `metric` and (new, additive) `dims`. The finding(s) whose `delivery.proposal_id` equals the event's `proposal_id` are the ones the release addressed. None found or no `proposal_id`: card state `unlinked` (no estimator call).
4. Table: the treated cell and its sibling controls (same metric, same channel, other reasons; for `category`/`channel`-only metrics the same rule on the first of `reason_category, category, channel`), `N` months (default 3) before the release from the PRE cell tables and `N` after from the POST cell tables; the release month is in neither window. Aggregates only (`bank_cells.py` NDJSON rows, 6 fields). Written to `<work>/outcome/<release_id>.<n>.cells.ndjson`.
5. Estimator: a subprocess, see the contract below.
6. Verdict card in `<work>/outcome/<release_id>.json`, a summary (`outcome_card`) on the finding record, and a console run `outcome-<job>`.
7. Idempotent per release id: a stored card is returned (`replay: true`), the estimator is not called again, whatever the event (`published`, then `promoted`...). `revoked` on a measured release marks `revoked: true`; on an unmeasured one the state is `revoked_not_measured`.

## JSON CLI contract (`pulso.outcome.v1`)

```
$PULSO_OUTCOME_CMD --cells <ndjson> --treated <json file> --release-date YYYY-MM --window-months N
```
- `--cells`: bank_cells NDJSON rows `{metric, dims, half, period, numerator, denominator}` (halves `discovery` / `holdout`): treated + controls, both windows.
- `--treated`: `{"metric": "M1", "dims": {"reason_category": "Queja", "channel": "Phone"}}`.
- stdout, one JSON object, exit 0: `{"contract": "pulso.outcome.v1", "release_period", "window_months", "cells": [ {metric, dims, status, reason, effect_pp, interval_family_adjusted_pp: [lo, hi], n_pre, n_post, control_dimension, control_siblings: [...], uncertainty_method} ]}`. `dims` of the answered cell are the CALLER's labels. `effect_pp` is the difference in differences in percentage points (negative = the rate fell; every bank metric is "higher is worse").
- `status` is the closed vocabulary `improved | no_detectable_change | worsened | inconclusive`; `reason` carries `underpowered_minimum_support`, `half_directions_or_intervals_disagree`, `incomplete_*`, `no_published_sibling_control`.
- Engine-side guards: contract mismatch, a status outside the vocabulary, a missing/ambiguous treated cell, a crash, a non-zero exit, a timeout (`PULSO_OUTCOME_TIMEOUT_SECS`, default 120, the process is killed) or non-JSON output all become an `inconclusive` card with `reason: estimator_failed:<code>` (never a verdict, never a failed job). An `improved` whose effect and whole interval are not below zero (or a `worsened` not above zero) is downgraded to `inconclusive` with the claim in the caveats.

Configuration (environment of `pulso run`): `PULSO_OUTCOME_PRE_CELLS` (default `PULSO_CELLS_NDJSON`; turns the step on), `PULSO_OUTCOME_POST_CELLS` (default: the pre tables), `PULSO_OUTCOME_PSEUDO_RELEASE` (`YYYY-MM`, historical data), `PULSO_OUTCOME_WINDOW_MONTHS` (1..12, default 3), `PULSO_OUTCOME_CMD` (default `python -m scripts.aggregate.outcome`), `PULSO_OUTCOME_CWD`, `PULSO_OUTCOME_TIMEOUT_SECS`, `PULSO_OUTCOME_DATA_LABEL` (e.g. `synthetic-planted-effect`: on the card, in the caveats and in the dossier), `PULSO_OUTCOME=off`.

## Plugging Codex's estimator (T1)

Codex's T1 (not merged; read from `worktrees/improvement-engine-t1-t5-consolidated`, nothing copied into this tree) exposes `estimate_outcomes(rows, release_period, window_months=...)` in `scripts/aggregate/outcome/outcome_estimator.py` and a CLI `--data-root --out` that runs its placebo/sensitivity validation. It has NO verdict CLI. So:
- `scripts/out1/outcome_cli_adapter.py` is the reference adapter of the contract above over `estimate_outcomes`. It contains no estimator code; it imports the estimator relative to the working directory. Run it with `PULSO_OUTCOME_CMD="python <repo>/scripts/out1/outcome_cli_adapter.py"` and `PULSO_OUTCOME_CWD=<dir holding scripts/aggregate/outcome and docs/data/opbench>`.
- The estimator folds labels into a closed vocabulary (`Queja` -> `complaint`, `Phone` -> `phone`, several reasons into `unclassified`) and returns its cells in those labels. The adapter looks the treated cell up under the same folding and answers under the caller's labels; the estimator's labels stay in `estimator_dims` and `control_siblings`.
- Once T1 merges with the same flags, set `PULSO_OUTCOME_CMD` to it and drop the adapter. `scripts/out1/outcome_double.py` is the scripted double used by the tests (not an estimator).

Design note (a disagreement to settle): the brief asked for a second control by customer-hash half. Codex's T1 treats the halves as REPLICATION cohorts that must agree (both directions and intervals), not as untreated controls, because both halves are exposed to the release. The engine follows T1: the card says so in `controls.kind`.

## What the card holds

`verdict`, `reason`, `underpowered`, `effect_pp`, `interval_pp` (family-adjusted, Bonferroni over the cells passed), `n_pre`, `n_post`, `controls` (dimension and the sibling cells used), `window`, `period_kind` (`pseudo_release_historical` | `live_release_event`), `power_note`, `caveats` ("association, not cause"; one release, one cell, short window; pseudo-release; the interval treats contacts as independent and is therefore too narrow; underpowered is valid and expected), `dossier {es, pt}` (short, platform style; a success sentence exists only for `improved`), `success_claimed` (true only for `improved`), `data_label`.

## Live result (2026-10-05; real estimator copy outside git, real bank cell tables, aggregates only)

M1 contact_unresolved_rate, Queja x Phone, controls = the other reasons on Phone. The data are stationary and no release happened: every date below is a PSEUDO-release.

| pseudo-release | verdict | effect pp | interval pp | n pre / post |
|---|---|---|---|---|
| 2024-06 | inconclusive (halves disagree) | +0.02 | -2.25 .. +2.30 | 8259 / 8508 |
| 2024-12 | inconclusive (halves disagree) | -1.67 | -3.97 .. +0.63 | 8174 / 8147 |
| 2025-06 | inconclusive (halves disagree) | +0.84 | -1.42 .. +3.09 | 8397 / 8604 |
| 2025-12 | inconclusive (halves disagree) | +1.81 | -0.47 .. +4.08 | 8312 / 8357 |
| 2026-02 | inconclusive (halves disagree) | +0.25 | -2.03 .. +2.53 | 8381 / 8237 |

Five of five stationary pseudo-releases are inconclusive and none is called an improvement: this is the honest expected result, and the interval half-width (about 2.3 pp) says which effects this cell could ever detect in a 3+3 month window.

SYNTHETIC planted effect (labelled as such on the card): the same tables, but the post tables have the treated cell's numerator cut by 8 pp of its denominator after 2025-06. Verdict `improved`, effect -7.16 pp, interval -9.42 .. -4.89, `success_claimed: true`, dossier ending "DATOS SINTETICOS: efecto plantado para probar el flujo, no es un resultado real." This only proves the `improved` path.

Regenerate: copy the estimator files to a scratch directory (outside git) with `cells.ndjson`, then
`OUT1_LIVE_SCRATCH=<dir> OUT1_LIVE_OUT=<dir>/out cargo test -j 1 -p pulso --test live_out1 -- --ignored --nocapture`.

## Data gaps and what to ask

1. Platform events carry no `release` field, so the engine cannot split real contacts into before/after a real release. ASK (platform): a `release` (id or version) on the case/contact events the exporter publishes.
2. A run does not carry the agent alias (`agent_id/alias/before` are added to release events by agent-core PR 49, https://github.com/pulso-factored/agent-core/pull/49; not on runs). ASK (agent-core): `alias` and the release id on `run_started`, so a contact can be attributed to a release.
3. Lineage: `GET /runs/{id}/lineage` exists on agent-core main (PR 48) but the engine does not read it yet. The live path (read lineage per run, join platform events by run id, build the post tables from real post-release contacts) is NOT built; the post tables are passed in (`PULSO_OUTCOME_POST_CELLS`). Until then a card on bank data is a pseudo-release and says so.
4. The release trigger needs `proposal_id` in its subject (it is on the whitelist) and `event.at`; a release without `proposal_id` is `unlinked`. ASK (agent-core): `proposal_id` on `release.published|promoted|revoked` for releases born from an engine proposal.
5. Trigger records live in memory (TRIGGERS.md): after a restart the runner cannot read the trigger back from the audit run and the job falls back to the old behaviour. ASK: persist the trigger body with the keyed job (a payload column on the job store).
6. Codex T1: a verdict CLI with the contract above (or accept the adapter); a published minimum detectable effect per cell size so the card's power note can quote it instead of the interval half-width.
7. The bank tables are stationary and the cells are small outside Phone: most real cards will be `inconclusive`, many `underpowered`. That is the result, not a bug.
