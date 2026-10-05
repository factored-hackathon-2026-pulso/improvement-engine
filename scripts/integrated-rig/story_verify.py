#!/usr/bin/env python3
"""API-level verification of the integrated story (stage 1). Standard library only. Prints aggregates and ids, never a credential.

    story_verify.py cases  --platform URL --out case-ids.json
    story_verify.py verify --platform URL --core URL --tokens .dev-stack/tokens.json --loop-result F --announce F --out report.json

`cases`   logs in as the seeded supervisor and writes the ids of the platform's seeded OPEN cases (the G1 demo evidence list).
`verify`  for every announced proposal of the loop result checks, in this order:
            1. agent-core (builder token of the engine): origin auto_detect, state draft, created_by the engine, a dossier (docs.description) on
               the changes, an eval_suite change when the draft carries one;
            2. the announce POST and its replay (200/201 each) recorded by run_story.ps1;
            3. each seeded supervisor (password and dev MFA code of the SEEDED dev accounts): exactly ONE `improvement_proposed` notification
               for the proposal, with the dossier summary and CASE- evidence links;
            4. the platform Automatizacion list (`/builder/proposals`): the proposal is there with source `engine`;
            5. the proposal detail (`/builder/proposals/{id}`): title, docs.description, docs.rationale, docs.changelog.
          Exit 0 only when every check passes.
Dev stack only: the accounts, the password and the MFA code are the platform's seeded demo values (CC_ENV=dev).
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

SUPERVISORS = (("Lucia Herrera", "lucia.herrera"), ("Martin Salazar", "martin.salazar"), ("Renata Villalba", "renata.villalba"),
               ("Felipe Echeverri", "felipe.echeverri"))
DOMAIN = "latambank.example"
DEV_PASSWORD = "demo1234"  # seeded demo accounts, dev stack only (platform seed + report E2E_RIG_AND_RELAXATIONS)
DEV_MFA = "000000"  # the platform's CC_DEV_MFA_CODE for seeded accounts, dev stack only
CASE_ID = re.compile(r"^CASE-[0-9A-HJKMNP-TV-Z]{26}$")
SECRET_PATTERN = re.compile(r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{5,}\.[A-Za-z0-9_\-]*|svc-[A-Za-z0-9]{20,}")

_secrets: list[str] = []


def mask(text: str) -> str:
    """Hide any registered secret and anything shaped like a JWS or a service token."""
    for s in _secrets:
        if s and len(s) >= 6:
            text = text.replace(s, "***")
    return SECRET_PATTERN.sub("***", text)


def say(text: str = "") -> None:
    print(mask(text))


def call(method: str, url: str, token: str | None = None, body: dict | None = None, headers: dict | None = None, timeout: float = 30.0):
    """(status, parsed json or None). Never raises on an HTTP error status."""
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            status = r.status
    except urllib.error.HTTPError as exc:
        raw, status = exc.read(), exc.code
    except (urllib.error.URLError, TimeoutError, OSError):
        return 0, None
    try:
        return status, json.loads(raw.decode("utf-8")) if raw else None
    except ValueError:
        return status, None


def login(platform: str, username: str) -> str | None:
    status, body = call("POST", f"{platform}/api/v1/auth/login", body={"email": f"{username}@{DOMAIN}", "password": DEV_PASSWORD})
    if status != 200 or not body:
        return None
    status, body = call("POST", f"{platform}/api/v1/auth/mfa", body={"challengeId": body["challengeId"], "code": DEV_MFA, "method": (body.get("methods") or ["totp"])[0]})
    if status != 200 or not body:
        return None
    _secrets.append(body["token"])
    return body["token"]


# ---- pure parts (unit-tested) ---------------------------------------------------------------------------------------------------------

def improvement_notices(items: list, proposal_id: str) -> list:
    """The `improvement_proposed` notifications of a proposal in one page of a person's notifications."""
    return [n for n in items or [] if n.get("kind") == "improvement_proposed" and (n.get("improvement") or {}).get("proposalId") == proposal_id]


def change_summary(changes: list) -> dict:
    kinds: dict[str, int] = {}
    with_description = 0
    for c in changes or []:
        kinds[c.get("kind", "?")] = kinds.get(c.get("kind", "?"), 0) + 1
        if ((c.get("docs") or {}).get("description") or "").strip():
            with_description += 1
    return {"count": len(changes or []), "kinds": kinds, "with_description": with_description, "has_eval_suite": "eval_suite" in kinds}


def check(name: str, ok: bool, detail: str = "") -> dict:
    return {"check": name, "ok": bool(ok), "detail": detail}


