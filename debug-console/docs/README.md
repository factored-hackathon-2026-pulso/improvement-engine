# debug-console (L7, fixture-backed slice)

Commands: `npm ci`, `npm run typecheck|test|build|test:e2e|fixture|dev`. The e2e run starts the fixture API/SSE (:4010) and Vite (:5173, same-origin proxy).
Gaps vs plan 17.3.7: no react-router/react-query/react-virtual (hash routing, plain fetch), no Dockerfile/axe/contract suite/evidence manifest, scenarios F01-F25 only partly covered (active, refuted, blocked/unknown, step-up, memory revoked), no 410/reconnect/backoff yet, no CSRF/cookie hardening beyond header, UI strings partly es-419.
