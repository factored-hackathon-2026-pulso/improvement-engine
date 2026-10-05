# Bank cell tables: metric definitions (AG2 additions M7-M10)

Producer: `scripts/aggregate/bank_cells.py` (stdlib only; pyarrow is cached offline in this environment but not needed). Consumer: the metric-generic Rust sensor `steps::cells` (`steps_cli cells`). Every row is `{metric, dims, half, period, numerator, denominator}`; no ids, no free text, no row-level output, nothing committed. Source of the design: `docs/reports-claude/BANK_DATA_AUDIT_2026-10-04.md` (F03, F08, F10, F12).

## Common rules (all metrics, M1-M10)

- Grain per row: `metric x dims x half x period`. `half` = low bit of SHA-256(salt || customer_id): `discovery` (A) or `holdout` (B). `period` = literal `YYYY-MM` of the naive event timestamp; the partial months 2023-06 and 2026-06 are excluded.
- k rule: a row is emitted only when `denominator >= 10`, and `numerator` and `denominator - numerator` are each `0` or `>= 10`. So `(valid, pos)` is masked whenever either is 1-9; a zero is published only with `valid >= 10`. A masked row is dropped (the sensor treats absence as missing, never as 0); only the suppressed count per metric is reported in the local summary.
- No margins: no row without dims is emitted for M7-M10, so a masked cell cannot be recovered by subtracting a published total. (The existing M3 and M6 complementary-suppression rules are unchanged.)
- Reference files (`customers.csv`, `marketing_campaigns.csv`) are read through a column allowlist (`REF_COLUMNS`); PII columns are never indexed.
- Customer attributes (segment, consent) are the extract's CURRENT values, not as-of the event. No timezone claim.

## Finding types of the sensor (W1-4)

- `contrast` (default, M1-M10): a cell against the rest of its metric (same channel, excluding its own reason); BH over all explored cells; holdout; R2 windows.
- `level_risk` (`type: "level_risk"`, `class: "risk"`): a metric whose VALUE is the risk, judged against an explicit threshold registered in `Config::level_risks` BEFORE looking at data (analyst policy parameter, not a legal threshold). Registered today: M8, threshold 0.10, minimum excess 0.05. Test: pooled rate of the metric's valid cells, one-sided z against the threshold, Wilson 95% interval, excess floor, holdout confirmation (half of the floor), R2 windows, count of months above the threshold, count of cells above the threshold (descriptive). Statuses `candidate | corroborated | refuted | uncertain`; `claim: association`; no cause, no legal conclusion, no consent-at-send-time claim.
- Multiplicity: level tests are a separate pre-registered family, Bonferroni over `level_risks.len()` (the registered count, fixed in advance, whether or not the metric is present in the table), reported as `level_tests`. They are NOT added to `cells_explored` (the contrast BH family), so neither family dilutes the other; a metric registered as a level test still also runs the contrast test (M8 stays `refuted / no_differential` there, which is correct).
- Discards and k: the same k rule and `k_violation` discard apply to the rows; a pooled discovery support below `min_support` is the named discard `level_below_min_support`.
- Metrics that are NOT registered (M7, M9, ...) never get a level verdict: M7 (6.0% vs 4.5%, under the 5 pp effect floor) stays descriptive and M9 stays refuted.
- Scoring: `scripts/scoring/score_findings.py` matches `level_risk` signals only to catalog entries of type `risk` (reported under `risk`, outside problem recall/precision).

## Segment cells (language x channel): not built

Checked in the real tables 2026-10-05: `customers.csv` has no language or locale column (only `country` and `detected_accent`, the latter 29.9% null and an accent label, not a language); `call_transcripts.detected_language` is `es` in 100% of rows (no variation, 25% of contacts only). Language (es 95% / pt 5%) exists only in the E0 dispute cases, a different source and not the bank. So language x channel cells would be a constant in the bank data and are skipped; revisit if a customer language column appears.

## M7 `digital_error_rate` (descriptive)

