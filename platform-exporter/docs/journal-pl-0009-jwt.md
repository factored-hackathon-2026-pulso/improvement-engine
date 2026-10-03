# PL-0009: per-attempt service-JWT minting

- `platform_exporter/auth.py` mirrors the core-exporter signer (`core-bridge/.../exporter/auth.py`, ADR 0009, A03):
  EdDSA, `typ=JWT`, `iss=core-bridge`, `sub`=source binding, `tenant_id`, singular `scope`, `iat`, `exp` = iat+60 s
  (hard cap 300 s), fresh `jti` (uuid4) on every `token_for` call. `Exporter._post` already asks for a token per HTTP
  attempt, so retries never reuse a `jti`.
- Route map (plan A03 / 16.13.5): observations -> aud `control-api`, scope `observations`, purpose
  `platform_observations`; cursor read -> `control-api`, `observations`, no purpose; artifacts -> aud `lab-broker`,
  scope `artifact_write`, purpose `artifact_upload`. No `source_id`/`job_id` claim: the annex lists
  `iss,aud,sub,tenant_id,scope,exp,jti` for this class (job_id is only for Rust->bridge and executor->broker).
- Key: `PULSO_EXPORTER_KEY_CONTROL_API` file (0400, written by the entrypoint from the infra seed) is REQUIRED;
  missing/malformed/unset -> `RuntimeConfigInvalid` (`pulso:runtime_config_invalid: <VAR> ...`, exit 2, names the variable
  only). `PULSO_SERVICE_TOKEN` works only with `PULSO_DEV_STATIC_TOKEN=1` (dev override; set without the flag is rejected).
- DECISION, second key: the artifacts route needs `aud=lab-broker` (separate keypair per audience; the control-api key
  must not be reused, a token for one audience is rejected on the other). The infra platform secret has no lab-broker
  seed, so artifact uploads fail closed (`RuntimeConfigInvalid` naming the audience, process exits 2) until infra adds
  `PULSO_EXPORTER_KEY_LAB_BROKER_SEED`. The entrypoint/`__main__` already accept it as OPTIONAL
  (`PULSO_EXPORTER_KEY_LAB_BROKER[_SEED]`, same 0400 materialisation). core-bridge's exporter secret already carries it.
- Tests (`tests/test_service_jwt.py`, 21): claims/header per route, unique jti, exp window/ttl cap, no lab key -> closed,
  verification by the ingest fixture with the PUBLIC keys (the fixture already had `FixtureAuth`; no platform-sim change),
  retry with 503/429 accepted by the replay store, wrong key rejected, malformed/missing key files, static-token override,
  no seed/token/Bearer in logs or state. First RED: `ModuleNotFoundError: platform_exporter.auth`.
- Runtime dependency `cryptography>=43` added to pyproject/uv.lock (was dev-only; the image installs from the lock).
