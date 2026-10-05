# W11 evaluate before announce (plan W1-1 + W1-2)

A proposal is ANNOUNCED only after the engine proved it on agent-core: the regression suite of the finding FAILS on the base
and PASSES with the candidate. Otherwise the outcome is the internal `not_announced:<verdict>` and the dossier stays in the
engine job record.

## Flow (`registry_writer::proof::prove`, wired in `pulso run` by `ProofConfig`)

1. `scripts/regression/build_suite.py --finding <f> --target <ref> --out <dir>` (subprocess). Exit 0 prints `{"suite": id}` and
   writes `<dir>/<id>.bundle.json`; exit 2 prints `{"refused": code, "why": ..}` -> `not_announced:suite_refused`.
2. Base draft (suite only) and candidate draft (changes + suite) as MANUAL-origin proposals (the 10/24h `auto_detect` quota is not
   consumed), `put_draft`, `validate`, `freeze`, `evaluate` through the guarded writer (`registry_writer::eval`). Retries only for
   infrastructure (transport error, 5xx, 429, `evaluate` -> `failed_infra`): 4 retries, 20 s backoff, each recorded in
   `infra_retries`. Every try has its own `Idempotency-Key` (`<finding key>-<candidate digest>-r<n>-<label>-t<try>`).
3. `scripts/regression/judge_story.py` (subprocess, stdin `w11.judge_input/1` -> stdout `reg1.verdict_story/1`) applies the same
   rules as `prove_fails_on_base.py` (wording probes, `decide`, story text). Verdicts: `regression_suite_proven`,
   `non_discriminating`, `not_fixed`, `guard_regressed`, `infra_failed`; the engine adds `suite_refused`, `suite_error`,
   `dossier_error`. `generated_contains` probes (prompt targets) need a model and are not measured here: that case fails with
   "not measured", so a prompt finding is never announced on a probe nobody ran.
4. `reasoning::dossier::build` carries the story. `announce` = proven AND corroborated AND a real change.
5. Only if `announce`: the compiled changes are delivered with origin `auto_detect`, their docs (description, rationale, changelog)
   being the dossier ES text, plus the proven `eval_suite` as one more change so the supervisor's own freeze/evaluate can run it.

## Allow-list (`registry_writer::guard`)

Added: `POST /v1/registry/proposals/{id}/freeze` and `/evaluate`. Still refused (RED tests): approve, publish, promote, reject,
reopen, revoke, alias writes, any query/fragment/control byte smuggling, any other method.

## Configuration (`pulso run`)

| Variable | Meaning |
|---|---|
| `PULSO_EVAL_BEFORE_ANNOUNCE` | `on`/`off`; default ON for the live path with `PULSO_REGISTRY_VIA=api`, OFF for `via=run` (an agent-authored draft cannot be proven); unit tests build `ValueLoop` with `proof: None`. |
| `PULSO_REGRESSION_PYTHON` | command that runs python with PyYAML (default `python`; e.g. `uv run --with pyyaml python`) |
| `PULSO_REGRESSION_SCRIPTS` | dir of `build_suite.py`/`judge_story.py` (default `scripts/regression`) |
| `PULSO_EVAL_TIMEOUT_SECS` | HTTP timeout of the evaluation transport (default 900) |

The job record of a finding gets `evaluation` (verdict, story summary, GateItems, infra retries, honest labels: rubric, judge
family, `uncalibrated`, runtime, and the whole dossier ES/PT) and `outcome` (`announced`, `not_announced:<verdict>` or
`proven_not_delivered:<reason>`). Conclusive proofs are kept in `<work>/w11-proofs.json` keyed by finding key and candidate digest
(a replay makes no request); an `infra_failed` proof is re-run under a fresh key salt.

## Live (own local stack)

    PULSO_AGENT_CORE_DIR=<agent-core> PULSO_SERVE_AGENTS=disputas,consultas PULSO_REGISTRY_DIR=<agent-core>/tests/fixtures/registry-e2e \
    PULSO_SERVE_E2E=1 PULSO_STACK_PREFIX=pulso-w11 PULSO_PG_PORT=55471 PULSO_GW_PORT=8171 PULSO_CORE_PORT=8161 \
        python scripts/dev-stack/stack.py up --reset-db
    PULSO_DEV_STACK_DIR=<worktree>/.dev-stack PULSO_STACK_ADDR=127.0.0.1:8161 \
        cargo test -j 1 -p registry-writer --test live_proof -- --ignored --nocapture --test-threads 1

Result (2026-10-05, agent-core daf4604, scripted compiled `t/estado_pqr` proposal, M4 finding with invented counts): proven candidate ->
8/8 finding cases fail on the base, all 11 cases pass on the candidate, 15/15 GateItems, 2 infra retries absorbed, `announced`,
delivered `auto_detect` with 2 changes (template + eval_suite) and the dossier ES changelog; non-improving candidate ->
`not_announced:not_fixed`, dossier kept in the record.

## Limits

Prompt (`generated_contains`) findings are not announceable until a generator is wired into the judge; `new_agent` targets have no
suite mechanism with guards (refused). The suite proves that it discriminates, not that the change helps customers. The judge is
deterministic (no model); the dossier labels it so and stays `uncalibrated`.
