# 0666 E8: the proof's scratch proposals no longer reach an approver (Claude)

## Problem (found live by the user)
The regression proof (W11) opens two throwaway manual-origin proposals per run on agent-core (`evaluation base ...`, `evaluation candidate-N`, creator `pulso-engine`) to freeze and evaluate the suite. The platform's Automatizacion list shows every agent-core proposal, so a supervisor saw them beside the real patch, approved and PUBLISHED the suite-only scratch, and moved staging of consultas to its release while the real patch was in prod.

## What agent-core allows the engine principal (read in a fresh clone of main)
- States: `draft -> candidate (freeze) -> evaluated (evaluate pass) -> approved -> published`. A failed evaluation sends the proposal back to `draft`; `failed_infra` leaves it `candidate`.
- `reject` needs an approver human with step-up and is only legal from `evaluated`. No withdraw, no delete, no close verb.
- `reopen` is a plain `constructor` operation (`candidate | evaluated | approved -> draft`, no human, no step-up, nothing is released).
- `evaluate` needs a frozen proposal (`candidate` state): there is no evaluation on a bare candidate hash without a proposal.
- `GET /v1/registry/proposals` filters by `agent_id`, `state`, `created_by` only (no origin filter); the platform lists everything and its state filter is optional.

## Decision
(a) is possible with `reopen`: after the proof the engine reads the scratch back (origin manual, title carries the mark, state `candidate` or `evaluated`) and reopens it to `draft`, the one state nobody can approve or publish. Never for `approved` (a human decision), never for a proposal that is not its own scratch. `reject` stays out of reach (human decision). The general allow-list still refuses `reopen`; `guard::scratch_close_allowed` admits exactly `POST /v1/registry/proposals/{id}/reopen`, called only by `Writer::close_scratch`.
(b) is added on top: title `[improvement-engine] [proof-scratch] evaluation <label> <suite> - not a deliverable, never approve or publish` and every scratch change docs description opens with `kind: proof_scratch;`.
The supervisor can still see a draft scratch, so a platform PR hides the prefix from the list (support-platform PR 35).

## Live finding
agent-core replays a KEYED `reopen` as a no-op, and a rerun of the same finding reuses the keyed scratch, so a stable close key swallowed the second close (state stayed `evaluated` while the engine believed it closed). The close carries no Idempotency-Key; the read-back state is the guard. Also: a rerun on the same keys against a scratch whose rev moved (put_draft expected_rev from the replayed create) ends `infra_failed` (pre-existing, not changed here; use a fresh salt/stack).

## Evidence
Offline: 6 new proof tests + 1 guard test (RED 5/6 before the change), `registry-writer` 95 tests green. Live on an own stack (prefix e8, ports 55491/8191/8181): proven and non-improving runs, 4 scratch, all `draft` after the proof, 0 left open, the announced `auto_detect` deliverable delivered and untouched. Stack shut down.

## Not done / asks
- agent-core: an origin or `kind` filter on `GET /proposals` and a non-approvable flag would remove the need for the title convention; not opened (the title prefix is enough for the platform PR).
- A platform-side block on approve/publish of a scratch reached by direct id (a draft is not approvable at agent-core).
- `reopen` could undo a human approval in the milliseconds between our read-back and the call (only if a human approves a scratch in that window); accepted.
