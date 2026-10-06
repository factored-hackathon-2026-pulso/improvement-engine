# Engine container image

The root `Dockerfile` builds the improvement-engine image: one `pulso` executable (plus `pulso-synth-runner`, which
`pulso` spawns next to itself) and the static debug console. Build context is the repo root, the path infra expects
(`aws-prod.ps1` service `pulso-engine`: `Dockerfile`, context `.`; `scripts/release-engine.ps1` takes `-Dockerfile`).
The infra template `docker/pulso.Dockerfile` is only a fallback; this file supersedes it.

## Stages

1. `console`: `npm ci`, `tsc --noEmit`, `vite build` of `debug-console` -> `/opt/pulso/console`.
2. `build`: `cargo build --release --locked -p pulso --bins` from `seams/` (the crate's `build.rs` embeds
   `migrations/*.sql`, so the image has no migrations directory).
3. runtime: `debian:bookworm-slim` + `ca-certificates`, user `pulso` uid/gid 10001, no shell login, no compiler.

Base images are tag-pinned. Pin by digest before a production release (`podman image inspect`), as
`debug-console/Dockerfile` does.

## Runtime contract

- `ENTRYPOINT ["/usr/local/bin/pulso"]`, `CMD ["run"]`. Other subcommands: `docker run IMAGE serve ...`, `monitor`,
  `healthcheck`.
- `HEALTHCHECK` runs `pulso healthcheck` (GET `/readyz` on 127.0.0.1, port from `PULSO_LISTEN_ADDR`, prefix from
  `PULSO_BASE_PATH`). It needs no curl or wget. Build with `podman build --format docker`, otherwise the OCI format
  drops HEALTHCHECK. The compose bundle in infra (`deploy/hackathon/engine/compose.yaml`) also declares it.
- Configuration is environment only. Baked, non-secret defaults: `PULSO_LISTEN_ADDR=0.0.0.0:8080`,
  `PULSO_ALLOW_NON_LOOPBACK=1`, `PULSO_CONSOLE_DIR=/opt/pulso/console`, `PULSO_WORK_DIR=/var/lib/pulso/work`,
  `PULSO_STORE_DIR=/var/lib/pulso/store`.
- Supplied at run time, never in the image: `PULSO_DATA_MODE`, `PULSO_DATABASE_URL` (or `PULSO_STORAGE=memory`),
  `PULSO_DEBUG_TOKEN` and `PULSO_ADMIN_TOKEN` (mandatory on the non-loopback bind: >= 24 characters, different),
  `PULSO_CORE_URL`, `PULSO_STORAGE_PREFIX`, and the rest listed in `pulso run --help`. Without them the container
  exits with code 2 and a named reason; it never serves unauthenticated.
- Writable paths: `/var/lib/pulso` (volume) and `/tmp`. Compatible with a read-only root filesystem.
- Port 8080.

## Build and check

```
podman build --format docker -f Dockerfile -t pulso-engine:local .
python -m unittest scripts/dev-stack/tests/test_engine_dockerfile.py
```

The unit test checks the file only (non-root, no secret-like ENV/ARG, healthcheck, entrypoint, `.dockerignore`).
It does not build. The full release build is heavy: run nothing else meanwhile (see `LOCAL_STACK.md`, Podman
machine `pulso-dev`).

## Status

Image build: `exercised` on Podman machine `pulso-dev` (4 GiB), `--format docker`, `--build-arg CARGO_BUILD_JOBS=1`
(the Dockerfile `build` stage now has `ARG CARGO_BUILD_JOBS=1`; raise it on bigger machines).

- Build: succeeded first try, 353 s wall (cold layers for console + cargo), no OOM. The sibling dirs copied for
  `include_str!` (`migrations`, `contracts`, `bridge-contract`, `platform-contract`) were sufficient.
- Size: 98.1 MB.
- `podman run --rm IMG --help`: prints the `pulso` usage (exit 0). Runs as uid 10001.
- `pulso run` with no env: refuses to start, `config_missing: PULSO_DATA_MODE is required`, exit 2.
- `pulso healthcheck` with nothing listening: exit 1 (`cannot reach readyz: ConnectionRefused`).
- With `PULSO_STORAGE=memory`, `PULSO_DATA_MODE=dataset` and test tokens (no database): the engine starts
  (monitor + worker tasks, stub adapter) and `pulso healthcheck` exits 0. Not exercised: Postgres-backed mode.
- On the pulso-dev rootless runtime, `podman run` needs `--pids-limit=0` (cgroup `pids` controller unavailable);
  `podman build` does not take that flag.

## Cells aggregator in the image (CELLS-AUTO)

The runtime image also carries `scripts/aggregate/bank_cells.py` at `/opt/pulso/aggregate/bank_cells.py` (one stdlib file; needs the `python3` of the loop image, PR 130). The AWS loader (infra `deploy/hackathon/engine/loader/run-bank-cells.sh`, `docs/auto-loader.md`) runs it right after the upload, with `--entrypoint python3`, `--network none`, no credentials, a read-only root and a memory cap: `python3 /opt/pulso/aggregate/bank_cells.py --data-root /in --out /out/cells.ndjson --k 10`. `/in` is the loader's copy of the six tables and two reference files under `landing/bank/`; the output is gated (`check_cells_k.py`) and reaches the engine through the inputs mirror (`PULSO_LOOP_INPUTS_DIR`, `cells.ndjson`). Contract test: `scripts/aggregate/tests/test_bank_cells_loader_contract.py`. Not run on real data here.
