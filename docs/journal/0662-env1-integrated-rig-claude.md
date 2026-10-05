# 0662 ENV1 integrated rig stage 1 (UTC 2026-10-05T16:45Z, CLAUDE) [DONE, approve/publish/outcome not run]

Lane ENV1, branch `claude/env1-integrated-rig` from origin/main 6c379a6e; platform main 5261ecf and agent-core main 2ad5d08 in detached worktrees under `tmp/env1`.
- New `scripts/integrated-rig/` (up, down, health, run_story, merge_keys.py, story_verify.py, rig.lib.ps1, Pester + unittest), `docs/dev/INTEGRATED_RIG.md`; `identity.py mint` now also mints a read-only `exporter` token (G9); `New-AnnouncePayload -EvidenceLinks` (G1) and a single-link array bug fixed.
- LIVE: cells -> loop (262 s, USD 0.0019, regression_suite_proven) -> announce 200/200 -> 22/22 checks (4 supervisors one notification each, list source engine, detail docs) -> eval suite attached and passed. Rig stopped afterwards.
- NOT run: approve, publish, promote, release event, outcome step (permission layer denied the automation; needs a user decision).
- Findings: platform needs `tzdata` on Windows; engine announce should use the platform evidence route; LFC lane shares ports 55510/8210. Validation: Pester 59, unittest 15, owners 5.
