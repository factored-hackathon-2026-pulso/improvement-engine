"""Pin bump 86a7674 -> 789d6c8 (N-01..N-11, PR #26): explicit composition decisions, closed-list drift.

Non-PG: config parsing and compat. PG-gated: the composed app (export off by default, `/version` sha, key reload)."""

from __future__ import annotations


import pytest

from pulso_core_runtime import PIN_SHA
from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.compat import PIN_SYMBOLS, PinDrift, assert_compat
from pulso_core_runtime.errors import RuntimeConfigError

pytestmark = pytest.mark.runtime


# ---- non-PG ------------------------------------------------------------------------------------------------------
def test_keys_reload_seconds_is_explicit_and_validated() -> None:
    assert runtime_main.keys_reload_seconds({}) == 5.0
    assert runtime_main.keys_reload_seconds({"PULSO_KEYS_RELOAD_SECONDS": "0"}) == 0.0
    assert runtime_main.keys_reload_seconds({"PULSO_KEYS_RELOAD_SECONDS": "15"}) == 15.0
    for bad in ("-1", "abc", "86400"):
        with pytest.raises(RuntimeConfigError):
            runtime_main.keys_reload_seconds({"PULSO_KEYS_RELOAD_SECONDS": bad})


def test_synthesised_args_carry_the_reload_interval() -> None:
    paths = runtime_main.factory_paths({})
    assert runtime_main.synthesise_args({}, paths).keys_reload_seconds == 5.0
    assert runtime_main.synthesise_args({"PULSO_KEYS_RELOAD_SECONDS": "0"}, paths).keys_reload_seconds == 0.0


def test_export_is_off_unless_explicitly_enabled() -> None:
    assert runtime_main.export_enabled({}) is False
    assert runtime_main.export_enabled({"PULSO_CORE_EXPORT_ENABLED": "0"}) is False
    assert runtime_main.export_enabled({"PULSO_CORE_EXPORT_ENABLED": "1"}) is True


def test_bad_reload_value_exits_2_before_composing() -> None:
    import io
    err = io.StringIO()
    code = runtime_main.run([], env={"PULSO_KEYS_RELOAD_SECONDS": "soon"}, stderr=err, serve=lambda *a, **k: None)
    assert code == 2 and "PULSO_KEYS_RELOAD_SECONDS" in err.getvalue()


def test_compat_covers_the_new_symbols_and_detects_drift_on_each() -> None:
    names = {(m, n) for m, n, _, _ in PIN_SYMBOLS}
    for sym in (("agent_core.adapters.identity_keys", "ReloadingIdentityVerifier"),
                ("agent_core.ports.export", "RunExport"), ("agent_core.registry.models", "ReleaseSettings"),
                ("agent_core.registry.models", "RELEASE_SETTINGS"), ("agent_core.registry.http", "problem_response")):
        assert sym in names, sym
    assert_compat()
    with pytest.raises(PinDrift):
        assert_compat((("agent_core.composition.serve", "build_api_deps", "func", ("build_sha_renamed",)),))
    with pytest.raises(PinDrift):
        assert_compat((("agent_core.registry", "RegistryService", "class", ("no_such_method",)),))
    with pytest.raises(PinDrift):
        assert_compat((("agent_core.registry.models", "NO_SUCH_CONST", "const", ()),))


def test_version_info_reports_reload_error_type_only() -> None:
    class V:
        last_reload_error = "SchemaError"

    class Ports:
        verifier = V()

    info = runtime_main.version_info({"PULSO_SHA": "x"}, ["d"], Ports())()
    assert info["keys_reload_error"] == "SchemaError" and info["agent_core_sha"] == PIN_SHA
    assert runtime_main.version_info({}, [], None)()["keys_reload_error"] is None
