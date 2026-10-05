# Quickstart: run the local loop on your own machine

For a teammate who has to run the Pulso demo (engine + agent-core + llm-gateway + support-platform + SPA) on a laptop that is not the one
the scripts were written on. Windows, Linux and macOS, always with PowerShell 7 (`pwsh`). Technical background:
[`INTEGRATED_RIG.md`](INTEGRATED_RIG.md), [`DEMO_LOOP.md`](DEMO_LOOP.md).

## What was actually run (read this first)

| Part | Status |
| --- | --- |
| Pester for DevConfig, doctor, rig, demo-loop, langfuse closure; unittest for the podman selector | run on Windows (pwsh 7 and Windows PowerShell 5.1) |
| `doctor.ps1` | run on Windows (read-only) |
| Linux and macOS paths (`/proc/meminfo`, `vm_stat`, `/bin/sh -c`, no `.exe`, native podman, `kill`) | only unit-tested with a mocked OS. NOT run on Linux, NOT run on macOS |
| `up.ps1`, `run_story.ps1`, `run.ps1` on a machine other than the original Windows one | NOT run (no stack was started for this change) |

Expect to fix small things on Linux/macOS; send them back as a PR.

## 1. Layout

Put the four checkouts side by side (names matter only for the defaults; any location works with `devconfig.env`):

```
work/
  improvement-engine/      this repo   (cwd for every command below)
  agent-core/              pulso-factored/agent-core        main
  llm-gateway/             pulso-factored/llm-gateway       main
  support-platform/        support-platform repo            main
  .pulso-env/
    agent-core.env         your credentials, created by you, never committed, never printed
    llm-gateway.env
```

The two env files hold the credentials of the model gateway and agent-core. Ask the Pulso team for the NAMES of the keys; the scripts only read the
files, mask every value in anything they print, and `doctor.ps1` only counts the keys.

Tools: `pwsh` 7, `podman`, `uv`, Python 3.11+ (`python` or `python3`) with PyYAML, Node 20+ (for the SPA), Rust (`cargo`) to build the engine.

## 2. Configuration (optional)

Resolution order for every setting: `devconfig.env` at the repo root (or the file named by `PULSO_DEVCONFIG`) > environment variable >
default relative to this repo (`../agent-core`, `../llm-gateway`, `../support-platform`, `../.pulso-env/*.env`, `./target`) > the original
machine's `D:\` layout. If you follow the layout above you need no file at all. Otherwise:

```
cp scripts/dev-stack/devconfig.example.env devconfig.env      # Windows: Copy-Item
# uncomment and set only what differs, for example:
#   PULSO_AGENT_CORE_DIR=/home/me/src/agent-core
#   PULSO_PODMAN_MACHINE=pulso-dev        (macOS/Windows: a ROOTFUL machine; connection pulso-dev-root. Linux: leave unset)
#   PULSO_RIG_PG_PORT=55511               (ports, if something else holds them)
```

`devconfig.env` is gitignored and accepts only the names in the example (paths, ports, podman names). Never put a key in it.

Podman: Linux uses native podman, no machine. macOS and Windows need a rootful machine:
`podman machine init --rootful --cpus 2 --memory 4096 pulso-dev && podman machine start pulso-dev`, then `PULSO_PODMAN_MACHINE=pulso-dev`.

## 3. Build the engine (once)

The Rust workspace is `seams/`. Keep the artifacts in `./target` of this repo (gitignored), one job at a time (the build is memory hungry):

```
# Linux / macOS (bash/zsh)
export CARGO_TARGET_DIR="$PWD/target"
(cd seams && cargo build -j 1 -p pulso -p steps)

# Windows (PowerShell)
$env:CARGO_TARGET_DIR = "$PWD\target"
Push-Location seams; cargo build -j 1 -p pulso -p steps; Pop-Location
```

The scripts then find `target/debug/pulso` (`pulso.exe` on Windows) and `target/debug/steps_cli`. Elsewhere: `PULSO_EXE` / `PULSO_STEPS_EXE` in `devconfig.env`.

## 4. Doctor

```
pwsh scripts/dev-stack/doctor.ps1
```

A table, one row per check (OS, pwsh, podman reachable, free RAM, python 3.11 and PyYAML, uv, node, pnpm, cargo, the pulso binary, the three
checkouts, the two env files, ports free) with a fix hint on every WARN/BLOCK row. Exit 1 while a BLOCK remains. It starts nothing.

## 5. The full cycle with the SPA (a person clicks)

Use ONE shell that stays open (agent-core runs as a child of it).

```
pwsh scripts/integrated-rig/up.ps1 -Spa          # postgres + gateway + agent-core + platform API + SPA on http://127.0.0.1:5174
pwsh scripts/integrated-rig/run_story.ps1 -Cycle # the engine proposes; then YOU click in the SPA: Aprobar, Publicar, Pasar a produccion
pwsh scripts/integrated-rig/health.ps1           # one OK/FAIL line per component
pwsh scripts/integrated-rig/down.ps1             # stop everything (-Purge also drops the volume)
```

`up.ps1` refuses to start with 1500 MB of free RAM or less (`-WaitRamMin N` waits, `-Force` skips). The engine never approves, publishes or promotes by itself.

## 6. The stand-alone demo loop

```
pwsh scripts/demo-loop/run.ps1 -Up                       # own stack, about 80 s the first time
pwsh scripts/demo-loop/run.ps1 -Cells -Loop -Show        # cells, the loop, the readable dossier
pwsh scripts/demo-loop/run.ps1 -Down                     # free the memory
```

Without the restricted bank data (the normal case outside the Pulso team) use the labelled SYNTHETIC planted cells:

```
pwsh scripts/demo-loop/run.ps1 -Cells -Loop -Show -Synthetic
pwsh scripts/integrated-rig/run_story.ps1 -Profile planted        # the rig default is already synthetic
```

Synthetic numbers are invented and say so in every report; do not present them as bank results.

## 7. Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `doctor`: podman not reachable | machine stopped, or wrong name | `podman machine start <name>`; set `PULSO_PODMAN_MACHINE`; Linux: unset it |
| `doctor`: free RAM BLOCK | less than 1500 MB free | close programs; `up.ps1 -WaitRamMin 10`; `-Force` only if you accept swapping |
| `pulso binary not found` | not built, or built elsewhere | section 3, or `PULSO_EXE` |
| `... checkout not found at <path>` | different location | `PULSO_*_DIR` in `devconfig.env` |
| `agent-core.env` BLOCK | file missing or empty | create it at the path shown (or `PULSO_AGENT_CORE_ENV`); never commit it |
| `python not found` on Linux/macOS | only `python3` exists | the scripts try `python3` too; install Python 3.11+ |
| port already in use | an earlier run, another app | `down.ps1`; or `PULSO_RIG_*_PORT` |
| agent-core dies right after `up.ps1` | `up.ps1` was started by a launcher that closes the shell | run it from an interactive shell that stays open |
| SPA step fails | no Node, or no network for the frozen lockfile install | Node 20+; check the proxy |
| `-Cells` says no cells table | no bank data | add `-Synthetic` |
