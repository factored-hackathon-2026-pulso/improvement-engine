# ADR 0002: JSON is the executable SourceContract wire format

## Status

Accepted for Layer 0.

## Context

The tech spec named YAML manifests while this repository publishes JSON Schema
contracts and a dependency-free Python fixture harness. Having a YAML source
of truth plus a JSON mirror would create a second, mutable representation and
make it unclear which digest a `SourceSnapshot` pins.

## Decision

`contracts/sources/*.json` is the executable, versioned `SourceContract`
format. Its raw SHA-256 is the value pinned by a snapshot's
`source_contract_ref`. Golden CSV headers remain contract fixtures, not source
data, and prove the physical header expected by an adapter. YAML may be
accepted by a future import adapter only if it is converted, reviewed and
stored as this canonical JSON contract before use.

## Consequences

The spec now names JSON as the authority. This avoids an unpinned YAML parser
and duplicate authority in Layer 0. It does not implement an adapter, compute
the source-file digests, or authorize data access; those belong to later data
slices.
