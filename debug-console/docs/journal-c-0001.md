# Journal C-0001: DebugApi data-access seam

Scope: `debug-console/` is the internal backoffice of the engine (spec 25). Only `/internal/v1/debug` routes of spec 25 are typed; no endpoint was added.

## What was built (`src/api/debug/`)
- `port.ts`: `DebugApi` port (mode, session, runs, graph, events, model-calls, queries, evals, memory-diff, external-commands, dependency health, SSE `openStream`, `command`).
- Providers (`index.ts`, `createDebugApi` / lazy `bootDebugApi`): `http` (real control-api over fetch), `fixture`, `stand-in`. All three are the SAME provider code over `fetch`; fixture and stand-in point it at an in-process backend (`backend.ts` + `worlds.ts`), http at a base URL. Switching demo -> real is `public/config.json` (`dataProvider: http|fixture|stand-in`, `apiBase`), not a rewrite. Unknown value falls back to `http`. Worlds are code-split (dynamic import), so an http deployment never loads demo data.
- `dto.ts`: zod DTOs hand-written from spec 25 (RunEvent per the instrumentation contract; envelope with `projection_revision`, `as_of`, `available_commands`), uniform `DebugApiError` (status, code, correlationId, retryable, conflict, recovery) built by `parseProblem`; unrecognised bodies never echoed. Invalid success bodies -> `invalid_response`.
- `stream.ts`: SSE reader: `after_sequence` + `Last-Event-ID` resume, dedup by (run,sequence), foreign-run events dropped, gap -> backfill from `/events` (never guessed), 410 `cursor_expired` -> read snapshot, `onReset` (replace projection), resume after recovery cursor, second consecutive 410 backs off, snapshot ref of another run -> terminal, 401/403/404 terminal.
- `commands.ts`: pause/resume/cancel/retry/fork_replay typed; `commandState(cmd, available_commands, scopes)` enables only when the server offers the command AND the session scope allows it; role labels (`debug_operator`, `approver`) never enable anything. Acks are `requested`, never `confirmed`.
- UI: `ProviderDeclaration` (provider/target/runtime_profile/doubles[]) next to the existing stand-in `ModeBanner`; a failed declaration reads "unverified".

## Migrated screen
Run list (`features/RunList.tsx`, formerly inline in `App.tsx`) now depends only on the port via `DebugApiProvider`. All pre-existing tests stayed green (121 unit + 29 contract before; unchanged assertions).

## Tests (strict TDD; RED commit ed112a4: all three new suites failed to load)
- `tests/unit/debug/contract.test.ts`: one suite run against fixture, stand-in and http (http through a real local node http server wrapping the same backend). `dto.test.ts`, `tests/component/RunListPort.test.tsx` (port stub never touches fetch; config-driven selection).

## Gaps for the control-api owner (Codex)
1. Per-panel field names (model-calls, queries, evals, memory-diff, external-commands, health) are PROVISIONAL: the spec lists columns, not names. Please record the shapes; `dto.ts` is the single place to align.
2. 410 body: spec says "ref de snapshot + cursor de recuperacion"; we expect `recovery: {snapshot_ref, after_sequence}` (legacy fixture `snapshot_url`/`recovery_after_sequence` also accepted). The snapshot is read through `GET /runs/{id}/graph`; no snapshot route is defined by spec 25. Confirm.
3. Command scopes (`debug:command:<name>`) are provisional: the spec names roles only; the session must expose scopes and `available_commands[]` per run (graph envelope) and per job. Command bodies: `{expected_revision, reason}` + `Idempotency-Key` + CSRF; ack `{command_ref, status_url, state: requested}`.
4. `as_of` is optional in the client until the real API returns it (spec requires it). The http provider still reads `GET /debug/profile` and `/api/v1/auth/session` (existing consumer proposals, not spec 25) for the mode declaration and CSRF.
5. Only the run list is migrated; other screens (graph, investigation, gates, memory, decision) still use `api/client.ts`, so fixture/stand-in providers are partial until they move.
6. Dockerfile now copies `fixtures/demo-world.json` (stand-in world is bundled lazily).
