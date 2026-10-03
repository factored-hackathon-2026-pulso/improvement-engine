"""Pin-bump review follow-ups: NF-04 readiness on key reload errors, build sha validation, no placeholder token."""

from __future__ import annotations

import io
from pathlib import Path
from types import SimpleNamespace

import pytest
from pulso_core_runtime import PIN_SHA
from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.llm.config import parse_llm_config
from pulso_core_runtime.readiness import key_files_check

pytestmark = pytest.mark.runtime
GW = {"AGENTCORE_LLM_GATEWAY_URL": "http://llm-gateway.test:8080", "AGENTCORE_LLM_GATEWAY_TOKEN": "tok-ok"}


def _run(env: dict[str, str]) -> tuple[int, str]:
    err = io.StringIO()
    return runtime_main.run([], env=env, stderr=err, serve=lambda *a, **k: None), err.getvalue()


def test_key_files_readiness_fails_when_a_verifier_reports_a_reload_error(tmp_path: Path) -> None:  # NF-04 first RED
    f = tmp_path / "k.json"
    f.write_text("{}")
    healthy = SimpleNamespace(last_reload_error=None)
    broken = SimpleNamespace(last_reload_error="JSONDecodeError")
    assert key_files_check([f], verifiers=lambda: [healthy, None])() is True
    assert key_files_check([f], verifiers=lambda: [healthy, broken])() is False  # staff or identity
    assert key_files_check([f])() is True


@pytest.mark.parametrize("token", ["unset", "UNSET", " unset "])
def test_placeholder_token_is_a_config_error(token: str) -> None:
    cfg, problems = parse_llm_config({**GW, "AGENTCORE_LLM_GATEWAY_TOKEN": token})
    assert cfg is None and "AGENTCORE_LLM_GATEWAY_TOKEN" in problems[0]
    code, err = _run({**GW, "AGENTCORE_LLM_GATEWAY_TOKEN": token})
    assert code == 2 and "AGENTCORE_LLM_GATEWAY_TOKEN" in err


def test_core_sha_must_equal_the_pin() -> None:
    code, err = _run({**GW, "PULSO_CORE_SHA": "f" * 40})
    assert code == 2 and "pulso:runtime_config_invalid" in err and "PULSO_CORE_SHA" in err
    _, err = _run({**GW, "PULSO_CORE_SHA": PIN_SHA})
    assert "PULSO_CORE_SHA" not in err
