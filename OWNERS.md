# OWNERS

Single path map for the improvement engine. Every tracked path belongs to exactly one
lane. Team CL (Claude) lanes start with `L-`, team CX (Codex) lanes with `X-`.
The map below is the source of truth; `docs/agents/tests/test_owners.py` (loader:
`docs/agents/owners_map.py`) fails on any unowned or tied tracked path and on any
planned new path whose owner differs from the one declared here.
Baseline: 0 unowned and 0 tied tracked paths at origin/main 853b029 (1683 files).

## Resolution

- Glob syntax: `*` one path segment, `**` any depth, `{a,b}` alternation,
  `[x-y]` and `(a|b)` pass through as regex.
- The most specific glob wins (specificity = glob length without `*`).
  Two different lanes at the same specificity is a tie and is a failure.
- Paths of the infra repo are written `infra:<path>` and belong to L-INFRA.

## Rules for new paths

1. A new file must match exactly one lane before its first commit. If no glob matches,
   the author adds a glob (or a `newpaths` line) to this file in the same PR; only
   L-GOV edits this file, so the author opens a `[DEP-ASK]` to L-GOV.
2. A glob added here must not create a tie with an existing glob of another lane.
3. A lane never edits a path owned by another lane; it requests the change from the
   owner (`[DEP-ASK]`). Single integrators: X-FACADE for root `Cargo.toml`, `Cargo.lock`,
   `.github/**`, `crates/core/Cargo.toml`, `crates/core/src/lib.rs` and the Codex gate
   scripts; L-CLIENT for `seams/Cargo.toml` and `seams/Cargo.lock`.
4. Numbered ranges (migrations, ADR, journal) are allocated per lane in the globs below;
   a new number is taken only inside the lane's own range.
5. Planned paths are declared in the `newpaths` block and checked by the same test.

## Owners

