"""registry-wire-contract (V3 31.10.3). Inputs: REGISTRY_BASE_URL, TARGET in {mock, a2, real} (default mock).

mock != fixture  -> `mock_infidelity` (ordinary CI fails)
a2/real != fixture -> `wire_drift_detected` (blocks a pin bump)
A mock-only case never counts toward fidelity; a report states its `target` and never claims real_local from mock/a2.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

import httpx
import pytest

from parity import runner
from parity.servers import serve
from registry_mock.sim_common import PIN_SHA, fixtures_digest

TARGET = os.environ.get("TARGET", "mock")
BASE_URL = os.environ.get("REGISTRY_BASE_URL")
SIM = TARGET in ("mock", "a2")
LABEL = "mock_infidelity" if TARGET == "mock" else "wire_drift_detected"
REPORT = Path(os.environ.get("PARITY_REPORT", runner.HERE / ".out" / f"parity_report.{TARGET}.json"))
CASES = runner.load_cases()
_results: dict[str, str] = {}
_route_digest: list[str] = []

pytestmark = pytest.mark.parity


@pytest.fixture(scope="session")
def client():
    if BASE_URL:
        with httpx.Client(base_url=BASE_URL, timeout=120) as c:
            yield c
    elif TARGET == "real":
        pytest.skip("TARGET=real needs REGISTRY_BASE_URL")
    else:
        with serve(TARGET) as url, httpx.Client(base_url=url, timeout=120) as c:
            yield c


def _applies(case: runner.Case) -> str | None:
    if case.applies_to == "mock_only" and TARGET != "mock":
        return "mock_only"
    if case.requires_sim and not SIM:
        return "requires_sim"
    return None


@pytest.mark.parametrize("case", CASES, ids=[c.id for c in CASES])
def test_case(client, case: runner.Case) -> None:
    skip = _applies(case)
    if skip:
        _results[case.id] = f"skipped:{skip}"
        pytest.skip(skip)
    fixture = runner.fixture_path(case.id)
    assert fixture.exists(), f"no recorded fixture for {case.id}: run `python -m parity.record --target a2`"
    expected = json.loads(fixture.read_text(encoding="utf-8"))["steps"]
    actual = runner.run_case(client, case, sim=SIM).steps
    problems = runner.diff_steps(expected, actual)
    _results[case.id] = "failed" if problems else "passed"
    assert not problems, f"{LABEL} [{case.id}] ({TARGET}):\n  " + "\n  ".join(problems[:12])


def test_fixtures_exist_for_every_case() -> None:
    missing = [c.id for c in CASES if not runner.fixture_path(c.id).exists()]
    assert not missing, missing


def test_every_mock_only_case_is_justified() -> None:
    assert all(c.justification.strip() for c in CASES if c.applies_to == "mock_only")


def test_sim_info_reports_pin(client) -> None:
    if not SIM:
        pytest.skip("/_sim is absent from the real server by design")
    info = client.get("/_sim/info").json()
    assert info["pinned_sha"] == PIN_SHA
    assert info["contract_version"] == "1.3.0"
    assert info["fixtures_digest"] == fixtures_digest()


def test_route_table_equals_a2_snapshot(client) -> None:
    """Route table of the target equals the one derived from the a2 app at the pin (a pin bump adding a route fails)."""
    snap = runner.WIRE / "derived" / "registry_openapi.json"
    expected = _route_table(json.loads(snap.read_text(encoding="utf-8")))
    actual = _route_table(client.get("/openapi.json").json())
    _route_digest.append(hashlib.sha256(chr(10).join(actual).encode()).hexdigest())
    assert actual == expected, {"missing": sorted(set(expected) - set(actual)), "extra": sorted(set(actual) - set(expected))}
    assert len(expected) == 16


def _route_table(openapi: dict) -> list[str]:
    return sorted(f"{m.upper()} {p}" for p, ops in openapi["paths"].items() if p.startswith("/v1/registry")
                  for m in ops if m in ("get", "post", "put", "delete", "patch"))


@pytest.fixture(scope="session", autouse=True)
def _report():
    yield
    both = [c for c in CASES if c.applies_to == "both"]
    ran = [c for c in both if _results.get(c.id) in ("passed", "failed")]
    passed = [c.id for c in ran if _results[c.id] == "passed"]
    failed = [c.id for c in ran if _results[c.id] == "failed"]
    table = hashlib.sha256("\n".join(_ROUTES).encode()).hexdigest() if (_ROUTES := globals().get("_ROUTES_LIST", [])) else None
    report = {
        "target": TARGET, "sha": PIN_SHA, "label": {"mock": "contract_mock", "a2": "a2_real_service_in_memory",
                                                    "real": "real_local"}[TARGET],
        "cases_total": len(CASES), "cases_both": len(both), "cases_ran": len(ran), "passed": len(passed),
        "failed": failed,
        "skipped": sorted(k for k, v in _results.items() if v.startswith("skipped")),
        "mock_only": [c.id for c in CASES if c.applies_to == "mock_only"],
        "route_table_digest": _route_digest[0] if _route_digest else None, "fixtures_digest": fixtures_digest(),
        "claims_real_local": False,
    }
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
