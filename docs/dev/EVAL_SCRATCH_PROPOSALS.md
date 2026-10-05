# E8: regression-proof scratch proposals are visible to approvers (design, not implemented)

Problem. `registry-writer::eval` proves a candidate by creating a MANUAL-origin proposal (`[improvement-engine] evaluation base|candidate-N`, created by
`pulso-engine`), putting the candidate and the suite in it, then `validate`, `freeze`, `evaluate`. The proposal ends in state `evaluated`, which is exactly the
state an approver can `approve` and `publish` (it happened live: a supervisor published one and moved staging). Nothing marks it as scratch.

What agent-core allows the engine (read in `agent_core/registry/service.py`, `roles.py`, main 52e6de8):
- `reject`, `approve`, `publish` need a HUMAN approver with step-up (`require_approver`): the engine cannot reject or withdraw. There is no delete.
- `reopen` needs only `constructor`: it takes a proposal in `candidate`, `evaluated` or `approved` back to `draft` and clears the candidate hash. A draft cannot be
  approved or published (`approve` expects `evaluated`, `publish` expects `approved`).
- `list_proposals` filters by `agent_id`, `state`, `created_by`; no title or origin-class filter beyond what the platform does itself. `manual` is not quota-limited.

Options.
1. Engine reopens its scratch proposals right after `evaluate` (and after reading the report). Needs no agent-core change. Residual: a window of seconds in which an
   `evaluated` proposal exists, and an approver could still see a draft titled `evaluation ...`. Cost: add `reopen` to the writer allow-list for proposal ids the writer
   itself created (receipt store), nothing else; the existing test that forbids `reopen` becomes "only on own evaluation drafts".
2. Distinct marker: fixed title prefix (already `[improvement-engine] evaluation`) plus `created_by = pulso-engine`; the platform hides or labels these in the approval
   queue. Cheap, but it is a convention, the server still lets an approver publish one.
3. Server-side scratch origin in agent-core (`Origin.eval_scratch` or `purpose=evaluation`): approve/publish refuse it (`illegal_transition`), excluded from the default list and from
   the auto_detect quota. Removes the failure class.
4. Prove without a proposal: a stateless `POST /v1/registry/evaluate` that takes the candidate changes and the suite in the body and returns the EvalReport. Nothing persists, nothing to
   approve. Largest agent-core change, cleanest result.

Recommendation. Do 1 now (engine only, same lane as the writer) and 2 in parallel (platform filter), ask for 3 (small: one enum value and two guards) and keep 4 as the target.
The announced `auto_detect` proposal is the ONLY proposal meant for approvers; everything else is scratch.

Exact asks.
- agent-core: add `Origin.eval_scratch`; `approve` and `publish` on it answer `illegal_transition`; exclude it from `list_proposals` unless `origin=eval_scratch` is asked for and from the
  per-day quota; allow `constructor` to create it. Optional (later): stateless evaluate endpoint.
- platform (support-platform): in the approval queue hide proposals whose `created_by` is the engine principal and whose title starts with `[improvement-engine] evaluation`, or whose origin is `eval_scratch` once it exists;
  never offer `Publicar` on them.
- engine (this repo, when approved): `Writer::reopen_own(proposal_id)` after each evaluation run, only for ids created by this writer; switch the creation origin to `eval_scratch` when agent-core has it.
