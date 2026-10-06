# 0672 Bank cells automatic in AWS: the aggregator ships in the engine image (UTC 2026-10-05, CLAUDE)

Lane CELLS-AUTO, branch `claude/engine-cells-auto`, from `origin/main` 735e007. Companion: infra PR `claude/infra-cells-auto` (loader runs the aggregator).

- Decision (user): the AWS deployment is automatic from uploaded data to the engine's input cells. The gap was `loader_cells_cmd` empty and `bank_cells.py` in no image.
- Change here: `Dockerfile` copies `scripts/aggregate/bank_cells.py` to `/opt/pulso/aggregate/bank_cells.py` (after the console copy, away from the PR 130 hunks; needs PR 130 for python3 in the image). Nothing else in the aggregator changed.
- Why the engine image and the landing CSV (not the pipeline image, not silver/gold): `docs/data/bank-cells-metrics.md`, section "Automatic export in AWS". Checked against the data-pipeline repo: bronze is all-varchar parquet, silver `interactions` (not `call_center_interactions`) drops quarantined rows, gold pseudonymises ids.
- E0: `docs/data/opbench/e0-export` is the R3-4 benchmark exporter (Rust, local E0 parquet), not an input of `pulso loop` (`PULSO_CELLS_SOURCE=bank`). Not automated; a frozen E0 package stays an operator upload under `engine/inputs/e0/`.
- Tests (Python, `scripts/aggregate/tests/test_bank_cells_loader_contract.py`, synthetic fixtures): Dockerfile path, stdlib-only imports, the eight inputs, the loader's CLI invocation writes cells that pass the k>=10 gate with no identifiers.
- Not verified: no image build here (podman/Docker not run), `bank_cells.py` never run on real bank data in this lane; duration and memory on EC2 unmeasured (laptop: about 8 minutes, a few hundred MB).
