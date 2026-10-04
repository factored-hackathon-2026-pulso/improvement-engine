"""BKNL: live denied-kind negatives. Five drafts, each denied for one NAMED reason, expressed twice:
the engine ChangeSpec operation (the compile step labels it) and the Core draft (`changes`) that the real
`POST /core-authoring/dry-run` receives through the bridge. Nothing here publishes: dry-run writes nothing, and the
live test compares the registry before and after.

`release_settings` is not expressible as an engine operation (the compile schema only knows prompt/eval_suite): its
only guard is the bridge (`pulso:release_settings_not_allowed`, 422)."""
from __future__ import annotations

import copy
from dataclasses import dataclass
from typing import Any, Callable

from . import compile_step as cmp
from .contracts import validate_in
from .core_hooks import DOCS, _load

PROMPT_REF, SUITE_REF = "prompt:resumen_radicado@1", "eval_suite:disputas-tarea-suite@1"


def _digest(world: dict, ref: str) -> str:
    return cmp.asset_digest(world, ref)


@dataclass(frozen=True)
class Negative:
    id: str
    expected_reason: str
    engine_op: Callable[[dict], dict] | None  # world -> ChangeSpec operation
    core_changes: Callable[[dict], list]  # world -> Core `changes`
    why: str
    core_refuses: bool = True  # False: the real Core ACCEPTS this draft (live finding), the engine is the only guard


def _prompt(world: dict) -> dict:
    return _load(cmp._slots(world)["prompt"]["file"])


def _draft(kind: str, content: dict) -> dict:
    return {"kind": kind, "content": content, "docs": dict(DOCS)}


def _op_add_prompt(w: dict) -> dict:
    return {"op": "add", "target_kind": "prompt", "target_ref": PROMPT_REF, "new_ref": "prompt:resumen_radicado@2",
            "precondition_digest": _digest(w, PROMPT_REF)}


def _op_stale(w: dict) -> dict:
    return {"op": "replace", "target_kind": "prompt", "target_ref": PROMPT_REF, "new_ref": "prompt:resumen_radicado@2",
            "precondition_digest": "sha256:" + "0" * 64}


def _op_outside(w: dict) -> dict:
    return {"op": "replace", "target_kind": "prompt", "target_ref": "prompt:otro_prompt@1",
            "new_ref": "prompt:otro_prompt@2", "precondition_digest": _digest(w, PROMPT_REF)}


def _op_overwrite(w: dict) -> dict:
    return {"op": "replace", "target_kind": "prompt", "target_ref": PROMPT_REF, "new_ref": PROMPT_REF,
            "precondition_digest": _digest(w, PROMPT_REF)}


def _core_add_prompt(w: dict) -> list:  # add_prompt: a NEW prompt id next to the replaceable one
    c = _prompt(w); c["id"], c["version"] = "p/resumen_adicional", "1.0.0"
    return [_draft("prompt", c)]


def _core_replace(w: dict) -> list:  # the same draft the green path sends (the stale precondition is engine-only state)
    c = _prompt(w); c["version"] = "2.0.0"
    return [_draft("prompt", c)]


def _core_outside(w: dict) -> list:
    c = _prompt(w); c["id"], c["version"] = "p/otro_prompt", "2.0.0"
    return [_draft("prompt", c)]


def _core_overwrite(w: dict) -> list:  # same id@version as the published prompt, different content
    c = _prompt(w); c["locales"] = {**c["locales"], "es": c["locales"]["es"] + " Se amable."}
    return [_draft("prompt", c)]


def _core_release_settings(w: dict) -> list:
    return [_draft("release_settings", {"interrupts": []})]


NEGATIVES: list[Negative] = [
    Negative("add_prompt", "kind_not_supported", _op_add_prompt, _core_add_prompt,
             "add on a prompt (add_prompt) is not a targetable kind of the seeded world"),
    Negative("stale_precondition", "missing_precondition", _op_stale, _core_replace,
             "precondition_digest differs from the digest of the asset in the world", core_refuses=False),
    Negative("outside_bridge", "outside_bridge", _op_outside, _core_outside,
             "target prompt is not the slot used by the bridged flow"),
    Negative("overwrite_published", "mutable_reference", _op_overwrite, _core_overwrite,
             "new_ref is not a strictly newer version: it would overwrite a published immutable prompt"),
    Negative("release_settings", "release_settings_not_allowed", None, _core_release_settings,
             "release_settings is denied by the bridge (CAP-23 / N-07); the engine has no operation for it"),
]


def spec_doc(world: dict, op: dict) -> dict:
    flow = world["replaceable_prompt"]["used_by"]["flow"]
    bundle = cmp.bundle_ref(world)
    return {"contract_version": "engine-steps/0", "step": "compile", "run_id": "run-bknl-0001", "data_class": "synthetic",
            "base_bundle_ref": bundle,
            "change_spec": {"base_bundle_ref": bundle, "opportunity_ref": "opportunity:bknl@1",
                            "workflow_bridge_ref": cmp.bridge_ref(world), "operations": [op],
                            "expected_mechanism": "negative probe", "affected_routes": [flow], "rollback_ref": bundle}}


def engine_label(neg: Negative, world: dict, compile_fn: Callable = cmp.compile_change_spec) -> dict:
    """The compile step's answer for the negative's operation (never raises on a denial)."""
    assert neg.engine_op is not None, f"{neg.id} has no engine operation"
    doc = spec_doc(world, neg.engine_op(world))
    assert validate_in("compile", copy.deepcopy(doc)) == [], "the negative must be schema-valid so the compile step decides"
    return compile_fn(doc, world)


def assert_engine_denies(neg: Negative, world: dict, compile_fn: Callable = cmp.compile_change_spec) -> str:
    out = engine_label(neg, world, compile_fn)
    assert out.get("status") == "denied", f"{neg.id}: a denied kind slipped through the compile step ({out.get('status')})"
    assert out.get("denied_reason") == neg.expected_reason, f"{neg.id}: {out.get('denied_reason')} != {neg.expected_reason}"
    assert "draft_plan" not in out
    return str(out["denied_reason"])


def core_request(world: dict, neg: Negative, *, tenant: str, agent_id: str, base_release_id: str | None) -> dict:
    return {"schema_version": "1", "tenant_id": tenant, "agent_id": agent_id, "base_release_id": base_release_id,
            "changes": neg.core_changes(world)}


def classify_core(status: int, body: Any) -> dict:
    """Closed classification of the real Core/bridge answer to a dry-run: accepted | refused | denied | error."""
    b = body if isinstance(body, dict) else {}
    if status == 200 and b.get("valid") is True:
        return {"outcome": "accepted"}
    if status == 200:
        return {"outcome": "refused", "rules": [v.get("rule") for v in b.get("violations", [])]}
    code = b.get("code")
    if isinstance(code, str) and code.startswith("pulso:") and 400 <= status < 500:
        return {"outcome": "denied", "code": code, "http": status}
    return {"outcome": "error", "http": status}
