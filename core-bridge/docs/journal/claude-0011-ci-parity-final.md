# claude-0011: final CI parity run

- Rebased claude/r-runtime-wire onto origin/main 589e8dd (clean).
- Hosted ci.yml is Rust-only; this branch touches no Rust/contracts/scripts, so those jobs are unaffected and were not rerun. Python suites run via core-bridge/scripts/ci.ps1 on PG16 (podman pulso-dev).
- lint, contract-drift, modules-scan, agent-core-assets, platform-sim (206) green; core-bridge 529 passed / 13 skipped / 1 failed.
- The one failure (tests/l5/test_routes.py mutate7) was Windows ephemeral-port exhaustion (WSAEADDRINUSE to the PG port) under load; the file re-ran 12/12 green alone.
- The E402 lint fix (test_image.py import hoist) is in the branch.
