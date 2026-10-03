"""WP-B first RED: fail-closed gateway config (exit 2 `pulso:runtime_config_invalid` naming the piece, never a value)
and the `llm_gateway` readiness probe (plain reachability, GET /healthz). No Postgres needed."""

from __future__ import annotations

import io
import json
from typing import Any

import httpx
import pytest

from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.llm.config import llm_doubles, parse_llm_config
from pulso_core_runtime.llm.probe import GatewayProbe, llm_gateway_check

from .gateway_double import GatewayDouble

SECRET = "SECRET-CONSUMER-TOKEN-do-not-print"
URL = "http://llm-gateway.test:8080"


def _run(env: dict[str, str]) -> tuple[int, str]:
    err = io.StringIO()
    code = runtime_main.run([], env=env, stderr=err, serve=lambda *a, **k: None)
    return code, err.getvalue()


def test_missing_gateway_env_exits_2_naming_both_pieces() -> None:  # the package's first RED
    code, err = _run({})
    assert code == 2
    assert "pulso:runtime_config_invalid" in err
    assert "AGENTCORE_LLM_GATEWAY_URL" in err and "AGENTCORE_LLM_GATEWAY_TOKEN" in err


def test_half_pair_names_only_the_missing_piece_and_never_prints_the_token() -> None:
    code, err = _run({"AGENTCORE_LLM_GATEWAY_TOKEN": SECRET})
    assert code == 2 and "AGENTCORE_LLM_GATEWAY_URL" in err and SECRET not in err
    assert "AGENTCORE_LLM_GATEWAY_TOKEN is" not in err
    code, err = _run({"AGENTCORE_LLM_GATEWAY_URL": URL})
    assert code == 2 and "AGENTCORE_LLM_GATEWAY_TOKEN" in err and "AGENTCORE_LLM_GATEWAY_URL is" not in err


@pytest.mark.parametrize("bad", ["ftp://gw", "llm-gateway:8080", "http://", "not a url"])
def test_bad_url_is_named_without_echoing_it(bad: str) -> None:
    code, err = _run({"AGENTCORE_LLM_GATEWAY_URL": bad, "AGENTCORE_LLM_GATEWAY_TOKEN": SECRET})
    assert code == 2 and "AGENTCORE_LLM_GATEWAY_URL" in err
    assert bad not in err.replace("AGENTCORE_LLM_GATEWAY_URL", "") and SECRET not in err


def test_explicit_disabled_mode_is_the_only_way_out_and_is_reported_as_a_double() -> None:
    cfg, problems = parse_llm_config({"PULSO_LLM_MODE": "disabled"})
    assert problems == [] and cfg is not None and cfg.mode == "disabled"
    assert any("disabled" in d for d in llm_doubles(cfg))
    _, problems = parse_llm_config({"PULSO_LLM_MODE": "off"})
    assert problems and "PULSO_LLM_MODE" in problems[0]
    _, err = _run({"PULSO_LLM_MODE": "disabled"})
    assert "AGENTCORE_LLM_GATEWAY" not in err  # passes the LLM check, fails later on something else


def test_valid_pair_parses_and_the_token_is_not_in_repr_or_doubles() -> None:
    cfg, problems = parse_llm_config({"AGENTCORE_LLM_GATEWAY_URL": URL, "AGENTCORE_LLM_GATEWAY_TOKEN": SECRET})
    assert problems == [] and cfg is not None and cfg.mode == "gateway" and cfg.url == URL
    assert SECRET not in repr(cfg) and SECRET not in json.dumps(llm_doubles(cfg))


