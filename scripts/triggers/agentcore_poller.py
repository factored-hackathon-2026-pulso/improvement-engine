"""Trigger service prototype (TR1): agent-core pull endpoints -> engine automation requests.

Pull-only. Polls agent-core `GET /v1/export/runs`, `/v1/export/runs/{id}/events` and `/v1/export/registry-events`
(role `exporter`) with a persisted cursor, turns `run.closed` and `release.published|promoted|revoked` into
`pulso.trigger.v1` requests, and sends them through one sink. `explicit` and `scheduled` triggers use the same
request builder and sink. Every request has `trigger_key = sha256(scope + kind + event ref)`, sent as the
`Idempotency-Key` header, so replays, overlap re-reads and retries cannot create a second run.

Guarantees: at-least-once delivery (the cursor is saved only after the sink accepted the whole page; keys already
sent are skipped on replay). Only loopback hosts are contacted. Tokens are read from the environment or a local JSON
file, are never printed, logged or stored in the state file, and are redacted from error text.

Usage (values never printed):
  python agentcore_poller.py poll --once --state s.json --out triggers.jsonl --token-env AGENTCORE_EXPORT_TOKEN
  python agentcore_poller.py explicit KEY --engine-url http://127.0.0.1:8099
  python agentcore_poller.py scheduled --interval-secs 3600 --out triggers.jsonl
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from datetime import datetime, timezone
from pathlib import Path

SCHEMA = "pulso.trigger.v1"
RELEASE_TYPES = {"published": "release.published", "promoted": "release.promoted", "revoked": "release.revoked"}
SESSION_PATH = "/api/v1/auth/session"
TRIGGERS_PATH = "/internal/v1/automation/triggers"  # served by debug-api (TR2) and by `pulso run`
SEEN_CAP = 5000
LOOPBACK = {"127.0.0.1", "localhost", "::1"}


class SinkError(RuntimeError):
    pass


def require_loopback(url: str) -> str:
    u = urllib.parse.urlparse(url)
    if u.scheme not in ("http", "https") or (u.hostname or "") not in LOOPBACK:
        raise ValueError("only loopback hosts are allowed")
    return url


def redact(text: str, secrets: list[str]) -> str:
    for s in secrets:
        if s:
            text = text.replace(s, "[redacted]")
    return re.sub(r"(?i)(bearer\s+)\S+", r"\1[redacted]", text)


def trigger_key(*, tenant: str, mission: str, source: str, config_digest: str, kind: str, ref: str) -> str:
    raw = json.dumps([tenant, mission, source, config_digest, kind, ref], separators=(",", ":"), ensure_ascii=True)
    return "sha256:" + hashlib.sha256(raw.encode()).hexdigest()


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ---------------------------------------------------------------------------------------------- transport

class HttpFetch:
    """GET JSON from agent-core (loopback only). `token` is held in memory only."""

    def __init__(self, base_url: str, token: str):
        self.base, self.token = require_loopback(base_url).rstrip("/"), token

    def __call__(self, path: str, params: dict) -> dict:
        q = urllib.parse.urlencode(params)
        req = urllib.request.Request(f"{self.base}{path}?{q}", headers={"Authorization": f"Bearer {self.token}"})
        try:
            with urllib.request.urlopen(req, timeout=15) as r:  # noqa: S310 (loopback enforced)
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"agent-core HTTP {e.code} on {path}") from None
        except OSError as e:
            raise RuntimeError(redact(f"agent-core unreachable: {type(e).__name__}", [self.token])) from None


class JsonlSink:
    """Append-only file sink; the key is the dedupe identity, so a restart never repeats a line."""

    def __init__(self, path: Path):
        self.path = Path(path)

    def _keys(self) -> set[str]:
        if not self.path.exists():
            return set()
        return {json.loads(x)["trigger_key"] for x in self.path.read_text(encoding="utf-8").splitlines() if x.strip()}

    def send(self, req: dict) -> None:
        if req["trigger_key"] in self._keys():
            return
        with self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(req, sort_keys=True) + "\n")


class HttpSink:
    """POST to the engine debug-api; Idempotency-Key = trigger_key. 409 means already accepted.

    The engine requires `X-CSRF-Token`: it is read from `GET /api/v1/auth/session` (same bearer) once, kept in memory (never
    logged) and re-read once when the engine answers 403 (a restarted engine has a new token)."""

    def __init__(self, base_url: str, token: str | None = None):
        self.base = require_loopback(base_url).rstrip("/")
        self.url, self.token, self._csrf = self.base + TRIGGERS_PATH, token, None

    def _auth(self) -> dict:
        return {"Authorization": f"Bearer {self.token}"} if self.token else {}

    def _fetch_csrf(self) -> str:
        r = urllib.request.Request(self.base + SESSION_PATH, headers=self._auth(), method="GET")
        try:
            with urllib.request.urlopen(r, timeout=15) as resp:  # noqa: S310 (loopback enforced)
                tok = json.loads(resp.read()).get("csrf_token")
        except urllib.error.HTTPError as e:
            raise SinkError(f"engine session HTTP {e.code}") from None
        except (OSError, ValueError) as e:
            raise SinkError(redact(f"engine session unavailable: {type(e).__name__}", [self.token or ""])) from None
        if not isinstance(tok, str) or not tok:
            raise SinkError("engine session carries no csrf_token")
        return tok

    def _post(self, req: dict) -> None:
        h = {"Content-Type": "application/json", "Idempotency-Key": req["trigger_key"], "X-CSRF-Token": self._csrf, **self._auth()}
        r = urllib.request.Request(self.url, data=json.dumps(req).encode(), headers=h, method="POST")
        urllib.request.urlopen(r, timeout=15).close()  # noqa: S310 (loopback enforced)

    def send(self, req: dict) -> None:
        try:
            if self._csrf is None:
                self._csrf = self._fetch_csrf()
            try:
                self._post(req)
            except urllib.error.HTTPError as e:
                if e.code != 403:
                    raise
                self._csrf = self._fetch_csrf()  # the engine restarted: one refresh, one retry
                self._post(req)
        except urllib.error.HTTPError as e:
            if e.code != 409:
                raise SinkError(f"engine HTTP {e.code}") from None
        except OSError as e:
            raise SinkError(redact(f"engine unreachable: {type(e).__name__}", [self.token or ""])) from None


# ---------------------------------------------------------------------------------------------- poller

class Poller:
    def __init__(self, *, fetch: Callable[[str, dict], dict], sink, state_path: Path, tenant: str, mission: str,
                 source: str, config_digest: str, page_limit: int = 100, overlap: int = 0,
                 only_origin: str | None = None, only_agent: str | None = None,
                 clock: Callable[[], str] = _now_iso):
        self.fetch, self.sink, self.state_path = fetch, sink, Path(state_path)
        self.scope = dict(tenant=tenant, mission=mission, source=source, config_digest=config_digest)
        self.page_limit, self.overlap, self.only_origin, self.only_agent = page_limit, overlap, only_origin, only_agent
        self.clock = clock
        self.state = {"runs_cursor": 0, "registry_after": 0, "run_event_cursors": {}, "emitted": []}
        if self.state_path.exists():
            self.state.update(json.loads(self.state_path.read_text(encoding="utf-8")))
        self._seen = set(self.state["emitted"])

    # -- state
    def _save(self) -> None:
        self.state["emitted"] = self.state["emitted"][-SEEN_CAP:]
        tmp = self.state_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.state, sort_keys=True), encoding="utf-8")
        os.replace(tmp, self.state_path)

    # -- request path shared by every kind
    def _request(self, kind: str, ref: str, event: dict) -> dict:
        key = trigger_key(kind=kind, ref=ref, **self.scope)
        return {"schema": SCHEMA, "trigger_key": key, "kind": kind, **self.scope,
                "event": {**event, "ref": ref}, "requested_at": self.clock()}

    def _emit(self, req: dict) -> bool:
        key = req["trigger_key"]
        if key in self._seen:
            return False
        self.sink.send(req)
        self._seen.add(key)
        self.state["emitted"].append(key)
        return True

    def _pages(self, path: str, after: int):
        while True:
            page = self.fetch(path, {"after": after, "limit": self.page_limit})
            yield page["items"]
            nxt = page["next_after"]
            if nxt == after or not page["items"]:
                return
            after = nxt

    # -- outcome triggers
    def poll_once(self) -> int:
        sent = 0
        try:
            sent += self._poll_runs()
            sent += self._poll_registry()
        finally:
            self._save()  # keys already delivered survive a failure; cursors only move on success
        return sent

    def _poll_runs(self) -> int:
        start = self.state["runs_cursor"]
        sent, last = 0, start
        for items in self._pages("/v1/export/runs", max(0, start - self.overlap)):
            for r in items:
                last = max(last, r["cursor"])
                if r.get("closed_at") is None or (self.only_agent and r["agent"]["id"] != self.only_agent):
                    continue
                sent += self._run_closed(r)
        self.state["runs_cursor"] = last
        return sent

    def _run_closed(self, r: dict) -> int:
        rid = r["run_id"]
        seq0 = self.state["run_event_cursors"].get(rid, -1)
        ref, closed_by = f"run:{rid}", None
        for evs in self._pages(f"/v1/export/runs/{rid}/events", seq0):
            for e in evs:
                seq0 = max(seq0, e["seq"])
                if e.get("type") == "run_closed":
                    ref, closed_by = e["event_id"], (e.get("payload") or {}).get("closed_by")
        subject = {"run_id": rid, "agent": r["agent"]["id"], "release": r["release"], "outcome": r.get("outcome"),
                   "closed_by": closed_by}
        req = self._request("outcome", ref, {"type": "run.closed", "at": r["closed_at"], "subject": subject})
        self.state["run_event_cursors"][rid] = seq0
        return int(self._emit(req))

    def _poll_registry(self) -> int:
        start = self.state["registry_after"]
        pos, sent, last = max(0, start - self.overlap), 0, start
        for items in self._pages("/v1/export/registry-events", pos):
            for e in items:
                pos += 1
                last = max(last, pos)
                typ = RELEASE_TYPES.get(e["type"])
                if typ is None or (self.only_origin and e.get("origin") != self.only_origin):
                    continue
                subject = {k: e.get(k) for k in ("release_id", "proposal_id", "candidate_hash", "origin")}
                ref = f"{pos}:{e['type']}:{e.get('release_id')}"  # registry events carry no id: position is the identity
                sent += int(self._emit(self._request("outcome", ref, {"type": typ, "at": e["at"], "subject": subject})))
        self.state["registry_after"] = last
        return sent

    # -- explicit and scheduled: same request path
    def explicit(self, idempotency_key: str, reason: str = "") -> bool:
        try:
            return self._emit(self._request("explicit", idempotency_key, {"type": "run.now", "at": self.clock(),
                                                                           "subject": {"reason": reason}}))
        finally:
            self._save()

    def scheduled(self, now: float, interval_secs: int) -> bool:
        slot = int(now // interval_secs)
        try:
            return self._emit(self._request("scheduled", f"slot:{interval_secs}:{slot}",
                                            {"type": "schedule.tick", "at": self.clock(),
                                             "subject": {"interval_secs": interval_secs, "slot": slot}}))
        finally:
            self._save()


# ---------------------------------------------------------------------------------------------- CLI

def _token(args) -> str:
    if args.token_file:
        return json.loads(Path(args.token_file).read_text(encoding="utf-8"))[args.token_key]
    return os.environ.get(args.token_env, "")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["poll", "explicit", "scheduled"])
    ap.add_argument("key", nargs="?", help="idempotency key (explicit)")
    ap.add_argument("--core-url", default="http://127.0.0.1:8001")
    ap.add_argument("--engine-url", help="POST requests to this loopback engine debug-api")
    ap.add_argument("--out", help="append requests to this JSONL file instead")
    ap.add_argument("--state", default="triggers-state.json")
    ap.add_argument("--tenant", default="tenant-local")
    ap.add_argument("--mission", default="default")
    ap.add_argument("--source", default="agent-core")
    ap.add_argument("--config-digest", default="unset")
    ap.add_argument("--token-env", default="AGENTCORE_EXPORT_TOKEN")
    ap.add_argument("--token-file", help="JSON file holding the token (local dev stack); never printed")
    ap.add_argument("--token-key", default="admin")
    ap.add_argument("--only-origin")
    ap.add_argument("--only-agent")
    ap.add_argument("--overlap", type=int, default=0)
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--interval-secs", type=int, default=60)
    ap.add_argument("--reason", default="")
    a = ap.parse_args(argv)
    if bool(a.engine_url) == bool(a.out):
        print("error: pass exactly one of --engine-url or --out", file=sys.stderr)
        return 2
    sink = HttpSink(a.engine_url, os.environ.get("PULSO_ENGINE_TOKEN")) if a.engine_url else JsonlSink(Path(a.out))
    tok = _token(a) if a.cmd == "poll" else ""
    p = Poller(fetch=HttpFetch(a.core_url, tok) if a.cmd == "poll" else (lambda *_: {}), sink=sink,
               state_path=Path(a.state), tenant=a.tenant, mission=a.mission, source=a.source,
               config_digest=a.config_digest, overlap=a.overlap, only_origin=a.only_origin, only_agent=a.only_agent)
    try:
        if a.cmd == "explicit":
            print("sent" if p.explicit(a.key or "", a.reason) else "duplicate")
        elif a.cmd == "scheduled":
            print("sent" if p.scheduled(time.time(), a.interval_secs) else "duplicate")
        else:
            while True:
                print(f"emitted={p.poll_once()} runs_cursor={p.state['runs_cursor']} registry_after={p.state['registry_after']}")
                if a.once:
                    break
                time.sleep(a.interval_secs)
    except (RuntimeError, SinkError, ValueError) as e:
        print(f"error: {redact(str(e), [tok])}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
