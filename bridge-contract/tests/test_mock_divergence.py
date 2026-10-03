"""The mock-vs-contract divergence report is a checked-in artifact; it must be regenerated when either side moves."""

from __future__ import annotations

import divergence


def test_divergence_report_is_current() -> None:
    assert divergence.OUT.is_file(), "run: python bridge-contract/divergence.py"
    assert divergence.OUT.read_text(encoding="utf-8") == divergence.render(), (
        "platform-sim bridge_mock or the contract changed: regenerate with python bridge-contract/divergence.py "
        "and review the diff (new divergences need a known_different entry or a mock fix)")


def test_every_mock_known_different_group_is_a_registered_divergence() -> None:
    from conformance.known_different import DIVERGENCES, MOCK
    assert {div for div, _ in MOCK.values()} <= set(DIVERGENCES)
