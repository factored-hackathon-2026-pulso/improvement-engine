# PL-0008: platform-exporter container image

- `platform-exporter/Dockerfile` (python 3.12-slim, 2 stages, deps only from `uv.lock` via `uv export --frozen --no-dev`,
  uid 10001, read-only-root friendly), `docker-entrypoint.sh`, `.dockerignore`.
- Pre-created 0700 uid 10001: `/run/pulso-keys` and `/var/lib/pulso-platform-exporter` (Fargate ephemeral volumes inherit
  ownership from the image path). `EXPORTER_STATE_PATH` defaults to `/var/lib/pulso-platform-exporter/state.sqlite`.
- Entrypoint reuses the core-bridge ADR 0009 pattern: materialise `PULSO_EXPORTER_KEY_CONTROL_API_SEED` into
  `/run/pulso-keys/exporter-control-api.key` (0400, app uid), validate b64url 32 bytes, fail closed with exit 2 and
  `pulso:runtime_config_invalid` naming the variable (never a value), export `PULSO_EXPORTER_KEY_CONTROL_API`, unset the seed.
  File mode (path var set, no seed) is accepted.
- RED first: `tests/test_image.py` static tests failed (no Dockerfile); then 5 static + 7 container tests green
  (`PULSO_TEST_IMAGE=localhost/pulso-platform-exporter:2323c04`, Podman `pulso-dev`).
- Smoke (throwaway postgres:16 on pulso-dev, simulator scenario via the pg test DDL, least-privilege role, read-only
  root fs, fresh volume, `--network host` because rootless pasta has no container IPs): the container read Postgres and
  delivered to the ingest fixture double (2x observations 202, cursor 200, artifact 201); state.sqlite written on the
  volume owned by 10001. Containers/volume removed afterwards.
- Name mismatch with infra branch claude/u-infra-bridge-exporters: the infra module injects
  `PULSO_EXPORTER_KEY_CONTROL_API_SEED`, `PLATFORM_DB_URL`, `PULSO_CONTROL_API_URL`, `PULSO_TENANT_ID`, `PLATFORM_INSTANCE`,
  `PULSO_BINDING_REF`, `EXPORTER_STATE_PATH`: all consistent with the code. Gap: the exporter Python does not consume the
  key yet (no JWT minting, PL-L5; `__main__` reads only the optional `PULSO_SERVICE_TOKEN`), so the seed is materialised
  and validated but unused; the lab-broker audience (artifacts route) has no seed in the infra secret.
