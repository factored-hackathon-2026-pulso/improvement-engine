# claude-0012: independent review of alias read + authoring dry-run (commit 83001c6)

Reviewer: independent Claude agent. Pin used: agent-core 789d6c8 (the 86a7674 venv lacks `get_alias`/`RELEASE_SETTINGS`).

## Findings
- MEDIUM (fixed): dry-run answered 500 for non-finite JSON numbers (`NaN`), absurd nesting in the body or in
  `content` (RecursionError / canonicalisation failure outside the error mapping). Now 422 `pulso:invalid_request`,
  nothing written. Test: `test_dry_run_non_finite_numbers_and_deep_nesting_are_422_and_write_nothing`.
- LOW (open, mock-owned by bridge-contract): mock returns 400 for `invalid_request` and uses purpose `state_read`
  for alias reads, real returns 422 / `alias_read`; already listed as known-different (MOCK-STATUS). Mock does not
  simulate `base_release_unknown`.
- OK: read-only (store transaction only calls get_release/release_refs/get_version/blobs; no proposal/quota/event rows,
  PG table snapshot test), cross-agent `base_release_id` is the same 404 as unknown, alias 404 identical for unknown
  agent / unset alias, release_settings denied before evaluation, guardrail edits reported, body cap 413 enforced by
  the app, 51 changes -> REG-LIMIT, hashes equal the real freeze (in-memory and PG tests).
- Note: tenant is only the deployment allow-list plus body/claim equality; Core's registry is single-tenant, so
  alias reads are not tenant-partitioned by design.

## Runs (real PG16, pulso-dev)
authoring+runtime+wire+llm+l5+l6: 311 passed, 5 skipped; tests/integration: 37 passed; test_image.py not run.
