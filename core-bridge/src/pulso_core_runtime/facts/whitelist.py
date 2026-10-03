"""Stage fact whitelist + strict/Core-subset schemas (plan 17.3.3 "Facts / `ReadRunResult`").

Only `run.facts[name]` for the stage whitelist is projected; slots, `token_map`, actions, principal, decisions and
pages are dropped. Core's `output_schema` is a closed subset (`type, enum, properties, required,
additionalProperties(bool), items`), so there are two schemas per fact: the Core-subset embedded in the Flow
(`stages.catalog`) and a strict JSON Schema 2020-12 here; `is_relaxation(core, strict)` is the CI assertion that
the Core subset accepts everything the strict one accepts. Limits: fact 128 KiB, result 256 KiB. Non-integer
JSON numbers are forbidden (decimals are strings). `source_kind="agent"` is never promoted."""

from __future__ import annotations

import hashlib
import json
from collections.abc import Collection, Mapping
from functools import cache
from pathlib import Path
from typing import Any

from agent_core.domain.json import canonical_bytes
from jsonschema import Draft202012Validator

from pulso_core_runtime.stages.catalog import CATALOG

SCHEMA_DIR = Path(__file__).parent / "schemas"
MAX_FACT_BYTES = 128 * 1024
MAX_RESULT_BYTES = 256 * 1024
CORE_SUBSET_KEYS = frozenset({"type", "enum", "properties", "required", "additionalProperties", "items"})

# Raw facts of the writer Flow that feed the bridge-composed `pulso_writer_receipts` projection.
WRITER_SOURCE_FACTS = ("proposal", "created", "put_verified", "reopen_verified", "validation", "frozen",
                       "freeze_verified",
                       "evaluation", "evaluate_verified")


class FactError(Exception):
    def __init__(self, code: str, detail: str = "") -> None:
        super().__init__(code + (f": {detail}" if detail else ""))
        self.code = code


def stage_facts(stage: str) -> tuple[str, ...]:
    spec = CATALOG[stage]
    return WRITER_SOURCE_FACTS if spec.projection else (spec.fact,)


