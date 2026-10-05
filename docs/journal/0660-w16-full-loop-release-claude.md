# 0660 W16 full value loop release (UTC 2026-10-05, CLAUDE)

One branch (`claude/w16-full-loop-release`) merges map1 (which holds b3 .. demo1) with inh1, engo and main (PR 102 squash). Conflicts: OWNERS by union (and the two `0658` journal globs made disjoint,
the owners test failed otherwise); battery/eval files and `stack.py` kept from the later lanes (ev3, prb1, bld1); `value_loop.rs` re-woven by hand (engo story/trace and calls around the MAP1 per-candidate attempts).

Defect (found by the engo test after the merge): calls of the second candidate collided on `(evidence_ref, role, n)` and on the generation span id. `append_calls` renumbers per finding.

Live pass on an own stack with a local agent-core scratch branch (main + PR 50 + PR 51): 19 corroborated, 9 announced (4 new agents included), 1 `not_fixed`, 9 unlinked, USD 0.0135.
See `docs/reports/w16-full-loop.md`. Journals 0658 and 0659 each exist twice (two lanes each); kept. Not verified: `traceparent` at the gateway (no collector).
