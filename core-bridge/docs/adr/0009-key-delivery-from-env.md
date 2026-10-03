# ADR 0009: Key delivery from env secrets in the image entrypoint

Status: accepted.

## Context

The image entrypoint only materialised the identity, staff and service key files from env. The runtime also needs
four bridge signer files and the exporter needs two key files, so the image could not start in AWS from
Fargate-injected secrets alone (secrets arrive as env, not files; DR-89).

## Decision

- `docker-entrypoint.sh` materialises every key file of the selected entrypoint from a named env var into tmpfs
  (`/run/pulso-keys`, 0400, owned by the app uid 10001) and exports the existing path variable pointing at it.
  Variables: runtime `PULSO_BRIDGE_{IDENTITY,STAFF,CALLBACK,EXECUTOR}_SIGNER_JSON`, `CORE_IDENTITY_KEYS_JSON`,
  `CORE_STAFF_KEYS_JSON`, `PULSO_SERVICE_KEYS_JSON`; exporter `PULSO_EXPORTER_KEY_CONTROL_API_SEED`,
  `PULSO_EXPORTER_KEY_LAB_BROKER_SEED`; migrate/agentcore none.
- Validation: signer JSON `{kid, key}` with a 32-byte b64url seed, service keys with 32-byte keys, identity/staff
  non-empty JSON objects, exporter seeds 32 bytes. Any required variable that is missing (and has no valid file already
  present) or malformed exits 2 with `pulso:runtime_config_invalid: <VAR>`; all problems are listed, no value is ever
  printed.
- The runtime refuses to start if the executor signer key equals the callback signer key (separation of duties).
- The secret variables are `unset` before `exec`, so the child process environment and `/proc/<pid>/environ` do not
  contain them. The materialised files are the only copy, readable by the app uid only.
- File mode stays compatible: local/core mounts files and sets the path variables; env content is optional there.
- The image creates `/run/pulso-keys` (0700, uid 10001); with a read-only root filesystem the platform must supply a
  writable tmpfs/ephemeral volume at that path.

## Consequences

The infra task definition injects the nine secret variables above (per entrypoint) via `secrets`, with no key volume
mounts. Rotation is by task replacement (the file watcher still reloads mounted files in file mode).
