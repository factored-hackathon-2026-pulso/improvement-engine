# scripts/gov

`pre-pr-gate.ps1` (G0p): local gate before a PR is opened (owner-map check, touched-package tests; the
`-TouchedPackages` argument is validated against command injection). Tests:
`uv run --python 3.12 --with pytest python -m pytest scripts/gov/tests -q -p no:cacheprovider`.
Owner lane: L-GOV.

GT0 package (all Python, stdlib only; tests in `scripts/gov/tests`):

- `crv0_closure.py`: validates `docs/reviews/claude/*.review.json` (format in that folder's README), exits 0 only when every log is closed, reviewer differs from author, and every required WP is covered.
- `trn0_train.py check|merge|bundle|receipt`: lane merge in dependency order, restack check, PR size cap (lane-hours), W0 receipt per train PR, `exchange/` bundle at the cap, `train-receipt/v1`. Never pushes. `"mode": "consolidated"` manifests record an already-merged train as one PR: merge order is observed from git, unknown lane-hours stay null, and going over the cap needs a recorded `cap_deviation.authority` that is printed as a deviation.
- `gt0_collect.py`: runs the replay thread and writes `report.json` and `replay.json` (needs the e2e-core dependencies and the sensor exe).
- `gt0_gate.py`: manifest of receipts (`docs/reports/gates/gt0/manifest.json`); recomputes G0p, TRN0, CRV0, the 8 honesty tests on the real report, scanner ids, doubles, C-2/C-12 digests, replay, live-window log and capacity; exit 1 on any missing item.
- `gt0_report.py`: writes `docs/reports/gates/gt0-report.md` from the artifacts only (missing artifact = MISSING).
