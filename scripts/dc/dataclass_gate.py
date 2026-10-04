"""DC0 data-class gate: content scanner and push scan (pure stdlib, independent of any other module).

Integration point: the roleplay scanner (roleplay-llm) and TPS can call `scan_text` / `scan_receipt_body`
(or `main(["scan", ...])`) and record the scanner id `SCANNER_ID` on receipts; nothing here imports them.

Usage:
  python scripts/dc/dataclass_gate.py push-scan [--root DIR]    # git-tracked files; exit 1 on findings
  python scripts/dc/dataclass_gate.py scan FILE [FILE...]        # explicit files; exit 1 on findings
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

SCANNER_ID = "dc0-content-scan/1"

# Sentinels carried by raw E0 rows or payloads. Written as regexes so this source never matches itself.
E0_MARKERS = (re.compile(r"\bE0[_-](?:ROW|RAW|RECORD|PAYLOAD)\b", re.I),)
# Receipt bodies: `data=E0` (text) or a data / data_class key whose value is E0 (JSON).
_RECEIPT_TEXT = re.compile(r"\bdata(?:_class)?\s*=\s*E0\b")
_DATA_KEYS = frozenset({"data", "data_class", "dataclass"})
MAX_BYTES = 5_000_000


@dataclass(frozen=True)
class Finding:
    path: str
    rule: str
    line: int = 0


def scan_text(text: str, path: str = "<text>") -> list[Finding]:
    out: list[Finding] = []
    for n, line in enumerate(text.splitlines(), 1):
        if any(m.search(line) for m in E0_MARKERS):
            out.append(Finding(path, "e0_marker", n))
    return out


def _is_text(raw: bytes) -> bool:
    return b"\x00" not in raw[:8192]


_B64 = re.compile(rb"[A-Za-z0-9+/_-]{12,}={0,2}")
_MARK_BYTES = re.compile(rb"E0[_-](?:ROW|RAW|RECORD|PAYLOAD)", re.I)


def _decoded_views(raw: bytes) -> list[str]:
    """Text views of raw bytes: utf-8, utf-16 and ascii-in-binary, plus base64 runs. Binary/large files are not skipped."""
    import base64

    views = [raw.decode("utf-8", errors="replace")]
    if b"\x00" in raw:
        views += [raw.decode(enc, errors="ignore") for enc in ("utf-16-le", "utf-16-be")]
        views.append(raw.replace(b"\x00", b"").decode("latin-1"))
    for m in _B64.finditer(raw):
        chunk = m.group(0)
        for alt in (chunk, chunk.replace(b"-", b"+").replace(b"_", b"/")):
            for off in range(4):  # alignment inside a longer run
                piece = alt[off:]
                piece = piece[: len(piece) // 4 * 4]
                try:
                    dec = base64.b64decode(piece)
                except ValueError:
                    continue
                if _MARK_BYTES.search(dec):
                    views.append(dec.decode("utf-8", errors="replace"))
    return views


def scan_file(path: Path, root: Path | None = None) -> list[Finding]:
    rel = str(path.relative_to(root)).replace("\\", "/") if root else str(path)
    try:
        raw = path.read_bytes()
    except OSError:
        return []
    out: list[Finding] = []
    for view in _decoded_views(raw):
        out += scan_text(view, rel)
        if out:
            break
    return out


def _json_has_e0(node: object) -> bool:
    if isinstance(node, dict):
        for k, v in node.items():
            if str(k).lower() in _DATA_KEYS and isinstance(v, str) and v.strip().upper() == "E0":
                return True
            if _json_has_e0(v):
                return True
    elif isinstance(node, list):
        return any(_json_has_e0(v) for v in node)
    return False


def scan_receipt_body(body: str, path: str = "<receipt>") -> list[Finding]:
    hit = bool(_RECEIPT_TEXT.search(body))
    if not hit:
        try:
            hit = _json_has_e0(json.loads(body))
        except ValueError:
            hit = False
    return [Finding(path, "receipt_data_e0")] if hit else []


def is_receipt_path(rel: str) -> bool:
    return "receipt" in rel.lower()


def tracked_files(root: Path) -> list[str]:
    res = subprocess.run(["git", "-C", str(root), "ls-files", "-z", "--cached"], check=True, capture_output=True)
    return [p for p in res.stdout.decode("utf-8", errors="replace").split("\0") if p]


def push_scan(root: Path) -> list[Finding]:
    root = root.resolve()
    findings: list[Finding] = []
    for rel in tracked_files(root):
        p = root / rel
        if not p.is_file():
            continue  # deleted in the working tree
        findings += scan_file(p, root)
        if is_receipt_path(rel):
            try:
                raw = p.read_bytes()
            except OSError:
                continue
            if len(raw) <= MAX_BYTES and _is_text(raw):
                findings += scan_receipt_body(raw.decode("utf-8", errors="replace"), rel)
    return findings


def _report(findings: list[Finding]) -> None:
    for f in findings:
        print(f"BLOCKED {f.path}:{f.line} {f.rule}", file=sys.stderr)  # never echo the matched content


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="dataclass_gate")
    sub = ap.add_subparsers(dest="cmd", required=True)
    ps = sub.add_parser("push-scan")
    ps.add_argument("--root", default=".")
    sc = sub.add_parser("scan")
    sc.add_argument("files", nargs="+")
    args = ap.parse_args(argv)
    if args.cmd == "push-scan":
        findings = push_scan(Path(args.root))
    else:
        findings = [f for p in args.files for f in scan_file(Path(p))]
    _report(findings)
    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
