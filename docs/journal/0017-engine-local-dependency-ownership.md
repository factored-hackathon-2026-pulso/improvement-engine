# Engine local dependency ownership

## Decision

The optional Podman Compose stack, Windows secret initializer and its contract
test are owned by `improvement-engine`. They are runtime/test dependencies of
the engine; sibling `infra` owns Terraform and AWS deployment only.

## Migration

Migrated `local/compose.yaml`, `local/.env.example`,
`scripts/init-local-env.ps1` and `tests/test_local_compose_contract.py` from
the pending infra bootstrap work. Paths are now repository-relative to the
engine. The checked-in Compose contract has pinned PostgreSQL/LocalStack images
and loopback ports; no secret is committed.

## Verification

The initializer unit tests run on Windows. The Compose rendering check remains
opt-in behind `PULSO_RUN_CONTAINER_TESTS=1`, because a config render is not an
available container backend. `infra` Terraform validation has no dependency on
these assets.
