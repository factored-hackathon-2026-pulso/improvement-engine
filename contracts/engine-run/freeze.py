"""Digest freeze of C-2 (engine-run report contract) and C-12 (roleplay queue protocol, scanner allow-list, ledger).

FREEZE.json pins sha256 of every covered file (LF-normalised) and a contract digest over the sorted file map.
Usage: python freeze.py            verify, exit 1 on drift
       python freeze.py --write    re-pin (a deliberate contract revision; needs a journal entry)
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PIN = "contracts/engine-run/FREEZE.json"

CONTRACTS = {
    "C-2": ["contracts/engine-run/engine_run.py", "contracts/engine-run/report.schema.json"],
    "C-12": ["roleplay-llm/roleplay_llm/protocol.py", "roleplay-llm/roleplay_llm/scanner.py",
             "roleplay-llm/roleplay_llm/shim.py"],
}


def file_digest(p: Path) -> str:
    return "sha256:" + hashlib.sha256(p.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def contract_digest(files: dict) -> str:
    return "sha256:" + hashlib.sha256(json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def compute(root: Path = ROOT) -> dict:
    out = {}
    for cid, rels in CONTRACTS.items():
        files = {rel: file_digest(root / rel) for rel in rels}
        out[cid] = {"status": "frozen", "digest": contract_digest(files), "files": files}
    return {"schema": "contract-freeze/v1", "contracts": out}


def verify(root: Path = ROOT) -> list:
    """Return human-readable drift problems; empty means every covered file matches the pin."""
    pin = json.loads((root / PIN).read_text(encoding="utf-8"))["contracts"]
    problems = []
    for cid, c in pin.items():
        for rel, want in c["files"].items():
            p = root / rel
            if not p.is_file():
                problems.append(f"{cid}: {rel} missing")
            elif file_digest(p) != want:
                problems.append(f"{cid}: {rel} drifted from the pinned digest")
        if contract_digest(c["files"]) != c["digest"]:
            problems.append(f"{cid}: pin file is internally inconsistent")
    return problems


if __name__ == "__main__":
    if "--write" in sys.argv:
        (ROOT / PIN).write_text(json.dumps(compute(), indent=2) + "\n", encoding="utf-8", newline="\n")
    probs = verify()
    print("\n".join(probs) or "C-2 and C-12 match FREEZE.json")
    sys.exit(1 if probs else 0)
