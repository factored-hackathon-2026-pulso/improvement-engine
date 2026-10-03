"""Make `parity`, `registry_mock` and `bridge_mock` importable whatever the working directory is."""

import sys
from pathlib import Path

_PLATFORM_SIM = Path(__file__).resolve().parents[1]
for p in (_PLATFORM_SIM, _PLATFORM_SIM / "tests"):
    if str(p) not in sys.path:
        sys.path.insert(0, str(p))
