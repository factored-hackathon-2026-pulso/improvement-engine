# Journal claude-0016: WP5 stack bump to agent-core 894fa65 (our paths only)

- First RED: `tests/runtime/test_image.py::test_dockerfile_core_sha_args_equal_the_runtime_pin` (Dockerfile ARGs 789d6c8);
  Pester `Grants.Tests.ps1` "Core rate-limit knobs" (compose lacked them).
- Pins moved to 894fa65 in Dockerfile, local/core (README, doctor, evidence, smoke, tests). `build-image.ps1` was already bumped.
- Image `localhost/pulso-core-runtime:894fa65-c3616d4` built with Podman pulso-dev from a scratch clone at 894fa65
  (`references\agent-core-894fa65` does not exist; the doctor's default-checkout Pester test fails until it does; doctor passes with `-Checkout`).
  digest sha256:f563d1dd8ade3a9ddeb98d689016cf4add11f88d25bcf4e6901a94be3a02be30.
- `test_image.py` with PULSO_TEST_IMAGE: 12 passed. Pester local/core: 89 passed, 1 failed (above), 1 skipped.
- e2e-core/run.ps1: 16 unit + 33 live passed, 1 skipped (bank lost-response needs stateful scenario). Torn down.
- Compose now forwards AGENTCORE_RATE_MAX_HITS=30, _RATE_WINDOW_SECONDS=60, _DAILY_BUDGET_USD=5.00, _RATE_SERVICE_MULTIPLIER=10
  (Core defaults; read by `limits_from_env`). Idempotency lease (reserved_until) and the release_settings admin gate have no env knob in
  Core; nothing to configure on our side (the bridge does not write interrupts).
- demo/run.ps1: outcome ok, live_real_local_core; no hard-coded release ids in demo/e2e-core/local.
- gen_wire test docstring no longer carries the old id.
