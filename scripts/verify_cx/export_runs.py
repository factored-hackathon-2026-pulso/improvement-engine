"""Export ALL runs + per-run events from a LOCAL agent-core (`/v1/export/*`) into the envelope Codex's T3 CLI reads.

Paging follows the real contract: `next_after` is the last cursor (or the requested `after` when the page is empty);
it is NEVER null. A listing is complete when a page comes back EMPTY. `--null-terminal` rewrites the final cursor to null
(what T3's `next_after is None` rule expects) ONLY after an empty terminal page was actually observed.
Exporter token: agent-core TEST staff issuer (local keys only); never printed. Needs PYTHONPATH=<agent-core>.
"""
import argparse, json, sys, urllib.request


def get(base, tok, path):
    req = urllib.request.Request(base + path, headers={"Authorization": "Bearer " + tok})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def pages(base, tok, path, start, key):
    items, after = [], start
    while True:
        p = get(base, tok, f"{path}{'&' if '?' in path else '?'}after={after}&limit=500")
        if not p["items"]:
            return items, p["next_after"], True
        items += p["items"]
        after = p["next_after"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8004")
    ap.add_argument("--out", required=True)
    ap.add_argument("--null-terminal", action="store_true")
    ap.add_argument("--label", default="SYNTHETIC local agent-core vx stack; battery + eval runs")
    a = ap.parse_args()
    from agent_core.adapters.system_clock import SystemClock
    from testing.fakes.identity import TestStaffIssuer
    tok = TestStaffIssuer(SystemClock()).exporter_bot()
    runs, nxt, done = pages(a.base, tok, "/v1/export/runs", 0, "cursor")
    ev = {}
    for r in runs:
        its, enx, ok = pages(a.base, tok, f"/v1/export/runs/{r['run_id']}/events", -1, "seq")
        ev[r["run_id"]] = {"items": its, "next_after": None if (a.null_terminal and ok) else enx}
    env = {"_label": a.label, "runs": {"items": runs, "next_after": None if a.null_terminal and done else nxt}, "events": ev}
    open(a.out, "w", encoding="utf-8").write(json.dumps(env))
    print("runs", len(runs), "terminal_empty_page_seen", done)


main()
