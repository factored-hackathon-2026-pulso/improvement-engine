# Scheduled probes: a failing agent probe becomes a detection signal (PRB1, plan W1-5)

Loop: **scheduled battery -> trigger -> cells -> finding -> reasoning -> proposal**.

```
run_battery.py (agents x scenarios x 3 reps) --result JSON-->  schedule_probes.py
   flake rule + persisted state ----> pulso.trigger.v1 (kind scheduled) --POST--> engine /internal/v1/automation/triggers
   probe_cells.py: cell table P1..P3 (ndjson) --> probe findings (level_risk-like) --> [reasoning --> proposal]
```

## Files

| Path | Role |
|---|---|
| `scripts/battery/schedule_probes.py` | once/scheduled run of the battery (`--run`) or consume a result (`--result`); builds and posts the trigger |
| `scripts/battery/probe_cells.py` | flake-aware verdicts, cell table, findings, state (stdlib, pure) |
| `scripts/battery/tests/test_probes.py` | offline tests on the RECORDED results in `agent-core-assets/eval-battery/results/` |
| `scripts/battery/demo_core.py` | own local stack; `PULSO_STACK_PREFIX`, `PULSO_STACK_PORT_{CORE,PG,GW}` (defaults = EV2) |

## Flake rule (pre-registered, in `probe_cells.py`)

The Understand model is an LLM: one utterance flips on a few percent of runs (EV2: 2 of 111 amount runs).

1. A scenario **fails in a run** when it fails in >= ceil(2/3 x valid reps) repetitions (>= 2 of 3). Repetitions that ended in
   a transport error (`... http NNN`) are not valid; fewer than 2 valid repetitions = `inconclusive`, never a failure. A
   behavioural error (`confirm step without a pending confirmation`) is a failed repetition. 1 of 3 = `flaky`: reported, never
   a trigger.
2. A scenario is **confirmed** when it failed in this run AND in the immediately previous run (2 consecutive runs).
3. The same confirmed scenario set is **not re-triggered** (state `triggered_digest`); a changed set triggers again; an empty
   set resets the digest, so a recurrence after recovery triggers again. A result file with the same `finished_at` is not a
   new run (re-reading never counts twice).
4. `policy_divergence` (amount 500 vs 250, human-owned) is not a failure.

State (`--state`, JSON, atomic write): `prev` verdicts of the last run, `triggered_digest`, `runs`, `last_result_at`. No
credentials, no text.

## The trigger request

`pulso.trigger.v1`, `kind: scheduled`, `event.type: schedule.tick`, `event.ref = probe:<sha of the confirmed set>`,
`trigger_key = sha256([tenant, mission, "agent-battery", config_digest, "scheduled", ref])` (same function as
`scripts/triggers/agentcore_poller.py`), sent as `Idempotency-Key`; CSRF from `GET /api/v1/auth/session`, bearer from
`PULSO_ENGINE_TOKEN` (process environment). `config_digest` hashes the floors and the rule, so changing either is a new trigger
scope. Extra top-level fields: `evidence_class: probe_synthetic` and `probe_cells`, ONLY the treated cells of corroborated
findings: `{schema pulso.probe_cell/1, kind probe, label probe, evidence_class, metric, agent, scenario_family, outcome fail,
n_failed, n_scenarios, confirmed_scenarios}`. No scenario id, no reply text, no free text.

Engine fact (TRIGGERS.md): the endpoint stores only whitelisted `subject` fields and does not persist other body fields; they
are accepted (and covered by the idempotency digest) but the engine does not yet read `probe_cells`. Until a follow-up in
L-CAPI persists them, the cell table reaches the value loop as a file (`--cells-out`, `PULSO_CELLS_NDJSON`).

## Sensor side: probe cells as a source

Cell-table rows (the `steps_cli cells` schema of `scripts/aggregate/bank_cells.py`, bank convention "higher is worse"):

| Metric | Cell | numerator / denominator | Pre-registered floor |
|---|---|---|---|
| P1 `probe_fail_rate_scenarios` | agent x scenario_family | scenarios failing in the run / scenarios | pass >= 0.80 (fail rate <= 0.20) |
| P2 `probe_fail_rate_agent` | agent | same | pass >= 0.90 |
| P3 `probe_fail_rate_reps` | agent x scenario_family | failing valid reps / valid reps | none (flake meter) |

