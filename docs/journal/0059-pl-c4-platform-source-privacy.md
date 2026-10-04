# PL-C4 platform-live source privacy guard

**Status:** internal fail-closed policy primitives and focused integration checks implemented; source-contract integration pending.

## Behavior

- Added a typed `platform_live` relation/column plan. The reader cannot receive arbitrary table names or credential-table variants.
- Excluded current case status/closure labels from the projection; event payload is explicitly local-only.
- Added exact event classification: known business events pass, unknown events quarantine, and `auth.*` is denied unless its exact event type is allow-listed.
- Added a population predicate excluding `customers.simulator=true`.
- Raw turn text fails closed before the broker. An adversarial follow-up showed that a crate-private constructor plus a caller-supplied treatment label/digests is not proof that names, email, or phone were removed. Until a real verifier exists, the constructor returns `AuthorityUnavailable` and the egress boundary rejects every purported treated receipt before broker invocation.
- Arbitrary event payload has a distinct local-only type; its egress method rejects without calling the broker, and a compile-fail doctest prevents conversion to the turn-text type.
- Event catalogs have positive pinned versions and stable domain-separated SHA-256 digests over the built-in business set and exact auth allow-list. The source plan exposes serializable catalog provenance for a run/source manifest.
- Raw source text remains non-serializable and redacted in debug output. Fail-closed unavailability blocks all text egress until an authoritative treatment receipt and adapter are available; even purported treated output is rejected before the existing projection authorization boundary.

## Tests and limits

First RED: the initial source-policy contract test failed because the policy module did not exist. Adversarial RED after the first review showed the new treatment/provenance, payload type, and versioned-catalog APIs were absent (compiler errors in the focused test). A further adversarial regression supplies a realistic name/email/phone to the internal treatment boundary and expects `AuthorityUnavailable`; before the fix the constructor returned `Ok`, demonstrating the flaw. Final focused verification after removing the unused treated-text value field: `cargo test -p improvement-engine-core --test platform_source_policy --no-default-features` passed 6/6; `cargo test -p improvement-engine-core --lib platform_source_policy::treatment_tests --no-default-features` passed 3/3; `cargo clippy -p improvement-engine-core --test platform_source_policy -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check` passed. The separate treatment unit tests exercise failure to mint without an authority, realistic PII, and forged-receipt rejection before broker invocation.

This is not the exporter, source-schema validation, text DLP, a live SQLite/Postgres integration, nor evidence that hosted-model egress is permitted. The exporter must map these semantic projections against the versioned platform schema, enforce identifier join-only handling, provide authoritative treatment provenance, bind the catalog reference in the persisted run/source manifest, and use the active grant authority. Unknown/mismatched schema is a blocker, not a best-effort projection.

## Independent adversarial re-review — 2026-10-03T22:23Z UTC

**Result: accepted for the explicitly bounded fail-closed policy scope; no current egress bypass found.** The treatment constructor always returns `AuthorityUnavailable`; the text egress method rejects any receipt before broker invocation; event-payload egress returns `LocalOnlyEventPayload`; payload cannot convert to turn text; and raw text/payload `Debug` output is redacted. External callers cannot construct `PlatformTreatedText` because its fields are private. The focused integration tests exercise raw PII rejection/no broker, event payload rejection/no broker, and text redaction; the separate module unit tests exercise receipt-mint denial, realistic PII, and a forged internal receipt. Final test/Clippy/fmt/diff results are recorded above.

**Residual integration constraint:** `PlatformSourceReader` receives a closed enum projection, but this trait signature is not itself a sandbox against an adapter that ignores the plan or accesses/logs other data. The future versioned exporter must enforce the mapping and prove it with contract-backed tests; this limitation is consistent with source-contract integration remaining pending. Minor comment drift: the `PlatformSourceText` type comment currently says its egress path asks the broker, while the implementation intentionally returns before broker access until treatment authority exists. Neither point changes the current scoped fail-closed result; no code was changed during this review.
