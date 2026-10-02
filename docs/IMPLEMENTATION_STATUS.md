# Cumulative implementation status

This document is the entry point for the long-lived cumulative implementation
PR. It distinguishes merged capability from work in progress; it never treats a
green unit test as proof that a broader product flow is complete.

## Integrated into `main`

- U29: tenant-scoped platform observation ingestion, source-contract
  provenance, truthful coverage and PostgreSQL RLS contract.
- U34: durable, fenced and truthful operator run-control receipts.
- U13: autonomous Scout drafts with bounded U08/U12/Core/model provenance.
  Its generic `DeterministicSensor` route remains public/test-oriented and is
  never evidence authenticated for E0 improvement. U13-E adds a separate,
  opaque U12-E composition and U13-A durable-admission route; U13 must still
  never treat a public `QueryResult` as improvement evidence.
- U33: scoped immutable published-memory revisions, head CAS and idempotent use
  receipts. Its present in-memory adapter is not production-durability proof;
  the U33 corrective slice covers an overflow atomicity regression.

## Implemented in the cumulative branch (not yet merged)

- U07 durable V2 run-event persistence: new `pulso_jobs` / `pulso_run_events`
  migration and PostgreSQL ledger atomically compare-and-sets a child job
  status (only V2 §15 reducer edges), advances the root run sequence, and
  appends its UUIDv7 event; run-level events may omit `job_ref`. The
  migration enforces tenant/run references and prohibits child-owned sequence
  state. This is a persistence boundary only: it is not wired to U06 admission
  or lease/quota/fence gates, and it deliberately does not implement the current
  job/timestamp `RunActivityReadModel` because that API cannot represent V2's
  run/sequence cursor without a versioned contract change. See ADR 0004 and
  journal 0051; the isolated PostgreSQL test requires explicit DB opt-in.

- Local E2E composition/runner (new vertical): opt-in `local-sim` CLI joins
  immutable source provenance to the safe event projection and invokes local
  detection. Only positive supported signal evidence proceeds into the local
  Scout/verifier path and may emit a non-executable simulated draft and
  structural-only evaluation; zero positive support produces an explicit
  no-op with no candidate, proposal, verifier, or evaluation. It writes an
  atomic JSON result plus NDJSON timeline and never calls external providers
  or Agent Core. A versioned recurrence detector counts distinct Arranque
  cases sharing the leading opaque Copilot-query signature; its default
  support floor is 20, configurable from the CLI and committed in provenance.
  After a supported recurrence becomes the primary signal and the core has
  admitted an opportunity candidate, the runner now evaluates the selected
  opaque signature against distinct queried Reproduccion cases. This is
  strictly post-selection and never feeds back into discovery, candidate
  scoring, or proposal construction. The JSON result/timeline contain only
  policy/source commitments, aggregate counts/rate, a safe status and a
  non-causal interpretation; no selected candidate means no holdout result.
  A synthetic CLI regression proves that changing only Reproduccion signatures
  changes holdout status while Arranque metrics, proposal hypothesis and
  candidate count stay fixed. The actual augmented-sample smoke is documented
  once in `docs/architecture/e0-recurrence-holdout.md`.
  Technical errors remain an independent metric. If both qualify, the explicit
  primary policy prefers direct observed technical failures over semantically
  opaque query recurrence; every measured metric remains visible, but only the
  primary signal currently traverses the single-candidate Scout simulation.
  If the optional `copilot_query` source table is absent, recurrence is marked
  unavailable rather than reported as zero. The signal proposal is a
  simulated, unverified, non-executable `unclassified_candidate`; it makes no
  semantic, causal, lift, or real-bank claim. Only the selected supported
  signal proceeds into the local Scout/verifier path; zero positive support
  produces an explicit no-op. The formal route is always `do_nothing`.
  Actual local E0 smoke (2026-10-02) used 200 Arranque cases; the total
  Reproduccion population is intentionally suppressed. The leading opaque query signature recurred in
  154/200 cases (policy floor 20); technical errors remained 0/187 supported,
  with 13 missing. The runner recorded three Scout candidates and one
  exploratory proposal. Persisted output contains the exact
  configured cutoff and no known PII sentinels or evaluator labels.
  This demonstrates only bounded local detection/simulation behavior, not
  native Agent Core execution, causal validation, release, or business lift.
  Original-bank local execution now supports an independently sealed,
  privacy-safe snapshot projection for call-center reason × channel. The API
  names its metric `record_count`: it counts CSV records and does not deduplicate
  `interaction_id`. A nonblank `reason_category` takes precedence, then a
  nonblank `contact_reason`; both blank maps to `unclassified`. It is not an
  event-date cohort or cutoff-filtered result; naive source timestamps are not
  compared to the run cutoff. The local motor reports this descriptively but
  emits no signal, candidate, or proposal. Repeat contacts, PQR/SLA, technical
  errors, causal relationships, and outcome claims remain unsupported; customer
  IDs, row-level facts, free text, and contact outcomes are not used. `k`
  defaults to 5 and is configurable via `--min-contact-cell-count` (5–10,000);
  policy version and threshold are sealed into the prepared-source manifest and
  validated again at the core boundary. Tests are synthetic only; no real-data
  prevalence claim is made.
