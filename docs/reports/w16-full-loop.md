# W16: full value loop, live pass (2026-10-05)

Aggregates and proposal slugs only. No customer row, no secret, no token and no model text is in this report. Branch `claude/w16-full-loop-release`
(chain b3 -> w9 -> reg1 -> w14 -> w12 -> w11 -> bld1 -> w13 -> prb1 -> ev3 -> o11y1 -> o11y3 -> align1 -> ann1 -> out1 -> demo1 -> map1, then inh1, engo, main).

## Stack

Own stack `pulso-w16` (Postgres :55610, llm-gateway :8310, agent-core :8311, engine :4310), `scripts/demo-loop/run.ps1`. agent-core: LOCAL scratch branch
`scratch/w16-main-pr50-pr51` = origin/main (`5e3fef9`) + PR 50 (evaluate candidate prompts) + PR 51 (constructor release settings / eval drafts), never pushed.
llm-gateway `bdd9a9d`. Credentials only through the scrubbed helper into child environments. Models: Scout `xiaomi/mimo-v2.6-flash`, Verifier
`xiaomi/mimo-v2.6-pro`, Builder `xiaomi/mimo-v2.6-flash` with `pro` as the escalation tier, judge `z-ai/glm-5.3-flash`. Cells: the cached REAL bank cells
(7,995 treated rows, sha256 prefix `739cde15ff17`). `-Up` 56.5 s; `-Cells -Loop -Show` 790.8 s (Loop 787.1 s, 19 findings). Nothing was approved, published or promoted.

## The loop on the real bank cells (all 19 corroborated findings, no cap)

Corroborated 19, reasoned 19, proposed 10, delivered 9, **announced 9**, not announced 1 (`not_fixed`), unlinked 9 (7 `dependency_metric`, 2 `dependent_on_m1`),
human-owned 0, blocked 0. Model cost USD 0.01353 for the whole run (about USD 0.0013 per announced proposal). Builder escalation to `pro`: 0 (flash sufficed).

| finding | metric | cell | outcome | candidate tried | rubric (engine, structural) | cost USD | model latency s |
|---|---|---|---|---|---|---|---|
| 1 | M1 | Phone x Comercial | announced | new_agent:consultas | 23/24 | 0.00195 | 120 |
| 2 | M1 | Phone x Queja | announced | template:t/estado_pqr | 23/24 | 0.00113 | 32 |
| 3 | M10 | Phone x Queja | unlinked:dependent_on_m1 | - | - | 0 | - |
| 4 | M1 | Email x Queja | announced | template:t/estado_pqr | 23/24 | 0.00117 | 63 |
| 5 | M1 | Phone x Tecnico | announced | new_agent:consultas | 23/24 | 0.00110 | 58 |
| 6 | M1 | Phone x Retencion | not_announced:not_fixed | new_agent:consultas, then prompt:p/copiloto (`suite_refused`) | 23/24 | 0.00256 | 36 |
| 8 | M1 | App x Queja | announced | template:t/estado_pqr | 23/24 | 0.00118 | 69 |
| 10 | M1 | WhatsApp x Queja | announced | template:t/estado_pqr | 23/24 | 0.00108 | 37 |
| 11 | M1 | Web Chat x Queja | announced | template:t/estado_pqr | 23/24 | 0.00106 | 53 |
| 17 | M1 | Email x Comercial | announced | new_agent:consultas | 23/24 | 0.00124 | 38 |
| 19 | M1 | App x Comercial | announced | new_agent:consultas | 23/24 | 0.00106 | 28 |
| 7, 9, 12, 13, 14, 16, 18 | M6 | various | unlinked:dependency_metric | - | - | 0 | - |
| 15 | M10 | Phone x Retencion | unlinked:dependent_on_m1 | - | - | 0 | - |

Proof verdict story: every announced finding is `regression_suite_proven` (the suite fails on the base in 8 of 8 cases for the `estado_pqr` patches and
6 of 6 for the new agents, and passes on the candidate at the first attempt). Finding 6 (Retencion) is `not_fixed` on the new agent (the base fails 6 of 6, the candidate
fails 6) and its second candidate is `suite_refused`: it stays internal, as designed. A dossier ES was produced for each announced proposal and read back from agent-core
as `state=draft origin=auto_detect created_by=pulso-engine`.

**The new-agent path now announces.** With PR 51 in the scratch agent-core, the four new-agent findings (Comercial x3, Tecnico x1; new specialists of 16 changes) are announced; before it they ended
`not_announced:infra_failed (put_draft forbidden_role HTTP 403)`. The INH1 `inherit_from` (donor release id) is present on the `release_settings` change; read back on finding 1 (agent-core went
down before the other three could be re-read).

## Rubric with the glm judge (`scripts/scoring/score_proposal.py`, judge z-ai/glm-5.3-flash, other family than the Builder)

