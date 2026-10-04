# scripts/env

PowerShell helpers for the machine and worktree envelope (lane L-ENV):

- `lane-target-dir.ps1 -Lane <lane>`: per-lane `CARGO_TARGET_DIR` on D: (G0e).
- `check-target-dirs.ps1 -Assignment lane=dir ...`: exit 1 on shared or non-D: dirs.
- `worktree-inventory.ps1`: WTR1, read-only inventory (dry run only, never removes).
- `SLOT_PROTOCOL.md`: one cargo slot at a time, per-lane target dirs.

Tests: `scripts/env/tests/test_env_scripts.py` (needs `pwsh`). Run:
`uv run --python 3.12 --with pytest python -m pytest scripts/env/tests -q -p no:cacheprovider`.
No secrets; paths only. Owner lane: L-ENV.
