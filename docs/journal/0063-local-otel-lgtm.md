# Local opt-in OpenTelemetry backend

## Decision

The engine's Podman Compose file provides Grafana's LGTM distribution as an
optional local-only backend. The default stack remains PostgreSQL plus
LocalStack; developers opt in with the `observability` Compose profile. The
image is pinned to version `0.33.1` and an immutable digest. The stack is
connected only to the internal `pulso-internal` network, while the three host
ports needed for Grafana and OTLP are bound to loopback.

Metrics, logs and traces default to a 24-hour retention window, configurable
through `PULSO_OTEL_*_RETENTION`. Data is intentionally ephemeral: no `/data`
or host secret mount and no named data volume is configured. Compose checks
both the upstream `/tmp/ready` marker and the image healthcheck script, with a
five-minute startup grace. The pinned `v0.33.1` source confirms the baked
healthcheck and that the marker is written after all bundled services report
ready:
<https://github.com/grafana/docker-otel-lgtm/blob/v0.33.1/docker/Dockerfile>
and
<https://github.com/grafana/docker-otel-lgtm/blob/v0.33.1/docker/run-all.sh>.
The upstream project describes the image as intended for development, demos
and testing, not production:
<https://github.com/grafana/docker-otel-lgtm>.
The upstream configuration documentation describes the supported backend
`*_EXTRA_ARGS` mechanism and whitespace splitting. The checked-in defaults are
contract-tested; caller overrides are not runtime-validated and must be a
single valid Go duration token (for example `24h`), never arbitrary flags.
Malformed overrides may prevent startup. No extra service or wrapper is
introduced just to validate these local-operator-controlled values:
<https://github.com/grafana/docker-otel-lgtm#customize-backend-configuration>.

## Security and scope

The image's local Grafana default is `admin` / `admin`; it is reachable only on
127.0.0.1 and this profile is strictly for local development. Do not send
production/customer PII or credentials to it. The profile supplies backend
endpoints only; it does not add engine instrumentation, emit spans, or claim
that traces currently capture engine execution. Host processes use the
loopback-published endpoints; containers on `pulso-internal` use service DNS
`otel-lgtm`. Because the Podman machine socket was inaccessible during
validation, Compose rendering/startup and effective retention arguments remain
unverified.

## Verification

The Python contract tests check opt-in selection, digest pinning, loopback
bindings, internal networking, startup readiness marker plus image healthcheck,
retention defaults and the
absence of persistent/secret mounts. When a working Podman backend is
available, `PULSO_RUN_CONTAINER_TESTS=1` additionally renders the full Compose
model. A local stack startup or telemetry ingestion is not claimed unless
explicitly exercised.
