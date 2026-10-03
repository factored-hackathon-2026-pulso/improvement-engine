"""platform-sim container entrypoint: registry mock, bridge mock and ingest fixture in ONE process (DOUBLES).

Each app is served on its own port (SIM_REGISTRY_PORT, SIM_BRIDGE_PORT, SIM_INGEST_PORT). The ingest app additionally
answers GET /_sim/info so that evidence code can detect a double (the real runtime never serves /_sim/info).
"""

from __future__ import annotations

import asyncio
import json
import os

import uvicorn


def _trusted_kids() -> list[str]:
    """kids of the bridge executor public keys the lab-broker double trusts (file from core-keygen; empty if absent)."""
    path = os.environ.get("SIM_LAB_BROKER_TRUST", "")
    try:
        with open(path, encoding="ascii") as fh:
            return sorted(json.load(fh)["keys"])
    except (OSError, ValueError, KeyError):
        return []


def _apps() -> list[tuple[object, int, str]]:
    from bridge_mock.app import create_app as bridge
    from ingest_fixture.app import create_app as ingest
    from registry_mock.app import create_app as registry

    ing = ingest()

    @ing.get("/_sim/info")
    def _info() -> dict[str, object]:  # pragma: no cover - trivial
        return {"double": True, "pieces": ["registry_mock", "bridge_mock", "ingest_fixture"],
                "lab_broker_trusted_kids": _trusted_kids()}

    reg, brg = registry(), bridge()
    for app in (reg, brg):
        app.add_api_route("/_sim/info", lambda: {"double": True}, methods=["GET"])
    return [(reg, int(os.environ.get("SIM_REGISTRY_PORT", "8601")), "registry"),
            (brg, int(os.environ.get("SIM_BRIDGE_PORT", "8611")), "bridge"),
            (ing, int(os.environ.get("SIM_INGEST_PORT", "8621")), "ingest")]


async def main() -> None:
    servers = [uvicorn.Server(uvicorn.Config(app, host="0.0.0.0", port=port, log_level="warning", access_log=False))
               for app, port, _ in _apps()]
    await asyncio.gather(*(s.serve() for s in servers))


if __name__ == "__main__":
    asyncio.run(main())
