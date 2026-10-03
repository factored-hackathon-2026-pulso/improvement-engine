"""Level real_local (registry part): the REAL `RegistryService` + `registry_extension` of the pinned agent-core over
`PgRegistryStore` on a real Postgres 16, with the sim staff verifier and a fixed-pass `EvalPort` (a declared double).

Unlike a2 there is NO `/_sim` channel: the clock and ids are the system ones and the evaluator cannot be programmed, so
the cases that need `requires_sim` are not run here. Isolation between cases is done by the *harness* (`Harness.reset`:
a fresh throw-away database, schema + registry-demo seed), never through the served app.

    PULSO_CORE_CHECKOUT=<pin checkout> python -m registry_mock.real_app --admin-dsn postgresql://postgres:pw@127.0.0.1:55483/postgres
"""

from __future__ import annotations

import os
import sys
import threading
import time
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any

CHECKOUT = Path(os.environ.get("PULSO_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core"))
if str(CHECKOUT) not in sys.path:
    sys.path.insert(0, str(CHECKOUT))

import psycopg  # noqa: E402
from fastapi import FastAPI  # noqa: E402

from agent_core.adapters.jws_identity import JwsIdentityVerifier  # noqa: E402
from agent_core.adapters.postgres_uow import apply_schema  # noqa: E402
from agent_core.adapters.system_clock import SystemClock  # noqa: E402
from agent_core.adapters.system_ids import SystemIds  # noqa: E402
from agent_core.api.problems import install_error_handlers  # noqa: E402
from agent_core.api.tracing import install_tracing  # noqa: E402
from agent_core.domain import Principal  # noqa: E402
from agent_core.registry import PgRegistryStore, apply_registry_schema  # noqa: E402
from agent_core.registry.evaluation.ports import EvalRequest  # noqa: E402
from agent_core.registry.evaluation.report import EvalReport  # noqa: E402
from agent_core.registry.http import registry_extension  # noqa: E402
from agent_core.registry.service import RegistryService  # noqa: E402

from registry_mock import jws  # noqa: E402

DOUBLES = ["eval_port:fixed_pass (not programmable)", "identity:sim_staff_key (JwsIdentityVerifier, sim public key)"]


class FixedPassEval:
    """`EvalPort` double: always `pass`. It cannot be programmed, so verdict-dependent cases are skipped on real."""

    def run(self, request: EvalRequest) -> EvalReport:
        return EvalReport(verdict="pass", detail="contract_fixture")


def _admin() -> Principal:
    return Principal.model_validate(
        jws.principal_payload("root", "builder", ["constructor", "aprobador", "admin"], human=True, step_up=True))


def _with_db(admin_dsn: str, name: str) -> str:
    return admin_dsn.rpartition("/")[0] + "/" + name


class Harness:
    """Owns the throw-away databases; the served app reads `dsn` late so `reset()` swaps the world."""

    def __init__(self, admin_dsn: str, checkout: Path = CHECKOUT) -> None:
        self.admin_dsn, self.checkout = admin_dsn, checkout
        self.tag = uuid.uuid4().hex[:8]
        self.n = 0
        self.dsn = ""
        self.names: list[str] = []
        self.reset()

    def reset(self) -> None:
        self.n += 1
        name = f"parity_{self.tag}_{self.n}"
        with psycopg.connect(self.admin_dsn, autocommit=True) as admin:
            admin.execute(f'CREATE DATABASE "{name}"')
        dsn = _with_db(self.admin_dsn, name)
        with psycopg.connect(dsn) as conn:
            apply_schema(conn, None)
            apply_registry_schema(conn, None)
        store = PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False))
        RegistryService(store, FixedPassEval(), SystemClock(), SystemIds()).import_seed(  # type: ignore[arg-type]
            _admin(), self.checkout / "tests" / "fixtures" / "registry-demo")
        old, self.dsn = self.names[-1:], dsn
        self.names.append(name)
        for o in old:
            self._drop(o)

    def _drop(self, name: str) -> None:
        with psycopg.connect(self.admin_dsn, autocommit=True) as admin:
            admin.execute(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE)')

    def close(self) -> None:
        for n in self.names:
            self._drop(n)
        self.names.clear()

    def build_app(self) -> FastAPI:
        h = self
        app = FastAPI(title="real-registry", version="1.0.0", docs_url=None, redoc_url=None, openapi_url="/openapi.json")
        ids = SystemIds()
        install_tracing(app, ids)
        install_error_handlers(app)

        class _Dispatch:
            def __getattr__(self, name: str) -> Any:  # late-bound: a fresh store/service on the current database
                dsn = h.dsn
                store = PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False))
                return getattr(RegistryService(store, FixedPassEval(), SystemClock(), ids), name)  # type: ignore[arg-type]

        verifier = JwsIdentityVerifier({jws.SIM_KID: jws.sim_public_key()}, {}, lambda _r, _n: False)
        registry_extension(_Dispatch(), verifier, SystemClock())(app, lambda r, a: (_ for _ in ()).throw(RuntimeError()))  # type: ignore[arg-type]
        return app


@contextmanager
def serve_real(admin_dsn: str) -> Iterator[tuple[str, Harness]]:
    """Run the real app in a uvicorn thread; yield (base_url, harness). Cleans every throw-away database."""
    import uvicorn

    from parity.servers import free_port

    h = Harness(admin_dsn)
    port = free_port()
    server = uvicorn.Server(uvicorn.Config(h.build_app(), host="127.0.0.1", port=port, log_level="warning", access_log=False))
    t = threading.Thread(target=server.run, daemon=True)
    t.start()
    try:
        deadline = time.time() + 30
        while not server.started and time.time() < deadline:
            time.sleep(0.1)
        if not server.started:
            raise RuntimeError("real registry app did not start")
        yield f"http://127.0.0.1:{port}", h
    finally:
        server.should_exit = True
        t.join(10)
        h.close()