def verify_core(proposal: dict, pid: str, engine_id: str = "pulso-engine") -> list:
    p = (proposal or {}).get("proposal") or {}
    cs = change_summary((proposal or {}).get("changes") or [])
    return [
        check(f"{pid}: agent-core origin", p.get("origin") == "auto_detect", f"origin={p.get('origin')}"),
        check(f"{pid}: agent-core state", p.get("state") == "draft", f"state={p.get('state')}"),
        check(f"{pid}: agent-core created_by", p.get("created_by") == engine_id, f"created_by={p.get('created_by')}"),
        check(f"{pid}: dossier on the changes", cs["with_description"] > 0, f"{cs['with_description']}/{cs['count']} changes carry docs.description"),
        # informational: the eval_suite is attached by script AFTER this check (EV1 / R6), so its absence is expected at this point
        check(f"{pid}: draft change kinds", True, ("eval_suite present, " if cs["has_eval_suite"] else "no eval_suite yet, ") + ",".join(f"{k}:{v}" for k, v in sorted(cs["kinds"].items()))),
    ]


def verify_detail(detail: dict, pid: str) -> list:
    d = detail or {}
    title = ((d.get("proposal") or {}).get("title") or "").strip()
    docs = [(c.get("docs") or {}) for c in d.get("changes") or []]
    has = {f: any((x.get(f) or "").strip() for x in docs) for f in ("description", "rationale", "changelog")}
    return [check(f"{pid}: detail title", bool(title), f"{len(title)} chars"),
            check(f"{pid}: detail docs.description", has["description"]), check(f"{pid}: detail docs.rationale", has["rationale"]),
            check(f"{pid}: detail docs.changelog", has["changelog"])]


def render_checks(checks: list) -> list:
    w = max([len(c["check"]) for c in checks] + [5])
    out = [f"{'check'.ljust(w)}  result  detail"]
    out += [f"{c['check'].ljust(w)}  {'PASS' if c['ok'] else 'FAIL'}    {c['detail']}" for c in checks]
    return out


# ---- commands -------------------------------------------------------------------------------------------------------------------------

def cmd_cases(a) -> int:
    """G1: the evidence ids. Preferred: the platform's evidence sampler (GET /internal/evidence/cases, service token, k-anonymity floor on
    the cell); fallback: the seeded open cases of the supervisor view. Either way they are example cases, labelled."""
    ids: list[str] = []
    source = "none"
    svc = None
    if a.secrets and Path(a.secrets).exists():
        svc = json.loads(Path(a.secrets).read_text(encoding="utf-8")).get("service_token")
        _secrets.append(svc or "")
    if svc:
        status, body = call("GET", f"{a.platform}/api/v1/internal/evidence/cases?limit=8", svc)
        if status == 200 and body and not body.get("suppressed"):
            ids = [x for x in body.get("caseIds", []) if CASE_ID.match(x)]
            source = f"platform evidence route (matched {body.get('matched')})"
        else:
            say(f"cases: evidence route answered HTTP {status}" + (" (cell suppressed by k-anonymity)" if body and body.get("suppressed") else ""))
    if not ids:
        tok = login(a.platform, "lucia.herrera")
        for lang in ("es", "pt") if tok else ():
            status, body = call("GET", f"{a.platform}/api/v1/supervision/open-cases?language={lang}", tok)
            if status == 200 and body:
                ids += [r["case"]["id"] for r in body.get("cases", []) if CASE_ID.match(r.get("case", {}).get("id", ""))]
        ids = sorted(set(ids))[:8]
        source = "seeded open cases (supervisor view)"
    Path(a.out).parent.mkdir(parents=True, exist_ok=True)
    Path(a.out).write_text(json.dumps({"label": "seeded platform cases, example cases (not the cases the finding came from)", "source": source, "ids": ids}), encoding="utf-8")
    say(f"cases: {len(ids)} case ids from {source}")
    return 0 if ids else 1


def human_progress(state: str, events: list, proposal_id: str) -> dict:
    """Pure: what the registry says about a proposal a person is deciding. `events` are registry export events.
    A `published` event carries the proposal id; the later `promoted` event carries only the RELEASE id (proposal_id is null), so a
    promotion belongs to the proposal when its release id is the one the proposal published."""
    mine = [e for e in events or [] if e.get("proposal_id") == proposal_id]
    releases = {e.get("release_id") for e in mine if e.get("type") in ("published", "release.published") and e.get("release_id")}
    promoted = [e for e in events or [] if e.get("type") in ("promoted", "release.promoted") and e.get("release_id") in releases and e.get("alias") in (None, "prod")]
    types = sorted({e.get("type", "?") for e in mine} | {"promoted@" + str(e.get("alias")) for e in promoted})
    return {"state": state, "events": types, "done": bool(promoted), "release_id": next(iter(releases), None)}


def cmd_wait_human(a) -> int:
    """READ-ONLY wait: a person approves, publishes and promotes in the platform; this only watches agent-core (builder token to read the
    proposal, exporter token to read registry events) and prints one line per change. It never decides anything."""
    import time
    tokens = json.loads(Path(a.tokens).read_text(encoding="utf-8"))
    _secrets.extend(str(v) for v in tokens.values())
    deadline = time.time() + a.timeout_min * 60
    last = None
    t0 = time.time()
    while time.time() < deadline:
        st, body = call("GET", f"{a.core}/v1/registry/proposals/{a.proposal_id}", tokens.get("builder"))
        state = ((body or {}).get("proposal") or {}).get("state") or f"HTTP {st}"
        st2, page = call("GET", f"{a.core}/v1/export/registry-events?after=0&limit=500", tokens.get("exporter"))
        prog = human_progress(state, (page or {}).get("items") if st2 == 200 else [], a.proposal_id)
        key = (prog["state"], tuple(prog["events"]))
        if key != last:
            say(f"wait-for-human [{int(time.time() - t0)} s] proposal state={prog['state']} registry events={','.join(prog['events']) or '-'}")
            last = key
        if prog["done"]:
            say("wait-for-human: published and promoted to prod")
            return 0
        time.sleep(a.interval)
    say(f"wait-for-human: timeout after {a.timeout_min} min")
    return 1


