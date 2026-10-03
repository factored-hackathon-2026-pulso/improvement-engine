"""CAP-63 scan entry point: prints a JSON list of findings of local_identity.contract.scan_remote_config(root).

Pure stdlib (the contract module imports nothing else). Usage: PYTHONPATH=local-identity/src python scan_remote.py <root>."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from local_identity.contract import scan_remote_config

root = Path(sys.argv[1]).resolve()
print(json.dumps([{"path": str(f.path.relative_to(root)).replace("\\", "/"), "rule": f.rule} for f in scan_remote_config(root)]))
