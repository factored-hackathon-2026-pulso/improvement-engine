"""Our system as a llm-gateway CONSUMER: the gateway is the scripted gateway-protocol double (declared). Asserts only OUR
behaviour: no provider keys in the runtime, gateway outage fails the task stage closed (never a silent success), and the
consumer token / prompt content do not leak into runtime logs, receipts or exports."""

from __future__ import annotations

import hashlib
import json
import re
from typing import Any

import pytest

from codex_standin.engine import RESEARCH, TENANT, hypotheses_output
from codex_standin.stack import podman, project, secret_of
from helpers import tag

pytestmark = pytest.mark.live
NOT_OK = {"unknown", "manual_reconcile", "terminal_failed", "failed", "denied", "dependency_unavailable"}


def _scout(e: Any, n: str, logical: str, briefing: str | None = None) -> Any:
    return e.stage("scout", f"job-g-{n}", logical, "pulso-scout", {"briefing_ref": briefing or f"wiki/g-{n}.md"},
                   memory_snapshot_ref=f"mem-{n}", extract_manifest_ref=f"ex-{n}")


def test_runtime_holds_no_provider_keys(stack: Any, effect: Any) -> None:
    ns = stack.env["namespace"]
    raw = podman("inspect", f"{project(ns)}-core-runtime-1", "--format", "{{json .Config.Env}}")
    names = sorted(x.split("=", 1)[0] for x in json.loads(raw))
    bad = [k for k in names if (re.search(r"(_API_KEY$|^LLM_KEY|PROVIDER)", k) or k in {"LLM_ENDPOINTS", "PULSO_LLM_API_KEY"})
           and k != "AGENTCORE_JEV_API_KEY"]  # JEV key: documented exception until the gateway carries JEV
    assert bad == [], bad
    assert "AGENTCORE_LLM_GATEWAY_URL" in names and "AGENTCORE_LLM_GATEWAY_TOKEN" in names
    effect("runtime_env_names_checked", len(names))


def test_gateway_outage_fails_the_stage_closed_and_recovers_with_a_new_attempt(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    e.configure(llm_replace=True, llm_rules=[])
    e.script_stage_model(f"scout-g-{n}", RESEARCH, hypotheses_output(TENANT))
    e.configure(faults={"llm": ["unavailable"] * 6})
    try:
        s = _scout(e, n, "outage")
    finally:
        e.configure(faults={"llm": []})
    # Current behaviour accepted: fail closed. Tighten to `dependency_unavailable` when that outcome lands.
    assert s.out["state"] != "terminal_ok" and s.out.get("outcome") != "completed", s.out
    facts = e.facts(s.out["core_run_id"]) if s.out.get("core_run_id") else {}
    assert "pulso_hypotheses" not in facts, facts  # no hypotheses were invented (no silent degradation)
    again = _scout(e, n, "outage-retry")  # a new logical key is a new attempt: the gateway is back
    assert again.out["state"] == "terminal_ok", again.out
    effect("gateway_outage_state", s.out["state"])


def test_consumer_token_and_prompt_content_do_not_leak(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    ns = stack.env["namespace"]
    token = secret_of(ns, "AGENTCORE_LLM_GATEWAY_TOKEN")
    canary = f"CANARY-INPUT-{n}"
    e.configure(llm_replace=True, llm_rules=[])
    e.script_stage_model(f"scout-c-{n}", RESEARCH, hypotheses_output(TENANT))
    s = _scout(e, n, "canary", briefing=f"wiki/{canary}.md")
    assert s.out["state"] == "terminal_ok", s.out
    st = e.state()
    seen = st["llm_seen"]
    assert seen and all(x["auth_sha256"] == hashlib.sha256(f"Bearer {token}".encode()).hexdigest() for x in seen)
    reached = any(canary in x["user"] or canary in x["system"] for x in seen)
    assert reached, "the briefing reference must reach the model (it is the model's input)"
    surfaces = {
        "runtime_logs": podman("logs", f"{project(ns)}-core-runtime-1"),
        "exporter_logs": podman("logs", f"{project(ns)}-core-exporter-1"),
        "version": json.dumps(stack.bridge.version()),
        "exports": json.dumps(st.get("ingest", {}), default=str),
        "control_api_requests": json.dumps([r for r in st["requests"]], default=str),
        "receipts": json.dumps(stack.runtime_db.rows("select r::text from pulso_bridge.receipts r where tenant_id=%s",
                                                     TENANT), default=str),
    }
    for name, text in surfaces.items():
        assert token not in text, f"consumer token leaked into {name}"
        if name != "receipts":  # receipts keep the stage input (the briefing ref) by design
            assert canary not in text, f"prompt content leaked into {name}"
    effect("leak_surfaces_checked", sorted(surfaces))
