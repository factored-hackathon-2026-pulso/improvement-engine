"""FIRST RED (plan 17.3.3): binding denied -> zero lab queries, zero model calls, zero registry writes."""

from __future__ import annotations

import pytest
from agent_core.domain.errors import GatewayError
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools.context import RegistryMutationCommitment, use_binding

from .support import Env


@pytest.mark.parametrize("mode", ["conflict", "digest_mismatch", "not_found", "unavailable", "timeout"])
def test_binding_denied_means_zero_effects(mode: str) -> None:
    env = Env()
    commitment = RegistryMutationCommitment(mode="write", proposal_id=None, expected_rev=None,
                                            base_release_id=None, evaluate_enabled=False)
    ic = env.invocation("writer", commitment=commitment)
    env.backend.bind_mode = mode

    assert env.bind(ic).status is ToolStatus.denied

    # Every other tool is denied until the binding is confirmed; none reaches the broker.
    for tool, args in (("pulso/lab_query", {"sql": "select 1"}), ("pulso/wiki_read", {"path": "a.md"}),
                       ("pulso/wiki_explore", {"query": "q"}), ("pulso/artifact_get", {"artifact_ref": "art-1"}),
                       ("pulso/wiki_transform", {"source_ref": "s", "transform": "t"})):
        assert env.call(ic, tool, args).status is ToolStatus.denied, tool
    # Registry writes through the protected executor: zero effect.
    r = env.call(ic, "registry/create_proposal", {"agent_id": "a", "origin": "builder_chat", "title": "t"},
                 key="engine-key-1")
    assert r.status is ToolStatus.denied
    assert env.inner.calls == []
    assert env.backend.lab_requests == 0
    assert env.backend.wiki_requests == 0
    assert env.backend.count("/artifacts/") == 0

    # Model budget: an Agent node reaching the gateway/provider before binding is confirmed is refused.
    with use_binding(ic.binding_ref):
        with pytest.raises(GatewayError):
            env.guard.generate(None, {}, "en")  # type: ignore[arg-type]
        with pytest.raises(Exception, match="binding"):
            env.provider_guard.predict(None, {}, {}, "en")
    assert env.gateway.calls == 0 and env.provider.calls == 0
