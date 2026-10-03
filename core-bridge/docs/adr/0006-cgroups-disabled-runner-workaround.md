# ADR 0006: `--cgroups=disabled` runner for Podman machines without the `pids` controller

Status: accepted as a local-development workaround (L8, commit 85f2a66; observed 2026-10-03). Not a production design.

## Context
On the Claude Podman machines `pulso-dev` (rootless) and `pulso-dev-root`, starting a container with the default cgroup
handling fails with `crun: controller 'pids' is not available` (BITACORA L2 and L8 entries; verified rootless and
rootful). `compose` cannot express `--cgroups=disabled` or `--pids-limit=0`, and `docker-compose config` drops `x-*`
extension keys, so the flags cannot be carried as extensions either. The same limitation applies to the throwaway PG16
containers used by the test suites.

## Decision
1. Every service in `local/core/compose.core.yaml` carries labels `com.pulso.runner.cgroups=disabled` and
   `com.pulso.runner.pids_limit=0` (labels survive `config`). `com.pulso.runner.copy` lists `{src,dest}` files copied
   into the created container (a remote engine cannot bind-mount Windows paths).
2. `local/core/lib/runner.ps1` renders the model (`docker-compose config --format json`), computes the start order
   from `depends_on`, and creates each container with `podman create --cgroups=disabled --pids-limit=0 ...` (name
   `<project>-<service>-1`, network `<project>_core-net`, aliases, env, volumes, loopback ports, memory, healthcheck,
   entrypoint), then copies files and starts it.
3. A machine with working cgroups can ignore the labels and run plain compose with the same file.
4. `mem_limit` is declared but only enforced where the memory controller works; `doctor.core.ps1` reports
   `memory_limit_unenforced`, and the typed failure `runtime_cgroup_unavailable` (exit 8) exists for the case where the
   workaround cannot be applied.
5. Test suites that need Postgres use the same flag by hand: a throwaway PG16 container on `pulso-dev` with
   `--cgroups=disabled`, a unique name and port per agent, bound to 127.0.0.1, removed afterwards; the DSN goes in
   `PULSO_TEST_PG_ADMIN`.

## Consequences
No pid accounting limit on these containers (and no CPU/memory limit where the controller is missing), so a runaway
process is not bounded by the engine; acceptable for local, throwaway stacks only. Machine names are validated against
`local/core/machine.registry.json` (`machine_not_registered` otherwise); Codex-owned machines are never touched.
