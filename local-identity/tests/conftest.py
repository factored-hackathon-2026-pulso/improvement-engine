"""Shared fixtures: a throw-away keyset, an in-process app and a client wired to it (no network)."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from local_identity.app import build_app
from local_identity.client import LocalIdentityClient, ServiceSigner
from local_identity.config import Config, load_config
from local_identity.keys import generate_keyset

TENANT = "tenant-a"
SUPERVISOR = "local-supervisor"
ADMIN = "local-admin"
CONSTRUCTOR_ONLY = "local-builder-human"
HASH = "a" * 64


class Clock:
    def __init__(self) -> None:
        self.now = datetime(2026, 10, 3, 12, 0, 0, tzinfo=UTC)

    def __call__(self) -> datetime:
        return self.now


@dataclass
class Env:
    dir: Path
    config: Config
    clock: Clock
    app: FastAPI
    http: TestClient
    client: LocalIdentityClient
    make_client: Callable[..., LocalIdentityClient]


def env_for(keys: Path, **extra: str) -> dict[str, str]:
    return {
        "LOCAL_IDENTITY_PROFILE": "local",
        "LOCAL_IDENTITY_SERVICE_KEYS": str(keys / "human-issuer-service-keys.json"),
        "LOCAL_IDENTITY_SESSION_SIGNER": str(keys / "session-signer.json"),
        "LOCAL_IDENTITY_HUMAN_STAFF_SIGNER": str(keys / "human-staff-signer.json"),
        "LOCAL_IDENTITY_IDENTITIES": str(keys / "identities.json"),
        **extra,
    }


@pytest.fixture
def env(tmp_path: Path) -> Env:
    keys = generate_keyset(tmp_path / "keys", tenant_id=TENANT)
    clock = Clock()
    config = load_config(env_for(keys.dir))
    app = build_app(config, now=clock)
    http = TestClient(app, base_url="http://human-issuer:8083")

    def make_client(**kw: Any) -> LocalIdentityClient:
        signer = kw.pop("signer", None) or ServiceSigner.from_file(keys.dir / "control-api-signer.json")
        kw.setdefault("tenant_id", TENANT)
        return LocalIdentityClient("http://human-issuer:8083", signer, http_client=http, now=clock, **kw)

    return Env(keys.dir, config, clock, app, http, make_client(), make_client)


def proposal_target(**over: object) -> dict[str, object]:
    return {"proposal_id": "prop-1", "candidate_hash": HASH, "expected_revision": 3, **over}


def release_target(**over: object) -> dict[str, object]:
    return {"release_id": "rel-1", "agent_id": "agent-1", "alias": "prod", "expected_revision": 1, **over}
