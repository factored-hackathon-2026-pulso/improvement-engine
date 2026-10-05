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
