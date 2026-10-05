"""Attach a Pulso eval_suite to a proposal draft on the LOCAL agent-core stack and run `evaluate`.

    python scripts/dev-stack/attach_eval_suite.py --suite agent-core-assets/eval-suites/pulso-min/disputas/disputas-min@1.0.0.yaml \
        [--proposal-id <id> | --create] [--changes extra-changes.json] [--base http://127.0.0.1:8001] [--offline-only]

What it does (and only this): validate the suite with agent-core's own `EvalSuite` / `suite_problems` (offline, uv),
then, if the local registry API answers, add the `eval_suite` change to the draft (existing changes kept), `validate`,
`freeze`, `evaluate`, and print pass/fail per gate item. It NEVER calls approve, publish, promote or reject.
If the stack is down the live part is reported as `not_exercised` and the exit code is 3.

Credentials: the local staff token is read from <state-dir>/tokens.json (gitignored, written by stack.py) and sent only
as a bearer header to the local registry. It is never printed. Needs PyYAML (`uv run --with pyyaml ...` works).
Env: PULSO_AGENT_CORE_DIR (agent-core checkout used for the offline validator).
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

OFFLINE_CODE = r"""
import json, sys
from agent_core.registry.suite import EvalSuite, suite_problems
from agent_core.domain import Agent
d = json.load(sys.stdin)
suite = EvalSuite.model_validate(d["suite"])
agent = Agent.model_validate(d["agent"])
probs = [p.model_dump(mode="json") for p in suite_problems(agent, suite)]
print(json.dumps({"scenarios": len(suite.scenarios), "problems": probs}))
"""


def load_suite(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".json":
        return json.loads(text)
    import yaml  # PyYAML
    return yaml.safe_load(text)


def offline_validate(suite: dict, agent: dict, ac_dir: Path) -> dict:
    cmd = ["uv", "run", "--offline", "--project", str(ac_dir), "python", "-c", OFFLINE_CODE]
    r = subprocess.run(cmd, input=json.dumps({"suite": suite, "agent": agent}), capture_output=True, text=True,
                       cwd=ac_dir)
    if r.returncode != 0:
        return {"error": (r.stderr or r.stdout)[-600:]}
    return json.loads(r.stdout.strip().splitlines()[-1])


class Api:
    def __init__(self, base: str, token: str) -> None:
        self.base, self.token = base.rstrip("/"), token

    def call(self, method: str, path: str, body: object | None = None) -> tuple[int, dict]:
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + "/v1/registry" + path, data=data, method=method,
                                     headers={"Authorization": "Bearer " + self.token,
                                              "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=900) as resp:
                return resp.status, json.loads(resp.read() or b"{}")
        except urllib.error.HTTPError as err:
            raw = err.read() or b"{}"
            try:
                return err.code, json.loads(raw)
            except ValueError:
                return err.code, {"raw": raw[:300].decode("utf-8", "replace")}


def stack_up(base: str) -> bool:
    try:
        with urllib.request.urlopen(base.rstrip("/") + "/healthz", timeout=3) as r:
            return r.status == 200
    except Exception:
        return False


def with_default_thresholds(suite: dict, agent: dict) -> tuple[dict, list[str]]:
    """Every gate/guardrail metric the agent declares needs a threshold (suite_problems missing_threshold).
    Added with noise_margin 0 and NO floor (a floor is a human/measured decision); listed in the report."""
    added: list[str] = []
    out = dict(suite)
    out["thresholds"] = dict(suite.get("thresholds") or {})
    for m in agent.get("metrics") or []:
        if m.get("role") in ("gate", "guardrail") and m["id"] not in out["thresholds"]:
            out["thresholds"][m["id"]] = {"noise_margin": "0"}
            added.append(m["id"])
    return out, added


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--suite", type=Path, required=True)
    ap.add_argument("--base", default="http://127.0.0.1:8001")
    ap.add_argument("--state-dir", type=Path, default=REPO / ".dev-stack")
    ap.add_argument("--token-key", default="admin", help="key in tokens.json; must carry the constructor role")
    ap.add_argument("--proposal-id")
    ap.add_argument("--create", action="store_true", help="create a manual proposal for the suite's agent")
    ap.add_argument("--changes", type=Path, help="JSON list of extra EntityDraft changes to put in the same draft")
    ap.add_argument("--offline-only", action="store_true")
    ap.add_argument("--agent-file", type=Path, help="agent entity YAML/JSON for the offline check when no stack is up")
    ap.add_argument("--agent-core", type=Path, default=Path(os.environ.get("PULSO_AGENT_CORE_DIR", "")) or None)
    args = ap.parse_args()

    suite = load_suite(args.suite)
    agent_id = suite["agent_id"]
    report: dict = {"suite": f'{suite["id"]}@{suite["version"]}', "agent": agent_id,
                    "scenarios": len(suite["scenarios"])}
    up = (not args.offline_only) and stack_up(args.base)
    api = None
    agent_entity: dict | None = None
    if up:
        tok = json.loads((args.state_dir / "tokens.json").read_text(encoding="utf-8"))[args.token_key]
        api = Api(args.base, tok)
        st, ent = api.call("GET", f"/entities/agent/{agent_id}")
        if st == 200:
            agent_entity = ent.get("content") or ent.get("spec") or ent
        report["agent_read_http"] = st
    if agent_entity is None and args.agent_file:
        agent_entity = load_suite(args.agent_file)
    suite, added = with_default_thresholds(suite, agent_entity or {})
    report["thresholds_added_without_floor"] = added
    if agent_entity is not None and args.agent_core and str(args.agent_core):
        report["offline"] = offline_validate(suite, agent_entity, args.agent_core)
    else:
        report["offline"] = "not_run (no agent entity or no PULSO_AGENT_CORE_DIR)"
    if not up:
        report["live"] = "not_exercised: " + ("--offline-only" if args.offline_only else f"no stack at {args.base}")
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return 3
    assert api is not None
    extra = json.loads(args.changes.read_text(encoding="utf-8")) if args.changes else []
    pid = args.proposal_id
    if not pid:
        if not args.create:
            sys.exit("give --proposal-id or --create")
        st, p = api.call("POST", "/proposals", {"agent_id": agent_id, "origin": "manual",
                                               "title": f"EV1 attach {report['suite']}"})
        report["create_http"] = st
        if st != 201:
            report["live"] = {"failed_at": "create", "body": p}
            print(json.dumps(report, indent=2, ensure_ascii=False))
            return 1
        pid = p["proposal_id"]
    st, p = api.call("GET", f"/proposals/{pid}")
    if st != 200:
        report["live"] = {"failed_at": "get", "http": st, "body": p}
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return 1
    rev = (p.get("proposal") or p).get("rev", 0)
    kept = [c for c in (p.get("changes") or []) if not (c.get("kind") == "eval_suite"
                                                         and (c.get("content") or {}).get("id") == suite["id"])]
    draft = kept + extra + [{"kind": "eval_suite", "content": suite, "docs": {
        "description": f'Synthetic minimal eval_suite {suite["id"]} for {agent_id}',
        "rationale": "Makes agent-core evaluation reachable: scripted synthetic scenarios, es/pt, protected behaviours.",
        "changelog": "Adds the suite (new yardstick)."}}]
    live: dict = {"proposal_id": pid}
    st, body = api.call("PUT", f"/proposals/{pid}/draft", {"expected_rev": rev, "changes": draft})
    live["put_draft"] = st
    if st == 200:
        st, body = api.call("POST", f"/proposals/{pid}/validate")
        live["validate"] = {"http": st, "valid": body.get("valid"), "violations": body.get("violations") or None}
        if st == 200 and body.get("valid", True):
            st, body = api.call("POST", f"/proposals/{pid}/freeze")
            live["freeze"] = st
            if st == 200:
                st, body = api.call("POST", f"/proposals/{pid}/evaluate",
                                    {"suite_id": suite["id"], "suite_version": suite["version"]})
                live["evaluate_http"] = st
                rep = body.get("payload") if st == 409 and "payload" in body else body
                live["verdict"] = (rep or {}).get("verdict") or body.get("code")
                items = (rep or {}).get("items") or []
                live["gates"] = [{"metric": i["metric_id"], "phase": i["phase"], "passed": i["passed"],
                                  "value": i.get("value"), "reason": i.get("reason") or None} for i in items]
                live["scenarios_failed"] = [i["metric_id"] for i in items if not i["passed"]]
                fails: dict = {}
                for r in (rep or {}).get("results") or []:
                    sc = r.get("score") or {}
                    if not sc.get("passed", True):
                        fails.setdefault(r["scenario_id"], sc.get("failures") or [])
                live["failure_reasons"] = fails  # first failing repetition per scenario
                live["detail"] = (rep or {}).get("detail")
                live["problem"] = None if st in (200, 409) else {k: body.get(k) for k in ("code", "detail", "violations")}
            else:
                live["problem"] = {k: body.get(k) for k in ("code", "detail", "violations")}
        else:
            live["problem"] = {k: body.get(k) for k in ("code", "detail", "violations")}
    else:
        live["problem"] = {k: body.get(k) for k in ("code", "detail", "violations")}
    live["never_called"] = ["approve", "publish", "promote"]
    report["live"] = live
    print(json.dumps(report, indent=2, ensure_ascii=False, default=str))
    return 0 if live.get("verdict") == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
