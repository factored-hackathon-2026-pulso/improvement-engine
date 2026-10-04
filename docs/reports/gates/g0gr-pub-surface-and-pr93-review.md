# G0gr: pub-surface audit and post-merge review of PR #93

Baseline: improvement-engine main `853b029` (PR #93 merge). Method: static text scan
(`pub_surface_audit.py`, no cargo). Machine output: `pub-surface-audit.json`.
Regenerate: `python docs/reports/gates/pub_surface_audit.py . --out docs/reports/gates/pub-surface-audit.json`.

## Measured counts (replace the plan's 521 and 59)

| Metric | Measured | Plan figure |
|---|---|---|
| `pub` types (struct, enum, trait, type, union; excludes `pub(crate)`) | 553 | 521 |
| of which `crates/core` / `source-adapters` / `runner` | 525 / 28 / 0 | - |
| Types with `Serialize`/`Deserialize` derive | 210 (core 174, source-adapters 24, runner 12) | - |
| `compile_fail` references | 59 (all in `crates/core`) | 59 |
| `.rs` files scanned | 108 | - |

Notes: the scan is lexical (macro-generated items are invisible; indented items in nested
modules and test files are included). The 521 figure was most likely taken before the PR #93
modules and is not reproducible by this method; treat 553 as the new baseline. The
`compile_fail` count matches the plan. `crates/core/src/lib.rs` declares 48 `pub mod` and no
private `mod`: the whole module tree is public API, so WDX has no narrow export surface to
preserve and must define facades additively.

## Export request list for WDX (candidates, to be confirmed by CF0/WDX)

- `paired_scenario`: `PairPlan`, `ArmObservation`, `ArmBinding`, `PairVerdict`,
  `InfrastructureFailure`, `PairError` (DGATE consumes these).
- `platform_source_policy`: `PlatformSourceReadPlan`, `PlatformSourceReader`,
  `PlatformRelation`, `PlatformColumn`, `PlatformSourceText`, `PlatformTreatedText`
  (DPLAT consumes these).
- Sealed constructors must stay sealed (`PlatformTreatedText::from_authoritative_treatment`
  is `pub(crate)` and currently always errors).

## Maxima read

ADR max 0005; journal max 0067; migrations 0001-0004 (duplicate `0002_` prefixes exist:
`0002_model_attempt_ledger`, `0002_pulso_memory_control`, `0002_pulso_platform_observations`).

## PG16 vs PG17 preflight (static)

Engine CI uses `postgres:17` (digest-pinned); local Core fragment and agent-core compose use
`postgres:16`. PR #93 states PostgreSQL destructive tests were not run, and journal 0052/0053
say PG17 compatibility is unconfirmed. No PG-specific SQL was added by PR #93 (no migration
touched). Risk: a migration valid on 17 and invalid on 16 would only surface in the Core stack.
Action: L-PG should run migrations 0001-0004 against both versions before 0050+.

## PR #93 post-merge review (read-only, no code edited)

Scope: 28 files, +3387/-42, merged as `853b029`. Reviewed `platform_source_policy.rs` and
`paired_scenario.rs` in full and the PR body and journal 0067.

Positive:
- Credential relations are unrepresentable (`PlatformRelation` is a closed enum); no dynamic
  table or SQL API.
- Raw-text types (`PlatformSourceText`, `PlatformEventPayloadLocalOnly`) have no serde and a
  redacted `Debug`; a `compile_fail` doctest guards the payload-to-body conversion.
- `digest_parts` is length-prefixed, so concatenation ambiguity is avoided; tri-state
  oracle matching keeps infra failures distinct from behavioural failures.
- Honest non-claims (no lift, fixture-only) are stated in code docs and the journal.

Findings:
1. (Medium) `ArmBinding` has all-`pub` fields and `ArmObservation::completed` takes a
   caller-supplied binding. Anyone can forge a binding and a "completed" observation; the
   doc comment admits it. DGATE must not treat `NoRegressionObserved` as evidence of
   sandbox execution. Consider a sealed constructor or an authenticated runner receipt.
2. (Medium) `PlatformTreatedText::from_authoritative_treatment` always returns
   `AuthorityUnavailable`, so no turn-body model egress path exists. Fail-closed is correct,
   but DPLAT/M-stage WPs that need text must stand in or wait for a treatment authority.
3. (Low) `EVENT_COLUMNS` includes `EventPayloadLocalOnly` in the default read plan: the
   adapter reads arbitrary event JSON by default. It is wrapped as local-only, but the
   read itself widens the exposure surface; DPLAT should make it opt-in.
4. (Low) `PlatformSourceReader::read_relation` returns `Result<(), E>`: it yields no rows, so
   the trait is a policy shape and not yet a usable adapter port; its signature will change
   (breaking for any implementor) when DPLAT lands.
5. (Low) `PlatformTurnBody::from_source` is public and unvalidated (no size cap).
6. (Info) `PlatformSourcePolicyError<E>` has a single variant; `positive_count_plumbing_v1`
   trigger is explicitly non-calibrated; hosted CI result for #93 was not locally inferred.
7. (Info) Only 1 `compile_fail` doctest sits in `platform_source_policy.rs` and none in
   `paired_scenario.rs`; the sealed `ArmBinding`/`PairPlan` guarantees have no compile-time
   negative tests.

Verdict: no blocking defect found; findings 1 and 2 should be carried into DGATE and DPLAT
acceptance criteria. Codex owns the code; these are findings only.
