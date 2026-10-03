"""Demo steps 8-9 against the REAL runtime image and the REAL local human issuer.

After the shared pipeline produced a frozen + evaluated candidate, the stand-in plays Codex's `HumanAuthorizationPort`:
durable single-use intention -> command-authorization JWS from the real `human-issuer` service (internal only, reached
through `podman exec`) -> `assert_bound` with the intention's values -> the SAME JWS bytes to Core's registry
(`/v1/registry`) for approve -> publish -> promote. Facts asserted:

* a bot credential cannot approve/publish (`forbidden_role`); a wrong hash is `candidate_changed`; a replayed approval
  is refused by Core (`illegal_transition`) and by the port (intention single use);
* human approval fixes operation/hash and is NOT publication: staging does not move; publish moves staging (confirmed
  by an alias read); prod moves only on the explicit promote;
* revoke needs an admin-issued JWS (a supervisor cannot even obtain one), a revoked release cannot be promoted;
* no JWS bytes appear in container logs, Postgres, the fixtures' recorded exports or the stand-in's own store.

Runs LAST (it publishes a release for `pulso-scout` and moves aliases)."""

from __future__ import annotations

import base64
import json
import subprocess
import uuid
from pathlib import Path
from typing import Any

import httpx
import pytest
from codex_standin import report
from codex_standin.engine import TENANT
from codex_standin.human_port import (
    HumanAuthorizationPort,
    IntentionConsumed,
    IntentionStore,
    PodmanExecTransport,
    SensitiveJws,
)
from codex_standin.stack import CONNECTION, PODMAN, REPO, podman, project
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from local_identity.client import (
    BindingMismatch,
    IssuerError,
    LocalIdentityClient,
    ServiceSigner,
    assert_bound,
)
from local_identity.contract import scan_remote_config
from local_identity.keys import b64url_decode

pytestmark = pytest.mark.live

SUPERVISOR, ADMIN = "local-supervisor", "local-admin"
AGENT = "pulso-scout"
REVEALED: list[str] = []  # every JWS this module obtained, for the leak scan (never printed)


class Flow:
    """Shared state of the sequential scenario (tests run in file order)."""

    port: HumanAuthorizationPort
    bot: str
    stack: Any
    pid: str
    cand_hash: str
    base_rel: str
    staging0: str | None
    prod0: str | None
    release: str | None = None
    approve_jws: SensitiveJws | None = None


F = Flow()


def core(method: str, path: str, bearer: str | SensitiveJws, **kw: Any) -> httpx.Response:
    token = bearer.reveal() if isinstance(bearer, SensitiveJws) else bearer
    headers = {"Authorization": f"Bearer {token}", **kw.pop("headers", {})}
    return httpx.request(method, f"{F.stack.runtime}/v1/registry{path}", headers=headers, timeout=60, **kw)


def human(operation: str, target: dict[str, Any], *, actor: str = SUPERVISOR) -> tuple[Any, SensitiveJws]:
    it = F.port.create_intention(tenant_id=TENANT, actor_ref=actor, operation=operation, target=target)
    jws = F.port.authorize(it.intention_id)
    REVEALED.append(jws.reveal())
    return it, jws


def proposal() -> dict[str, Any]:
    r = core("GET", f"/proposals/{F.pid}", F.bot)
    assert r.status_code == 200, r.text
    return r.json()["proposal"]  # type: ignore[no-any-return]


def alias(name: str) -> str | None:
    r = core("GET", f"/aliases/{AGENT}/{name}", F.bot)
    assert r.status_code == 200, r.text
    return r.json()["release_id"]  # type: ignore[no-any-return]


def proposal_target(rev: int, *, hash_: str | None = None) -> dict[str, Any]:
    return {"proposal_id": F.pid, "candidate_hash": hash_ or F.cand_hash, "expected_revision": rev}


def release_target(release_id: str, name: str) -> dict[str, Any]:
    return {"release_id": release_id, "agent_id": AGENT, "alias": name, "expected_revision": 0}


def claims_of(jws: str) -> dict[str, Any]:
    return json.loads(base64.urlsafe_b64decode(jws.split(".")[1] + "=="))  # type: ignore[no-any-return]


