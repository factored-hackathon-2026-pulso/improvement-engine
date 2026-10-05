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

Image build: `not_exercised` when this file was authored (a cargo process was running on the author machine, so the heavy release build was skipped). Build it before the first release.
