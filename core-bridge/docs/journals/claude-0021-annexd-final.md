# claude-0021: Annex D final - review fixes, rebase, local CI parity

## Fixes (first RED: tests/annexd/test_arm_request_annex.py, 409 idempotency_conflict on a later-deadline retry)
- Arms: `deadline` is excluded from the single-flight digest; a retry with a later or absent deadline replays the report.
  Annex D.4 defines no arms deadline-expiry error, so a past deadline is format-checked only (no `pulso:deadline_expired`).
- Arms: `agent_id` is filled (given, else derived from the target, read-only load) before hashing; omitting it equals
  sending the derived value, both directions tested. The broker payload digest uses the same normalised digest.
- ADR 0011: deadline/agent_id rules, `|` rejected in evaluation_context_ref inputs, Rust must copy the `evc-` formula exactly.
- Wire shapes unchanged: `bridge-contract/gen.py --check` clean, no golden re-record needed.

## Rebase
`git rebase --onto origin/main f129898` onto 1e81423 (main moved past 5ec0530); no conflicts; ADR numbering intact.

## CI parity (PG16 on Podman pulso-dev, 127.0.0.1:55477, PULSO_REQUIRE_POSTGRES=1; container removed afterwards)
lint pass; gen-wire -Check pass; modules-scan 1 passed; agent-core-assets 22 passed; platform-sim 246 passed/2 skipped;
core-bridge 637 passed/15 skipped; bridge-contract real 337 passed; mock 7 passed/29 skipped/149 xfailed (known divergences);
wire mock 186/1 skipped; wire a2 185/2 skipped. NOT reproduced: real-wire, Rust jobs. No stuck tests.
Note: a test run rewrites tracked core-bridge/.reports/exporter-report.json; it was reverted, not committed.
