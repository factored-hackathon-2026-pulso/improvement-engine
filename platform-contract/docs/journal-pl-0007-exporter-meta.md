# PL-0007: exporter metadata discriminator (reply to CX-0122 / CL-0032)

Revision: platform-contract **1.1.0** (`catalog_version` 1.1.0, profile `platform_live.phase1`), additive over 1.0.0.
All platform-contract, platform-exporter and platform-sim files stay Claude-owned.

## What Codex consumes

| Artifact | Path (repo-relative) |
|---|---|
| Exporter metadata body schema | `platform-contract/schemas/exporter_finding.schema.json` |
| Domain row body schema | `platform-contract/schemas/domain_event.schema.json` |
| Observation view (identity + nullable sequence + discriminated `source_event`) | `platform-contract/schemas/source_observation.schema.json` |
| Catalog (kinds, finding codes, legacy rule) | `platform-contract/event-catalog.json` (`source_event_kinds`, `exporter_finding`, `legacy_exporter_prefix`) |
| Golden fixtures, valid | `platform-contract/examples/source_events/valid/` : `exporter_finding`, `exporter_finding_null_sequence`, `exporter_finding_late_sequence`, `exporter_finding_dedup`, `domain_event` (`*.valid.json`) |
| Golden fixtures, malformed/unsupported | `platform-contract/examples/source_events/invalid/*.json` (10 cases) |
| Reference checks | `platform_contract.conformance`: `classify_source_event`, `validate_source_observations`, `dedup_observations`, `check_observation_sequences` |

## Rules

- Discriminator: `source_event.kind` is `exporter_finding` or `domain_event`. Wire `kind` stays `platform_event`
  (pulso-observations-2 is unchanged). A `domain_event` `event_type` may not start with `exporter.`.
- `exporter_finding` body: `finding_code` (closed list), `severity` (info|warning|error), nullable
  `described_native_event_id` / `described_source_sequence`, inline `details` (<= 64 keys, <= 32 KiB serialized).
  `source_id`, `native_event_id` (`finding:|profile:|dimensions:` prefix), `observed_at` and `source_sequence` stay on
  the observation envelope, outside the digest, so re-emission (rescan, re-POST) is idempotent.
- `source_sequence` of a finding is null (gap/turn findings) or equals `described_source_sequence` (a finding about a
  late row reuses that row's sequence). Findings never count for continuity and never fill a hole.
- Dedup key `(tenant_id, source_id, native_event_id)`; same identity with different content is `identity_conflict`.
- Classification for consumers: `exporter_finding`, `domain_event`, `legacy_exporter_prefix` / `legacy_domain_event`
  (1.0.0 shape, no `kind`), else `unsupported` (quarantine with an explicit reason).

## Exporter

`ExporterConfig.legacy_prefix` defaults to **False**: the exporter now emits 1.1.0 (profile and dimension snapshot
become `finding_code` `capability_profile` / `dimension_snapshot`, `details` holds the former payload). The 1.0.0
`exporter.` prefix shape remains available with `legacy_prefix=True` for a consumer that has not adopted the
discriminator; it is the documented interim for 1.0.0 and is rejected by the 1.1.0 schemas. Risk: until the Codex
adapter reads `source_event.kind`, run the exporter with `legacy_prefix=True`.

## Still open (unchanged, see journal-pl-0004-review)

Backfill-state loss, permanently open missing sequences and sequence zero remain source-completeness gaps.

## Evidence

First RED: contract 15 failed (no 1.1.0), exporter 7 of 8 new tests failed. Final: platform-contract 55, platform-sim
plive 20, platform-exporter 61 (all incl. PG16 on Podman pulso-dev, removed) = 136 passed; ruff clean.
