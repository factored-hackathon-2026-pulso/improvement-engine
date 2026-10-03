"""Steps 8-9: the approval flow (intention -> human gate -> authorization -> approve -> publish -> staging alias read), offline fakes."""
import json
import threading
import time

import pytest

from pulso_demo import decide, human_flow
from pulso_demo.decision_hook import DecisionRequest
from pulso_demo.offline_ports import OfflineAuthorizer, OfflineRegistry

HASH = "c" * 64


def req():
    return DecisionRequest(run_id="run-demo", proposal_id="prop-1", candidate_hash=HASH, operation="approve")


def flow(tmp_path, gate=None, promote=False, registry=None, authorizer=None):
    reg = registry or OfflineRegistry("prop-1", HASH, base_release="rel-base")
    return human_flow.ApprovalFlow(reg, authorizer or OfflineAuthorizer(), gate or human_flow.ScriptedGate(), promote=promote), reg


def events(res):
    return [e["event"] for e in res.detail["trail"]]


def test_scripted_flow_requests_approves_publishes_and_confirms_staging_by_alias_read(tmp_path):
    f, reg = flow(tmp_path)
    res = f.run(req())
    assert events(res) == ["decision_requested", "human_decided", "approved", "published", "staging_confirmed"]
    assert res.state == "approved" and res.detail["stage"] == "staging_confirmed"
    assert res.detail["release_id"] == reg.aliases["staging"] != "rel-base" and reg.aliases["prod"] == "rel-base"
    assert res.detail["simulated_human"] is True and "SIMULATED" in res.detail["trail"][1]["label"]


def test_approval_is_not_publication_the_registry_is_unchanged_between_the_two(tmp_path):
    f, reg = flow(tmp_path)
    f.run(req())
    ap = next(i for i, c in enumerate(reg.calls) if c[0] == "approve")
    assert [c[0] for c in reg.calls[ap:ap + 3]][:2] == ["approve", "proposal"]
    assert reg.staging_at_approve == "rel-base"  # staging had not moved when the approval landed


def test_operation_and_hash_are_fixed_by_the_intention_not_by_the_caller(tmp_path):
    auth = OfflineAuthorizer()
    f, reg = flow(tmp_path, authorizer=auth)
    f.run(req())
    ops = [(i["operation"], i["target"]["candidate_hash"] if "candidate_hash" in i["target"] else None) for i in auth.intentions]
    assert ops == [("approve", HASH), ("publish", HASH)]
    assert reg.approved_hash == HASH


def test_prod_is_not_promoted_without_the_explicit_flag_and_is_with_it(tmp_path):
    f, reg = flow(tmp_path)
    assert f.run(req()).detail["promoted"] is False and reg.aliases["prod"] == "rel-base"
    f2, reg2 = flow(tmp_path, promote=True)
    res = f2.run(req())
    assert events(res)[-1] == "promoted" and reg2.aliases["prod"] == reg2.aliases["staging"] and res.detail["stage"] == "promoted"


def test_a_changed_candidate_is_refused_before_any_human_step(tmp_path):
    reg = OfflineRegistry("prop-1", "d" * 64, base_release="rel-base")
    f, _ = flow(tmp_path, registry=reg)
    res = f.run(req())
    assert res.state == "pending" and res.detail["stage"] == "failed" and res.detail["error"].startswith("candidate_changed")
    assert not any(c[0] == "approve" for c in reg.calls)


def test_publish_failure_leaves_approved_not_published_and_never_claims_staging(tmp_path):
    reg = OfflineRegistry("prop-1", HASH, base_release="rel-base", fail_publish=True)
    f, _ = flow(tmp_path, registry=reg)
    res = f.run(req())
    assert res.state == "approved" and res.detail["stage"] == "approved_not_published" and res.detail["release_id"] is None
    assert events(res)[-2:] == ["approved", "publish_failed"] and reg.aliases["staging"] == "rel-base"


def test_unconfirmed_staging_alias_is_a_failure_not_a_success(tmp_path):
    reg = OfflineRegistry("prop-1", HASH, base_release="rel-base", ignore_alias_move=True)
    f, _ = flow(tmp_path, registry=reg)
    res = f.run(req())
    assert res.detail["stage"] == "published_unconfirmed" and "staging_confirmed" not in events(res)


