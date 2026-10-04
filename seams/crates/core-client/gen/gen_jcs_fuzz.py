#!/usr/bin/env python3
"""Generates tests/fixtures/jcs_fuzz.json: 150 seeded random integer/string documents, canonical form by `rfc8785`,
plus the put_draft digest and sealed-artifact digest of random draft plans.
Usage (repo root): uv run --with rfc8785 python seams/crates/core-client/gen/gen_jcs_fuzz.py
"""
import hashlib
import json
import pathlib
import random

import rfc8785

OUT = pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/jcs_fuzz.json"
R = random.Random(20261004)
POOL = ['"', "\\", "/", "\b", "\f", "\n", "\r", "\t", "\x00", "\x1f", "\x7f", "\x80", "é", "中", "\U0001f600", "\U00010000",
        "￿", "", "퟿", " ", "﻿", "a", "Z", "0", " ", "@", ":", "."]


def s():
    return "".join(R.choice(POOL) for _ in range(R.randint(0, 6)))


def val(d=0):
    k = R.randint(0, 6 if d < 3 else 3)
    if k == 0: return None
    if k == 1: return R.choice([True, False])
    if k == 2: return R.choice([0, -1, 1, R.randint(-10**6, 10**6), 9007199254740991, -9007199254740991])
    if k in (3, 4): return s()
    if k == 5: return [val(d + 1) for _ in range(R.randint(0, 4))]
    return {s(): val(d + 1) for _ in range(R.randint(0, 4))}


def obj():
    return {s(): val(1) for _ in range(R.randint(0, 4))}


def digest(v):
    return hashlib.sha256(rfc8785.dumps(v)).hexdigest()


docs = []
for _ in range(150):
    v = val()
    docs.append({"input": json.dumps(v, ensure_ascii=True), "canonical": rfc8785.dumps(v).decode("utf-8")})
plans = []
for _ in range(40):
    changes = [{"kind": s() or "k", "content": obj(), "docs": obj()} for _ in range(R.randint(1, 3))]
    p = {"agent_id": s() or "a", "title": s() or "t", "changes": changes}
    plans.append({"plan": p, "put_draft_digest": digest({"proposal_id": None, "expected_rev": None, "changes": changes}),
                  "artifact_digest": digest(p)})
OUT.write_bytes((json.dumps({"docs": docs, "plans": plans}, ensure_ascii=True) + "\n").encode("ascii"))
print("wrote", OUT)
