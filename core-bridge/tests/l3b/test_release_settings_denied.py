"""N-07 safety gap (agent-core 789d6c8): `release_settings` is a draft kind that can replace the whole interrupt list
(drop a fraud interrupt). The protected writer denies it by default, EVEN when the committed digest covers it,
until a Pulso-side guardrail exists (upstream D-17)."""

from __future__ import annotations

from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools.builder import put_draft_digest

from .test_protected_builder import CHANGES, CREATE, commitment, writer

DOCS = {"description": "d", "rationale": "r", "changelog": "c"}
SETTINGS = {"kind": "release_settings", "content": {"interrupts": []}, "docs": DOCS}


def test_release_settings_is_denied_with_zero_effect_even_if_committed() -> None:
    changes = [*CHANGES, SETTINGS]
    env, ic = writer(commitment(put_draft_digest=put_draft_digest(None, None, changes)))
    assert env.call(ic, "registry/create_proposal", CREATE, key="e1").status is ToolStatus.ok
    n = len(env.inner.calls)
    r = env.call(ic, "registry/put_draft", {"proposal_id": "prop-1", "expected_rev": 1, "changes": changes}, key="e2")
    assert r.status is ToolStatus.denied and r.error == "pulso:release_settings_not_allowed"
    assert len(env.inner.calls) == n


def test_ordinary_drafts_are_unaffected() -> None:
    env, ic = writer()
    env.call(ic, "registry/create_proposal", CREATE, key="e1")
    r = env.call(ic, "registry/put_draft", {"proposal_id": "prop-1", "expected_rev": 1, "changes": CHANGES}, key="e2")
    assert r.status is ToolStatus.ok
