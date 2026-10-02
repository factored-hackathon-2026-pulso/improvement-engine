# Open gaps

This ledger contains only blockers that require an explicit decision, credential,
external capability, or action from a human owner. It is deliberately not a
backlog: implementation work that the engine team can advance belongs in GitHub
Issues and continues without waiting here.

## Active gaps

| Gap | Owner | Blocked capability | Evidence needed | Safe fallback |
|---|---|---|---|---|
| Local container-backend smoke | development environment owner | Executing the versioned engine-local Podman Compose stack against a usable backend | `local/compose.yaml`, launcher and contract test are now owned by this repository; a successful opt-in Podman smoke with the selected Windows backend is still required | Ephemeral PostgreSQL CI and structural Compose contract only; do not claim a live local-stack run |
| Private debug ingress | engine + infra + security | AWS `/internal/v1/debug` console | Approved listener/auth/proxy/private-access contract and integration test | Keep debug transport unexposed in AWS |
| Database runtime secret binding | engine + infra | ECS task connection to RDS | Environment-bound secret ARN/schema, least-privilege IAM, rotation and smoke evidence | No deployment claim; use isolated test database only |
| Controlled external egress | infra + security + external-provider owner | Live model/provider calls from AWS runtime | Approved `egress_profile`, destination policy and redacted observability | `aws_private_endpoints_only`; dependency remains unavailable |
| Operational alarm contract | engine + infra + service owner | Actionable continuous operation | Metric namespace/dimensions, destination, runbooks and firing/resolved evidence | Resource telemetry only; do not call the service operationally monitored |

## Operating rule

When a dependency is discovered, record the exact owner, blocked capability,
evidence, safe fallback, and the decision needed. Continue every unrelated
slice. Remove an entry only when the external state has been verified, leaving
a journal link for the resolution.
