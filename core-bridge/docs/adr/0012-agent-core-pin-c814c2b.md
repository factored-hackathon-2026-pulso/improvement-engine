# ADR 0012: Bump the agent-core pin 894fa65 -> c814c2b (PR #30)

Status: proposed (implementer: Claude; reviewer and integrator approval pending). Analysis:
`docs/AGENT_CORE_PIN_BUMP_3_ANALYSIS_CLAUDE.md`. One consolidated package: pin constants, wire, bridge contract,
assets, mock/sim, local/core, image.

## Context
`c814c2bad9f154d10c092326558815dca9562be7` is six commits (PR #30) on top of 894fa65. Under `agent_core/` only
`composition/{builder_tools,serve,serve_ports}.py` changed; `contracts/`, migrations, `api/`, `registry/`, `domain/`,
`uv.lock` and the Dockerfile are untouched. New upstream surface: `--lang-thresholds` / `AGENTCORE_LANG_THRESHOLDS`
(`ServePorts.lang_thresholds`), and `RoutedTools` (builder tools in `run_serve` only).

## Decisions
1. **Wire is byte-identical except the two sha lines.** `wire/agent_core@c814c2b/` was `git mv`-ed from
   `agent_core@894fa65/` and regenerated with `gen-wire.ps1`: 250 files, 0 schema / registry / event / derived / golden /
   openapi differences, 0 routes added or removed. The only differences are `MANIFEST.json` lines 1028 (`sha`) and 1030
   (`tool_versions.agent_core_checkout_sha`).
   **MANIFEST digest: `68f335f27cae0fa713c90537066bdc1d88b595e417312f75ca1cb967bffb876a`** (regenerated and verified;
   the 894fa65 digest was `ed000b81...809b5`). `contracts/agent_core/pin.json` still does not exist in this tree; whoever
   creates it records this digest.
2. **Mandatory code fix: `synthesise_args`.** `resolve_ports` now reads `args.lang_thresholds` by attribute. Our
   `synthesise_args` built the Namespace by hand, so the real runtime would have died with `AttributeError` at startup
   (only PG/real-Core tests see it; non-PG tests stub `resolve`). The Namespace now starts from the defaults of Core's
   real `serve` parser (`add_serve_parser`) and overlays our values, with `lang_thresholds=None` explicit. Any future
   flag Core adds therefore defaults correctly, and a test compares our Namespace to the real parser's attribute set.
   Dual-pin safe: an extra attribute is ignored by 894fa65.
3. **Language switching by detection stays off** (no `PULSO_LANG_THRESHOLDS` knob yet; separate decision). Same
   behaviour as before the bump.
4. **`RoutedTools` is not used.** Our `main` composes `resolve_ports` + `build_api_deps` itself with our own
   `PulsoToolDispatcher` / `ProtectedBuilderToolExecutor`; `BuilderToolExecutor` and `BUILDER_TOOL_DEFS` are unchanged.
   Nothing added to `compat.PIN_SYMBOLS` (`assert_compat` stays valid on both pins; same dual-pin rule as ADR 0010 #3).
5. **Expand/contract.** `test_expand_contract.py` is now OLD=894fa65 / NEW=c814c2b: no SQL script differs, the
   fingerprint is equal, rollback is clean at this step. The `Interrupt.locked` rollback hazard of ADR 0010 is a property
   of 894fa65 and not of this bump.
6. **Fixtures.** `platform-sim/fixtures/agent_core_wire/894fa65` -> `c814c2b` by rename only (wire and routes are
   identical, no re-record). Assets: only the `pin.sha` fields move (`release_ids` unchanged, seed untouched).
7. **Reference checkout** `references/agent-core-c814c2b` is a clean `git clone` at the pin; the older reference dirs are
   untouched. Venv for the pin: `%TEMP%\pulso-wire-venv-c814c2b` (gen-wire) and `%TEMP%\pulso-ci-venv-c814c2b` (CI).

## Consequences
- Rollback to 894fa65 is safe (no schema/data change, wire identical).
- Deploy: no migration step is needed for this bump; roll the image.
- Open for the agent-core team: tolerant `getattr(args, "lang_thresholds", None)` or a documented minimal args contract
  for embedders; whether `RoutedTools` becomes reusable; fixture id reuse between `registry-demo` and `registry-e2e`.
