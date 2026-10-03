# 0011 — P4: governed memory expiry

## Scope

Implement an explicit snapshot expiry boundary for the in-memory U15/U22/U33
memory path. This complements immutable revocation tombstones; it does not
implement blob deletion, production retention scheduling, or a durable storage
adapter.

## Contract

- `MemoryWiki` payloads may include `expires_at_unix_seconds`.
- If absent, legacy/no-expiry behavior is preserved.
- If present, expiry must be strictly later than `available_at_unix_seconds`.
- Expiry is an exclusive deadline: memory is usable only when
  `allowed_at_unix_seconds < expires_at_unix_seconds`. At the deadline and
  afterward the wiki mount is rejected before pages enter the run workspace.
- The same check applies to transform verification and governed U33 use; an
  expired use cannot create a receipt.
- A published revision preserves the base expiry exactly. Edits cannot extend
  retention or create a zero-lifetime successor. Physical purge is a separate
  retention operation and remains out of scope.
- Seeding a scope head validates snapshot structure, not a fabricated “now”.
  Read/use checks compare expiry with the trusted caller-provided replay/as-of
  value. This adapter does not authenticate that value as current wall-clock
  time and cannot prevent a caller from backdating it; this is not a retention
  or security clock.

## Verification

Public integration tests cover t=149 allowed, t=150/t=151 denied for expiry=150,
invalid expiry at/before availability, published expiry preservation, publish
rejection at the deadline, no U33 use receipt after expiry, and a public
artifact-reference regression proving expiry participates in the immutable
snapshot digest.

Validated locally on Windows:

- `cargo test -p improvement-engine-core --test wiki_scratch` — 10 passed.
- `cargo test -p improvement-engine-core --test published_memory` — 8 passed.
- `cargo test -p improvement-engine-core --test wiki_scratch changing_memory_expiry_changes_the_immutable_artifact_digest -- --exact` — 1 passed.
- `cargo clippy -p improvement-engine-core --tests -- -D warnings` — passed.
- `cargo fmt --all --check` — passed.
- `git diff --check` — passed.

Independent adversarial review accepted with no blocking findings; its digest
coverage suggestion is verified by the focused test above. This slice must not
be described as production durable expiry or physical erasure.

## Public page access and caller-clock boundary (2026-10-03)

`WikiWorkspace::pages()` now requires `WikiAuthorizationPort` and `WikiAccess`,
then applies the same `validate_workspace_access` grant, scope, availability,
and expiry checks as `read` and `transform`. This closes the public raw-map
accessor that could expose already-mounted content after the supplied expiry
time. Public tests exercise access at and after the deadline through the
pages API.

The timestamp remains deliberately a trusted composition-provided replay/as-of
value. A regression demonstrates the limit: the local adapter rejects an
access carrying `allowed_at_unix_seconds == expires_at_unix_seconds`, but a
caller can backdate that public field and the adapter cannot detect it. No
authenticated time source was invented; production retention/security expiry
requires a trusted temporal issuer or clock at composition. The check here is a
deterministic policy boundary, not a wall-clock or physical-erasure guarantee.

`WikiWorkspace` debug formatting is redacted: it reports page count and
availability/expiry metadata without serializing page bodies or the stable
workspace correlation handle, so formatting a workspace cannot bypass the
authorized content-read boundary. Explicitly
returned `WikiReadResult` and `WikiTransformResult` values remain caller-owned
copies after an authorized operation; expiry does not retroactively revoke
content already returned to a caller.

Verification after the public-access and Debug-boundary changes (Windows,
Rust 1.98.1): `wiki_scratch` 13/13, `published_memory` 8/8, Core test Clippy
with `-D warnings`, `cargo fmt --all -- --check`, and `git diff --check` passed.
Independent final adversarial review is pending.
