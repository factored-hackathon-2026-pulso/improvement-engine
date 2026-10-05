# platform-contract

Contract pack for the `platform_live` source profile (TECH_SPEC_PULSO_AUTOMEJORA_V3 section 32,
package PL-L3): JSON Schemas for the allow-listed tables of the real support platform, the event
type catalog, golden examples and a conformance suite with drift checks. Codex's PL-C1 adapter
and the exporter (PL-L1) test against these files.

## Version stamp

| Field | Value |
|---|---|
| Contract version | 1.3.0 (profile `platform_live.phase1`); additive over 1.2.0, 1.1.0 and 1.0.0 |
| Source | support-platform CODE at `eeb73a8` (`tables.py`, `domain/cases/values.py`, `application/audit/catalog.py`, domain events, `backend/openapi.json`); the Product team's data-model artifact `BWx4saeWfYsLbQEbkNKMPg` ("Modelo de datos · Plataforma CC") is the 1.0.0/1.1.0 origin and is stale |
| Platform commit | `5261ecf` (main, 2026-10-05, after PRs #27/#28; 1.3.0). 1.2.0 was derived from `eeb73a8`. Previous row: `eeb73a8` (main, merge of PR #10, 2026-10-04 15:33 UTC-5) |
| Captured | 2026-10-04 |
| Previous platform commit | `a492bfa` (slices 0-3, cited by 1.0.0/1.1.0): **unreachable**. The platform history was rewritten on 2026-10-04 when it was split out of the data repo (`git cat-file -t a492bfa` fails, the GitHub API answers 422). `7d2ae3a` "Plataforma S3: supervision" (2026-10-03) is the closest old commit by slice naming: **an inference of the engine team, not a statement of the product team**. |

Digest pin: `scripts/contracts/pinned_digests.json` records the digest of the platform schema sources at `eeb73a8`;
`python scripts/contracts/drift_digest.py --platform <checkout>` fails when they change.

The artifact is third-party data. Anything it does not state (payload shapes per event type,
sequence contiguity after rollbacks, meaning of `ingested_at`) is an open question for Product
(see PLATFORM_DATA_MODEL_IMPACT_CLAUDE.md section 7) and is NOT invented here: `event_log.payload`
is only constrained to be a JSON object.

## Layout

- `platform_contract/model.py`: single source of truth (tables, enums, event catalog, denylist).
- `schemas/*.schema.json`, `event-catalog.json`: generated from the model
  (`python -m platform_contract` regenerates; a drift test fails if files differ).
- `examples/*.valid.json`, `examples/invalid/*.json`: golden vectors.
- `platform_contract/conformance.py`: row validation, `check_event_stream` (gap_suspected,
  unknown/planned event type, late event), `check_turn_sequences`, `check_database` (SQLite).
- `tests/`: contract tests (run from repo root, see below).

## Allow-list and denylist (spec 32.2.2)

Allowed: `cases`, `turns`, `assignments`, `customer_case_slots`, `customers`, `event_log`,
`staff_dimensions` (projection of `staff`: id, roles, languages, team/team_id, active, version).

Never read: tables `login_accounts`, `mfa_challenges`, `staff_sessions` (no schema exists; loading
raises `TableRefused` before any query); columns `staff.name|email`, `customers.display_name|suggestions`,
`cases.search_text|last_message_preview|last_turn_preview`. Any table not allow-listed (including the
announced `teams`, `admin_roster` and `analyst_availability`) is refused by default. Text columns
(`turns.text`, `cases.close_note`) carry `x-sensitive-text` and need the pre-ModelPort treatment.

## Event catalog

Admitted (1.1.0): `case.*` (opened, queued, assigned, status_changed, read, first_responded, closed, viewed),
`turn.created`, `staff.availability_changed`, and `auth.login_failed|account_locked|session_started|session_ended`.
Denied (known, never ingested): `auth.password_accepted`, `auth.mfa_challenge_issued`, `auth.mfa_failed`,
`customer.session_started`. Planned (announced prefixes `staff.`, `team.`) and unknown types are counted and
quarantined with a quality finding; they never fail the batch.

## Changelog

### 1.3.0 (2026-10-05, platform `5261ecf`), additive (lane SIG1)

- Event catalog 1.3.0 admits 12 more types (53 -> 65 admitted): `copilot.suggestion_requested|ready|none|failed|decided`,
  `copilot.tool_used` (data class `copilot`), `case.type_changed` (`operational`), `ai.stage_advanced|stage_moved_back|agent_ready|agent_activated`
  and `platform.ai_toggled` (new data class `maturity`). "ignored" and "shown" are NOT event types: `ignored` is a `decision` value of
  `copilot.suggestion_decided` (used|edited|discarded|ignored|accepted); the platform emits no "shown" event.
- Payload keys are ids, enums, counters and flags (see `EVENT_PAYLOAD_KEYS`); no new type has a free-text key. `assistant.turn_answered`
  gains `release`; `copilot.suggestion_ready|none|decided` carry `release`; `decided` carries `turn_id` (join with `turn.created`) and `agent`.
  Rows written before the platform carried `release`/`turn_id` lack the key: treat as null, never as drift.
- `EVENT_PAYLOAD_ENUMS` (new, catalog field `payload_enums`): closed value sets. `copilot.suggestion_decided.subject` (reply|escalation) looks like
  free text by name (the exporter redacts any `subject`), so it is forwarded only while its value is in the set.
- `cases.case_type` (optional, closed enum `none|unrecognized_charge|undue_charge|app_issue|branch_service|service_quality|virtual_card`;
  `virtual_card` is team-generated on the platform). It is the slicing dimension for per-case-type acceptance and maturity.
- Unchanged: denied tables/columns/events, administration types `staff.*`/`team.*` stay planned. Tables `copilot_suggestions`,
  `case_type_maturity`, `platform_settings`, `staff_preferences` are not allow-listed (their texts are purged at 24 h; the events carry what is needed).
  `turns.staff_line` (facts of a staff-only line) is known-unreadable.
- Privacy: this contract admits event-level ids/enums; the k >= 10 rule is applied downstream by the aggregators and by the platform
  evidence route (`CC_EVIDENCE_MIN_CELL`). Nothing here widens what leaves the platform database role.
- Digest re-pin: `scripts/contracts/pinned_digests.json` platform pin moved from `eeb73a8` to `5261ecf` DELIBERATELY after reading the diff of the six
  schema-source files (openapi.json, values.py, tables.py, audit catalog, cases/events.py, ai/events.py): the changes are the ones listed above plus
  admin/builder routes and audit catalog entries not read by the engine.

### 1.2.0 (2026-10-04, platform `eeb73a8`), additive

- Enums extended (1.1.0 values stay valid): `cases.status` + `with_assistant`; `cases.channel` + `chat_app`, `chat_web`,
  `phone_inbound`, `phone_outbound`, `email` (the platform renamed the chat channels; `app_chat`/`web_chat` stay readable);
  `cases.priority` + `none`, `critical`; `turns.kind` + `transcript`, `note`, `email`; `turns.author_role`, every
  `*_author_role`/`*_by_role`/`actor_role` + `assistant`; `assignments.reason` + `outbound_call`, `assistant_handoff`.
- New optional `cases` columns: `rating_score` (1-4), `rated_at`, `open_escalation_id`, `active_call_id`.
  Never read, denied: `cases.rating_comment`, `cases.rating_key`, `turns.subject`.
- Event catalog 1.2.0 admits 39 types, each with a data class (`operational`, `assistant`, `copilot`, `builder`), the
  payload keys the platform emits and the keys that carry free text (`free_text_keys`, never forwarded):
  `case.priority_changed|rated|assistant_started|assistant_released`, `assistant.*` (6), `copilot.*` (2),
  `builder.*` (14), `escalation.*` (7), `call.*` (6). AI payloads carry ids, enums and counters only (platform
  `domain/ai/events.py`); free text exists only in `case.rated.comment`, `assistant.input_queued.answer`,
  `escalation.opened.motive`, `escalation.answered.note`, `call.started.reason` (plus the 1.1.0 `case.closed.note`,
  `turn.created.text|subject`).
- Unchanged: denied tables (`login_accounts`, `mfa_challenges`, `staff_sessions`), denied columns, denied events
  (`auth.password_accepted`, `auth.mfa_*`, `customer.session_started`). Administration types (`staff.created`,
  `staff.invitation_*`, `team.*`, ...) stay `planned` (quarantined): their payloads carry names and e-mails.
  Tables `assistant_sessions`, `copilot_threads`, `builder_*`, `escalations`, `calls`, `notifications`,
  `bank_customer_links` stay refused (not allow-listed).
- Platform `DATA_MODEL.md` is stale (says slices 0-12, three author roles); code and `backend/openapi.json` are truth.

### 1.1.0 (2026-10-03, platform `a492bfa`, unreachable)

Exporter metadata discriminator (below).

## Revision 1.1.0: exporter metadata discriminator

`source_event.kind` separates exporter metadata from domain rows:

- `exporter_finding`: `schemas/exporter_finding.schema.json`. Body: `kind`, `source_namespace`, `catalog_version`,
  `tenant_id`, `finding_code` (closed list), `severity` (info|warning|error), `described_native_event_id` and
  `described_source_sequence` (nullable), `details` (inline, <= 32 KiB serialized, <= 64 keys). Observation envelope
  (outside the digest, so re-emission stays idempotent): `source_id`, `native_event_id` (identity, must be
  `finding:|profile:|dimensions:` prefixed), `observed_at` (wall clock), `source_sequence` (null, or equal to
  `described_source_sequence`: a finding about a late row reuses that row's sequence). Never a domain row, never part
  of source-sequence continuity or population/window projections. Dedup key `(tenant_id, source_id, native_event_id)`.
- `domain_event`: `schemas/domain_event.schema.json`; `event_type` must not start with `exporter.`.
- `schemas/source_observation.schema.json`: self-contained observation view (identity + nullable sequence +
  discriminated `source_event`). Wire `kind` stays `platform_event` (pulso-observations-2 is unchanged).
- Golden fixtures: `examples/source_events/valid/*.json` (normal finding, null sequence, late-event sequence reuse,
  dedup, domain row) and `examples/source_events/invalid/*.json` (unsupported code/kind, missing field, extra field,
  prefix misuse, identity/tenant mismatch, legacy shape, domain without sequence).
- Interim rule: for contract 1.0.0 the `exporter.` event_type prefix identifies exporter metadata. It remains valid
  for 1.0.0 and for exporters run with `legacy_prefix=True`; it is rejected by the 1.1.0 schemas.

## Run

From the repository root:

    uv run --python 3.12 --no-project --with pytest --with jsonschema python -m pytest platform-contract/tests
    uvx ruff --isolated check --select E4,E7,E9,F platform-contract
