"""Read-only HTTP acceptance check for a draft proposal in local Agent Core."""

from __future__ import annotations

import argparse
import ipaddress
import json
import os
import re
import sys
from dataclasses import dataclass
from datetime import datetime
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import unquote, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener


PII_TOKEN = re.compile(r"(?:\[\s*PII(?:_[A-Z0-9_-]+)?\s*\]|<PII(?:_[A-Z0-9_-]+)?>|\bpii:[a-z0-9_-]+)", re.I)
MAX_RESPONSE_BYTES = 1_048_576
EMAIL_CANARY = re.compile(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", re.I)
PHONE_CANDIDATE = re.compile(r"(?<!\w)\+?\d[\d().\s-]{7,}\d(?!\w)")
UUID_CANARY = re.compile(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b", re.I)
LONG_NUMBER_CANARY = re.compile(r"\b\d{12,}\b")
NON_TEXT_METADATA = frozenset({
    "id", "*_id", "rev", "version", "candidate_hash", "created_at", "updated_at",
    "published_at", "timestamp", "digest", "sha256",
})
SECTION_ALIASES = {
    "problem": ("problema observado",),
    "evidence": ("evidencia y comparación", "evidência e comparação"),
    "change": ("qué cambiaría", "o que mudaria"),
    "effect": ("efecto esperado", "efeito esperado"),
    "measurement": ("cómo se evaluará", "como será avaliado"),
    "risk": ("riesgo", "risco"),
    "unchanged": ("qué no cambia", "o que não muda"),
}
FORBIDDEN_ACTIONS = ("approve", "approved", "publish", "published", "promote", "promoted")


class _RejectRedirects(HTTPRedirectHandler):
    """Do not let a loopback acceptance endpoint redirect requests elsewhere."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


@dataclass(frozen=True)
class AcceptanceResult:
    failures: tuple[str, ...]
    not_exercised: tuple[str, ...]

    @property
    def exit_code(self) -> int:
        return 1 if self.failures else 2 if self.not_exercised else 0


def _proposal_object(value: Any) -> dict[str, Any] | None:
    if isinstance(value, dict) and "proposal" in value:
        # Agent Core ProposalDetail carries artifact changes beside the proposal
        # summary. Preserve those siblings when normalizing to the checker view.
        detail = value
        if not isinstance(detail.get("proposal"), dict) or not _valid_proposal_summary(detail["proposal"]):
            return None
        if not {"proposal", "changes", "last_eval"}.issubset(detail):
            return None
        if set(detail) - {"proposal", "changes", "last_eval", "review"}:
            return None
        if not isinstance(detail["changes"], list):
            return None
        # This checker targets an un-evaluated draft; any recorded EvalRun is
        # outside this acceptance path and must not be accepted as a valid draft.
        if detail["last_eval"] is not None:
            return None
        if detail.get("review") is not None:
            return None
        value = dict(detail["proposal"])
        for field in ("changes", "last_eval", "review"):
            if field in detail:
                value[field] = detail[field]
    return value if isinstance(value, dict) else None


def _valid_proposal_summary(proposal: Any) -> bool:
    """Validate Proposal's pinned required fields and constraints for a draft."""
    if not isinstance(proposal, dict):
        return False
    required = {
        "proposal_id", "agent_id", "origin", "state", "base_release_id",
        "title", "created_by", "updated_at",
    }
    allowed = required | {"candidate_hash", "rev"}
    if required - proposal.keys() or proposal.keys() - allowed:
        return False
    if any(not isinstance(proposal[key], str) for key in ("proposal_id", "created_by")):
        return False
    agent_id = proposal["agent_id"]
    if not isinstance(agent_id, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_/-]*", agent_id):
        return False
    if not isinstance(proposal["origin"], str) or proposal["origin"] not in {"manual", "builder_chat", "auto_detect", "import"}:
        return False
    if not isinstance(proposal["state"], str) or proposal["state"] not in {"draft", "candidate", "evaluated", "approved", "published"}:
        return False
    title = proposal["title"]
    if not isinstance(title, str) or not 1 <= len(title) <= 200:
        return False
    base_release_id = proposal.get("base_release_id")
    if base_release_id is not None and not isinstance(base_release_id, str):
        return False
    if "candidate_hash" in proposal and proposal["candidate_hash"] is not None and not isinstance(proposal["candidate_hash"], str):
        return False
    if "rev" in proposal and (not isinstance(proposal["rev"], int) or isinstance(proposal["rev"], bool) or proposal["rev"] < 0):
        return False
    timestamp = proposal["updated_at"]
    if not isinstance(timestamp, str):
        return False
    try:
        parsed = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError:
        return False
    return "T" in timestamp and parsed.tzinfo is not None


def _valid_entity_drafts(changes: Any) -> bool:
    """Validate generic EntityDraft wire shape, not kind-specific artifact semantics."""
    if not isinstance(changes, list) or not changes:
        return False
    for change in changes:
        if not isinstance(change, dict) or set(change) != {"kind", "content", "docs"}:
            return False
        kind, content, docs = change["kind"], change["content"], change["docs"]
        if not isinstance(kind, str) or not kind.strip() or len(kind) > 64:
            return False
        if not isinstance(content, dict) or not content:
            return False
        if not isinstance(docs, dict) or set(docs) != {"description", "rationale", "changelog"}:
            return False
        description, rationale, changelog = (
            docs["description"], docs["rationale"], docs["changelog"]
        )
        if not isinstance(description, str) or not description.strip() or len(description) > 4000:
            return False
        if not isinstance(rationale, str) or len(rationale) > 4000:
            return False
        if not isinstance(changelog, str) or len(changelog) > 8000:
            return False
    return True


def _dossier_text(changes: Any) -> tuple[str, bool]:
    if not isinstance(changes, list) or not changes:
        return "", False
    parts: list[str] = []
    required_docs_present = True
    for change in changes:
        docs = change.get("docs") if isinstance(change, dict) else None
        if not isinstance(docs, dict):
            required_docs_present = False
            continue
        for field in ("description", "rationale", "changelog"):
            text = docs.get(field)
            if not isinstance(text, str) or not text.strip():
                required_docs_present = False
            else:
                parts.append(text.strip())
    return "\n".join(parts), required_docs_present


def _description_sections(description: str) -> dict[str, list[str]]:
    matches: list[tuple[int, int, str]] = []
    for section, aliases in SECTION_ALIASES.items():
        alternatives = "|".join(re.escape(alias) for alias in sorted(aliases, key=len, reverse=True))
        heading = re.compile(rf"(?:{alternatives})\s*[.:]\s*(?:\*\*)?", re.IGNORECASE)
        for match in heading.finditer(description):
            matches.append((match.start(), match.end(), section))
    matches.sort()
    section_bodies: dict[str, list[str]] = {}
    for index, (_start, end, section) in enumerate(matches):
        next_start = matches[index + 1][0] if index + 1 < len(matches) else len(description)
        body = description[end:next_start].strip(" \t\r\n.*_:-")
        if body:
            section_bodies.setdefault(section, []).append(body)
    return section_bodies


def _descriptions_have_required_sections(changes: Any) -> bool:
    """Require each changed artifact's rendered description to stand alone."""
    if not isinstance(changes, list) or not changes:
        return False
    for change in changes:
        docs = change.get("docs") if isinstance(change, dict) else None
        description = docs.get("description") if isinstance(docs, dict) else None
        if not isinstance(description, str) or not description.strip():
            return False
        if any(not _description_sections(description).get(section) for section in SECTION_ALIASES):
            return False
    return True


def _descriptions_have_comparable_evidence(changes: Any) -> bool:
    if not isinstance(changes, list) or not changes:
        return False
    for change in changes:
        docs = change.get("docs") if isinstance(change, dict) else None
        description = docs.get("description") if isinstance(docs, dict) else None
        if not isinstance(description, str):
            return False
        evidence_text = " ".join(_description_sections(description).get("evidence", []))
        numeric_values = re.findall(r"(?<!\w)\d+(?:[.,]\d+)?\s*(?:%|pp)?", evidence_text, re.IGNORECASE)
        has_comparison = re.search(r"(?:\bvs\.?\b|\bversus\b|\bcontra\b|\bfrente\s+a\b|\bcomparad[oa]\s+con\b)",
                                   evidence_text, re.IGNORECASE)
        has_source_scope = re.search(r"\b(?:snapshot|fuente|source|dataset|audit|audited)\b",
                                     evidence_text, re.IGNORECASE)
        if len(numeric_values) < 2 or not has_comparison or not has_source_scope:
            return False
    return True


def _visible_text(value: Any, *, skip_opaque_metadata: bool = True) -> list[str]:
    """Collect text, excluding envelope IDs but scanning artifact content fully."""
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        return [text for item in value for text in _visible_text(item, skip_opaque_metadata=skip_opaque_metadata)]
    if isinstance(value, dict):
        result = []
        for key, item in value.items():
            normalized = str(key).casefold()
            if skip_opaque_metadata and (normalized in NON_TEXT_METADATA or normalized.endswith("_id")):
                continue
            result.append(str(key))
            result.extend(_visible_text(item, skip_opaque_metadata=skip_opaque_metadata and normalized != "content"))
        return result
    return []


def _has_obvious_pii_canary(text: str) -> bool:
    if PII_TOKEN.search(text) or EMAIL_CANARY.search(text) or UUID_CANARY.search(text):
        return True
    if LONG_NUMBER_CANARY.search(text):
        return True
    return any(sum(character.isdigit() for character in match.group()) >= 10
               for match in PHONE_CANDIDATE.finditer(text))


def _valid_history(history: Any) -> bool:
    if not isinstance(history, dict) or not isinstance(history.get("actions"), list):
        return False
    return all(isinstance(event, dict) for event in history["actions"])


def _quota_failure(quota: Any) -> str | None:
    if not isinstance(quota, dict):
        return "quota_response_invalid"
    used = quota.get("used_24h")
    limit = quota.get("limit")
    accepted = quota.get("within_limit_accepted")
    rejected = quota.get("over_limit_rejected")
    if not isinstance(used, int) or isinstance(used, bool) or used < 0 or limit != 10:
        return "quota_configuration_invalid"
    if used > limit:
        return "quota_exceeded_without_rejection"
    if accepted is not True:
        return "quota_within_limit_not_accepted"
    if rejected is not True:
        return "quota_rejection_not_observed"
    return None


def check_acceptance(
    proposal_response: Any,
    *,
    expected_proposal_id: str | None = None,
    history: Any = None,
    quota: Any = None,
) -> AcceptanceResult:
    """Check recorded or HTTP-decoded responses without importing engine code."""
    failures: list[str] = []
    not_exercised: list[str] = []
    proposal = _proposal_object(proposal_response)
    if proposal is None:
        return AcceptanceResult(("proposal_response_invalid",), ())
    if expected_proposal_id is not None and proposal.get("proposal_id") != expected_proposal_id:
        failures.append("proposal_id_mismatch")
    if proposal.get("origin") != "auto_detect":
        failures.append("proposal_origin_not_auto_detect")
    if proposal.get("state") != "draft":
        failures.append("proposal_not_draft")
    changes = proposal.get("changes")
    if not isinstance(changes, list) or not changes:
        failures.append("proposal_changes_empty")
    elif not _valid_entity_drafts(changes):
        failures.append("proposal_changes_invalid")
    dossier, docs_present = _dossier_text(changes)
    if not docs_present:
        failures.append("dossier_docs_missing")
    if not _descriptions_have_required_sections(changes):
        failures.append("dossier_sections_missing")
    if not _descriptions_have_comparable_evidence(changes):
        failures.append("dossier_evidence_incomplete")
    if not isinstance(proposal.get("rationale"), str) or not proposal["rationale"].strip():
        # Some Agent Core versions carry rationale only in per-change VersionDocs.
        if not docs_present or not all(
            isinstance(change.get("docs", {}).get("rationale"), str)
            and change["docs"]["rationale"].strip()
            for change in changes
            if isinstance(change, dict) and isinstance(change.get("docs"), dict)
        ):
            failures.append("rationale_missing")
    if not isinstance(proposal.get("changelog"), str) or not proposal["changelog"].strip():
        if not docs_present or not all(
            isinstance(change.get("docs", {}).get("changelog"), str)
            and change["docs"]["changelog"].strip()
            for change in changes
            if isinstance(change, dict) and isinstance(change.get("docs"), dict)
        ):
            failures.append("changelog_missing")
    visible_text = "\n".join(_visible_text(proposal))
    if _has_obvious_pii_canary(visible_text):
        failures.append("pii_token_detected")

    if history is None:
        not_exercised.append("lifecycle_history")
    elif not _valid_history(history):
        failures.append("lifecycle_history_observation_invalid")
        not_exercised.append("lifecycle_history_provenance_and_completeness")
    else:
        if any(
            str(event.get("actor", "")).lower() in {"engine", "pulso", "pulso_engine", "auto_detect"}
            and any(
                token in str(event.get("action", "")).lower().replace("-", "_")
                for token in FORBIDDEN_ACTIONS
            )
            for event in history["actions"]
            if isinstance(event, dict)
        ):
            failures.append("engine_lifecycle_mutation")
        # A plausible response shape is not proof of immutable, proposal-bound,
        # complete history. Keep it non-green until an authenticated read-only
        # contract for that evidence exists.
        not_exercised.append("lifecycle_history_provenance_and_completeness")
    if quota is None:
        not_exercised.append("quota_boundary")
    else:
        quota_failure = _quota_failure(quota)
        if quota_failure:
            failures.append(quota_failure)
        # These counts/booleans are not bound by the pinned API to a tenant,
        # proposal, 24h window, or actual enforced request outcomes.
        not_exercised.append("quota_scope_and_enforcement_provenance")
    return AcceptanceResult(tuple(sorted(set(failures))), tuple(sorted(not_exercised)))


def get_json(url: str, *, timeout: float = 5.0) -> Any:
    """Issue a bounded, read-only GET and decode its JSON response."""
    parsed = urlsplit(url)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("acceptance endpoints must be HTTP(S) loopback URLs without embedded credentials")
    try:
        is_loopback = ipaddress.ip_address(parsed.hostname).is_loopback
    except ValueError:
        is_loopback = parsed.hostname.casefold() == "localhost"
    if not is_loopback:
        raise ValueError("acceptance endpoints must resolve to localhost or a loopback IP")
    headers = {"Accept": "application/json"}
    token = os.environ.get("PULSO_ACCEPTANCE_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = Request(url, headers=headers, method="GET")
    opener = build_opener(_RejectRedirects())
    with opener.open(request, timeout=timeout) as response:
        body = response.read(MAX_RESPONSE_BYTES + 1)
        if len(body) > MAX_RESPONSE_BYTES:
            raise ValueError("acceptance response exceeds size limit")
        return json.loads(body.decode("utf-8"))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--proposal-url", required=True, help="HTTP GET URL for one registry proposal")
    parser.add_argument("--history-url", help="Optional approved HTTP GET URL for proposal authoring history")
    parser.add_argument("--quota-url", help="Optional approved HTTP GET URL for a quota observation")
    parser.add_argument("--timeout", type=float, default=5.0)
    args = parser.parse_args(argv)
    try:
        proposal = get_json(args.proposal_url, timeout=args.timeout)
        history = get_json(args.history_url, timeout=args.timeout) if args.history_url else None
        quota = get_json(args.quota_url, timeout=args.timeout) if args.quota_url else None
    except (HTTPError, URLError, TimeoutError, OSError, ValueError) as exc:
        print(f"acceptance blocked: HTTP endpoint unavailable or invalid ({type(exc).__name__})", file=sys.stderr)
        return 2
    resource_id = unquote(urlsplit(args.proposal_url).path.rstrip("/").rsplit("/", 1)[-1])
    if not resource_id:
        print("acceptance blocked: proposal URL does not identify a resource", file=sys.stderr)
        return 2
    result = check_acceptance(
        proposal, expected_proposal_id=resource_id, history=history, quota=quota
    )
    print(json.dumps({
        "status": "fail" if result.failures else "not_exercised" if result.not_exercised else "pass",
        "failures": result.failures,
        "not_exercised": result.not_exercised,
    }, sort_keys=True))
    return result.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
