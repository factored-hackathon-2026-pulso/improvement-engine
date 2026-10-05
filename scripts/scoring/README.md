# Scoring (SC1)

Offline scorers, standard library only. Synthetic fixtures under `tests/`; run
`uv run --no-project python -m unittest discover -s scripts/scoring/tests`.

- `score_findings.py --catalog opbench-lite.json --signals cells.json`: scores `steps_cli cells` output
  (`{signals:[{metric,dims,status,direction,discovery.diff}]}`) against the OPBENCH-lite catalog. Match key is
  metric_id + normalized cell + direction (catalog direction = sign of `effect.difference`). Reports recall,
  precision (reported non-findings and unknown cells both count against it, listed separately), and Spearman
  ranking agreement. Reported = `corroborated` (`--include-candidate` adds `candidate`). Catalog status
  `refuted` entries are non-findings; descriptive corroborated entries are neutral. `type: "level_risk"`
  signals (W1-4) are matched only to catalog entries of type `risk` (metric + cell, `{}` = `{"scope":"overall"}`)
  and are reported under `risk` (recall over risk entries, unmatched level risks listed), outside the problem
  recall/precision; a `refuted` risk entry reported as a level risk counts as a non-finding.
- `score_proposal.py`: 12-criterion rubric. Mechanical: R3, R4, R5, R6 (independence), R7, R11 (gates
  R4/R5/R6/R7/R11). Judged: R1, R2, R8, R9, R10, R12 via an optional hook. Preconditions: anchored patch
  applies byte-exact, target exists in the registry export (skipped, and reported as skipped, if none is
  given). Adequate = total >= 19, no zero, hard gates >= 1.

Judge hook: `--judge module:callable --builder-model X --judge-model Y`. The judge family must differ from the
Builder family, is sampled twice (min taken, disagreement > 1 escalates to a human), never sees builder
reasoning, and may only score judged criteria. This repo holds no keys; the callable owns its credentials.

Input shapes were fixed against the audit and the local, not yet delivered, OPBENCH-lite output; the
registry export is read generically (any JSON whose objects carry `id`/`kind`). Re-check both when the
Codex catalog and the real agent-core export land.

## Judge (SC2)

- `judges/gateway_judge.py` (`--judge judges.gateway_judge:judge`): calls the local llm-gateway `POST /v1/generate`
  (`{prompt, inputs, schema, profile, labels}` -> `{output,...}`, same contract as the engine's `LlmGateway`).
  Output schema per criterion: `{"score": 0|1|2, "justification": "<one line>"}`. Malformed output is re-asked once,
  then denied (`JudgeError`); HTTP errors deny. Builder reasoning keys are stripped before the prompt; the proposal
  text is framed as data. Refuses when judge family == builder family or the gateway is not loopback/private.
- Environment only (never printed, never in argv or files): `PULSO_LLM_GATEWAY_ADDR`, `PULSO_LLM_GATEWAY_KEY`
  (fallback `GATEWAY_TOKEN_AGENT_CORE`), `PULSO_JUDGE_MODEL` (default `z-ai/glm-5.3-flash`),
  `PULSO_JUDGE_BUILDER_MODEL` (default `xiaomi/mimo-v2.6-flash`; Verifier is `xiaomi/mimo-v2.6-pro`),
  `PULSO_LLM_GATEWAY_ALIAS`, `PULSO_LLM_GATEWAY_TIMEOUT_S`. `model_family` now knows zhipu (glm, z-ai) and xiaomi (mimo).
- Double sampling, min and the gap > 1 escalation stay in `run_judge`; the callable is one sample.
- `judge_calibration.py [--golden f.json] [--live] [--limit N]`: agreement of the judge with a golden set
  (`golden/synthetic_golden_12.json`: 12 synthetic proposals, expected scores derived from the rubric anchors, NOT human
  labels; a Codex set may replace it, same shape). Reports exact, within-1, hard-gate agreement (a 0 is a rejection
  gate: judge-zero == expected-zero) per criterion and per proposal; escalated items are listed, not scored. Without
  `--live` or gateway env the status is `not_exercised`. Offline tests use a scripted transport.

### Live run (local stack, real gateway, `z-ai/glm-5.3-flash`, builder family xiaomi -> other_family)

Credentials: the local gateway accepts the consumer token it was started with (`GATEWAY_TOKEN_AGENT_CORE`, same wiring as
`scripts/dev-stack/stack.py`); a throwaway wrapper loaded it into the child process environment only (nothing printed,
written or in argv). The gateway container already had the `openrouter` endpoint alias (`LLM_ENDPOINTS`), so no dev-stack
change was needed. Full 12 golden proposals, 2 samples each (25 gateway calls):

- Agreement vs the synthetic expected scores (66 scored pairs; syn-06 denied, see below): exact 0.909, within-1 1.000,
  hard-gate (zero == zero) 0.955, proposal-level gate agreement 0.818. Per criterion exact: R1 0.64, R12 0.82, R2/R8/R9/R10 1.00.
  No sample gap > 1, so nothing escalated. The disagreement is concentrated in R1 and R12 (judge more lenient/strict by one).
- Latency per call: min 13 s, mean 61 s, max 240 s (glm is a reasoning model; 1-8k output tokens per call).
  Cost from gateway usage: about USD 0.030 for the run (27k tokens in, 67k out); `profile.price` is a declared estimate
  (0.1 / 0.4 USD per Mtok, env-overridable), not the provider price.
- Real defects found and fixed (tests first): (1) the gateway requires `profile.price` as decimal strings (HTTP 400);
  (2) it rejects the `maxLength` schema keyword; (3) glm answers multi-line / long justifications, which the strict
  one-line check denied: now normalised (scores stay strictly validated); (4) glm sometimes exhausts `max_tokens` and the
  gateway answers 502 `invalid_output`: treated as malformed (one re-ask), `max_tokens` 8000, timeout 240 s;
  (5) a dropped connection is retried once; (6) calibration now counts a proposal whose judge is denied (`denied`) instead of
  aborting the whole run, while family refusal and an unreachable gateway stay fatal.
- Residual: ~1 in 12 proposals is denied because glm runs away or times out (syn-06: 504 after 240 s); the judge fails
  closed. A quicker model or a reasoning-effort knob in the gateway profile would fix the latency and the denials.
- Environment note: the dev-stack gateway container was removed twice during the session by other activity on the
  `pulso-dev` podman machine; it was recreated with the stack's own wiring.
