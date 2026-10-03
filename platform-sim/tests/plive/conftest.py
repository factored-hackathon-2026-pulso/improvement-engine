"""Make the platform-contract package importable for the platform_live simulator tests."""

import sys
from pathlib import Path

_CONTRACT = Path(__file__).resolve().parents[3] / "platform-contract"
if str(_CONTRACT) not in sys.path:
    sys.path.insert(0, str(_CONTRACT))
