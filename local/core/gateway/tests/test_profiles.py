"""DC0: gw-e0 has no key and no external route; gw-hosted never accepts E0 data."""

from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

GATEWAY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GATEWAY))

import profile_check as pc  # noqa: E402


def load(name: str) -> dict:
    return json.loads((GATEWAY / "profiles" / f"{name}.json").read_text("utf-8"))


def rules(violations) -> set[str]:
    return {v.rule for v in violations}


def test_shipped_profiles_pass() -> None:
    assert pc.check_profile(load("gw-e0")) == []
    assert pc.check_profile(load("gw-hosted")) == []
    assert pc.check_all(GATEWAY / "profiles") == []


def test_e0_profile_has_no_key_and_no_external_route() -> None:
    p = load("gw-e0")
    assert p["network"]["internal"] is True
    assert pc.collect_keys(p) == []
    assert all(pc.is_internal_url(u) for u in pc.collect_urls(p))


def test_e0_with_key_fails() -> None:
    p = load("gw-e0")
    p["api_key_env"] = "OPENAI_API_KEY"
    assert "e0_has_key" in rules(pc.check_profile(p))


def test_e0_with_inline_secret_fails_anywhere() -> None:
    p = load("gw-e0")
    p["upstream"]["headers"] = {"Authorization": "Bearer abc"}
    assert "e0_has_key" in rules(pc.check_profile(p))


def test_e0_with_external_route_fails() -> None:
    p = load("gw-e0")
    p["upstream"]["base_url"] = "https://api.example.com/v1"
    assert "e0_external_route" in rules(pc.check_profile(p))


def test_e0_network_must_be_internal() -> None:
    p = load("gw-e0")
    p["network"]["internal"] = False
    assert "network_not_internal" in rules(pc.check_profile(p))


def test_e0_fallback_route_external_fails() -> None:
    p = load("gw-e0")
    p["fallbacks"] = [{"base_url": "http://8.8.8.8:80"}]
    assert "e0_external_route" in rules(pc.check_profile(p))


def test_hosted_accepting_e0_fails() -> None:
    for cls in ("E0", "csv", "original-treated"):
        p = copy.deepcopy(load("gw-hosted"))
        p["data_classes"].append(cls)
        assert "hosted_accepts_restricted" in rules(pc.check_profile(p)), cls


def test_hosted_key_must_be_env_reference_not_value() -> None:
    p = load("gw-hosted")
    p["api_key_env"] = "sk-" + "a" * 24
    assert "secret_value" in rules(pc.check_profile(p))


def test_hosted_requires_key_reference() -> None:
    p = load("gw-hosted")
    p["api_key_env"] = None
    assert "hosted_missing_key_ref" in rules(pc.check_profile(p))


def test_route_decision() -> None:
    e0, hosted = load("gw-e0"), load("gw-hosted")
    assert pc.route_allowed(e0, "E0") is True
    assert pc.route_allowed(hosted, "E0") is False
    assert pc.route_allowed(hosted, "synthetic") is True
    assert pc.route_allowed(e0, "unknown-class") is False


def test_internal_url_classifier() -> None:
    for u in ("http://llm-local:8080", "http://127.0.0.1:1", "http://10.1.2.3/x", "http://gw.internal/x", "http://localhost"):
        assert pc.is_internal_url(u), u
    for u in ("https://api.openai.com", "http://8.8.8.8", "http://example.com"):
        assert not pc.is_internal_url(u), u


def test_compose_fragment_declares_internal_network() -> None:
    text = (GATEWAY / "compose.gateway.yaml").read_text("utf-8")
    assert pc.compose_has_internal_network(text, "pulso-gw-e0")
    assert not pc.compose_has_internal_network(text, "pulso-gw-hosted")


def test_e0_external_route_under_non_url_key_or_fallback_string_fails() -> None:
    base = load("gw-e0")
    for mut in ({"upstream": {"kind": "local", "endpoint": "https://api.openai.com/v1"}},
                {"fallbacks": ["https://api.openai.com/v1"]},
                {"upstream": {"kind": "external", "base_url": "http://llm-local:8080"}}):
        p = {**base, **mut}
        assert pc.check_profile(p), mut


def test_restricted_class_never_on_non_local_upstream() -> None:
    p = {"name": "x", "data_classes": ["csv"], "network": {"internal": False},
         "api_key_env": "K", "upstream": {"kind": "external", "base_url": "https://a.example.com"}}
    assert pc.check_profile(p)
