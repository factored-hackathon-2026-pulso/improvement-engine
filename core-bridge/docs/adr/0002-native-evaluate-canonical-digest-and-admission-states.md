# ADR 0002: Canonical `native_evaluate` broker digest, in-progress admissions, explicit early close

Status: accepted (L5 review round 2, plan 17.3.5).

## Context
Two code paths asked the broker for `native_evaluate` with different payload digests: L3b `ProtectedBuilderToolExecutor`
(`sha256(JCS({proposal_id, evaluation_context_ref}))`) and L5 `AdmissionGate` (also bound candidate hash and suite).
A broker that commits to one digest per binding cannot accept both. Separately, a second caller finding an admission in
state `consumed` demoted it to `unknown`, which could poison the still-running first evaluation; and early run closure
inside `PulsoScenarioHarness` was swallowed silently.

## Decision
1. **One digest function.** `pulso_core_runtime.evaluation.digests.native_evaluate_digest(proposal_id,
   evaluation_context_ref, candidate_hash, suite_id, suite_version, suite_digest)` is the only definition of the
   `native_evaluate` payload digest (the broader one). The module is pure (hashlib + canonical JSON) so L3b can import it
   without pulling psycopg. L5 (`EvaluationRuntime.evaluate`, the HTTP-campaign wrapper through it) already uses it.
   **Open wiring item for L3b:** `tools/builder.py::_evaluate` still computes the narrow digest and must either call
   this function with the admission's candidate hash and suite (it only has the `evaluation_context_ref`, so it needs the
   admission loaded through the evaluation gate) or drop its own `native_evaluate` check and rely on the gate's, which is
   the one that has the data. Until then the two checks disagree; the broker must be configured for the broad digest only.
2. **Admission states.** `consumed` + `now < deadline` + no stored result: the caller is denied with
   `evaluation_in_progress` (409) and the state is NOT mutated, so the first run's finish/reset still succeeds and a
   replay of the same admission returns the stored report. `consumed` past the deadline without a stored result becomes
   `unknown` (crash recovery; the budget meter bounds every run by the deadline). Losing the `admitted -> consumed` CAS
   is also `evaluation_in_progress`.
3. **Early close is evidence.** `RunRecord.closed_early` is set when a later scenario step raises `run_closed`.
   `ArmReport` carries `closed_early` and `closed_early_runs`; the stored native eval report (bridge table, not the 409
   body) carries `pulso_evidence{closed_early, closed_early_runs, runs}`. Codex scores; the bridge never turns an early
   close into a silent pass.

## Update (documentation pass, HEAD 4984d92)
The open wiring item for L3b is closed: `tools/builder.py::_evaluate` loads the admission through the injected admission
store and calls `native_evaluate_digest(...)` with the admission's candidate hash and suite, so both paths use the
broad digest. See ADR 0007 for the `evaluation_context_ref` format.

## Consequences
Digest field names are contract. A stuck `consumed` row blocks until its deadline, then `unknown` (retry needs a new
admission with `evaluation_attempt+1`).
