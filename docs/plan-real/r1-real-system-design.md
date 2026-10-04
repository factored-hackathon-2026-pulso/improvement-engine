# R1 - Minimum real system design

Status: design, 2026-10-04 (Team CL). Goal: one real engine container in an AWS ECS task, one Postgres, one object
store, monitoring a support platform's data, proposing changes to the agent-core artifacts that platform runs, with
honest labels for everything that is still simulated. Nothing here is built unless the table in section 3 says so.

## 1. Principles (binding decisions)

1. **The signal source is the support platform.** The engine monitors what the platform generates (`event_log`,
   `cases`, `turns`, `assignments`, escalations, calls). E0 and the 13 bank CSV tables are *development, seed, replay
   and evaluation* data: E0 is synthetic platform history shaped like the platform output; neither is production.
2. **One database, one storage, shared by every component.** One Postgres instance with schemas `raw`, `augmented`,
   `product`, `pulso` (no per-component databases) and one object store (one bucket, prefixes in section 8). The
   Core runtime sidecar keeps its own logical databases only where agent-core requires them (it is a Core
   dependency, not an engine component); everything we own lives in the shared instance.
3. **Two data modes, one code path.** `data_mode = dataset | platform` (section 4). The sensor reads through one
   source-adapter port; it does not know which mode or adapter feeds it.
4. **Two product adapters behind that port**, SQLite and Postgres (section 5).
5. **Honesty first.** Every run records `data_mode`, `data_origin`, `source_id` and the evidence class of each stage.
   Release and observation events stay `simulated` until the platform emits them (EXT-2).
6. **Privacy.** The denylist columns/tables of the platform are unreadable by construction (no column, no grant).
   E0 is never sent to hosted models; readers cannot see free text (column-level grants, `db/sql/090_grants.sql`).
   The evaluator tables (`labels`, `timeline`) and `pseudonym_map` are never loaded.

## 2. Flow (monitor loop)

```
source adapter (product SQLite|Postgres | dataset raw/augmented)
  -> watermark read (event_log.sequence > last, per source_id)       [pulso.source_watermark]
  -> normalise to platform events (E0 mapping, section 6)
  -> signals (windowed counts/rates; history-dependent ones gated, section 7)
  -> problems / opportunities (typed, with evidence ids, no free text)
  -> proposal (agent-core artifact, only kinds the pinned Core accepts)
  -> Core validate (dry-run) -> Core native evaluation
  -> viable / non-viable ledger (with reason)
  -> human approval (control-api)  -> publish to staging (Core publish)
  -> observation of the release window: `simulated` until EXT-2
```

Proposal kinds are limited by `contracts/artifact-kinds/matrix.json` (BK0, Core pin c814c2b): only `prompt`
(`replace` operation only) is `supported` on propose/validate/evaluate/publish, and `eval_suite` is draft-only
supported. `flow, agent, decision_model, policy, template, language_detection, injection_ruleset, model_profile,
knowledge_snapshot` are `denied` on propose; `tool` is `blocked` (no executor); `release_settings` is denied on every
verb. A problem whose best fix is any other kind ends as `not_evaluable` (a valid ledger outcome), never as a forged
proposal.

## 3. Components: exists vs missing

| Component | Exists | Missing for R1 |
|---|---|---|
| Source adapter port + SQLite/PG adapters | `platform-exporter` `PostgresSource`/SQLite source with allow-list policy (Python, contract 1.1.0) | Rust (or exporter-sidecar) port with two adapters, `source_id` and watermark table in `pulso`; exporter stays the policy reference |
| Postgres layout | migrations 0050-0052 (jobs, claim/commit, control api), roles bootstrap (`seams/crates/pg`) | `db/` DDL (this change), loader (this change), `pulso.source_watermark`, `pulso.run_source` migrations (L-PG range) |
| Monitor loop + signals | detectors in Codex crates, signal portfolio; Scout drafts (U13), platform observation ingestion (U29) | loop driven by watermark over the port; history gating (section 7) |
| Proposals, Core validate/evaluate/publish | bridge, BK0 matrix, real Core live for `prompt` replace | none for `prompt`; other kinds blocked by Core |
| Ledger, approval | durable run control (U34), control-api | viable/non-viable ledger rows keyed by `source_id` |
| Release/observation | simulator events | real events: EXT-2 (product) |
| New `pulso` binary, console | other agents' work (do not touch) | wiring to `data_mode` and profile display |
| Platform in Postgres | none: support-platform is SQLite by default, Postgres untested, no migrations, slice 12 vs our 1.1.0 (a492bfa) | product team deliverable (gap G1) |