@pytest.fixture(scope="module")
def flow(stack: Any, pipeline: Any, tmp_path_factory: pytest.TempPathFactory) -> Flow:
    ns = stack.env["namespace"]
    container = f"{project(ns)}-human-issuer-1"
    seed = json.loads(podman("exec", container, "cat", "/run/hi-keys/control-api-signer.json"))
    signer = ServiceSigner(seed["kid"], Ed25519PrivateKey.from_private_bytes(b64url_decode(seed["key"])))
    http = httpx.Client(transport=PodmanExecTransport(CONNECTION, container, podman=PODMAN), base_url="http://human-issuer:8083")
    client = LocalIdentityClient("http://human-issuer:8083", signer, tenant_id=TENANT, http_client=http)
    F.stack = stack
    F.port = HumanAuthorizationPort(IntentionStore(tmp_path_factory.mktemp("intentions") / "intentions.sqlite3"), client)
    F.pid, F.cand_hash, F.base_rel = pipeline.proposal_id, pipeline.candidate_hash, pipeline.base_rel
    issued = stack.bridge.call("POST", "/core-credentials/issue", "credentials", TENANT,
                               json={"tenant_id": TENANT, "role": "constructor", "purpose": "registry_write"})
    assert issued.status_code == 200, issued.text
    F.bot = issued.json()["jws"]
    return F


def test_human_issuer_is_internal_only_a_labelled_double_and_core_trusts_only_its_public_key(stack: Any, flow: Flow,
                                                                                            effect: Any) -> None:
    ns = stack.env["namespace"]
    ctr = f"{project(ns)}-human-issuer-1"
    assert podman("port", ctr, check=False) == ""  # no host publication at all
    labels = json.loads(podman("inspect", ctr, "--format", "{{json .Config.Labels}}"))
    assert labels["com.pulso.role"] == "double" and labels["com.pulso.team"] == "claude"
    assert "container:human-issuer" in {d["piece"] for d in report.double_containers(ns)}  # doubles[] is honest
    staff = json.loads(podman("exec", f"{project(ns)}-core-runtime-1", "cat", "/run/pulso-keys/staff.json"))["principal_keys"]
    ident = json.loads(podman("exec", f"{project(ns)}-core-runtime-1", "cat", "/run/pulso-keys/identity.json"))["principal_keys"]
    human_kids = [k for k in staff if k.startswith("local-sim-human-")]
    assert len(human_kids) == 1 and "bridge-staff-local" in staff
    assert staff[human_kids[0]] != staff["bridge-staff-local"]  # bot and human never share a key
    assert human_kids[0] not in ident  # run principals never trust the human key
    private = podman("exec", f"{project(ns)}-core-runtime-1", "sh", "-c", "ls /run/pulso-keys")
    assert "human" not in private  # Core's key volume holds no human private material
    env = podman("inspect", f"{project(ns)}-core-runtime-1", "--format", "{{json .Config.Env}}")
    assert "local-sim" not in env and "simulated" not in env.lower()  # CAP-63 on the running runtime's config
    assert scan_remote_config(REPO) == []  # CAP-63 on the repository's remote (staging/prod) config
    effect("human_issuer_published_ports", 0)


def test_candidate_is_frozen_and_evaluated_before_any_human_step(stack: Any, flow: Flow) -> None:
    p = proposal()
    assert p["state"] == "evaluated" and p["candidate_hash"] == F.cand_hash
    run = stack.runtime_db.one("select verdict from reg_eval_runs where proposal_id=%s order by 1 limit 1", F.pid)
    assert run == "pass", run
    F.staging0, F.prod0 = alias("staging"), alias("prod")
    assert F.staging0 == F.prod0 == F.base_rel  # nothing is published yet


def test_a_bot_credential_cannot_approve_or_publish(flow: Flow, effect: Any) -> None:
    r = core("POST", f"/proposals/{F.pid}/approve", F.bot, json={"candidate_hash": F.cand_hash})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role", r.text
    r = core("POST", f"/proposals/{F.pid}/publish", F.bot, headers={"Idempotency-Key": "k-bot-" + uuid.uuid4().hex})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role", r.text
    assert proposal()["state"] == "evaluated"
    effect("bot_approve", "forbidden_role")


