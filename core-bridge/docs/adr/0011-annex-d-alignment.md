# ADR 0011: Annex D / V3 31.5 alignment of the core-bridge runtime

Status: accepted (claude/r3-annexd). Contract revision stays `pulso-two-teams-1` (A02: replies and journal entries do
not bump the wire); every change below is either additive/optional or tightens a field the annex already names.

## Context
The contract pack (`bridge-contract/`) showed the runtime diverging from annex D (D.1-D.4) and the accepted A02/A03/A04
replacement texts. Codex's Rust client codes against the annex, so the runtime moves to it; where the annex is
ambiguous or the runtime is safer, the runtime is kept and listed below.

## Decisions (changed)
1. **Class (i) tokens (A03 i).** `sub` must be `worker:<non-empty id>` on every `aud=core-bridge` route (403
   `pulso:auth_denied`, `details.reason=sub_not_worker`, checked before the jti is consumed). On `POST
   /core-tasks/invoke` the `job_id` claim must equal the body `job_id` (403 `auth_denied`, reason `job_mismatch`; a
   missing claim is a mismatch). Admissions keep "job_id claim required" (their body has no job_id; the claim is the job).
2. **Idempotency-Key.** Invoke: unchanged (required, formula, 422 on mismatch). Arms: the header is accepted; when both
   header and body `idempotency_key` are present they must be equal (422 `invalid_request`,
   `details.fields=["Idempotency-Key"]`); body-only keeps working (back-compat); neither is 422. Admissions: the header
   is accepted and validated against `[A-Za-z0-9_.:-]{1,200}`; the admission is identified by the derived ref (item 4), so
   the header does not change identity. Annex D.4 gives admissions no body key, so there is no "body key" to compare.
3. **ArmRequest names.** Annex names `execution_profile` (`attention_stateful_complementary` | `evolution_task`),
   `sandbox_session_ref`, `deadline`. Deprecated aliases accepted for ONE release, then removed: `mode`
   (`stateful_attention` = `attention_stateful_complementary`, `native` = `evolution_task`; `task_builder` has no annex
   profile and stays alias-only), `seed_manifest_ref` (= `sandbox_session_ref`), `agent_id` (annex has none: derived from
   the target's single Agent entity; if given it must match, else `failed_infra` / `target_preparation_failed` /
   `detail=target_agent_mismatch`). An alias that contradicts its annex name is 422. The single-flight digest is computed
   over the normalised request, so alias and annex spellings of one request replay one report. Normalisation always
   fills `agent_id` BEFORE hashing (given value, else the target's derived agent; read-only target load, unresolvable
   -> hashed as given and the run reports the failure), so omitting `agent_id` and sending the derived value are the
   same request in both directions. `deadline` is NOT part of the digest: it is a per-attempt bound that a Rust retry
   recomputes, so a retry of the same logical arm with a later (or absent) deadline replays the stored report, never
   `409 idempotency_conflict`.
4. **Admission `evaluation_context_ref` is server-derived.** `ref = "evc-" + sha256_hex(tenant|job_id|binding_ref|
   proposal_id|candidate_hash|evaluation_attempt)[:40]` (`job_id` = token claim). The response returns it; Codex puts it
   in the writer commitment. An explicit body value is accepted only if equal to the derived one, otherwise 422
   `pulso:evaluation_context_invalid`. The request digest is not part of the derivation, so a replay with another digest
   is `409 idempotency_conflict`. None of `tenant`, `job_id`, `binding_ref`, `proposal_id`, `candidate_hash`,
   `evaluation_attempt` may contain `|` (422): the separator would make `("a|b","c")` and `("a","b|c")` collide.
   The formula is the contract: Codex's Rust client MUST copy it exactly (prefix `evc-`, `|` separator, field order,
   lowercase hex, first 40 characters) or its writer commitment will not match the admission. ADR 0007's charset/never-truncate rules still hold for the derived value.
5. **CoreVersion** adds `schema_version:"1"` and `bridge_instance_id` (`PULSO_BRIDGE_INSTANCE`, default `bridge-1`).
   The credential request stays the closed `{tenant_id, role, purpose}` (V3 31.5.10 shape, extra=forbid, non-empty
   strings) and additionally tolerates `schema_version:"1"` (annex D.1 spelling); other values are 422.
6. **Formats.** `deadline` (admissions, arms, invoke) and invoke `cutoff` are UTC RFC3339 with a literal `Z`
   (`2030-01-01T00:00:00Z`, optional fraction); offsets/naive are 422. Admission and invoke `request_digest` must be 64
   lowercase hex characters.

## Decisions (runtime kept)
- **No new required request field.** `deadline` on arms stays optional (annex lists it; making it required would break
  every recorded client); when present it is format-checked only (Z-RFC3339).
  Annex D.4 defines no deadline-expiry error for arms, so a past deadline is NOT rejected (no `pulso:deadline_expired`);
  admissions keep their existing deadline rule. Enforcement stays with the budget `deadline` of the resolved
  `budget_ref`. `execution_profile` is required unless the
  deprecated `mode` is sent.
- **`sandbox_session_ref` semantics.** The annex lists it in ArmRequest but also has the bank open sessions from a
  `seed_manifest_ref`; the runtime opens bank sessions itself, so it treats `sandbox_session_ref` as the value it hands to
  the bank port (the old seed manifest slot). `evolution_task` with a non-null ref stays `409 mixed_world_rejected`.
- **`request_digest` equality is not verified on admissions** (invoke does verify it). Only the format is enforced; the
  digest is stored and compared on replay.
- **Other class (i) claims** (`job_id` on read-only routes, `/version`) stay optional; `/version` has no tenant claim.
- **Credential response** stays `{jws, kid, exp}` (no `schema_version`): it mirrors Core's principal issue shape.
- `POST /evaluation/arms/{arm_id}/run` (not in the annex) is kept as a back-compat alias of `/arms/run`.
- Error codes are unchanged; new `details.reason` values are `sub_not_worker`, `job_mismatch`.

## Consequences
Rust must send `sub=worker:<id>`, the invoke `job_id` equal to its claim, Z-suffixed timestamps and lowercase-hex digests,
read the admission ref from the 201/200 body (or derive it), and use the annex ArmRequest names. Aliases disappear in the
next contract revision. `bridge-contract` artifacts were regenerated with `gen.py` (also picks up the 894fa65 pin) and
the goldens re-recorded on the real runtime over PG16.
