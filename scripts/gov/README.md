# scripts/gov

`pre-pr-gate.ps1` (G0p): local gate before a PR is opened (owner-map check, touched-package tests; the
`-TouchedPackages` argument is validated against command injection). Tests:
`uv run --python 3.12 --with pytest python -m pytest scripts/gov/tests -q -p no:cacheprovider`.
Owner lane: L-GOV.

GT0 package (all Python, stdlib only; tests in `scripts/gov/tests`):

- `crv0_closure.py`: validates `docs/reviews/claude/*.review.json` (format in that folder's README), exits 0 only when every log is closed, reviewer differs from author, and every required WP is covered.
- `trn0_train.py check|merge|bundle|receipt`: lane merge in dependency order, restack check, PR size cap (lane-hours), W0 receipt per train PR, `exchange/` bundle at the cap, `train-receipt/v1`. Never pushes. `"mode": "consolidated"` manifests record an already-merged train as one PR: merge order is observed from git, unknown lane-hours stay null, and going over the cap needs a recorded `cap_deviation.authority` that is printed as a deviation.
- `gt0_collect.py`: runs the replay thread and writes `report.json` and `replay.json` (needs the e2e-core dependencies and the sensor exe).
- G0p cargo coverage: when the receipt's ci leg names `cargo ... --manifest-path X/Cargo.toml`, every Rust file changed vs the base must live under `X/`, otherwise g0p fails. Claude's receipt runs `cargo test --manifest-path seams/Cargo.toml --offline -j 2`; Codex's root-workspace gates (fmt, clippy, cargo test of `crates/`) are Codex-owned and are NOT run by Claude's receipt.
- `gt05_collect.py`: re-executes itself under `uv run --offline --python 3.12` with the full e2e-core dependency set, so any Python can start it: `python scripts/gov/gt05_collect.py --out-dir docs/reports/gates/gt05 --steps-exe <steps_cli.exe> --ratchet` (STEPS_RUNNER_EXE = ED0 runner).
- `gt0_gate.py`: manifest of receipts (`docs/reports/gates/gt0/manifest.json`); recomputes G0p, TRN0, CRV0, the 8 honesty tests on the real report, scanner ids, doubles, C-2/C-12 digests, replay, live-window log and capacity; exit 1 on any missing item.
- `gt0_report.py`: writes `docs/reports/gates/gt0-report.md` from the artifacts only (missing artifact = MISSING).

GT05 package (DEMO-1a, Rust steps called by the Python host; tests in `scripts/gov/tests`):

- `gt05_collect.py`: runs the thread in replay twice (`--steps rust` and default) and writes `rust-report.json`, `default-report.json`, `rust-run.json` (bound to the report bytes and the steps_cli binary), optionally `ratchet.json` (`--ratchet`, pytest through uv offline). Needs `STEPS_CLI_EXE` and the sensor exe.
- `gt05_gate.py`: manifest `docs/reports/gates/gt05/manifest.json`; checks the five flips (host python, semantics claude-standin, steps_cli sha256 equals the binary on disk), no silent Python fallback, G1 `check()` clean, GT0 gate passing on every item but capacity, the default run unchanged, CRV1 closure and RG-1 (every real-narrow step served by a Rust step; ratchet-test receipt and the ED0b original run print MISSING until recorded). Exit 1 on any failing or MISSING item.
- `gt05_report.py`: writes `gt05-report.md` and `gate-result.json` next to the manifest from the artifacts only.
