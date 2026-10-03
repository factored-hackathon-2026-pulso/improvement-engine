"""First RED (plan 17.3.2): omitting one factory exits 2 naming it; full exit-2 matrix, no Postgres needed."""

from __future__ import annotations

import io

import pytest

from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.factories import FACTORY_NAMES

pytestmark = pytest.mark.runtime


def _run(env: dict[str, str]) -> tuple[int, str]:
    err = io.StringIO()
    code = runtime_main.run([], env=env, stderr=err, serve=lambda *a, **k: None)
    return code, err.getvalue()


@pytest.mark.parametrize("name", FACTORY_NAMES)
def test_omitting_a_factory_exits_2_naming_it(name: str) -> None:
    code, err = _run({f"PULSO_FACTORY_{name.upper().replace('-', '_')}": ""})
    assert code == 2
    assert "pulso:adapter_missing" in err
    assert name in err


def test_demo_flag_exits_2_before_resolve_ports() -> None:
    called: list[bool] = []
    err = io.StringIO()
    code = runtime_main.run([], env={"AGENTCORE_ALLOW_DEMO": "1"}, stderr=err,
                            serve=lambda *a, **k: called.append(True),
                            resolve=lambda *a, **k: called.append(True))
    assert code == 2 and "pulso:demo_double_in_real_mode" in err.getvalue()
    assert called == []


@pytest.mark.parametrize("name", FACTORY_NAMES)
def test_testing_path_rejected(name: str) -> None:
    code, err = _run({f"PULSO_FACTORY_{name.upper().replace('-', '_')}": "testing.serve_demo:tools"})
    assert code == 2
    assert "pulso:runtime_config_invalid" in err and name in err
    assert "testing.serve_demo" not in err or "rejected" in err
