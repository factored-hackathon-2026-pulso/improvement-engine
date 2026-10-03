# claude-0010: local CI parity run

Head sha at run time: `c6b8ddd` (worktree improvement-engine-claude-r; core-bridge/platform-sim/agent-core-assets paths
at `744f13f` plus later commits of other agents). Windows 11, Python 3.12 via uv, venv `%TEMP%\pulso-ci-venv-789d6c8`,
throwaway Postgres 16 (podman machine `pulso-dev`, `--cgroups=disabled`, 127.0.0.1:55471, removed afterwards),
`PULSO_TEST_PG_ADMIN` set so `PULSO_REQUIRE_POSTGRES=1` makes a skipped PG test a failure. One job at a time:
`pwsh core-bridge/scripts/ci.ps1 -Job <job>`.

| Job | Result | Counts |
|---|---|---|
| lint | PASS | ruff E4,E7,E9,F (ignore F811) on core-bridge, platform-sim, agent-core-assets |
| contract-drift | PASS | 250 files, `manifest_sha256=890edd7aeea261061d3e0dad2268900f9dc52c15ebe0f76781f7e3bec35d6565` |
| mock-wire | PASS | 166 passed, 1 skipped |
| a2-wire | PASS | 165 passed, 2 skipped |
| core-bridge | PASS | 529 passed, 5 skipped (non-PG skips; PG skips would fail the run) |
| platform-sim | PASS | 206 passed |
| agent-core-assets | PASS | assetcheck check + validate OK, 22 passed |
| modules-scan | PASS | 1 passed (no `testing.*` module imported) |
| rust (`scripts/verify-local-ci.ps1`) | PASS | cargo fmt/clippy/test all ok (e.g. 136 passed, 1 ignored per lib target), Pester 15 + 5 passed |

## Not reproduced
- real-wire via an external `agentcore serve` (`REGISTRY_BASE_URL`): not run (needs identity/staff key files and a
  migrated registry DB). The real `RegistryService` over `PgRegistryStore` on PG16 is covered in-process by the
  platform-sim suite (`registry_mock/real_app.py`, `real_local`), which passed.
- Hosted-only: ubuntu leg of `verify`, `postgres:17@sha256` service container (`-PostgresTestUrl` not given), `actions/checkout`.
- mypy (`-Mypy`) was not requested.

## Lint note
The CI rule set does not include I001. Import order was fixed with `ruff --select I001 --fix` on the files touched by
recent work only (commit `744f13f`); about 100 older I001 findings elsewhere are untouched on purpose.
