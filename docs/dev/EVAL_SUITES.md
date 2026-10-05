# Eval suites for agent-core evaluation (EV1)

Why: agent-core's `approve` needs a proposal in state `evaluated`; `evaluate` fails ("no existe la suite") without an
`eval_suite`, and no real agent ships one. A draft for `disputas` or `consultas` must therefore carry its own suite
(draft kind `eval_suite`, same proposal as the prompt change). The engine's constructor bot may call `evaluate`; approval
stays human. Report: `docs/reports-claude/SHARED_ATTACH_GAPS_2026-10-04.md` gap 4.

## Where

`agent-core-assets/eval-suites/pulso-min/{disputas,consultas}/` (README maps case -> protected behaviour). Replaceable by
the larger bank (Codex T2) in a sibling directory; the attach script takes `--suite <path>`.

## Format (agent-core `registry/suite.py`)

`id, version, agent_id, repetitions (1..10), scenarios[1..200], thresholds{metric: {noise_margin, floor?}}`. Scripted
scenario: customer `principal`, `steps` (`start`, `turn`, `confirm`), per-step `auth`, `seed.tools` FIFO replies,
`sensitive_values`, `expect{outcome, actions_verified, escalated}`, `assertions` over the closed event catalog. A scenario
passes only if all repetitions pass. Thresholds are required for every `gate`/`guardrail` metric the agent declares
(`missing_threshold`) and forbidden for undeclared ones (`unknown_threshold_metric`); `disputas` and `consultas` declare
none, so `thresholds: {}`; the four `platform_*` guardrails are automatic and must be 0.

## Validate offline (no network)

    uv run --offline --with pyyaml python -m unittest agent-core-assets/eval-suites/pulso-min/test_pulso_min.py
    PULSO_AGENT_CORE_DIR=<agent-core checkout> uv run --offline --with pyyaml python scripts/dev-stack/attach_eval_suite.py \
        --suite <suite.yaml> --offline-only --agent-file <agent-core>/tests/fixtures/registry-e2e/agents/disputas@1.0.0.yaml

Result at agent-core main 789edc0: `disputas-min` 20 scenarios and `consultas-min` 11, `suite_problems == []`.

## Attach and evaluate on the LOCAL stack

    export PULSO_AGENT_CORE_DIR=<agent-core> PULSO_LLM_GATEWAY_DIR=<llm-gateway> PULSO_SERVE_AGENTS=disputas,consultas
    export PULSO_REGISTRY_DIR=<agent-core>/tests/fixtures/registry-e2e PULSO_SERVE_E2E=1
    python scripts/dev-stack/stack.py up --reset-db
    uv run --with pyyaml python scripts/dev-stack/attach_eval_suite.py --suite <suite.yaml> --create
    # or: --proposal-id <id> [--changes extra-changes.json]   (keeps the draft's existing changes)

The script reads the agent, adds the `eval_suite` change, `put_draft`, `validate`, `freeze`, `evaluate`, and prints the
verdict plus per-gate items and per-scenario failure reasons. It never calls approve, publish, promote or reject; exit 0 only
on verdict `pass`, 3 when the stack is down (live part labelled `not_exercised`; the offline part still runs).
It uses the local `admin` token from `.dev-stack/tokens.json` (constructor role; never printed). Evaluate leaves a `pass`
proposal in state `evaluated`: a human still has to approve.

Stack switches added to `scripts/dev-stack/stack.py` (all optional, defaults unchanged): `PULSO_REGISTRY_DIR` (import the
real-agent registry instead of `pulso-builder`), `PULSO_SERVE_AGENTS`, `PULSO_SERVE_E2E=1` (agent-core's e2e demo tools,
classifier and calibration, as `scripts/e2e/serve.ps1`), `PULSO_FIELD_CLASSIFIER` (the module moved on agent-core main to
`agent_core.composition.classification:field_classifier`).

## Live findings (2026-10-05, local stack, agent-core 789edc0, JEV real, classifier a keyword double)

1. With the stock `stack.py serve` ports (`testing.serve_demo` calibration, only `cal-demo`) every conversational scenario
   stalls: `decision_made` carries `above_threshold: false` for command and interrupt, so the run never closes (outcome
   None). The registry-e2e agents reference `cal-transfer-demo`; serve with the e2e doubles (`PULSO_SERVE_E2E=1`).
2. The keyword `match-cargo` double matches the amount as DIGITS or a merchant word; spelled-out amounts ("ciento veinte")
   give `ninguna` and the run clarifies forever. Case texts use digits and the seeds carry `merchant`.
3. Policy `escalamiento-disputa-monto` compares `facts.monto_usd.value > 500`: a seeded string reply ("600.00") did not
   escalate; a number does. Seed `convertir_moneda` with numbers.
4. A `verification_failed` scenario can never pass: the failed read-back of a write is `platform_unverified_write` = 1, so the
   gate fails by construction. Removed from the suite.
5. A scenario with a `turn` after the run already closed makes `evaluate` fail with HTTP error `run_closed` (not a gate
   verdict); keep multi-turn scenarios to what the flow actually needs.
6. `evaluate` on `fail` returns 409 `gate_failed` and sends the proposal back to `draft` (rev+1, must re-freeze); the script
   reads the report from the 409 payload.
7. Final: `disputas-min` pass (20/20) and `consultas-min` pass (11/11), platform guardrails 0. Manual-origin proposals were
   used, so the 10 per 24 h `auto_detect` quota was not consumed (about 9 manual proposals were created while iterating).

## Limits

Native evaluation cannot read wording or tool identity; tools are seeded; a green verdict is the Core yardstick, not
improvement evidence. Floors on a new metric must come from the base measurement, never from the candidate's run.
