# 0223 W1-2 decision dossier (UTC 2026-10-05T06:00Z, CLAUDE)

Lane W12, branch `claude/w12-dossier`, created from `origin/claude/w9-consolidated` with `origin/claude/reg1-regression-suites` and `origin/claude/w14-level-risk` merged (both clean). No dependence on any merge, no PR. Plan: `docs/reports-claude/PLAN_PROPOSER_AND_ATTACH_2026-10-04.md` W1-2.

Delivered
- `seams/crates/reasoning/src/dossier.rs`: pure, deterministic `dossier::build(finding, proposal, verdict?, labels)`; ES with PT variant. Fields: `title`, `description`, `rationale`, `changelog` (agent-core `VersionDocs` names, caps 200/3800/1500/1500) plus `sections` {problem, evidence, diff, result, expected_effect, measurement, risks, unchanged, honesty}. Labels follow the Codex T4 DOSSIER_SPEC (Problema observado, Evidencia y comparacion, Que cambiaria, Efecto esperado, Como se evaluara, Riesgo, Que no cambia) plus `Resultado base vs candidato` (REG1 verdict story) and `Etiquetas` (honesty).
- Honesty enforced in code: `announce` only for `regression_suite_proven` + corroborated finding + a real change; absent verdict, `non_discriminating`, `not_fixed`, `guard_regressed`, `infra_failed`, `base_only` and `not_exercised` give `announce: false` and a first line that says so; a forged `announce: true` on a non-proven story is ignored. Expected effect always "no medido"; outcome vocabulary improved / no_detectable_change / worsened / inconclusive; `uncalibrated` unless a human calibration flag; runtime `doubles` is forced when the record has doubles; rubric shown as structural self-score, not the judge; judge family from the story's `model_policy`; synthetic data labelled.
- PII: every output string is refused (typed error, never scrubbed) on an email-like at-sign, a PII token marker or a digit run of 6+; counts print with thousands separators.
- `reason_cli dossier --proposal --finding [--verdict] [--runtime] [--lang] [--format json|md]`; `pipeline::Reasoned.dossier` (built without verdict, so `announce: false`, "not evaluated"); `dossier::schema()` JSON Schema; `Finding::to_signal_json`.
- `docs/dev/DOSSIER_EXAMPLES.md`: dossiers rendered for the two REG1 live results (t/estado_pqr, p/resumen_radicado) plus the negative ones.

Differences from the Codex DOSSIER_SPEC
- Spec goldens are hand-written prose; here the text is assembled from numbers, so wording is more formulaic. Same section order and field names; two extra sections; `description` stays below 4,000 (spec limit) with a 3,800 ceiling.
- Spec counts are all-channel Queja (56.4 %, 117,021); the golden test here uses the Queja x Phone rates 56.1 % vs 16.6 % (fixture-scale counts) and the M8 level 874,417/1,746,801. The two estimands are not mixed.
- The spec forbids any lift estimate; the dossier keeps `min_detectable_gap` only as a measurement threshold ("not a prediction").
- Contrast interval is the Wilson 95 % interval of the cell rate (the sensor emits no interval for contrasts); level risks use the sensor interval.

Integration point (registry-writer is NOT on this base; branches `claude/b2-registry-writer` and `claude/b3-wire` are unmerged and untouched)
- Call `dossier::build(&signal, &reasoning_record, Some(&verdict_story), &labels)` in the writer; use `dossier.es.title` for `Proposal.title`, and write `dossier.es.description|rationale|changelog` into `changes[0].docs` (PT variant in `dossier.pt`, same shape). Publish only when `dossier.announce` is true; otherwise keep the proposal as an internal draft. `dossier::schema()` is the contract.
- Not verified: how the SPA shows `description`/`rationale`/`changelog` (open platform questions in the Codex spec, section "UI rendering").

Limits: association, not cause; no effect estimate; rubric is structural; the judge protocol and human spot-check are still pending; the REG1 live results are on invented counts (`source: synthetic`).
