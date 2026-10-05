# Regression suites that fail on the base (REG1, plan W1-1)

Goal: from a detected finding, build an agent-core `eval_suite` and PROVE it "fails today, passes with the fix" before the
engine announces a proposal. A suite that the base already passes is reported `non_discriminating` and is never called a
regression suite.

## Pieces

| File | What |
|---|---|
| `scripts/regression/build_suite.py` | finding (metric, dims, discovery/holdout counts) + target ref -> bundle: `suite` (agent-core format, JSON), `meta` (per case: finding key, behaviour protected, check kind), `probes`, case id lists. Deterministic (byte-identical for the same input), 6-10 ES/PT finding cases + 3 guard cases copied verbatim from `pulso-min`, k floor 10 (`k_below_minimum` refuses), PII lint (no digit run of 6+, no emails/urls/phones; `pii_pattern` refuses), `no_guards_for_agent` / `no_mechanism` refusals. |
| `scripts/regression/prove_fails_on_base.py` | attaches the suite to a draft of the BASE (suite only) and of each CANDIDATE (compiled `changes` + suite), freeze + `evaluate`, merges probes, decides, prints the JSON verdict story. Never approves/publishes/promotes. |
| `scripts/regression/tests/test_regression.py` | 29 offline tests (determinism, k/PII rules, guards verbatim, probes, non-discriminating, not_fixed, guard_regressed, attempt story). |
| `scripts/regression/fixtures/` | invented finding, candidate patches in the reasoning crate's compiled shape (`{"changes": [{kind, content, docs}]}`). |
| `scripts/regression/results/` | the real bundles and verdict stories of the live runs below. |

Mechanisms (`--mechanism`, default from the target): `status_message_gap` (`template:t/estado_pqr`, agent `consultas`),
`closing_followup` (`prompt:p/resumen_radicado`, agent `disputas`), `uncovered_topic` (`new_agent:*`, agent `recepcion`; generator
only, refused today because pulso-min has no `recepcion` guards, never exercised live).

## Two kinds of checks, and why

agent-core's scorer is deterministic over `expect.outcome/escalated/actions_verified`, the closed event catalog and 4 platform
guardrails. It never reads response text. So each finding case has a native part (the flow still resolves, the response is
emitted, no `response_failed`) and, where the finding is about wording, a PROBE run by the harness:

- `state_reflected` (template): the candidate text is rendered with the same `{{ path }}` substitution as agent-core
  `render_template`, against the seeded state; the sentence must reflect the state. Deterministic, offline.
