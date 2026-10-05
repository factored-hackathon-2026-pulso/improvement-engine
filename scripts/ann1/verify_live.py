"""ANN1 live harness: reads the platform as the demo supervisors/analyst and prints counts and opaque ids only.
    python verify_live.py <port> <backend_dir>
"""
from __future__ import annotations

import json
import sys
import urllib.request

port, backend = int(sys.argv[1]), sys.argv[2]
sys.path.insert(0, backend)
from tests.support import ANALYST, DEV_MFA_CODE, PASSWORD, SECOND_SUPERVISOR, SUPERVISOR  # noqa: E402

BASE = f"http://127.0.0.1:{port}/api/v1"


def call(method: str, path: str, body=None, token: str | None = None):
    req = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def sign_in(email: str) -> str:
    login = call("POST", "/auth/login", {"email": email, "password": PASSWORD})
    return call("POST", "/auth/mfa", {"challengeId": login["challengeId"], "code": DEV_MFA_CODE})["token"]


for label, who in (("supervisor 1", SUPERVISOR), ("supervisor 2", SECOND_SUPERVISOR), ("analyst", ANALYST)):
    t = sign_in(who.email)
    items = [n for n in call("GET", "/me/notifications", token=t)["items"] if n["kind"] == "improvement_proposed"]
    print(f"{label}: improvement_proposed notifications = {len(items)}", [n["improvement"]["proposalId"] for n in items])
t = sign_in(SUPERVISOR.email)
print("builder proposals:", [(p["proposalId"], p["source"], p["origin"], p["state"]) for p in call("GET", "/builder/proposals", token=t)["items"]])