def test_a_wrong_hash_is_candidate_changed_and_leaves_the_proposal_untouched(flow: Flow, effect: Any) -> None:
    wrong = "0" * 64
    _, jws = human("approve", proposal_target(proposal()["rev"], hash_=wrong))  # authorised for a hash that is not the candidate
    r = core("POST", f"/proposals/{F.pid}/approve", jws, json={"candidate_hash": wrong})
    assert r.status_code == 409 and r.json()["code"] == "candidate_changed", r.text
    assert proposal()["state"] == "evaluated"
    effect("wrong_hash_approve", "candidate_changed")


def test_the_port_refuses_a_jws_for_another_operation_hash_or_revision(flow: Flow) -> None:
    rev = proposal()["rev"]
    it, jws = human("approve", proposal_target(rev))
    F.approve_jws = jws
    base = {"tenant_id": TENANT, "actor_ref": SUPERVISOR, "command_ref": it.command_ref, "operation": "approve",
            "target": proposal_target(rev), "challenge_ref": it.challenge_ref}
    assert_bound(jws.reveal(), **base)  # the intention's own values pass
    for over in ({"operation": "publish"}, {"target": proposal_target(rev, hash_="f" * 64)},
                 {"target": proposal_target(rev + 1)}, {"command_ref": "cmd-other"}, {"challenge_ref": "chal-other"}):
        with pytest.raises(BindingMismatch):
            assert_bound(jws.reveal(), **{**base, **over})
    with pytest.raises(IntentionConsumed):  # single use: the stand-in port never mints a second credential
        F.port.authorize(it.intention_id)


def test_approval_fixes_operation_and_hash_and_is_not_publication(stack: Any, flow: Flow, effect: Any) -> None:
    assert F.approve_jws is not None
    releases_before = stack.runtime_db.one("select count(*) from reg_releases")
    r = core("POST", f"/proposals/{F.pid}/approve", F.approve_jws, json={"candidate_hash": F.cand_hash})
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["actor"] == SUPERVISOR and body["decision"] == "approved" and body["candidate_hash"] == F.cand_hash
    assert proposal()["state"] == "approved"
    assert alias("staging") == F.staging0 and alias("prod") == F.prod0  # approval alone moves nothing
    assert stack.runtime_db.one("select count(*) from reg_releases") == releases_before
    effect("approval_moved_staging", alias("staging") != F.staging0)


def test_a_replayed_approval_is_rejected_by_core(flow: Flow, effect: Any) -> None:
    assert F.approve_jws is not None
    r = core("POST", f"/proposals/{F.pid}/approve", F.approve_jws, json={"candidate_hash": F.cand_hash})
    assert r.status_code == 409 and r.json()["code"] == "illegal_transition", r.text
    assert proposal()["state"] == "approved"
    effect("replayed_approval", "illegal_transition")


def test_publish_moves_staging_confirmed_by_alias_read_and_prod_stays(stack: Any, flow: Flow, effect: Any) -> None:
    _, jws = human("publish", proposal_target(proposal()["rev"]))
    key = "pub-" + uuid.uuid4().hex
    r = core("POST", f"/proposals/{F.pid}/publish", jws, headers={"Idempotency-Key": key})
    assert r.status_code == 200, r.text
    F.release = r.json()["release_id"]
    assert alias("staging") == F.release != F.staging0  # staging confirmed by an alias read, not by the 200
    assert alias("prod") == F.prod0  # exposure needs the explicit promote
    again = core("POST", f"/proposals/{F.pid}/publish", jws, headers={"Idempotency-Key": key})
    assert again.status_code == 200 and again.json()["release_id"] == F.release  # Core's own idempotency
    assert stack.runtime_db.one("select count(*) from reg_releases where release_id=%s", F.release) == 1
    effect("published_release", F.release)


def test_promote_moves_prod_only_when_explicit(flow: Flow, effect: Any) -> None:
    assert F.release
    _, jws = human("promote", release_target(F.release, "prod"))
    r = core("POST", f"/aliases/{AGENT}/prod", jws, json={"release_id": F.release, "reason": "e2e promote"})
    assert r.status_code == 200, r.text
    assert alias("prod") == F.release and alias("staging") == F.release
    effect("prod_after_explicit_promote", alias("prod") == F.release)


