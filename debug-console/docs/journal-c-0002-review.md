# Journal C-0002: adversarial review of the DebugApi seam (ed112a4 / 27ee974)

Method: read all of `src/api/debug/`, mutated 24 behaviours (dedup, foreign run, gap backfill, 410 reset/cursor, cross-run snapshot, Last-Event-ID, after_sequence, 401, command gating x2, scope map, schema skip, ack state, CSRF, idempotency key, error echo, retryable set, provider fallback, trace id, limit clamp, CSRF reset on 401, unverified banner). 8 survived the original suite: foreign-run, cross-run snapshot ref, after_sequence query, 401 on stream, scope map, ack default, retryable set, CSRF reset on 401.

## Findings
- MEDIUM fixed: SSE buffer unbounded (an unterminated frame grew forever). Now capped at 1 Mi chars: drop and resume from the last applied sequence.
- MEDIUM fixed: ack `state` was trusted from the server; a `confirmed` ack was displayable. It is now always `requested` (confirmation is `external-commands.confirmed_at`).
- MEDIUM fixed: with fixture/stand-in, run/sources/memory screens still use the legacy client (real same-origin API) under a fixture/stand-in declaration. They now carry an explicit `provider-partial` notice.
- LOW fixed: handlers could fire after stop() from an in-flight backfill.
- LOW fixed: weak tests (see mutants) now killed.
- LOW open: `apiBase` accepts any http(s) origin. Bounded in production by nginx CSP `connect-src 'self'` (cross-origin API calls are blocked, cookies are `same-origin`); not bounded under `vite dev`. A hostile config.json already implies control of the static host.
- LOW open: 410 without a recovery body retries forever (capped backoff) instead of terminal.
- LOW open: attempt counter resets on every successful open, so a server that accepts then closes instantly is retried at the minimum delay (1 s).
- LOW open: `scrub()` assigns `__proto__` keys from parsed JSON onto the output object (prototype of that object only; zod strips unknown keys, no global pollution). Pre-existing code.
- INFO: demo world chunks (`worlds-*.js`, `backend-*.js`) ship in dist but are lazy-loaded only when config selects fixture/stand-in; the index chunk contains no demo data. No localStorage/sessionStorage/cookie use; no innerHTML; run ids only reach `href="#/run/..."`.
- INFO: authority paths: only `commandState` (server `available_commands[]` AND scope); no UI consumes it yet.
- Dockerfile/tsconfig edits are correct (`resolveJsonModule`, demo-world.json copied for the build).