def test_stage_policy_is_validated_and_optionally_required() -> None:
    base = {"AGENTCORE_LLM_GATEWAY_URL": URL, "AGENTCORE_LLM_GATEWAY_TOKEN": SECRET}
    good = {"scout": {"endpoint_alias": "pulso-scout-llm", "model": "m", "input_per_mtok": "0.10",
                      "output_per_mtok": "0.32", "max_tokens": 4000}}
    cfg, problems = parse_llm_config({**base, "PULSO_LLM_STAGE_POLICY_JSON": json.dumps(good)})
    assert problems == [] and cfg is not None and cfg.policy is not None and cfg.policy.stages == ("scout",)
    assert any("pinned" in d for d in llm_doubles(cfg))
    _, problems = parse_llm_config({**base, "PULSO_LLM_STAGE_POLICY_JSON": "{not json"})
    assert problems and "PULSO_LLM_STAGE_POLICY_JSON" in problems[0]
    bad = {"scout": {**good["scout"], "output_per_mtok": "2000000"}}
    _, problems = parse_llm_config({**base, "PULSO_LLM_STAGE_POLICY_JSON": json.dumps(bad)})
    assert problems and "PULSO_LLM_STAGE_POLICY_JSON" in problems[0] and "2000000" not in problems[0]
    cfg, problems = parse_llm_config(base)  # unpinned is allowed unless required
    assert problems == [] and cfg is not None and cfg.policy is None
    assert any("unpinned" in d for d in llm_doubles(cfg))
    _, problems = parse_llm_config({**base, "PULSO_LLM_POLICY_REQUIRED": "1"})
    assert problems and "PULSO_LLM_STAGE_POLICY" in problems[0]


def test_policy_file_wins_over_inline(tmp_path: Any) -> None:
    entry = {"endpoint_alias": "a", "model": "m", "input_per_mtok": "1", "output_per_mtok": "2", "max_tokens": 10}
    path = tmp_path / "policy.json"
    path.write_text(json.dumps({"verifier": entry}))
    cfg, problems = parse_llm_config({
        "AGENTCORE_LLM_GATEWAY_URL": URL, "AGENTCORE_LLM_GATEWAY_TOKEN": SECRET, "PULSO_LLM_STAGE_POLICY": str(path),
        "PULSO_LLM_STAGE_POLICY_JSON": json.dumps({"scout": entry})})
    assert problems == [] and cfg is not None and cfg.policy is not None and cfg.policy.stages == ("verifier",)


# --- readiness probe ---------------------------------------------------------------------------------------


class Clock:
    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


def test_probe_ok_when_the_gateway_answers_healthz_and_sends_no_token() -> None:
    double = GatewayDouble()
    probe = GatewayProbe(URL, SECRET, client=double.client(), clock=Clock())
    assert probe.check() is True and probe.state == "ok"
    sent = double.raw[0]
    assert sent.method == "GET" and sent.url.path == "/healthz" and "authorization" not in sent.headers
    assert double.requests == []


def test_probe_unreachable_and_5xx_are_not_ready() -> None:
    def boom(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("down")
    down = GatewayProbe(URL, "t", client=httpx.Client(transport=httpx.MockTransport(boom)), clock=Clock())
    assert down.check() is False and down.state == "unreachable"
    bad = GatewayProbe(URL, "t", client=httpx.Client(transport=httpx.MockTransport(
        lambda r: httpx.Response(502, json={}))), clock=Clock())
    assert bad.check() is False and bad.state == "unexpected_status"


def test_probe_is_cached_and_recovers_after_the_ttl() -> None:
    state = {"up": True}

    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200 if state["up"] else 503)
    seen: list[int] = []
    clock = Clock()
    client = httpx.Client(transport=httpx.MockTransport(lambda r: (seen.append(1), handler(r))[1]))
    probe = GatewayProbe(URL, None, client=client, clock=clock, ttl_s=15, fail_ttl_s=5)
    assert probe.check() and probe.check() and len(seen) == 1
    state["up"] = False
    clock.now += 16
    assert probe.check() is False and len(seen) == 2
    assert probe.check() is False and len(seen) == 2  # failure cached for the short ttl
    state["up"] = True
    clock.now += 6
    assert probe.check() is True


def test_readiness_check_is_named_llm_gateway() -> None:
    name, check = llm_gateway_check(URL, "tok-ok", client=GatewayDouble().client())
    assert name == "llm_gateway" and check() is True
