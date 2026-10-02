# Deployment-boundary alignment

**Date:** 2026-10-02

## Objective

Reconcile the service-owned local/runtime contracts with the Terraform-owned
AWS boundary without claiming deployment.

## Decision

`improvement-engine` owns its future Compose/LocalStack replacement, while
`infra` owns AWS declarations. The local stack is still a documented gap, not
evidence supplied by ephemeral PostgreSQL CI. AWS debug ingress is blocked on
the selected internal-ALB plus identity-proxy contract. Runtime database
connectivity requires one environment-bound secret ARN, and external egress
requires a versioned profile.

## Evidence and follow-up

The decision is recorded in
`docs/architecture/deployment-boundary.md` and `docs/gaps/OPEN_GAPS.md`.
No runtime, Terraform resource, secret or deployment was changed. Follow-up
slices must include their own RED/GREEN contracts and integration evidence.
