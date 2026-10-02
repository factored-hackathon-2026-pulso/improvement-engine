# U16 — provisional workflow bridge and alternatives

## Objective

Turn independently verified evidence into a tightly scoped, provisional
mechanism assessment without turning a historical pattern into a causal or
release claim.

## Implemented boundary

- `WorkflowBridge::assess_verified` accepts a `VerificationReport` from U14,
  never a raw Scout draft or caller-supplied verification status.
- El validador interno acuña `ValidatedLinkEvidence` y un recibo recomputable
  ligado a reporte, snapshot, input, digest de catálogo, recibo de
  disponibilidad y hechos. Ningún builder externo puede acuñar esos flags.
- The bridge commits the U14 candidate, provenance, input and receipt digests
  together with its typed bridge input and deterministic linkage facts.
- A supported result can reach only `mechanism_proxy` in U16, even where all
  known links are present. `same_outcome_linked` remains unavailable until the
  later sealed oracle/baseline and eligibility boundaries (U20/U35).
- Refuted evidence is `unlinked`; uncertain evidence is `not_evaluable`.
  Neither provides a selectable route. Every outcome retains `do_nothing`.
- The slice owns no persistence, ChangeSpec, Agent Core artifact selection,
  ranking, model call, sandbox execution or publication.

## TDD evidence

The first public contract test was RED because `workflow_bridge` did not
exist. The smallest module then made it GREEN. Subsequent regression tests
cover the non-elevation invariant, refutation and uncertainty. Local checks:

```text
cargo +1.98.1 test -p improvement-engine-core --lib workflow_bridge
cargo +1.98.1 clippy -p improvement-engine-core --lib -- -D warnings
```

Both passed before the independent code review. Full workspace and CI remain
required before cumulative integration.

## Deliberate limits

`VerificationReport` does not encode outcome, population, key or route
semantics; U16 therefore recibe esos valores como input tipado y exige una
validación interna sellada; no pretende inferirlos desde U14. El adaptador de
catálogo/fuente real, escenarios/oracle, persistencia y
`eligible_for_proposal` pertenecen a slices posteriores. Esta frontera no se
presenta como integración E2E de catálogo.
