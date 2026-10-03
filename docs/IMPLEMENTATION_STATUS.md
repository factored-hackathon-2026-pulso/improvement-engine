# Cumulative implementation status

This document describes capabilities present in the consolidated code tree.
It distinguishes an implemented boundary from an integrated product flow;
a green unit test does not prove the broader product is complete.

PRs #69 and #71 originally merged into feature branches after their parent PRs
had already merged into main. The restore-main consolidation ports their unique
commits onto main's #68 baseline, preserving unrelated main capabilities and
repairing the malformed/duplicated source privacy fixture. See journal 0055.

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

- P2 signal portfolio (isolated composition boundary): preserves each
  independently qualifying opaque E0 diagnostic signal as a stable,
  provenance-bearing candidate record; rejects mixed tenant/run/grant/
  authority/snapshot/cutoff scope; keeps unsupported and unknown metrics
  explicit without fabricated zero counts; and distinguishes a measured
  `no_qualifying_signals` result from `insufficient_evidence` when unavailable
  metrics or a zero-known-denominator measurement leave no qualifying
  candidate, and for an empty input portfolio (reason `no_signals_provided`;
  never a negative finding). This boundary is not wired into
  local_simulation, original-source discovery, or Agent Core. It currently
  accepts the authenticated E0 diagnostic contract only; see
  docs/data/signal-portfolio.md and journal 0057.

## Implemented boundaries and remaining integration limits

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

