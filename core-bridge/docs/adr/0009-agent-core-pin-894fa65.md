# ADR 0009: Bump the agent-core pin 789d6c8 -> 894fa65 (PR #29)

Status: proposed (implementer: Claude; reviewer and integrator approval pending). Analysis:
`docs/AGENT_CORE_PIN_BUMP_2_ANALYSIS_CLAUDE.md`. Scope of this ADR: WP1 (pin, wire) and WP3 (runtime). The mock/parity
fixtures (WP2) and the assets (WP4) are separate packages.

## Context
`894fa65575d83420523f33ec1c6919b8965f7ebe` is PR #29 on top of 789d6c8. `contracts/VERSION` is still 1.3.0 (not a drift
signal, see ADR 0008). Route table and `openapi.json` are byte-identical. Relevant behaviour: run idempotency
reservation (concurrent same-key `start_run` -> 409 `idempotency_conflict`), `Interrupt.locked`, `Agent.input_schema`,
rate-limit env knobs (`rate_limits_from_env`), harness key `eval-<run>-<label>-<scenario>` (F-01 fixed).

## Decisions
1. **Pin and wire.** `PIN_SHA`, exporter pin, `gen-wire.ps1`/`ci.ps1`/`build-image.ps1` constants and the `gen_wire.py`
   fallback move to 894fa65. `wire/agent_core@894fa65/` replaces `wire/agent_core@789d6c8/`: 250 files, same file set,
   10 changed (`schemas/{Agent,Interrupt,Release,RunSummary}`, `registry/{ReleaseDetail,ReleaseSettings,StoredRelease}`,
   `derived/{ProposalDetail,ReleaseDetail}.schema.json`, `golden/hash_vectors.json`) plus MANIFEST.json.
   **MANIFEST digest: `ed000b815a324d3b212f2adb03458d22943d333d7a6964a5ca54b70c682809b5`** (`contracts/agent_core/pin.json`
   does not exist in this tree and is outside our paths; whoever creates it records this digest).
2. **No hard-coded release id in `gen_wire.py`.** The seeded release is `atencion:prod` (`seeded_release_id`); its id moved
   `rel-98130317a1003849` -> `rel-e26df0070f6be82f` because the seed interrupt is now `locked: true`. Tests assert the
   source carries no `rel-<16 hex>` literal. `expand_contract_worker.py` derives the id the same way.
3. **Runtime is single-pin at deploy time, bridge logic is dual-pin.** `PULSO_CORE_SHA` must equal `PIN_SHA` (unchanged
   rule), so one image runs one pin. But the new code works against BOTH Core versions (expand/contract): the in-flight
   409 branch never triggers on 789d6c8 (it never emits that 409), and `limits_from_env` falls back to
   `RateLimitConfig()` when `rate_limits_from_env` does not exist. We deliberately did NOT add 894-only symbols
   (`reserve_run_idempotency`, `RunSummary.cursor`, `Interrupt.locked`) to `compat.PIN_SYMBOLS`, so `assert_compat` passes
   on both pins; adding them is a contract step once 789d6c8 is no longer supported.
4. **409 `idempotency_conflict` has two meanings.** Core distinguishes them only by `detail`: in flight carries
   "otra peticion con esta clave sigue en curso" (we match the fragment `sigue en curso`), another-body has none.
   The invoke service resends the SAME request (same key and body, safe by Core's idempotency) with backoff
   (0.25 s doubling, cap 4 s) for up to `core_inflight_wait` = 60 s (= Core `lease_ttl`). A committed first attempt then
   returns its stored result (201); a dead one releases the reservation and the retry takes it over. If still in flight
   after the lease the receipt becomes `unknown` / `core_idempotency_in_flight` (HTTP 202), never `manual_reconcile`.
   A 409 without the marker keeps the old behaviour (`manual_reconcile`, `pulso:digest_conflict`). Relay item: ask the
   Agent Core team for a distinct problem code so the text match can go.
5. **Reconciler.** A reserved-but-uncommitted run is invisible to `get_run_idempotency` (it filters `result_json IS NOT
   NULL`). For receipts parked as `core_idempotency_in_flight` the reconciler returns `unknown` /
   `core_reservation_pending` until 75 s (lease + margin) after the park; a stored result always wins first.
6. **Rate limits.** `main.py` applies `rate_limits_from_env(env)` (invalid values exit 2 with the variable name in the
   message); the effective config is exposed as `app.state.pulso_limits` for composition tests.
7. **F-01 test inverted.** `test_stock_harness_no_longer_replays_on_a_persistent_eval_db` guards the fix. Our own harness
   stays (sealed input, budget metering, snapshot-only registry).
8. **Expand/contract evidence** (`tests/integration/test_expand_contract.py`, now OLD=789d6c8 / NEW=894fa65 on PG16):
   only `adapters/sql/schema.sql` changed (additive: `runs.change_xid`, `run_idempotency.reserved_until`, nullable
   `result_json`); the new binary migrates an old DB; the old binary runs on a DB migrated by the new one.
   **Rollback hazard (data):** releases written by 894fa65 that carry `Interrupt.locked` cannot be read by 789d6c8
   (`extra=forbid`). Our four pulso-evolution releases have no interrupts, so rollback is safe for them; the upstream demo
   seed is not.

## Consequences
- Deploy order: `agentcore migrate` with the owner role, then pods (Codex WP5).
- ADR 0004 "Core validation of run-input slots is NOT to be expected" is outdated for agents that declare the opt-in
  `Agent.input_schema`; `bind_context` stays for the binding gate (analysis section 6).
