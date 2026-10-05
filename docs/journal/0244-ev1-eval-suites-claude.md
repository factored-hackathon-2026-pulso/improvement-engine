# 0244 EV1 minimal eval suites and live attach (UTC 2026-10-05T02:50Z, CLAUDE)

Lane EV1, branch `claude/ev1-eval-suites` off origin/main 2a9682de.

Delivered
- `agent-core-assets/eval-suites/pulso-min/`: `disputas-min@1.0.0` (20 synthetic cases es/pt) and `consultas-min@1.0.0` (11),
  README case -> behaviour map, unittest `test_pulso_min.py` (5 tests, offline).
- `scripts/dev-stack/attach_eval_suite.py`: validate with agent-core's `EvalSuite`/`suite_problems`, attach to a draft, validate,
  freeze, evaluate, per-gate report; never approve/publish. `stack.py` got opt-in env switches (see docs/dev/EVAL_SUITES.md).
- `docs/dev/EVAL_SUITES.md`.

Commands run (live, local stack agent-core 789edc0, Podman Postgres + llm-gateway, real JEV): `stack.py up --reset-db` with
`PULSO_SERVE_E2E=1`; `attach_eval_suite.py --create` several times. Final: disputas-min verdict pass (20/20 scenarios, platform
guardrails 0), consultas-min verdict pass (11/11). Real defects found and fixed are listed in docs/dev/EVAL_SUITES.md (stock serve
calibration double stalls all conversations; digit-only amounts for the keyword classifier; numeric policy input; a
verification_failed scenario cannot pass the platform gate; run_closed on a surplus turn).

Limits: wording and tool contracts not assertable; no `recepcion`/`copiloto-asesor` suite; the larger bank (Codex T2) replaces
these; agent entity thresholds default to margin 0 without floor.
