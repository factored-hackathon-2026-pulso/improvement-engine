# debug-console (L7, fixture-backed slice)

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

## Review-defect closure

- Single rate-limited live region (`src/a11y`), only changed nodes are announced; no other `role=status/alert/aria-live`.
- Session: a failed session fetch or any 401 shows a session banner; mutations are never sent with an empty CSRF; step-up errors are reported.
- F11 (degraded trace panel when `trace_id` is null) and F21 (decision CAS: `expected_revision`, 409 `stale_revision`, reload with a new key) have UI and fixtures.
- Fixture sets the loopback-HTTP cookie `pulso_local_session` (HttpOnly, SameSite=Lax, Path=/, no Domain/Secure); canary HAR/cookie/storage tests in `tests/e2e/leaks.spec.ts`.
- axe for the S3-S6 panels in their states (`tests/e2e/a11y-panels.spec.ts`); `ModeBanner` and `DecisionPanel` component tests.
- Dockerfile base images are digest-pinned; build with `podman build --format docker` or HEALTHCHECK is dropped.

## Known gaps vs plan 17.3.7

- No react-router / react-query / react-virtual (hash routing, plain fetch, no virtualised timeline); S3-S6 are panels of the run view, not routes.
- `stale_revision` as the 409 code and `trace_id` on graph nodes are consumer assumptions until Codex records them; traceLinkOrigins links are not rendered.
- No evidence manifest, `LocalServiceManifest` fragment or CI patch yet.
- The 410/reconnect/axe/leak checks were authored by the implementer; the plan requires an independent reviewer for a11y/security.