```owners
X-FACADE: Cargo.toml ; Cargo.lock ; rust-toolchain.toml ; .github/** ; scripts/verify-local-ci.ps1 ; scripts/run-local-*.ps1 ; scripts/init-local-env.ps1 ; tests/run-local-*.Tests.ps1 ; tests/test_postgres_ci_caller_contract.py ; crates/core/Cargo.toml ; crates/core/src/lib.rs ; crates/core/src/{debug_console,durable_run_events,model_provider,quota_grant,run_activity,run_config,run_timeline_v2,workflow_bridge}.rs ; crates/core/src/{pipeline,authority}.rs ; crates/core/tests/{model_provider,artifact_repository,model_attempt_repository,postgres_artifact_migration,postgres_run_events,tracer,quota_grant,run_activity,run_config}.rs ; contracts/pipeline/** ; migrations/0001_* ; migrations/0002_model_attempt_ledger.sql ; migrations/0003_* ; migrations/003[0-9]_* ; crates/core/src/facade_*.rs ; crates/core/tests/facade_*.rs ; docs/adr/033[2-9]-* ; docs/journal/(046[4-9]|047[0-9])-*
X-CONF: crates/core/src/durable_jobs.rs ; crates/core/tests/durable_jobs.rs ; crates/core/tests/conformance/** ; migrations/004[0-2]_* ; docs/adr/(031[6-9]|032[0-3])-* ; docs/journal/(043[2-9]|044[0-7])-*
X-COMPILE: crates/core/src/{change_compiler,core_task}.rs ; crates/core/tests/{change_compiler,core_task}.rs ; docs/adr/(030[8-9]|031[0-5])-* ; docs/journal/(041[6-9]|042[0-9]|043[0-1])-*
X-MAP: crates/core/src/{e0_builder_design,e0_core_draft_binding,e0_investigation_plan,e0_mechanism_resolution,e0_opportunity_qualification,e0_proposal_assembly}.rs ; crates/core/tests/{e0_investigation_plan,e0_mechanism_resolution}.rs ; docs/adr/(035[6-9]|036[0-3])-* ; docs/journal/(051[2-9]|052[0-7])-*
X-ARTIF: crates/core/src/artifact_kind_*.rs ; crates/core/tests/artifact_kind_*.rs ; migrations/004[3-4]_* ; docs/adr/030[0-7]-* ; docs/journal/(040[0-9]|041[0-5])-*
X-GATE: crates/core/src/{evaluation_plan,final_eligibility,paired_scenario,native_evaluation,e0_safety_oracle,independent_verifier,governed_registry,jev_decision,sandbox,policy_oracle}.rs ; crates/core/tests/{sandbox,evaluation_plan,final_eligibility,paired_scenario,native_evaluation_admission,e0_safety_oracle,independent_verifier,governed_registry,jev_decision,sandbox_identity,policy_oracle}.rs ; migrations/002[5-9]_* ; docs/adr/034[0-7]-* ; docs/journal/(048[0-9]|049[0-5])-*
X-SENS: crates/core/src/{autonomous_scout,deterministic_sensor,e0_deterministic_sensor,e0_query_lab,enriched_history,local_lab,local_simulation,platform_sensor,signal_portfolio}.rs ; crates/core/tests/{autonomous_scout,deterministic_sensor,e0_deterministic_sensor,e0_query_lab,enriched_history,local_lab,local_simulation,platform_sensor,platform_scout}.rs ; crates/core/src/detectors/** ; crates/runner/** ; migrations/004[5-6]_* ; docs/adr/038[0-7]-* ; docs/journal/(056[0-9]|057[0-5])-*
X-SRC: crates/core/src/{original_contact_projection,platform_discovery,platform_discovery_verifier,platform_observations,platform_source_policy,source_validation}.rs ; crates/core/tests/{original_contact_projection,platform_observations,platform_source_policy,postgres_platform_observations,source_validation}.rs ; crates/source-adapters/** ; contracts/*.schema.json ; contracts/{README.md,__init__.py,validate_fixtures.py} ; contracts/fixtures/** ; contracts/sources/** ; tests/test_artifact_envelope_contract.py ; tests/test_source_snapshot_partition_inventory.py ; migrations/0002_pulso_platform_observations.sql ; migrations/(001[5-9]|002[0-4])_* ; docs/adr/(038[8-9]|039[0-5])-* ; docs/journal/(057[6-9]|058[0-9]|059[0-1])-*
X-LEARN: crates/core/src/{memory_store,governed_memory_use,memory_temporal_protocol,e0_frozen_memory_cycle,e0_frozen_memory_publication,e0_frozen_summary,e0_frozen_verifier,run_fork,wiki_scratch}.rs ; crates/core/src/replay_*.rs ; crates/core/tests/{governed_memory_use,memory_temporal_protocol,run_fork,wiki_scratch,postgres_memory_temporal_receipts,published_memory}.rs ; crates/core/tests/replay_*.rs ; migrations/0002_pulso_memory_control.sql ; migrations/0004_* ; migrations/(000[5-9]|001[0-4])_* ; docs/adr/(034[8-9]|035[0-5])-* ; docs/journal/(049[6-9]|050[0-9]|051[0-1])-*
X-SCEN: crates/core/src/{value_model}.rs ; crates/core/src/scenario_*.rs ; crates/core/tests/{value_model}.rs ; crates/core/tests/scenario_*.rs ; migrations/004[7-9]_* ; docs/adr/037[2-9]-* ; docs/journal/(054[4-9]|055[0-9])-*
X-DOC: docs/IMPLEMENTATION_STATUS.md ; docs/architecture/** ; docs/data/** ; docs/gaps/** ; docs/platform-observations.md ; docs/local-e0-e2e-runner.md ; docs/spec-amendments/codex/** ; docs/adr/(032[4-9]|033[0-1])-* ; docs/journal/(044[8-9]|045[0-9]|046[0-3])-*
X-REV: docs/reviews/codex/** ; docs/adr/(036[4-9]|037[0-1])-* ; docs/journal/(052[8-9]|053[0-9]|054[0-3])-*
L-GOV: AGENTS.md ; docs/plan-real/r1-* ; docs/reports/w4a-live/** ; CLAUDE.md ; CONTEXT.md ; README.md ; OWNERS.md ; .gitattributes ; .gitignore ; docs/BITACORA_PULSO.md ; docs/agents/** ; docs/adr/** ; docs/journal/00[0-9][0-9]-* ; docs/spec-amendments/claude/** ; docs/reports/gates/** ; docs/reviews/claude/** ; docs/lanes/** ; contracts/engine-run/** ; contracts/engine-steps/** ; scripts/gov/** ; contracts/artifact-kinds/** ; migrations/009[5-9]_* ; docs/adr/020[0-9]-* ; docs/journal/(026[0-9]|027[0-5])-*
L-ENV: local/compose.yaml ; scripts/env/** ; scripts/verify-local-all.ps1 ; scripts/verify-local-fast.ps1 ; scripts/dev.ps1 ; tests/test_local_compose_contract.py ; seams/xtask/** ; local/.env.example ; local/.secrets/** ; tests/verify-local-*.Tests.ps1 ; local/compose.d/** ; docs/adr/018[0-9]-* ; docs/journal/(022[8-9]|023[0-9]|024[0-3])-*
L-MODEL: roleplay-llm/** ; local/core/gateway/** ; scripts/dc/** ; agent-core-assets/worlds/** ; agent-core-assets/corpus/** ; core-bridge/src/pulso_core_runtime/llm/** ; core-bridge/src/pulso_core_runtime/stages/** ; core-bridge/tests/llm/** ; core-bridge/tests/runtime/test_stages* ; docs/adr/023[0-9]-* ; docs/journal/(030[8-9]|031[0-9]|032[0-3])-* ; local/compose.d/l-model.yaml
L-BRIDGE: core-bridge/** ; bridge-contract/** ; agent-core-assets/** ; local/core/** ; scripts/core/** ; scripts/contracts/** ; docs/reports/contracts/** ; docs/adr/012[0-9]-* ; docs/journal/(013[2-9]|014[0-7])-* ; local/compose.d/l-bridge.yaml
L-E2E: e2e-core/** ; demo/** ; scripts/e0/** ; docs/reports/demo-quejas/** ; seams/crates/thread10/** ; seams/crates/pulso/** ; scripts/demo-magic* ; scripts/demo-platform* ; docs/reports/demo-magic/** ; docs/reports/demo-platform/** ; docs/reports/e2e/** ; docs/adr/016[0-9]-* ; docs/journal/(019[6-9]|020[0-9]|021[0-1])-* ; local/compose.d/l-e2e.yaml
L-PLAT: platform-contract/** ; platform-exporter/** ; platform-sim/** ; contracts/product/** ; seams/crates/control-api/src/correlation/** ; migrations/007[0-4]_* ; docs/adr/026[0-9]-* ; docs/journal/(035[6-9]|036[0-9]|037[0-1])-* ; local/compose.d/l-plat.yaml
L-CLIENT: seams/scripts/** ; seams/Cargo.toml ; seams/Cargo.lock ; seams/README.md ; seams/crates/core-client/** ; seams/crates/model-client/** ; docs/adr/014[0-9]-* ; docs/journal/(016[4-9]|017[0-9])-* ; local/compose.d/l-client.yaml
L-ENGINE: scripts/aggregate/** ; scripts/reasoning/** ; seams/crates/reasoning/** ; seams/crates/abi/** ; seams/crates/engine/** ; contracts/engine-handlers/** ; seams/crates/steps/** ; seams/crates/swap/** ; docs/adr/017[0-9]-* ; docs/journal/(021[2-9]|022[0-7])-* ; local/compose.d/l-engine.yaml
L-CAPI: seams/crates/control-api/** ; seams/crates/debug-api/** ; seams/crates/maturity/** ; contracts/control-api/** ; migrations/008[5-9]_* ; docs/adr/013[0-9]-* ; docs/journal/(014[8-9]|015[0-9]|016[0-3])-* ; local/compose.d/l-capi.yaml ; scripts/triggers/**
L-PG: docs/reports/pglive/** ; seams/crates/pg/** ; seams/crates/sources/** ; db/** ; seams/crates/worker/** ; local/pg/** ; migrations/00[56][0-9]_* ; docs/adr/025[0-9]-* ; docs/journal/(034[0-9]|035[0-5])-* ; local/compose.d/l-pg.yaml
L-EVAL: seams/crates/eval/** ; agent-core-assets/eval-suites/** ; migrations/009[0-4]_* ; docs/adr/019[0-9]-* ; docs/journal/(024[4-9]|025[0-9])-* ; local/compose.d/l-eval.yaml ; scripts/scoring/**
L-AUTH: seams/crates/authority/** ; local-identity/** ; migrations/008[0-4]_* ; docs/adr/010[0-9]-* ; docs/journal/(010[0-9]|011[0-5])-* ; local/compose.d/l-auth.yaml
L-MEM: seams/crates/memory/** ; migrations/007[5-9]_* ; docs/adr/022[0-9]-* ; docs/journal/(029[2-9]|030[0-7])-* ; local/compose.d/l-mem.yaml
L-CONSOLE: debug-console/** ; docs/adr/015[0-9]-* ; docs/journal/(018[0-9]|019[0-5])-* ; local/compose.d/l-console.yaml
L-OPS: Dockerfile ; .dockerignore ; docs/runbooks/** ; docs/dev/** ; scripts/dev-stack/** ; scripts/o11y/** ; docs/security/** ; docs/contracts/metrics.md ; local/observability/** ; docs/adr/024[0-9]-* ; docs/journal/(032[4-9]|033[0-9])-* ; local/compose.d/l-ops.yaml
L-BREADTH: seams/crates/artifacts/** ; docs/adr/011[0-9]-* ; docs/journal/(011[6-9]|012[0-9]|013[0-1])-* ; local/compose.d/l-breadth.yaml
L-INFRA: infra:** ; docs/adr/021[0-9]-* ; docs/journal/(027[6-9]|028[0-9]|029[0-1])-*
```

