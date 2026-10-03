"""Conformance suite wiring. Target selection by env (names only; values are never printed):

  CONTRACT_TARGET            label: real | mock | external | <your label>   (default: real)
  CONTRACT_BASE_URL          http://host:port  (unset + real -> in-process runtime on PG16; unset + mock -> spawned mock)
  CONTRACT_SERVICE_KID       kid of the control-api signing key the target trusts          (external)
  CONTRACT_SERVICE_KEY_FILE  JSON {"kid":..., "seed": <b64url 32-byte Ed25519 seed>}         (external)
  CONTRACT_SERVICE_ISS/AUD   defaults control-api / core-bridge
  CONTRACT_TENANT / CONTRACT_OTHER_TENANT / CONTRACT_UNKNOWN_TENANT   a deployed tenant, a second deployed tenant,
                             and one that is NOT in the deployment set (defaults t1 / t2 / t-not-deployed)
  CONTRACT_WORLD_FILE        optional JSON with seeded refs: scout_release, writer_release, agent_version, caps[]
  PULSO_TEST_PG_ADMIN        only for the in-process real target (throwaway PG16)
"""

from __future__ import annotations

import os
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from conformance.kit import Api, schema_errors
from conformance.known_different import KNOWN_DIFFERENT
from conformance.worlds import World

TARGET = os.environ.get("CONTRACT_TARGET", "real")


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line("markers", "needs(cap): the test needs a target capability (world.caps)")
    config.addinivalue_line("markers", "pending_route: covers a route marked x-status pending-implementation")


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    table = KNOWN_DIFFERENT.get(TARGET, {})
    for item in items:
        base = item.nodeid.split("::", 1)[-1].split("[")[0]
        for pattern, (div_id, reason) in table.items():
            if base == pattern or (pattern.endswith("*") and base.startswith(pattern[:-1])):
                item.add_marker(pytest.mark.xfail(strict=True, reason=f"known-different {div_id}: {reason}"))
                break


@pytest.fixture(scope="session")
def world() -> Iterator[World]:
    base = os.environ.get("CONTRACT_BASE_URL")
    if TARGET == "real" and not base:
        from conformance.worlds.real import start
        w = start()
    elif TARGET == "mock":
        from conformance.worlds.mock import start as start_mock
        w = start_mock(base)
    else:
        from conformance.worlds.external import start as start_external
        w = start_external(TARGET)
    try:
        yield w
    finally:
        w.close()


@pytest.fixture(scope="session")
def api(world: World) -> Api:
    return world.api


@pytest.fixture(autouse=True)
def _needs(request: pytest.FixtureRequest) -> None:
    marker = request.node.get_closest_marker("needs")
    if marker is not None:
        world = request.getfixturevalue("world")
        missing = [c for c in marker.args if c not in world.caps]
        if missing:
            pytest.skip(f"target '{world.target}' has no capability {missing} (seeding is implementation-specific)")


def assert_valid(name: str, instance: object) -> None:
    errs = schema_errors(name, instance)
    assert not errs, f"{name}: {errs}"


def assert_error(resp, status: int, code: str, *, retryable: bool | None = None) -> dict:  # type: ignore[no-untyped-def]
    assert resp.status_code == status, f"{resp.status_code} {resp.text[:300]}"
    body = resp.json()
    assert_valid("ErrorEnvelope", body)
    assert body["code"] == code, body
    if retryable is not None:
        assert body["retryable"] is retryable
    return dict(body)