`half = discovery` is this run, `holdout` the previous run (the replication slot). A pass rate below its floor is a
**level risk against a threshold**, not a vs-rest contrast, so the finding type is `level_risk`: `status` `candidate`
(first time, or the previous run was above the floor, or other scenarios failed), `corroborated` (below the floor now and in
the previous run with >= 1 confirmed scenario in the cell), `refuted` (at or above the floor). The record has the keys of the
Rust `level_json` on branch `claude/w14-level-risk` (`type level_risk`, `class risk`, `claim association`, `discovery` with
`rate`, `baseline_rate` = fail ceiling, `diff`, Wilson interval) and the contrast keys (`status`, `direction`, `discovery`,
`holdout`). `normalise_signal()` reads a contrast signal or a level_risk signal into one shape, so the mapping works with or
without w14 merged (w14 is read, not merged). No p-value is produced: scripted runs are not a sample, the rule above replaces
the test.

k rules and evidence class. Probe cells are synthetic: no customer is in a cell, k-anonymity is `not_applicable_synthetic`
(replaced by >= 2 valid repetitions). Every row, finding and trigger carries `evidence_class: probe_synthetic` / label `probe`,
and `combine_reports()` keeps real findings and probe findings in separate lists with separate counts (it raises when a probe
signal is in the real list or the reverse). Never pool them.

What the Rust cells sensor would need (not done: heavy build, outside this lane): `agent` and `scenario_family` in
`ALLOWED_DIMS`, a probe config (`k_min 1`, `min_support` of a few scenarios) and a `level_risk` spec per P metric.
`to_ndjson(strict_schema=True)` already emits exactly the allowed fields. On branch w14 the reasoning roles skip `level_risk`
signals (`skipped.status = level_risk`), so a probe finding reaches reasoning through a probe branch of the value loop
(follow-up) that carries `evidence_class` into the Scout input and the proposal rationale ("synthetic probe", never "customers").

## Acceptance: the real defects of the demo agents as probe findings

LIVE on an own stack (prefix `pulso-prb1`, ports 8120/55496/8121; real gateway and JEV; engine = `debug-api.exe` of b3-wire on
:4121 with its in-memory admitter, not `pulso run`; credentials only in the process environment), `schedule_probes.py --run
--reps 3`, two runs:

* run 1: `action none`, 5 findings, all `candidate / no_previous_run`; 0 triggers.
* run 2: `action triggered`, 5 findings `corroborated / replicated_in_previous_run`; the engine listed 1 trigger
  (`kind scheduled`, `state admitted`, `job-0`); replaying the same result: `skip_already_consumed`, no second trigger.

| Finding | Cell | Real defect |
|---|---|---|
| P1 corroborated | consultas x attacker:vague_customer (1/1) | `radicado` slot accepts any text and answers "Ya consulte tu PQR" |
| P1 corroborated | consultas x attacker:language_switch (1/1) | es -> pt switch not honoured |
| P1 corroborated | disputas x attacker:language_switch (1/1) | same |
| P2 corroborated | consultas 2/7, disputas 1/7 | agent-level view (floor 0.90) |

Offline the same cells come from the recorded base result (tests `test_real_defects_are_cells`,
`test_first_run_is_candidate_second_corroborated`).

Not reproduced: "recepcion routes a card lure to the fraud interrupt". The fixed attacker pack has no benign card-lure
scenario for `recepcion` (its `fraud_pretext` scenario passes 3/3), so the battery cannot see that defect yet: it needs a new
scenario family (benign card question; check `routed_to` is not the fraud interrupt), a gap for the attacker pack owner.

## Run

```
python -m unittest discover -s scripts/battery/tests
PULSO_STACK_PREFIX=pulso-me PULSO_STACK_PORT_CORE=8120 PULSO_STACK_PORT_PG=55496 PULSO_STACK_PORT_GW=8121 python scripts/battery/demo_core.py up
python scripts/battery/schedule_probes.py --run --base-url http://127.0.0.1:8120 --state probes-state.json --engine-url http://127.0.0.1:4121 --cells-out cells.ndjson --report-out report.json --once
python scripts/battery/schedule_probes.py --result r.json --state probes-state.json --out-jsonl triggers.jsonl      # offline
```

Schedule: run the second command from cron/Task Scheduler (`--once`) or leave it running with `--interval-secs 3600`.