- P1 platform discovery-input seam (current feature branch): measured U30
  signals can be converted to a non-forgeable, provenance-preserving typed
  input for hypothesis generation, bound to the tenant in the U29 projection
  and checked against the active task scope; insufficient evidence and
  cross-tenant replay are rejected. This is not yet U13 Scout integration or an Opportunity/proposal. U13 currently
  requires U08/U09/U10 receipts or E0-specific U04/U08/U12 evidence; a trusted
  platform-specific invocation/receipt contract is still required before the
  input can traverse Scout. See journal 0052.

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
  E0 tool-call retry counts are now carried through the safe event projection
  as a separate descriptive metric (`e0_tool_retry_case_rate`), between direct
  technical errors and opaque recurrence in primary-signal priority. Its rate
  counts distinct cases with a known retry count in the denominator; absent or
  null counts remain explicitly missing. Retries do not establish cause or
  savings. This signal is aggregate-only and may produce only the existing
  simulated, unverified, non-executable review draft.
  Proposal evidence now includes a case-level retry/technical-error
  co-occurrence diagnostic. It uses only cases with known retry-positive and
  technical-error status; both the co-occurring and non-co-occurring cells
  must meet a versioned minimum of five before counts/rates are emitted under
  `e0_retry_error_overlap_k_v2`.
  Unknown technical-error status among retry-positive cases is excluded from
  the denominator and its count is not serialized. The diagnostic is
  descriptive association, not cause or direction.
  Its statuses distinguish incomplete retry-count coverage
  (`insufficient_retry_status_coverage`), reportable support, and all other
  complete-coverage cases (`suppressed_below_minimum_support`). Missing
  technical-error status, zero support, and small support share the same
  generic suppressed status; none exposes whether positive retries were
  observed. No overlap status specifically names zero retries, and no
  counts/rates/direction are emitted unless both cells meet k.
  Completeness is per case over `tool_call` events: each discovery case must
  contain at least one such event and every call must have a retry count. A
  case with no `tool_call`, or with mixed known/missing call counts, is
  incomplete even if a known call is positive. Any incomplete case makes
  overlap values unavailable; missing status is never called no retries or no
  errors.
  The primary-signal policy is versioned as `local_primary_signal_v3` because
  adding retries changes candidate-selection priority; v2 remains historical.
  If the optional `copilot_query` source table is absent, recurrence is marked
  unavailable rather than reported as zero. The signal proposal is a
  simulated, unverified, non-executable `unclassified_candidate`; it makes no
  semantic, causal, lift, or real-bank claim. Only the selected supported
  signal proceeds into the local Scout/verifier path; zero positive support
  produces an explicit no-op. The formal route is always `do_nothing`.
  Actual local E0 smoke (2026-10-02) used 200 Arranque cases; the total
  Reproduccion population is intentionally suppressed. The leading opaque query signature recurred in
  154/200 cases (policy floor 20); technical errors remained 0/187 supported,
  with 13 missing. The overlap status is
  `insufficient_retry_status_coverage`; it does not treat absent ToolCall rows
  or missing call counts as zero retries. The runner recorded three Scout candidates and one
  exploratory proposal. Persisted output contains the exact
  configured cutoff and no known PII sentinels or evaluator labels.
  This demonstrates only bounded local detection/simulation behavior, not
  native Agent Core execution, causal validation, release, or business lift.
  A separate P2 core API now deterministically assembles all qualifying E0
  portfolio signals into descriptive proposal seeds with run/snapshot, signal,
  and summary-commitment provenance. It validates three-metric disposition
  coverage, recomputes qualification and coverage, and rejects stale summary
  changes. The runner now persists the assembler output under
  `proposal_assembly` for E0 only; OriginalBank does not receive an E0
  portfolio or assembly. E0 also persists one sanitized aggregate
  `proposal_assembly` RunEvent after any holdout event, identically in
  `result.json` and `events.ndjson`; OriginalBank does not emit it. This is
  local run observability, not a native Agent Core
  artifact or a complete evaluation flow. The commitment is unkeyed staleness
  detection, not authenticity; these seeds are not U08/U13 or Agent Core
  evidence, evaluated changes, or business lift. Focused runner tests pass
  24/24 for this slice. Prior actual-data E0 and OriginalBank smoke runs both
  exited 0, and the full eight-gate local CI passed before this event addition.
  See `docs/data/e0-proposal-assembly.md` and journals 0061–0063.
  For a qualifying E0 recurring-query candidate, the runner now serializes
  P3's candidate-bound mechanism evidence and exact route resolution in
  `e0_mechanism_resolution`. Evidence is labeled `e0_local_run`; the empty
  catalog is separately labeled `team_generated_empty_local_catalog_fixture`
  with ephemeral durability. It returns `unlinked` because no exact mapping
  was supplied. This is not a Core registry read, Flow existence/execution,
  evaluation, or business-lift result. A run without a qualifying candidate
  has no mechanism packet/event; OriginalBank remains without E0 output.
  Focused runner tests pass 24/24 for this follow-on. See journal 0064.
  A typed E0 `e0_investigation_proposal_plan` now composes the descriptive
  proposal seed, exact candidate-bound evidence packet and exact route/catalog
  resolution. It includes a deterministic content digest and a bounded
  pending-review decision envelope: unlinked -> `investigate_mapping` or
  `do_nothing`; mapped contract fixture -> read-only `investigate_mapped_flow`
  or `do_nothing`. The artifact is explicitly not an Agent Core Proposal,
  grants no authority and is non-executable. It is persisted with one
  contiguous, privacy-safe event after mechanism resolution. Runs without a
  qualifying recurring-query candidate and OriginalBank receive no plan/event.
  No Core execution, evaluation, causality or lift is claimed. Focused Core and
  Runner suites, core doctests, formatting, and all-target Clippy passed; full
  local CI and actual-source smoke are pending. The resolver receipt is opaque,
  resolver-minted, and bound to the exact evidence packet, so callers cannot
  reconstruct it from public route fields. See journal 0065.
  The CLI additionally supports opt-in `--progress-jsonl` diagnostics on
  stderr: flushed phase-start/completion/skip/failure records with monotonic
  elapsed milliseconds and no source values, identifiers, paths, proposal
  text, or raw errors. This is live process progress only; the final domain
  timeline remains atomically persisted, and no durable U07, OpenTelemetry,
  health endpoint, or production-monitoring claim is added. See journal 0053.
  Original-bank local execution now supports an independently sealed,
  privacy-safe snapshot projection for call-center reason × channel. The flat
  API metric `record_count` counts CSV records and does not deduplicate
  `interaction_id`; it is still not an event-date cohort or cutoff-filtered
  result. A separate descriptive projection groups valid literal source months
  × reason × channel, with `coverage=partial` and `final_extract_facts_only`.
  Naive timestamps are never assigned a timezone or compared with the run
  cutoff. Its k threshold applies within each month × reason × channel cell;
  only rows with valid month and usable grouping labels enter that cell, and
  only k-qualified cells contribute to `supported_contact_count`. Rejected
  rows and suppressed cells never increase a visible cell denominator; their
  exact counts are omitted from agent-facing serialization and `result.json`.
  When complaint cells qualify, the runner emits a finding
  and a local descriptive proposal envelope bound to snapshot, manifest, and
  projection digests. It is not a U13/Agent Core candidate:
  `dependency_blocked_snapshot_semantics`, `simulated_unverified`,
  `not_executed`, `publication_eligible=false`, formal route `do_nothing`.
  Agent Core/U13 has no offline snapshot-candidate contract; no query receipt,
  candidate admission, artifact compilation, release, cause, ROI, or outcome
  claim is fabricated. Customer IDs, row-level facts, free text, and contact
  outcomes are not used. `k=5` is fixed on the discovery-facing CLI path; no
  per-run override is accepted, avoiding easy differencing across comparable
  runs. Policy version and threshold are sealed into the source manifest and
  revalidated at the core boundary. Synthetic CLI E2E verifies this boundary.
