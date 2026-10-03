"""First RED of L1b and permanent guard: mutating ONE mock field must break the parity suite (`mock_infidelity`).
Also covers the mock-only /_sim channel: faults default off, clock, programmable evaluation, golden hashes on a2."""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import httpx
import pytest

from parity import runner
from parity.servers import PLATFORM_SIM, serve
from registry_mock import jws

pytestmark = pytest.mark.parity
SUBSET = ["proposal-create-ok", "auth-missing-bearer", "freeze-golden", "evaluate-pass", "publish-missing-idempotency-key"]

MUTATIONS = {
    "drop a response field": ('"candidate_hash": p.candidate_hash,', ""),
    "problem without type": ('body: dict[str, Any] = {"type": f"urn:agentcore:{urn}:{code}", ', 'body: dict[str, Any] = {'),
    "accept publish without key": ('idempotency_key: Annotated[str, Header(max_length=255)],', 'idempotency_key: Annotated[str, Header(max_length=255)] = "none",'),
    "wrong status on create": ("return j(proposal_json(p), 201)", "return j(proposal_json(p), 200)"),
}


def _mutated_tree(tmp: Path, old: str, new: str) -> Path:
    root = tmp / "platform-sim"
    shutil.copytree(PLATFORM_SIM / "registry_mock", root / "registry_mock", ignore=shutil.ignore_patterns("__pycache__"))
    shutil.copytree(PLATFORM_SIM / "fixtures", root / "fixtures")
    app = root / "registry_mock" / "app.py"
    text = app.read_text(encoding="utf-8")
    assert old in text, f"mutation anchor vanished: {old[:40]}"
    app.write_text(text.replace(old, new, 1), encoding="utf-8")
    return root


def _failures(base: str) -> list[str]:
    bad = []
    wanted = {c.id: c for c in runner.load_cases() if c.id in SUBSET}
    with httpx.Client(base_url=base, timeout=60) as client:
        for cid, case in wanted.items():
            expected = json.loads(runner.fixture_path(cid).read_text(encoding="utf-8"))["steps"]
            if runner.diff_steps(expected, runner.run_case(client, case, sim=True).steps):
                bad.append(cid)
    return bad


def test_unmutated_mock_passes_the_subset() -> None:
    with serve("mock") as base:
        assert _failures(base) == []


@pytest.mark.parametrize("name", list(MUTATIONS))
def test_mutating_one_mock_field_breaks_parity(name: str, tmp_path: Path) -> None:
    old, new = MUTATIONS[name]
    root = _mutated_tree(tmp_path, old, new)
    with serve("mock", platform_sim=root, extra_env={"PULSO_WIRE_DIR": str(runner.WIRE)}) as base:
        failed = _failures(base)
    assert failed, f"mock_infidelity NOT detected for mutation {name!r}"


# --- /_sim channel (mock-only behaviour; default off) -------------------------------------------------------

@pytest.fixture()
def mock():
    with serve("mock") as base, httpx.Client(base_url=base, timeout=30) as c:
        yield c


H = {"Authorization": "Bearer " + jws.issue("bot")}


def test_faults_are_off_by_default(mock: httpx.Client) -> None:
    for _ in range(5):
        assert mock.get("/v1/registry/releases/rel-98130317a1003849", headers=H).status_code == 200


def test_fault_500_once_then_normal(mock: httpx.Client) -> None:
    mock.post("/_sim/fault", json={"mode": "status500"})
    r = mock.get("/v1/registry/releases/rel-98130317a1003849", headers=H)
    assert r.status_code == 500 and r.json()["code"] == "internal_error"
    assert mock.get("/v1/registry/releases/rel-98130317a1003849", headers=H).status_code == 200


def test_fault_drop_after_commit_loses_the_response_but_keeps_the_state(mock: httpx.Client) -> None:
    mock.post("/_sim/fault", json={"mode": "drop_after_commit"})
    with pytest.raises(httpx.TransportError):
        mock.post("/v1/registry/proposals", headers=H, json={"agent_id": "atencion", "title": "lost"})
    seen = mock.get("/v1/registry/proposals/proposal-0001", headers=H)
    assert seen.status_code == 200 and seen.json()["proposal"]["title"] == "lost"


def test_fault_disconnect_does_not_commit(mock: httpx.Client) -> None:
    mock.post("/_sim/fault", json={"mode": "disconnect"})
    with pytest.raises(httpx.TransportError):
        mock.post("/v1/registry/proposals", headers=H, json={"agent_id": "atencion", "title": "never"})
    assert mock.get("/v1/registry/proposals/proposal-0001", headers=H).status_code == 404


def test_fault_latency(mock: httpx.Client) -> None:
    import time

    mock.post("/_sim/fault", json={"mode": "latency", "seconds": 0.4})
    t = time.perf_counter()
    mock.get("/v1/registry/releases/rel-98130317a1003849", headers=H)
    assert time.perf_counter() - t >= 0.35


def test_unknown_fault_mode_is_rejected(mock: httpx.Client) -> None:
    assert mock.post("/_sim/fault", json={"mode": "chaos"}).status_code == 422


def test_clock_is_injectable_never_wall_time(mock: httpx.Client) -> None:
    before = mock.post("/_sim/clock/advance", json={"seconds": 0}).json()["now"]
    after = mock.post("/_sim/clock/advance", json={"seconds": 3600}).json()["now"]
    assert before.startswith("2026-01-01T00:00:00") and after.startswith("2026-01-01T01:00:00")
    # a token minted at the epoch with a 1 h TTL is valid at epoch and expired after the advance
    short = jws.issue("bot", exp=jws.SIM_EPOCH.replace(minute=30))
    assert mock.get("/v1/registry/releases/rel-98130317a1003849", headers={"Authorization": f"Bearer {short}"}).status_code == 401


def test_info_exposes_pin(mock: httpx.Client) -> None:
    info = mock.get("/_sim/info").json()
    assert set(info) >= {"pinned_sha", "contract_version", "fixtures_digest"}
    assert info["pinned_sha"] == "86a767474042a566a0dbd6ed23588959f27ebdb3" and info["contract_version"] == "1.3.0"


# --- a2: the golden case of V3 31.4.12 reproduces the hashes of the wire snapshot --------------------------

def test_a2_golden_candidate_matches_wire_vectors() -> None:
    if not Path(os.environ.get("PULSO_CORE_PYTHON", "") or (Path(os.environ.get("TEMP", ".")) / "pulso-wire-venv-86a7674" / "Scripts" / "python.exe")).exists():
        pytest.skip("pinned venv missing; run core-bridge/scripts/gen-wire.ps1")
    vectors = json.loads((runner.WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
    with serve("a2") as base, httpx.Client(base_url=base, timeout=60) as c:
        pid = c.post("/v1/registry/proposals", headers=H, json={"agent_id": "atencion", "title": "golden"}).json()["proposal_id"]
        c.put(f"/v1/registry/proposals/{pid}/draft", headers=H, json=runner._golden_draft(0)).raise_for_status()
        frozen = c.post(f"/v1/registry/proposals/{pid}/freeze", headers=H).json()
        base_rel = c.get("/v1/registry/releases/rel-98130317a1003849", headers=H).json()
    assert frozen["candidate_hash"] == vectors["candidate"]["candidate_hash"]
    assert frozen["release_id_preview"] == vectors["candidate"]["release_id_preview"]
    flow = next(e for e in base_rel["entities"] if e["ref"]["id"] == "disputa-cargo")
    assert flow["content_hash"].startswith("6b5b579464f54370")
