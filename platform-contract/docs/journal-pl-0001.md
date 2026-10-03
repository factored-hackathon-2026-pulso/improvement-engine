# Journal PL-0001: platform_live contract pack and simulator (Team Claude)

Date: 2026-10-03. Scope: PL-L3 (platform-contract/) then PL-L2 (platform-sim/platform_live/).
Input: Product artifact BWx4saeWfYsLbQEbkNKMPg (commit a492bfa), spec section 32, impact analysis doc.

## PL-L3

- First RED: `platform-contract/tests/test_contract.py` failed at collection (no `platform_contract`).
- Single source of truth `platform_contract/model.py`; schemas and `event-catalog.json` generated
  (`python -m platform_contract`); drift test regenerates and compares. 19 tests green.
- Decisions: names/emails/display_name/suggestions/search_text/previews are denied columns, not just
  untreated (spec 32.2.2 says names are never read). `staff` is exposed as `staff_dimensions`
  with both `team` and `team_id` optional. `analyst_availability`, `teams`, `admin_roster` are not
  allow-listed (availability is carried by `staff.availability_changed` events).
- Event catalog: 14 admitted (`case.*`, `turn.created`, `staff.availability_changed`, 4 `auth.*`),
  4 denied (`auth.password_accepted`, `auth.mfa_*`, `customer.session_started`), planned prefixes
  `staff.`/`team.`; unknown types are never a schema error (quarantine finding).
- `event_log.payload` is only "object": Product has not published per-type payloads (open question).

## PL-L2

- First RED: `platform-sim/tests/plive` failed at import (`platform_live` had no `PlatformLiveSim`).
- 18 tests (17 + 1 sqlglot Postgres-parse test, skipped if sqlglot is absent). Passes
  `conformance.check_database` clean, and under every injected fault the schema conformance still
  passes while findings match exactly the injected faults.
- Open: Postgres DDL is only parsed (sqlglot), not executed; PL-L6 should run it on a real PG.
  Payload shapes, `waited_seconds` and `open_cases_at_assignment` semantics are assumptions.
  Initial queue status is implied by `case.opened` (no `status_changed` for it).