## Planned new paths

```newpaths
crates/core/src/artifact_kind_flow.rs -> X-ARTIF
crates/core/tests/artifact_kind_flow.rs -> X-ARTIF
crates/core/src/policy_oracle.rs -> X-GATE
crates/core/tests/policy_oracle.rs -> X-GATE
crates/core/src/pipeline.rs -> X-FACADE
crates/core/src/authority.rs -> X-FACADE
crates/core/src/facade_steps.rs -> X-FACADE
crates/core/tests/facade_steps.rs -> X-FACADE
crates/core/src/replay_clock.rs -> X-LEARN
crates/core/src/scenario_factory.rs -> X-SCEN
crates/core/src/detectors/family_01.rs -> X-SENS
crates/core/tests/conformance/jobs.rs -> X-CONF
contracts/pipeline/transcript.json -> X-FACADE
contracts/artifact-kinds/matrix.json -> L-GOV
contracts/product/fixture.json -> L-PLAT
contracts/engine-steps/pack/manifest.json -> L-GOV
contracts/engine-run/report.schema.json -> L-GOV
contracts/control-api/openapi.yaml -> L-CAPI
contracts/engine-handlers/abi.md -> L-ENGINE
roleplay-llm/server.py -> L-MODEL
roleplay-llm/tests/test_scanner.py -> L-MODEL
local/pg/init.sql -> L-PG
seams/crates/pulso/build.rs -> L-E2E
seams/crates/pulso/README.md -> L-E2E
seams/crates/pulso/src/config.rs -> L-E2E
seams/crates/pulso/src/health.rs -> L-E2E
seams/crates/pulso/src/healthcheck.rs -> L-E2E
seams/crates/pulso/src/run/supervisor.rs -> L-E2E
seams/crates/pulso/src/run/tasks.rs -> L-E2E
seams/crates/pulso/src/run/db.rs -> L-E2E
seams/crates/pulso/tests/run_process.rs -> L-E2E
local/observability/alerts.yaml -> L-OPS
local/compose.d/l-pg.yaml -> L-PG
local/compose.d/l-model.yaml -> L-MODEL
local/compose.d/shared.yaml -> L-ENV
seams/Cargo.toml -> L-CLIENT
seams/crates/abi/src/lib.rs -> L-ENGINE
seams/crates/steps/src/lib.rs -> L-ENGINE
seams/crates/steps/src/cells.rs -> L-ENGINE
seams/crates/steps/tests/cells.rs -> L-ENGINE
scripts/aggregate/bank_cells.py -> L-ENGINE
scripts/aggregate/tests/test_bank_cells.py -> L-ENGINE
seams/crates/swap/src/lib.rs -> L-ENGINE
seams/crates/engine/src/models/mod.rs -> L-ENGINE
seams/crates/engine/src/models/gateway.rs -> L-ENGINE
seams/crates/engine/src/ledger.rs -> L-ENGINE
seams/crates/engine/src/real_core.rs -> L-ENGINE
seams/crates/thread10/src/pipeline.rs -> L-E2E
seams/crates/thread10/src/requests.rs -> L-E2E
docs/journal/0220-r1e-model-port-and-viability-ledger.md -> L-ENGINE
seams/crates/pg/src/lib.rs -> L-PG
seams/xtask/src/main.rs -> L-ENV
docs/reviews/codex/rev1.md -> X-REV
docs/reviews/claude/crv0.md -> L-GOV
docs/spec-amendments/codex/pl-c6.md -> X-DOC
docs/spec-amendments/claude/px0.md -> L-GOV
docs/reports/gates/gt0.md -> L-GOV
docs/reports/e2e/run-01.md -> L-E2E
docs/runbooks/rb-01.md -> L-OPS
docs/security/threat-model.md -> L-OPS
docs/lanes/l-pg/notes.md -> L-GOV
scripts/env/baseline.ps1 -> L-ENV
scripts/gov/w0-receipt.ps1 -> L-GOV
scripts/triggers/agentcore_poller.py -> L-CAPI
scripts/dc/scan.py -> L-MODEL
scripts/verify-local-all.ps1 -> L-ENV
tests/verify-local-all.Tests.ps1 -> L-ENV
agent-core-assets/corpus/smap-01.json -> L-MODEL
agent-core-assets/eval-suites/flow.json -> L-EVAL
agent-core-assets/worlds/seed.json -> L-MODEL
migrations/0005_x.sql -> X-LEARN
migrations/0050_x.sql -> L-PG
migrations/0095_x.sql -> L-GOV
docs/adr/0100-x.md -> L-AUTH
docs/adr/0230-x.md -> L-MODEL
docs/adr/0300-x.md -> X-ARTIF
docs/adr/0395-x.md -> X-SRC
docs/journal/0100-l-auth-x.md -> L-AUTH
docs/journal/0405-x-artif-x.md -> X-ARTIF
docs/journal/0591-x-src-x.md -> X-SRC
```
