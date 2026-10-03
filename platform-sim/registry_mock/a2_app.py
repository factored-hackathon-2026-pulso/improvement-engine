"""Level a2: the REAL `RegistryService` + `registry_extension` of the pinned agent-core over `InMemoryRegistryStore`
with a scripted `EvalPort` and the sim staff verifier. Needs the pinned checkout on `sys.path` (PULSO_CORE_CHECKOUT;
`testing/` is NOT used). Adds the same `/_sim/*` control channel as the mock (never present on the real server).

    PULSO_CORE_CHECKOUT=<pin checkout> python -m registry_mock.a2_app --port 8602
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

CHECKOUT = Path(os.environ.get("PULSO_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core-894fa65"))
if str(CHECKOUT) not in sys.path:
    sys.path.insert(0, str(CHECKOUT))

from fastapi import FastAPI, Request  # noqa: E402
from fastapi.responses import JSONResponse  # noqa: E402

from agent_core.adapters.jws_identity import JwsIdentityVerifier  # noqa: E402
from agent_core.api.problems import install_error_handlers  # noqa: E402
from agent_core.api.tracing import install_tracing  # noqa: E402
from agent_core.domain import Principal  # noqa: E402
from agent_core.registry.evaluation.ports import EvalRequest  # noqa: E402
from agent_core.registry.evaluation.report import EvalReport  # noqa: E402
from agent_core.registry.http import registry_extension  # noqa: E402
from agent_core.registry.memory import InMemoryRegistryStore  # noqa: E402
from agent_core.registry.service import RegistryService  # noqa: E402

from registry_mock import jws  # noqa: E402
from registry_mock.sim_common import CONTRACT_VERSION, PIN_SHA, SimClock, SimIds, fixtures_digest  # noqa: E402


@dataclass
class ScriptedEval:
    """Scripted `EvalPort`: labelled `contract_fixture`; never simulates improvement."""

    script: list[str] = field(default_factory=list)
    calls: int = 0

    def run(self, request: EvalRequest) -> EvalReport:
        self.calls += 1
        verdict = self.script.pop(0) if self.script else "pass"
        if verdict == "timeout":  # a harness timeout never counts as pass or fail
            return EvalReport(verdict="failed_infra", detail="contract_fixture: timeout")
        return EvalReport(verdict=verdict, detail="contract_fixture")  # type: ignore[arg-type]


def _admin() -> Principal:
    return Principal.model_validate(
        jws.principal_payload("root", "builder", ["constructor", "aprobador", "admin"], human=True, step_up=True))


class World:
    def __init__(self, checkout: Path) -> None:
        self.clock, self.ids, self.evaluator = SimClock(), SimIds(), ScriptedEval()
        self.store = InMemoryRegistryStore()
        self.service = RegistryService(self.store, self.evaluator, self.clock, self.ids)  # type: ignore[arg-type]
        self.service.import_seed(_admin(), checkout / "tests" / "fixtures" / "registry-demo")


def build_app(checkout: Path = CHECKOUT) -> FastAPI:
    holder: dict[str, World] = {"w": World(checkout)}

    app = FastAPI(title="a2-registry", version="1.0.0", docs_url=None, redoc_url=None, openapi_url="/openapi.json")
    install_tracing(app, holder["w"].ids)
    install_error_handlers(app)

    class _Dispatch:
        """Late-bound service so /_sim/reset can swap the world without re-installing routes."""

        def __getattr__(self, name: str) -> Any:
            return getattr(holder["w"].service, name)

    class _Clock:
        def now(self) -> Any:
            return holder["w"].clock.now()

        def monotonic_ns(self) -> int:
            return holder["w"].clock.monotonic_ns()

    verifier = JwsIdentityVerifier({jws.SIM_KID: jws.sim_public_key()}, {}, lambda _r, _n: False)
    registry_extension(_Dispatch(), verifier, _Clock())(app, lambda r, a: (_ for _ in ()).throw(RuntimeError()))  # type: ignore[arg-type]

    @app.get("/_sim/info")
    def info() -> dict[str, str]:
        return {"pinned_sha": PIN_SHA, "contract_version": CONTRACT_VERSION, "fixtures_digest": fixtures_digest(),
                "level": "a2"}

    @app.post("/_sim/reset")
    def reset() -> dict[str, str]:
        holder["w"] = World(checkout)
        return {"status": "reset"}

    @app.post("/_sim/clock/advance")
    async def advance(request: Request) -> JSONResponse:
        holder["w"].clock.advance(float((await request.json())["seconds"]))
        return JSONResponse({"now": jws.iso(holder["w"].clock.now())})

    @app.post("/_sim/eval")
    async def program_eval(request: Request) -> dict[str, list[str]]:
        holder["w"].evaluator.script = list((await request.json())["script"])
        return {"script": holder["w"].evaluator.script}

    return app


if __name__ == "__main__":
    import argparse

    import uvicorn

    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8602)
    a = ap.parse_args()
    uvicorn.run(build_app(), host="127.0.0.1", port=a.port, log_level="warning", access_log=False)
