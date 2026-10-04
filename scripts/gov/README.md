# scripts/gov

`pre-pr-gate.ps1` (G0p): local gate before a PR is opened (owner-map check, touched-package tests; the
`-TouchedPackages` argument is validated against command injection). Tests:
`uv run --python 3.12 --with pytest python -m pytest scripts/gov/tests -q -p no:cacheprovider`.
Owner lane: L-GOV.