## 4. Data modes

`data_mode` (config, env `PULSO_DATA_MODE`) selects only *which adapter and which source id* the loop reads:

| | dataset mode (A) | platform mode (B) |
|---|---|---|
| Source | `raw` E0 / `augmented` canonical, loaded by the user | `product` via SQLite or Postgres adapter |
| Purpose | demo, replay, evaluation cases, development | real signals from the live platform |
| `data_origin` | `dataset_replay` (E0: `synthetic_platform_history`, bank: `bank_csv_seed`) | `platform_live` |
| Time | replay clock over history, deterministic | wall clock, watermark follows `event_log.sequence` |
| Labels shown | "demo/replay data, not production" | "real platform signals; release and observation simulated until EXT-2" |
| Release/observation | `simulated` | `simulated` until the product emits release events |

Same code path: dataset mode maps E0 history to platform events (section 6) and feeds the same sensor; the sensor
sees a stream of platform events and cannot tell the mode.

Recording: every run row stores `data_mode`, `data_origin`, `source_id`, `adapter`, `watermark_from/to`, and
`evidence_class` per stage (`real-core-live`, `simulated`, ...). The console profile and `doubles[]` show them
(`data_mode`, `data_origin`, adapter, and one double for release/observation).

Coexistence: both modes can live in one database. Isolation rules: (a) each source has its own `source_id`
(`dataset:e0:<batch>`, `platform:<tenant>`) and its own watermark row; (b) a run is bound to exactly one `source_id`
and never reads another; (c) signals, problems and ledger rows carry `source_id`, and queries never join across
sources; (d) the database role of a platform-mode run has no grant on `raw`/`augmented` and vice versa
(grants per schema, `db/sql/090_grants.sql`).

## 5. Source-adapter port

```
trait SourceAdapter {
  fn source_id(&self) -> &SourceId;                       // stable, per data_mode
  fn read_events(&self, after: Watermark, limit: usize)   // ordered by sequence, allow-listed columns only
        -> Result<Vec<PlatformEvent>>;
  fn read_dimension(&self, table: AllowedTable, ids: &[Id]) -> Result<Vec<Row>>;  // cases/staff/turns metadata
  fn schema_fingerprint(&self) -> Fingerprint;            // drift detection against contract 1.1.0
}
```

Adapters: `product-sqlite`, `product-postgres`, `dataset-pg` (reads `raw`/`augmented` and emits mapped events).
Config: `PULSO_SOURCE_ADAPTER = product-sqlite | product-postgres | dataset-pg`, plus a DSN or path in secrets.
Both product adapters build every query through the allow-list (as `platform-exporter/policy.py::select_sql`) and
fail closed on any table or column outside it; Postgres additionally connects as `pulso_product_ro` (column-level
grants, `default_transaction_read_only=on`); SQLite is opened with `mode=ro` plus the engine-level authorizer.

SQLite product source in a shared-infra world: the platform's SQLite file is a snapshot, never opened for write by
us. Two supported shapes, both read-only: (1) *alongside*: the adapter reads the file directly (mounted EFS or a
copy fetched from the shared bucket under `snapshots/`), watermark in `pulso.source_watermark`; (2) *into*: a
snapshotter copies allow-listed columns into `product.*` with the loader (generic, lineage columns), after which the
Postgres adapter reads it. R1 uses (2) for ECS (no file system dependency in the task) and (1) for laptops.

## 6. Mapping E0 history to platform tables and event types

