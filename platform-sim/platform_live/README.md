# platform_live simulator (PL-L2)

Deterministic, SQLite-backed double of the real support platform (Product artifact, commit
a492bfa). Pulso-side counterpart of `core_mock`, for exporter and engine tests.

    from platform_live import PlatformLiveSim
    sim = PlatformLiveSim(seed=7, path="platform.db")   # ":memory:" by default
    sim.generate(n_cases=60, faults=True)                # scenario + injected faults

- 11 tables (`render_ddl("sqlite"|"postgres")`; timestamps TEXT/TIMESTAMPTZ, JSON TEXT/JSONB).
- `event_log`: explicit contiguous `sequence`, `event_time`, `ingested_at`, JSON `payload`.
- Mutable `cases`/`staff`/`customer_case_slots` with `version`; append-only `turns`,
  `assignments`, `event_log` (SQLite triggers make violations raise).
- Rules: one open case per customer, reopen only via a new case with `previous_case_id`, H1
  language assignment, SLA 5/15/60 min, queue drain on availability.
- Demo customers (`simulator=1`); credential tables hold only `FAKE-*` placeholders.
- Faults: `inject_late_event`, `inject_sequence_gap`, `inject_turn_gap`,
  `inject_unknown_event_type`, `evolve_teams` (teams/admin_roster, `staff.team` -> `team_id`,
  `team.*`/`staff.team_changed` events). Every fault is recorded in `sim.faults`.
- Event payload shapes are simulator assumptions (not published by Product); no message text.

Tests: `uv run --python 3.12 --no-project --with pytest --with jsonschema --with sqlglot python -m pytest platform-sim/tests/plive`
(conformance via `platform-contract`).
