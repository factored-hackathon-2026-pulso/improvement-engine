# e2e-core

Claude-side E2E precursor. `src/claude_standin/` holds the stand-ins and steps (`thread01.py` ten-step runner,
`ed0_detect`/`ed0_lab` sensor and k-anonymous lab, `smap.py` catalogue-driven builder target, `compile_step`
CMPpy, `gate_step` GSIpy, `core_hooks.py` real-Core hooks). `src/codex_standin/` drives the real_local stack.

- Ten-step thread in replay: [`THREAD01.md`](THREAD01.md); sensor spike [`ED0.md`](ED0.md).
- Light tests (no containers, no cargo; verified), from `e2e-core`:

      PYTHONPATH='src;tests;../core-bridge/src;../local-identity/src;../platform-sim;../platform-contract;..' \
      uv run --python 3.12 --with pytest --with pyyaml --with fastapi --with httpx --with cryptography \
      python -m pytest tests/unit/test_e2e_thread_01.py -q -p no:cacheprovider

  (Use `;` as PYTHONPATH separator on Windows.) `tests/live` and `run.ps1` need the Podman stack and are out of scope here.

Data-class rules: packages are synthetic; E0 is read only at runtime via `ED0_RUNNER_EXE`, never committed.
Honesty labels per step are in THREAD01.md. Owner lane: L-E2E.
