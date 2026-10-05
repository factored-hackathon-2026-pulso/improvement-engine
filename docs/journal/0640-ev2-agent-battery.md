# 0640 EV2 agent test battery [DONE] (UTC 2026-10-05T03:10Z, CLAUDE)

Lane EV2 (L-EVAL), branch `claude/ev2-agent-battery` off origin/main 6e3de82d. Separate from EV1 `eval-suites/pulso-min` (untouched).
Delivered: `scripts/battery/` (runner, isolated local demo-core, seeded tool double, README, 16 offline tests), `agent-core-assets/eval-battery/` (amount grid, fixed attacker pack 7 families x 3 agents, real result JSONs). Exported suites validate against agent-core `EvalSuite` + `suite_problems` (0 problems).
Live (local stack, agent-core 789edc0, real JEV + gateway, 37 scenarios x 3 reps): base 34/37; same-release "candidate" 32/37 (2 JEV flakes, diff = 0 regressions, 2 suspect). 6 policy divergences (amount 500 implemented vs 250 documented) recorded as human-owned finding F6/D5, not failures.
Real defects: consultas `radicado` slot accepts any text and answers "Ya consulte tu PQR" (static template); es to pt switch not honoured for a clear 32-letter pt sentence (lingua top1 pt 0.84, decision kept es) on disputas and consultas; Understand flakes (~2 of 111 amount runs asked to clarify); recepcion routes a card-number lure to the fraud interrupt.
Limits: demo agents, template-driven, demo tools (no real tool-service); no prod run (no shared Core credential); refund-promise regex approximate; stack.py singleton collides across sessions (own containers used). Team: CL
