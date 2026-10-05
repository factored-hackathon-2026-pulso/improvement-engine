# OPBENCH-lite cross-check and scoring protocol

This pack is an independent deterministic benchmark, not ground truth about
causes or production impact. Compare only normalized aggregate findings with
the same metric and population.

## Matching and agreement

Match on `(metric_id, normalized cell, adverse direction)`. The fixed dimension
vocabulary is `reason_category` (`complaint`, `transactional`, `technical`,
`general_inquiry`, `product`, `account`, `card`, `loan`, `other`,
`unclassified`), `channel` (`phone`, `web`, `chat`, `email`, `branch`,
`mobile_app`, `other`), and `pqr_category` (same closed output set). M6 channel
is the survey delivery channel; E1 has only `{"scope":"overall"}` and no
signature or signature digest is exposed.

Two findings agree when their keys match, their status family agrees
(`corroborated*`, `refuted`, or `uncertain/candidate*`), and the absolute
difference between effect estimates is at most 0.05 (five percentage points).
If both provide intervals, overlap is required for full effect agreement;
non-overlap is an acceptable disagreement to investigate, not a reason to
average the values. A difference in significance caused by a documented
multiplicity family or k-support rule is acceptable if the direction and
estimate agree within tolerance. Missing from one side is a miss, not a
disagreement in effect size.

## Scores

- **Recall** = matched independent positive benchmark findings / all benchmark
  positive findings.
- **Precision** = matched independent positive benchmark findings / all
  engine positive findings. An engine finding with no benchmark match counts
  as unmatched, not automatically false in the underlying bank.
- **Ranking agreement** = Spearman rank correlation over matched findings
  ordered by absolute adverse rate difference; calculate only with at least
  three matched findings and otherwise report `not_scored`.
- **Coverage** = matched benchmark keys / benchmark keys with released `k`
  support. Report alongside recall so suppression is visible.

The supplied `scoring.example.json` is a schema example only; no engine run or
score is asserted here. The protocol does not instruct or import Claude's
implementation and is intentionally safe to use independently.

In `opbench-lite.json`, the top-level `numerator`/`denominator` are the
deduplicated full-snapshot estimate (including rows without a split key when
the metric permits); `discovery` gives the discovery-half cell and pooled
complement used for effect testing; `replication` gives the independent
second-half figures. All three are separately suppressed if either event or
non-event support is below `k=10`.
