# Claude review logs (CRV0)

One `<name>.review.json` per reviewed WP group, format `review-log/v1`:

- `wps`: WP ids covered. `author.id` and `reviewer.id` must differ after normalisation (case, `-`, `_`, space).
- `provenance`: `contemporaneous` or `reconstructed`. A reconstructed log must list `sources` (journal entries, commits) and
  contains only findings those sources record. `reviewer.identity_recorded: false` says the per-review identifier was not kept.
- `findings[]`: `id`, `loop`, `summary`, `status` = `open` | `fixed` (needs `fix_ref`, optional `red_ref`) | `accepted` (needs `reason`).
- `verdict`: `closed` only when no finding is open.

Closure: `python scripts/gov/crv0_closure.py` exits 0 only when every log is valid and closed and every required WP (CRV0
dependencies plus the E0 data path) is covered by a closed log. Today it exits 1 on purpose: the open findings (TPS opaque ids,
ED0L complementary suppression, M2a runtime wiring, E0 sensor build receipt) are real and are not closed by this backfill.
The backfilled logs cover loop 1 only; loops 2 and 3 have not happened.
