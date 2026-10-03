# Journal claude-0017: CI parity for PR 4 (rebase onto main 4a1fa7b, bridge-contract at pin 894fa65)

Branch `claude/r2-bridge-gaps`. GitHub API answered 503 during the run; `git fetch` showed origin/main = 4a1fa7b (as expected).

## Rebase
- 8 commits rebased onto 4a1fa7b. Two trivial conflicts in the WP5 commit: `tests/runtime/test_image.py` (kept both the key-delivery
  tests from main and the CORE_SHA==PIN_SHA test) and `local/core/doctor.core.ps1` (kept main's `humanissuer` lib, pin 894fa65).
- Main's ADR 0009 is key delivery from env; our pin-bump ADR is renumbered to 0010 (file, heading, ADR 0004 update note, journal 0013).
- The rebase brought main's runtime/entrypoint changes, so the image was rebuilt by `e2e-core/run.ps1` from a clean HEAD worktree.

## bridge-contract (first RED -> GREEN)
- RED: `gen.py --check` drift, golden `auth_and_envelope`/`arms`, `/version` pin test (4 failures after the pin bump); new
  `test_no_route_or_schema_is_marked_pending_implementation` and `test_alias_and_dry_run_goldens_cover_...` failed.
- gen.py: no `x-status` on routes/DTOs, `pending` flags and the SCAN_SKIP_DIRS exemption removed; `pulso:compile_violation`
  (never emitted, a 200 cannot carry an error code) dropped from the closed ErrorEnvelope list.
- Conformance `test_pending.py` -> `test_authoring.py` (live, no skips) plus valid dry-run and unknown-base 404; the mock xfails the
  unknown-base case (MOCK-DRYRUN). Goldens re-recorded against the real runtime (arms commitment, /version pin; authoring gained
  `dry_run_valid`, `dry_run_base_release_unknown`). There is no MANIFEST digest in bridge-contract; the pin lives in contract.json.
- Real target: 297 passed, 0 skipped. Mock target: 157 passed, 25 skipped (capability), 115 xfailed (known-different).

## Parity run (PG16 `pulso-claude-pr4-pg`, pulso-dev, --cgroups=disabled, 127.0.0.1, PULSO_REQUIRE_POSTGRES=1; removed after)
lint ok; contract-drift ok; modules-scan 1 passed; agent-core-assets 22 passed; platform-sim 228 passed; core-bridge 563 passed,
13 skipped; wire mock 186 passed/1 skipped; wire a2 185 passed/2 skipped; local/core Pester 3.4: 100 passed, 1 skipped; local/core python 20 passed
(needs PYTHONPATH=core-bridge/src); e2e-core 44 passed, 1 skipped (bank lost-response). No stuck or flaky tests.
Not run: real-wire (needs a running `agent-core serve`), rust job (Codex-owned).
