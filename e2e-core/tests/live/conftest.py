"""Live fixtures: the real_local stack started by run.ps1 (E2E_ENV_FILE / E2E_KEYS_FILE). Without them the live
tests are skipped (never reported as passed). Also collects the honest e2e-report.json."""

from __future__ import annotations

import json
import os
import time
from collections import defaultdict
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx
import pytest

from codex_standin import report
from codex_standin.bridge import Bridge
from codex_standin.engine import TENANT, Db, Engine
from codex_standin.stack import SECRETS

STARTED = datetime.now(UTC).isoformat()
OUTCOMES: dict[str, dict[str, Any]] = defaultdict(lambda: {"passed": 0, "failed": 0, "skipped": [], "xfailed": []})
GAPS: dict[str, dict[str, str]] = {}
EFFECTS: dict[str, Any] = {}
SESSION: dict[str, Any] = {}


@dataclass
class Stack:
    env: dict[str, Any]
    keys: dict[str, Any]
    bridge: Bridge
    fx: httpx.Client
    pg_password: str = field(default="", repr=False)
    engine: Engine | None = None
    runtime_db: Db | None = None
    eval_db: Db | None = None
    extra: dict[str, Any] = field(default_factory=dict)

    @property
    def runtime(self) -> str:
        return f"http://127.0.0.1:{self.env['ports']['runtime']}"


def pytest_collection_modifyitems(config: Any, items: list[Any]) -> None:
    if not os.environ.get("E2E_ENV_FILE"):
        skip = pytest.mark.skip(reason="no live stack: run e2e-core/run.ps1")
        for item in items:
            if "live" in item.keywords:
                item.add_marker(skip)


def pytest_runtest_logreport(report: Any) -> None:
    mod = report.nodeid.split("::")[0].split("/")[-1]
    row = OUTCOMES[mod]
    if report.when == "call" and report.passed:
        row["passed"] += 1
    elif report.failed:
        row["failed"] += 1
    elif report.skipped and hasattr(report, "wasxfail"):
        row["xfailed"].append({"case": report.nodeid.split("::")[-1], "reason": str(report.wasxfail)})
    elif report.skipped:
        reason = report.longrepr[2] if isinstance(report.longrepr, tuple) else str(report.longrepr)
        row["skipped"].append({"case": report.nodeid.split("::")[-1], "reason": reason})


def _pg_password(ns: str) -> str:
    for line in (SECRETS / ns / "core.env").read_text(encoding="utf-8").splitlines():
        if line.startswith("POSTGRES_PASSWORD="):
            return line.split("=", 1)[1].strip()
    raise RuntimeError("POSTGRES_PASSWORD missing")


@pytest.fixture(scope="session")
def stack() -> Stack:
    env = json.loads(Path(os.environ["E2E_ENV_FILE"]).read_text(encoding="ascii"))
    keys = json.loads(Path(os.environ["E2E_KEYS_FILE"]).read_text(encoding="ascii"))
    runtime = f"http://127.0.0.1:{env['ports']['runtime']}"
    bridge = Bridge(runtime, keys["service_seed"], keys["service_kid"])
    fx = httpx.Client(base_url=env["url"], timeout=120)
    deadline = time.time() + 90
    while time.time() < deadline:
        try:
            if httpx.get(runtime + "/readyz", timeout=3).status_code == 200 and fx.get("/_e2e/info").status_code == 200:
                break
        except httpx.HTTPError:
            pass
        time.sleep(1)
    pw = _pg_password(env["namespace"])
    pg = env["ports"]["postgres"]
    st = Stack(env, keys, bridge, fx, pw)
    st.engine = Engine(bridge, fx, TENANT)
    st.runtime_db = Db(f"postgresql://postgres:{pw}@127.0.0.1:{pg}/core_runtime")
    st.eval_db = Db(f"postgresql://postgres:{pw}@127.0.0.1:{pg}/core_eval")
    SESSION["stack"] = st
    return st


@pytest.fixture(scope="session")
def pipeline(stack: Stack) -> Any:
    from helpers import run_pipeline
    assert stack.engine is not None
    return run_pipeline(stack.engine, stack.runtime_db)


@pytest.fixture
def gap() -> Any:
    def add(code: str, detail: str, request: str) -> None:
        GAPS[code] = {"code": code, "detail": detail, "request": request}
    return add


@pytest.fixture
def effect() -> Any:
    def record(name: str, value: Any) -> None:
        EFFECTS[name] = value
    return record


def pytest_sessionfinish(session: Any, exitstatus: int) -> None:
    st = SESSION.get("stack")
    if st is None or not os.environ.get("E2E_REPORT"):
        return
    ver = st.bridge.version()
    fx_pieces = st.fx.get("/_e2e/info").json()["pieces"]
    declared = report.declared_doubles(fx_pieces)
    suites = [{"name": mod, "total": r["passed"] + r["failed"] + len(r["skipped"]) + len(r["xfailed"]),
               "passed": r["passed"], "failed": r["failed"], "not_run": len(r["skipped"]),
               "skipped": r["skipped"], "known_gaps_xfailed": r["xfailed"]} for mod, r in sorted(OUTCOMES.items())]
    rep = report.build(
        env=st.env, version=ver, runtime_url=st.runtime, declared=declared, suites=suites,
        gaps=sorted([*report.KNOWN_STACK_GAPS, *GAPS.values()], key=lambda g: g["code"]), effects=EFFECTS,
        commands=["e2e-core/run.ps1", "local/core/start.ps1 -Profile real_local", "pytest tests/unit tests/live"],
        started=STARTED, extra={"pytest_exit_status": int(exitstatus), "llm_unscripted_calls_total":
                                st.engine.state()["llm_unscripted"] if st.engine else None})
    report.write(Path(os.environ["E2E_REPORT"]), rep)