@cache
def strict_schema(fact: str) -> dict[str, Any]:
    schema: dict[str, Any] = json.loads((SCHEMA_DIR / f"{fact}.strict.json").read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return schema


def core_schema(fact: str) -> dict[str, Any] | None:
    for spec in CATALOG.values():
        if spec.fact == fact:
            return spec.core_output_schema
    return None


def uses_only_core_subset(schema: Any) -> bool:
    if isinstance(schema, dict):
        for key, value in schema.items():
            if key not in CORE_SUBSET_KEYS:
                return False
            if key == "properties":
                if not all(uses_only_core_subset(v) for v in value.values()):
                    return False
            elif key == "items" and not uses_only_core_subset(value):
                return False
    return True


def _types(schema: dict[str, Any]) -> set[str]:
    t = schema.get("type")
    return {t} if isinstance(t, str) else set(t or [])


def is_relaxation(core: dict[str, Any], strict: dict[str, Any]) -> bool:
    """True when every document valid under `strict` is valid under `core` (structural check)."""
    if core.get("type") is not None and not _types(strict) <= _types(core):
        return False
    if "enum" in core and not ("enum" in strict and set(map(repr, strict["enum"])) <= set(map(repr, core["enum"]))):
        return False
    if not set(core.get("required", [])) <= set(strict.get("required", [])):
        return False
    core_props, strict_props = core.get("properties", {}), strict.get("properties", {})
    if core.get("additionalProperties") is False and not set(strict_props) <= set(core_props):
        return False
    for name, sub in core_props.items():
        if name in strict_props and not is_relaxation(sub, strict_props[name]):
            return False
    return not ("items" in core and ("items" not in strict or not is_relaxation(core["items"], strict["items"])))


def _walk_numbers(value: Any) -> None:
    if isinstance(value, float):
        raise FactError("pulso:fact_schema_violation", "non-integer number")
    if isinstance(value, dict):
        for v in value.values():
            _walk_numbers(v)
    elif isinstance(value, list):
        for v in value:
            _walk_numbers(v)


EVIDENCE_KEYS = ("evidence_refs", "counterevidence_refs")
EVIDENCE_FACTS = ("pulso_hypotheses", "pulso_verification", "pulso_change_spec")


def _collect_refs(value: Any, out: list[Mapping[str, Any]]) -> None:
    """Every ArtifactRef under `evidence_refs`/`counterevidence_refs` (D.2); non-object entries are kept so that
    a bare string or wiki path is rejected rather than silently skipped."""
    if isinstance(value, dict):
        for k, v in value.items():
            if k in EVIDENCE_KEYS and isinstance(v, list):
                out.extend(x if isinstance(x, Mapping) else {"id": x} for x in v)
            else:
                _collect_refs(v, out)
    elif isinstance(value, list):
        for v in value:
            _collect_refs(v, out)


def _norm_digest(value: Any) -> Any:
    return value.removeprefix("sha256:") if isinstance(value, str) else value


def _ref_in_call_log(ref: Mapping[str, Any], log: Collection[str]) -> bool:
    """`log` is `{artifact_id: {digest, media_type}}` (`InvocationRegistry.seen_artifacts`): the ref must name an
    artifact the invocation really fetched/produced, with the same digest and media type where those are known.
    A plain collection of ids (legacy) checks the id only."""
    rid = ref.get("id")
    if not isinstance(rid, str) or rid not in log:
        return False
    if not isinstance(log, Mapping):
        return True
    known = log[rid] or {}
    for field in ("digest", "media_type"):
        have = _norm_digest(known.get(field)) if field == "digest" else known.get(field)
        if have is not None and have != _norm_digest(ref.get(field)):
            return False
    return True


def _string_leaves(value: Any) -> list[str]:
    """Unescaped string keys/values: the JSON text escapes quotes, newlines and backslashes, hiding canaries."""
    out: list[str] = []
    if isinstance(value, str):
        out.append(value)
    elif isinstance(value, dict):
        for k, v in value.items():
            out.append(str(k))
            out.extend(_string_leaves(v))
    elif isinstance(value, list):
        for v in value:
            out.extend(_string_leaves(v))
    return out


def validate_fact(name: str, value: Any, *, canaries: Collection[str] = (),
                  call_log_refs: Collection[str] | None = None) -> str:
    """Validates one fact; returns its digest `sha256(JCS(value))`."""
    try:
        _walk_numbers(value)
        raw = canonical_bytes(value)
    except (TypeError, ValueError) as exc:
        raise FactError("pulso:fact_schema_violation", "not canonicalisable") from exc
    if len(raw) > MAX_FACT_BYTES:
        raise FactError("pulso:output_too_large", name)
    errors = sorted(Draft202012Validator(strict_schema(name)).iter_errors(value), key=lambda e: list(e.path))
    if errors:
        raise FactError("pulso:fact_schema_violation", f"{name}: {errors[0].message[:120]}")
    text = raw.decode("utf-8")
    if any(c and (c in text or any(c in leaf for leaf in _string_leaves(value))) for c in canaries):
        raise FactError("pulso:canary_detected", name)
    if call_log_refs is not None and name in EVIDENCE_FACTS:
        refs: list[Mapping[str, Any]] = []
        _collect_refs(value, refs)
        if not all(_ref_in_call_log(r, call_log_refs) for r in refs):
            raise FactError("pulso:fact_schema_violation", "evidence_ref not in the invocation call log")
    return hashlib.sha256(raw).hexdigest()


_ABSENT = object()


def _fact_parts(fact: Any) -> tuple[Any, str]:
    """`(value, source_kind)`; `value` is `_ABSENT` for a fact that lacks one (never a KeyError/AttributeError).
    A missing source is never promoted: it reads as `agent`."""
    if isinstance(fact, Mapping):
        source = fact.get("source")
        kind = source.get("kind") if isinstance(source, Mapping) else None
        return fact.get("value", _ABSENT), str(kind or "agent")
    source = getattr(fact, "source", None)
    return getattr(fact, "value", _ABSENT), str(getattr(source, "kind", None) or "agent")


def project_result(stage: str, *, core_run_id: str, status: str, outcome: str | None,
                   run_facts: Mapping[str, Any], canaries: Collection[str] = (),
                   call_log_refs: Collection[str] | None = None,
                   writer_receipts: dict[str, Any] | None = None) -> dict[str, Any]:
    """Envelope `{schema_version:"1", core_run_id, status, outcome, facts:{name:{value,source_kind,digest}},
    output_digest}`. For the writer, `writer_receipts` (from `compose_writer_receipts`) is the single fact."""
    spec = CATALOG[stage]
    facts: dict[str, dict[str, Any]] = {}
    if outcome == "completed":
        if spec.projection:
            if writer_receipts is None:
                raise FactError("pulso:output_missing", spec.fact)
            digest = validate_fact(spec.fact, writer_receipts, canaries=canaries)
            facts[spec.fact] = {"value": writer_receipts, "source_kind": "compute", "digest": digest}
        else:
            value, source_kind = _fact_parts(run_facts.get(spec.fact))
            if value is _ABSENT or value is None:
                raise FactError("pulso:output_missing", spec.fact)
            digest = validate_fact(spec.fact, value, canaries=canaries, call_log_refs=call_log_refs)
            facts[spec.fact] = {"value": value, "source_kind": source_kind, "digest": digest}
    envelope = {"schema_version": "1", "core_run_id": core_run_id, "status": status, "outcome": outcome,
                "facts": facts,
                "output_digest": hashlib.sha256(canonical_bytes({k: v["digest"] for k, v in facts.items()})).hexdigest()}
    if len(canonical_bytes(envelope)) > MAX_RESULT_BYTES:
        raise FactError("pulso:output_too_large", "result")
    return envelope


def _nonneg_int(value: Any) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else 0


def _tool_id(action: Mapping[str, Any]) -> str:
    tool = action.get("tool")
    return str(tool.get("id", "")) if isinstance(tool, Mapping) else str(action.get("tool_id", ""))


def compose_writer_receipts(run_facts: Mapping[str, Any], actions: list[Mapping[str, Any]], *,
                            evaluation_report: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Builds `pulso_writer_receipts` from verify-node facts and `RunState.actions[]`. `state` is `unknown`
    whenever any action is `executing|uncertain` without a verified readback. `evaluation_report`
    (`{eval_run_ref, report_digest}`, from the bridge's `eval_reports` store) completes `native_evaluation`."""

    def value(name: str) -> Any:
        got = _fact_parts(run_facts.get(name))[0]
        return None if got is _ABSENT else got

    proposal, frozen, evaluation = value("proposal"), value("frozen"), value("evaluation")
    verified_ops = {v.get("op"): v for v in (value(n) for n in ("created", "reopen_verified", "put_verified",
                                                                "freeze_verified", "evaluate_verified")) if isinstance(v, dict)}
    receipts: list[dict[str, Any]] = []
    for op in ("create_proposal", "reopen", "put_draft", "freeze", "evaluate"):
        rec = verified_ops.get(op)
        if rec is None:
            continue
        key = next((str(a.get("idempotency_key")) for a in actions if _tool_id(a) == f"registry/{op}"), "")
        receipts.append({"op": op, "key_digest": hashlib.sha256(key.encode()).hexdigest(),
                         "rev_after": _nonneg_int(rec.get("rev_after")), "request_hash": str(rec.get("request_hash", "")),
                         "verified": True})
    # unknown whenever any action is executing|uncertain without a verified readback for its op
    verified = set(verified_ops)
    unresolved = [a for a in actions if str(a.get("state")) in ("executing", "uncertain")
                  and _tool_id(a).removeprefix("registry/") not in verified]
    state = "confirmed" if receipts and not unresolved else "unknown"
    native = None
    if isinstance(evaluation, dict) and evaluation.get("verdict") is not None:
        report = evaluation_report or {}
        native = {"verdict": evaluation["verdict"], "eval_run_ref": report.get("eval_run_ref"),
                  "report_digest": report.get("report_digest")}
    pid = (proposal or {}).get("proposal_id") if isinstance(proposal, dict) else None
    return {"schema_version": "1", "proposal_id": pid or "", "rev": _nonneg_int(proposal.get("rev")) if isinstance(proposal, dict) else 0,
            "candidate_hash": frozen.get("candidate_hash") if isinstance(frozen, dict) else None,
            "native_evaluation": native, "write_receipts": receipts, "state": state}
