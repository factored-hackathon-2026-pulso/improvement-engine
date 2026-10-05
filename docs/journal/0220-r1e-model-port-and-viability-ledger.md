# 0220 - R1E: ModelPort, viability ledger, N-signal pipeline, RealCore readiness

Date: 2026-10-04. Team CL, lane R1E (worktree `improvement-engine-claude-w6-r1e`, branch `claude/w6-r1e`).
Plan: `docs/plan-real/r1-real-system-design.md` section 2 (proposal -> Core validate/evaluate -> ledger).

## What exists now

- `engine::models` (`ModelPort`, `Recording`, `Scripted`, `Roleplay`, `Gateway`, `tps`). One trait, one seam for scout,
  verifier and builder. Every call is recorded (`CallRecord`: role, label, model id, data class, outcome). The label
  vocabulary is `scripted | roleplay | local-model | gateway`; a step or a model record is `real` only for an ANSWERED
  `gateway` call (`CallRecord::status_provider`, tested over the whole label x outcome matrix). A refusal or an outage is
  `blocked(model_refused|model_unavailable|model_invalid)`: there is no fallback to another port.
- `Roleplay` is replay only: it reads `<queue>/responses/<key>.json`, never writes a request, and its key equals the python
  shim key (golden hashes computed with `roleplay_llm.shim.replay_key`). Payloads that would need the shim's uuid/id ordinal
  substitution are refused (`replay_key_unsupported_ids`) instead of guessed.
- `Gateway` is a plain-HTTP client of an llm-gateway-compatible `POST /v1/chat/completions` (the shim contract). Disabled
  unless `PULSO_MODEL_GATEWAY=enabled` plus `PULSO_GATEWAY_ADDR` and `PULSO_GATEWAY_MODEL` (key `PULSO_GATEWAY_KEY`, kind
  `PULSO_GATEWAY_KIND=gateway|local-model`). Before any byte is sent: E0/original data class is refused, and the payload
  must pass the Rust TPS scan. Tests run against a local fake gateway on a std `TcpListener`.
- `engine::models::tps` ports the python scanner rule by rule (differences stated in its module doc: no NFKC, stricter on
  non-ASCII numerics).
- `engine::ledger`: `viable | not_viable | not_evaluable` with a closed reason vocabulary, immutable CAS entries
  (`ledger/e<ordinal>` in any `JobStore`), conformance scenario in `engine::conformance` (FileStore here; the Pg suite runs
  it through `run_suite`), BK0 check embedded from `contracts/artifact-kinds/matrix.json` (read only, not edited).
- `thread10`: scout claim, verifier check and builder change spec are `ModelPort` calls answered BEFORE the job and
  persisted as `model/<role>` in the job store (a resume never asks a model twice; an outage is not persisted).
  `Opts::{model, seed, job, core}`; reports carry `models[]`, step statuses and `ports[].llm_gateway` derived from the records.
- `thread10::pipeline::run_signals`: N signals -> N proposals -> N ledger verdicts -> N `proposal_verdict` run events
  (`NewEvent{kind: proposal_verdict, entity_kind: proposal, entity_id: <proposal id>, data: <entry>}`), emitted once per newly
  recorded verdict.
- `engine::real_core`: `PULSO_CORE_PORT=live` builds the `LiveCore` from env (no network at construction);
  `LiveCore::readiness` = version probe against the pin + prod alias read, tested against the core-client GoldenCore
  transcripts. `CorePort::is_real()` (default false) drives the `real-narrow` labels; doubles can never claim it.

## Verdict rules (what the pipeline derives from what the job committed)

| Situation | Verdict | Reason |
|---|---|---|
| aggregate row below k | not_evaluable | evidence_insufficient (no model call: the row is suppressed) |
| model refused / unavailable / unusable | not_evaluable | model_refused / model_unavailable / model_invalid |
| verifier disagrees | not_viable | verifier_refuted |
| recompute does not corroborate | not_viable | claim_not_corroborated |
| compile denies the kind | not_evaluable | kind_not_supported / release_settings_not_allowed (BK0 and compile must agree) |
| structural gate fail | not_viable | gate_failed (an override stays on the trail, labelled simulated) |
| structural gate not_evaluable | not_evaluable | gate_not_evaluable |
| gate pass, native evaluation fail | not_viable | native_eval_failed |
| gate pass on its own, native pass | viable | structural_gate_passed |

`LedgerEntry::validate` refuses `viable` with any non-pass gate, any override, no gate result or no evidence.

## Honest gaps

- The stand-in sensor still emits only `sig-0001`; the N signals of a pipeline run are seeds given by the caller (aggregates), each
  filed under that thread-local id in its own lab row. Real signals arrive with the R1M monitor.
- On the offline `DoublePort` the structural gate never passes (identical arms), so a viable verdict is exercised only by a test
  fake whose baseline arm does not complete one case; no real Core evidence of a viable proposal exists.
- `Gateway` has no TLS (std only; a localhost sidecar is the target) and no live model was called. `Roleplay` does not reimplement
  id ordinals. No container was started; `RealCore` has not run against a live Core in this lane.
- The console does not fold `proposal_verdict` yet (debug-api projection ignores unknown kinds; L-CAPI/L-CONSOLE).
- A crash between ledger write and event emit loses that one event (the entry is durable and replays silently).
