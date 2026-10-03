"""FIRST RED of the e2e-core hardening pass: the harness no longer works around defects fixed upstream (db grants,
exporter key issuer, runtime service-key overlay, LLM overlay image) and stays SHA-agnostic."""

from __future__ import annotations

import json
from pathlib import Path

import yaml

from codex_standin import PIN_SHA, report, stack

REPO = Path(__file__).resolve().parents[3]


def test_pin_sha_is_read_from_the_manifest_never_hardcoded() -> None:
    manifest = yaml.safe_load((REPO / "agent-core-assets" / "manifest.yaml").read_text(encoding="utf-8"))
    assert PIN_SHA == manifest["pin"]["sha"]
    assert "86a7674" not in (REPO / "e2e-core" / "run.ps1").read_text(encoding="utf-8")


def test_grants_workaround_is_gone() -> None:
    assert not hasattr(stack, "apply_grants_workaround") and not hasattr(stack, "GRANTS_WORKAROUND")


def test_no_overlay_image_the_stack_forwards_gateway_pair_and_budgets_via_core_env(tmp_path: Path) -> None:
    keys = stack.prepare("claude-e2e-1", tmp_path)
    assert not (tmp_path / "context").exists() and not hasattr(stack, "overlay")
    assert "service_seed" not in keys  # the control-api seed is read from the stack volume after start
    lines = stack.core_env_lines()
    assert any(x.startswith("PULSO_EVAL_BUDGETS_JSON={") and "bud-e2e" in x for x in lines)
    compose = (REPO / "local" / "core" / "compose.core.yaml").read_text(encoding="utf-8")
    for var in ("AGENTCORE_LLM_GATEWAY_URL", "AGENTCORE_LLM_GATEWAY_TOKEN", "PULSO_EVAL_BUDGETS_JSON"):
        assert var + ": ${" + var in compose, var


def test_llm_gateway_settings_use_the_agent_core_gateway_env_pair() -> None:
    lines = stack.core_env_lines()
    assert any(line.startswith("AGENTCORE_LLM_GATEWAY_TOKEN=") for line in lines)
    url = next(line for line in lines if line.startswith("AGENTCORE_LLM_GATEWAY_URL="))
    assert url.split("=", 1)[1].startswith("http://e2e-fixtures:") and not any("LLM_ENDPOINTS" in x for x in lines)


def test_scripted_llm_double_speaks_the_llm_gateway_generate_protocol() -> None:
    from fastapi.testclient import TestClient

    from codex_standin.fixtures_app import World, create_app
    from codex_standin.jwtsvc import KeyRing
    world = World(KeyRing({}))
    world.llm_rules = [{"id": "r", "match": {"system_contains": "research stage"}, "repeat_last": False,
                        "responses": [{"kind": "final", "output": {"ok": 1}}]}]
    client = TestClient(create_app(world))
    body = {"prompt": "You are a research stage.", "inputs": {"goal": "g"}, "schema": {}, "profile": {"model": "m"},
            "labels": {}}
    assert client.post("/llm/v1/generate", json=body).status_code == 401  # bearer required
    r = client.post("/llm/v1/generate", json=body, headers={"Authorization": "Bearer t"})
    assert r.status_code == 200, r.text
    out = r.json()
    assert out["output"] == {"kind": "final", "output": {"ok": 1}} and out["usage_known"] is True
    assert isinstance(out["cost_usd"], str) and isinstance(out["tokens_in"], int) and out["model"]
    miss = client.post("/llm/v1/generate", json=body, headers={"Authorization": "Bearer t"})
    assert miss.status_code == 500 and world.llm_unscripted == 1  # unscripted: counted, fails closed


def test_ring_trusts_the_stack_exporter_keys_as_issued_without_issuer_override() -> None:
    cb = {"kid": "cb", "key": stack.jwtsvc.seed_of(stack.jwtsvc.Ed25519PrivateKey.generate())}
    svc = {"keys": {"exporter-control-api": {"iss": "core-bridge", "aud": "control-api", "key": "AAAA"},
                    "control-api-core-bridge": {"iss": "control-api", "aud": "core-bridge", "key": "BBBB"}}}
    trust = {"keys": {"x-1": {"iss": "core-bridge", "aud": "lab-broker", "key": "CCCC"}}}
    out = stack.build_verify_keys(cb, svc, trust)
    assert out["ingest"]["keys"] == {"exporter-control-api": svc["keys"]["exporter-control-api"]}
    assert out["ring"]["x-1"] == ["core-bridge", "lab-broker", "CCCC"]


def test_report_lists_only_the_remaining_doubles_and_gaps() -> None:
    pieces = report.declared_doubles(["llm:scripted", "control-api", "lab-broker", "bank", "ingest"])
    kinds = {d["piece"] for d in pieces}
    assert kinds == {"llm:scripted", "control-api", "lab-broker", "bank", "ingest", "codex-standin"}
    assert not any("workaround" in d["piece"] for d in pieces)
    codes = {g["code"] for g in report.KNOWN_STACK_GAPS}
    assert not codes & {"core_app_engine_table_grants_missing", "eval_sequences_grant_missing",
                        "no_control_api_to_bridge_service_key", "exporter_key_issuer_mismatch",
                        "llm_env_not_passed_through_compose"}
    assert not codes & {"eval_budgets_not_passed_through_compose", "llm_gateway_env_not_passed_through_compose"}
