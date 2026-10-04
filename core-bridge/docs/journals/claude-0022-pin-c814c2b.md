# claude-0022: agent-core pin 894fa65 -> c814c2b (PR #30)

One consolidated package (WP1 pin/wire, WP2 mock/sim, WP4 assets, WP5 local/core + image). Decision record: ADR 0012.
Analysis: `docs/AGENT_CORE_PIN_BUMP_3_ANALYSIS_CLAUDE.md`. Branch `claude/r4-pin-c814c2b` off `claude/r3-annexd` (8643a2d).

## What changed
- **Mandatory fix, RED first.** `synthesise_args` built the argparse Namespace by hand; c814c2b `resolve_ports` reads
  `args.lang_thresholds`. RED (against the real c814c2b Core, venv `pulso-ci-venv-c814c2b`): `tests/runtime/test_bump_c814c2b.py`
  (3 tests: parser-attribute coverage, the real `serve_ports._lang_thresholds`, our overrides kept) and the existing PG test
  `test_pg_runtime.py::test_composed_app_ready_and_version`, all `AttributeError: 'Namespace' object has no attribute
  'lang_thresholds'`. Fix: start from the defaults of Core's real `serve` parser, overlay ours, `lang_thresholds=None`.
  GREEN: 21 passed (new + `test_pg_runtime.py` + `test_bump_789d6c8.py`).
- **Pin constants** moved in 31 files (runtime `PIN_SHA`, exporter config, gen-wire/ci/build-image/Dockerfile `CORE_SHA`,
  `gen_wire.py` fallback, local/core scripts + Pester, assets `pin.sha`, platform-sim paths/README, bridge-contract).
  Wire dir and platform-sim fixture dir renamed with `git mv`.
- **Wire** regenerated: 250 files, byte-identical except MANIFEST lines 1028/1030; digest
  `68f335f27cae0fa713c90537066bdc1d88b595e417312f75ca1cb967bffb876a` (reproduced). `gen-wire.ps1 -Check`: no drift.
  `bridge-contract/gen.py`: pin string only (contract.json, openapi, README, one example); goldens not re-recorded.
- **Expand/contract test** now OLD=894fa65 / NEW=c814c2b: no SQL script differs, equal fingerprint, clean rollback.
- **Reference checkout** `references/agent-core-c814c2b` (clean `git clone` at the pin; other reference dirs untouched).
- **Image** `localhost/pulso-core-runtime:c814c2b-a44003e` (Podman `pulso-dev`).

## Verification (throwaway PG16 on pulso-dev, `--cgroups=disabled`, 127.0.0.1, PULSO_REQUIRE_POSTGRES=1; pinned CI venv)
- lint: 3 ruff jobs pass. `gen-wire.ps1 -Check`: no drift. modules-scan: 1 passed.
- agent-core-assets: `assetcheck check`/`validate` ok, 22 passed (no digest or `release_ids` change).
- platform-sim (core-bridge pyproject): mock 246 passed / 2 skipped, a2 245 / 3, real 208 / 40 (skips: `sqlglot` not
  installed, `requires_sim`, `mock_only`).
- core-bridge full: 654 passed / 1 skipped (with `PULSO_TEST_IMAGE` set; `test_image.py` alone 24 passed;
  `test_expand_contract.py` 4 passed against real 894fa65 + c814c2b toolchains).
- bridge-contract: real 337 passed; mock 7 passed / 29 skipped / 149 xfailed (known divergences).
- wire parity (`test.ps1`): mock 186 / 1 skipped, a2 185 / 2 skipped.
- local/core: Pester 3.4 100 passed / 0 failed / 1 skipped (live, needs PULSO_LIVE=1); python 20 passed (PYTHONPATH=core-bridge/src).
- e2e-core: unit 25 passed; live against the new image 31 passed / 13 failed / 1 skipped, see below.
- demo/run.ps1 real_local against the new image: outcome ok (`live_real_local_core`).
- Not reproduced: real-wire job (needs an externally running `agent-core serve`) and the Rust jobs.

## e2e-core live failures are NOT from the pin
The stand-in (`e2e-core`) was last aligned before the Annex D commits of r3 (6332305 `job_id` claim on invoke, da9b67d
server-derived admission ref / Z deadline / hex digests / ArmRequest names). First run: 9 failed, 31 errors, all
`pulso:auth_denied job_mismatch`. Fixed here (one line, `Bridge.invoke` signs `job_id` = body job): 13 failed remain, all in
the evaluation admission / arm DTO (422 `invalid_request`) and the human-approval chain that builds on it. Updating the
stand-in DTOs to Annex D is a separate package; the stack itself boots and the runtime starts on c814c2b (the
`lang_thresholds` fix is what lets it start).

## Not done / open
- Language switching by detection stays off (no `PULSO_LANG_THRESHOLDS` knob); decide separately.
- `RoutedTools` is Core's `run_serve` only; we do not use it.
- Relay items for the agent-core team are in the final report of this package (tolerant `getattr` for embedders, reusable
  builder-tools composition, fixture id reuse between registry-demo and registry-e2e, distinct problem code for in-flight
  `idempotency_conflict`).
