# Scoring (SC1)

Offline scorers, standard library only. Synthetic fixtures under `tests/`; run
`uv run --no-project python -m unittest discover -s scripts/scoring/tests`.

- `score_findings.py --catalog opbench-lite.json --signals cells.json`: scores `steps_cli cells` output
  (`{signals:[{metric,dims,status,direction,discovery.diff}]}`) against the OPBENCH-lite catalog. Match key is
  metric_id + normalized cell + direction (catalog direction = sign of `effect.difference`). Reports recall,
  precision (reported non-findings and unknown cells both count against it, listed separately), and Spearman
  ranking agreement. Reported = `corroborated` (`--include-candidate` adds `candidate`). Catalog status
  `refuted` entries are non-findings; descriptive corroborated entries are neutral.
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
