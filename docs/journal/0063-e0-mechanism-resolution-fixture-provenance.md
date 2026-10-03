# E0 mechanism-resolution fixture provenance

The RED contract test `crates/core/tests/e0_mechanism_resolution.rs` exercises
the proposal path for `e0_recurring_copilot_query_cases`, the qualifying metric
recorded by the actual local E0 smoke in P2 journal 0050. The test's repeated
query signature is explicitly `team_generated`; it is a source-shaped contract
fixture, not an E0-observed query record. Its `LocalSourceKind::E0` value only
selects the E0 evidence contract in local simulation.

The evidence packet must bind the assembled candidate to its matching
`SignalSummary`, including `pattern_ref`, signal digest, summary commitment,
source snapshot and observed cutoff. This contract fixture checks privacy by
ensuring the generated signature and tenant scope do not appear in the packet.
Recurrence is descriptive only: the signature does not establish intent, an
attention route, a specific Agent Core Flow, or a causal opportunity.

Resolution uses an explicitly supplied immutable route-catalog reference
(artifact id, revision and digest), even when that catalog has no route
entries. Missing exact mapping must resolve `unlinked` against that catalog
reference; there is no default or guessed route. Compilation and sandbox trial
remain disallowed. The real E0 smoke's 154/200 support is a separate local-run
observation recorded in P2's journal; this synthetic test neither reproduces
that count nor claims to resolve that observed candidate. A source-backed route
mapping would require explicit, versioned evidence and must not be inferred
from the opaque query signature.

Catalog resolution is tenant-bound to the validated source run. A catalog
whose tenant differs from the packet's source tenant must fail closed as
`unlinked` (`catalog_tenant_mismatch`); the internal tenant scope is skipped
from packet and resolution serialization. The Flow reference validates the
declared pin and syntax only; it does not prove registry existence or verify
the digest against retrieved Flow content. The exact pinned `agent-core`
checkout SHA `86a767474042a566a0dbd6ed23588959f27ebdb3` defines entity ids as
`[a-z0-9][a-z0-9_/-]*` and exact versions as numeric `X.Y.Z` without leading
zeroes or suffixes. Route-catalog artifact IDs use the engine's existing
ArtifactRef UUIDv7 schema. These grammars are enforced before values can be
serialized. The pinned schema permits human-looking Flow IDs, so syntax alone
does not make an ID trustworthy: a syntactically valid Flow ID must originate
from a trusted Agent Core registry lookup, never from user-provided text.

## RED and implementation status

The original focused RED on P2's locally validated base
`5aa8e58f2098c91f45913011add72af26612c7fe` failed for the missing module
(`E0432`). The added cross-tenant regression then failed for the intended
behavior: the foreign catalog was not returned as `Unlinked`. After binding
tenant scope internally and rejecting mismatches, the focused test passed:

During the first focused run, the generated signature fixture initially used a
non-contract string and correctly failed input validation. It was corrected to
the contract's opaque `sha256_` plus 56-hex format before recording the
cross-tenant RED; this was a fixture correction, not a product behavior change.

```text
cargo +1.98.1 test --locked --offline --jobs 2 -p improvement-engine-core \
  --features local-simulation --test e0_mechanism_resolution
1 passed
```

No broader suite or CI claim is made by this focused result.

Two additional privacy-sentinel assertions first failed because the catalog
accepted a non-opaque artifact id and the Flow reference accepted a newline
plus secret-like version suffix. After enforcing UUIDv7, pinned Core entity-id
grammar, and exact version syntax, the same focused target passed again. These
are contract-validation fixtures only; the synthetic explicit route still
does not represent an observed Core registry entry.

The final adversarial sentinels also rejected an email-style Flow ID, a
leading-zero version (`01.0.0`), and a prerelease suffix (`1.0.0-beta`). The
focused test passed after these assertions and validators were in place.
