# Journal claude-0004: L3b tools, protected writer, facts and stages

Contract revision: `pulso-two-teams-1`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3`. Package: L3b (plan 17.3.3).
Code: `src/pulso_core_runtime/{tools,facts,stages}/`, `tools/factory.py`. Commits: 0fb361b, 60042a9, 0190f95, 669cfb8,
5a09323, 447772b, b3af773, 4984d92. Documentation pass at HEAD `4984d92`.

## Purpose
One `ToolExecutor` for every stage (Core has no `executor_ref`): the closed `pulso/*` catalogue plus the protected
`registry/*` writer tools, the binding guards, the fact whitelist with strict and Core-subset schemas, and the stage
catalogue that must equal the Flows in `agent-core-assets`.

## Flow
`PulsoToolDispatcher.execute`: registered tool, context via signed attrs, binding confirmed, stage allow-list, closed args,
broker authorization, handler (see `core-binding-context-channel.md`). Registry tools go to
`ProtectedBuilderToolExecutor` (ADR 0003). Catalogue: `bind_context`, `lab_query`, `lab_get_result` (handler only, no
ToolDef: folded into `lab_query`), `wiki_read`, `wiki_explore`, `wiki_transform`, `artifact_get`. Projection
(`facts/whitelist.py::project_result`): only the stage whitelist leaves the process; slots, token map, actions, principal,
decisions and pages are dropped; fact <= 128 KiB, result <= 256 KiB; non-integer JSON numbers forbidden (decimals are
strings); `source_kind=agent` is never promoted; evidence refs are `ArtifactRef` objects validated against the invocation
artifact log; canaries scan the unescaped text. The writer fact `pulso_writer_receipts` is composed by the bridge from
verify-node facts and `RunState.actions`.

## Input / output
Tool args are closed per tool and never carry tenant, job, grant or binding fields. Stage facts: `pulso_hypotheses`
(scout), `pulso_verification` (verifier), `pulso_change_spec` (builder_design), `pulso_writer_receipts` (writer), shapes per
annex D.2 (`facts/schemas/*.strict.json`; Core-subset copies in `stages/catalog.py`).

## Transactions and idempotency
Write keys `pulso-w:<sha256(command_key|stage|ordinal)[:48]>` with the ordinal from the committed `operations` array;
evaluate key `pulso-eval:<ref>`; one create per invocation; every draft write is followed by its own verify by
idempotency key. Key map per binding is atomic; `get_write` is scoped to this command's keys.

## Errors
Tool statuses: `denied` (`pulso:context_missing|context_mismatch|binding_unconfirmed|authorization_denied|
commitment_mismatch|write_without_key|evaluation_gate_unavailable`, `tool_not_allowed`, `invalid_args`), `timeout`
(`pulso:broker_timeout`), `error` (`pulso:broker_<status>:<code>`, `pulso:tool_internal_error`), `uncertain` for write
exceptions. Fact errors: `output_missing`, `fact_schema_violation`, `output_too_large`.

## Permissions
Broker authorization before every broker call and every write (`allowed=false`, 5xx, timeout or expired `valid_until` are
denials; caching only until `valid_until` for the exact operation, resources and digest). Broker JWT: `aud=lab-broker`, executor
key (ADR 0005); binding callback: callback key.

## Config
`PULSO_LAB_BROKER_URL`, `PULSO_CONTROL_API_URL`, executor and callback signer files. Stage tool allow-lists live in
`stages/catalog.py` and must equal each Agent's `tools_allowed` (CI: `check_against_assets`).

## Observability
`leak_signal` hook for canary hits; broker error codes are surfaced as closed strings; no payload logging.

## Commands (head `4984d92`)
`python -m pytest -c pyproject.toml tests/l3b -p no:cacheprovider` (no Postgres needed). At `4984d92` the shared pinned venv
`%TEMP%\pulso-wire-venv-86a7674` lacked `jsonschema`, so `test_catalog_and_facts.py`, `test_d2_facts_and_ordinals.py` and
`test_review_fixes.py` failed to collect (`ModuleNotFoundError: jsonschema` from `facts/whitelist.py`) in a documentation-pass
run; `jsonschema` is declared in `pyproject.toml` and `runtime-requirements.txt`. Environment: Windows 11, Python 3.12.10.

## RED / GREEN
First RED (`tests/l3b/test_binding_denied_first_red.py`): with the binding denied (parametrised over `conflict`,
`digest_mismatch`, `not_found`, `unavailable`, `timeout`) there must be zero lab queries, zero model calls, zero registry
writes (ModuleNotFoundError before implementation). GREEN as reported: 59 passed including 32-way concurrency and the
thread-pool variant; `mypy --strict --ignore-missing-imports` clean on the three packages; later D.2 reconciliation
(669cfb8). About 68 test functions exist. Not re-run in full here (collection issue above).

## Trade-offs
Two schemas per fact (Core accepts only a closed subset of JSON Schema) with a CI assertion that the Core subset is a
relaxation of the strict one. Wrapping the upstream builder executor instead of subclassing avoids private-attribute
coupling but needs a mapping layer for keys.

## Gaps
- `registry/evaluate` ToolDef still declares `suite_id`/`suite_version` args (plan: `{proposal_id}` only); the wrapper
  ignores them for the digest and uses the admission's suite.
- Writer Agent asset and `registry/reopen@1`: reopen path tested end-to-end in tests (0190f95); asset-side status not
  re-verified here.
- `pulso/lab_get_result` has a handler but no ToolDef.
