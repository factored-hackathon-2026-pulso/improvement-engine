"""Ledger hardening on real PG16: a negative spend can never refund the budget."""

from __future__ import annotations

import pytest
from pulso_core_runtime.internal.store import ensure_schema
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

from .conftest import PgDbs

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def test_negative_or_non_finite_spend_is_refused_and_never_applied(pg: PgDbs) -> None:
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    s = ReceiptStore(pg.runtime)
    assert s.meter_spend("t", "j", "st", 1, cost_usd="0.5", cap_usd="1") is True
    for bad in ("-0.4", "NaN", "Infinity", "-Infinity", "abc"):
        assert s.meter_spend("t", "j", "st", 1, cost_usd=bad, cap_usd="1") is False, bad
    row = s.meter_get("t", "j", "st", 1)
    assert row is not None and str(row["cost_usd"]).rstrip("0").rstrip(".") == "0.5" and row["calls"] == 1


SCAN = r"""
import io, json, sys
from pulso_core_runtime import main
env = json.load(sys.stdin)
cap = []
err = io.StringIO()
code = main.run([], env=env, stderr=err, serve=lambda app, **kw: cap.append(app))
from fastapi.testclient import TestClient
TestClient(cap[0]).get("/readyz") if cap else None
bad = sorted(m for m in sys.modules if m == "testing" or m.startswith("testing.") or ".testing" in m
             or m.split(".")[0] in ("agent_core_testing",))
print(json.dumps({"code": code, "bad": bad, "err": err.getvalue()[-300:]}))
"""


def test_composed_runtime_never_imports_test_doubles(pg: PgDbs, tmp_path) -> None:  # type: ignore[no-untyped-def]
    """Demo-mode escape scan: compose in a clean interpreter (no pytest, no seeding helpers) and assert no
    `testing.*` module was imported while composing, serving readyz or resolving the factories."""
    import json
    import os
    import subprocess
    import sys

    from .test_pg_runtime import _env, keys

    k = keys.__wrapped__(tmp_path) if hasattr(keys, "__wrapped__") else None  # fixture body, called directly
    assert k is not None
    ensure_schema(pg.runtime)
    out = subprocess.run([sys.executable, "-c", SCAN], input=json.dumps(_env(pg, k)), capture_output=True,
                         text=True, timeout=120, check=False,
                         env={**os.environ, "PYTHONPATH": os.pathsep.join(p for p in sys.path if p)})
    assert out.returncode == 0, out.stderr[-500:]
    result = json.loads(out.stdout.strip().splitlines()[-1])
    assert result["code"] == 0, result["err"]
    assert result["bad"] == []
