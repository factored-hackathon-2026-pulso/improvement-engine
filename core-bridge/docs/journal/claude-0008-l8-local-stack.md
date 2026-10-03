# Journal claude-0008: L8 standalone Core local stack

Contract revision: `pulso-two-teams-1`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3`. Package: L8 (plan 17.3.8).
Directories: `local/core/**`, `scripts/core/**`. Commits: 85f2a66, 3ff1c98 (review fixes), 016086b (composed kill -9 test).
Documentation pass at HEAD `4984d92`. ADR 0006 explains the Podman cgroup workaround.

## Purpose
Run the real Core runtime, its Postgres and the exporter on a registered Claude Podman machine, with platform doubles for
what Codex owns, and with an honest smoke report that never claims more than was proven.

## Flow
`start.ps1 -Profile real_local|fixture [-Namespace claude-<id>] [-Machine pulso-dev] [-PortBase N] [-Image ref] [-DryRun]`:
register/ownership checks (`machine.registry.json`, `Assert-ClaudeNamespace`), memory budget, port plan (loopback
only, derived and probed, never fixed 5432/8000), generate secrets and keys into the git-ignored
`local/.secrets/<ns>/`, render the compose model, create containers through `lib/runner.ps1` (ADR 0006), then run
`core-postgres` -> `core-migrate` (`migrate --app-role core_app`) -> `core-grants` (exporter, seed-state and eval grants) ->
seed (`init/seed_assets.py`, five releases equal to the manifest release ids, marker digest makes a second start write
nothing) -> `core-runtime` (`/readyz`) -> `core-exporter`; `smoke.ps1`, `stop.ps1`, `reset.ps1`, `doctor.core.ps1 -Json`.

## Input / output
In: namespace, profile, pinned image (digest-checked). Out: `local/.secrets/<ns>/{core.env,state.json}`, ports, evidence
reports (target derived: `real_local` only when `/_sim/info` is absent, sha equals the pin, the image digest equals the
expected one and `/readyz` passes; otherwise `evidence_target_unproven`), `manifests/local-service-manifest.json`
(generated from the compose model). Fragments for Codex without editing Codex files: `compose.core.yaml` (includable),
`fragments/{ci.fragment.yml,otel-collector.yaml,include.fragment.yaml,patch.json}`, `scripts/core/*.ps1` wrappers and
`verify-fragments.ps1`.

## Transactions and idempotency
Seed is idempotent by marker digest; `start.ps1` on an existing namespace reuses its ports (not a conflict with itself);
namespace ownership is enforced.

## Errors
Typed exit codes (`lib/errors.ps1`): 2 `machine_not_registered|namespace_not_owned|confirm_required`, 3
`insufficient_memory`, 4 `port_conflict`, 5 `core_not_ready`, 6 `core_migrate_failed`, 7 `core_postgres_unavailable`, 8
`runtime_cgroup_unavailable`, 9 `core_seed_failed`, 10 `core_demo_doubles_active`. Doctor also reports
`core_checkout_wrong_sha`, `core_toolchain_missing`, `contracts_drift`, `assets_drift`, `core_unreachable_from_stack`.

## Permissions
Only machines in `machine.registry.json` and namespaces prefixed `claude-` are touched; `pulso-codex` and any Codex-owned
resource are never used; secrets are generated per namespace and never printed; roles `core_app`, `core_eval_app`,
`exporter_ro` with separate DSNs; host ports bind 127.0.0.1.

## Config
Profiles `local/core/profiles/{fixture,real_local}.env.example` (placeholders only), `PULSO_CORE_IMAGE`,
`PULSO_SIM_IMAGE`, `PULSO_NS`, `PULSO_PROJECT`, `PORT_*`. In `real_local` the broker, control-api and ingest are doubles
(`platform-sim`) until Codex's assembly provides them; `AGENTCORE_JEV_API_KEY` is a local placeholder.

## Observability
Evidence JSON per run (commands, suites with `pass|fail|not_run` and reasons), observed image digest, `doctor -Json`.

## Commands (head `4984d92`)
- `pwsh local/core/start.ps1 -Profile real_local -DryRun` (plan only, no engine)
- `pwsh local/core/start.ps1 -Profile real_local -Machine pulso-dev`, `smoke.ps1 -Namespace <ns> -Exec`, `stop.ps1`, `reset.ps1`
- Pester: `local/core/tests/*.Tests.ps1`; pytest: `local/core/tests/test_seed_assets.py`
Environment: Windows 11, PowerShell 7, Podman machine `pulso-dev` (rootless), `docker-compose` for model rendering.

## RED / GREEN
Reported in BITACORA (2026-10-03 14:59 UTC): fixture and real_local profiles reached healthy on `pulso-dev`; keygen ->
migrate -> grants -> seed (5 releases equal to the manifest) -> `/readyz` 200 -> exporter healthy; second start seeded nothing;
smoke derives `target=real_local` only with a matching image digest; Pester 62 and pytest 7 passed. The first RED is not
named in the entry. Not re-run in this documentation pass (no engine started).

## Trade-offs
Runner script instead of plain compose (ADR 0006) keeps the compose file includable by Codex but adds a second source of
truth for container flags. Platform doubles keep the stack standalone but mean `real_local` evidence is about Core only.

## Gaps
- Scripted task step reported `not_run` at the time (needed invoke wiring plus a model script).
- Runtime image lacked `jsonschema`/`referencing` at the time; a later commit (4984d92) added them to
  `runtime-requirements.txt` and the Dockerfile; the platform-sim overlay image also adds them.
- `core-bridge/scripts/ci.ps1` did not exist when the CI fragment was written; it exists at `4984d92`.
- Root `local/compose.yaml` inclusion and `core-egress` non-internal network are Codex-owned and unverified (`include`
  support and host publishing on internal networks to be verified).
