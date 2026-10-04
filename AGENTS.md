# Improvement engine — agent contract

Read CONTEXT.md and relevant docs/adr before changes. Implement detection and autonomous improvement only. Agent Core executes Agent/Flow/Jev primitives; never build another runtime or an LLM gateway. Infrastructure belongs to sibling infra.

## Development discipline

- Windows/PowerShell first; do not move development to WSL without documented incompatibilities and a decision.
- Feature branches, isolated concurrent worktrees, small PRs, clear commits/tags. No direct main feature work.
- One vertical behavior at a time: RED for the correct reason, minimal GREEN, refactor. Prefer real PG/S3/sandbox dependencies; mock external platform boundaries explicitly.
- Independent adversarial reviewers and an integrator close each slice. Structural checks do not prove runtime, safety or improvement.
- Source data immutable; no PII, source datasets, credentials, recordings or state in Git. Preserve privacy/holdout/time cutoffs even in debugging.
- Every slice updates its technical docs and journal: behavior, commands actually run, results, trade-offs and limitations. No large documentation-only claim of implementation.
- No paid external calls, AWS deployment or new access without scoped authorization. External missing capabilities remain dependency_blocked.

## Current verification

The engine has implemented, independently reviewed slices for platform
observation ingestion (U29), durable run control (U34), and autonomous Scout
drafts (U13). Verification remains slice-specific: inspect the relevant
journal, PR/commit and current CI before claiming a behavior is green. Do not
advertise product health from a structural check alone.

The cumulative delivery state is recorded in `docs/IMPLEMENTATION_STATUS.md`.
Human-owned external blockers go in `docs/gaps/OPEN_GAPS.md`; do not stop
unrelated implementation while one is open.

## Agent skills

### Issue tracker

GitHub Issues in pulso-factored/improvement-engine. See docs/agents/issue-tracker.md.

### Triage labels

Canonical vocabulary in docs/agents/triage-labels.md.

### Domain docs

Single context CONTEXT.md and docs/adr/. See docs/agents/domain.md.

## Ownership, governance and data classes

- Path ownership: `OWNERS.md` (one lane per path; checked by `docs/agents/tests/test_owners.py`).
- Journal tags, state table and Team trailer: `docs/agents/governance.md`.
- Codex lanes: `docs/agents/codex-onboarding.md`.
- Data-class rule for all developer agents: work from schemas, digests and synthetic
  fixtures, never raw E0 text, CSV sources or other original data; both Claude and
  Codex are hosted third parties.
