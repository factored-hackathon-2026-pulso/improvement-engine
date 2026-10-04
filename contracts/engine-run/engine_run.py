"""Engine-run report contract (C-2, WP G1): report checks, honesty tests, doubles[] generator.

Pure stdlib. The DC0 scanner is reused by import path (scripts/dc/dataclass_gate.py), not edited.
The roleplay-llm scanner id ("tps-1") is accepted on receipts; its package is not imported here.
"""
from __future__ import annotations

import importlib.util
import re
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
NONREAL_PROVIDERS = frozenset({"agent_roleplay", "recorded", "scripted", "claude-standin"})
THIRD_PARTY_PROVIDERS = frozenset({"hosted", "agent_roleplay"})
RESTRICTED_CLASSES = frozenset({"E0", "CSV", "original-treated"})
LABELS = ["DEMO-0", "DEMO-1a", "DEMO-1b", "DEMO-2"]
PROVENANCE_KEYS = ("target", "sha", "contract_revision", "host")


def _v(rule, where, msg):
    return {"rule": rule, "where": where, "msg": msg}


def scan_receipt(body: str) -> list:
    return _dc0.scan_receipt_body(body)


_BLOCKED = re.compile(r"blocked\([A-Za-z0-9._-]+\)")


def _status_ok(s):
    return isinstance(s, str) and (s in STATUSES or _BLOCKED.fullmatch(s) is not None)


def _norm(x):
    """Identity normalisation: case, surrounding space, and -/_/space separators are not distinct identities."""
    return re.sub(r"[\s_\-]+", "-", x.strip().casefold()) if isinstance(x, str) else x


def _restricted(dc):
    """Restricted data class by normalised prefix: E0-derived, e0_treated, CSV, Original Treated all count."""
    n = _norm(dc)
    return isinstance(n, str) and n.startswith(("e0", "csv", "original"))


def _lies_real(st):
    prov = (st.get("receipt") or {}).get("provider")
    return st.get("status") == "real" and (not prov or _norm(prov) in {_norm(p) for p in NONREAL_PROVIDERS})


def _exercised(st):
    s = st.get("status")
    return s != "not_exercised" and not (isinstance(s, str) and _BLOCKED.fullmatch(s))


def _check_gate(report: dict, steps: list) -> list:
    """G1: approval or publish after a gate that did not pass needs an explicit, labelled human override
    (`overrides[]`: step=approval, of=gate, verdict=<the gate verdict>, by=human, a reason) and quality_claims forbidden.
    A report that exercises approval/publish must state the gate verdict. A stray override on a passing gate is rejected."""
    out = []
    ran = [s for s in steps if s.get("id") in ("approval", "publish") and _exercised(s)]
    gate = (report.get("gate") or {}).get("verdict")
    ovs = report.get("overrides") or []
    if ran and not gate:
        out.append(_v("G1", "gate", "approval/publish exercised without a reported gate verdict"))
    if gate == "pass" and ovs:
        out.append(_v("G1", "overrides", "override recorded although the gate passed"))
    if ran and gate and gate != "pass":
        good = [o for o in ovs if isinstance(o, dict) and o.get("step") == "approval" and o.get("of") == "gate"
                and o.get("verdict") == gate and o.get("by") == "human" and o.get("label") == "human_override"
                and isinstance(o.get("reason"), str) and o["reason"].strip()]
        if not good:
            out.append(_v("G1", "gate", f"approval/publish after gate verdict {gate!r} without a labelled human override"))
        elif report.get("label") == "DEMO-0" and not all(o.get("simulated") is True for o in good):
            out.append(_v("G1", "overrides", "a DEMO-0 override is a SIMULATED human and must say simulated: true"))
        elif report.get("quality_claims") != "forbidden":
            out.append(_v("G1", "quality_claims", "a human override of a failed gate requires quality_claims forbidden"))
    return out


