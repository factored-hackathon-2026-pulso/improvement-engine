# Journal 0009 (Claude): agent-core pin bump 86a7674 -> 789d6c8

Branch `claude/r-runtime-wire`. Decisions: ADR `docs/adr/0008-agent-core-pin-789d6c8.md`. Analysis:
`docs/AGENT_CORE_PIN_BUMP_ANALYSIS_CLAUDE.md`.

## Objective
Move the pin to agent-core `789d6c89b2fca90fc10e2abf157da51dc81c5d51` (N-01..N-11, PR #26 llm-gateway, PR #27 AWS scale-out)
without regressing any suite, regenerate the wire snapshot, adopt the new Core surface explicitly, and prove old/new
binary compatibility with the (unchanged) schema.

## Steps and evidence (commits in order)
1. `2c1bc83` pre-stage: literal tracer name `agent_core.adapters.llm` (works at both SHAs; PR #26 removes `GATEWAY_TRACER`).
2. `a0e8218` mechanical SHA/paths in ~40 files (`wire/agent_core@789d6c8`, fixtures dir, venv name, image tag), `gen_wire.py`
   copies `contracts/registry/`, `contracts/VERSION` is a note not a gate. Regenerated wire: 194 schemas + 2 events + 31
   registry schemas, new `schemas/RunSummary.json`; golden hashes byte-identical.
3. `5a606b8` runtime composition: `build_sha`, explicit `keys_reload_seconds`, export off by default, `problem_response`,
   compat additions, `keys_reload_error` in `/internal/v1/version`. First RED: `tests/runtime/test_bump_789d6c8.py`
   (`keys_reload_seconds` did not exist) and the PG-gated `test_bump_789d6c8_pg.py` (export routes 404 by default, `/version`
   sha, reload resilience).
4. `7ff61d9` platform-sim: mock + a2 + parity for N-02/N-03/N-10; fixtures re-recorded (a2, `real_local`,
   `real_pg_scripted`); evidence JSONs regenerated; golden vectors carry `release_detail`.
5. `5d38af9` expand/contract smoke (`tests/integration/test_expand_contract.py`, CAP-57).
6. release_settings default deny in `ProtectedBuilderToolExecutor` (`tests/l3b/test_release_settings_denied.py`; RED then GREEN).

## What broke / changed at the new SHA (measured)
- `GATEWAY_TRACER` import (PR #26): fixed by literal.
- Parity: 14 case failures + route table on a2 before the mock/fixture update, all N-02/N-03/N-10; none unexplained.
- `build_api_deps` now auto-mounts `/v1/export/*` (needs `run_export` + staff verifier, both present) and `/version`: export
  nulled by default, `/version` fed with the build sha.
- Silent default: key reload every 5 s; now explicit.
- Release ids of `agent-core-assets` did NOT change (`expected-state.json` byte-identical), because entity hashes and
  `release_hash` inputs are unchanged without a `release_settings` draft.

## Doubles
Mock: hand-written registry mock (no `release_settings`, `/version`, export). a2: real `RegistryService` over the in-memory
store. `real_local`: real service over PG16 with a fixed-pass EvalPort and sim staff key. `real_pg_scripted`: same with a
scripted EvalPort + fake clock. Expand/contract: real binaries of both pins over PG16; our own bridge schema not included.

## Gaps
- `release_settings` (N-07) is not simulated by the mock and has no parity case at all (a2 accepts it; no recorded cases).
- No test with `HttpLLMGateway` through `SpendMeteringGateway` (needs an llm-gateway double); deployment needs the new env.
- Upstream HTTP export PG queries (`list_runs`, `read_after`) are not exercised (export off).
- GitHub Actions budget unknown: results are local only.
