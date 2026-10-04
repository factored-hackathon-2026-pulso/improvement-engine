"""agent_roleplay shim: an OpenAI chat-completions compatible upstream for the real llm-gateway.

The gateway posts `POST /v1/chat/completions` with messages [system, user]; the user message is the
RFC 8785 canonical JSON of the agent input dict. This shim:

1. parses the user message and runs the treated-payload scanner BEFORE anything is written to the queue;
2. derives a replay key from (system prompt, inputs with volatile fields normalised);
3. serves a recorded or late answer from `responses/<key>.json` if present;
4. otherwise writes `requests/<key>.json` (the only file a responder may read), holds the connection up to
   `hold_s` (55 s by default) for `responses/<key>.json`, and else returns a typed 504 `responder_timeout`;
   the late answer then serves the retry.

Answers are tool use as JSON in the message content (`{"kind":"tool_call"|"final", ...}`), never OpenAI
`tool_calls`. Every answer is labelled `agent_roleplay` and `quality_claims` is forbidden.
Faults come only from the scripted side channel `faults/next.json` or `faults/<key>.json`.
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

from .scanner import DEFAULT_K, SCANNER_ID, Registry, scan_payload, static_text_violations

PROTOCOL = "roleplay-queue/1"
PROVENANCE = "agent_roleplay"
HOLD_S = 55.0
VOLATILE_KEYS = {"run_id", "turn_id", "session_id", "labels"}
_UUID = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
_PREFIXED = re.compile(r"\b(binding|job|artifact)[-_:][0-9A-Za-z][0-9A-Za-z-]{5,}")
_ROLES = {"scout", "verifier", "builder", "builder_design"}
_FORBIDDEN_RESPONSE_KEYS = ("quality", "score", "confidence", "rating")
_RESPONSE_KEYS = {"protocol", "key", "provenance", "quality_claims", "responder", "content"}


def _canon(x: Any) -> str:
    return json.dumps(x, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


class _Ordinals:
    def __init__(self) -> None:
        self.seen: dict[str, str] = {}

    def sub(self, text: str) -> str:
        def rep(kind: str):
            def f(m: re.Match) -> str:
                raw = m.group(0)
                return self.seen.setdefault(raw, f"<{kind(m)}#{len(self.seen) + 1}>")
            return f
        text = _UUID.sub(rep(lambda m: "id"), text)
        return _PREFIXED.sub(rep(lambda m: m.group(1)), text)


def _norm(x: Any, ords: _Ordinals) -> Any:
    if isinstance(x, dict):
        return {k: ("<volatile>" if k in VOLATILE_KEYS else _norm(v, ords)) for k, v in sorted(x.items())}
    if isinstance(x, list):
        return [_norm(v, ords) for v in x]
    if isinstance(x, str):
        return ords.sub(x)
    return x


def normalise_inputs(inputs: dict[str, Any]) -> dict[str, Any]:
    """Volatile fields replaced by a marker; uuids and binding/job/artifact ids by first-seen ordinals."""
    return _norm(inputs, _Ordinals())


def stage_id(system: str) -> str:
    return hashlib.sha256(_Ordinals().sub(system).encode()).hexdigest()[:16]


def replay_key(system: str, inputs: dict[str, Any]) -> str:
    material = _canon({"stage": stage_id(system), "inputs": normalise_inputs(inputs)})
    return hashlib.sha256(material.encode()).hexdigest()[:32]


def _diff(a: Any, b: Any, path: str = "") -> list[str]:
    if isinstance(a, dict) and isinstance(b, dict):
        out: list[str] = []
        for k in sorted(set(a) | set(b)):
            out += _diff(a.get(k, "<absent>"), b.get(k, "<absent>"), f"{path}.{k}" if path else k)
        return out
    if isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in _diff(x, y, f"{path}[{i}]")]
    return [] if a == b else [f"{path}: recorded={_canon(a)[:200]} got={_canon(b)[:200]}"]


def _error(status: int, type_: str, message: str, **extra: Any) -> tuple[int, dict[str, Any]]:
    return status, {"error": {"type": type_, "message": message, **extra}}


class Shim:
    def __init__(self, queue_dir: Path | str, *, hold_s: float = HOLD_S, poll_s: float = 0.25,
                 k: int = DEFAULT_K, replay_only: bool = False, registry: Registry | None = None) -> None:
        self.registry = registry or Registry()
        self.queue = Path(queue_dir)
        self.hold_s, self.poll_s, self.k, self.replay_only = hold_s, poll_s, k, replay_only
        for sub in ("requests", "responses", "faults"):
            (self.queue / sub).mkdir(parents=True, exist_ok=True)

    # -- public ---------------------------------------------------------------------------------------
    def handle(self, raw: bytes) -> tuple[int, dict[str, Any]]:
        parsed = self._parse(raw)
        if isinstance(parsed[0], int):
            return parsed  # type: ignore[return-value]
        model, system, inputs = parsed
        scan = scan_payload(inputs, k=self.k, registry=self.registry)
        sys_v = static_text_violations(system, "system", 20000)
        if sys_v:
            scan = type(scan)(False, tuple(scan.violations) + tuple(sys_v), scan.scanner_id)
        if not scan.ok:
            self._ledger("scanner_rejected", scanner_id=scan.scanner_id, violations=list(scan.violations))
            return _error(422, "treated_payload_rejected", "payload failed the treated-payload scanner",
                          violations=list(scan.violations))
        key = replay_key(system, inputs)
        fault = self._fault(key)
        if fault:
            self._ledger("fault", key=key, fault_type=fault["type"])
            return _error(fault["status"], fault["type"], "scripted fault")
        answer = self._answer(key)
        if answer is None and self.replay_only:
            return self._miss(system, inputs, key)
        if answer is None:
            self._enqueue(key, system, inputs, scan.scanner_id)
            answer = self._hold(key)
        if answer is None:
            self._ledger("responder_timeout", key=key)
            return _error(504, "responder_timeout", "no responder answer within the hold time", key=key)
        if "error" in answer:
            return 502, answer
        return 200, self._completion(model, system, inputs, answer, key)

    # -- internals ------------------------------------------------------------------------------------
    def _parse(self, raw: bytes):
        try:
            req = json.loads(raw)
        except (ValueError, RecursionError):
            return _error(400, "bad_request", "body is not JSON")
        msgs = req.get("messages") if isinstance(req, dict) else None
        if (not isinstance(req, dict) or not isinstance(req.get("model"), str) or not isinstance(msgs, list)
                or req.get("stream")):
            return _error(400, "bad_request", "expected model and messages, no stream")
        by_role = {m.get("role"): m.get("content") for m in msgs if isinstance(m, dict)}
        system, user = by_role.get("system"), by_role.get("user")
        if not isinstance(system, str) or not isinstance(user, str):
            return _error(400, "bad_request", "expected system and user messages")
        try:
            inputs = json.loads(user)
        except (ValueError, RecursionError):
            self._ledger("scanner_rejected", scanner_id="tps-1", violations=["user message is not JSON"])
            return _error(422, "treated_payload_rejected", "user message is not canonical JSON")
        return req["model"], system, inputs

    def _ledger(self, event: str, **fields: Any) -> None:
        with (self.queue / "ledger.jsonl").open("a", encoding="utf-8") as f:
            f.write(_canon({"event": event, "provenance": PROVENANCE, **fields}) + "\n")

    def _fault(self, key: str) -> dict[str, Any] | None:
        for name in (f"{key}.json", "next.json"):
            path = self.queue / "faults" / name
            if path.exists():
                try:
                    fault = json.loads(path.read_text())
                    path.unlink()
                    status = int(fault["status"])
                    if not 400 <= status <= 599:
                        return None
                    return {"status": status, "type": str(fault["type"])[:64]}
                except (ValueError, KeyError, OSError, TypeError):
                    return None
        return None

    def _enqueue(self, key: str, system: str, inputs: dict[str, Any], scanner_id: str) -> None:
        path = self.queue / "requests" / f"{key}.json"
        if path.exists():
            return
        doc = {"protocol": PROTOCOL, "key": key, "provenance": PROVENANCE, "scanner_id": scanner_id,
               "stage": stage_id(system), "step": inputs.get("step"), "system_prompt": _Ordinals().sub(system),
               "inputs": normalise_inputs(inputs), "respond_to": f"responses/{key}.json",
               "answer_shape": {"kind": "tool_call|final", "tool": "exact tool ref", "args": {}, "output": {}},
               "rules": "Read only this file. Write only the respond_to file. No quality claims."}
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(doc, indent=1), encoding="utf-8")
        tmp.replace(path)
        self._ledger("queued", key=key, stage=doc["stage"], step=doc["step"])

    def _hold(self, key: str) -> dict[str, Any] | None:
        deadline = time.monotonic() + self.hold_s
        while True:
            answer = self._answer(key)
            if answer is not None:
                return answer
            if time.monotonic() >= deadline:
                return None
            time.sleep(min(self.poll_s, max(0.0, deadline - time.monotonic())))

    def _answer(self, key: str) -> dict[str, Any] | None:
        path = self.queue / "responses" / f"{key}.json"
        if not path.exists():
            return None
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
            problem = self._validate(doc, key)
        except (ValueError, OSError):
            problem = "response is not valid JSON"
        if problem:
            path.replace(path.with_name(f"{key}.rejected.json"))
            self._ledger("response_rejected", key=key, reason=problem)
            return {"error": {"type": "invalid_output", "message": problem}}
        self._ledger("answered", key=key)
        return {"content": doc["content"], "responder": doc.get("responder")}

    @staticmethod
    def _validate(doc: Any, key: str) -> str | None:
        if not isinstance(doc, dict):
            return "response is not an object"
        if doc.get("protocol") != PROTOCOL or doc.get("key") != key:
            return "protocol or key mismatch"
        if doc.get("provenance") != PROVENANCE:
            return "response must be labelled agent_roleplay"
        if doc.get("quality_claims") != "forbidden":
            return "quality_claims must be 'forbidden'"
        extra = set(doc) - _RESPONSE_KEYS
        if extra or any(w in k.lower() for k in doc for w in _FORBIDDEN_RESPONSE_KEYS if k != "quality_claims"):
            return f"unexpected or quality-claiming fields: {sorted(extra)}"
        r = doc.get("responder")
        if (not isinstance(r, dict) or set(r) != {"id", "role"} or not isinstance(r["id"], str)
                or not re.fullmatch(r"[A-Za-z0-9._-]{1,64}", r["id"]) or r["role"] not in _ROLES):
            return "responder must be {id: [A-Za-z0-9._-]{1,64}, role: scout|verifier|builder}"
        c = doc.get("content")
        if not isinstance(c, dict):
            return "content is not an object"
        allowed = {"kind", "output"} if c.get("kind") == "final" else {"kind", "tool", "args"}
        if set(c) - allowed:
            return f"unexpected content fields: {sorted(set(c) - allowed)}"
        if c.get("kind") == "final":
            return None if "output" in c else "final step needs output"
        if c.get("kind") == "tool_call":
            return None if isinstance(c.get("tool"), str) and isinstance(c.get("args", {}), dict) \
                else "tool_call needs tool and args"
        return "content.kind must be tool_call or final"

    def _miss(self, system: str, inputs: dict[str, Any], key: str) -> tuple[int, dict[str, Any]]:
        norm, stage = normalise_inputs(inputs), stage_id(system)
        diff: list[str] = ["no recorded request for this stage"]
        for path in sorted((self.queue / "requests").glob("*.json")):
            try:
                rec = json.loads(path.read_text(encoding="utf-8"))
            except ValueError:
                continue
            if rec.get("stage") == stage and rec.get("step") == inputs.get("step"):
                diff = _diff(rec["inputs"], norm) or ["inputs equal; system prompt differs"]
                break
        self._ledger("replay_miss", key=key, stage=stage)
        return _error(409, "replay_miss", "no recorded answer for this normalised key", key=key, diff=diff)

    def _completion(self, model: str, system: str, inputs: dict[str, Any], answer: dict[str, Any],
                    key: str) -> dict[str, Any]:
        text = _canon(answer["content"])
        tin, tout = (len(system) + len(_canon(inputs)) + 3) // 4, (len(text) + 3) // 4
        return {"id": f"chatcmpl-rp-{key[:12]}", "object": "chat.completion", "created": int(time.time()),
                "model": model,
                "choices": [{"index": 0, "finish_reason": "stop",
                             "message": {"role": "assistant", "content": text}}],
                "usage": {"prompt_tokens": tin, "completion_tokens": tout, "total_tokens": tin + tout},
                "usage_estimated": True,
                "x_roleplay": {"provenance": PROVENANCE, "quality_claims": "forbidden", "key": key,
                               "responder": answer.get("responder")}}


def serve(shim: Shim, *, host: str = "127.0.0.1", port: int = 8640, api_key: str = "dummy") -> ThreadingHTTPServer:
    class Handler(BaseHTTPRequestHandler):
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
            if self.path != "/v1/chat/completions":
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
