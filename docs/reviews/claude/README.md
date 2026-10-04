# Claude review logs (CRV0)

One `<name>.review.json` per reviewed WP group, format `review-log/v1`:

- `wps`: WP ids covered. `author.id` and `reviewer.id` must differ after normalisation (case, `-`, `_`, space).
- `provenance`: `contemporaneous` or `reconstructed`. A reconstructed log must list `sources` (journal entries, commits) and
  contains only findings those sources record. `reviewer.identity_recorded: false` says the per-review identifier was not kept.
- `findings[]`: `id`, `loop`, `summary`, `status` = `open` | `fixed` (needs `fix_ref`, optional `red_ref`) | `accepted` (needs `reason`).
- `verdict`: `closed` only when no finding is open.

Closure: `python scripts/gov/crv0_closure.py` exits 0 only when every log is valid and closed and every required WP (CRV0
dependencies plus the E0 data path) is covered by a closed log. `closed` does not mean risk-free: residual risks stay in the
log as `accepted` findings with `reason`, `owner`, `tier` and `tracking` (hex-encoded ids in TPS, undeclared overlaps in ED0L,
unrun real-PG16 and gateway smoke, post-hoc sensor receipt). `closure_verification` records who verified the closure and how;
the closer is not an independent reviewer, and loop-2 reviewer identities were not recorded.
