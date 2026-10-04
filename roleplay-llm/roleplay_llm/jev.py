"""agent_roleplay Jev server: the upstream behind the llm-gateway `POST /v1/jev` pass-through.

The gateway forwards the Jev `POST /v1/systemone` body unchanged (`{"state": {"locale", "input"}, "model",
"questions": {id: question}}`) and returns the upstream answer (`{"model", "answers", "usage"}`). This server
answers on both `/v1/jev` (direct test) and `/v1/systemone` (what the gateway calls). Same protocol as the
chat-completions shim, in its own queue namespace `<queue>/jev/{requests,responses,faults,ledger.jsonl}`:

1. the treated-payload scanner (TPS, deny-by-default registry) runs BEFORE any queue write;
2. a replay key over the normalised request serves a recorded or late answer;
3. otherwise `requests/<key>.json` is written, the connection held up to `hold_s`, then a typed 504
   `responder_timeout` (the late answer serves the retry).

Every answer is labelled `agent_roleplay` with `quality_claims: forbidden`. Plumbing only: probabilities are
whatever the responder wrote, never a quality statement. Faults come only from `faults/next.json` or
`faults/<key>.json`.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import re
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from .scanner import DEFAULT_K, Registry, scan_payload
from .shim import HOLD_S, PROTOCOL, PROVENANCE, _canon, _diff, _error, _Ordinals, _norm

ROLE = "jev"
PATHS = ("/v1/jev", "/v1/systemone")
_REQ_KEYS = {"state", "model", "questions"}
_STATE_KEYS = {"locale", "input"}
_LOCALE = re.compile(r"^[a-z]{2}(-[A-Za-z]{2})?$")
_MODEL = re.compile(r"^[A-Za-z0-9._:/@-]{1,128}$")
_QID = re.compile(r"^[A-Za-z0-9_.-]{1,96}$")
_QTYPES = {"choice", "noul", "score"}
_RESPONSE_KEYS = {"protocol", "key", "provenance", "quality_claims", "responder", "content"}
_CONTENT_KEYS = {"answers", "usage"}
_RESPONDER_ID = re.compile(r"^[A-Za-z0-9._-]{1,64}$")


def _strings(x: Any, ords: _Ordinals) -> Any:
    """uuid/binding ids to ordinals in every string; no key is masked (questions carry no volatile fields)."""
    if isinstance(x, dict):
        return {k: _strings(v, ords) for k, v in sorted(x.items())}
    if isinstance(x, list):
        return [_strings(v, ords) for v in x]
    return ords.sub(x) if isinstance(x, str) else x


def normalise_request(request: dict[str, Any]) -> dict[str, Any]:
    """Volatile fields are masked only inside state.input; model and questions keep every key and value."""
    ords = _Ordinals()
    state = request["state"]
    return {"state": {"locale": state["locale"], "input": _norm(state["input"], ords)},
            "model": request["model"], "questions": _strings(request["questions"], ords)}


def jev_replay_key(request: dict[str, Any]) -> str:
    """Hash of the normalised request: ids in state.input replaced by first-seen ordinals."""
    return hashlib.sha256(_canon(normalise_request(request)).encode()).hexdigest()[:32]


def _stage(request: dict[str, Any]) -> str:
    n = normalise_request(request)
    return hashlib.sha256(_canon({"model": n["model"], "questions": n["questions"]}).encode()).hexdigest()[:16]


def _levels(q: dict[str, Any]) -> int:
    c = q.get("criteria")
    return len(c) if isinstance(c, (list, dict)) else 100


def _is_num(x: Any) -> bool:
    return isinstance(x, (int, float)) and not isinstance(x, bool) and 0 <= x <= 1


def _is_count(x: Any) -> bool:
    return isinstance(x, int) and not isinstance(x, bool) and 0 <= x <= 10**9


class JevShim:
    def __init__(self, queue_dir: Path | str, *, hold_s: float = HOLD_S, poll_s: float = 0.25,
                 k: int = DEFAULT_K, replay_only: bool = False, registry: Registry | None = None) -> None:
        self.registry = registry or Registry()
        self.queue = Path(queue_dir) / "jev"
        self.hold_s, self.poll_s, self.k, self.replay_only = hold_s, poll_s, k, replay_only
        for sub in ("requests", "responses", "faults"):
            (self.queue / sub).mkdir(parents=True, exist_ok=True)

    # -- public ---------------------------------------------------------------------------------------
    def handle(self, raw: bytes) -> tuple[int, dict[str, Any]]:
        try:
            req = json.loads(raw)
        except (ValueError, RecursionError):
            return _error(400, "bad_request", "body is not JSON")
        if (not isinstance(req, dict) or not isinstance(req.get("model"), str)
                or not isinstance(req.get("state"), dict) or not isinstance(req.get("questions"), dict)
                or not req["questions"]):
            return _error(400, "bad_request", "expected state, model and a non-empty questions object")
        violations = self._scan(req)
        if violations:
            self._ledger("scanner_rejected", violations=violations)
            return _error(422, "treated_payload_rejected", "payload failed the treated-payload scanner",
                          violations=violations)
        key = jev_replay_key(req)
        fault = self._fault(key)
        if fault:
            self._ledger("fault", key=key, fault_type=fault["type"])
            return _error(fault["status"], fault["type"], "scripted fault")
        answer = self._answer(key, req)
        if answer is None and self.replay_only:
            return self._miss(req, key)
        if answer is None:
            self._enqueue(key, req)
            answer = self._hold(key, req)
        if answer is None:
            self._ledger("responder_timeout", key=key)
            return _error(504, "responder_timeout", "no responder answer within the hold time", key=key)
        if "error" in answer:
            return 502, answer
        return 200, {"model": req["model"], "answers": answer["answers"], "usage": answer["usage"],
                     "x_roleplay": {"provenance": PROVENANCE, "quality_claims": "forbidden", "key": key,
                                    "responder": answer["responder"]}}

    # -- scanner --------------------------------------------------------------------------------------
    def _scan(self, req: dict[str, Any]) -> list[str]:
        v = [f"request.{k}: key not allowed" for k in sorted(set(req) - _REQ_KEYS)]
        state = req["state"]
        v += [f"state.{k}: key not allowed" for k in sorted(set(state) - _STATE_KEYS)]
        if not isinstance(state.get("locale"), str) or not _LOCALE.fullmatch(state["locale"]):
            v.append("state.locale: not a locale code")
        if not isinstance(state.get("input"), dict):
            v.append("state.input: must be an object")
        if not _MODEL.fullmatch(req["model"]):
            v.append("model: not an identifier")
        for qid, q in req["questions"].items():
            if not _QID.fullmatch(str(qid)):
                v.append(f"questions.<id>: not an identifier: {str(qid)[:32]!r}")
            elif not isinstance(q, dict) or not isinstance(q.get("type"), str) or q["type"] not in _QTYPES:
                v.append(f"questions.{qid}.type: must be one of {sorted(_QTYPES)}")
        scan = scan_payload({"goal": "jev", "step": 0, "observations": [], "inputs": state["input"] if isinstance(state.get("input"), dict) else {},
                             "output_schema": req["questions"]}, k=self.k, registry=self.registry)
        return v + list(scan.violations)

    # -- internals ------------------------------------------------------------------------------------
    def _ledger(self, event: str, **fields: Any) -> None:
        with (self.queue / "ledger.jsonl").open("a", encoding="utf-8") as f:
            f.write(_canon({"event": event, "provenance": PROVENANCE, "surface": "jev", **fields}) + "\n")

    def _fault(self, key: str) -> dict[str, Any] | None:
        for name in (f"{key}.json", "next.json"):
            path = self.queue / "faults" / name
            if path.exists():
                try:
                    fault = json.loads(path.read_text())
                    path.unlink()
                    status = int(fault["status"])
                    return {"status": status, "type": str(fault["type"])[:64]} if 400 <= status <= 599 else None
                except (ValueError, KeyError, OSError, TypeError):
                    return None
        return None

    def _enqueue(self, key: str, req: dict[str, Any]) -> None:
        path = self.queue / "requests" / f"{key}.json"
        if path.exists():
            return
        doc = {"protocol": PROTOCOL, "key": key, "provenance": PROVENANCE, "scanner_id": "tps-1",
               "surface": "jev", "stage": _stage(req), "request": normalise_request(req),
               "inputs": normalise_request(req)["state"]["input"], "respond_to": f"responses/{key}.json",
               "answer_shape": {"answers": {"<question id>": {
                   "choice": {"type": "choice", "choice": "<option>", "probabilities": {"<option>": 0.5}},
                   "noul": {"type": "noul", "noul": 0.5}, "score": {"type": "score", "probabilities": {"0": 0.5}}}},
                   "usage": {"input_tokens": 0, "output_tokens": 0}},
               "rules": "Read only this file. Write only the respond_to file as content {answers, usage}. "
                        "One answer per question id, probabilities in [0,1]. No quality claims."}
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(doc, indent=1), encoding="utf-8")
        tmp.replace(path)
        self._ledger("queued", key=key, stage=doc["stage"])

    def _hold(self, key: str, req: dict[str, Any]) -> dict[str, Any] | None:
        deadline = time.monotonic() + self.hold_s
        while True:
            answer = self._answer(key, req)
            if answer is not None:
                return answer
            if time.monotonic() >= deadline:
                return None
            time.sleep(min(self.poll_s, max(0.0, deadline - time.monotonic())))

    def _answer(self, key: str, req: dict[str, Any]) -> dict[str, Any] | None:
        path = self.queue / "responses" / f"{key}.json"
        if not path.exists():
            return None
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
            problem = self._validate(doc, key, req["questions"])
        except (ValueError, OSError, TypeError, KeyError):
            problem = "response is not valid JSON"
        if problem:
            path.replace(path.with_name(f"{key}.rejected.json"))
            self._ledger("response_rejected", key=key, reason=problem)
            return {"error": {"type": "invalid_output", "message": problem}}
        self._ledger("answered", key=key)
        return {"answers": doc["content"]["answers"], "usage": doc["content"]["usage"],
                "responder": doc["responder"]}

    @classmethod
    def _validate(cls, doc: Any, key: str, questions: dict[str, Any]) -> str | None:
        if not isinstance(doc, dict):
            return "response is not an object"
        if doc.get("protocol") != PROTOCOL or doc.get("key") != key:
            return "protocol or key mismatch"
        if doc.get("provenance") != PROVENANCE:
            return "response must be labelled agent_roleplay"
        if doc.get("quality_claims") != "forbidden":
            return "quality_claims must be 'forbidden'"
        if set(doc) - _RESPONSE_KEYS:
            return f"unexpected fields: {sorted(set(doc) - _RESPONSE_KEYS)}"
        r = doc.get("responder")
        if (not isinstance(r, dict) or set(r) != {"id", "role"} or not isinstance(r["id"], str)
                or not _RESPONDER_ID.fullmatch(r["id"]) or r["role"] != ROLE):
            return "responder must be {id: [A-Za-z0-9._-]{1,64}, role: jev}"
        c = doc.get("content")
        if not isinstance(c, dict) or set(c) != _CONTENT_KEYS:
            return "content must be exactly {answers, usage}"
        u = c["usage"]
        if not isinstance(u, dict) or set(u) != {"input_tokens", "output_tokens"} \
                or not all(_is_count(u[k]) for k in u):
            return "usage must be {input_tokens, output_tokens} non-negative integers"
        a = c["answers"]
        if not isinstance(a, dict) or set(a) != set(questions):
            return "answers must cover exactly the requested question ids"
        for qid, q in questions.items():
            problem = cls._check_answer(qid, q, a[qid])
            if problem:
                return problem
        return None

    @staticmethod
    def _check_answer(qid: str, q: dict[str, Any], ans: Any) -> str | None:
        if not isinstance(ans, dict) or ans.get("type") != q["type"]:
            return f"{qid}: answer missing or of another type"
        if q["type"] == "noul":
            return None if set(ans) == {"type", "noul"} and _is_num(ans["noul"]) else f"{qid}: bad noul answer"
        probs = ans.get("probabilities")
        if not isinstance(probs, dict) or not probs or not all(_is_num(p) for p in probs.values()):
            return f"{qid}: probabilities must be numbers in [0,1]"
        if q["type"] == "choice":
            options = q.get("criteria")
            options = set(options) if isinstance(options, dict) else set()
            if set(ans) != {"type", "choice", "probabilities"} or not isinstance(ans["choice"], str) or ans["choice"] not in options \
                    or not set(probs) <= options:
                return f"{qid}: choice or probabilities outside the requested options"
        elif set(ans) != {"type", "probabilities"} or not all(
                re.fullmatch(r"\d{1,2}", k) and int(k) < _levels(q) for k in probs):
            return f"{qid}: bad score answer"
        return None

    def _miss(self, req: dict[str, Any], key: str) -> tuple[int, dict[str, Any]]:
        norm, stage = normalise_request(req), _stage(req)
        diff = ["no recorded request for this stage"]
        for path in sorted((self.queue / "requests").glob("*.json")):
            try:
                rec = json.loads(path.read_text(encoding="utf-8"))
            except ValueError:
                continue
            if rec.get("stage") == stage:
                diff = _diff(rec["request"], norm) or ["request equal"]
                break
        self._ledger("replay_miss", key=key, stage=stage)
        return _error(409, "replay_miss", "no recorded answer for this normalised key", key=key, diff=diff)


def serve_jev(shim: JevShim, *, host: str = "127.0.0.1", port: int = 8641,
              api_key: str = "dummy") -> ThreadingHTTPServer:
    class Handler(BaseHTTPRequestHandler):
        timeout = 30  # a client that stalls mid-body must not pin a thread forever

        def _send(self, status: int, body: dict[str, Any]) -> None:
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self) -> None:  # noqa: N802
            self._send(200, {"ok": True, "provenance": PROVENANCE}) if self.path == "/healthz" \
                else self._send(404, {"error": {"type": "not_found"}})

        def do_POST(self) -> None:  # noqa: N802
            if self.path not in PATHS:
                return self._send(404, {"error": {"type": "not_found"}})
            auth = self.headers.get("Authorization", "")
            if not hmac.compare_digest(auth.encode("utf-8", "replace"), f"Bearer {api_key}".encode()):
                return self._send(401, {"error": {"type": "unauthorized"}})
            try:
                length = int(self.headers.get("Content-Length") or 0)
            except ValueError:
                length = -1
            if length < 0:
                return self._send(400, {"error": {"type": "bad_request"}})
            if length > 1 << 20:
                return self._send(413, {"error": {"type": "payload_too_large"}})
            status, body = shim.handle(self.rfile.read(length))
            self._send(status, body)

        def log_message(self, *_: Any) -> None:  # request bodies and headers are never logged
            return

    server = ThreadingHTTPServer((host, port), Handler)
    server.daemon_threads = True
    return server
