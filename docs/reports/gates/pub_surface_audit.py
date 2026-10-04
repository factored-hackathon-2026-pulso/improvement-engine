"""G0gr pub-surface audit: counts pub types, serde-derived types and compile_fail references.

Usage: python pub_surface_audit.py REPO_ROOT [--out report.json] ; exit 1 if counts are absent.
Static text scan only (no cargo). Counts are lexical, so macro-generated items are not seen.
"""
import json
import pathlib
import re
import sys

PUB_TYPE = re.compile(r"^\s*pub\s+(?:struct|enum|trait|type|union)\b")
PUB_RESTRICTED = re.compile(r"^\s*pub\s*\(")
ITEM = re.compile(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:struct|enum|trait|type|union)\b")
DERIVE = re.compile(r"#\[derive\([^\]]*\b(?:Serialize|Deserialize)\b")
KEYS = ("pub_types", "serde_derive_types", "compile_fail_refs", "files")
SKIP = {"target", ".git", "node_modules"}


def scan_text(text):
    pub = serde = cf = 0
    pending = False
    for line in text.splitlines():
        s = line.strip()
        if DERIVE.search(line):
            pending = True
        if "compile_fail" in line:
            cf += 1
        if ITEM.match(line):
            if pending:
                serde += 1
            if PUB_TYPE.match(line):
                pub += 1
            pending = False
        elif s and not s.startswith(("#", "//")):
            pending = False
    return {"pub_types": pub, "serde_derive_types": serde, "compile_fail_refs": cf, "files": 1}


def _rs_files(root):
    for p in sorted(root.rglob("*.rs")):
        if not SKIP.intersection(p.relative_to(root).parts):
            yield p


def measure_by_crate(root):
    out = {}
    for p in _rs_files(root):
        rel = p.relative_to(root).parts
        key = "/".join(rel[:2]) if rel[0] == "crates" and len(rel) > 2 else rel[0]
        c = scan_text(p.read_text(encoding="utf-8", errors="replace"))
        agg = out.setdefault(key, dict.fromkeys(KEYS, 0))
        for k in KEYS:
            agg[k] += c[k]
    return out


def measure(root):
    tot = dict.fromkeys(KEYS, 0)
    for v in measure_by_crate(root).values():
        for k in KEYS:
            tot[k] += v[k]
    return tot


def check(totals):
    return [f"missing {k}" for k in KEYS if not isinstance(totals.get(k), int)]


def main(argv):
    root = pathlib.Path(argv[1])
    report = {"totals": measure(root), "per_crate": measure_by_crate(root)}
    errs = check(report["totals"])
    if "--out" in argv:
        pathlib.Path(argv[argv.index("--out") + 1]).write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report["totals"]))
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
