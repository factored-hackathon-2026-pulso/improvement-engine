# 0499 SC2 rubric judge (UTC 2026-10-05T01:00Z, CLAUDE)

Lane SC2, branch `claude/sc2-judge` off origin/main 2a9682de. Python only, standard library.

Delivered
- `scripts/scoring/judges/gateway_judge.py`: judge for `score_proposal.py --judge`, calling the local llm-gateway `POST /v1/generate` (contract read from `engine::models::llm_gateway`). Model `z-ai/glm-5.3-flash` (Builder `xiaomi/mimo-v2.6-flash`, Verifier `xiaomi/mimo-v2.6-pro`), configurable by env. Strict per-criterion `{score 0|1|2, justification}` schema, one re-ask on malformed output then deny, family guard, builder-reasoning keys stripped, credentials from env only (`PULSO_LLM_GATEWAY_KEY`, fallback `GATEWAY_TOKEN_AGENT_CORE`), loopback/private address only, key scrubbed from errors.
- `score_proposal.model_family` learned zhipu (glm, z-ai) and xiaomi (mimo, xiaomi); before, the guard could not classify either and refused.
- `scripts/scoring/judge_calibration.py` plus `golden/synthetic_golden_12.json` (12 synthetic proposals; expected scores derived from the rubric anchors, not human labels). Reports exact, within-1 and hard-gate agreement.
- Offline tests (`tests/test_gateway_judge.py`, scripted transport): family guard, double sampling and min, escalation on gap > 1, malformed retry and deny, no reasoning leakage, no credentials in outputs.

Live run: not_exercised. The gateway env vars (address and key) are not in this shell's environment, and credentials are only read from the environment. Run `python scripts/scoring/judge_calibration.py --live --limit 3` with the local stack up and the vars exported.

Limits: agreement on synthetic labels measures anchor-following, not human agreement; replace with the Codex set (same shape) and recompute kappa on R1, R2, R9, R10 per the rubric report.
