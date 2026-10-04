"""Treated-payload scanner (TPS): deny-by-default allow-list over the fixed LLMAgentPort input dict.

Top-level keys are exactly those `LLMAgentPort.step` builds. Static text (goal, tool catalogue, schemas)
is bounded and PII-pattern checked. Everything that can carry data (inputs, observations) must be opaque
ids, enums, booleans, integers, or k-anonymous aggregate rows; any other string is rejected.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any

SCANNER_ID = "tps-1"
DEFAULT_K = 10
TOP_KEYS = {"goal", "inputs", "step", "tools", "observations", "feedback", "output_schema"}
OBS_KEYS = {"tool", "args", "status", "result", "error"}
TOOL_KEYS = {"tool", "description", "args_schema"}
ROW_KEYS = {"metric_id", "window_id", "count", "rate", "evidence_ref"}
SUPPRESSED = "<k"

_OPAQUE = re.compile(r"^[A-Za-z0-9_.:/-]{1,128}$")
_TOOLREF = re.compile(r"^[a-z0-9_.-]+/[a-z0-9_.-]+@\d+\.\d+\.\d+$")
_ENUM = re.compile(r"^[a-z0-9][a-z0-9_.-]{0,63}$")
_HASH = re.compile(r"^[0-9a-f]{16}$")
_EVREF = re.compile(r"^ev_[0-9a-z]{8,64}$")
_EMAIL = re.compile(r"[^\s@]+@[^\s@]+\.[^\s@]+")
_LONG_DIGITS = re.compile(r"\d[\d\s().-]{6,}\d")
_MAX_STATIC = 2000
_MAX_FEEDBACK = 500


@dataclass(frozen=True)
class ScanResult:
    ok: bool
    violations: tuple[str, ...]
    scanner_id: str = SCANNER_ID


def suppress_rows(rows: list[dict[str, Any]], k: int = DEFAULT_K) -> list[dict[str, Any]]:
    """Below-k rows keep only their enums and a `<k` count; rate, hashed keys and evidence ref are dropped."""
    out = []
    for r in rows:
        if _is_int(r.get("count")) and r["count"] < k:
            kept = {key: r[key] for key in ("metric_id", "window_id") if key in r}
            out.append({**kept, "count": SUPPRESSED})
        else:
            out.append(dict(r))
    return out


def scan_payload(payload: Any, *, k: int = DEFAULT_K) -> ScanResult:
    v: list[str] = []
    if not isinstance(payload, dict):
        return ScanResult(False, ("payload: not an object",))
    for key in sorted(set(payload) - TOP_KEYS):
        v.append(f"payload.{key}: key not allowed")
    for key in ("goal", "step", "observations"):
        if key not in payload:
            v.append(f"payload.{key}: missing")
    if "goal" in payload:
        _static_text(payload["goal"], "goal", v)
    if "step" in payload and (not _is_int(payload["step"]) or payload["step"] < 0):
        v.append("step: not a non-negative integer")
    if "inputs" in payload:
        _opaque_tree(payload["inputs"], "inputs", v)
    if "feedback" in payload and payload["feedback"] is not None:
        _static_text(payload["feedback"], "feedback", v, _MAX_FEEDBACK)
    if "output_schema" in payload:
        _static_tree(payload["output_schema"], "output_schema", v)
    _tools(payload.get("tools", []), v)
    _observations(payload.get("observations", []), k, v)
    return ScanResult(not v, tuple(v))


def _is_int(x: Any) -> bool:
    return isinstance(x, int) and not isinstance(x, bool)


def _static_text(x: Any, path: str, v: list[str], limit: int = _MAX_STATIC) -> None:
    if not isinstance(x, str):
        v.append(f"{path}: not a string")
    elif len(x) > limit:
        v.append(f"{path}: longer than {limit}")
    elif _EMAIL.search(x) or _LONG_DIGITS.search(x):
        v.append(f"{path}: contains a PII-like pattern")


def _static_tree(x: Any, path: str, v: list[str], depth: int = 0) -> None:
    if depth > 12:
        v.append(f"{path}: too deep")
    elif isinstance(x, dict):
        for key, val in x.items():
            _static_text(key, f"{path}.<key>", v, 128)
            _static_tree(val, f"{path}.{key}", v, depth + 1)
    elif isinstance(x, list):
        for i, val in enumerate(x):
            _static_tree(val, f"{path}[{i}]", v, depth + 1)
    elif isinstance(x, str):
        _static_text(x, path, v)
    elif not (x is None or isinstance(x, (bool, int, float))):
        v.append(f"{path}: unsupported type")


def _opaque_str(x: str, path: str, v: list[str]) -> None:
    if not _OPAQUE.match(x) or _EMAIL.search(x) or re.fullmatch(r"\+?[\d().-]{7,}", x):
        v.append(f"{path}: string is not an opaque id or enum")


def _opaque_tree(x: Any, path: str, v: list[str], depth: int = 0) -> None:
    if depth > 8:
        v.append(f"{path}: too deep")
    elif isinstance(x, dict):
        for key, val in x.items():
            if not isinstance(key, str) or not _ENUM.match(key):
                v.append(f"{path}.<key>: key is not an identifier")
                continue
            _opaque_tree(val, f"{path}.{key}", v, depth + 1)
    elif isinstance(x, list):
        for i, val in enumerate(x):
            _opaque_tree(val, f"{path}[{i}]", v, depth + 1)
    elif isinstance(x, str):
        _opaque_str(x, path, v)
    elif not (x is None or isinstance(x, (bool, int, float))):
        v.append(f"{path}: unsupported type")


def _tools(tools: Any, v: list[str]) -> None:
    if not isinstance(tools, list):
        v.append("tools: not a list")
        return
    for i, t in enumerate(tools):
        p = f"tools[{i}]"
        if not isinstance(t, dict):
            v.append(f"{p}: not an object")
            continue
        for key in sorted(set(t) - TOOL_KEYS):
            v.append(f"{p}.{key}: key not allowed")
        if not isinstance(t.get("tool"), str) or not _TOOLREF.match(t["tool"]):
            v.append(f"{p}.tool: not an exact tool ref")
        _static_text(t.get("description"), f"{p}.description", v)
        _static_tree(t.get("args_schema"), f"{p}.args_schema", v)


def _observations(obs: Any, k: int, v: list[str]) -> None:
    if not isinstance(obs, list):
        v.append("observations: not a list")
        return
    for i, o in enumerate(obs):
        p = f"observations[{i}]"
        if not isinstance(o, dict):
            v.append(f"{p}: not an object")
            continue
        for key in sorted(set(o) - OBS_KEYS):
            v.append(f"{p}.{key}: key not allowed")
        if not isinstance(o.get("tool"), str) or not _TOOLREF.match(o["tool"]):
            v.append(f"{p}.tool: not an exact tool ref")
        if not isinstance(o.get("status"), str) or not _ENUM.match(o["status"]):
            v.append(f"{p}.status: not an enum")
        if "args" in o:
            _opaque_tree(o["args"], f"{p}.args", v)
        if o.get("error") is not None and (not isinstance(o["error"], str) or not _ENUM.match(o["error"])):
            v.append(f"{p}.error: must be null or an enum code")
        _result(o.get("result"), f"{p}.result", k, v)


def _result(res: Any, path: str, k: int, v: list[str]) -> None:
    if res is None:
        return
    if not isinstance(res, dict) or set(res) - {"rows"}:
        v.append(f"{path}: only a rows list of treated aggregates is allowed")
        return
    rows = res.get("rows")
    if not isinstance(rows, list):
        v.append(f"{path}.rows: not a list")
        return
    for i, r in enumerate(rows):
        _row(r, f"{path}.rows[{i}]", k, v)


def _row(r: Any, path: str, k: int, v: list[str]) -> None:
    if not isinstance(r, dict):
        v.append(f"{path}: not an object")
        return
    for key in sorted(set(r) - ROW_KEYS):
        if not (key.startswith("g_") and _ENUM.match(key) and isinstance(r[key], str) and _HASH.match(r[key])):
            v.append(f"{path}.{key}: field not allowed (group keys are g_* HMAC hashes of 16 hex chars)")
    for key in ("metric_id", "window_id"):
        if not isinstance(r.get(key), str) or not _ENUM.match(r[key]):
            v.append(f"{path}.{key}: must be an enum identifier")
    count = r.get("count")
    if count == SUPPRESSED:
        extra = set(r) - {"metric_id", "window_id", "count"}
        if extra:
            v.append(f"{path}: suppressed row carries {sorted(extra)}")
        return
    if not _is_int(count) or count < k:
        v.append(f"{path}.count: must be an integer >= k={k} or the suppressed marker")
    if "rate" in r:
        rate = r["rate"]
        if not isinstance(rate, (int, float)) or isinstance(rate, bool) or round(rate, 2) != rate:
            v.append(f"{path}.rate: must be a number rounded to 2 decimals")
    if "evidence_ref" in r and (not isinstance(r["evidence_ref"], str) or not _EVREF.match(r["evidence_ref"])):
        v.append(f"{path}.evidence_ref: not an opaque ev_ id")
