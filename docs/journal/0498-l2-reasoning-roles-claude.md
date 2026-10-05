# 0498 L2 reasoning roles (UTC 2026-10-05T00:30Z, CLAUDE)

Lane L2, branch `claude/l2-reasoning` (L1 cells sensor merged below it, rebased on main 7f38beb).

Delivered
- `engine::models::llm_gateway::LlmGateway`: ModelPort for the real llm-gateway `POST /v1/generate` contract (the older `Gateway` speaks chat-completions only). Guard (E0/original refused, TPS, size) before any byte; role schema travels as the gateway `schema`; only an answered call is `real`.
- Crate `seams/crates/reasoning`: Scout, Verifier (separate port, prompt and context; deterministic recompute the model cannot overrule), Builder; mapping finding -> real agent-core artifact ids; anchor menu (protected clauses never offered); byte-exact anchored-patch compiler; new-agent closure copy of `consultas`; structural rubric self-score (not the judge); `reason_cli`.
- Baseline artifacts: `scripts/reasoning/export_base_artifacts.py` -> `fixtures/base_artifacts.json`, labelled `fixture-baseline` (agent-core f91ac44 registry-e2e seed, not a live registry).
- Opt-in: bank/E0-derived aggregates reach a real hosted model only with `allow_derived_aggregates`; synthetic never needs it.

Commands run: `cargo test -j 1 -p reasoning` (8 compile + 12 pipeline, offline, scripted), `-p engine --test models_llm_gateway` (5), live `--ignored` against the local stack with deepseek/deepseek-v4.1-flash on synthetic aggregates: 3 of 3 scenarios produced a compiled proposal (new agent, t/estado_pqr patch, p/copiloto patch); one needed a second attempt (rationale over length, since constrained in the prompt).

Limits: no eval_suite authored (R6b not evaluated); new-agent entities are not validated by the Core's `registry/validate`; live registry content not read (fixture baseline); `{{ facts.pqr.value.status }}` shape unverified on the real tool; same model for Scout and Verifier unless `PULSO_LLM_GATEWAY_VERIFIER_MODEL` is set.
