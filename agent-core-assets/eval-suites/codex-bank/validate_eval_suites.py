"""Offline structural/privacy-pattern validator for synthetic suite drafts.

This checks local conventions only. It is not an Agent Core runtime or a policy
oracle; the pinned Pydantic model check is kept in the focused test module.
"""

from __future__ import annotations

import json
import re
from typing import Any


OUTCOMES = {
    "resolved", "abstained", "cancelled", "clarify_exhausted", "completed",
    "failed", "abandoned", "escalated", "transferred",
}
SCENARIO_FIELDS = {
    "id", "source", "principal", "steps", "seed", "sensitive_values",
    "expect", "assertions", "repetitions",
}
STEP_FIELDS = {"op", "text", "answer", "lang", "auth"}
STEP_OPS = {"start", "turn", "confirm"}
SUITE_FIELDS = {"id", "version", "agent_id", "repetitions", "scenarios", "thresholds"}
PRINCIPAL_FIELDS = {"id", "attrs"}
EXPECT_FIELDS = {"outcome", "actions_verified", "escalated"}
CONFIRM_ANSWERS = {"yes", "no"}
EMAIL_RE = re.compile(r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}", re.IGNORECASE)
LONG_DIGIT_RE = re.compile(r"\d{6,}")
PHONE_RE = re.compile(r"(?<!\w)\+?\d[\d ().-]{7,}\d(?!\w)")