- Windows local E0 convenience wrapper: scripts/run-local-e0-e2e.ps1 invokes
  the opt-in local simulation with locked/offline Cargo, required UTC cutoff,
  default Arranque/support settings and a fresh non-overlapping output path;
  observed existing reparse-point components fail closed before Cargo runs.
  Mapped-drive/UNC alias equivalence and concurrent path mutation are outside
  this guarantee. It prints only allowlisted aggregate summaries, including
  post-selection holdout status and, only when support meets the privacy floor,
  matching/queried counts marked descriptive-only; below the floor all exact
  holdout counts/rate are null. The E0 top-level excluded-replay total is
  always null (including absent/unavailable holdout), and the wrapper prints
  `counts=suppressed` / `replay_excluded=suppressed` as applicable.
  Missing holdout is `none`. Pester tests use a temporary
  Cargo shim and do not substitute for a real package run. Usage and safety
  boundaries are in docs/local-e0-e2e-runner.md.
- U13-A / Issue #43: opaque, verified Scout-candidate admission. Candidate
  batches are canonical and atomic; durable reload validates member and batch
  commitments; downstream code receives a read-only capability rather than a
  forgeable draft.
- U13-E / Issue #53: authenticated E0 Scout admission. Only a crate-private
  composition of the U12-E immutable signal and matching U09/U10 receipts can
  form the opaque capability. It revalidates full scope, U04 raw binding,
  profile/cutoff/source commitments and E0 query receipts; it persists typed,
  canonical E0 provenance —incluyendo tabla y commitment de campos— through
  the existing U13-A batch so U14 receives
  only `VerifiedScoutCandidate`. It does not use public `QueryResult` or the
  generic Scout authority, and it has no recorder beyond U13-A, registry,
  promotion, release, runtime or Agent Core execution effect.

- U30 / Issue #41: deterministic platform sensor. It consumes the U29 safe
  projection and emits only sealed, mapping-resolution-bound signals; it never
  reconstructs observation batches or coverage.
- U14: independent verifier. It accepts only the U13-A opaque capability and
  emits a provenance-bound supported/refuted/uncertain report; persistent
  reports and U11 Jev-adapter wiring remain later dependent work.
- U14-E: Frozen E0 consistency verifier. It is a separate, deterministic
  route from the generic verifier: it rehydrates only U13-A's E0 candidate,
  validates sealed Frozen provenance under a versioned policy and emits an
  opaque `Consistent` report bound to exact commitments. `Consistent` is not
  causal corroboration, outcome evidence, eligibility or release authority;
  this slice has no pluggable receipt/port, storage, runtime or Agent Core
  side effect.
- U14-EQ: pure Frozen E0 opportunity qualification. It accepts only the exact
  U13-A capability plus its recomputed U14-E report and emits a nonpersistent,
  opaque qualification: frozen provenance consistent; commercial impact and
  operational effort not assessed; mechanism/evaluation still required; route
  none. It is not U16, a proposal/value/causal conclusion, evaluation or
  release authority and has no storage, LLM, Core or runtime effect.
- U15-EQ: Frozen E0 scratch-summary preparation. A crate-private composition
  revalidates the exact U13-A/U14-E/U14-EQ and U04-B replay chain before a
  single static allowlisted U15 `Create` transform. Scope/access are exact
  Frozen bindings with `allowed_at == cutoff`; output is opaque commitments
  only, never pages, workspace/text, publication, memory use or an artifact.
  U33-E publication and U23-E governed-memory linkage remain explicitly
  pending.