def test_rejection_by_the_human_publishes_nothing(tmp_path):
    f, reg = flow(tmp_path, gate=human_flow.ScriptedGate(decision="reject"))
    res = f.run(req())
    assert res.state == "rejected" and not [c for c in reg.calls if c[0] in ("approve", "publish")]


def test_no_credential_bytes_in_the_result(tmp_path):
    auth = OfflineAuthorizer()
    f, _ = flow(tmp_path, authorizer=auth)
    res = f.run(req())
    assert auth.issued and all(j not in json.dumps(res.detail) for j in auth.issued)


def test_manual_gate_waits_for_the_cli_approval_and_binds_it_to_the_request(tmp_path):
    gate = human_flow.ManualGate(tmp_path, timeout_s=20, poll_s=0.05)
    f, reg = flow(tmp_path, gate=gate)
    t = threading.Thread(target=lambda: setattr(f, "res", f.run(req())))
    t.start()
    for _ in range(100):
        if (tmp_path / human_flow.REQUEST_FILE).exists():
            break
        time.sleep(0.05)
    assert json.loads((tmp_path / human_flow.REQUEST_FILE).read_text("utf-8"))["candidate_hash"] == HASH
    assert not any(c[0] == "approve" for c in reg.calls)  # still waiting
    assert decide.main(["--out", str(tmp_path), "approve"]) == 0
    t.join(30)
    assert f.res.detail["stage"] == "staging_confirmed" and f.res.detail["simulated_human"] is False and f.res.detail["mode"] == "manual"


def test_manual_gate_times_out_into_pending_and_a_mismatching_decision_is_a_rejection(tmp_path):
    f, reg = flow(tmp_path, gate=human_flow.ManualGate(tmp_path, timeout_s=0.2, poll_s=0.05))
    res = f.run(req())
    assert res.state == "pending" and res.detail["stage"] == "requested" and events(res)[-1] == "human_timeout"

    def late_mismatch():  # written AFTER the request exists (a decision predating the request is discarded, see the stale-file test)
        while not (tmp_path / human_flow.REQUEST_FILE).exists():
            time.sleep(0.02)
        (tmp_path / human_flow.DECISION_FILE).write_text(json.dumps({"decision": "approve", "proposal_id": "prop-1", "candidate_hash": "e" * 64}), "utf-8")
    threading.Thread(target=late_mismatch, daemon=True).start()
    res2 = flow(tmp_path, gate=human_flow.ManualGate(tmp_path, timeout_s=5, poll_s=0.05))[0].run(req())
    assert res2.state == "rejected" and res2.detail["reason"] == "decision_file_mismatch"
    assert not (tmp_path / human_flow.DECISION_FILE).exists()  # consumed: a stale approval can never be replayed
    assert not [c for c in reg.calls if c[0] == "approve"]


@pytest.mark.parametrize("mode", ["scripted", "manual"])
def test_gate_factory_labels_the_mode(tmp_path, mode):
    g = human_flow.make_gate(mode, tmp_path, timeout_s=1)
    assert g.mode == mode and g.simulated is (mode == "scripted")


def test_stale_decision_file_from_a_previous_run_is_discarded_not_applied(tmp_path):
    (tmp_path / human_flow.DECISION_FILE).write_text(json.dumps({"decision": "approve", "proposal_id": "prop-1", "candidate_hash": HASH}), "utf-8")
    gate = human_flow.ManualGate(tmp_path, timeout_s=0.3, poll_s=0.05)
    res = flow(tmp_path, gate=gate)[0].run(req())
    assert res.detail["stage"] == "requested" and "approved" not in events(res)  # the pre-existing approval predates the request: ignored


def test_manual_run_with_no_decision_reports_awaiting_human_not_ok(tmp_path):
    from pulso_demo import driver
    assert driver.main(["--out", str(tmp_path), "--offline", "--human-mode", "manual", "--human-timeout", "0.2"]) == 0
    assert json.loads((tmp_path / "demo-report.json").read_text("utf-8"))["outcome"] == "awaiting_human_decision"


def test_outputs_never_contain_a_jws_or_private_material(tmp_path):
    from pulso_demo import driver
    driver.main(["--out", str(tmp_path), "--offline"])
    blob = "".join(p.read_text("utf-8") for p in tmp_path.glob("*.json"))
    import re
    assert not re.search(r"eyJ[A-Za-z0-9_-]{10,}\.", blob) and "PRIVATE KEY" not in blob and "bearer" not in blob.lower()
