"""Verify (or build) the FRZ0 pack: one digest over a manifest of 7 parts.

Codex lanes verify only against this pack: python verify_pack.py [--write]
Stdlib only; no Core, Postgres or Podman. Exit code 1 when verification fails.
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PARTS = [
    ("c7_claim_next", "C-7 v1.1 claim-next signature and claim traces"),
    ("c8_schemas", "C-8 ChangeSpec and ScenarioCase schemas"),
    ("c9_transcript", "C-9 pipeline transcript seed"),
    ("c10_gate_result", "C-10 gate result shape"),
    ("builder_corpus", "builder-output corpus v0 (10 synthetic outputs) and verdicts"),
    ("arm_report_goldens", "arm-report goldens"),
    ("bridge_goldens", "bridge dry-run and writer goldens"),
]


def file_digest(p):
    return "sha256:" + hashlib.sha256(Path(p).read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def pack_digest(parts):
    body = [{"id": p["id"], "files": p["files"]} for p in parts]
    raw = json.dumps(body, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def build(root=HERE):
    parts = []
    for pid, name in PARTS:
        d = root / "parts" / pid
        files = {}
        for p in sorted(d.rglob("*")):
            if p.is_file():
                files[p.relative_to(root).as_posix()] = file_digest(p)
        parts.append({"id": pid, "name": name, "files": files})
    return {"contract_version": "engine-steps-pack/0", "parts": parts,
            "pack_digest": pack_digest(parts)}


def verify(root=HERE):
    root = Path(root)
    errs = []
    mp = root / "manifest.json"
    if not mp.exists():
        return ["missing manifest.json"]
    m = json.loads(mp.read_text(encoding="utf-8"))
    ids = [p["id"] for p in m.get("parts", [])]
    if ids != [p for p, _ in PARTS]:
        errs.append(f"manifest parts {ids} != expected {[p for p, _ in PARTS]}")
    listed = set()
    for part in m.get("parts", []):
        if not part["files"]:
            errs.append(f"part {part['id']} lists no files")
        for rel, dg in part["files"].items():
            listed.add(rel)
            f = root / rel
            if not f.exists():
                errs.append(f"missing {rel}")
            elif file_digest(f) != dg:
                errs.append(f"digest mismatch {rel}")
    for pid, _ in PARTS:
        for f in (root / "parts" / pid).rglob("*") if (root / "parts" / pid).exists() else []:
            rel = f.relative_to(root).as_posix()
            if f.is_file() and rel not in listed:
                errs.append(f"unlisted {rel}")
    if m.get("pack_digest") != pack_digest(m.get("parts", [])):
        errs.append("pack_digest does not match parts")
    return errs


if __name__ == "__main__":
    if "--write" in sys.argv:
        out = build()
        (HERE / "manifest.json").write_text(
            json.dumps(out, indent=2) + "\n", encoding="utf-8", newline="\n")
    errors = verify()
    for e in errors:
        print("FAIL:", e)
    print(json.loads((HERE / "manifest.json").read_text("utf-8"))["pack_digest"])
    sys.exit(1 if errors else 0)
