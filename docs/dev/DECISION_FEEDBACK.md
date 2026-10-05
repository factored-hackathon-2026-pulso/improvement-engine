# Decision feedback (FDBK1): what happens to a proposal after delivery

Question from the user: can the system learn from "a proposal was not approved because of X"? Today the engine never
reads what happened to its proposals (plan section 18, W2-1 outcome loop and wired MEM1). This note records the
FACTS (verified in code at agent-core `origin/main` 630a4a7 and support-platform `origin/main` a451001, and live on an
own local stack), the GAPS, the smallest ASKS, and the engine side that is built here (`scripts/feedback/`).

## 1. Facts: what is recorded when a human decides

References are `path:line` at the commits above. agent-core is `AC`, support-platform is `PL`.

| Decision | What agent-core records | Where | What the platform records |
|---|---|---|---|
| approve | `Approval(decision="approved", actor, candidate_hash, yardstick_loosened, at)`; NO reason field is ever set | `AC agent_core/registry/service.py:597-620`, model `models.py:126-133`, table `reg_approvals` `postgres/schema.sql:25-27` | audit event `builder.proposal_approved` with counts only (`PL backend/src/cc_platform/application/ai/builder.py:637-665`) |
| reject | `Approval(decision="rejected", reason=reason[:2000])` and the proposal goes back to `draft`, `candidate_hash=None`, `rev+1`. There is NO `rejected` proposal state | `AC service.py:622-631`; `ProposalState` has only draft, candidate, evaluated, approved, published `models.py:40-45` | audit event `builder.proposal_rejected` with `reason_length` only, NOT the text (`PL builder.py:669-692`, `domain/ai/events.py:293-298`) |
| reject, input | free text is MANDATORY: `ReasonBody.reason: str` (`AC http.py:92-93`, route `http.py:244-246`); platform `RejectRequest.reason` 1..2000 chars, stripped (`PL api/schemas/builder.py:368-370`); the SPA shows a required free textarea (`PL frontend/src/features/automation/components/ProposalNextStep.tsx:300-308`) | | |
| expire | NOTHING. No TTL, no `expired` state, no event. A proposal nobody decides stays `draft/candidate/evaluated` forever | grep of `expire/ttl` in `AC agent_core/registry/service.py`, `models.py`: no match | the platform index only caches title/state (`PL builder.py:236-258`); no expiry either |
| reopen (undo approve) | event `reopened`, state back to `draft` | `AC service.py:430` | `builder.proposal_reopened` |
| stale (base alias moved) | event `proposal_staled`, state back to `draft` | `AC service.py:650` | |
| publish | event `published` with `release_id`, `agent_id`, `alias` | `AC service.py:692` | `builder.proposal_published` |

What an external reader can SEE over HTTP:

- `GET /v1/export/registry-events` (role exporter, `AC agent_core/composition/export_http.py:75-84`): events
  `proposal_created, draft_updated, frozen, evaluated, approved, rejected, reopened, proposal_staled, published,
  promoted, revoked`. Fields (`AC models.py:157-171`): `type, actor, principal_type, origin, proposal_id,
  candidate_hash, release_id, agent_id, alias, before, at`. NO reason on `rejected`. Events are stored as JSON
  (`reg_events.event_json`, `schema.sql:33`), so a new optional field needs no migration.
- `GET /v1/registry/proposals` (`AC http.py:190-204`, `service.py:470-483`): `agent_id, state, created_by, limit<=200,
  offset`, newest first by `updated_at`. NO `origin` filter and NO `created_at`.
- `GET /v1/registry/proposals/{pid}` (`AC http.py:206-209`, `service.py:434-444`): `proposal, changes, last_eval,
  review`. `changes[].docs.{description, rationale, changelog}` carry our header. NO approvals list.
- The reject reason is WRITE-ONLY over HTTP: `latest_approval` is used only inside publish/import
  (`AC service.py:665,839`). Nobody outside the database can read it, and the platform audit drops it.

Verified live (own stack `pulso-fdbk1`, agent-core 630a4a7, admin stand-in): a `rejected` event has the keys
`actor, agent_id, alias, at, before, candidate_hash, origin, principal_type, proposal_id, release_id, type` and
nothing else; after the reject the detail shows `state: draft`; the reason text (a synthetic phone-number canary) appears
nowhere in the detail JSON.