- Table `digital_events`. Numerator: events with `event_type = 'Error'`. Denominator: events with that action and channel. Dims: `action` x `channel` (accent-folded). Period: `event_date` month.
- Population: events with an identified `customer_id` (anonymous events cannot be split A/B and are excluded and counted) and an action outside `STRUCTURAL_ACTIONS` = {null, `login`, `logout`, `view_product`}, which have no Error events by construction (audit F10) and would otherwise deflate the baseline.
- Supports: descriptive concentration of Error events on transactional actions (`view_transactions`, `initiate_transfer`, `initiate_payment` about 6.0% vs about 4.5% for other views) and its stability by channel, half and period.
- Cannot support: that an `Error` event is a technical failure (no taxonomy validated), any link to contacts (not higher after an Error, audit F10), causes, or app-version effects. Class: friction candidate, descriptive only. The sensor's default effect floor (5 pp) is above the real gap (about 1.5 pp), so by default it is reported as no differential, which is the honest verdict for a descriptive gap.

## M8 `send_to_nonconsenting_rate` (RISK, descriptive)

- Table `campaign_sends` joined to `customers.csv` (`accepts_marketing`) and `marketing_campaigns.csv` (`campaign_type`). Numerator: sends to customers with `accepts_marketing = false`. Denominator: sends with a known consent flag and a known campaign. Dims: `campaign_type` x `channel` (send channel). Period: `send_date` month.
- Supports: the LEVEL of the compliance risk (about half of all sends go to non-consenting customers) when the cell rates are read directly from the cells.
- Cannot support: a differential finding. The share equals the base rate of non-consenting customers in every cell (sends are independent of consent), so a "vs rest" sensor correctly finds no differential; the risk is the level, not a cell contrast. Also cannot support: consent at send time (current flag), legal conclusions, any effect on conversion or opens.

## M9 `tx_decline_rate` (expected FLAT, non-finding)

- Table `transactions` joined to `customers.csv` (`segment`). Numerator: `transaction_status = 'Declined'`. Denominator: transactions with a status and a known customer segment. Dims: `channel` x `customer_segment`. Period: `transaction_date` month.
- Supports: refuting heterogeneity: decline rate is about 5.0% in every cell, so the sensor must report no differential (audit F12). A corroborated M9 signal would be a sensor failure.
- Cannot support: any statement on fraud, response codes or customer impact. Class: non-finding (`refuted`).

## M10 `handle_time_unresolved_share` (re-expression of M1)

- Table `call_center_interactions`. Numerator: handled HOURS of contacts with `was_resolved = false`. Denominator: handled hours of contacts with a known flag and a non-null `duration_seconds`. Dims: `reason_category` x `channel`. Period: `interaction_date` month. Seconds are summed per cell and floored to whole hours at emission, so the sensor's effective n is about 7x smaller than the contact count (conservative; seconds would inflate the binomial n).
- Population: Phone, App and Web video only (duration is null for chat and email by design); channels without duration produce no M10 cells.
- Supports: the effort cost of the M1 problem (about 29% of handled time goes to contacts that stay unresolved).
- Cannot support: an independent problem. It is the same label (`was_resolved`) weighted by duration, so the sensor lists it `depends_on = M1` (one-line dependency in `cells.rs`, `Config::default`); burdens are not summed with M1. Floor-to-hours also means very small cells are masked (hours must be 0 or >= 10).

## Full-period cells, pooled support and baseline (DET1)

Why: the monthly k rule (k >= 10 on numerator, complement and denominator per metric x dims x half x MONTH) deleted whole cells from the table (low-rate cells, all of Retencion and Web) and left a same-channel baseline made only of the survivors (inflated, effects attenuated). Cause analysis: `docs/reports-claude/DETECTION_GAP_ANALYSIS_2026-10-05.md`.

- The aggregator now also emits, per metric x dims x half, a FULL-PERIOD row (`period: "ALL"`) and two R2 window rows (`W1` = 2023-07..2024-12, `W2` = 2025-01..2026-05), all derived from the month cells. k (numerator, complement, denominator) is checked on each pooled row. Month rows stay (k per month) for the per-month counts.
- The sensor takes the discovery/holdout half totals from the `ALL` rows and R2 from the `W1`/`W2` rows when a metric has them (month rows are then not summed again). Tables without those rows (older exports) behave as before. M8 (level-risk) keeps the monthly-only contract.
- Baseline: same-channel baseline of a cell = the sum of the PUBLISHED full-period cells of the same metric and channel in the same half, minus the cell itself (excluding its own reason). A cell suppressed by k is absent from both the tested set and the baseline; the share lost is now only cells whose pooled count is below k, not any cell with one small month. The stage carries `baseline_numerator` / `baseline_denominator` so the baseline is auditable.
- Support floor: `min_support` (500) applies to the POOLED discovery + holdout support (the full period), not to the discovery half. Replication is still evaluated on halves; a holdout half with support < `min_support / 2` gives status `uncertain`, reason `replication_underpowered` (the cell is kept, not dropped).
- Dependency flag: `depends_on` comes from the metric dependency table (`M6`, `M6R`, `M6U`, `M10` depend on `M1`) on every cell of the dependent metric, whether or not M1 has a finding on the same cell. A dependent signal is a re-expression of its parent, never an independent problem.
- Reason-only family: `M1` with a sole `reason_category` dimension (channel pooled) is an additional family. Multiplicity: BH per family (`main` = everything else, `reason_only`), the explored count of each is reported in `method.families`; `cells_explored` is the total. Existing tables without reason-only cells are unaffected (the main family is the old family).

