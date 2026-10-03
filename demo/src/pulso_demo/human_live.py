"""LIVE ports for the approval flow (real_local stack): Core registry over HTTP and the REAL local human issuer.

The human issuer is internal only (no host port): `PodmanExecTransport` (e2e-core codex_standin.human_port) relays each request through
`podman exec` inside its container. The durable single-use intention and the `assert_bound` check are the Codex `HumanAuthorizationPort`
stand-in (e2e-core/src/codex_standin/human_port.py). Same wiring as e2e-core/tests/live/test_07_human_approval.py. Imports are lazy: they need
PYTHONPATH to contain e2e-core/src, local-identity/src and core-bridge/src (demo/run.ps1 does that)."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import httpx

from pulso_demo.human_flow import AGENT, ACTOR, Bearer, RegistryError


class LiveRegistry:
    """Core `/v1/registry` with a bot (constructor) credential for reads and the human JWS (bearer) for approve/publish/promote."""

    def __init__(self, runtime: str, bot_jws: str, agent: str = AGENT) -> None:
        self.base, self.bot, self.agent = f"{runtime}/v1/registry", bot_jws, agent

    def _call(self, method: str, path: str, bearer: str, **kw: Any) -> httpx.Response:
        headers = {"Authorization": f"Bearer {bearer}", **kw.pop("headers", {})}
        return httpx.request(method, self.base + path, headers=headers, timeout=60, **kw)

    @staticmethod
    def _ok(r: httpx.Response) -> dict[str, Any]:
        if r.status_code != 200:
            try:
                code = r.json().get("code", "unknown")
            except ValueError:
                code = "unknown"
            raise RegistryError(r.status_code, str(code))
        return r.json()  # type: ignore[no-any-return]

    def proposal(self, proposal_id: str) -> dict[str, Any]:
        return self._ok(self._call("GET", f"/proposals/{proposal_id}", self.bot))["proposal"]  # type: ignore[no-any-return]

    def alias(self, name: str) -> str | None:
        return self._ok(self._call("GET", f"/aliases/{self.agent}/{name}", self.bot))["release_id"]  # type: ignore[no-any-return]

    def approve(self, proposal_id: str, bearer: Bearer, candidate_hash: str) -> dict[str, Any]:
        return self._ok(self._call("POST", f"/proposals/{proposal_id}/approve", bearer.reveal(), json={"candidate_hash": candidate_hash}))

    def publish(self, proposal_id: str, bearer: Bearer, idempotency_key: str) -> str:
        return str(self._ok(self._call("POST", f"/proposals/{proposal_id}/publish", bearer.reveal(), headers={"Idempotency-Key": idempotency_key}))["release_id"])

    def promote(self, release_id: str, bearer: Bearer) -> None:
        self._ok(self._call("POST", f"/aliases/{self.agent}/prod", bearer.reveal(), json={"release_id": release_id, "reason": "demo explicit promote"}))


class LiveAuthorizer:
    """Durable intention -> command-authorization JWS from the real human issuer, bound-checked against the intention (`HumanAuthorizationPort`)."""

    def __init__(self, port: Any, tenant: str, actor: str = ACTOR) -> None:
        self.port, self.tenant, self.actor = port, tenant, actor

    def open(self, operation: str, target: dict[str, Any]) -> dict[str, str]:
        it = self.port.create_intention(tenant_id=self.tenant, actor_ref=self.actor, operation=operation, target=target)
        return {"intention_id": it.intention_id, "command_ref": it.command_ref}

    def authorize(self, intention_id: str) -> Any:
        return self.port.authorize(intention_id)  # SensitiveJws: redacted repr, `.reveal()` only for Core; has `.kid`


def build_live_ports(env: dict[str, Any], bridge: Any, tenant: str, out_dir: Path) -> tuple[LiveRegistry, LiveAuthorizer]:
    from codex_standin.human_port import HumanAuthorizationPort, IntentionStore, PodmanExecTransport
    from codex_standin.stack import CONNECTION, PODMAN, podman, project
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from local_identity.client import LocalIdentityClient, ServiceSigner
    from local_identity.keys import b64url_decode

    container = f"{project(env['namespace'])}-human-issuer-1"
    seed = json.loads(podman("exec", container, "cat", "/run/hi-keys/control-api-signer.json"))
    signer = ServiceSigner(seed["kid"], Ed25519PrivateKey.from_private_bytes(b64url_decode(seed["key"])))
    http = httpx.Client(transport=PodmanExecTransport(CONNECTION, container, podman=PODMAN), base_url="http://human-issuer:8083")
    client = LocalIdentityClient("http://human-issuer:8083", signer, tenant_id=tenant, http_client=http)
    store_dir = out_dir / ".human"
    store_dir.mkdir(parents=True, exist_ok=True)
    port = HumanAuthorizationPort(IntentionStore(store_dir / "intentions.sqlite3"), client)
    issued = bridge.call("POST", "/core-credentials/issue", "credentials", tenant, json={"tenant_id": tenant, "role": "constructor", "purpose": "registry_write"})
    if issued.status_code != 200:
        raise RuntimeError(f"bot credential for registry reads not issued ({issued.status_code})")
    runtime = f"http://127.0.0.1:{env['ports']['runtime']}"
    return LiveRegistry(runtime, issued.json()["jws"]), LiveAuthorizer(port, tenant)
