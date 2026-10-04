"""M3 stage hardening: gateway schema subset, stage policy alignment with the registry profile, timeout plan, and
replay of the synthetic corpus through the roleplay shim (rung 2). Plumbing only: no model quality is claimed."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
from decimal import Decimal
from pathlib import Path
from types import SimpleNamespace

import jsonschema
import pytest
from agent_core.domain.schema import check_output, unsupported_keyword

from pulso_core_runtime.llm.policy import ModelPolicy
from pulso_core_runtime.stages.catalog import CATALOG, STEP_CAPS, core_schema_problems
from pulso_core_runtime.stages.policy import evolution_policy, load_registry_profile
from pulso_core_runtime.stages.timeouts import CLIENT_MARGIN_S, timeout_plan

from . import stage_corpus as corpus

ROOT = Path(__file__).resolve().parents[3]
PROFILE_FILE = ROOT / "agent-core-assets/worlds/pulso-evolution/model_profiles/pulso-evolution-structured@1.0.0.yaml"
STRICT = ROOT / "core-bridge/src/pulso_core_runtime/facts/schemas"
sys.path.insert(0, str(ROOT / "roleplay-llm"))
from roleplay_llm.protocol import STEP_CAPS as ROLEPLAY_CAPS  # noqa: E402
from roleplay_llm.scanner import scan_payload  # noqa: E402
from roleplay_llm.shim import HOLD_S, Shim, replay_key  # noqa: E402

LLM_STAGES = ("scout", "verifier", "builder_design")
PROFILE_SHA256 = hashlib.sha256(PROFILE_FILE.read_bytes()).hexdigest()


def test_registry_profile_bytes_are_untouched() -> None:
    # Moving this file moves release digests; M3 aligns the policy to it, never the other way round.
    assert hashlib.sha256(PROFILE_FILE.read_bytes()).hexdigest() == PROFILE_SHA256
    profile = load_registry_profile(PROFILE_FILE)
    assert (profile.endpoint_alias, profile.model, profile.timeout_s) == ("pulso-evolution-llm",
                                                                         "external-reasoning-model", 60)
    assert (profile.price.input_per_mtok, profile.price.output_per_mtok) == (Decimal(1), Decimal(4))


# -- gateway schema subset -----------------------------------------------------------------------------
@pytest.mark.parametrize("stage", LLM_STAGES)
def test_core_schema_is_inside_the_closed_gateway_subset(stage: str) -> None:
    schema = CATALOG[stage].core_output_schema
    assert schema is not None
    assert core_schema_problems(schema) == []
    assert unsupported_keyword(schema) is None  # the real Core's own check agrees


@pytest.mark.parametrize("bad", [{"type": "array", "minItems": 1}, {"type": "string", "pattern": "^a"},
                                 {"oneOf": [{"type": "string"}]}, {"type": "object", "properties": {
                                     "x": {"type": "array", "items": {"type": "string", "maxLength": 3}}}}])
def test_schema_checker_rejects_keywords_outside_the_subset(bad: dict) -> None:
    assert core_schema_problems(bad) != []


@pytest.mark.parametrize("stage", LLM_STAGES)
def test_synthetic_final_is_valid_in_core_subset_and_strict_schema(stage: str) -> None:
    spec, final = CATALOG[stage], corpus.SCRIPTS[stage][1]
    assert check_output(spec.core_output_schema, final) is None
    strict = json.loads((STRICT / f"{spec.fact}.strict.json").read_text(encoding="utf-8"))
    jsonschema.Draft202012Validator(strict).validate(final)
    assert check_output(spec.core_output_schema, {**final, "schema_version": "2"}) is not None


@pytest.mark.parametrize("stage", LLM_STAGES)
def test_stage_inputs_pass_the_treated_payload_scanner(stage: str) -> None:
    for step in (1, 2):
        assert scan_payload(corpus.step_inputs(stage, step)).ok


# -- stage policy alignment ----------------------------------------------------------------------------
def test_policy_is_built_from_the_registry_profile_for_llm_stages_only() -> None:
    profile = load_registry_profile(PROFILE_FILE)
    policy = evolution_policy(profile)
    assert isinstance(policy, ModelPolicy) and policy.stages == tuple(sorted(LLM_STAGES))
    assert policy.for_stage("writer") is None  # projection stage: never calls a model
    for stage in LLM_STAGES:
        assert policy.check(stage, profile) is None
        pinned = policy.for_stage(stage)
        assert (pinned.endpoint_alias, pinned.model, pinned.input_per_mtok, pinned.output_per_mtok,
                pinned.max_tokens) == ("pulso-evolution-llm", "external-reasoning-model", Decimal(1), Decimal(4),
                                       4000)


@pytest.mark.parametrize(("over", "reason"), [
    ({"model": "agent_roleplay"}, "model_mismatch"),
    ({"endpoint_alias": "agent_roleplay"}, "alias_mismatch"),
    ({"price": SimpleNamespace(input_per_mtok=Decimal(0), output_per_mtok=Decimal(0))}, "price_mismatch")])
def test_a_profile_that_does_not_match_the_registry_is_rejected(over: dict, reason: str) -> None:
    base = load_registry_profile(PROFILE_FILE)
    policy = evolution_policy(base)
    drifted = SimpleNamespace(**{**vars(base), **over})
    assert [policy.check(s, drifted) for s in LLM_STAGES] == [reason] * 3


# -- step caps and timeout plan ------------------------------------------------------------------------
def test_step_caps_match_the_responder_protocol_and_exclude_the_writer() -> None:
    assert STEP_CAPS == {"scout": 5, "verifier": 5, "builder_design": 7}
    assert STEP_CAPS == {"scout": ROLEPLAY_CAPS["scout"], "verifier": ROLEPLAY_CAPS["verifier"],
                         "builder_design": ROLEPLAY_CAPS["builder"]}


def test_default_timeout_plan_fits_the_core_invoke_timeout() -> None:
    plan = timeout_plan(profile_timeout_s=60, invoke_timeout_s=600, hold_s=HOLD_S)
    assert plan.problems == [] and CLIENT_MARGIN_S == 5.0
    assert plan.client_wait_s == 65 and plan.worst_case_s == {"scout": 275, "verifier": 275, "builder_design": 385}


@pytest.mark.parametrize(("kwargs", "needle"), [
    ({"invoke_timeout_s": 300}, "builder_design"),            # 7 x 55 s = 385 s > 300 s
    ({"profile_timeout_s": 50}, "hold"),                      # gateway would time out before the shim answers
    ({"profile_timeout_s": 301}, "gateway maximum"),          # llm-gateway rejects timeout_s > 300
    ({"hold_s": 0}, "hold")])
def test_timeout_plan_names_the_stage_or_limit_it_breaks(kwargs: dict, needle: str) -> None:
    base = {"profile_timeout_s": 60, "invoke_timeout_s": 600, "hold_s": HOLD_S}
    plan = timeout_plan(**{**base, **kwargs})
    assert any(needle in p for p in plan.problems), plan.problems


# -- replay of the synthetic corpus (rung 2) -----------------------------------------------------------
def _record(queue: Path, stage: str, step: int, key: str) -> None:
    doc = {"protocol": "roleplay-queue/1", "key": key, "provenance": "agent_roleplay", "quality_claims": "forbidden",
           "responder": {"id": f"synthetic-{stage}", "role": stage}, "content": corpus.content(stage, step)}
    (queue / "responses").mkdir(parents=True, exist_ok=True)
    (queue / "responses" / f"{key}.json").write_text(json.dumps(doc), encoding="utf-8")


@pytest.mark.parametrize("stage", LLM_STAGES)
def test_stage_replays_three_times_with_fresh_ids_and_zero_misses(stage: str) -> None:
    model = load_registry_profile(PROFILE_FILE).model
    with tempfile.TemporaryDirectory() as tmp:
        queue = Path(tmp)
        for step in (1, 2):
            first = corpus.step_inputs(stage, step)
            _record(queue, stage, step, replay_key(corpus.system_prompt(stage), first))
        shim = Shim(queue, replay_only=True)
        for _ in range(3):  # fresh binding/artifact ids each time: a new Core DB
            for step in (1, 2):
                status, body = shim.handle(corpus.chat_body(stage, corpus.step_inputs(stage, step), model))
                assert status == 200, body
                assert body["model"] == model  # echo of the registry model: policy sees no model_mismatch
                answer = json.loads(body["choices"][0]["message"]["content"])
                assert answer == corpus.content(stage, step)
                if answer["kind"] == "final":
                    assert check_output(CATALOG[stage].core_output_schema, answer["output"]) is None


def test_replay_drift_reports_a_diff_instead_of_going_live() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        queue = Path(tmp)
        recorded = corpus.step_inputs("scout", 1)
        Shim(queue, hold_s=0.01, poll_s=0.005).handle(
            corpus.chat_body("scout", recorded, "external-reasoning-model"))  # enqueues, then times out
        drifted = dict(recorded, goal="Synthetic scout goal, revised.")
        status, body = Shim(queue, replay_only=True).handle(
            corpus.chat_body("scout", drifted, "external-reasoning-model"))
        assert status == 409 and body["error"]["type"] == "replay_miss"
        assert any(line.startswith("goal:") for line in body["error"]["diff"])
