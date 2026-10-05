# Outcome monitoring validation (T1)

T1 is **partial against the round-2 brief**: it implements a deterministic
screen, finite overlapping-window placebo summary, and aggregate-shift
response check, but does not establish a calibrated <=5% false-positive rate,
statistical power, or MDE. See the explicit acceptance boundary in
[`discovery_v1.md`](discovery_v1.md). Do not describe this diagnostic as a
validated estimator or use it to claim a release improved outcomes.

This module is an exploratory screen for an imagined release boundary. The
historical bank extract contains no release history or untreated cohort, so
the analysis does not estimate causal impact or business lift.

## Input and output contract

`scripts/aggregate/outcome/outcome_estimator.py` is the canonical T1 module. It
consumes the exact six-field NDJSON records produced in memory by
`scripts/aggregate/bank_cells.py`. The older
`scripts/aggregate/outcome_estimator.py` module is a compatibility wrapper to
the same implementation; both module entrypoints emit the v4 report contract.
T1's CLI uses its frozen registered scope and reads only
`call_center_interactions`, `complaints`, and `satisfaction_surveys`; it does
not scan the remaining bank tables. The active `bank_cells.py` producer also
emits M10 from call interactions (unresolved handled-time share), so M10 is
part of the registered source metric vocabulary and uses the same bounded
`reason_category` × `channel` dimensions. A report is preregistered only when the
exact protocol revision is committed before that report is generated.

M10 is retained in the report only as a descriptive source metric. Its
denominator is handled hours floored from duration seconds, not a count of
independent Bernoulli observations. The estimator therefore does not calculate
an M10 difference-in-differences effect or interval and always emits
`status=inconclusive`, `reason=non_bernoulli_metric_descriptive_only`, and
`uncertainty_method=not_computed_non_bernoulli_aggregate`; its inferential
effect, interval, pre/post sample sizes, and control identity are null/empty.
In particular, an M10
change cannot be classified as `improved`, `worsened`, or
`no_detectable_change` using the generic binomial variance. The aggregate-shift
screen also cannot be interpreted as M10 detection sensitivity.
The report retains all registered cell rows, marks them with `inference_scope`,
and makes the scope explicit: only non-M10 metrics/cells enter Bonferroni
multiplicity, placebo status counts, and injected-shift response denominators.
M10 metric IDs and bounded cell labels are listed separately as
`excluded_descriptive_metrics` and `excluded_descriptive_cells`; k-suppressed
cell-count fields remain suppressed.

The estimator rejects
unknown metric/dimension schemas and maps source labels into the bounded
OPBENCH reason, channel, and PQR vocabularies before serialization. No source
label, identifier, raw row, or control count margin is copied into the report.
Upstream k=10 validation is re-applied after vocabulary mapping and merging.

For each inferentially screened cell, the estimator compares its three-month
pre/post change with published sibling categories, then checks whether the
direction appears in both customer-hash halves. Descriptive-only cells remain
in the output but do not enter this test family. These halves are replication cohorts,
not untreated controls. Missing months or sibling cells fail closed. The
report now names the bounded sibling dimension values selected for each target
cell, so the candidate comparison set is visible without exposing row-level
records. When required monthly sibling data is unavailable, those selected
labels were not sufficient to estimate the comparison and the cell remains
inconclusive. The sibling set is descriptive context, not an untreated causal control. The
reported status is only a candidate-screening label: the interval uses a naive
event-level binomial variance and is not adjusted for repeated customers.
The output names this limitation and labels all findings descriptive, not
causal or confirmatory.

The original preregistration and its amendments are recorded in
[discovery_v1.md](discovery_v1.md). The fixed primary boundary is 2025-01;
three full months before and after are required, with the boundary month
excluded. Bonferroni is used across screened cells and halves; M10 is
descriptive-only and excluded. The 1,500
support floor and 2 pp materiality margin are exploratory screening settings,
not validated release gates.

## Temporal screen and limitations

Eligible placebo boundaries are sampled without replacement with a fixed
seed. Windows overlap in time, so their results are dependent; the report only
reports the finite-horizon empirical candidate-window frequency and whether
that observed frequency is at or below the preregistered 5% screening bound.
This is not a calibrated type-I error rate, confidence interval, or general
false-positive guarantee because the windows overlap and are dependent.

