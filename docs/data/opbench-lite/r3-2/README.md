# OPBENCH-lite R3-2: blinded proposal-rubric exercise

## Purpose and limits

This is a synthetic-only exercise for checking whether the proposal judge follows the written R1–R12 rubric. It is not a benchmark of bank findings, not a measurement of production quality, and not evidence that the judge is calibrated. No customer rows, contact text, identifiers, or provider responses are included.

The proposals are paired: each pair addresses the same authored synthetic finding, with one stronger and one intentionally weaker candidate. The 40 proposals span patch, template, and new-agent artifacts; Spanish and Portuguese; and clearly-good, borderline, and clearly-bad cases. Existing fixture artifact IDs are pinned in `artifact_index.json`; newly proposed agent IDs are explicitly new outputs, not fixture IDs. Evidence references are authored synthetic support counts only.

## Rubric and provenance

The referenced `docs/reports-claude/ARTIFACT_ANATOMY_AND_RUBRIC_2026-10-04.md` was not present at the verified repository ref. The operative definitions were therefore read directly from `scripts/scoring/judges/gateway_judge.py`, `scripts/scoring/score_proposal.py`, `scripts/scoring/judge_calibration.py`, and `seams/crates/reasoning/src/rubric.rs`. The source artifact catalog is pinned at `seams/crates/reasoning/fixtures/base_artifacts.json` in `artifact_index.json`.

All twelve criteria are rated on the 0/1/2 scale. `expected` is the six-criterion subset currently scored by the LLM judge: R1, R2, R8, R9, R10, and R12. The other six are retained to make the complete proposal rubric auditable, but are not included in judge-agreement calculations. The reported two-rater hard-criterion zero/non-zero score is descriptive agreement only, not the engine's proposal-acceptance implementation.

## Labeling protocol and current status

1. Author the candidate set and seal the pair IDs, expected direction, and design stratum outside each judge-visible `proposal` payload.
2. Label all R1–R12 from the written rubric, without calling the judge on this corpus first. `labels_pass1.json` is the primary Codex pass. It was produced in the authoring context, where pair identity and design strata were known; it is therefore **not blind**.
3. A separate model-agent instance labeled the shuffled opaque-ID file `rater2_blind_input.json`. It did not receive pass-1 labels, pair IDs, variants, design strata, quality bands, or the generator. Its output is preserved in `labels_pass2_blind.json`; `rater2_manifest.json` restores identities only after labeling. This is an asymmetric, partial-blind cross-check between independent contexts, **not independent human annotation**, not a fully blinded two-rater study, and not cross-provider validation.
4. Across 40 proposals × 12 criteria (480 ordinal ratings), the two passes show exact agreement **79.58%**, within-one agreement **96.67%**, descriptive Cohen's kappa **0.672**, hard-criterion zero/non-zero agreement **97.0%** (200 ratings), proposal-level hard-gate agreement **95.0%**, and matched-pair winner-direction agreement **95.0%** (19/20 pairs). Kappa is nominal, pooled, and descriptive only; rows within a proposal are dependent. Exact agreement is lowest on R3 (47.5%), R4 (30.0%), and R9 (50.0%); R4 disagreements are mostly adjacent 1-vs-2 ratings. One proposal has complete 12/12 opposite-polarity disagreement. These are rubric/label review targets, not evidence that either rater is correct. Do not average or silently overwrite labels; adjudicate against criterion definitions and preserve raw passes.
5. Only after labels are frozen should the exact proposal payloads be submitted to the judge. Preserve one judge row per proposal and judged criterion in `pulso.judge-rows.v1`; do not derive Cohen's kappa from aggregate-only calibration summaries.

The referenced scoring code currently exposes aggregate calibration output, not per-proposal/per-criterion rows. No existing row-level judge-output example was found at the reviewed main/PR refs. `agreement.py` accepts the documented row-level shape and fails closed on aggregate-only input; it can pass through the legacy aggregate summary but explicitly returns no kappa and marks it incomparable. The unit tests use tiny invented in-memory fixtures only to test arithmetic; those values are **not judge results** and must never be cited as judge-agreement evidence.

## Agreement calculations

The script reports exact and within-one agreement, descriptive hard-criterion zero/non-zero agreement, proposal hard-gate agreement, pooled nominal unweighted Cohen's kappa, per-criterion agreement, pairwise winner agreement, and mismatched rows. Missing judge rows remain missing; no imputation is performed. For two-labeler agreement, hard criteria are R4/R5/R6/R7/R11; this is not the engine's proposal-acceptance decision. Pairwise agreement compares the aggregate six judge-scored criteria within each matched pair.

The script reports comparisons only; it does not choose a deployment threshold or pass/fail the judge. Any operational acceptance threshold must be preregistered separately and supported by enough independent labels.

## Regeneration and checks

From this directory:

```powershell
python -m unittest -v test_agreement.py
python generate.py --emit authored
python generate.py --emit corpus
python generate.py --emit artifact-index
python agreement.py --pass1 labels_pass1.json --manifest rater2_manifest.json --pass2 labels_pass2_blind.json
```

The generator writes JSON to stdout (UTF-8), so a caller can redirect it in an authorized repository checkout. It reads only the small synthetic authoring/label/index files in this directory; it does not read the bank dataset, call a model/provider, or write files. `agreement.py` supports `--pass1 <json> --manifest <json> --pass2 <json>` for blinded two-rater agreement, or `--golden <path> --judge-output <row-level-or-aggregate-json> [--out <path>]` for judge comparison. Legacy aggregate summaries are marked non-comparable; kappa is never fabricated.

## Remaining limitations / next validation gate

- The second rater is an independent model-agent context, not a human or different provider; the first pass was unblinded to authored strata. R3-2 does not establish independent blind inter-rater reliability. Review rubric wording and adjudicate R3/R4/R9 plus the fully polarized proposal disagreement without altering raw passes.
- Find or produce an authorized judge run that emits row-level scores for this exact 40-item set; the current aggregate-only calibration output is insufficient for kappa.
- Run the script against genuine row-level output when available. Current repo evidence is aggregate-only, so judge-vs-labeler exact/within-one/kappa remain **not measured**. Do not present the labeler-vs-labeler statistics above as judge performance.
- Validate the corpus against the final current main artifact catalog before publishing; the fixture reference in `artifact_index.json` is intentionally immutable provenance, not a claim about later catalog revisions.