def check(report: dict) -> list:
    out = []
    steps = report.get("steps") or []
    if not steps:
        out.append(_v("S1", "steps", "report lists no steps"))
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
        if _lies_real(st):
            out.append(_v("H1", sid, f"real with {prov!r} receipt provider"))
        nprov = _norm(prov)
        if _restricted(st.get("data_class")) and nprov in {_norm(p) for p in THIRD_PARTY_PROVIDERS}:
            if rc.get("scanner_id") not in KNOWN_SCANNER_IDS:
                out.append(_v("H2", sid, "restricted class to third party without passing scanner id"))
            if nprov == "hosted" and rc.get("model_kind") == "real" and not rc.get("third_party_ok"):
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
    if not missing and len({_norm(a[r]) for r in roles}) != len(roles):
        out.append(_v("H5", "authors", "world, suite, effect and judge authors must be distinct"))
    sealed, cand = a.get("suite_sealed_at"), a.get("candidate_created_at")
    compile_step = next((s for s in steps if s.get("id") == "compile"), None)
    no_candidate = compile_step is not None and not _exercised(compile_step) and cand is None
    if not sealed or (not no_candidate and (not cand or sealed >= cand)):
        out.append(_v("H5", "suite", "suite must be sealed before the candidate exists"))
    hosts = {report.get("host")} | {s.get("host") for s in steps}
    label = report.get("label")
    if "python" in hosts and label in LABELS and LABELS.index(label) > LABELS.index("DEMO-1a"):
        out.append(_v("H6", "label", "host=python cannot carry a label above DEMO-1a"))
    s1 = next((s for s in steps if s.get("id") == "scout"), None)
    s2 = next((s for s in steps if s.get("id") == "verifier"), None)
    if s1 and s2:
        if _norm(s1.get("actor")) == _norm(s2.get("actor")) or _norm(s1.get("model")) == _norm(s2.get("model")):
            out.append(_v("H7", "verifier", "scout and verifier need distinct actors and model identities"))
    out.extend(_check_gate(report, steps))
    return out


def check_mapping_mutation(mapper, categories):
    """Honesty test 3: rename and permute mutation test (evidence from ED0's sensor run).

    `categories` maps label -> support (evidence the label carries); `mapper(categories)`
    returns the winning label. The winner must follow the data: after a rename the winner is
    the renamed label of the same evidence, and after a permutation of labels over the
    evidence it is the label that now carries the original winner's evidence. A mapping keyed
    to the winning label fails.
    """
    if not categories:
        return []
    out = []
    labels = sorted(categories)
    winner = mapper(dict(categories))
    renamed = {f"renamed-{i}": categories[l] for i, l in enumerate(labels)}
    want = f"renamed-{labels.index(winner)}" if winner in categories else None
    if want is None or mapper(dict(renamed)) != want:
        out.append(_v("H3", "mapping", "winner changed or ignored the evidence when labels were renamed"))
    rotated = {labels[(i + 1) % len(labels)]: categories[l] for i, l in enumerate(labels)}
    want = labels[(labels.index(winner) + 1) % len(labels)] if winner in categories else None
    if want is None or mapper(dict(rotated)) != want:
        out.append(_v("H3", "mapping", "winner did not follow its evidence when labels were permuted"))
    return out


def generate_doubles(report: dict, observed: dict) -> list:
    """Engine-generated doubles[]: only from the report's observed facts, never hand-written."""
    d = []
    for st in report.get("steps", []):
        if st.get("status") != "real" or _lies_real(st):
            d.append({"part": st["id"], "status": st["status"], "data_class": st.get("data_class"),
                      "provider": (st.get("receipt") or {}).get("provider")})
    for k, v in sorted(observed.items()):
        d.append({"part": k.replace("_", ".", 1), "status": v, "observed": True})
    scanners = sorted({(s.get("receipt") or {}).get("scanner_id") for s in report.get("steps", [])} - {None})
    d.extend({"part": "scanner", "status": sid} for sid in scanners)
    for o in report.get("overrides") or []:
        d.append({"part": f"{o.get('of')}.override",
                  "status": f"{o.get('label')}(simulated human, DEMO-0 stand-in)" if o.get("simulated") else o.get("label"), "verdict": o.get("verdict"), "by": o.get("by")})
    for p in report.get("ports", []):
        d.append({"part": f"port.{p.get('port')}", "status": p.get("provenance"), "price_source": p.get("price_source")})
    return d
