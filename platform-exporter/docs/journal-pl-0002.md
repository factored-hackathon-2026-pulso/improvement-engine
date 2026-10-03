# Journal pl-0002: platform-exporter (PL-L1)

Spec: TECH_SPEC_PULSO_AUTOMEJORA_V3 section 32 (+24, 24.2, 31.6). Contract: `pulso-observations-2` HTTP DTO of the core
exporter (plan: Batching and DTO row, X-3, DR-10, CLQ-07). Code: `platform-exporter/src/platform_exporter/`, tests
`platform-exporter/tests`. Python 3.12 via uv.

## Purpose
Read-only cursor over the real platform `event_log` (SQLite or Postgres) that emits `platform_event` observations to
`POST /internal/v1/platform/observations`, without ever reading credential tables or the mutable `cases` state.

## Flow
poll -> read `event_log` rows after the cursor (global contiguous `sequence`) -> hold back sparse holes during the grace
period -> classify (admitted / denied / planned / unknown / bad row) -> persist the batch -> POST -> durable ACK -> one
local transaction (delta + cursor). Backfill: rows appearing inside a declared gap are re-read and sent as `late`
(`scan_mode=rescan`, never moves the checkpoint). Rescan: capability profile, dimension snapshot digest, per-case
`turns.sequence` completeness findings.

## Mapping (spec 32.2)
- `kind=platform_event`, `level=null`, `source_sequence=event_log.sequence`, `native_event_id=event_id`,
  `observed_at=available_at=ingested_at` (measured; no replay assumption), partition `tenant.<tenant_id>`, source
  `<instance>.events`. Treated payload inline in `source_event` (no artifact blobs needed); only the schema artifact is
  uploaded.
- Quality findings are `platform_event` observations with `event_type=exporter.finding` and a deterministic id
  (`finding:<type>:<key>`), so retries and rescans dedupe: `unknown_event_type` (with `catalog_status` unknown|planned),
  `denied_event_type`, `bad_row`, `gap_suspected` (`backfill_requested`, `verify_with_owner`), `late_event`
  (`window_start/end`, `window_revision_required`), `turn_sequence_gap`. The DTO has no other channel for them.
- Late event: `ingested_at > window_end(event_time) + allowed_lateness`; event gets `coverage_marker=late` plus a finding.
- Simulator rows: `customers.simulator=1` (via `cases.customer_id`, an immutable dimension column) gives
  `evidence_kind=team_generated`, `population_excluded=true`.
- Capability profile manifest `platform_live.phase1/1`: introspected from the live schema (names only), emitted once per
  digest on the next batch and on every rescan; a superset schema flips capabilities to `present`.

## Denial by construction
`policy.select_sql` asserts table and columns before any SQL text exists (PL-01 first RED). The SQLite source also
installs an engine-level authorizer (denied tables/columns and every write are refused by SQLite itself; reads can be
recorded for tests). The Postgres variant relies on the same builder plus a least-privilege role with column grants; the
tests prove the DB refuses `login_accounts`, `cases.status`, `staff.name`, `turns.text` as well.
State never comes from `cases`: `case_extract(cutoff)` rebuilds from events visible at the cutoff
(`event_time <= cutoff` and `ingested_at <= cutoff`).

## Commands (run, from `platform-exporter/`)
    set UV_PROJECT_ENVIRONMENT=<venv outside the repo>
    uv run --python 3.12 pytest                      # 39 pass with PULSO_TEST_PG_ADMIN set; 35 pass + 4 skipped without
    uvx ruff check --isolated --select E4,E7,E9,F .
Postgres: throwaway `postgres:16` on machine `pulso-dev` (`--connection pulso-dev`, `--cgroups=disabled`, bound to
127.0.0.1), removed afterwards.

## RED / GREEN
First RED (PL-01, `tests/test_pl01_allowlist.py`): `ModuleNotFoundError: platform_exporter.policy`. The PL-02..PL-05
and PL-08 file (`tests/test_first_red_pl02_pl05_pl08.py`) failed at collection with `ImportError: cannot import name
'Exporter'` before the implementation existed; then 15 passed. Later: 29 (own SQLite fixture), 35 (+ simulator), 39 (+ real PG16).

## Trade-offs
- Findings travel as observations (no DTO change needed) at the cost of a synthetic `exporter.*` event_type namespace
  that Codex's adapter must route to quality findings; the quarantined payload itself is never forwarded.
- Hold-back of sparse holes (default 120 s) trades latency for fewer false `gap_suspected`.
- Column allow-list follows the Product artifact names (`cases.id`, `staff.id`, `event_log.entity/actor_role`).

## Gaps
- Payload shapes per `event_type` are simulator assumptions (Product did not publish them); `asof` reads `to`/`status`,
  `close_reason`, `staff_id`.
- `start_sequence` defaults to 1: a platform whose log starts later needs it configured, otherwise a gap is declared.
- Token minting uses an injected `token_for` (static env token in `__main__`); the ADR 0009 key delivery and the
  exporter role/secret are PL-L5. No Dockerfile or compose fragment here (infra is not this package).
- The real control-api was not exercised; the receiver is `platform-sim/ingest_fixture` (a double), which accepts
  `kind=platform_event` with `level=null` per the shared DTO.
- Staff dimension is only digested (never forwarded); the `tenant_id` filter on `event_log` is not applied (single
  tenant per database assumed; the optional column is read but not used for filtering).
