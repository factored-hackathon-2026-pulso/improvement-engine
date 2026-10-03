"""The stack is the real thing: version probe, no /_sim/info, pin sha, doubles declared."""

from __future__ import annotations

import httpx
import pytest

from codex_standin import PIN_SHA

pytestmark = pytest.mark.live


def test_runtime_is_real_and_pinned(stack) -> None:  # type: ignore[no-untyped-def]
    assert httpx.get(stack.runtime + "/readyz", timeout=5).status_code == 200
    assert httpx.get(stack.runtime + "/_sim/info", timeout=5).status_code == 404  # never a sim
    ver = stack.bridge.version()
    assert ver["agent_core_sha"] == PIN_SHA and ver["runtime_profile"] == "agent_core_real"
    assert ver["image_digest"] == stack.env["expected_image_digest"]
