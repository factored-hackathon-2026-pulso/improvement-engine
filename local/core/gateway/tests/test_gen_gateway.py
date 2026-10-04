"""M1: real llm-gateway wiring is generated config only: env references, pinned SHA, alias -> shim, stand-in fallback."""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

import pytest

GATEWAY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GATEWAY))

import gen_gateway as gg  # noqa: E402
import profile_check as pc  # noqa: E402


def test_pin_is_a_full_sha_matching_facts() -> None:
    assert re.fullmatch(r"[0-9a-f]{40}", gg.GATEWAY_SHA)
    facts = json.loads((GATEWAY.parents[2] / "docs/reports/gates/facts.json").read_text("utf-8"))
    assert gg.GATEWAY_SHA == facts["llm_gateway"]["main_sha"]


def test_endpoints_alias_points_at_shim_without_key_value() -> None:
    eps = gg.llm_endpoints()
    assert set(eps) == {"pulso-evolution-llm"}
    assert eps["pulso-evolution-llm"]["base_url"] == "http://roleplay-llm:8640/v1"
    assert eps["pulso-evolution-llm"]["api_key_env"] == "ROLEPLAY_LLM_API_KEY"
    assert pc.is_internal_url(eps["pulso-evolution-llm"]["base_url"]) or "roleplay-llm" in eps["pulso-evolution-llm"]["base_url"]


def test_consumers_use_env_references_only() -> None:
    c = gg.gateway_consumers()
    assert c == {"pulso-core": {"token_env": "GATEWAY_TOKEN_PULSO_CORE"}}


def test_profile_matches_registry_price_and_model() -> None:
    p = gg.generate_profile()
    assert p == {"endpoint_alias": "pulso-evolution-llm", "model": "external-reasoning-model",
                 "price": {"input_per_mtok": "1", "output_per_mtok": "4"}}


def test_env_template_has_no_secret_values() -> None:
    text = gg.env_template()
    for line in text.splitlines():
        if line.startswith("#") or not line.strip():
            continue
        k, _, v = line.partition("=")
        if k.endswith(("_TOKEN_PULSO_CORE", "API_KEY")) or k.startswith("GATEWAY_TOKEN"):
            assert v == "", line
    assert "LLM_ENDPOINTS=" in text and "GATEWAY_CONSUMERS=" in text
    assert not pc._SECRET_VALUE.search(text)


def test_overlay_is_static_valid_and_internal_only() -> None:
    import yaml

    doc = yaml.safe_load((GATEWAY / "compose.gateway-real.yaml").read_text("utf-8"))
    svc = doc["services"]
    assert set(svc) == {"llm-gateway", "roleplay-llm", "core-runtime"}
    for s in (svc["llm-gateway"], svc["roleplay-llm"]):
        assert s["networks"] == ["pulso-gw-e0"]
        assert "ports" not in s
    assert gg.GATEWAY_SHA in svc["llm-gateway"]["build"]["context"]
    env = svc["llm-gateway"]["environment"]
    assert env["GATEWAY_TOKEN_PULSO_CORE"] == "${GATEWAY_TOKEN_PULSO_CORE:?set}"
    assert not pc._SECRET_VALUE.search(json.dumps(doc))
    assert doc["networks"]["pulso-gw-e0"]["internal"] is True


def test_overlay_matches_generator() -> None:
    assert (GATEWAY / "compose.gateway-real.yaml").read_text("utf-8") == gg.overlay_yaml()


def test_stand_in_when_image_absent() -> None:
    assert gg.gateway_double(image_present=False) == "gateway=stand-in"
    assert gg.gateway_double(image_present=True) == f"gateway=real@{gg.GATEWAY_SHA[:12]}"


def test_runbook_mentions_pin_and_fallback() -> None:
    t = (GATEWAY / "RUNBOOK.md").read_text("utf-8")
    assert gg.GATEWAY_SHA in t and "gateway=stand-in" in t and "PULSO_GATEWAY_SMOKE" in t


@pytest.mark.skipif(os.environ.get("PULSO_GATEWAY_SMOKE") != "1", reason="live smoke: needs containers")
def test_live_smoke_alias_answers() -> None:
    base = os.environ.get("PULSO_GATEWAY_URL", "http://127.0.0.1:8080")
    token = os.environ["GATEWAY_TOKEN_PULSO_CORE"]
    body = json.dumps({"prompt": "ping", "inputs": {}, "profile": {**gg.generate_profile(), "temperature": 0,
                       "max_tokens": 16, "structured": "prompted"}}).encode()
    req = urllib.request.Request(base + "/v1/generate", body, {"Authorization": f"Bearer {token}",
                                 "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            assert r.status == 200
    except urllib.error.HTTPError as e:  # pragma: no cover
        pytest.fail(f"alias returned {e.code}")


def test_core_runtime_can_reach_gateway_on_internal_network() -> None:
    import yaml

    core = yaml.safe_load(gg.overlay_yaml())["services"]["core-runtime"]
    assert "pulso-gw-e0" in core["networks"] and "core-net" in core["networks"]
    assert core["environment"]["AGENTCORE_LLM_GATEWAY_URL"] == "http://llm-gateway:8080"
    assert core["environment"]["AGENTCORE_LLM_GATEWAY_TOKEN"] == "${GATEWAY_TOKEN_PULSO_CORE:?set}"
