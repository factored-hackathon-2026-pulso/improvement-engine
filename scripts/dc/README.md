# scripts/dc (DC0 data-class gate)

Purpose: `dataclass_gate.py`, a stdlib-only scanner that blocks raw-E0 markers (including encoded, binary,
UTF-16, base64 and case variants), gateway egress in URL-like fields, and restricted data classes on non-local
upstreams. `pre-push` is the hook wrapper. Scanner id: `dc0-content-scan/1`.

    python scripts/dc/dataclass_gate.py push-scan             # git-tracked files, exit 1 on findings
    python scripts/dc/dataclass_gate.py scan FILE [FILE...]   # explicit files

Tests (verified):

    uv run --python 3.12 --with pytest python -m pytest scripts/dc/tests -q -p no:cacheprovider

Data-class rules: E0 and original rows never enter the repo; findings print the location, not the content.
Owner lane: L-MODEL.
