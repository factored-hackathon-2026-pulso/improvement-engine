"""registry-wire-contract (V3 31.10.3). Inputs: REGISTRY_BASE_URL, TARGET in {mock, a2, real, real_scripted} (default mock).

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
PG_ADMIN = os.environ.get("PULSO_TEST_PG_ADMIN")
SIM = TARGET in ("mock", "a2")
REAL = TARGET in ("real", "real_scripted")
_reset = [None]  # harness reset for TARGET=real* over the throw-away PG (the served app has no /_sim)
_control = [None]  # in-process eval/clock control (real_scripted only): the Harness, never the served app
_real_info: dict = {}
LABEL = "mock_infidelity" if TARGET == "mock" else "wire_drift_detected"
REPORT = Path(os.environ.get("PARITY_REPORT", runner.HERE / ".out" / f"parity_report.{TARGET}.json"))
CASES = runner.load_cases()
_results: dict[str, str] = {}
_route_digest: list[str] = []

pytestmark = pytest.mark.parity
REAL_VS_A2_DIFFERENCES: dict[str, str] = {}  # case id -> justification (none at the pin)
SCRIPTED_VS_A2_DIFFERENCES: dict[str, str] = {}  # real_pg_scripted vs a2: case id -> justification


@pytest.fixture(scope="session")
def client():
    if BASE_URL:
        with httpx.Client(base_url=BASE_URL, timeout=120) as c:
            yield c
    elif REAL:
        if not PG_ADMIN:
            pytest.skip("TARGET=real needs REGISTRY_BASE_URL or PULSO_TEST_PG_ADMIN (throw-away PG16)")
        import subprocess

        from registry_mock.real_app import CHECKOUT, DOUBLES, DOUBLES_SCRIPTED, serve_real

        head = subprocess.run(["git", "-C", str(CHECKOUT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        assert head == PIN_SHA, f"checkout HEAD {head} != pin"

        scripted = TARGET == "real_scripted"
        with serve_real(PG_ADMIN, scripted=scripted) as (url, harness), httpx.Client(base_url=url, timeout=120) as c:
            _reset[0] = harness.reset
            _control[0] = harness if scripted else None
            _real_info.update(doubles=DOUBLES_SCRIPTED if scripted else DOUBLES, sim_absent=c.get("/_sim/info").status_code == 404,
                              pg=_pg_version(PG_ADMIN))
            yield c
    else:
        with serve(TARGET) as url, httpx.Client(base_url=url, timeout=120) as c:
            yield c


def _pg_version(admin_dsn: str) -> str:
    import psycopg

    with psycopg.connect(admin_dsn) as conn:
        return str(conn.execute("show server_version").fetchone()[0])


def _applies(case: runner.Case) -> str | None:
    if case.applies_to == "mock_only" and TARGET != "mock":
        return "mock_only"
    if case.requires_sim and not SIM and TARGET != "real_scripted":
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
    actual = runner.run_case(client, case, sim=SIM, reset=_reset[0], control=_control[0]).steps
    problems = runner.diff_steps(expected, actual)
    _results[case.id] = "failed" if problems else "passed"
    assert not problems, f"{LABEL} [{case.id}] ({TARGET}):\n  " + "\n  ".join(problems[:12])


def test_real_recordings_equal_a2_fixtures() -> None:
    """Recorded `real/` fixtures (real service over PG16) vs the a2 ones: every difference must be listed in
    REAL_VS_A2_DIFFERENCES with its justification (empty at the pin: real == a2 on every case that runs on real)."""
    real_dir = runner.fixtures_dir("real")
    if not real_dir.is_dir():
        pytest.skip("no real/ recordings (run `python -m parity.record --target real`)")
    differing = []
    for f in sorted(real_dir.glob("*.json")):
        a2 = json.loads(runner.fixture_path(f.stem).read_text(encoding="utf-8"))["steps"]
        if runner.diff_steps(a2, json.loads(f.read_text(encoding="utf-8"))["steps"]):
            differing.append(f.stem)
    assert sorted(differing) == sorted(REAL_VS_A2_DIFFERENCES), differing


def test_real_covers_exactly_the_non_sim_both_cases() -> None:
    real_dir = runner.fixtures_dir("real")
    if not real_dir.is_dir():
        pytest.skip("no real/ recordings")
    expected = {c.id for c in CASES if c.applies_to == "both" and not c.requires_sim}
    assert {f.stem for f in real_dir.glob("*.json")} == expected


def test_real_scripted_recordings_equal_a2_fixtures() -> None:
    """Recorded `real_pg_scripted/` fixtures (real service over PG16, scripted eval + fake clock) vs the a2 ones:
    every difference is a mock/a2 infidelity or must be justified in SCRIPTED_VS_A2_DIFFERENCES."""
    d = runner.fixtures_dir("real_scripted")
    if not d.is_dir():
        pytest.skip("no real_pg_scripted/ recordings (run `python -m parity.record --target real_scripted`)")
    differing = []
    for f in sorted(d.glob("*.json")):
        a2 = json.loads(runner.fixture_path(f.stem).read_text(encoding="utf-8"))["steps"]
        if runner.diff_steps(a2, json.loads(f.read_text(encoding="utf-8"))["steps"]):
            differing.append(f.stem)
    assert sorted(differing) == sorted(SCRIPTED_VS_A2_DIFFERENCES), differing


def test_real_scripted_covers_exactly_the_both_cases() -> None:
    d = runner.fixtures_dir("real_scripted")
    if not d.is_dir():
        pytest.skip("no real_pg_scripted/ recordings")
    assert {f.stem for f in d.glob("*.json")} == {c.id for c in CASES if c.applies_to == "both"}


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
    assert len(expected) == 18


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
    report = {
        "target": TARGET, "sha": PIN_SHA, "label": {"mock": "contract_mock", "a2": "a2_real_service_in_memory",
                                                    "real": "real_local", "real_scripted": "real_local_scripted"}[TARGET],
        "cases_total": len(CASES), "cases_both": len(both), "cases_ran": len(ran), "passed": len(passed),
        "failed": failed,
        "skipped": sorted(k for k, v in _results.items() if v.startswith("skipped")),
        "mock_only": [c.id for c in CASES if c.applies_to == "mock_only"],
        "route_table_digest": _route_digest[0] if _route_digest else None, "fixtures_digest": fixtures_digest(),
        "claims_real_local": False,
    }
    if TARGET == "real_scripted" and _real_info:
        report.update({
            "target": "real_local_scripted", "label": "real_local_scripted",
            "runtime_profile": "registry_extension_over_pg_registry_store", "doubles": _real_info["doubles"],
            "postgres_version": _real_info["pg"], "real_vs_a2_differences": SCRIPTED_VS_A2_DIFFERENCES,
            "claims_real_local": False,
            "claim_note": "real code and real PG16, but with a scripted EvalPort and a fake clock programmed in-process: "
                          "this is NOT the production evaluator or clock and never claims real_local",
            "claims_real_pg_scripted": bool(_real_info["sim_absent"] and not failed and len(ran) == len(both)),
            "evidence_notes": ["harness app (no image, no /readyz): image_digest is null",
                               "control of eval/clock is in-process (Harness); the served app has no /_sim"],
        })
    if TARGET == "real" and _real_info:
        needs_eval = sorted(c.id for c in both if c.requires_sim)
        report.update({
            "target": "real_local", "label": "real_local", "runtime_profile": "registry_extension_over_pg_registry_store",
            "doubles": _real_info["doubles"], "postgres_version": _real_info["pg"],
            "skipped_requires_programmable_eval_or_clock": needs_eval,
            "skipped_reason": "the real server has no /_sim: the EvalPort verdict, the clock and quota exhaustion cannot be programmed",
            "real_vs_a2_differences": REAL_VS_A2_DIFFERENCES,
            "evidence_notes": ["harness app (no image, no /readyz): image_digest is null",
                               "agent_core_sha equals the pin (checkout verified by the pin test)"],
            # plan 17.3.8: real_local only if /_sim is absent on the target and the sha is the pin
            "claims_real_local": bool(_real_info["sim_absent"] and not failed and len(ran) == len(both) - len(needs_eval)),
        })
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
