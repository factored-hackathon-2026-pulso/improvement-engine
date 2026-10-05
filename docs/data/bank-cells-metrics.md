# Bank cell tables: metric definitions (AG2 additions M7-M10)

Producer: `scripts/aggregate/bank_cells.py` (stdlib only; pyarrow is cached offline in this environment but not needed). Consumer: the metric-generic Rust sensor `steps::cells` (`steps_cli cells`). Every row is `{metric, dims, half, period, numerator, denominator}`; no ids, no free text, no row-level output, nothing committed. Source of the design: `docs/reports-claude/BANK_DATA_AUDIT_2026-10-04.md` (F03, F08, F10, F12).

## Common rules (all metrics, M1-M10)

- Grain per row: `metric x dims x half x period`. `half` = low bit of SHA-256(salt || customer_id): `discovery` (A) or `holdout` (B). `period` = literal `YYYY-MM` of the naive event timestamp; the partial months 2023-06 and 2026-06 are excluded.
- k rule: a row is emitted only when `denominator >= 10`, and `numerator` and `denominator - numerator` are each `0` or `>= 10`. So `(valid, pos)` is masked whenever either is 1-9; a zero is published only with `valid >= 10`. A masked row is dropped (the sensor treats absence as missing, never as 0); only the suppressed count per metric is reported in the local summary.
- No margins: no row without dims is emitted for M7-M10, so a masked cell cannot be recovered by subtracting a published total. (The existing M3 and M6 complementary-suppression rules are unchanged.)
- Reference files (`customers.csv`, `marketing_campaigns.csv`) are read through a column allowlist (`REF_COLUMNS`); PII columns are never indexed.
- Customer attributes (segment, consent) are the extract's CURRENT values, not as-of the event. No timezone claim.

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
