"""FIRST RED of the Annex D alignment of the stand-in (PR #86, core-bridge ADR 0011): admission and arm DTOs must be
what a spec-following Rust client sends. The derivation is read from bridge-contract/contract.json (never re-derived
differently) and every body is checked against the contract schemas (required, closed property set, patterns)."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
from typing import Any

import pytest

from codex_standin import dto

CONTRACT = Path(__file__).resolve().parents[3] / "bridge-contract"
SPEC = json.loads((CONTRACT / "contract.json").read_text(encoding="utf-8"))
Z = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?Z$")


def schema(name: str) -> dict[str, Any]:
    return json.loads((CONTRACT / "schemas" / f"{name}.schema.json").read_text(encoding="utf-8"))  # type: ignore[no-any-return]


def check(name: str, body: dict[str, Any]) -> None:
    s = schema(name)
    assert set(body) <= set(s["properties"]), set(body) - set(s["properties"])
    assert set(s["required"]) <= set(body), set(s["required"]) - set(body)
    for k, v in body.items():
        pat = s["properties"][k].get("pattern")
        if pat and isinstance(v, str):
            assert re.match(pat, v), (k, v)


def spec_ref(tenant: str, job: str, binding: str, proposal: str, cand: str, attempt: int) -> str:
    """Evaluates contract.json idempotency.admissions.derivation.evaluation_context_ref literally."""
    formula = SPEC["idempotency"]["admissions"]["derivation"]["evaluation_context_ref"]
    m = re.fullmatch(r"'([^']*)' \+ sha256_hex\('([^']*)'\)\[:(\d+)\]", formula)
    assert m, formula
    prefix, template, n = m.group(1), m.group(2), int(m.group(3))
    text = template.format(tenant_id=tenant, job_id=job, binding_ref=binding, proposal_id=proposal,
                           candidate_hash=cand, evaluation_attempt=attempt)
    return prefix + hashlib.sha256(text.encode()).hexdigest()[:n]


def test_evaluation_context_ref_is_the_contract_derivation() -> None:
    args = ("tenant-local", "job-1", "b" * 64, "prop-1", "c" * 64, 1)
    assert dto.evaluation_context_ref(*args) == spec_ref(*args)
    assert dto.evaluation_context_ref(*args).startswith("evc-") and len(dto.evaluation_context_ref(*args)) == 44
    assert dto.evaluation_context_ref(*args[:5], 2) != dto.evaluation_context_ref(*args)


def test_admission_body_is_annex_d4_and_ref_is_server_derived() -> None:
    body = dto.admission(binding_ref="b" * 64, proposal_id="prop-1", candidate_hash="c" * 64, suite_id="s",
                         suite_version="1.0.0", suite_digest="d" * 64, budget_ref="bud")
    check("EvaluationAdmissionRequest", body)
    assert "evaluation_context_ref" not in body  # the bridge derives it; the response carries it
    assert Z.match(body["deadline"]) and re.fullmatch(r"[0-9a-f]{64}", body["request_digest"])


def test_arm_request_uses_annex_names_and_never_aliases_for_annex_profiles() -> None:
    target = {"kind": "published_release", "release_id": "rel-1"}
    native = dto.arm_request(key="k1", binding_ref="b", manifest_ref="m", profile="evolution_task", target=target)
    check("ArmRequest", native)
    assert native["execution_profile"] == "evolution_task" and "mode" not in native
    assert "seed_manifest_ref" not in native and "agent_id" not in native and Z.match(native["deadline"])
    bank = dto.arm_request(key="k2", binding_ref="b", manifest_ref="m", profile=None, target=target)
    check("ArmRequest", bank)  # task_builder has no annex profile: alias-only (ADR 0011 item 3)
    assert bank["mode"] == "task_builder" and "execution_profile" not in bank
    assert bank["sandbox_session_ref"] == "seed-e2e" and "seed_manifest_ref" not in bank
    with pytest.raises(ValueError):
        dto.arm_request(key="k3", binding_ref="b", manifest_ref="m", profile="native", target=target)  # type: ignore[arg-type]
