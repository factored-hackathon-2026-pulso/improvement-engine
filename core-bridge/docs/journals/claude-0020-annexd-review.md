# claude-0020: independent adversarial review of the Annex D alignment (claude/r3-annexd)

Reviewer: independent subagent. Scope: commits 13fd5e3, 893c0b6. Real PG16 (Podman pulso-dev, 127.0.0.1).

## Verified against the plan
- A03 (i) lists `version`, `core-credentials/issue`, read routes, aliases, dry-run, admissions, arms under
  `sub=worker:<id>`: `sub_prefix` on every `aud=core-bridge` route is what the annex says, no Rust client lock-out.
  The `job_id` claim is listed for class (i) but only invoke (body `job_id`) and admissions (claim is the job) can
  bind it; read routes keep it optional. Sub/purpose/tenant checks run before the jti is consumed; `job_mismatch`
  (handler level) burns the jti, which is the safe direction.
- Idempotency-Key matrix on arms: header only, body only, equal, different (422), neither (422), malformed (422),
  non-string body key (422). Digest is over `canonical()`, key scoped per tenant, so no cross-tenant poisoning.
- Mutation check (12 behaviours, annexd + bridge-contract real): all killed except an equivalent mutant (`timefmt`
  regex widened; `parse` still rejects). Arms alias/digest mutations are covered by annexd only, not bridge-contract.

## Findings fixed (first RED: tests/annexd/test_review_hardening.py)
1. Pathologically nested JSON (100k `[`) -> `RecursionError` -> 500 `internal_error retryable=true` on invoke,
   admissions and arms. Now parsed once in `app.py` after the body cap; 422 `invalid_request`.
2. Duplicated `Idempotency-Key` headers: first silently won. Now 422 `details.fields=["Idempotency-Key"]` on all routes.
3. Admission `evaluation_context_ref` derivation joined free-text fields with `|`: `("a|b","c")` and `("a","b|c")`
   hashed identically (same-tenant DoS / cross-tenant squat via tenant/job). `derive_context_ref` now refuses `|`
   in any field; the route answers 422. Formula unchanged for every legitimate input, so Rust's derivation holds.
4. Added accept/reject matrix tests for `parse_z_timestamp` (lowercase z, 10-digit fraction, leap second, offsets,
   unicode digits, trailing newline, huge).

## Unresolved / judgement calls
- Arms `deadline` is format-checked but not enforced and is part of the single-flight digest: a retry that recomputes
  the deadline gets 409 `idempotency_conflict`. Decide: exclude from digest and enforce, or document.
- `agent_id` stays in the digest: omitting it vs repeating the derived value are different requests.
- First-wins on duplicate `Authorization` headers (Starlette); not changed.
- Digest format of stored arm rows changed (`canonical()`); rows persisted before 893c0b6 would replay as conflicts.