The offline scorer needs a proposal in its own shape, so the same findings were re-reasoned once with `reason_cli --mode live` on the fixture baseline (USD 0.0116;
9 proposed; finding 5 hit a transient `model_unavailable`, finding 6 compiled), not the byte-identical announced drafts. All 9 were scored:

| | mean | range |
|---|---|---|
| judged R1,R2,R8,R9,R10,R12 (max 12) | 8.6 | 7 to 10 |
| mechanical R3,R4,R5,R11 (max 8) | 7.6 | 7 to 8 |
| total of 24 | 16.1 | 15 to 18 |

Every verdict is `reject` for one reason: R6 (evaluation exists) and R7 (evidence resolves) are 0 because the offline scorer was not given the regression suite or the evidence
store of the loop (the loop proves them with the suite verdict, not with this scorer). Read the judged and mechanical sums, not the verdict. R2 was 1 in 9 of 9 and R9 was 1 in 8 of 9 (a stable judge finding about
the expected-effect and measurement wording). The judge returned `gateway_http_504` in 6 of 14 first-pass calls (glm reasoning latency); a retry succeeded for all.

## Observability (ENGO)

35 model calls recorded (scout 11, verifier 11, builder 13), 34 answered and 1 `invalid`, 10 stories (one trace id per finding that reached the roles), 35 unique
`(evidence_ref, role, n)` keys, summed call latency 594.6 s, USD 0.01353 equal to the metering. They are in `<work>/value-loop/job-0.calls.ndjson` and served by `GET /internal/v1/debug/runs/{id}/model-calls`.
**Defect found and fixed in this merge:** MAP1 tries more than one candidate per finding and every attempt numbered its calls from 1 per role, so the second candidate's calls collided with the first on the store key
and on the generation span id; `append_calls` in `value_loop.rs` now continues the numbers across the attempts of a finding (test `every_model_call_of_a_trigger_job...`).
**Not verifiable here:** that `traceparent` reached the gateway. The stack has no OTLP collector (the gateway exports to :6006 and is refused) and the gateway request log carries no trace id; only the engine side is
tested (`core-client` trace tests, `scripts/o11y`). Stage span ids still repeat per `(stage, attempt)` across the candidates of one finding.

## Acceptance (Codex, read-only `check_proposal.py`, copied outside git)

9 of 9 announced drafts: 0 failures; `lifecycle_history` and `quota_boundary` are `not_exercised` (no history or quota URL given). The script expects the proposal object to carry
`changes`; this agent-core returns `{proposal, changes}`, so the changes were folded into the proposal object before calling its `check_acceptance` (the script itself was not modified; its plain
HTTP mode reports 5 false failures on this response shape).

## OUT1 outcome step (pseudo-release; real cells, Codex T1 estimator copy outside git)

M1 Queja x Phone (the cell of the announced finding 2; the finding record was written by the live test, not taken from this run's engine), controls = other reasons on Phone, 3+3 months:
5 pseudo-releases (2024-06, 2024-12, 2025-06, 2025-12, 2026-02): **5 of 5 inconclusive** (halves disagree), effects +0.02, -1.67, +0.84, +1.81, +0.25 pp, interval half-width about 2.3 pp, none called improved.
SYNTHETIC planted effect (labelled on the card): numerator cut by 8 pp after 2025-06 gives `improved`, -7.16 pp (interval -9.42 to -4.89), `success_claimed: true`. It proves the `improved` path only.

## Tests of the integrated tree

`cargo test -j 1` green on steps, reasoning, registry-writer, pulso, engine, debug-api, maturity, core-client. Python: aggregate 26, scoring 99, battery 30, regression 74, o11y 49, triggers 15, dev-stack 7,
ann1 4, out1 5, owners 5 (one real failure fixed: two journal 0658 globs overlapped), demo-loop Pester 38; dataclass gate push-scan clean.

## Limits, honestly

* Outcome verdicts are mostly inconclusive on stationary data; there was no real release. Pseudo-releases only.
* Platform evidence links in the announce are opaque ids (`CASE-` + 26 characters); no real case.
* Routing from `recepcion` to a new agent is a human-owned follow-up and is not part of any proposal.
* agent-core PRs 48 to 51 are needed for the full effect (new-agent announce needs 51, candidate-prompt evaluation 50) and are open.
* The regression judge is deterministic and `uncalibrated`: it proves the suite discriminates, not that customers benefit. The dossier of the new agents labels its runtime "dobles" (no real model call in their suite).
* The loop rubric (23/24) is a structural self-score; the glm score (16.1 of 24 mean) is on re-reasoned proposals and lacks the R6/R7 inputs.
* One run, one Builder sample per finding: variance not measured. Cells are the cached bank aggregation (snapshot 2026-06-18), `claude-standin` sensor.