Used by `dataset-pg` so the same sensor reads either source. E0 text columns are never read by the sensor.

| E0 table | Platform table | Platform event type(s) | Notes |
|---|---|---|---|
| case | cases | `case.opened`, `case.queued` | `customer_id` PSN- pseudonym; channel/priority enums need reconciliation (gap G3) |
| case.assigned_analyst_id, routing_step(tier=human) | assignments | `case.assigned` | `policy_rule_id` maps directly |
| turn | turns | `turn.created` | only metadata (role, time, kind); `text` not granted |
| case_close | cases (closed state), event_log | `case.closed`, `case.status_changed` | `resolution_code`, `csat` stay payload metadata |
| first analyst turn | event_log | `case.first_responded` | derived from the first non-customer turn |
| identity_check, routing_step, copilot_query, tool_call, approval, signal | none in platform 1.1.0 | (none) | evidence for dataset-mode signals only; platform equivalents arrive with agent-core integration |
| escalations, calls | escalations, calls | `case.escalated`, `call.*` (not in catalog 1.1.0) | gap G2 |

The adapter synthesises `event_log.sequence` as a dense replay ordinal (`event_time`, then id) so watermark logic is
identical; `event_id` is a deterministic hash of table and key.

## 7. Cold start and small volumes (only product data exists)

- **Day 0:** no history. The loop runs with watermark 0, ingests, and emits only *history-free* signals: state
  signals computed from the current window (open-case age against `sla_due_at`, queue length, unassigned time,
  escalation count, reopen via `previous_case_id`). Each signal carries `n` and the window.
- **History-dependent signals** (week-over-week change, baseline drift, seasonality, per-analyst deviation) are
  gated by minimum history (default 14 days and 200 cases per scope, configurable) and report `insufficient_history`
  with the missing amount instead of a value; they are not silently skipped.
- **Small volumes:** every signal has a minimum `n` and reports `support_cases`/`support_analysts` as E0 `signal`
  does; below threshold it is `weak` and cannot open a problem alone. Proposals need a problem with enough support.
- **Bootstrap with data:** the user may load E0 into `raw` (dataset mode) to exercise the same sensors and to supply
  evaluation cases; it never feeds platform-mode baselines (modes do not mix).
- **Outcome:** with only a fresh platform, expect signals but few or no proposals; that is a valid, honest state.

## 8. Targeting agent-core artifacts on the platform

Today the only artifacts that exist are our seeded world in Core (the BK0 `prompt` entities). When agent-core is
integrated into the platform, proposals must target the artifacts the platform actually runs: the proposal's target
is a Core entity reference (kind, id, version) resolved by a *target catalogue* read from the Core registry at
start. **Integration point with the agent-core team, via the user (gap G5):** the identifiers of the platform-run
agent and its prompts, the Core environment the platform uses for staging, and the publish mechanism. Until then
targets come from the seeded world and runs are labelled `target_world = seeded`.

Single object storage (one bucket, prefixes): `artifacts/` (proposals, candidate digests), `evidence/`
(treated evidence packs, no raw text), `reports/` (run reports, receipts), `console/` (built debug-console),
`snapshots/` (SQLite product snapshots), `datasets/` (user-uploaded E0 Parquet and CSV, private, never to hosted
models), `loader-staging/`. Per-prefix IAM: engine read/write on its prefixes, console read on `reports/` and
`console/`, loader read on `datasets/` and `snapshots/`.

## 9. ECS task layout (R4 target)

One task, four containers, one RDS Postgres, one bucket:

| Container | Role | Notes |
|---|---|---|
| `pulso` | engine + control-api + monitor loop | essential; exposes `/healthz` (process), `/readyz` (DB reachable, migrations at head, Core reachable) |
| `core-runtime` (sidecar) | agent-core runtime via the bridge | localhost; pinned image digest |
| `llm-gateway` (sidecar) | the model gateway (not ours to build) | secrets for model keys only here |
| `pulso-console` (optional) | static console build served from `console/` prefix or by `pulso` | no extra DB |

