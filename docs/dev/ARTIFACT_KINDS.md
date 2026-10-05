# What the engine can create beyond text patches (ART2)

Status: the compilers exist and are unit-tested (`seams/crates/reasoning/src/art2.rs`, 5 tests). They are NOT yet wired into the
mapping table, Builder request, suite generator or proof, so no live announcement exists. BK0 verdicts for `tool` and `policy` stay `denied`.

## Tool link (read-only) - `compile_link_tool`
Changes: Flow (new `tool` node on a menu edge, minor bump), `Agent.tools_allowed`, unchanged ToolDef copy. The Builder picks only an
`edge_id` from `edge_menu` (edges out of collect/tool/respond nodes with label ok/next, non-terminal, non-escalate target; edges leaving
rule/decide/confirm/verify/escalate/end are frozen). The new node is pass-through (ok -> original target; error/timeout/denied -> the
existing `tool_failure` escalate node). Preconditions (`check_link`): `read` risk only (`write_tool_human_only`), listed by tool-service
`GET /v1/tools` with equal risk/min_auth_level, `source` equal (`source_mismatch`/`source_missing`), agent `invocable_by` within
customer/advisor, not already linked. Finding: fixtures drift (registry source `productos` vs service `customer_products`), so with the
current agent-core fixtures every link is refused until a human aligns the ToolDef source.
Not done: agent-node mode, mapping rows, suite (error/timeout/denied seeds fail on base, pass on candidate), proof, dossier text.

## Policy
`policy_hypothesis` (no draft): boundary values 249/250/251/499/500/501 and the 250 vs 500 divergence (`policy_hypothesis`).
`tighten_policy`: single numeric comparison only; new value is a structured param; `not_weaker` is interval inclusion over the same
variable; the consuming rule's true branch must reach an escalate node (`direction_unknown`); owner other than `engine` sets
`needs_owner_ack` and `docs.owner_ack` (never announced automatically). Adding a "required confirmation" is a flow concern, not representable as a policy.