- Windows local snapshot wrappers: scripts/run-local-e0-e2e.ps1 now accepts
  `e0` or `original`; scripts/run-local-snapshots-e2e.ps1 runs both sequentially
  into separate `e0/` and `original/` output directories below one fresh root.
  Both use the explicit local-simulation CLI with locked/offline Cargo and a
  required UTC cutoff. E0 keeps its Arranque/support defaults; original uses a
  distinct summary and accepts only a partial, final-extract descriptive draft
  marked unverified, not executed, non-publishable and Agent-Core-blocked.
  Existing output roots, input/output overlap and observed reparse-point paths
  fail closed before Cargo runs.
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
- P1 platform discovery-input seam: measured U30 signals are carried into a
  bounded platform Scout candidate with tenant/job/grant/authority, metric,
  window, source, and Core/model receipt provenance. P2 adds a deterministic
  one-comparison descriptive-trend verifier. It does not implement full U14,
  infer causes or business lift, create opportunities/proposals, or grant
  execution authority. See
  `docs/data/platform-discovery-verification.md` and journal 0058.
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
  U33-E publication exists locally; U23-E admission re-attests the exact
  published revision against the exact U33-E publication sidecar and creates one
  opaque receipt per publication/scope/run. The trusted crate composition can
  now read a page for the later run only with the admitted run/grant/scope/
  snapshot/clock binding, while the scratch authority rechecks grant liveness.
  This remains a crate-private local semantic path, not wired into the CLI or
  durable runtime.
- U33-E: Frozen E0 summary publication boundary. A crate-private composer
  redeems only the opaque U15-EQ preparation after recomputing the exact
  U13-A/U14-E/U14-EQ/U04-B and canonical U15 transform chain. It emits an
  opaque publication capability, not MemoryUse, proposal, route or release
  authority. The local adapter models head-CAS/idempotent provenance-sidecar
  writes; the durable adapter is intentionally `DependencyUnavailable` until
  both the U33-E publication sidecar and transaction-bound U05 grant
  revision/liveness contract exist. Current PostgreSQL migrations persist U02
  revisions, generic U33 heads/tombstones/use receipts, and temporal cutoff
  receipts, but have no U33-E publication record and accept grant reference as
  a string rather than resolving its authority. U23-E is the sole consumer
  allowed to attest governed use:
  its local path checks the exact immutable publication revision,
  tenant/world/scope, replay cutoff, current grant liveness and explicit
  snapshot/ancestor revocation, and makes retries idempotent by
  `(publication_commitment, scope, run_id)`. A changed grant or access time for
  the same semantic use conflicts without a second receipt. Durable admission
  remains `DependencyUnavailable` until one database transaction can lock and
  resolve a U05 `GrantSnapshot`/`AuthorityDecision` (authority, tenant, grant
  revision, liveness/validity, revocation epoch, action, scope/snapshot binding
  and authorization digest), re-attest the U33-E publication/head and snapshot,
  then append one payload-free receipt. A pre-read bool or parallel grant
  table is not sufficient. The durable contract gap and required real-Postgres
  acceptance tests are recorded in `docs/journal/0053-p4-u23e-frozen-memory-admission.md`.
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
  reads legacy U07 activity through safe statuses and accessible summaries.
  The cumulative branch now adds a distinct V2 read adapter/composer over
  `pulso_run_events`, using `run_ref + after_sequence`, tenant-derived SQL
  scope, a 1..100 page bound, and a version-1 closed allowlist that maps
  unknown stored event vocabulary to `other`. The V2 request cannot select a tenant; the
  composer remains read-only and maps cross-tenant/missing runs to one safe
  `NotFound`. Its ignored, in-crate PostgreSQL integration test is present but
  the real-DB gate remains pending CI because local Windows PostgreSQL cannot
  bind a port. The reader stays `pub(crate)`; any future cross-crate endpoint
  must accept auth-issued context rather than a tenant string.
  This does not implement browser, HTTP, SSO, streaming, graph, or operator
  control-plane features.
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