## 2. Gaps

1. No structured rejection reason. The only reason is a mandatory free text that is not readable over HTTP.
2. Free text may contain PII (an approver types a customer name or a phone). It is untrusted input by construction.
3. A rejection is not a state: after reject the proposal is `draft` again; the only trace is the `rejected` event.
4. No expiry. "Nobody decided" is invisible; the engine derives it (idle for `--expire-days`, default 14).
5. `GET /proposals` cannot filter by origin and has no creation time: the reader uses events for the lifecycle and
   `created_by` plus `origin` from the detail.
6. Our dossier does not yet carry a machine-readable finding key. The reader parses a header tag
   `[finding: <key>]` at the start of `rationale` (then `changelog`) and falls back to the metric id in the title
   (`<target> - M4: ...`). The dossier owner must emit the tag (additive; one line in `dossier.rs`).

## 3. Asks (smallest additive change; text in `docs/reports-claude/ASKS/ASK_decision_reason.md`, Spanish)

Closed vocabulary `reason_code` (7 values): `insufficient_evidence, wrong_target, risk, duplicate, policy_conflict,
wording, other`.

- agent-core (3 additive lines of schema): `ReasonBody` gets optional `reason_code` (enum) next to `reason`; `Approval`
  gets optional `reason_code`; the `rejected` registry event gets optional `reason_code` (NOT the free text). Old
  clients that send only `reason` keep working (reason_code = null, reader maps to `unknown`). Optionally the free
  text becomes `note` limited to 280 chars. `GET /proposals/{pid}` could add `last_decision {decision, reason_code,
  at}`.
- platform: `RejectRequest` gets `reasonCode` (select in the existing reject dialog, required on the new UI), forwarded
  to agent-core; `BuilderProposalRejected` audit gets `reason_code`. The free textarea stays optional and bounded.
- No change is asked for expiry: derived on our side.

Until the ask lands every rejection reads as `unknown` (see live results below) and the suppression rule does nothing
by design (see 5b).

## 4. Engine side (this branch)

`scripts/feedback/decision_feedback.py` (stdlib only; tests `scripts/feedback/test_decision_feedback.py`, 36 cases):

- Reader: pull-only, persisted cursor (`registry_after` = events already read, like `agentcore_poller.py`), loopback
  guard, tokens from the environment or a local JSON file (never printed, stored or logged), at-least-once with a
  `record_key` dedupe in the JSONL sink and in the state file. The cursor and the per-proposal event ledger are
  committed together only after a whole poll succeeded.
- Scope: events with `origin == auto_detect` (`--origin` to change) and, if given, `created_by == --created-by` (the
  engine principal). Detail is fetched once per proposal and again only when a new event touches it.
- Record `pulso.decision/1` (one JSONL line per state change of a proposal; consumers take the LAST line per
  `proposal_id`): `proposal_id, agent_id, origin, finding_key, family, target ("kind:id" of the first
  non-suite change), kind, n_changes, state, created_at, decision_at, time_to_decision_secs, reason {code, source,
  note_len}, observed_at, record_key`.
  - state: `draft, candidate, evaluated, approved, rejected, published, expired`. `rejected` is sticky while the
    proposal is reworked and until it is frozen again; `expired` is derived (undecided and idle longer than
    `--expire-days`).
  - time to decision: first `approved` or `rejected` event minus `proposal_created`.
  - reason: `reason_code` (structured, source `structured`) when the event carries it; otherwise free text if present
    is mapped by a conservative keyword table (es/pt/en) to the vocabulary (source `mapped`, two different matches
    give `other`), else `unknown`. The text itself is dropped at ingestion: only `note_len` survives. Verified: a
    phone-number canary typed in a live reject appears in neither the JSONL nor the state file.
- Safety rule: reason text never goes into a model prompt. Only closed-vocabulary counts leave this module.

### Uses

