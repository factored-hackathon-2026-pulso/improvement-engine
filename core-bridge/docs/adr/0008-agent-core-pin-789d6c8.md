# ADR 0008: Bump the agent-core pin 86a7674 -> 789d6c8 (N-01..N-11, llm-gateway, AWS scale-out)

Status: proposed (implementer: Claude; reviewer and integrator approval pending). Contract revision: `pulso-two-teams-1`.

## Context
Upstream agent-core `main` at `789d6c89b2fca90fc10e2abf157da51dc81c5d51` carries our requests N-01..N-11, PR #26
(`HttpLLMGateway`, removes `GATEWAY_TRACER` and the in-process OpenAI-compatible gateway) and PR #27 (AWS scale-out:
connection pool, S3 blobs, SNS publisher, relay; all off by default). `contracts/VERSION` is still `1.3.0`, so the
version is **not** a drift signal. No `.sql` changed (byte-identical scripts), hence no migration and a trivial rollback.
Analysis: `docs/AGENT_CORE_PIN_BUMP_ANALYSIS_CLAUDE.md`; findings status: `docs/AGENT_CORE_PIN_FINDINGS_CLAUDE.md`.

## Decision
1. **Pin identity = SHA + MANIFEST digest, never `contracts/VERSION`.** `gen_wire.py` records the version and prints a note
   if it differs from the reviewed `1.3.0`, but only the SHA (checkout HEAD) and the `manifest_sha256` of `pin.json`
   (when declared) can fail generation. The wire dir is `wire/agent_core@789d6c8/`.
2. **Wire snapshot** now also byte-copies `contracts/registry/` (N-01, 31 schemas, outputs in serialization mode) and the new
   `schemas/RunSummary.json` (194 schemas). We keep deriving (validation mode) the models upstream still does not publish
   (`CandidateView`, `ValidationReport`, `ProposalDetail`, `EvalReport`) plus the models the mock/DTOs already read, and
   `registry_openapi.json`. Request bodies are published upstream as `CreateProposalBody, PutDraftBody, EvaluateBody,
   ApproveBody, PromoteBody, ReasonBody`; our derived `Create/Draft/Evaluate/Approve/Promote/Reason` keep their old names
   (Python classes upstream are still `_Create` etc.). `golden/hash_vectors.json` hash values are byte-identical to the old
   pin and now also carry `release_detail` (the seeded release as served with the N-03 fields).
3. **Runtime composition (`pulso_core_runtime.main`)**: the tracer name is the literal `agent_core.adapters.llm`; `build_sha`
   is `PULSO_CORE_SHA` (fallback: the pinned SHA) so Core's native `/version` reports it; `keys_reload_seconds` is explicit
   (`PULSO_KEYS_RELOAD_SECONDS`, default 5, 0 = off, invalid = exit 2); **`/v1/export/*` is disabled by default**
   (`run_export=None`, enable with `PULSO_CORE_EXPORT_ENABLED=1`; the PG exporter stays the ingest path because the HTTP export
   has no outbox, no raw `event_json` and needs an `exporter` staff credential minted by infra). `problem_response` replaces
   our import of the private `_problem`. `/internal/v1/version` gains `keys_reload_error` (exception type only).
4. **compat.py** (closed list) adds `ReloadingIdentityVerifier`, `ServePorts.run_export`, `build_api_deps(build_sha)`,
   `RegistryService.get_alias/list_events`, `RunExport/RunSummary`, `RELEASE_SETTINGS/ReleaseSettings`, `problem_response`,
   `_Problem`; `assert_compat()` passes at the new SHA and each new entry is covered by a drift test.
5. **Mock/parity**: `platform-sim` mock gains the alias and versions reads (N-02), the four `ReleaseDetail` fields (N-03,
   inherited by published releases) and `eval_run_id` in the `gate_failed` body (N-10); 11 new parity cases; the route table
   is 18. Fixtures were re-recorded for a2, `real_local` and `real_pg_scripted` (all equal). Not simulated by the mock: the
   `release_settings` draft kind (N-07), `/version`, export; a2/real remain the authority for those.
6. **Hot reload of keys (N-09)** does not remove our entrypoint materialisation: Fargate env secrets change only with a new
   task, so rotation still needs a refreshed key file. A corrupted file keeps the previous keys (it cannot revoke).
7. **llm-gateway (PR #26)**: the runtime now needs `AGENTCORE_LLM_GATEWAY_URL` and `AGENTCORE_LLM_GATEWAY_TOKEN` (both or
   neither; neither = generation falls back to templates). Passed through untouched; deployment is an infra change.
   `AGENTCORE_DB_POOL_MAX`, the S3 blob bucket and the SNS publisher are pass-through and stay off.

## Consequences
- Strict consumers of `ReleaseDetail` (Rust DTOs, mock) must accept `interrupts`, `language_detection` (required),
  `injection_ruleset`, `max_input_chars` (required) in the same release.
- Expand/contract (CAP-57) now has an executable proof for this bump (`tests/integration/test_expand_contract.py`): the old
  and the new binaries migrate/seed/operate against each other's schema (scripts are identical, fingerprints equal). Our own
  bridge schema is idempotent and unchanged in this bump and is not part of that proof.
- `release_settings` (N-07) replaces the whole interrupt list: a Pulso-side guardrail must exist before any builder emits it
  (upstream D-17). `ProtectedBuilderToolExecutor` therefore **denies by default** any `registry/put_draft` whose changes
  contain `kind: release_settings` (`pulso:release_settings_not_allowed`, zero effect, checked before the commitment so
  even a committed digest cannot carry it); `tests/l3b/test_release_settings_denied.py`.

## Rollback
Previous image digest, previous pin; no schema change to undo. The old pin checkout stays at `references/agent-core`.
