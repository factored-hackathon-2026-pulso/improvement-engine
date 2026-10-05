# registry-writer (B2)

Delivers a compiled proposal of `crates/reasoning` (anchored patch or new-agent closure) to the REAL agent-core. The engine
proposes; agent-core manages (evaluation, approval, publish). Nothing here approves, publishes, promotes, freezes or evaluates:
`guard::allowed` is a closed allow-list checked before every request.

## Two delivery paths (`Via`)

| Path | Requests | Who authors the draft | Credential |
|---|---|---|---|
| `RegistryApi` | `GET /v1/registry/entities/..` (base check), `POST /v1/registry/proposals` (`origin=auto_detect`), `PUT .../draft`, `POST .../validate`, `GET .../proposals/{id}` (read back) | the engine compiler, byte-exact | a principal the registry API accepts as `builder` |
| `BuilderRun` | `POST /v1/runs` of agent `pulso-builder` (`Idempotency-Key` = finding key), then validate and read back through the registry API | the `pulso-builder` agent (a model re-authors the draft from goal and evidence) | the engine-signed `builder` token for the run, a registry credential for the read-back |

Success is `proposal_id` + `valid` + a non-empty change list read back from the registry. Every other result is a closed
`Reason` (`Outcome::to_json().reason`): `nothing_to_propose`, `invalid_submission`, `base_changed`, `base_missing`, `unauthorized`,
`forbidden_role`, `step_up_required`, `quota_exceeded`, `validation_failed`, `draft_invalid`, `empty_draft`, `proposal_stale`,
`registry_unreachable`, `outcome_unknown`, `registry_error`, `run_not_completed`, `run_malformed`, `readback_unavailable`,
`forbidden_operation`. Details carry problem codes and rule ids only, never response free text or a token.

## Idempotency and quota

`Submission::key()` = `pulso-` + SHA-256 prefix of (finding evidence ref, target ref, kind). The registry at main has no list-proposals
route and no `Idempotency-Key` on HTTP (agent-core PRs 23/24), so a `ReceiptStore` (`MemoryStore`, `FileStore`) maps key -> proposal id:
a retry confirms the proposal in the registry and returns it (`replayed`), resumes a draft that was created but never written, and
drops a receipt the registry no longer knows. The key also travels as `Idempotency-Key` (honoured on `/v1/runs` today). The 10
proposals per 24 h quota is predicted from the receipts (`quota_exceeded`, nothing sent) and mapped from the registry's 429.
Limit: a create whose answer was lost (`outcome_unknown`) leaves an unreceipted proposal that must be looked up by hand.

## Live baseline

`Writer::refresh_catalog` replaces the `fixture-baseline` text of every artifact the registry serves
(`registry/get_entity`), relabels the catalogue `live-registry` or `mixed-live-and-fixture`, and lists what stayed fixture.
`deliver` re-reads a patch target before writing and refuses `base_changed` when its digest is not the compiled `base_digest`.

## Honest labels

Every outcome carries `labels.environment` (`local-stack: our own agent-core instance` or `shared-core`), `labels.via` and
`labels.authored_by`. Offline tests use a scripted transport; the live tests (`tests/live.rs`, `#[ignore]`) talk to the local stack.

## Commands

```
cargo test -j 1 -p registry-writer                      # offline, scripted
PULSO_DEV_STACK_DIR=<worktree>/.dev-stack cargo test -j 1 -p registry-writer --test live -- --ignored --nocapture --test-threads 1
```

The registry HTTP API of the local stack verifies only the staff keys; `scripts/dev-stack/identity.py keys` now also trusts the engine
kid there, so the engine-signed `builder` token works on `/v1/registry` after the next `stack.py up`. Until then the live test falls back
to the staff admin credential and prints that label.
