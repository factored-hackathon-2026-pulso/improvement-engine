# debug-console (L7, fixture-backed slice)

Purpose: `debug-console/` is the internal backoffice of the engine's detection and self-improvement system (spec V3
section 25): the operations and observability surface for the team that owns detection and self-improvement. It is not a
product UI and not the support-platform backoffice, which section 25.1 defers and keeps out of Pulso's scope. It is
read-first; the only writes are the versioned, audited debug commands of section 25, gated by the server's
`available_commands[]` and the caller's scope.

Commands: `npm ci`, `npm run typecheck|test|test:contract|build|test:e2e|fixture|dev`.
The e2e run starts the fixture API/SSE (:4010) and Vite (:5173, same-origin proxy).
`npm run test:contract` validates every fixture response with the zod schemas and checks transport rules
(`CONTRACT_TARGET=fixture|real`, `CONTRACT_BASE_URL` for real; it spawns its own fixture on :4011).

## What exists

- SSE client: own fetch reader, jittered exponential backoff (1 s .. 30 s, plan 16.13.4), `Last-Event-ID` resume,
  410 -> re-read the snapshot with `recovery_after_sequence` as the floor, show an "earlier history purged" banner (a second consecutive 410 backs off),
  stale after 2x the heartbeat without any event or heartbeat comment (`config.json` `sseHeartbeatMs`), 401/403/404 -> terminal state without retry.
  Reconnect never moves focus.
- Schemas: routes plan 16.10 does not define (investigation, gates, diff, memory, decision, session, profile, step-up) are marked
  `consumer_proposal` (zod description, registry `CONSUMER_PROPOSALS` with the R/M/CO/CLQ reference).
- `clientScrubber`: masks JWS/Bearer/DSN-shaped strings, `final_locked` refs, secret-like keys and canaries in every API/SSE payload and
  reports `security.unredacted_payload` to the console (kinds and counts only, never the value).
- Fixture control plane: `/__fixture/{reset,scenario,emit,deliver,fault,cut,state}`. Scenarios live in `fixtures/scenarios.mjs`
  (`SCENARIOS`, each with a machine-readable `expect`; `F_COVERAGE` maps F01-F25 to a scenario/control or a named gap).
- A11y: axe (critical/serious, wcag2a/aa + best-practice) on S1, S2 x3, S7 and an open drawer, keyboard walkthrough, text-equivalent
  coverage, reduced-motion check, and a harness self-check that axe flags a bad page. Reported as "no axe critical/serious
  violations on the listed screens", not as WCAG compliance; screen-reader review is `not_run`.
- Image: `Dockerfile` (node build stage, `nginx-unprivileged` runtime, :3000, `/healthz`, same-origin proxy to `PULSO_CONTROL_API_URL`),
  CSP without `unsafe-inline` in `nginx/security-headers.inc` (asserted by `tests/unit/csp.test.ts`).
- UI strings: es-419 dictionary in `src/i18n/es419.ts`; a unit test rejects hard-coded JSX text and missing keys. Codes stay verbatim.

## Added since the first slice

- Typed `DebugApi` seam (`src/api/debug/`, PR #85): one port with three providers, `http` (real same-origin transport,
  the default), `fixture` and `stand-in`, selected by `dataProvider` in `public/config.json`. It carries the spec 25 DTOs,
  SSE resume with deduplication and gap backfill, 410 snapshot recovery, a capped SSE frame buffer, CSRF and
  idempotency keys on commands, and command gating (server `available_commands[]` AND scope). Review notes:
  [`journal-c-0002-review.md`](journal-c-0002-review.md).
- Sources view (PR #80, `src/features/SourcesView.tsx`): read-only view of the platform source: capability profile,
  detector eligibility, exporter counters and insights, using the real exporter vocabulary.
- Demo panels (PR #77, `src/features/DemoPanels.tsx`): the demo steps with each step labelled `real`, `stand-in` or
  `simulated`, fed by `demo/` output.
- Origin of the Sources and demo data: [`journal-pl-0005.md`](journal-pl-0005.md), [`journal-c-0001.md`](journal-c-0001.md).

- Live side-panels: `doubles_declared` refetches the profile (mode banner) and `gates_set` refetches the run's gates, also when caught up after a reconnect; debounced 150 ms (`src/state/sideRefresh.ts`), latest response wins, the Gates panel is not blanked during a refetch and focus never moves (`tests/component/RunLiveRefresh.test.tsx`). Closes the stale-banner/Gates gap noted in `docs/reports/demo-magic/README.md`.

## Review-defect closure

- Single rate-limited live region (`src/a11y`), only changed nodes are announced; no other `role=status/alert/aria-live`.
- Session: a failed session fetch or any 401 shows a session banner; mutations are never sent with an empty CSRF; step-up errors are reported.
- F11 (degraded trace panel when `trace_id` is null) and F21 (decision CAS: `expected_revision`, 409 `stale_revision`, reload with a new key) have UI and fixtures.
- Fixture sets the loopback-HTTP cookie `pulso_local_session` (HttpOnly, SameSite=Lax, Path=/, no Domain/Secure); canary HAR/cookie/storage tests in `tests/e2e/leaks.spec.ts`.
- axe for the S3-S6 panels in their states (`tests/e2e/a11y-panels.spec.ts`); `ModeBanner` and `DecisionPanel` component tests.
- Dockerfile base images are digest-pinned; build with `podman build --format docker` or HEALTHCHECK is dropped.

## Known gaps vs plan 17.3.7

Still stand-in or fixture: there is no Rust `control-api` HTTP surface yet (Codex-owned, not published), so `provider=http`
has no real server behind it; the run, sources and memory screens still use the legacy client under a fixture or stand-in
declaration and show a `provider-partial` notice; field names of several debug routes and the 410 body shape are
provisional until the control-api contract is published; no screen consumes `commandState` yet, so commands are not
exposed in the UI.

- No react-router / react-query / react-virtual (hash routing, plain fetch, no virtualised timeline); S3-S6 are panels of the run view, not routes.
- `stale_revision` as the 409 code and `trace_id` on graph nodes are consumer assumptions until Codex records them; traceLinkOrigins links are not rendered.
- No evidence manifest, `LocalServiceManifest` fragment or CI patch yet.
- The 410/reconnect/axe/leak checks were authored by the implementer; the plan requires an independent reviewer for a11y/security.

## Verification (local CI parity)

`debug-console/` is outside the Cargo workspace (`members` lists only `crates/*`) and is not referenced by `.github/workflows/ci.yml`,
so the Rust CI job is unaffected by this directory. The console is verified locally:
`npm ci && npm run typecheck && npm test && npm run test:contract && npm run build`, then `npx playwright test <spec>` one spec at a time,
then `podman --connection pulso-dev build --format docker -t pulso-debug-console debug-console`.
No CI job runs these yet; wiring one is a Codex-owned workflow change, tracked as a row in
[`../../docs/gaps/OPEN_GAPS.md`](../../docs/gaps/OPEN_GAPS.md).
