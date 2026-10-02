# Deployment boundary and runtime contracts

**Status:** proposed; this describes the target boundary and current blockers.
It does not claim that a local stack, AWS resource, listener, secret or alarm
has been deployed.

## Ownership and status matrix

| Capability | Owner | Current status | Evidence needed before it is operational |
|---|---|---|---|
| Domain, worker, contracts and service telemetry | `improvement-engine` | Implemented incrementally by vertical slices | Slice-specific code, tests and CI |
| Local Podman Compose, LocalStack, disposable PostgreSQL and local smoke | `improvement-engine` | Compose manifest, Windows launcher and contract tests exist; **runtime smoke is not verified** (Windows Podman currently returns `Access is denied`) | Successful opt-in smoke with the selected Windows Podman backend, including healthy PostgreSQL/LocalStack and E0 CLI execution; infra must not be the fallback owner |
| Ephemeral PostgreSQL migration CI | `improvement-engine` | Implemented in the service CI | The isolated CI job and its destructive-test consent |
| AWS network, IAM, storage, database, compute and infrastructure alarms | `infra` | Terraform declarations are in progress; no apply is implied | Approved backend/OIDC/inputs, reviewed plan and separately authorized apply |
| Engine ingress and debug console exposure | Joint contract; infra provisions only after it is complete | **`dependency_blocked`** | Listener, health, auth, identity proxy and private-network contract, plus integration test |
| Agent Core and model routing | External dependencies | Not owned here | Consumer contract and integration evidence |

The engine owns local development. Its versioned `local/compose.yaml`, local
launcher and structural/contract tests are present, but they do not prove the
stack runs on this Windows host. Podman currently fails with `Access is denied`,
so the smoke remains an active environment blocker and no operational local
stack is claimed. The GitHub Actions PostgreSQL service is not a substitute for
a developer Compose/LocalStack stack.

## Concrete debug-ingress decision

The selected future mechanism is **an internal ALB in private subnets followed
by an identity-aware proxy**. The proxy is reachable only from an approved
private corporate-access path; it authenticates a human and forwards to the
engine debug listener. There is no public API Gateway placeholder, public ALB,
or direct task inbound rule.

Until all of the following are versioned and tested, `/internal/v1/debug` is
`dependency_blocked` in AWS and the engine exposes no deployable browser
surface:

1. listener protocol, health endpoint, request limits and immutable image
   digest;
2. proxy identity/role mapping for `debug_viewer` and `debug_operator`, plus
   explicit 401/403/unknown-route behavior;
3. security-group route `proxy -> internal ALB -> engine task`, with no
   `0.0.0.0/0` inbound rule;
4. private access-path ownership and approval; and
5. integration tests for authentication, tenant isolation, redaction and
   health/deployment failure.

The development console may run locally behind its local-only service profile;
it is not evidence that AWS ingress exists.

## Runtime-to-database secret contract

Terraform never supplies a secret value. Before an ECS task can be considered
operable, deployment configuration must provide a versioned
`database_connection_secret_arn` reference. It may reference the RDS-managed
master secret or an approved least-privilege application secret, but it must
resolve to one JSON secret schema with `host`, `port`, `dbname`, `username` and
`password` (and an optional `sslmode`).

The infra runtime role may read exactly that ARN and use KMS only through
Secrets Manager. The task definition injects only the ARN/configuration
reference; the engine retrieves it at startup, validates the schema without
logging its content, builds TLS-required database connectivity and emits a
treated readiness result. The database endpoint and secret reference must be
bound to the same environment. Rotation, unavailable-secret behavior and a
non-sensitive connection smoke test are mandatory deployment tests. A generic
empty `runtime` secret is not database connectivity by itself.

## Egress contract

`runtime -> AWS APIs` is not synonymous with unrestricted HTTPS. Each
environment declares one versioned `egress_profile`:

| Profile | Permitted path | Use |
|---|---|---|
| `aws_private_endpoints_only` | VPC endpoints for approved AWS services | Default when no external model/provider route is needed |
| `controlled_nat` | NAT through approved egress control whose destination policy includes the exact external provider route | Required only for an approved external dependency |

The current broad HTTPS security-group rule is an infrastructure foundation,
not proof of a least-privilege provider path. A deployment slice must document
DNS, destination policy, logging/redaction and failure behavior, and must not
call an arbitrary `0.0.0.0/0:443` rule an AWS-only route.

## Observability boundary

The engine emits treated OpenTelemetry signals and durable run events; `infra`
owns their AWS sink, alert transport and resource-health alarms. CPU alarms are
diagnostic only. Operational readiness requires the engine-to-infra metric
contract and actionable alarms for queue age/no progress, error rate, ingest
gap and budget in addition to resource saturation. Each alarm needs an owner,
destination, runbook, firing/resolved test and versioned threshold.
