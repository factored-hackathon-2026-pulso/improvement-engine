# 0245 REG1 regression suites that fail on the base (UTC 2026-10-05T03:41Z, CLAUDE)
[DONE] REG1 (plan W1-1), branch claude/reg1-regression-suites (EV1 merged), no PR. build_suite.py + prove_fails_on_base.py + 29 offline tests, docs/dev/REGRESSION_SUITES.md; stack.py opt-in PULSO_STACK_PREFIX/ports.
Live (agent-core ae437bb, real JEV, mimo-flash): t/estado_pqr suite 8 finding+3 guard: base 8/8 fail, bad-placeholder attempt fails natively, attempt 2 passes -> proven; p/resumen_radicado 6+3: base 6/6 fail (probe), attempt 2 passes -> proven; noop controls not_fixed; native-only prompt suite non_discriminating.
[FINDING] agent-core evaluate never exercises a candidate prompt (gateway bound to the live registry; identical-text bump falls back); template candidates are exercised. JEV drops intermittently (failed_infra). Asks via the user: candidate-bound gateway.
doubles[]: wording probe is harness-side; finding and mechanism mapping synthetic; uncovered_topic generator not exercised. Tests: 29 offline; gate: unittest + owners + dataclass push-scan.
Team: CL
