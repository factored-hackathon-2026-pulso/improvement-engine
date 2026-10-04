"""SNET spike: the probe must fail when the shim is unreachable and must type a call that outlives its limit."""

from __future__ import annotations

import threading

import pytest

import snet_probe as sp


@pytest.fixture
def shim():
    server = sp.make_server("127.0.0.1", 0)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{server.server_address[1]}"
    server.shutdown()


def test_reachability_and_skew(shim: str) -> None:
    r = sp.probe_reachability(shim)
    assert r["reachable"] is True and abs(r["skew_s"]) < 2 and r["rtt_s"] >= 0


def test_unreachable_shim_fails() -> None:
    r = sp.probe_reachability("http://127.0.0.1:9", timeout_s=1)
    assert r["reachable"] is False


def test_call_within_limit_is_measured(shim: str) -> None:
    r = sp.timed_call(shim, sleep_s=0.2, limit_s=5)
    # time.sleep and time.monotonic have ~15 ms granularity on Windows and elapsed_s is rounded to 2 decimals, so a 0.2 s
    # sleep can be measured as 0.19: the lower bound allows one clock tick (it flaked at an exact 0.2 bound).
    assert r["outcome"] == "ok" and 0.2 - 0.03 <= r["elapsed_s"] < 3


def test_call_over_limit_is_typed_timeout(shim: str) -> None:
    r = sp.timed_call(shim, sleep_s=2, limit_s=0.3)
    assert r["outcome"] == "timeout" and r["elapsed_s"] < 1.5


def test_exit_code_nonzero_when_unreachable() -> None:
    assert sp.main(["probe", "--base", "http://127.0.0.1:9", "--quick"]) == 1