def test_revoke_needs_an_admin_issued_jws_and_a_revoked_release_cannot_be_promoted(flow: Flow, effect: Any) -> None:
    assert F.release
    # roll prod back first (Core refuses to revoke a release prod points to), through an explicit human promote
    _, back = human("promote", release_target(F.base_rel, "prod"))
    r = core("POST", f"/aliases/{AGENT}/prod", back, json={"release_id": F.base_rel, "reason": "e2e rollback"})
    assert r.status_code == 200 and alias("prod") == F.base_rel, r.text
    # a supervisor cannot even obtain a revoke authorization (and the failed intention is burned)
    it = F.port.create_intention(tenant_id=TENANT, actor_ref=SUPERVISOR, operation="revoke",
                                 target=release_target(F.release, "staging"))
    with pytest.raises(IssuerError) as err:
        F.port.authorize(it.intention_id)
    assert err.value.code == "pulso:role_not_allowed"
    # a supervisor's promote JWS is not an admin credential
    _, promote_jws = human("promote", release_target(F.release, "staging"))
    r = core("POST", f"/releases/{F.release}/revoke", promote_jws, json={"reason": "wrong credential"})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role", r.text
    # the admin-issued JWS revokes
    _, admin = human("revoke", release_target(F.release, "staging"), actor=ADMIN)
    assert claims_of(admin.reveal())["roles"] == ["constructor", "aprobador", "admin"] or "admin" in claims_of(admin.reveal())["roles"]
    r = core("POST", f"/releases/{F.release}/revoke", admin, json={"reason": "e2e revoke"})
    assert r.status_code == 200, r.text
    st = core("GET", f"/releases/{F.release}", F.bot)
    assert st.status_code == 200 and st.json()["status"] == "revoked"
    _, again = human("promote", release_target(F.release, "prod"))
    r = core("POST", f"/aliases/{AGENT}/prod", again, json={"release_id": F.release, "reason": "revoked"})
    assert r.status_code == 409 and r.json()["code"] == "illegal_transition", r.text
    assert alias("prod") == F.base_rel
    effect("revoke_requires_admin", True)


def test_no_jws_bytes_in_logs_exports_or_the_standin_store(stack: Any, flow: Flow, tmp_path_factory: pytest.TempPathFactory) -> None:
    assert len(REVEALED) >= 8
    needles = {part for jws in REVEALED for part in (jws.split(".")[1], jws.split(".")[2])}  # payload + signature
    ns = stack.env["namespace"]
    haystacks: dict[str, str] = {}
    for svc in ("core-runtime", "human-issuer", "platform-sim", "core-exporter", "core-postgres", "e2e-fixtures"):
        proc = subprocess.run([PODMAN, "--connection", CONNECTION, "logs", f"{project(ns)}-{svc}-1"], capture_output=True,
                              text=True, timeout=120, check=False)
        haystacks[f"log:{svc}"] = proc.stdout + proc.stderr
    haystacks["fixtures_state"] = json.dumps(stack.engine.state())  # recorded broker/ingest/export traffic
    pw = subprocess.run([PODMAN, "--connection", CONNECTION, "exec", f"{project(ns)}-core-postgres-1", "sh", "-c",
                         "pg_dump -U postgres core_runtime; pg_dump -U postgres core_eval"], capture_output=True,
                        text=True, timeout=300, check=False)
    assert pw.returncode == 0, pw.stderr[:200]
    haystacks["pg_dump"] = pw.stdout
    store = F.port.store._path  # the stand-in's durable intentions: no credential persisted
    haystacks["intentions_db"] = Path(store).read_bytes().decode("latin-1")
    # control: the haystacks are real (an empty log would make the scan vacuous)
    assert all(len(haystacks[k]) > 0 for k in ("log:core-runtime", "log:human-issuer", "log:core-postgres"))
    assert len(haystacks["pg_dump"]) > 100_000 and len(haystacks["intentions_db"]) > 0
    for name, text in haystacks.items():
        for needle in needles:
            assert needle not in text, f"JWS material leaked into {name}"