- `generated_contains` (prompt): 3 real samples of the prompt text under test through the LOCAL llm-gateway
  (`xiaomi/mimo-v2.6-flash`, the engine's own generation model in this stack); every sample must name the follow-up and carry no
  digit. Model-in-the-loop, so the samples are recorded in the story. The judge model is not used (deterministic checks only),
  which satisfies "the judge never scores what its own family attacked".

A case passes only if native AND probe pass. `generated probe not measured` (gateway down) is a failure, never a pass.

## Decision (`outcome`)

`regression_suite_proven` (>= 1 finding case fails on the base, all cases pass on the final candidate, guards pass on both) ->
the only outcome with `announce: true` and `suite_is_regression_suite: true`. Others: `non_discriminating`, `not_fixed`,
`guard_regressed`, `infra_failed` (retries: `evaluate` answers `failed_infra` when the real JEV drops, 4 retries with 20 s
backoff, all recorded in `infra_retries`), `not_exercised` (stack down, exit 3), `base_only`.

## Verdict story (schema `reg1.verdict_story/1`, for the dossier)

`finding_key, finding_id, target, mechanism, agent, suite_id, suite_is_regression_suite, outcome, reason, announce,
base{verdict, per_case, failed_cases, guards_failed, gate_items, proposal_id, infra_retries}, attempts[{attempt, verdict,
per_case, failed_cases, gate_items, candidate_digest, ...}], gate_items (of the final candidate), story_text{es,pt},
model_policy`. `gate_items` are agent-core's own `GateItem`s (`scenario/<id>`, `platform_*`). A native `pass` with a failing
probe is reported `fail` with `verdict_note`.

## Run it (local stack)

    # second stack beside other lanes' (names/ports are opt-in env switches added to stack.py in this lane)
    export PULSO_AGENT_CORE_DIR=<agent-core> PULSO_LLM_GATEWAY_DIR=<any existing dir if the gateway image exists>
    export PULSO_SERVE_AGENTS=disputas,consultas PULSO_REGISTRY_DIR=<agent-core>/tests/fixtures/registry-e2e PULSO_SERVE_E2E=1
    export PULSO_STACK_PREFIX=pulso-reg1 PULSO_PG_PORT=55452 PULSO_GW_PORT=8092 PULSO_CORE_PORT=8021
    python scripts/dev-stack/stack.py up --reset-db
    uv run --with pyyaml python scripts/regression/build_suite.py --finding scripts/regression/fixtures/finding_m4_pqr_status.json \
        --target template:t/estado_pqr --out .dev-stack/reg
    uv run --with pyyaml python scripts/regression/prove_fails_on_base.py --bundle .dev-stack/reg/reg-consultas-<hash>.bundle.json \
        --candidate <attempt1>.json [--candidate <attempt2>.json] --base http://127.0.0.1:8021 --gateway http://127.0.0.1:8092 --out story.json

Credentials stay in the process environment: `stack.py` loads agent-core.env / llm-gateway.env into child processes; the proof
reads the local staff token from `.dev-stack/tokens.json` and `GATEWAY_TOKEN_AGENT_CORE` from agent-core.env, sends them only to
127.0.0.1 and never prints them. Drafts are `origin: manual`, so the `auto_detect` quota (10/24 h) is not consumed (about 40 manual
proposals were created in this lane's runs, each evaluation leaves a draft).

Offline tests: `uv run --offline --with pyyaml python -m unittest scripts/regression/tests/test_regression.py`.

## Real defects found (local stack, agent-core ae437bb, JEV real)

### Live results (4 verdict stories + 1 earlier non-discriminating run in `scripts/regression/results/`)

| Scenario (target, finding `M4 category=Queja channel=Phone locale=es`, invented counts 96/240 and 90/236) | Base | Candidates | Outcome |
|---|---|---|---|
| `template:t/estado_pqr` (consultas), suite `reg-consultas-ffbb5234`, 8 finding + 3 guard cases | 8/8 finding cases FAIL (wording probe: the static sentence names no state), native 11/11 pass | attempt 1 placeholder `facts.pqr.value.estado` (unknown path): the REAL engine escalates, 8/8 native fail; attempt 2 `{{ facts.pqr.value.status }}`: 11/11 pass, 15 GateItems pass | `regression_suite_proven`, announce true |
| same, non-improving rewording (control) | 8/8 fail | native pass, wording probe 8/8 fail | `not_fixed`, announce false |
| `prompt:p/resumen_radicado` (disputas), suite `reg-disputas-ffbb5234`, 6 finding + 3 guard | 6/6 FAIL (3/3 real mimo-flash samples name no follow-up), native pass | attempt 1 non-improving text: 6/6 fail; attempt 2 follow-up sentence: 9/9 pass | `regression_suite_proven`, announce true |
| same, non-improving prompt (control) | 6/6 fail | 6/6 fail | `not_fixed`, announce false |
| native-only version of the prompt suite (first try, `fallback_used` assertions) | 9/9 PASS | n/a | `non_discriminating` (reported as such, not a regression suite) |

Platform guardrails (pii leak, unverified success claim, unverified write, unapproved citation) were 0 on every evaluated run.

### Defects and limits found live (verified; owners named, not fixed here)

1. agent-core `evaluate` never exercises a CANDIDATE PROMPT. `serve_registry` builds the harness with `ports.gateway`, an
   `HttpLLMGateway` bound to the LIVE registry, so `registry.get(p/resumen_radicado@1.0.1)` raises, the responder's `except
   Exception: break` falls to the template (`fallback_used` true, `validator.ok` false, 0 tokens, 16 ms, no request in the
   gateway log). Control: a text-identical version bump fails the same native assertions that the base passes. Consequence: a
   native `fallback_used=false` assertion would fail EVERY prompt patch, and a prompt patch gets no native evidence of its
   wording; REG1 therefore asserts only the flow outcome natively and discriminates with the harness-side generated probe. Ask
   for agent-core (via the user): bind the harness gateway to the candidate registry.
2. A template candidate IS exercised natively (placeholder resolved by the real engine; an unknown path escalates). Wording is
   still not assertable: the static base sentence passes every native case, hence the wording probe.
3. The real JEV provider drops intermittently: `evaluate` answers `failed_infra` ("HarnessUnavailable ... ['jev']"), up to 5
   consecutive times in one window. The proof retries 4 times with 20 s backoff and records each retry.
4. llm-gateway `/v1/generate` answers 400 for an unknown `labels` key (`locale`); keep labels to `prompt`/`model_profile`.
5. `scripts/reasoning/export_base_artifacts.py` exports from the YAML fixtures, so a prompt's `extra.model_profile` is the shorthand
   `perfil-generacion@1`, while the live registry stores `{id, spec: 1.0.0}`. A patch compiled from the fixture carries the string
   form and agent-core `validate`/`freeze` accept it. Whether the runtime then resolves it is NOT verified (finding 1 hides it from
   `evaluate`); `resumen_radicado_attempt1.json` is the fixture-shaped patch, `..._attempt2.json` the exact-shaped one.
6. Stack: other lanes already use the `pulso-l3-*` containers and ports; `stack.py` now takes opt-in `PULSO_STACK_PREFIX`,
   `PULSO_PG_PORT`, `PULSO_GW_PORT`, `PULSO_CORE_PORT` (defaults unchanged, DSN port follows). The `serve` process once vanished
   mid-session (restart with `stack.py up`, no reset needed). EV1's findings (e2e doubles needed, digit amounts for the keyword
   classifier, numeric seeds) still hold; `PULSO_SERVE_E2E=1` was used throughout.

### Limits

The finding is synthetic (invented counts, same shape as a corroborated cells signal) and the finding -> artifact -> behaviour
mapping (`closing_followup`, `status_message_gap`) is a harness-side mechanism spec, not something the cells data proves. The
prompt probe is a 3-sample model check (a stricter candidate may need more samples); the `uncovered_topic` generator and its
recepcion guards are not exercised. A proven suite is evidence that the suite discriminates, not that the change helps customers.

## W13: prompt patches and new agents (see `W13_PROMPT_AND_AGENT_ANNOUNCE.md`)

`build_suite.py` also generates the suite of a `new_agent:<x>` target (cases run on the new agent; closure and donor release settings in
the draft, the latter evaluation-only and labelled). `sample_probes.py` collects real model samples for `generated_contains` probes;
`judge_story.py` adds `native_binding` (control run) and `coverage` (native / harness_probe / not_measured / assumptions) to
`reg1.verdict_story/1`. Live: prompt patch announced, text-identical prompt `not_fixed`, new agent announced (evaluation drafts with an admin
stand-in: the builder cannot draft release interrupts).