- U33-E: Frozen E0 summary publication boundary. A crate-private composer
  redeems only the opaque U15-EQ preparation after recomputing the exact
  U13-A/U14-E/U14-EQ/U04-B and canonical U15 transform chain. It emits an
  opaque publication capability, not MemoryUse, proposal, route or release
  authority. The local adapter models head-CAS/idempotent provenance-sidecar
  writes; the durable adapter is intentionally `DependencyUnavailable` until
  a U05 grant revision/liveness fence can be evaluated in the same durable
  transaction. U23-E remains pending and is the sole future consumer allowed
  to attest governed use of this publication.
- U16: provisional WorkflowBridge. It accepts only a U14 verification report
  and sealed internal catalogue/source-validation facts, preserves its
  commitments and scope, includes `do_nothing`, and caps a supported route at
  `mechanism_proxy`. Its as-of cutoff now reuses the U04-B fixed-width UTC
  timestamp validator; date-only, local/offset, fractional, impossible-date,
  and out-of-range-time values fail closed. The cutoff is part of the bridge
  commitment, which U17 preserves in compilation authorization. U20/U35 still
  own the sealed outcome/oracle and final eligibility gates.
- U26: stateful synthetic-bank sandbox. Each tenant/namespace evaluation arm
  starts from one sealed fixture seed, applies only typed permitted effects
  behind an expected-revision fence, and proves the outcome through scoped
  readback/reset receipts. Candidate and baseline arms do not share state; it
  has no network, filesystem, customer data, Agent Core runtime or bank
  authority. U36 extends this adapter with protected-fixture identity checks.
- U36: protected-fixture sandbox identity boundary. A protected arm receives
  an opaque issuer capability only through trusted composition; identities are
  nonce-registered to that arm, checked with the sandbox-owned clock, and
  rejected on revocation, expiry, forging, or cross-arm use. This is fixture
  protection, not a production identity-provider implementation.
- U04-B: E0 replay availability clock. A V2 replay binds its complete,
  tenant-scoped immutable source snapshot and a sealed file manifest before
  access; V1 snapshots cannot be replayed because they cannot prove that
  binding. Serialization loses non-serializable file seals and fails closed.
- U34-F: immutable original-run fork. The only in-memory commit entry point
  owns its grant, lifecycle, artifact-policy, idempotency and audit state; it
  atomically predicates a sealed fence before exposing one child, receipt and
  audit event. The local port models the durable adapter contract but is not a
  deployed durable transaction implementation.
- U20: sealed evaluation plan. Trusted composition attests the exact typed
  baseline, oracle, development-suite and final-suite revisions before it
  seals their scope, snapshot and evaluation semantics for a mechanism proxy.
  It cannot execute a candidate, assert the same outcome, make a proposal or
  release a change.
- U20-E / Issue #49: sealed E0 safety-oracle composition. It binds U04-B's
  replay tenant/world/cutoff/source-byte seal/profile, the exact U20 four inputs, and
  U36's protected-fixture case/channel/policy/questions commitments before a
  later evaluator may consume the context. The capability has no public
  constructor, exposes no identity answers/proofs/principals, fails closed on
  unsafe or unknown identity observations, and neither executes nor releases.
- U24: read-only technical timeline boundary. The internal debug composition
  derives its tenant only from an authenticated, opaque viewer capability and
  reads U07 activity through safe statuses and accessible summaries. It is not
  yet a browser, HTTP, SSO or streaming-control-plane implementation.
- U35: final eligibility gate. It is a deterministic, side-effect-free check
  over a supported U14 report, a mechanism-proxy U16 bridge and its U20 plan;
  it can only mark a proposal eligible and never asserts an outcome or causes
  execution, registry mutation, release or customer exposure.
- U22: governed published-memory admission. A second run receives only an
  opaque provenance capability after U33 atomically predicates the exact
  request, grant revision resolved inside its conditional boundary, live head
  and snapshot before recording its receipt.
  No raw pages,
  cache, publication, Scout, runtime or release capability is exposed.
- U17: sealed change compiler. Only an opaque authorization composed from a
  U35 eligible proposal and U16 mechanism proxy can compile an untrusted Flow
  into immutable drafts. Its full scope, route, exact canonical Flow body and
  typed executable write predicate are rechecked; U18 remains responsible for
  atomically evaluating that predicate and writing to the registry. This slice
  neither persists, executes nor releases a candidate.
