# 0593 B3 wire: value loop, trigger runner, model change (UTC 2026-10-05T03:30Z, CLAUDE)

Lane B3, branch `claude/b3-wire` (main + B2 + TR2 + b2tr2-review fixes).

Delivered
- `pulso run` value loop (`seams/crates/pulso/src/run/value_loop.rs`): cells sensor -> reasoning -> registry writer as `builder`; one record per finding in the engine job store (steps 1..), summary at step 0, console run, `<work>/value-loop/<job>.json`; per-finding idempotent (job-store records, writer receipts, Idempotency-Key on create and put_draft, `GET /proposals` lookup); cost cap `PULSO_LOOP_MAX_FINDINGS`.
- TR2 gaps: `pulso run` serves the trigger endpoint over its real JobRepository (`RepoAdmitter`), the worker runs `trigger:*` jobs, the poller sends `X-CSRF-Token` from `/api/v1/auth/session`. Replay table not persisted (the job store has no response column).
- Models: generation `xiaomi/mimo-v2.6-flash`, Verifier `xiaomi/mimo-v2.6-pro`, judge `z-ai/glm-5.3-flash`; independence level `same_family_other_tier`/`other_family`; per-model default prices; the model is per gateway port (one instance per role), so the role split is per request.
- Cells below reference stop before any model call (`better_than_reference`).
- Local stack on agent-core main `daf4604` (field classifier module moved to `agent_core.composition.classification`).

Commands run: `cargo test -j 1` on pulso, reasoning, registry-writer, debug-api, engine; python unittests scoring/aggregate/triggers; live: registry-writer 2/2, reasoning 0/3, bank pass (see `docs/reports/b3-live-pass.md`).
Limits: 0 proposals from the live loop (Builder flash output); outcome triggers also run the loop; the staff-keys trust of the engine kid is local-stack only.
