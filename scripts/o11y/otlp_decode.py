"""Minimal OTLP trace decoder (stdlib only) for tests and the mock Langfuse: protobuf and JSON bodies -> flat span dicts.

Span dict: {trace_id, span_id, parent_id, name, attrs{key: str|int|float|bool|list}, resource{key: ...}}.
Only the fields the closure checks need are decoded (ids, name, attributes, resource attributes).
"""
from __future__ import annotations

import gzip
import json


def _varint(b: bytes, i: int) -> tuple[int, int]:
    n = s = 0
    while True:
        c = b[i]
        i += 1
        n |= (c & 0x7F) << s
        if not c & 0x80:
            return n, i
        s += 7


def _fields(b: bytes):
    i = 0
    while i < len(b):
        key, i = _varint(b, i)
        f, w = key >> 3, key & 7
        if w == 0:
            v, i = _varint(b, i)
        elif w == 1:
            v, i = b[i:i + 8], i + 8
        elif w == 5:
            v, i = b[i:i + 4], i + 4
        elif w == 2:
            ln, i = _varint(b, i)
            v, i = b[i:i + ln], i + ln
        else:
            raise ValueError("unsupported protobuf wire type")
        yield f, w, v


def _anyvalue(b: bytes):
    for f, w, v in _fields(b):
        if f == 1:
            return v.decode("utf-8", "replace")
        if f == 2:
            return bool(v)
        if f == 3:
            return v - (1 << 64) if v >= 1 << 63 else v
        if f == 4:
            import struct
            return struct.unpack("<d", v)[0]
        if f == 5:  # ArrayValue{1: AnyValue}
            return [_anyvalue(x) for ff, _, x in _fields(v) if ff == 1]
    return None


def _kv(b: bytes) -> tuple[str, object]:
    k, val = "", None
    for f, w, v in _fields(b):
        if f == 1:
            k = v.decode()
        elif f == 2:
            val = _anyvalue(v)
    return k, val


def decode_protobuf(raw: bytes) -> list[dict]:
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    out = []
    for f, _, rs in _fields(raw):
        if f != 1:
            continue
        resource, scopes = {}, []
        for f2, _, v2 in _fields(rs):
            if f2 == 1:
                resource = dict(_kv(x) for ff, _, x in _fields(v2) if ff == 1)
            elif f2 == 2:
                scopes.append(v2)
        for sc in scopes:
            for f3, _, sp in _fields(sc):
                if f3 != 2:
                    continue
                d = {"trace_id": "", "span_id": "", "parent_id": "", "name": "", "attrs": {}, "resource": resource}
                for f4, _, v4 in _fields(sp):
                    if f4 == 1:
                        d["trace_id"] = v4.hex()
                    elif f4 == 2:
                        d["span_id"] = v4.hex()
                    elif f4 == 4:
                        d["parent_id"] = v4.hex()
                    elif f4 == 5:
                        d["name"] = v4.decode("utf-8", "replace")
                    elif f4 == 9:
                        k, v = _kv(v4)
                        d["attrs"][k] = v
                out.append(d)
    return out


def _jval(v: dict):
    for k in ("stringValue", "boolValue", "doubleValue"):
        if k in v:
            return v[k]
    if "intValue" in v:
        return int(v["intValue"])
    if "arrayValue" in v:
        return [_jval(x) for x in v["arrayValue"].get("values", [])]
    return None


def decode_json(body: dict) -> list[dict]:
    out = []
    for rs in body.get("resourceSpans", []):
        resource = {a["key"]: _jval(a["value"]) for a in rs.get("resource", {}).get("attributes", [])}
        for sc in rs.get("scopeSpans", []):
            for sp in sc.get("spans", []):
                out.append({"trace_id": sp.get("traceId", ""), "span_id": sp.get("spanId", ""),
                            "parent_id": sp.get("parentSpanId", ""), "name": sp.get("name", ""),
                            "attrs": {a["key"]: _jval(a["value"]) for a in sp.get("attributes", [])}, "resource": resource})
    return out


def decode(raw: bytes, content_type: str) -> list[dict]:
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    if "json" in content_type:
        return decode_json(json.loads(raw))
    return decode_protobuf(raw)
