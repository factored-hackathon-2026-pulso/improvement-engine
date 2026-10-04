"""Engine-run report contract (C-2, WP G1): report checks, honesty tests, doubles[] generator.

Pure stdlib. The DC0 scanner is reused by import path (scripts/dc/dataclass_gate.py), not edited.
The roleplay-llm scanner id ("tps-1") is accepted on receipts; its package is not imported here.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path

_ROOT = Path(__file__).resolve().parents[2]


def _load_dc0():
    spec = importlib.util.spec_from_file_location("dc0_dataclass_gate", _ROOT / "scripts" / "dc" / "dataclass_gate.py")
    mod = importlib.util.module_from_spec(spec)
    import sys
    sys.modules[spec.name] = mod  # dataclasses need the module registered
    spec.loader.exec_module(mod)
    return mod


_dc0 = _load_dc0()
SCANNER_ID = _dc0.SCANNER_ID
KNOWN_SCANNER_IDS = frozenset({SCANNER_ID, "tps-1"})

STATUSES = frozenset({"real", "real-narrow", "local-model", "agent_roleplay", "recorded", "stand-in",
                      "simulated", "not_exercised"})
NONREAL_PROVIDERS = frozenset({"agent_roleplay", "recorded", "scripted"})
THIRD_PARTY_PROVIDERS = frozenset({"hosted", "agent_roleplay"})
RESTRICTED_CLASSES = frozenset({"E0", "CSV", "original-treated"})
LABELS = ["DEMO-0", "DEMO-1a", "DEMO-1b", "DEMO-2"]
PROVENANCE_KEYS = ("target", "sha", "contract_revision", "host")


def _v(rule, where, msg):
    return {"rule": rule, "where": where, "msg": msg}


def scan_receipt(body: str) -> list:
    return _dc0.scan_receipt_body(body)


def _status_ok(s):
    return s in STATUSES or (isinstance(s, str) and s.startswith("blocked(") and s.endswith(")") and len(s) > 9)


def check(report: dict) -> list:
    out = []
    steps = report.get("steps", [])
    for st in steps:
        sid = st.get("id", "?")
        if not _status_ok(st.get("status")):
            out.append(_v("S1", sid, "status outside honesty vocabulary"))
        if not st.get("data_class"):
            out.append(_v("S1", sid, "missing data_class"))
        for k in PROVENANCE_KEYS:
            if not st.get(k):
                out.append(_v("S1", sid, f"missing {k}"))
        rc = st.get("receipt") or {}
        prov = rc.get("provider")
        if st.get("status") == "real" and prov in NONREAL_PROVIDERS:
            out.append(_v("H1", sid, f"real with {prov} receipt provider"))
        if st.get("data_class") in RESTRICTED_CLASSES and prov in THIRD_PARTY_PROVIDERS | {"hosted"}:
            if rc.get("scanner_id") not in KNOWN_SCANNER_IDS:
                out.append(_v("H2", sid, "restricted class to third party without passing scanner id"))
            if prov == "hosted" and rc.get("model_kind") == "real" and not rc.get("third_party_ok"):
                out.append(_v("H2", sid, "hosted real model needs third_party_ok"))
        if (st.get("stage_output") or {}).get("source") == "template_fallback":
            out.append(_v("H4", sid, "template-fallback stage output"))
        if rc.get("body") is not None and scan_receipt(rc["body"]):
            out.append(_v("H8", sid, "receipt body carries E0 data class"))
    ports = report.get("ports") or []
    if not ports:
        out.append(_v("H4", "ports", "no per-port Core double provenance listed"))
    for p in ports:
        if not p.get("provenance") or not p.get("price_source"):
            out.append(_v("H4", p.get("port", "?"), "port needs provenance and price_source"))
    a = report.get("authors") or {}
    roles = ("world", "suite", "effect", "judge")
    missing = [r for r in roles if not a.get(r)]
    for r in missing:
        out.append(_v("H5", r, "missing author"))
    if not missing and len({a[r] for r in roles}) != len(roles):
        out.append(_v("H5", "authors", "world, suite, effect and judge authors must be distinct"))
    sealed, cand = a.get("suite_sealed_at"), a.get("candidate_created_at")
    if not sealed or not cand or sealed >= cand:
        out.append(_v("H5", "suite", "suite must be sealed before the candidate exists"))
    hosts = {report.get("host")} | {s.get("host") for s in steps}
    label = report.get("label")
    if "python" in hosts and label in LABELS and LABELS.index(label) > LABELS.index("DEMO-1a"):
        out.append(_v("H6", "label", "host=python cannot carry a label above DEMO-1a"))
    s1 = next((s for s in steps if s.get("id") == "scout"), None)
    s2 = next((s for s in steps if s.get("id") == "verifier"), None)
    if s1 and s2:
        if s1.get("actor") == s2.get("actor") or s1.get("model") == s2.get("model"):
            out.append(_v("H7", "verifier", "scout and verifier need distinct actors and model identities"))
    return out


def check_mapping_mutation(mapper, categories):
    """Honesty test 3 skeleton: rename and permute mutation test (turned green by ED0 and SMAP)."""
    raise NotImplementedError("ED0/SMAP provide the mapping under test")


def generate_doubles(report: dict, observed: dict) -> list:
    """Engine-generated doubles[]: only from the report's observed facts, never hand-written."""
    d = []
    for st in report.get("steps", []):
        if st.get("status") != "real":
            d.append({"part": st["id"], "status": st["status"], "data_class": st.get("data_class"),
                      "provider": (st.get("receipt") or {}).get("provider")})
    for k, v in sorted(observed.items()):
        d.append({"part": k.replace("_", ".", 1), "status": v, "observed": True})
    scanners = sorted({(s.get("receipt") or {}).get("scanner_id") for s in report.get("steps", [])} - {None})
    d.extend({"part": "scanner", "status": sid} for sid in scanners)
    for p in report.get("ports", []):
        d.append({"part": f"port.{p.get('port')}", "status": p.get("provenance"), "price_source": p.get("price_source")})
    return d
