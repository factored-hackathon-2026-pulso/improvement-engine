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

Empty application bootstrap: no executable engine yet. Do not advertise cargo tests or product health as verified. Add tested commands with the first code slice.

## Agent skills

### Issue tracker

GitHub Issues in pulso-factored/improvement-engine. See docs/agents/issue-tracker.md.

### Triage labels

Canonical vocabulary in docs/agents/triage-labels.md.

### Domain docs

Single context CONTEXT.md and docs/adr/. See docs/agents/domain.md.
