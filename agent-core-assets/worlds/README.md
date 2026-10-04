# worlds, corpus, tools

- `seeded-base.world.yaml` (WRLD0): seeded base world, synthetic. Conformance:
  `cd agent-core-assets && uv run --python 3.12 --with pyyaml --with pytest python -m pytest -c pytest.ini tests/test_world_seeded_base.py -q -p no:cacheprovider` (verified, 12 passed).
- `attention-demo/`, `pulso-evolution/`: world declarations; `../corpus/smap-catalogue.json` (ReadBase catalogue) and
  `smap-recorded-v0.json` (10 recorded synthetic builder outputs: 4 valid, 2 unlinked, 2 not_evaluable, 2 invalid).
- `../tools/`: `assetcheck.py`, `worldcheck.py`, importers; see `../README.md`.

All content is synthetic (`data_class: synthetic`); the corpus is `recorded`, not model-quality evidence.
Owner lanes: worlds and corpus L-MODEL, the rest L-BRIDGE.