- U18 / Issue #48: governed registry writer. A crate-private conditional-write
  adapter accepts only U17 compiled drafts, independently reconstructs and
  verifies their sealed authorization/material, evaluates `EntityAbsent` at
  the final write boundary, and freezes the exact payload plus an idempotent
  candidate receipt per tenant. The adapter is an in-memory executable
  durability contract only: it does not call Agent Core, execute/evaluate a
  candidate, publish/release it, or prove production transaction durability.
- U19-0 / Issue #51: sealed native-evaluation admission boundary. It admits an
  opaque request only from a fresh, revocable and expiring Agent Core registry
  readback that binds the full U09/U20/Core candidate/suite/evaluator identity.
  Its opaque internal readback rechecks the same registry evidence before
  attribution; a future Unit 6 transport must supply a sealed readback issuer/
  repository rather than treating HTTP as evidence. It has no dispatch,
  runtime, verdict or `EvalRun`; U19 native execution remains blocked until
  U18-E supplies real registered-candidate evidence and Agent Core Unit 6
  supplies its contractual `EvalPort`, evaluation harness and sandbox. Those
  missing dependencies remain `dependency_unavailable`, never a simulated
  native result.
- U08-E / Issue #50: E0 read-only query receipt adapter. It accepts only an
  opaque candidate reloaded from U08's live governed ledger plus a U04-B V2
  projection. An explicit private mapping binds the U08 artifact-content
  reference to the sealed U04 SourceSnapshot identity (their digest domains
  are intentionally not compared); it also binds exact verified source
  table/rows. Field authorization comes from the actual `TableInput`
  projection and includes filter reads. Public receipt digests are integrity
  checks, not E0 authentication; the E0 result has only an unconstructable
  opaque capability marker, not an attestation claim. It blocks labels, later joins, future/cross-tenant or
  divergent snapshot mapping/evidence and source writes. It does not implement U12-E,
  Scout, model/Agent Core access, evaluation or release.
- U12-E / Issue #52: deterministic E0 diagnostic sensor. It accepts only the
  opaque U08-E result after its U04-B projection has been verified, then emits
  an immutable descriptive boolean-rate signal with numerator, denominator,
  missingness, virtual window/cutoff and complete source/projection/receipt
  commitments. Its only initial metric is a reviewed, versioned allowlist
  policy for the descriptive `technical_error` flag; no caller can provide
  `resolved`, labels, outcomes or arbitrary fields. It fails closed on evidence
  or context drift and does not infer labels/outcomes, access sources, execute
  SQL, call models/Agent Core, create claims or authorize a write/release. The
  older generic `DeterministicSensor` remains separate because its public
  `QueryResult` input is not E0 evidence.
- U23-P / Issue #47: the narrow Frozen/Continuous temporal protocol over
  U04-B's replay cutoff and U22/U33 governed admission. An opaque, trusted
  evidence commitment (not caller-provided timestamps) is part of the U33
  canonical receipt; Frozen forbids outcome feedback and Continuous requires
  an already-observable, provenanced outcome. The current non-test U04-B path
  admits Frozen only until U20-E/U27 supplies a sealed outcome adapter. This is not the
  full U23 E0 runner, CampaignManifest, scoring, or replay-result flow; those
  remain dependent on U09, U20-E and U27.
- P4 U33 temporal receipt persistence: migration `0004` binds the opaque U23
  commitment and one event reference to a live scoped head under a replay
  cutoff, with exact retry idempotency and fail-closed revocation/scope checks.
  The gated PostgreSQL 18 integration test passed against real U02/U33 tables,
  including concurrent exact retries. A Rust
  adapter/composition that invokes this only after U22/U23 and U05 admission,
  plus event→successor-run wiring, remains pending; this slice does not claim
  autonomous iteration.

## Still pending for the complete product flow

- Native Agent Core/Jev invocation and artifact execution are not connected;
  local Scout, proposal and holdout outputs remain simulation/adapter evidence.
- The E0 holdout checks descriptive recurrence only. Causal/business lift,
  customer resolution, and successful automation cannot be inferred from this
  augmented sample and have not been measured.
- The full stateful evaluation/release loop is not yet integrated end to end.
  U26 is an isolated synthetic-bank sandbox contract, not proof of a complete
  E0 candidate-vs-baseline deployment or canary flow.

## Delivery discipline

Changes enter this branch only after their own RED/GREEN evidence and an
independent adversarial review. The branch is not evidence of release or merge
readiness. See `docs/gaps/OPEN_GAPS.md` for dependencies that require a human
owner; no such gap is currently recorded.
