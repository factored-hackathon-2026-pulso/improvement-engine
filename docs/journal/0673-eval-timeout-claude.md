# 0673 Evaluate timeout for the long suites and the minted credential on a long call (UTC 2026-10-05, CLAUDE)

Lane QUOTA-WIRE (engine half), branch `claude/engine-eval-timeout`, from `origin/main`.

- Finding: `PULSO_EVAL_TIMEOUT_SECS` already defaulted to 900 s (`seams/crates/pulso/src/run/value_loop.rs`) and feeds `HttpTransport::new` for the proof's evaluate transport; `core_client::http` applies it to connect, read and write. copiloto-asesor-suite (22 scenarios x up to 3 runs, `max_workers=1`) takes over 180 s, so 900 s is the right default; no per-suite split needed.
- Change: the parse is now `eval_timeout_secs(get)`: default 900, override by the variable, a non-positive or unparsable value falls back to 900 (before, `0` reached the socket as a zero timeout) and under 60 s is raised to 60. Documented in `docs/dev/ENGINE_PROD.md`.
- MintingTransport: checked, it does not break. A credential (300 s TTL) is minted just before each request and the Core authenticates on arrival, so a 600 s evaluate is neither resent nor re-signed; the next request mints a new one. A test pins this (`an_evaluate_that_outlives_the_credential_ttl...`); it was green on first run, so it is a regression guard, not a fix.
- Tests: RED were the two `the_evaluate_timeout_*` tests (function missing). Pre-existing failure on origin/main, not touched: `w11::the_second_candidate_is_tried_only_when_the_first_is_not_proven...` (left 7, right 5).
- Infra: the loop job leaves the variable unset, so it gets 900 s; `docs/engine-loop.md` in the infra repo records it.
