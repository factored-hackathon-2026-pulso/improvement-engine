"""Responder protocol checks (RPP): a responder sees only its queue request file and writes only its
response file; scout, verifier and builder use distinct responders and model identities; step caps hold."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any

STEP_CAPS = {"scout": 5, "verifier": 5, "builder": 7}
_MANIFEST_KEYS = {"reads", "writes"}


@dataclass(frozen=True)
class Check:
    ok: bool
    problems: tuple[str, ...] = ()


def _norm(p: str) -> str:
    return p.replace("\\", "/")


def check_manifest(manifest: dict[str, Any], request_file: str, response_file: str) -> Check:
    """The declared access of a responder must be exactly [request_file] read, [response_file] write."""
    problems: list[str] = []
    for key in sorted(set(manifest) - _MANIFEST_KEYS):
        problems.append(f"{key}: capability not allowed")
    reads = [_norm(p) for p in manifest.get("reads", [])]
    writes = [_norm(p) for p in manifest.get("writes", [])]
    if reads != [_norm(request_file)]:
        problems.append(f"reads must be exactly the queue request file, got {reads}")
    if writes != [_norm(response_file)]:
        problems.append(f"writes must be exactly the queue response file, got {writes}")
    return Check(not problems, tuple(problems))


def check_assignments(assignments: dict[str, dict[str, str]], excluded: set[str] | None = None) -> Check:
    """Distinct responder ids and model identities per role; none may be an excluded actor
    (implementer or reviewer of the code under test)."""
    problems: list[str] = []
    for field in ("responder", "model"):
        seen: dict[str, str] = {}
        for role, a in sorted(assignments.items()):
            value = a.get(field)
            if not value:
                problems.append(f"{role}: missing {field}")
            elif value in seen:
                problems.append(f"{field} {value!r} shared by {seen[value]} and {role}")
            else:
                seen[value] = role
    for role, a in sorted(assignments.items()):
        if a.get("responder") in (excluded or set()):
            problems.append(f"{role}: responder is the implementer or reviewer of the code under test")
    return Check(not problems, tuple(problems))


def check_step_budget(role: str, steps_used: int) -> Check:
    cap = STEP_CAPS.get(role)
    if cap is None:
        return Check(False, (f"unknown role {role!r}",))
    if steps_used > cap:
        return Check(False, (f"{role} used {steps_used} steps, cap is {cap}",))
    return Check(True)
