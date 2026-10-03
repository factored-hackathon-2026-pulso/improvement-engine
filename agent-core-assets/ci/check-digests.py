"""Fail when manifest.yaml / expected-state.json drift from the worlds (offline; PyYAML only)."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import assetcheck  # noqa: E402

root = Path(__file__).resolve().parents[1]
violations = assetcheck.check_manifest(root)
for v in violations:
    print(v)
sys.exit(1 if violations else 0)