RDS Postgres (private subnets): databases = `pulso` (schemas raw, augmented, product, pulso) and, if Core needs it,
its own database in the same instance. Roles from `db/sql/001_roles_schemas.sql`; logins and passwords are set by
bootstrap from Secrets Manager, never in SQL or Git.

- **Config:** env vars (`PULSO_DATA_MODE`, `PULSO_SOURCE_ADAPTER`, `PULSO_PG_*_DSN` references, bucket and prefixes);
  non-secret in the task definition, secrets by ARN from Secrets Manager.
- **Migrations on start:** `pulso` runs `db/sql` (idempotent, `IF NOT EXISTS`) then `migrations/` under an advisory lock,
  then reports ready; failure keeps `/readyz` red and the task unhealthy.
- **Health:** container healthcheck on `/healthz`; ALB or ECS dependency ordering `core-runtime` HEALTHY before
  `pulso` starts.
- **SIGTERM:** stop claiming new jobs, finish or release leases (fence tokens already make a stale commit safe),
  flush the watermark, exit 0 within `stopTimeout` (>= 60 s).

## 10. Rungs

| Rung | Scope | Exit evidence |
|---|---|---|
| R1 | all local containers: Postgres (one instance, schemas), Core runtime, gateway stub; dataset mode and a simulated platform (SQLite and Postgres adapters) | acceptance checks below green with receipts |
| R2 | real models through the gateway (paid, scoped authorization), same code path | model attempt ledger, `doubles[]` shows `real` for models |
| R3 | real inbound read-only from the product (P3), still `simulated` release | signals from a real tenant, schema fingerprint stable |
| R4 | ECS: the task layout above, RDS, one bucket | first apply, health, SIGTERM drill, restore from RDS snapshot |

## 11. Risks

1. Product Postgres backing does not exist (SQLite default, no migrations): adapter (2) snapshot is the fallback.
2. Contract drift: platform is at slice 12, our contract 1.1.0 at a492bfa; fingerprint check fails closed.
3. Enum and id reconciliation (channel, priority, PSN-/CLI-) between E0, bank and platform.
4. No release/observation events: improvement stays unproven in platform mode (honestly `simulated`).
5. Small volumes: few proposals; do not tune thresholds to manufacture activity.
6. Core quotas (10 proposals per 24 h, 20 evals per proposal) throttle live runs.
7. Free-text leakage through payload: the exporter payload treatment runs before anything lands in `product.event_log`.
8. One shared DB: a mis-granted role would cross modes; grants are generated and diff-tested.

## 12. Acceptance checks

1. DDL drift: `db/gen_ddl.py` output equals committed SQL; product columns equal the exporter allow-list (tested).
2. Evaluator tables, `pseudonym_map` and credential tables do not exist in any schema; the loader refuses them.
3. Reader roles: `default_transaction_read_only=on`, column-level SELECT only, no grant on sensitive columns (live check on a throwaway container).
4. Loader: idempotent per `_batch_id`, refuses unknown columns, no row values in output (tested offline).
5. Same sensor code, two adapters: a conformance suite runs the identical event stream through `product-sqlite`,
   `product-postgres` and `dataset-pg` and yields identical signals.
6. `data_mode` isolation: a run in `platform` mode with `raw` populated reads zero rows from `raw`; separate
   watermarks per `source_id`; a run never references two source ids.
7. Run record and console profile show `data_mode`, `data_origin`, `source_id`, adapter and the `simulated`
   release/observation double; dataset mode is never labelled production.
8. Cold start: empty product yields history-free signals only and `insufficient_history` for gated ones; no proposal.
9. Proposal kinds: only `prompt` replace and `eval_suite` draft; any other kind ends `not_evaluable`.
10. Ledger viable/non-viable rows have reasons; approval required before publish; publish targets staging only.
11. ECS-shaped local run: `/readyz` red until migrations at head; SIGTERM mid-job leaves no stuck lease.
12. No secrets, row values or E0 text in logs, receipts or Git.