def cmd_verify(a) -> int:
    tokens = json.loads(Path(a.tokens).read_text(encoding="utf-8"))
    _secrets.extend(str(v) for v in tokens.values())
    loop = json.loads(Path(a.loop_result).read_text(encoding="utf-8-sig"))
    ann = json.loads(Path(a.announce).read_text(encoding="utf-8-sig")) if Path(a.announce).exists() else {"results": []}
    results = {r["proposalId"]: r for r in ann.get("results", [])}
    pids = [f["delivery"]["proposal_id"] for f in loop.get("findings", []) if f.get("outcome") == "announced" and f.get("delivery")]
    checks: list[dict] = []
    if not pids:
        checks.append(check("announced proposals in the loop result", False, "none: nothing to verify"))
    sessions = {}
    for label, user in SUPERVISORS:
        sessions[label] = login(a.platform, user)
    for label, tok in sessions.items():
        checks.append(check(f"login {label}", tok is not None, "dev password + dev MFA code of the seeded account"))
    first = next((t for t in sessions.values() if t), None)
    for pid in pids:
        st, body = call("GET", f"{a.core}/v1/registry/proposals/{pid}", tokens.get("builder"))
        checks += verify_core(body, pid) if st == 200 else [check(f"{pid}: agent-core read", False, f"HTTP {st}")]
        r = results.get(pid)
        if r is None:
            checks.append(check(f"{pid}: announce", False, "no announce result recorded"))
        else:
            checks.append(check(f"{pid}: announce POST", r.get("status") in (200, 201), f"HTTP {r.get('status')}, {len(r.get('evidenceLinks') or [])} evidence links"))
            checks.append(check(f"{pid}: announce replay idempotent", r.get("replay_status") in (200, 201), f"HTTP {r.get('replay_status')}"))
            checks.append(check(f"{pid}: evidence links are CASE- ids", all(CASE_ID.match(x) for x in r.get("evidenceLinks") or []) and bool(r.get("evidenceLinks")), ""))
        for label, tok in sessions.items():
            if not tok:
                continue
            st, page = call("GET", f"{a.platform}/api/v1/me/notifications?limit=50", tok)
            mine = improvement_notices((page or {}).get("items"), pid) if st == 200 else []
            checks.append(check(f"{pid}: notification for {label}", len(mine) == 1, f"{len(mine)} improvement_proposed (want exactly 1)"))
        if first:
            st, lst = call("GET", f"{a.platform}/api/v1/builder/proposals?refresh=false", first)
            row = next((i for i in (lst or {}).get("items", []) if i.get("proposalId") == pid), None)
            checks.append(check(f"{pid}: listed in Automatizacion", row is not None, f"HTTP {st}"))
            checks.append(check(f"{pid}: list source engine", bool(row) and row.get("source") == "engine", f"source={(row or {}).get('source')}"))
            st, det = call("GET", f"{a.platform}/api/v1/builder/proposals/{pid}", first)
            checks += verify_detail(det, pid) if st == 200 else [check(f"{pid}: detail fetch", False, f"HTTP {st}")]
    for line in render_checks(checks):
        say(line)
    passed = sum(1 for c in checks if c["ok"])
    say(f"verify: {passed}/{len(checks)} checks passed, {len(pids)} announced proposal(s)")
    Path(a.out).write_text(json.dumps({"passed": passed, "total": len(checks), "proposals": pids, "checks": checks}, indent=2), encoding="utf-8")
    return 0 if passed == len(checks) and pids else 1


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("cases")
    c.add_argument("--platform", required=True)
    c.add_argument("--out", required=True)
    c.add_argument("--secrets", help="rig secrets.json (service token for the evidence route)")
    w = sub.add_parser("wait-human")
    w.add_argument("--core", required=True)
    w.add_argument("--tokens", required=True)
    w.add_argument("--proposal-id", required=True)
    w.add_argument("--timeout-min", type=int, default=60)
    w.add_argument("--interval", type=float, default=10.0)
    v = sub.add_parser("verify")
    v.add_argument("--platform", required=True)
    v.add_argument("--core", required=True)
    v.add_argument("--tokens", required=True)
    v.add_argument("--loop-result", required=True)
    v.add_argument("--announce", required=True)
    v.add_argument("--out", required=True)
    a = ap.parse_args(argv)
    return {"cases": cmd_cases, "wait-human": cmd_wait_human, "verify": cmd_verify}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
