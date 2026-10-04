"""Pin c814c2b (agent-core PR #30): `resolve_ports` reads `args.lang_thresholds` with attribute access, but the runtime
builds its argparse Namespace by hand (`synthesise_args`). These tests run Core's REAL `serve_ports` code (no stubbed
`resolve`), so a flag Core adds and we forget fails here and not at container start. Dual-pin safe: on a pin without
the flag the lang-thresholds test skips and the parser-coverage test still holds."""

from __future__ import annotations

import argparse

import pytest

from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.factories import DEFAULT_PATHS, FACTORIES

pytestmark = pytest.mark.runtime


def _args() -> argparse.Namespace:
    return runtime_main.synthesise_args({}, dict(DEFAULT_PATHS))


def _core_serve_flags() -> set[str]:
    from agent_core.composition.serve_ports import add_serve_parser

    top = argparse.ArgumentParser()
    add_serve_parser(top.add_subparsers(dest="cmd"))
    serve = top.parse_args(["serve"])
    return set(vars(serve)) - {"cmd"}


def test_synthesised_args_carry_every_flag_of_the_real_serve_parser() -> None:
    missing = _core_serve_flags() - set(vars(_args()))
    assert not missing, f"synthesise_args lacks Core serve flags: {sorted(missing)}"


def test_synthesised_args_pass_the_real_lang_thresholds_reader() -> None:
    serve_ports = pytest.importorskip("agent_core.composition.serve_ports")
    if not hasattr(serve_ports, "_lang_thresholds"):
        pytest.skip("pin without --lang-thresholds (before c814c2b)")
    problems: list[str] = []
    assert serve_ports._lang_thresholds(_args(), {}, problems) == {}  # feature off: detection never switches language
    assert problems == []


def test_factories_still_come_from_the_runtime_paths_not_the_parser_defaults() -> None:
    ns = _args()
    for attr, name in FACTORIES:
        assert getattr(ns, attr) == DEFAULT_PATHS[name]
    assert ns.registry_api is True and ns.dsn is None and ns.lang_thresholds is None
