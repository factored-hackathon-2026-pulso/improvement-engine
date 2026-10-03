# agent-core-assets

Versioned Core assets (YAML entities) for the pinned agent-core (SHA `894fa65575d83420523f33ec1c6919b8965f7ebe`, contracts
1.3.0), plus the validator that keeps them honest. Contract revision: `pulso-two-teams-1`.

## Worlds

- `worlds/attention-demo`: the pinned registry-demo world (agent `atencion`, flow `disputa-cargo`, decision models, policy,
  templates, tools, calibration `cal-demo`, suite `disputas-suite` with 3 scenarios, release `demo`). Layer mapping in
  `layer_mappings/atencion@1.0.0.yaml`.
- `worlds/pulso-evolution`: four task stages (`pulso-scout`, `pulso-verifier`, `pulso-builder-design`, `pulso-writer`;
  `mode: task`, `invocable_by: [builder]`), Jev model `jev-evolution-route`, prompts, `pulso/*` tools, `registry/*` ToolDefs
  generated from `BUILDER_TOOL_DEFS` (`tools/gen_builder_tooldefs.py`) and the smoke suite `pulso-smoke`. Flows read run
  inputs from `facts.binding.value.*` produced by `pulso/bind_context` (ADR `core-bridge/docs/adr/0004`).
- `manifest.yaml`: pin, per-world `files_digest`, `capability_catalog_digest`, `expected_state_digest`, release ids
  (`atencion` `rel-e26df0070f6be82f`, `pulso-scout` `rel-1cddd55d1fe8f19f`, `pulso-verifier` `rel-db71ae5ed04c6131`,
  `pulso-builder-design` `rel-5f152bbf73ea31b4`, `pulso-writer` `rel-bf3f06148962dabb`) and `expected-state.json`. Both
  are generated (`assetcheck.py write-state`) and must change in the same change as the asset.

## Validator (`tools/assetcheck.py`)

    uv run --python 3.12 --with pyyaml python tools/assetcheck.py check      # offline tier
    uv run --python 3.12 --with pyyaml python tools/assetcheck.py validate   # agent-core tier (pinned checkout)
    uv run --python 3.12 --with pyyaml python tools/assetcheck.py write-state

Offline: layout, manifest and expected-state drift, stage invariants (no precomputed finding, no waiting nodes
`collect|confirm|transfer|await_approval`, no `const|default|examples` in output schemas), merge conflicts, layer mappings,
secret-looking keys and values. Agent-core tier (`AGENT_CORE_CHECKOUT` optional): `agentcore validate` per world and merged,
computed state equals `expected-state.json`, in-memory import (re-import `illegal_transition`, bot `forbidden_role`, session
admin `step_up_required`).

## Tests and CI

    pytest -c pytest.ini tests          # first RED: tests/test_policy_first_red.py
    pwsh ../core-bridge/scripts/ci.ps1 -Job agent-core-assets

The catalogue in `core-bridge/src/pulso_core_runtime/stages/catalog.py` must equal the Flows and Agents here
(`check_against_assets`: first node `pulso/bind_context@1`, `Agent.tools_allowed`, input slots, tool nodes, agent node
`save_as` and the Core-subset `output_schema`).

## Seeding

The standalone stack imports these worlds with `local/core/init/seed_assets.py` and verifies that the resulting five
release ids equal `manifest.yaml`.

## Known gaps

New attention scenarios have no provider scripts (harness side); CODEOWNERS lives outside this directory; the assets are not
copied into the runtime image; the writer flow was completed in commit 623f5fc but its end-to-end evaluate branch was not
re-verified in this documentation pass. Details: `core-bridge/docs/journal/claude-0005-l4-agent-core-assets.md`.
