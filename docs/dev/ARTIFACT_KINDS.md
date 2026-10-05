# What the engine can create beyond text patches (ART2)

Two kinds are end to end (mapping, Builder/roles, compiler, suite, proof, dossier) and were run LIVE on an own stack
(`PULSO_STACK_PREFIX=pulso-art2`, agent-core main a84080e, aligned registry-e2e import). Flows are NOT done (lane order: flows last).

| Kind | Op | Outcome | Guards |
|---|---|---|---|
| tool link, read-only | `link_tool` (`reasoning::art2::compile_link_tool`) | `proposed` -> proof -> `announced` (auto_detect draft: flow + agent + unchanged ToolDef copy + eval_suite) | frozen edges, pass-through node, tool_failure exits, 3 pulso-min guards + a pass-through guard |
| policy | `policy_hypothesis` (no draft) | note with the 250 vs 500 divergence and boundary values 249/250/251/499/500/501 | none to run, a person decides |
| policy | `tighten_policy` (`patch::compile_policy_tighten`) | `needs_owner_ack` (proven natively, NEVER announced) | monotone comparator, boundary guards on old and new threshold |

## Tool link
Changes: Flow (new `tool` node on a menu edge, minor bump; an exact entry-flow pin of the agent follows the new version), `Agent.tools_allowed`,
unchanged live ToolDef copy. The Builder picks only an `edge_id` from `edge_menu` (collect/tool/respond edges `ok`/`next`, non-terminal, non-escalate);
edges leaving rule/decide/confirm/verify/escalate/end are frozen. The new node is pass-through; error/timeout/denied go to the existing
`tool_failure` escalate node. References are written in the style of the entity (fixture `id@major`, registry `{id, spec}`).
Preconditions (`check_link`): `read` risk only (`write_tool_human_only`), listed by the tool-service (`tool_not_in_service`) with equal risk and
min_auth_level (`tool_def_drift`), equal `source` (`source_mismatch`, `source_missing`), no required args (`args_required`), agent `invocable_by`
within customer/advisor (`principal_not_served`), not already linked. Mapping: row `tool_peers_gap` (metric A5 on `leer_pqr_cliente`, TEL1 not
merged, `announceable_now: true`). Suite mechanism `tool_link`: seeds error/timeout/denied on the linked tool (es+pt, 6 cases, fail on base because
the base never calls the tool), guards = consultas pulso-min guards + pass-through ok. NOT measured: tool identity (`engine.tool_called` has no tool id),
that the answer uses the tool data (the link only makes it available to the flow), wording. agent-node mode: not done.

## Policy
`tighten_policy`: single numeric comparison only (`> >= < <=`); the new value is the structured `tighten_to` of the mapping table (never the Builder's,
no model call); `not_weaker` is interval inclusion over the same variable; the consuming rule's true branch must reach an escalate node
(`direction_unknown`); loosening is `policy_loosening_denied`; other shapes `unknown_policy_shape`. Owner other than `engine` -> `needs_owner_ack`
(text marker in the docs; agent-core `VersionDocs` has no extra fields), the dossier says `needs_owner_ack`, the loop never delivers it.
A "required confirmation" is a flow concern (rule G0-05), not representable as a policy.

## Fixture drift (ask, not fixed upstream)
agent-core registry-e2e ToolDef `source` differs from tool-service (`productos` vs `customer_products`, `movimientos` vs `customer_transactions`,
`pqr` vs `customer_cases`) or is missing (`obtener_pqr`, `buscar_transacciones`): the engine refuses every link on them. The engine fixture and the
local stack use ALIGNED sources (`scripts/reasoning/export_art2_graph.py`, additive). Ask to agent-core: align the fixtures; to tool-service: keep
`GET /v1/tools` as the oracle and publish the source classification. The tool-service listing used here is a snapshot of its `registry/tools`,
not a live `GET /v1/tools` (the service needs its dataset).

## ART3: the model drives the link (live)
`pipeline::reason_candidate` on `tool_link:consultas/leer_pqr_cliente`: Scout and Builder mimo flash, claim Verifier and an independent LINK REVIEW (second Verifier call, mimo pro, on
structured facts only: tool risk_class and source, edge node types, wiring) before the proof. The Builder sees the edge menu as a tool and answers `edge_id`; the compiler refuses an unknown
edge (`edge_unknown`) and a write tool (`write_tool_human_only`) whatever the model says; a refuting review blocks (`link_review_refuted`). Policy findings take no model call (deterministic hypothesis
note and tighten draft, `needs_owner_ack`). Live result: see journal 0665. `tool_called.tool_source` (agent-core PR 56) is in the event payload but NOT in the metric catalog
(`engine.tool_called` = status, latency_ms, attempt), so a scenario assertion cannot use it yet: ask to agent-core to add it to the catalog.
