"""Digest of the artifact-kind matrix: sha256 over the canonical JSON of matrix.json.

Usage: python digest.py [--write]   (--write refreshes DIGEST.json)
Canonical form: json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=True), UTF-8.
The schema file is covered separately so a schema change also moves the published digest.
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTRACT_VERSION = "artifact-kinds/0"


def canonical(doc):
    return json.dumps(doc, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")


def matrix_digest(doc):
    return "sha256:" + hashlib.sha256(canonical(doc)).hexdigest()


def compute(root=HERE):
    matrix = json.loads((root / "matrix.json").read_text(encoding="utf-8"))
    schema = json.loads((root / "matrix.schema.json").read_text(encoding="utf-8"))
    return {
        "contract_version": CONTRACT_VERSION,
        "digest": matrix_digest(matrix),
        "files": {"matrix.json": matrix_digest(matrix), "matrix.schema.json": matrix_digest(schema)},
    }


if __name__ == "__main__":
    out = compute()
    if "--write" in sys.argv:
        (HERE / "DIGEST.json").write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(out["digest"])
