# 0643 W11 evaluate before announce (UTC 2026-10-05T07:00Z, CLAUDE) [DONE]

Lane W11, branch `claude/w11-evaluate-before-announce` = origin/claude/b3-wire + w12-dossier (carries reg1, w14) merged; OWNERS union, stack.py serve flags unioned.
- registry-writer: guard allows `freeze` and `evaluate` only (RED tests for approve/publish/promote/reject/reopen/revoke/alias, method/suffix/case, query/fragment/control-byte smuggling); `eval` (manual-origin evaluation drafts, bounded infra retries, closed problems), `proof` (build_suite + judge_story subprocess contracts, verdict story into the dossier, announce only when proven, idempotent ProofStore, `announce_submission` with dossier ES docs + eval_suite).
- scripts/regression/judge_story.py: raw runs -> `reg1.verdict_story/1` with the SAME rules as prove_fails_on_base.py.
- pulso value loop: `ProofConfig`, default ON for the live api path (`PULSO_EVAL_BEFORE_ANNOUNCE`), OFF in unit tests; record `evaluation` + `outcome` (`announced` | `not_announced:<verdict>`).
- Bug found live and fixed: the evaluation Idempotency-Key must include the candidate digest (a second candidate of the same finding replayed the first one's frozen draft).
Tests: registry-writer 32 offline + 14 proof, reasoning 15+11, pulso value_loop 12 (5 new), python judge 5 + regression 29, live_proof 2/2 on own stack (see docs/dev/W11_EVALUATE_BEFORE_ANNOUNCE.md). Limits: prompt probes need a generator; no PR.
