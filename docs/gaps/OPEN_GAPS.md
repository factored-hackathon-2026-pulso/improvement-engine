# Open gaps

This ledger contains only blockers that require an explicit decision, credential,
external capability, or action from a human owner. It is deliberately not a
backlog: implementation work that the engine team can advance belongs in GitHub
Issues and continues without waiting here.

## Active gaps

| Gap | Owner | Blocked capability | Evidence needed | Safe fallback |
|---|---|---|---|---|
| Local container-backend smoke | development environment owner | Executing the versioned engine-local Podman Compose stack against a usable backend | `local/compose.yaml`, Windows launcher and contract tests exist; selected Windows Podman backend currently returns `Access is denied`; repeat the opt-in smoke once backend access is restored | Ephemeral PostgreSQL CI and structural Compose contract only; do not claim a live local-stack run |
| Private debug ingress | engine + infra + security | AWS `/internal/v1/debug` console | Approved listener/auth/proxy/private-access contract and integration test | Keep debug transport unexposed in AWS |
| Database runtime secret binding | engine + infra | ECS task connection to RDS | Environment-bound secret ARN/schema, least-privilege IAM, rotation and smoke evidence | No deployment claim; use isolated test database only |
| Controlled external egress | infra + security + external-provider owner | Live model/provider calls from AWS runtime | Approved `egress_profile`, destination policy and redacted observability | `aws_private_endpoints_only`; dependency remains unavailable |
| Operational alarm contract | engine + infra + service owner | Actionable continuous operation | Metric namespace/dimensions, destination, runbooks and firing/resolved evidence | Resource telemetry only; do not call the service operationally monitored |
| Codex control-api not published | Codex (engine) | Console `provider=http`, debug route field names, 410 body shape and the Rust ingest endpoint for the exporter | Published control-api contract under `contracts/product/` and a running service | Console runs on fixtures and the stand-in; exporter and e2e use the ingest fixture; no joint run |
| E0 -> Core flow mapping | Codex (engine) | Turning an E0 finding into a `ChangeSpec`/`DraftPlan` and calling the bridge (dry-run, invoke, admission, arms) from the Rust client | Rust client checked against `bridge-contract/` and the pinned wire snapshot; open items AM-24, AM-32, AM-38..AM-42, AM-45 confirmed | `e2e-core/` Codex stand-in drives the real stack; report it as a stand-in |
| Platform payload shapes unconfirmed | Product team (via the user) | Trusting `event_type` payloads, `sequence` contiguity and `teams` timeline of the support platform | Product answers to the questions in `PLATFORM_DATA_MODEL_IMPACT_CLAUDE.md` section 7 | `platform_live` shapes stay simulator assumptions; label them so in any evidence |
| Backfill gap after exporter state loss | engine + Product team | Source completeness after an exporter loses its state | Product-confirmed read mechanism and a durable backfill request path (backfill requests are lost with the exporter state; a permanently missing sequence and `start_sequence=0` are documented, not solved) | Gap and late-event findings are emitted; never claim complete coverage |
| Mock divergences | Team Claude | Treating the registry/bridge mocks as the real wire | `bridge-contract/conformance/known_different.py` lists each divergence id (`MOCK-*`); the mock suite carries strict xfails until the mock converges | Use the real runtime or the golden flows for contract claims; report the mock as a double |
| Real wire not run | Team Claude + Agent Core team | `real-wire` parity suite against a running `agent-core serve` and a joint H4 run | `REGISTRY_BASE_URL` of a real Core, run on the current pin | Mock and a2 harness evidence only; make no real-wire claim |
| GitHub Actions quota exhausted | repository owner | Hosted CI results for PRs; Python, console, bridge-contract and Terraform suites are not in `ci.yml` today (the workflow is Codex-owned) | Restored Actions budget and a workflow that runs those suites | Local validation reported in each PR body (`core-bridge/scripts/ci.ps1`); a local green run is not a hosted check |

## Operating rule

When a dependency is discovered, record the exact owner, blocked capability,
evidence, safe fallback, and the decision needed. Continue every unrelated
slice. Remove an entry only when the external state has been verified, leaving
a journal link for the resolution.
