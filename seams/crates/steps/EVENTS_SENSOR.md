# R1G: `sensor=rust-events` (Team CL)

A real Rust sensor over the event packages the monitor tick writes (`packages/<id>/{events.ndjson,manifest.json}` plus the
optional `cases.ndjson` dimension). Code: `src/events_sensor.rs`; tests: `tests/ev_sensor.rs` (generator in
`tests/ev_common/`), `../sources/tests/rust_events.rs` (through `monitor::tick`).

## Selecting it

- `monitor::tick` config key `sensor = stand-in | rust-events`. Default: `rust-events` in platform mode (tests green),
  `stand-in` in dataset mode (the stand-in wraps the Codex runner on E0 packages; this sensor does not read E0).
- Engine job path: `STEPS_SENSOR=rust-events` makes `steps::sensor::run` call this sensor (package root from
  `STEPS_SNAPSHOT_ROOT`, tunables `STEPS_MIN_SUPPORT`, `STEPS_ARRANQUE` (min cases per cell), `STEPS_K_ANON`,
  `STEPS_MIN_HISTORY_DAYS`, `STEPS_MIN_HISTORY_CASES`).

## What it computes

Cells are `language/channel`. The events carry no cell, so the tick also writes `cases.ndjson` (allow-listed columns
`case_id, channel, language, priority, previous_case_id` of the batch's cases; never `customer_id`, never text). Per cell:
`reassignment_rate` (case with a second `case.assigned`), `recurrence_rate` (`previous_case_id` set),
`first_response_delay`, `resolution_delay` (mean seconds). Candidate = cell WORSE than the rest of the population.
History is cumulative: the sensor reads the package plus earlier packages of the same `source_id` (watermark_to not beyond
the current one, newest first, 96 MB budget), so replays only see their own past.

## Statistics

1. Quarantine malformed or hostile rows (counted by reason, never echoed).
2. Cold-start gate (days of event time and cases opened) => `insufficient_history` discards, nothing admitted.
3. Split by `event_log.sequence`: discovery = first 60% of the span, holdout = rest; a case belongs to the window of its
   `case.opened` sequence.
4. One-sided tests: pooled two-proportion z (rates), Welch z (delays). Candidate needs nominal p <= 0.05 and a minimum
   effect (rate +0.05 absolute, delay x1.25).
5. Multiplicity: Bonferroni over all (family, cell) tests of the discovery window (alpha 0.05, so p <= 0.05/m).
6. Replication in the holdout: same direction, support (`min_support` positives or `min_cell_cases` observations), and
   p <= 0.05/K over the K discovery survivors. Only then ADMITTED.
7. Volume drift (cases/hour, holdout vs discovery, ratio >= 1.5 and |z| >= 3.29) is reported under `drift` and as a
   `volume_level.all` `drift_only` discard; delay candidates are then discarded `drift_only`.
8. k-anonymity: a cell with fewer than `k_anon` (10) cases is never reported (`k_anonymity`).

## Output

- Frozen `engine-steps/0` sensors output (unchanged schema). Named reasons map to its enum:
  `not_replicated -> failed_holdout`, `low_support | k_anonymity -> below_k`, `insufficient_history -> low_coverage`,
  `drift_only | multiple_comparison -> other`. `population` is the cell, `holdout_checked` is always true for admitted signals,
  numerator/denominator are the cell's counts over both windows (delays: numerator = cell cases slower than the rest's mean).
- Versioned superset `sensor-events/1` (`Report::to_json`, in the tick record under `sensor.report`): exact discard names,
  discovery/holdout rates and p-values, drift, quarantine counts by reason, `method`, and `not_done`.
- Downstream (scout/recompute) consumes only the frozen shape. W7: the recompute handler (`engine::adapters`) accepts a sensed
  `metric_id` (`reassignment_rate.pt.web_chat`) as a signal id next to the sensor's own `sig-NNNN` (additive; the `signal_id` rule is
  unchanged), and `thread10::SignalSeed::with_metric` registers the cell metric id for the scout/verifier/builder requests. In `pulso run`
  each admitted signal becomes one `SignalSeed` (cell metric id, numerator/denominator of the cell, data class of the record).

## Not done (honest)

Payload and free text are never read; no per-staff or per-customer analysis; cases without a dimension row are excluded;
improvements are not candidates; no causal claim; no seasonality/week-over-week; no day-clustering (overdispersion) correction, so bursty cell-days inflate the false-positive rate (independent review: ~5-15% under strong bursts vs 0-2% iid); release and observation stay simulated;
the dataset-pg (E0) path is not served by this sensor; `read_dimension` reads at most 10000 `cases` rows per batch (cases
beyond that cap lose their cell: counted as `cases_without_dimension`).
