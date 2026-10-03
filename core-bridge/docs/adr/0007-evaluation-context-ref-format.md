# ADR 0007: `evaluation_context_ref` format and the derived idempotency key

Status: accepted (L5, review rounds 1-2). Contract revision: `pulso-two-teams-1`.

## Context
The evaluation admission is identified by `evaluation_context_ref`, supplied by Codex and used (a) as primary key of
`pulso_bridge.eval_admissions`, (b) inside the registry idempotency key `pulso-eval:<ref>` that makes a second use of the
same admission replay the stored report, (c) in an HTTP header on the campaign wrapper, and (d) in the broker payload
digest. Truncating, trimming or normalising it would map two references to one key, or one reference to two.

## Decision
- Format: 1 to 200 ASCII characters from `[A-Za-z0-9_.:-]`, matched with `fullmatch` (a trailing newline is not a valid
  ref). Implementation: `evaluation/admission.py::valid_context_ref` (regex plus `isascii()`); `tools/builder.py` uses
  `\Z` and `FlowEvaluationGate` re-validates, because an earlier `$`-anchored check accepted a trailing newline.
- Never truncated and never normalised. An invalid ref is `evaluation_context_invalid` (422) on HTTP, or `denied`
  before any effect on the tool path.
- Key: `eval_key(ref) = "pulso-eval:" + ref`. It is also the registry write key, so one admission is one native
  evaluation, and a replay returns the stored report (including `failed_infra`; a retry needs a new admission with
  `evaluation_attempt + 1`).
- The ref lives in the sealed writer commitment (`registry_mutation_commitment.evaluation_context_ref`), never in tool
  arguments; the HTTP wrapper reads it from `X-Pulso-Evaluation-Context` and does not log it.
- Uniqueness: one admission row per ref; `(tenant, proposal, candidate_hash, evaluation_attempt)` is unique too
  (`evaluation_attempt_exists`, 409).
- The arm request `idempotency_key` uses the same charset check (`idempotency_key_invalid`, 422).

## Consequences
Codex must generate refs inside this alphabet and length (no `/`, no spaces). Open item: the generation scheme on the
Codex side (opaque random versus derived) is not specified in this repository.