(a) Dossier history line. `history(records, target, family)` and `history_line(h, "es"|"pt"|"en")`: "Historial: N
aprobadas y M rechazadas entre propuestas similares (motivo mas frecuente: R)". Lookup: same target AND same
finding family first; if none, same family on other targets (labelled). Counts only decided proposals; `unknown`
reasons are not listed. Integration point: `seams/crates/reasoning/src/dossier.rs` is NOT on `origin/main` (it lives on
`claude/w12-dossier`), so nothing is wired here and no branch is merged. When it lands: the caller of `dossier::build`
runs `decision_feedback.py history --in decisions.jsonl --target <target_ref> --family <metric> --lang es` and passes
the line as one more `Labels` entry; `build` appends it to the "Evidencia y comparacion" section. The line contains no
digits beyond the two counts and no free text, so it passes the SPA PII regex hazards in `SPA_S22_ALIGNMENT`.

(b) Suppression rule. `suppress(records, target, family, now, cooldown_days=30)`: when the latest decision on the same
target and family is a rejection whose STRUCTURED reason is in `{wrong_target, duplicate, policy_conflict, risk}` and
`now < decision_at + cooldown`, the engine must not re-propose (output `{suppress: true, reason, proposal_id, until}`).
`insufficient_evidence, wording, other, unknown` never suppress (new evidence or better wording may succeed). A later
approval for the same target and family lifts it. Reasons derived from free text (`mapped`) do not suppress unless
`--allow-mapped` is passed: a keyword guess must not silence the engine. Integration point: the proposer's announce
decision, before creating the registry proposal (the same place the dossier `announce` flag is computed).

(c) Aggregate metrics `pulso.decision_metrics/1` for the o11y/headline strip: `made, approved (approved+published),
rejected, expired, pending, approval_rate (approved over approved+rejected), median_time_to_decision_secs, reasons
histogram, by_kind`. CLI: `decision_feedback.py metrics --in decisions.jsonl`.

## 5. Live results (own stack, local test data only)

Stack: `PULSO_STACK_PREFIX=pulso-fdbk1 python scripts/feedback/live_stack.py up` (containers `pulso-fdbk1-postgres`
:55472 and `pulso-fdbk1-llm-gateway` :8120, agent-core :8012 from a worktree of agent-core `origin/main` 630a4a7,
credentials only in child-process environments). `live_drive.py` created five MANUAL-origin proposals on `disputas`
(a one-sentence `prompt` change plus a 2-scenario synthetic suite), real evaluate (verdict pass x3), then: A approved,
B rejected (free text with a duplicate hint and a phone canary), C rejected (free text "riesgo"), D frozen, E draft.

`decision_feedback.py poll --once --origin manual` produced 5 records and, on replay, 0 new lines (cursor
`registry_after=24`). States: approved (11 s), rejected (4 s), rejected (5 s), candidate, draft; finding families M4
and M2 read from the `[finding: ...]` header; all reasons `unknown` because agent-core does not expose them (gap 1).
Metrics: made 5, approved 1, rejected 2, expired 0, pending 2, approval_rate 0.33, median time to decision 5 s,
by_kind prompt 4 / unknown 1. With `--expire-days 0` the same events yield expired 2 (the two undecided).
What-if on a copy where the two rejections carry structured reasons (duplicate, insufficient_evidence): `suppress`
returns true for M4 (duplicate, until +30 days) and false for M2 (insufficient_evidence); the history line reads
"1 aprobadas y 1 rechazadas ... (motivo mas frecuente: duplicate)".

## 6. Run it

    python -m unittest discover -s scripts/feedback -p "test_*.py"
    python scripts/feedback/decision_feedback.py poll --once --origin auto_detect --created-by <engine-principal> \
        --core-url http://127.0.0.1:8001 --state decisions-state.json --out decisions.jsonl \
        --registry-token-env AGENTCORE_BUILDER_TOKEN --export-token-env AGENTCORE_EXPORT_TOKEN
    python scripts/feedback/decision_feedback.py metrics --in decisions.jsonl
    python scripts/feedback/decision_feedback.py suppress --in decisions.jsonl --target prompt:p/x --family M4
    python scripts/feedback/decision_feedback.py history --in decisions.jsonl --target prompt:p/x --family M4 --lang es

Limits: only the reader and the pure functions exist; nothing calls them from the engine yet (the dossier crate and the
announce path are on other branches). The keyword mapping is a stopgap that disappears once `reason_code` exists.
Records are local files; there is no durable store or memory (MEM1) write yet.
