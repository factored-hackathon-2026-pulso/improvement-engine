# WDX and DPLAT bounded facades and source-policy guard

## Scope

This journal records a bounded WDX public-facade slice and the PL-C4/read-plan
guard portion of DPLAT. Both are accumulated into the existing Codex draft PR
#95; neither creates a separate worktree or pull request.

## WDX facade

- Re-export the G0gr-requested paired-evaluation and platform-projection types
  through `facade_steps` and selected crate-root names.
- Keep `PlatformSourceReader` under `facade_steps::provisional`: its current
  result shape returns no rows and is expected to change when the concrete
  adapter contract is defined.
- Compile the consumer test as an integration target, so exports must be
  nameable outside the crate. A compile-fail doctest protects the private
  treatment constructor.
- Exporting caller-supplied fixture DTOs does not authenticate sandbox
  execution or upgrade their evidence provenance.

## DPLAT policy guard

- A caller-declared `ReadWrite` mode is rejected before any reader callback;
  the default allow-list still excludes credential relations.
- `PlatformSourceAccessMode` is a declaration, not an authority-issued
  capability. `execute_read_plan` is not a database snapshot operation and
  does not guarantee a consistent cut, authenticate permissions, or sandbox a
  reader. A future concrete adapter must implement and test those properties
  against its versioned source contract.
- This is not full DPLAT or platform-live support: there is no concrete DB or
  exporter reader, row return/materialization, snapshot identity/digest,
  event-to-observation mapping, capability outcome, or complete V3 §32 flow.

## Verification

- TDD RED: the consumer integration test initially failed because the curated
  root exports did not exist. A default-feature attempt failed during unrelated
  crate compilation due to existing `local-simulation`-gated symbols; rerunning
  with the workspace's `local-simulation` feature reached the intended RED.
- GREEN before final polish: consumer facade test 1/1 and platform-source test
  7/7. After renaming the policy API to avoid snapshot/authorization claims,
  both focused targets passed together (1/1 and 7/7); rustfmt was then applied
  to the consumer test. Final full local CI is pending at the time this journal
  entry was authored and must be recorded separately in the shared bitacora.
- Independent adversarial review caught (and closed) premature stable
  publication of `PlatformSourceReader`, and false implications of a
  consistent snapshot/authenticated grant. The API was moved to a clearly
  provisional module and renamed to caller-declared policy semantics.

## Remaining follow-up

The present guard is useful fail-closed policy plumbing only. Do not claim the
platform source is production-ready. A separately contract-backed adapter
slice must define snapshot consistency, source revision/digest, rows, mapping,
late/gap behavior, and the concrete read-only credential boundary before
those guarantees can be claimed.

## Final local validation — 2026-10-04

- Focused facade consumer and platform-source policy tests passed: 1/1 and 7/7.
- Full `scripts/verify-local-ci.ps1` exited 0 with one Cargo job. Core Rust
  unit tests: 200 passed, 2 opt-in PostgreSQL tests ignored. Rust integration,
  source adapter, CLI, doctest and workspace checks passed; Python contracts
  passed (16, one opt-in skip); contract fixture validation passed; Pester
  passed (20 + 5). No PostgreSQL test or GitHub Actions success is claimed.
- No container, Podman machine, or external service was started.