### Privacy: what the pooled cells reveal (honest note)

- Every published count and its complement is still 0 or >= 10 (the tests assert it on `ALL`, `W1/W2`, months and reason-only rows).
- Suppression is hierarchical and consistent: ALL = W1 + W2 and each window = its months, so if ALL is suppressed all windows and months of that cell/half are withheld; if any window is suppressed the other window is withheld; if a month of a window is suppressed and the suppressed months do not sum to a k-safe residual (and are fewer than two), all months of that window are withheld. The M1 reason-only parent is withheld when the suppressed reason x channel children cannot be differenced to a k-safe residual. Existing M3 / M6 margin rules apply at every level.
- What it does reveal: pooled full-period counts for small segments (e.g. Retencion x Web) that were hidden when only months were published. Each is still a count of >= 10 contacts of each kind; month-level information (seasonality of a small cell) is withheld for cells that fail at month level. Residual risk: when two or more cells are suppressed together, a reader can bound (not recover) their sum from a published margin; the sum itself passes k.
- Not claimed: no formal differential privacy; customer-hash halves are public by design.

## Exploratory profile (DET1)

`Config::exploratory()` / `steps_cli cells_exploratory`. The default profile (`steps_cli cells`) is the strict one and its output is unchanged by the profile existing. The exploratory profile runs the strict tier unchanged and ADDS a tier of `candidate_exploratory` signals, so downstream agents (Scout, Verifier, Builder, regression proof, rubric) receive more candidates; the quality gate for these is the regression proof, not the statistics.

| Knob | Strict | Exploratory | Rationale | False-positive cost |
|---|---|---|---|---|
| BH q | 0.01 | 0.10 | BH q bounds the expected share of false discoveries among the exploratory tier at 10% | up to ~10% of this tier can be noise before the effect floor |
| Effect floor | 5 pp | 3 pp | small effects can still be addressable (volume x diff) | more small, real-but-minor differences |
| Ratio floor | 1.25 | 1.15 | consistent with the lower effect floor | same |
| Min support (pooled) | 500 | 200 | low-volume channels | wider CIs; one half may be underpowered |
| Replication | gate (`corroborated`) | label | holdout outcome is recorded (`reason`), not required | cells whose holdout is weak are kept |

- Statuses: `corroborated` stays strict and the exploratory profile never emits it. `candidate_exploratory` has reasons `exploratory_discovery_only` (no holdout), `exploratory_replication_underpowered`, `exploratory_holdout_weak`, `exploratory_holdout_replicated`. A cell whose holdout reverses the direction is not reported (named discard `exploratory_holdout_reversed`). Strict cells that pass discovery but fail replication become `candidate_exploratory` in this profile.
- Privacy: `k_min` is never relaxed; the exploratory profile only reads the same published rows.
- `priority` (every reported cell signal): diff x ln(1 + pooled support) x evidence, evidence = 0.4 + 0.3 (holdout same direction) + 0.2 (holdout significant, i.e. strict corroboration) + 0.1 (R2 windows replicated). Exploratory signals are sorted by priority (best first), after `corroborated` and `candidate`, so a cap on findings per run takes the best first. Ranking only, never a test.
- Every exploratory signal carries `exploratory_note`: "exploratory: weaker statistical evidence; the regression proof is the quality gate". `method.profile` is `strict` or `exploratory`; `method.exploratory` echoes the knobs and the explored count.
- Scoring: `score_findings.py` keeps recall/precision on `corroborated` only and reports the exploratory tier under `exploratory` (newly matched positives, `unlabelled` cells = not in the catalog, neither findings nor false positives).
- Pipeline wiring (reasoning / value loop reading `candidate_exploratory` and showing the status to the Verifier) is NOT done here: `Finding::from_report` still only takes `corroborated`.
