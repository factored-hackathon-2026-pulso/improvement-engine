"""ANN1 live harness: runs the REAL support-platform backend (PR 17 worktree) with its OWN sqlite file and the in-process
agent registry double (the same seam PR 17 tests use), seeded with `auto_detect` proposals the engine can announce.

Run from the platform's backend directory with its venv python:
    python serve_platform_live.py <work_dir> <port> [n_proposals]
The service token is read from CC_INTERNAL_SERVICE_TOKEN in the process environment (never printed, never written).
Writes <work_dir>/proposals.json = {"ids": [...], "by_hand": "<id>"} (opaque ids only).
"""
from __future__ import annotations

import json
import os
import sys
from datetime import UTC, datetime
from pathlib import Path

sys.path.insert(0, os.getcwd())

import uvicorn  # noqa: E402
from pydantic import SecretStr  # noqa: E402

from cc_platform.application.ai.registry import ProposalOrigin  # noqa: E402
from cc_platform.bootstrap.app import create_app  # noqa: E402
from cc_platform.bootstrap.container import AgentCoreServices, build_container  # noqa: E402
from cc_platform.infrastructure.ai.ed25519_issuer import Ed25519AgentCredentialIssuer  # noqa: E402
from cc_platform.infrastructure.ai.keys import AgentSigningKeys  # noqa: E402
from cc_platform.infrastructure.ai.memory_registry import InMemoryAgentRegistry  # noqa: E402
from cc_platform.infrastructure.ai.memory_runtime import InMemoryAgentRuntime  # noqa: E402
from cc_platform.infrastructure.clock import FixedClock  # noqa: E402
from tests.support import make_settings  # noqa: E402


def main() -> None:
    work, port = Path(sys.argv[1]), int(sys.argv[2])
    n = int(sys.argv[3]) if len(sys.argv) > 3 else 2
    token = os.environ.get("CC_INTERNAL_SERVICE_TOKEN", "")
    if not token:
        raise SystemExit("CC_INTERNAL_SERVICE_TOKEN is not set in the process environment")
    work.mkdir(parents=True, exist_ok=True)
    clock = FixedClock(datetime(2026, 10, 5, 12, 0, tzinfo=UTC))
    keys = AgentSigningKeys.generate(suffix="ann1")
    registry = InMemoryAgentRegistry(clock=clock, staff_kid=keys.staff.kid)
    ids = [
        registry.seed_proposal(agent_id="disputas", title=f"engine proposal {i}", created_by="engine", origin=ProposalOrigin.AUTO_DETECT)
        for i in range(n)
    ]
    by_hand = registry.seed_proposal(agent_id="disputas", title="by hand", origin=ProposalOrigin.BUILDER_CHAT)
    (work / "proposals.json").write_text(json.dumps({"ids": ids, "by_hand": by_hand}))
    settings = make_settings(
        database_url=f"sqlite+aiosqlite:///{(work / 'ann1_platform.db').as_posix()}",
        internal_service_token=SecretStr(token),
    )
    container = build_container(
        settings,
        clock=clock,
        agent_core=AgentCoreServices(
            issuer=Ed25519AgentCredentialIssuer(keys, clock),
            runtime=InMemoryAgentRuntime(principal_kid=keys.principal.kid),
            registry=registry,
        ),
    )
    uvicorn.run(create_app(container=container), host="127.0.0.1", port=port, log_level="warning")


if __name__ == "__main__":
    main()
