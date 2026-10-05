#!/usr/bin/env python3
"""Drive MANUAL-origin proposals through draft / evaluate / approve / reject on the OWN local stack (live_stack.py).

LOCAL TEST DATA ONLY (loopback guard; the local admin token acts as the approver stand-in). Creates N proposals on the
agent `disputas`, each with a one-sentence prompt change and the 2-scenario synthetic suite derived from
agent-core-assets/eval-suites/pulso-min, then leaves them in the states of PLAN. The token is read from
.dev-stack/<prefix>/tokens.json and is never printed. Needs PyYAML (`uv run --with pyyaml`).

    python scripts/feedback/live_drive.py --prefix pulso-fdbk1 [--base http://127.0.0.1:8012]
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SUITE = REPO / "agent-core-assets" / "eval-suites" / "pulso-min" / "disputas" / "disputas-min@1.0.0.yaml"
LOOPBACK = {"127.0.0.1", "localhost", "::1"}
# (label, finding key header, final action, reject reason text sent as the reviewer would type it)
PLAN = [
    ("A", "M4|Queja|Phone|es", "approve", None),
    ("B", "M4|Queja|Phone|es", "reject", "Duplicado: ya existe una propuesta igual. Contactar a Juan al 3001234567"),
    ("C", "M2|Cargo|App|es", "reject", "Demasiado riesgo para produccion"),
    ("D", "M2|Cargo|App|es", "freeze", None),
    ("E", "M7|Tarjeta|Web|pt", "draft", None),
]


class Api:
    def __init__(self, base: str, token: str):
        u = urllib.parse.urlparse(base)
        if u.hostname not in LOOPBACK:
            raise SystemExit("only loopback hosts are allowed")
        self.base, self.token = base.rstrip("/") + "/v1/registry", token

    def call(self, method: str, path: str, body=None):
        req = urllib.request.Request(self.base + path, method=method,
                                     data=None if body is None else json.dumps(body).encode(),
                                     headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=900) as r:
                return r.status, json.loads(r.read() or b"{}")
        except urllib.error.HTTPError as e:
            raw = e.read() or b"{}"
            return e.code, json.loads(raw) if raw[:1] == b"{" else {"raw": raw[:200].decode("utf-8", "replace")}


def mini_suite() -> dict:
    import yaml
    s = yaml.safe_load(SUITE.read_text(encoding="utf-8"))
    s["id"], s["scenarios"], s["repetitions"] = "disputas-fdbk1", s["scenarios"][:2], 1
    return s


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--prefix", default="pulso-fdbk1")
    ap.add_argument("--base", default="http://127.0.0.1:8012")
    ap.add_argument("--only", help="comma list of PLAN labels")
    a = ap.parse_args()
    tok = json.loads((REPO / ".dev-stack" / a.prefix / "tokens.json").read_text(encoding="utf-8"))["admin"]
    api = Api(a.base, tok)
    st, base_prompt = api.call("GET", "/entities/prompt/p/resumen_radicado")
    assert st == 200, st
    suite, out = mini_suite(), []
    for i, (label, key, final, reason) in enumerate(PLAN):
        if a.only and label not in a.only.split(","):
            continue
        row = {"label": label, "final": final}
        st, p = api.call("POST", "/proposals", {"agent_id": "disputas", "origin": "manual",
                                               "title": f"prompt:p/resumen_radicado - {key.split('|')[0]}: fdbk1 {label}"})
        pid = row["proposal_id"] = p["proposal_id"]
        content = dict(base_prompt["content"])
        content["version"] = f"1.0.{i + 1}"
        content["locales"] = {k: v + " " for k, v in content["locales"].items()}
        content["locales"]["es"] += f"Variante {label} de prueba local."
        changes = [
            {"kind": "prompt", "content": content, "docs": {
                "description": f"Variante {label} (prueba local FDBK1)",
                "rationale": f"[finding: {key}] Hipotesis de prueba sobre una asociacion; solo datos sinteticos.",
                "changelog": "Prueba local de retroalimentacion de decisiones."}},
            {"kind": "eval_suite", "content": suite, "docs": {
                "description": "Suite sintetica minima (2 escenarios)", "rationale": "Hace alcanzable evaluate.",
                "changelog": "Suite local FDBK1."}},
        ]
        if final == "draft":
            row["state"] = "draft"
            out.append(row)
            continue
        st, _ = api.call("PUT", f"/proposals/{pid}/draft", {"expected_rev": p["rev"], "changes": changes})
        st, v = api.call("POST", f"/proposals/{pid}/validate")
        row["valid"] = v.get("valid")
        st, _ = api.call("POST", f"/proposals/{pid}/freeze")
        row["freeze"] = st
        if final == "freeze":
            row["state"] = "candidate"
            out.append(row)
            continue
        st, rep = api.call("POST", f"/proposals/{pid}/evaluate", {"suite_id": suite["id"], "suite_version": suite["version"]})
        row["evaluate"] = [st, rep.get("verdict")]
        if rep.get("verdict") != "pass":
            row["state"] = "evaluated-not-pass"
            out.append(row)
            continue
        st, d = api.call("GET", f"/proposals/{pid}")
        h = d["proposal"]["candidate_hash"]
        if final == "approve":
            st, ap_ = api.call("POST", f"/proposals/{pid}/approve", {"candidate_hash": h})
            row["approve_http"], row["state"] = st, "approved" if st == 200 else "approve-failed"
        else:
            st, _ = api.call("POST", f"/proposals/{pid}/reject", {"reason": reason})
            row["reject_http"], row["state"] = st, "rejected" if st == 200 else "reject-failed"
        out.append(row)
    print(json.dumps(out, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