def validate_suite_document(document: Any, *, expected_id: str) -> None:
    """Raise ``ValueError`` unless a suite meets this directory's safe draft contract."""
    if not isinstance(document, dict):
        raise ValueError("suite document must be a mapping")
    unexpected_suite_fields = set(document) - SUITE_FIELDS
    if unexpected_suite_fields:
        raise ValueError(
            f"suite contains fields forbidden by pinned Agent Core schema: {sorted(unexpected_suite_fields)}"
        )
    if document.get("id") != expected_id or document.get("agent_id") != expected_id:
        raise ValueError("suite id and agent_id must match the registered filename stem")
    if document.get("version") != "1.0.0" or document.get("repetitions") != 1:
        raise ValueError("suite version/repetitions do not match the draft contract")
    scenarios = document.get("scenarios")
    if not isinstance(scenarios, list) or not 20 <= len(scenarios) <= 30:
        raise ValueError("suite must contain 20 to 30 synthetic scenarios")

    ids: list[str] = []
    per_language: dict[str, list[tuple[str, dict[str, Any]]]] = {"es": [], "pt": []}
    for scenario in scenarios:
        if not isinstance(scenario, dict):
            raise ValueError("scenario must be a mapping")
        unexpected_scenario_fields = set(scenario) - SCENARIO_FIELDS
        if unexpected_scenario_fields:
            raise ValueError(
                f"scenario contains fields forbidden by pinned Agent Core schema: {sorted(unexpected_scenario_fields)}"
            )
        scenario_id = scenario.get("id")
        match = re.fullmatch(r"(happy|protected|negative)-(es|pt)-[a-z0-9-]+", str(scenario_id))
        if not match:
            raise ValueError("scenario id must encode its case type and language")
        if scenario.get("source", "scripted") != "scripted":
            raise ValueError("synthetic suite drafts support only scripted scenarios; dataset source is disabled")
        ids.append(scenario_id)
        case_type, language = match.groups()

        principal = scenario.get("principal")
        if isinstance(principal, dict) and set(principal) - PRINCIPAL_FIELDS:
            raise ValueError("principal contains fields forbidden by pinned Agent Core schema")
        if not isinstance(principal, dict) or not isinstance(principal.get("id"), str):
            raise ValueError("scenario principal is malformed")
        if not re.fullmatch(r"synthetic-[a-z0-9-]+", principal["id"]):
            raise ValueError("scenario principal must be a synthetic identifier")
        attrs = principal.get("attrs")
        if attrs not in (
            {"country": "MX"}, {"country": "CO"}, {"country": "AR"},
            {"role": "advisor"},
        ):
            raise ValueError("principal attributes must use the bounded synthetic vocabulary")

        steps = scenario.get("steps")
        if (not isinstance(steps, list) or not steps or not isinstance(steps[0], dict)
                or steps[0].get("op") != "start"):
            raise ValueError("scenario must begin with a start step")
        if any(isinstance(step, dict) and step.get("op") == "start" for step in steps[1:]):
            raise ValueError("scenario may contain only one initial start step")
        for step in steps:
            if not isinstance(step, dict):
                raise ValueError("step must be a mapping")
            unexpected_step_fields = set(step) - STEP_FIELDS
            if unexpected_step_fields:
                raise ValueError(
                    f"step contains fields forbidden by pinned Agent Core schema: {sorted(unexpected_step_fields)}"
                )
            if step.get("op") not in STEP_OPS:
                raise ValueError("step op must be one of start, turn, or confirm")
            if step.get("auth", "step_up") not in {"anonymous", "session", "step_up"}:
                raise ValueError("step auth is outside the pinned Agent Core enum")
            if step["op"] == "turn" and (
                not isinstance(step.get("text"), str) or not step["text"].strip()
                or len(step["text"]) > 4000
            ):
                raise ValueError("turn step requires nonempty text of at most 4000 characters")
            if step["op"] == "confirm" and step.get("answer") not in CONFIRM_ANSWERS:
                raise ValueError("confirm step requires answer yes or no")
        turns = [
            step for step in steps
            if isinstance(step, dict) and step.get("op") == "turn"
        ]
        turn_languages = [turn.get("lang") for turn in turns]
        if not turns or turn_languages[0] != language or any(
            turn.get("lang") not in {"es", "pt"} or not isinstance(turn.get("text"), str)
            or not turn["text"].strip()
            for turn in turns
        ):
            raise ValueError("scenario turn language/text is malformed")
        if len(set(turn_languages)) > 1:
            raise ValueError("automated scenarios must use one language per run; switch cases are manual-only")

        expected = scenario.get("expect")
        if isinstance(expected, dict) and set(expected) - EXPECT_FIELDS:
            raise ValueError("expect contains fields forbidden by pinned Agent Core schema")
        outcome = expected.get("outcome") if isinstance(expected, dict) else None
        if (
            not isinstance(expected, dict)
            or (outcome is not None and (not isinstance(outcome, str) or outcome not in OUTCOMES))
            or (expected.get("escalated") is not None
                and not isinstance(expected.get("escalated"), bool))
        ):
            raise ValueError("scenario expected outcome is malformed")
        if "actions_verified" in expected and (
            not isinstance(expected["actions_verified"], list)
            or any(not isinstance(action, str) for action in expected["actions_verified"])
        ):
            raise ValueError("expect.actions_verified must be a list of strings")
        if (outcome is not None and expected.get("escalated") is not None
                and expected["escalated"] != (outcome == "escalated")):
            raise ValueError("expected escalated must be true only for an Escalated event; transfer is not escalation")
        if outcome == "clarify_exhausted" and len(turns) != 2:
            raise ValueError("clarify_exhausted cases must stop at the observed terminal turn (two user turns)")

        markers = scenario.get("sensitive_values", [])
        if not isinstance(markers, list) or any(
            not isinstance(marker, str)
            or not re.fullmatch(r"SYNTH-[A-Z0-9-]+", marker)
            for marker in markers
        ):
            raise ValueError("sensitive_values must contain synthetic markers only")
        if case_type == "protected" and not markers:
            raise ValueError("protected scenarios require a synthetic sensitive marker")
        if any(marker not in " ".join(turn["text"] for turn in turns) for marker in markers):
            raise ValueError("sensitive marker must occur in its synthetic input text")
        per_language[language].append((case_type, scenario))

    if len(ids) != len(set(ids)):
        raise ValueError("scenario IDs must be unique within each suite")
    serialized = json.dumps(document, ensure_ascii=False, sort_keys=True)
    if EMAIL_RE.search(serialized) or LONG_DIGIT_RE.search(serialized) or PHONE_RE.search(serialized):
        raise ValueError("suite contains a value matching an email/phone-like PII pattern")
    for language, cases in per_language.items():
        if not 10 <= len(cases) <= 15:
            raise ValueError(f"suite must contain ten to fifteen {language} scenarios")
        for case_type, minimum in (("happy", 1), ("protected", 2), ("negative", 1)):
            if sum(kind == case_type for kind, _ in cases) < minimum:
                raise ValueError(f"suite lacks minimum {language} {case_type} coverage")
