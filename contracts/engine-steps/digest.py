"""Digest of the engine-steps contract: sha256 over a sorted manifest of file digests.

Usage: python digest.py [--write]   (--write refreshes DIGEST.json)
Covered files: ports.json, schemas/*.json, samples/*.json (LF-normalised).
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PATTERNS = ["ports.json", "schemas/*.json", "samples/*.json"]


def file_digest(p):
    data = p.read_bytes().replace(b"\r\n", b"\n")
    return "sha256:" + hashlib.sha256(data).hexdigest()


def digest_of(files):
    body = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(body).hexdigest()


def compute(root=HERE):
    files = {}
    for pat in PATTERNS:
        for p in sorted(root.glob(pat)):
            files[p.relative_to(root).as_posix()] = file_digest(p)
    return {"contract_version": "engine-steps/0", "digest": digest_of(files), "files": files}


if __name__ == "__main__":
    out = compute()
    if "--write" in sys.argv:
        (HERE / "DIGEST.json").write_text(
            json.dumps(out, indent=2) + "\n", encoding="utf-8", newline="\n"
        )
    print(out["digest"])