The fixed `2025-01` boundary also receives deterministic 2, 5, and 10 pp
aggregate reductions, one treated cell at a time, in both hash halves. The
report summarizes the estimator's response rate by mean monthly support bin
and labels the 80% threshold as algorithmic, not statistical power or MDE.
Rows that would violate k=10 are suppressed and make that target inconclusive.
This checks the algorithm against arithmetic shifts; it does not estimate the
probability of detecting a real intervention. Customer-cluster resampling or
equivalent sufficient statistics would be needed for confirmatory inference.

The underpowered-cell list is withheld when its size is 1-9. All reports are
aggregate-only and remain outside Git.
The placebo numerator and its complementary nonsignal-window count are both
privacy-checked. If either side is 1-9, the rate and `screen_bound_met` are
withheld together (`summary_suppressed=true`); publishing the rate or threshold
decision alone could reveal a small count. Zero and counts of at least ten are
publishable, subject to the stated non-independence limitation.

## Earlier preregistered v3 run (2026-10-05)

The protocol was committed as `df40b514` before this report was generated.
Two full runs over the registered tables produced byte-identical aggregate
JSON (SHA-256 `462AFF83F14AE1D20469F995E9E85C17642C4A704DF5F779D3DF72C11EFF81F0`);
the reports themselves remain outside Git. This recorded artifact predates
the M10 non-inferential and scope-exclusion rules in active protocol v4. In
that artifact, five M10 cells were counted among minimum-support failures;
with current code they instead receive the descriptive-only `inconclusive`
reason, and are excluded from inferential family sizes and injected-shift
denominators. Refresh before treating its hash, support-failure count, or
shift-response rates as current. The producer emitted the T1 metric family
over 35 full month labels and 84 registered metric/dimension cells.

| Diagnostic | Observed result | Interpretation |
| --- | --- | --- |
| Primary 2025-01 screen | 84 cells; all inconclusive | No evidence here that any cell improved or worsened. Missing complete treated/control windows and minimum-support failures keep the result inconclusive. |
| Minimum-support failures | 25 cells | These do not meet the exploratory 1,500-observation window floor; the report lists their bounded metric/dimension labels without their counts. |
| Temporal placebo | 0 of 29 eligible windows with any candidate signal; empirical frequency 0% | At/below the 5% screening bound for this finite overlapping-window screen only. It is not a calibrated false-positive rate or Type-I error guarantee. |
| Injected aggregate shifts | The only publishable support bin is `<500`; algorithmic response was 0% at 2, 5, and 10 pp; threshold `>10 pp` | This is a deterministic sensitivity check of the estimator on shifted aggregates, not statistical power or a real intervention effect. Other support-bin summaries are suppressed. |
| Published pre/post sample counts | No nonzero `n_pre`/`n_post` value in the report falls in 1–9 | The emitted report passed this count-field k scan; the input builder separately applies k=10 and complementary suppression. |

Thus the immediate finding is not “the service improved,” but that the
historical extract cannot support a reliable per-cell outcome verdict under
the frozen design: no primary cell is classifiable, and the tested detector
does not surface injected shifts up to 10 pp in the only publishable support
bin. This is a measurement/observability limitation, not evidence that a
future release would have no effect.

## Regeneration

Run from the engine repository root, writing outside Git:

```powershell
python -m scripts.aggregate.outcome.outcome_estimator `
  --data-root D:\.codex\factored\data `
  --out D:\.codex\factored\outcome-temp\report-m10-scope-v4-20261005.json
```

This is the exact refresh command for the active v4 protocol; the CLI
refuses to overwrite an existing output, so choose a fresh path for later
runs. The legacy command
`python -m scripts.aggregate.outcome_estimator` is retained for compatibility
and delegates to the same CLI implementation.

The command reads only `call_center_interactions`, `complaints`, and
`satisfaction_surveys`, builds the bank-cell table in memory, and writes a
stable aggregate report. It never persists raw or intermediate rows. Repeat
to a distinct output path and compare SHA-256 hashes. Do not use these
screening results alone to approve a customer-facing release.

Tests:

```powershell
python -m unittest scripts.aggregate.tests.outcome.test_outcome_estimator -v
```