## Local consolidated E0 + OriginalBank run (2026-10-03)

- The accumulated branch `feat/demo-e2e-proposal-resolution` passed all eight
  gates in `scripts/verify-local-ci.ps1` on Windows with Cargo 1.98.1,
  `CARGO_BUILD_JOBS=2`, and isolated target
  `target-local-demo-e2e-consolidation`. This included format, workspace
  Clippy, Rust unit/integration tests, runner/CLI tests, PowerShell contract
  tests, and local CI preflight. One destructive real-PostgreSQL test remains
  intentionally ignored unless its explicit isolated-database opt-in is set.
- The documented combined runner completed on both local sources into separate
  output directories under `output/snapshot-runs-2026-10-03-c`.
- E0: 200 discovery cases; one candidate; recurring copilot-query recurrence
  measured on all 200; selected holdout replicated 1,433/1,539 and explicitly
  `descriptive_only`; one proposal seed with
  `simulated_unverified/not_executed`; route `do_nothing`; the mechanism result
  is `unlinked/no_exact_supported_flow_mapping` against an explicitly labeled
  ephemeral empty catalog. Its result timeline has 15 contiguous activities,
  ending with holdout → proposal assembly → mechanism resolution.
- OriginalBank: all 7,671 source files (5,349,322,481 bytes) were inventoried;
  the contact projection completed over 1,097 files. The run produced a
  snapshot-only descriptive finding, no E0 proposal assembly and no E0
  mechanism resolution. Its six-activity timeline ended `complete`.
- These runs demonstrate an executable, source-separated local detection →
  descriptive seed/evidence → explicit route disposition path. They do not
  demonstrate a mapped Core artifact, real agent execution, causal resolution,
  business lift, or autonomous improvement. The measured holdout is not an
  outcome/efficacy test.

### Final-head revalidation after the E0 investigation-plan slice

- The complete eight-gate local CI was rerun on the final consolidated HEAD
  `11615ae2718f1949619229df452bf8c3618ad524` and passed. The Core suite had
  168 passed, 0 failed, and 1 intentionally ignored destructive PostgreSQL
  integration test; the local CI script's Rust, Python contract, fixture, and
  Pester gates were green.
- The final-head E0 actual-data run completed as
  `run_2588_1791058612421441700`: 200 discovery cases, 154 recurring-query
  cases, one descriptive candidate, and selected holdout recurrence 1,433 /
  1,539 (`descriptive_only`). The plan is
  `e0_read_only_investigation_plan_not_agent_core_proposal`, recommends
  `investigate_mapping`, is `pending_review` and `not_executable`, and records
  `unlinked/no_exact_supported_flow_mapping`. The formal route remains
  `do_nothing`; the timeline has 16 contiguous events and ends with the plan
  event. No business lift or resolution is claimed.
- A fresh OriginalBank rerun was started after E0, but the large source scan
  was stopped before completion after unusually slow progress (3,696 / 7,671
  files and about 610 MB / 5.35 GB). The completed OriginalBank smoke under
  `output/snapshot-runs-2026-10-03-c` is from the preceding consolidated
  revision, before the E0-only plan addition: it inventoried all 7,671 files,
  projected contacts from 1,097 files, and emitted no E0 artifacts. On the
  final HEAD, the full regression suite passed, including the explicit
  OriginalBank/no-plan contract. Treat the final-head OriginalBank live scan as
  not re-run, not as a failed test.

## Delivery discipline

Changes enter this branch only after their own RED/GREEN evidence and an
independent adversarial review. The branch is not evidence of release or merge
readiness. See `docs/gaps/OPEN_GAPS.md` for dependencies that require a human
owner; no such gap is currently recorded.
