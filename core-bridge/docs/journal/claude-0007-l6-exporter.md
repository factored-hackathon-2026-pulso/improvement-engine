# Journal claude-0007: L6 exporter

Contract revision: `pulso-two-teams-1`, wire `pulso-observations-2`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3`. Package:
L6 (plan 17.3.6). Code: `src/pulso_core_runtime/exporter/`, ingest double `platform-sim/ingest_fixture/`, tests
`core-bridge/tests/l6`. Commits: 8a820b1, 784f220 (review fixes), b3af773/4984d92 (token and final review touches). Flow:
`docs/flows/core-exporter-cursor-cas.md`. Documentation pass at HEAD `4984d92`.

## Purpose
Export Core's audit chain, registry events and outbox to the Pulso control API as verified, resumable batches, without
writing to Core and without trusting the network.

## Flow
poll (audit, registry, outbox) -> build batch -> persist pending -> POST -> ACK -> one-transaction commit of the delta and
cursor; rescan (anti-entropy plus open-run prefixes), sweep (24 h full chain check). Details and the CAS rules are in the
flow document.

## Input / output
In: read-only Core snapshot (`audit_events`, `reg_events`, `outbox`), Pulso cursor endpoint. Out: `pulso-observations-2`
batches (<= 500 events, 512 KiB), verification receipts, exact-bytes NDJSON chain artifacts (<= 1 MiB each, manifest for
larger chains), `PollReport` and `coverage()`.

## Transactions and idempotency
`Idempotency-Key = batch_digest = sha256(JCS(body))`. Local SQLite WAL (`synchronous=FULL`) with tables `run_head`,
`ledger`, `partition_cursor`, `hole_ranges`, `partition_status`, `pending_batch` (single row), `quarantine`, `meta`. The
pending batch is stored before the POST and re-POSTed unchanged; `commit_ack` is a single `BEGIN IMMEDIATE` transaction.

## Errors
Typed: `pulso:eval_db_misconfigured`, `pulso:exporter_overprivileged`, `pulso:schema_drift`. Partition stop reasons:
`ack_digest_mismatch`, `digest_conflict:<err>`, `quarantined:<err>`, `http_<code>`, `batch_too_large`,
`schema_invalid:<table>`, `source_conflict:<run>:<seq>`, `run_chain_broken:<run>:<seq>`.

## Permissions
`exporter_ro` DSN only; tokens per route class with one key and `kid` per audience (ADR 0005); no Core DSN reaches Rust
services; `platform-exporter` receives only `CORE_EXPORT_DATABASE_URL`, `PULSO_CONTROL_API_URL`, `PULSO_SERVICE_KEY_REF`,
`PULSO_TENANT_ID` (per the L8 compose fragment).

## Config
`ExporterConfig` (tenant, instance, expected runtime/eval DB names, binding ref, optional schema refs, verifier sha and
contract version, batch limits, `gap_grace_seconds` 120, `receipt_every` 200, `max_retries` 5, backoff cap 60 s); schedule
poll 5 s, rescan 900 s, sweep 24 h.

## Observability
Closed-code `LoopStats.errors`, `PollReport` (batches, events, duplicate ACKs, partial reasons, stopped partitions,
deferred), coverage reasons such as `registry_seq_gap_unverified`, `exporter_rebuild`, `open_runs:<n>`.

## Commands (head `4984d92`)
`python -m pytest -c pyproject.toml tests/l6 -p no:cacheprovider` with `PULSO_TEST_PG_ADMIN` (real PG16 for Core; the ingest
endpoint is a double); run: `python -m pulso_core_runtime.exporter`. Environment as in journal claude-0002.

## RED / GREEN
First RED (`tests/l6/test_first_red.py`): a crash after the ACK and before the cursor commit must not double count (re-POST
of the persisted pending batch returns 200 duplicate); also duplicate, gap and late-event cases. GREEN as reported: 23 then
49 tests on real PG16 after the adversarial review (ArtifactRef objects, pending-batch overwrite, per-attempt route-scoped
JWT, unverified rescan rows, row-column vs signed event, grant checks, hole ranges, digest-in-size, late receipts,
open-run prefix re-export, `sweep`, `__main__`, report, HTTP + JWT tests). About 44 test functions exist. Not re-run here.

## Trade-offs
Exact-bytes NDJSON (no re-serialisation) makes verification independent of JSON libraries at the cost of storing the
stored `event_json` verbatim and failing on multi-line serialisations. Holding back sparse gaps for 120 s trades latency for
fewer late rows.

## Gaps
- `cut_ref` always null; real control-api not exercised (fixture only).
- The `l6` pytest marker is registered in `tests/l6/conftest.py` (not in `pyproject.toml`, which L3a owns), so it is
  known only when `tests/l6` is collected (`--strict-markers` is on).
- Sweep startup from the image entrypoint is unverified (see flow document gaps).
