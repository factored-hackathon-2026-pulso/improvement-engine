# BKNL: live denied-kind negatives on the real Core dry-run

Date 2026-10-04. Image `localhost/pulso-core-runtime:c814c2b-920f5e3` (agent-core pin c814c2b), one fresh `real_local` stack
on machine `pulso-dev` (`e2e-core/run.ps1 -BaseImage ... -Namespace claude-e2e-bknl -Keep`), torn down afterwards
(`stop.ps1`, `reset.ps1 -Confirm`, stack cleanup, the stack's anonymous volume removed; the two `pulso-local-*` Created containers untouched).
Test: `e2e-core/tests/live/test_10_bknl_denied_kinds.py`; helpers `e2e-core/src/claude_standin/bknl.py`; offline:
`tests/unit/test_bknl_negatives.py`, `tests/unit/test_bknl_replay.py`; fixtures `e2e-core/tests/fixtures/bknl/*.json`
(provenance block per file: image, date, command, endpoint; no token or header is recorded).

Each negative is expressed twice: as the engine ChangeSpec operation (compile step labels it) and as the Core draft sent to
`POST /core-authoring/dry-run` through the Bridge (as `core_hooks.make_dry_run`). Before and after every case the registry is
compared: aliases prod and staging, `reg_releases` ids, `reg_proposals` and `reg_draft_writes` counts. All identical (nothing published).

| Negative | Engine label (compile) | What the real Core/bridge answered | Guard |
|---|---|---|---|
| add_prompt (add on a prompt) | `kind_not_supported` | 200 `valid:false`, `REG-UNREFERENCED` (new prompt referenced by nothing) | engine and Core |
| stale_precondition | `missing_precondition` | 200 `valid:true` (auto-bumped agent and flow, candidate hash returned) | engine ONLY |
| outside_bridge (prompt `otro_prompt`) | `outside_bridge` | 200 `valid:false`, `REG-UNREFERENCED` | engine and Core |
| overwrite_published (new_ref = published ref) | `mutable_reference` | 200 `valid:false`, `REG-VERSION-TAKEN` (already published with other content) | engine and Core |
| release_settings | not expressible (the compile schema only knows prompt and eval_suite) | 422 `pulso:release_settings_not_allowed` | bridge |

Honest findings:
- RED: with a permissive compile stub (`BKNL_COMPILE=permissive`, the denied kind slips through as `compiled`) 4 of 5 live cases failed
  ("a denied kind slipped through the compile step"); `release_settings` passed because only the bridge guards it. The real compile is GREEN.
- A second RED on the real stack: the first assertion "the real Core does not accept" failed for `stale_precondition`. The Core has no
  precondition notion, so a draft whose engine precondition is stale is a perfectly valid draft for it. The compile step is the only
  guard; the test pins this (`Negative.core_refuses=False`) so a Core change that starts refusing it flips the test and calls for a matrix update.
- The Core's refusals of the other three are incidental to our label (`REG-UNREFERENCED`, `REG-VERSION-TAKEN`), not a one-to-one
  mapping of the engine reasons; the engine label stays authoritative.
- Not touched: `contracts/artifact-kinds/matrix.json` and its DIGEST (no verdict changed; `release_settings` evidence level could now be
  raised to real-core-live by citing this test, left to the matrix owner).
- A whole-suite run (`run.ps1 ... -PytestArgs <file>` appends the file after `tests/live`, so it runs everything) showed
  `test_09_..._steps_5_6_8_9...[1]` failing in that run while the rest passed; not investigated here (out of scope), flagged for follow-up.
