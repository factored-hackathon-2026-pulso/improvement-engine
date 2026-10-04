# engine-steps (draft v0, C-3)

JSON in/out schemas for the engine steps `sensors`, `recompute`, `validation`,
`compile` and `gate`, plus the port list (`ports.json`).

- `schemas/<step>.{in,out}.schema.json`: JSON Schema subset (see `minischema.py`).
- `samples/`: valid samples and `invalid-*` samples that must be rejected.
- `DIGEST.json`: published digest over ports, schemas and samples
  (`python digest.py --write` refreshes it; the test fails when it is stale).
- `gen_draft_v0.py`: generator of the JSON files; the JSON files are the contract.
- Tests: `python contracts/engine-steps/tests/test_engine_steps.py`.

Every document carries `contract_version`, `step`, `run_id` and `data_class`
(`synthetic`, `treated`, `e0`, `original`). Draft status: changes before the freeze
need no stanza; afterwards `[CONTRACT-CHANGE]`.
