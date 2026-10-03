# local/core

Standalone local stack for the real Core runtime on a registered Claude Podman machine (default `pulso-dev`), plus
includable fragments for Codex's stack. Contract revision: `pulso-two-teams-1`. Agent Core pin
`86a767474042a566a0dbd6ed23588959f27ebdb3`. Never touches machine `pulso-codex` or any Codex-owned resource; machine and namespace
names are validated (`machine.registry.json`, namespaces prefixed `claude-`).

## Contents

| Path | Purpose |
|---|---|
| `compose.core.yaml` | includable model: `core-postgres`, `core-migrate`, `core-grants`, `core-keygen`, `core-seed`, `core-synth`, `core-runtime`, `core-exporter`, `platform-sim` (doubles). No host ports, no top-level secret values; service-scoped env by name. |
| `compose.standalone.yaml` | overlay that publishes loopback ports derived and probed by `start.ps1` (never fixed 5432/8000). |
| `start.ps1`, `stop.ps1`, `reset.ps1`, `smoke.ps1`, `doctor.core.ps1` | lifecycle, honest smoke report, diagnostics (`-Json`). |
| `lib/` | `errors`, `machine`, `namespace`, `ports`, `secrets`, `memory`, `evidence`, `runner` (see ADR `core-bridge/docs/adr/0006`). |
| `init/` | roles/DB SQL, exporter and eval grants, seed state, `gen_keys.py` (bridge identity/staff/callback/**executor** signers + `lab-broker-trust.json`), `seed_assets.py`, `probe_version.py`, `synth_chain.py`. |
| `profiles/*.env.example` | placeholders only; real values are generated into git-ignored `local/.secrets/<ns>/`. |
| `fragments/` | `ci.fragment.yml` (one job per `core-bridge/scripts/ci.ps1 -Job`: rust, contract-drift, mock-wire, a2-wire, real-wire, lint, core-bridge, platform-sim, agent-core-assets, modules-scan), `otel-collector.yaml`, `include.fragment.yaml`, declarative `patch.json` for Codex-owned files. |
| `manifests/local-service-manifest.json` | generated from the compose model (`gen-manifest.ps1`). |
| `doubles/` | `Dockerfile` and `run_doubles.py`: registry mock, bridge mock and ingest fixture in one container (doubles). |
| `tests/` | Pester (`*.Tests.ps1`) and `test_seed_assets.py`. |
| `../../scripts/core/` | thin wrappers for Codex's aggregator and `verify-fragments.ps1`. |

## Use

    pwsh local/core/start.ps1 -Profile real_local -DryRun            # plan only, no engine call
    pwsh local/core/start.ps1 -Profile real_local -Machine pulso-dev  # fixture | real_local
    pwsh local/core/smoke.ps1 -Namespace claude-<id> -Exec
    pwsh local/core/doctor.core.ps1 -Json
    pwsh local/core/stop.ps1 -Namespace claude-<id>

Startup order: postgres -> migrate (`migrate --app-role core_app`) -> grants -> seed (five releases must equal
`agent-core-assets/manifest.yaml`; a second start writes nothing) -> runtime `/readyz` -> exporter.
Exit codes: 2 machine/namespace, 3 insufficient memory, 4 port conflict, 5 core not ready, 6 migrate failed, 7 postgres
unavailable, 8 runtime cgroup unavailable, 9 seed failed, 10 demo doubles active.

`smoke.ps1` derives the evidence target: `real_local` only when `/_sim/info` is absent, the reported sha equals the pin, the
image digest equals the expected one and `/readyz` passes; otherwise `evidence_target_unproven`. Steps that cannot run are
`not_run` with a reason, never `pass`. In `real_local` the broker, control-api and ingest are **doubles** (`platform-sim`).

## Executor key (A03 iii)

The runtime requires a separate executor keypair (`PULSO_BRIDGE_EXECUTOR_SIGNER`, default `bridge-executor.json`) and fails
closed at startup if it is missing or equals the callback key. `core-keygen` writes it with a distinct kid and also writes
`lab-broker-trust.json` (its PUBLIC half, `iss=core-bridge`, `aud=lab-broker`), which `platform-sim` (the lab-broker double)
mounts read-only and lists in `/_sim/info` (`lab_broker_trusted_kids`). `doctor.core.ps1` (`bridge_executor_key`) and
`smoke.ps1 -Exec` (`executor_key`) verify presence, distinctness and trust without printing key material. Keys are
generated into the `core-keys` volume only, so `reset.ps1` rotates them. The image entrypoints are exactly
`runtime | exporter | migrate | agentcore`; seeding is the `core-seed` job (`python /init/seed_assets.py`).

## Podman cgroup workaround

`pulso-dev` cannot start containers with default cgroup handling (`pids` controller). Services carry
`com.pulso.runner.*` labels and `lib/runner.ps1` creates them with `--cgroups=disabled --pids-limit=0`. See ADR 0006.

## Tests

    Invoke-Pester local/core/tests            # Pester 3.4
    python -m pytest local/core/tests         # test_seed_assets.py, test_gen_keys.py

## Known gaps

The scripted-task smoke step was `not_run` when the stack was first delivered (needed invoke wiring and a model script);
`AGENTCORE_JEV_API_KEY` is a local placeholder; root `local/compose.yaml` inclusion and a non-internal `core-egress` network
are Codex-owned and unverified. Details: `core-bridge/docs/journal/claude-0008-l8-local-stack.md`.
