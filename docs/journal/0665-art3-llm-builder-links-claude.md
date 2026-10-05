# 0665 ART3 the model drives the tool link, live (UTC 2026-10-05, CLAUDE)

Stacked on PR 113. Independent link review added (`roles::link_review_request`, `pipeline` stage `link_review`), offline negative tests (`tests/art3.rs`: invalid edge, write-tool target, refuting review).
Live on own stack pulso-art2 (agent-core main 52e6de8, real gateway, flash for Scout/Builder, pro for the Verifier and the link review): first attempt compiled; 4 model calls (scout, claim verifier, builder,
link review), 3534 tokens in, 1556 out, USD 0.001574. The Builder chose edge `pedir_radicado.ok` (the scripted run used `consultar.ok`); review supported (read-only, valid edge); proof: base fails 6/6, candidate passes,
guards pass, 14/14 GateItems; announced as an auto_detect draft (4 changes, stand-in credential). Policy finding: no model call, hypothesis + tighten draft proven 16/16, `needs_owner_ack`, not delivered.
Not done: `tool_source` assertions (not in the metric catalog). E8 design: docs/dev/EVAL_SCRATCH_PROPOSALS.md.
