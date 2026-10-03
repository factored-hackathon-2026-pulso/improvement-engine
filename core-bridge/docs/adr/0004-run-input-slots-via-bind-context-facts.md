# ADR 0004: Run inputs reach Flows as `bind_context` facts (pinned Core keeps slots `claimed`)

Status: accepted (integrator, commit 447772b). Contract revision: `pulso-two-teams-1`.

## Context
Task invocations pass declared inputs (`briefing_ref`, `hypotheses_ref`, `design_input_ref`, `draft_plan_ref`,
`proposal_id`, `base_release_id`, ...) as the run `input`. The pinned Core (agent-core `86a7674`, contracts 1.3.0) keeps
run-input slots in the `claimed` state and never validates them (BITACORA 2026-10-03, integrator entry), and a Flow
reading `slots.*` of such a slot escalated before any tool call (recorded first as a strict xfail in
`tests/integration/test_scout.py`, then fixed).

## Decision
1. `pulso/bind_context` (the mandatory first node of every Flow) re-exposes the stage's declared input slots as
   **tool-origin facts**: `facts.binding.value.<slot>` (`tools/bind.py::_facts`). Only the slots listed in
   `stages/catalog.py::CATALOG[stage].input_slots` are exposed; absent ones are `null`. The writer also gets
   `evaluate_enabled` coerced to a strict boolean (`true` only for the literal `True`).
2. The invoke service rejects undeclared slots before any effect (`unknown_input_slot`, using
   `InvokeSettings.stage_slots` built from the catalogue), so only declared names can appear as facts.
3. The values come from the frozen `InvocationContext.inputs` (registered by the invoke service from the digested
   request), never from tool arguments or model output. `bind_context` also returns `binding_state`, `tenant_id`,
   `job_id` and the stage flags (`is_scout|is_verifier|is_builder|is_writer`).
4. CI check `stages/catalog.py::check_against_assets` fails when a Flow reads `slots.X` (legacy), reads a slot that is not
   in the catalogue, or the catalogue lists a slot the Flow does not read.
5. Assets (L4) read `facts.binding.value.*`; the Flows therefore depend on the binding having been confirmed (a denied
   binding means the facts are never produced and the model is never reached).

## Consequences
The slot values are visible to the model as facts (they are refs and flags, not secrets; sensitive material travels as
refs resolved by the broker). If a later Core pin validates run-input slots, this ADR can be retired by reading
`slots.*` again, and the catalogue check inverted. Unknown: whether the `claimed` state is intended upstream behaviour or
a defect; no upstream issue was filed from this repository.
