# 0222 W1-4 level_risk finding type (UTC 2026-10-05T04:00Z, CLAUDE)

Lane W14, branch `claude/w14-level-risk`, stacked on `origin/claude/ag2-more-cells` (not on a merge). Plan: `docs/reports-claude/PLAN_PROPOSER_AND_ATTACH_2026-10-04.md` W1-4.

Delivered
- `steps::cells`: second finding type `level_risk` (class risk, claim association): pooled level of a pre-registered metric (`Config::level_risks`, default M8 > 10%, excess floor 5 pp) with one-sided z, Wilson 95% interval, support, holdout replication, R2 windows, months and cells above the threshold. Statuses `candidate | corroborated | refuted | uncertain`. Level tests are a separate Bonferroni family over the registered count (`level_tests`), not added to `cells_explored`; k rule and named discards (`k_violation`, `level_below_min_support`) unchanged. M7 and M9 are not registered: M7 stays descriptive, M9 refuted.
- `reasoning::finding`: `level_risk` signals are skipped (status `level_risk`) so no risk level becomes a contrast finding.
- `scripts/scoring/score_findings.py`: level risks match only catalog entries of type `risk`, reported under `risk`; problem recall/precision unchanged.
- Segment cells language x channel: NOT built. No language/locale column in customers.csv (only country and accent); call_transcripts.detected_language is es in 100%; language lives only in E0. See `docs/data/bank-cells-metrics.md`.

Limits: the threshold is an analyst policy parameter; the consent flag is the extract's current value, so the level is not consent at send time; no legal conclusion, no cause, no effect on conversion.
