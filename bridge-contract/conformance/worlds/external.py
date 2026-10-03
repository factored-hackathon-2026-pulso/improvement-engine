"""World for ANY implementation reachable over HTTP (your Rust double, a deployed runtime): key material by env."""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from conformance.kit import Api, Signer, b64u_decode
from conformance.worlds import World


class ExternalWorld(World):
    def __init__(self, target: str) -> None:
        self.target = target
        self.base_url = os.environ.get("CONTRACT_BASE_URL", "")
        key_file = os.environ.get("CONTRACT_SERVICE_KEY_FILE")
        if not self.base_url or not key_file:
            pytest.skip("external target needs CONTRACT_BASE_URL and CONTRACT_SERVICE_KEY_FILE")
        material = json.loads(Path(key_file).read_text(encoding="utf-8"))
        self.tenant = os.environ.get("CONTRACT_TENANT", "t1")
        self.other_tenant = os.environ.get("CONTRACT_OTHER_TENANT", "t2")
        self.unknown_tenant = os.environ.get("CONTRACT_UNKNOWN_TENANT", "t-not-deployed")
        self.signer = Signer(os.environ.get("CONTRACT_SERVICE_KID", material["kid"]), b64u_decode(material["seed"]),
                             iss=os.environ.get("CONTRACT_SERVICE_ISS", "control-api"),
                             aud=os.environ.get("CONTRACT_SERVICE_AUD", "core-bridge"))
        world_file = os.environ.get("CONTRACT_WORLD_FILE")
        extra = json.loads(Path(world_file).read_text(encoding="utf-8")) if world_file else {}
        self.scout_release = extra.get("scout_release", self.scout_release)
        self.writer_release = extra.get("writer_release", self.writer_release)
        self.agent_version = extra.get("agent_version", self.agent_version)
        self.caps = set(extra.get("caps", ["credentials"]))
        self.api = Api(self.base_url, self.signer, self.tenant)

    def close(self) -> None:
        self.api.close()


def start(target: str) -> ExternalWorld:
    return ExternalWorld(target)
