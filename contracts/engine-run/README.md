# engine-run (G1)

Purpose: `engine_run.py`, which checks an
engine-run report against honesty rules H1-H8 (`check()`) and generates the `doubles[]` list from the steps that
are stand-ins (`generate_doubles`). A report that claims a real step while using a double, or hides a double, fails.

Tests (verified):

    uv run --python 3.12 --with pytest --with pyyaml python -m pytest contracts/engine-run/tests -q -p no:cacheprovider

Rule G1 (gate honesty): `report.gate.verdict` must be stated when `approval`/`publish` are exercised; after a gate that is not `pass`
they need `overrides[]` with step=approval, of=gate, verdict=<gate verdict>, by=human, label=human_override, a reason, and
`quality_claims: forbidden`; an override on a passing gate is rejected; `doubles[]` lists `gate.override`.

Data-class rules: reports carry ids and labels only, never rows. Honesty labels: `real`, `real-narrow`, `local-model`,
`agent_roleplay`, `recorded`, `stand-in`, `simulated`, `not_exercised`, `blocked(<reason>)`. Consumed by `e2e-core` (`build_report`).

Owner lane: L-GOV.
