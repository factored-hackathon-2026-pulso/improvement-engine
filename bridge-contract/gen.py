#!/usr/bin/env python
"""Generates the published `/internal/v1` contract from the REAL core-bridge source.

    python gen.py            # (re)write contract.json, schemas/*.schema.json, openapi/bridge-internal-v1.yaml
    python gen.py --check    # exit 1 when a checked-in artifact differs from what the source derives (drift gate)

Request DTOs come from the pydantic models in `pulso_core_runtime` (`model_json_schema`), the stage rules from
`stages.catalog.CATALOG`, limits/TTLs/state machine from module constants, the route table from
`internal.app.ROUTES`, the fact schemas from `facts/schemas/*.strict.json`. Response DTOs that have no model in the
source (they are built as dicts) are declared here ONCE, with every closed list read from the source. The wire error
codes are checked against a scan of the source: a `pulso:*` literal that is neither classified below nor listed as
non-wire makes the build fail, so a new error code cannot ship without a contract decision.

Alias read (CAP-08) and authoring dry-run (CAP-16 L2) are implemented and reviewed: their DTOs are generated from the
runtime models like every other route (no `x-status` marker remains)."""

from __future__ import annotations

import argparse
import copy
import json
import re
import sys
from pathlib import Path
from typing import Any, get_args

import yaml

HERE = Path(__file__).resolve().parent
SRC_ROOT = HERE.parent / "core-bridge" / "src"
SRC = SRC_ROOT / "pulso_core_runtime"
if str(SRC_ROOT) not in sys.path:
    sys.path.insert(0, str(SRC_ROOT))

CONTRACT_REVISION = "pulso-two-teams-1"
CONTRACT_SCHEMA_VERSION = "1"
SCHEMA_DRAFT = "https://json-schema.org/draft/2020-12/schema"


class GenError(Exception):
    """The source moved in a way the contract does not classify yet."""


# --------------------------------------------------------------------------------------------------------------
# Closed code classification. `routes` lists the route ids (see `route_id`) that can answer with the code; "*" = all.
# Statuses are the ones the source uses (verified against a regex scan of the source below).
# --------------------------------------------------------------------------------------------------------------
INV, RD, CRED, ADM, ARM, VER, ALIAS, DRY = ("invoke", "read_task", "issue_credential", "admit_evaluation",
                                           "run_arm", "version", "read_alias", "authoring_dry_run")
READ_ARM = "read_arm"
ALL = "*"

WIRE_CODES: dict[str, dict[str, Any]] = {
    # auth / envelope (internal/app.py)
    "pulso:auth_invalid": {"status": [401], "retryable": False, "routes": [ALL],
                           "doc": "Missing/malformed/expired/unknown-kid/bad-signature/replayed token; "
                                  "`details.reason` is a closed reason (see auth.reasons)."},
    "pulso:auth_denied": {"status": [403], "retryable": False, "routes": [ALL],
                          "doc": "Valid token, wrong purpose or missing tenant claim (`details.reason`); "
                                 "also admissions without a `job_id` claim."},
    "pulso:tenant_mismatch": {"status": [403], "retryable": False, "routes": [ALL],
                              "doc": "Verified tenant claim outside the deployment tenant set, or body tenant != claim."},
    "pulso:invalid_request": {"status": [422], "retryable": False, "routes": [ALL],
                              "doc": "Body/params fail the DTO; `details.fields` lists offending paths (names only)."},
    "pulso:not_found": {"status": [404], "retryable": False, "routes": [RD, ARM, READ_ARM, ALL],
                        "doc": "Unknown path or a ref that is not visible to the caller's tenant (no existence oracle)."},
    "pulso:http_error": {"status": [405], "retryable": False, "routes": [ALL],
                         "doc": "Other framework-level HTTP error (e.g. method not allowed)."},
    "pulso:internal_error": {"status": [500], "retryable": True, "routes": [ALL], "doc": "Unhandled bridge error."},
    "pulso:not_implemented": {"status": [501], "retryable": False, "routes": [ALL],
                              "doc": "Route authenticated but no handler is wired."},
    "pulso:payload_too_large": {"status": [413], "retryable": False, "routes": [ALL],
                                "doc": "Body > MAX_BODY_BYTES (declared length or actual bytes)."},
    # invoke / read
    "pulso:input_too_large": {"status": [413], "retryable": False, "routes": [INV],
                              "doc": "JCS(input + input_artifact_refs) > MAX_INPUT_BYTES."},
    "pulso:unknown_input_slot": {"status": [400], "retryable": False, "routes": [INV],
                                 "doc": "`input` has a key the stage does not declare (`details.slots`)."},
    "pulso:stage_unknown": {"status": [400], "retryable": False, "routes": [INV], "doc": "stage not in STAGES."},
    "pulso:stage_agent_mismatch": {"status": [422], "retryable": False, "routes": [INV],
                                   "doc": "agent_id is not the stage's catalogue agent; zero effects."},
    "pulso:digest_conflict": {"status": [409], "retryable": False, "routes": [INV],
                              "doc": "Same Idempotency-Key, different request digest."},
    "pulso:bridge_busy": {"status": [429], "retryable": True, "routes": [INV],
                          "doc": "In-flight cap reached; nothing was sent, the same key retries cleanly."},
    "pulso:core_unavailable": {"status": [503], "retryable": True, "routes": [INV], "doc": "Core not reachable."},
    "pulso:release_pin_unavailable": {"status": [409], "retryable": False, "routes": [INV],
                                      "doc": "Release missing / not active / agent version or closure digest differs."},
    "pulso:release_revoked": {"status": [409], "retryable": False, "routes": [INV], "doc": "Release revoked."},
    "pulso:release_drift": {"status": [409], "retryable": False, "routes": [INV], "doc": "Release drifted from the pin."},
    "pulso:run_not_found": {"status": [404], "retryable": False, "routes": [RD, INV], "doc": "Core run not found."},
    "pulso:output_missing": {"status": [422], "retryable": False, "routes": [INV, RD],
                             "doc": "Completed run without the required stage fact."},
    "pulso:output_too_large": {"status": [413], "retryable": False, "routes": [INV, RD],
                               "doc": "A fact > FACT_CAP or the whole result > RESULT_CAP."},
    "pulso:fact_schema_violation": {"status": [422], "retryable": False, "routes": [INV, RD],
                                    "doc": "Fact fails its strict schema, has a non-integer JSON number, or cites an "
                                           "evidence ref that is not in the invocation call log."},
    "pulso:canary_detected": {"status": [422], "retryable": False, "routes": [INV, RD],
                              "doc": "A configured canary string appeared in a fact."},
    "pulso:task_unknown": {"status": [200], "retryable": False, "routes": [RD],
                           "doc": "Carried as `code` inside a 200 CoreTaskReceipt body (state=unknown)."},
    "pulso:task_in_progress": {"status": [200], "retryable": False, "routes": [RD],
                               "doc": "Carried as `code` inside a 200 CoreTaskReceipt body (non-terminal state)."},
    # credentials
    "pulso:credential_not_issuable": {"status": [403], "retryable": False, "routes": [CRED],
                                      "doc": "(purpose, role) pair is not in POLICY (humans/approval roles never)."},
    "pulso:credential_signing_unavailable": {"status": [503], "retryable": True, "routes": [CRED],
                                             "doc": "Signer missing or signing failed."},
    # evaluation
    "pulso:evaluation_context_invalid": {"status": [422], "retryable": False, "routes": [ADM],
                                         "doc": "evaluation_context_ref outside [A-Za-z0-9_.:-]{1,200}."},
    "pulso:broker_denied": {"status": [403], "retryable": False, "routes": [ADM, ARM],
                            "doc": "Lab broker authorization check refused or failed (fail closed)."},
    "pulso:admission_expired": {"status": [409], "retryable": False, "routes": [ADM], "doc": "deadline <= now."},
    "pulso:budget_unknown": {"status": [403], "retryable": False, "routes": [ADM], "doc": "budget_ref not resolvable."},
    "pulso:proposal_not_found": {"status": [404], "retryable": False, "routes": [ADM], "doc": "Unknown proposal."},
    "pulso:candidate_changed": {"status": [409], "retryable": False, "routes": [ADM],
                                "doc": "Proposal is not `candidate` or its candidate_hash moved."},
    "pulso:suite_mismatch": {"status": [409], "retryable": False, "routes": [ADM],
                             "doc": "suite_digest differs from the bridge's own."},
    "pulso:idempotency_conflict": {"status": [409], "retryable": False, "routes": [ADM, ARM],
                                   "doc": "Same key (context ref / arm key) with a different body."},
    "pulso:mixed_world_rejected": {"status": [409], "retryable": False, "routes": [ARM],
                                   "doc": "native mode with a bank pointer."},
    "pulso:sandbox_required": {"status": [409], "retryable": False, "routes": [ARM],
                               "doc": "task mode without a sandbox / seed_manifest_ref."},
    "pulso:idempotency_key_invalid": {"status": [422], "retryable": False, "routes": [ARM],
                                      "doc": "Arm key outside [A-Za-z0-9_.:-]{1,200}."},
    "pulso:supersedes_invalid": {"status": [409], "retryable": False, "routes": [ARM],
                                 "doc": "supersedes_execution_id is not a reconciled `unknown` arm."},
    # runtime-internal admission checks that surface on the evaluate path (tool side) but are part of the closed
    # admission vocabulary the Rust side may see inside a writer receipt/native_evaluation reason
    "pulso:evaluation_in_progress": {"status": [409], "retryable": True, "routes": [ADM],
                                     "doc": "A prior evaluation for the same admission is still running."},
    "pulso:evaluation_unknown": {"status": [409], "retryable": False, "routes": [ADM],
                                 "doc": "A prior evaluation outcome is unknown: reconcile, never start a new one."},
    "pulso:evaluation_attempt_exists": {"status": [409], "retryable": False, "routes": [ADM],
                                        "doc": "(tenant, proposal, candidate, attempt) already admitted under another ref."},
    "pulso:admission_missing": {"status": [403], "retryable": False, "routes": [ADM], "doc": "No such admission."},
    "pulso:admission_not_admitted": {"status": [409], "retryable": False, "routes": [ADM],
                                     "doc": "Admission is consumed/expired."},
    "pulso:admission_cross_tenant": {"status": [403], "retryable": False, "routes": [ADM], "doc": "Other tenant's admission."},
    "pulso:admission_attempt_mismatch": {"status": [403], "retryable": False, "routes": [ADM], "doc": "Attempt differs."},
    "pulso:admission_proposal_mismatch": {"status": [403], "retryable": False, "routes": [ADM], "doc": "Proposal differs."},
    "pulso:binding_not_confirmed": {"status": [403], "retryable": False, "routes": [ADM], "doc": "Binding not confirmed."},
    "pulso:evaluate_disabled": {"status": [403], "retryable": False, "routes": [ADM], "doc": "evaluate_enabled=false."},
    "pulso:evaluation_context_missing": {"status": [403], "retryable": False, "routes": [ADM], "doc": "No context ref."},
    # authoring routes (V3 CAP-16/17/08)
    "pulso:dry_run_unavailable": {"status": [503], "retryable": True, "routes": [DRY],
                                  "doc": "The dry-run engine is unavailable."},
    "pulso:base_release_unknown": {"status": [404], "retryable": False, "routes": [DRY],
                                   "doc": "base_release_id does not exist for the agent."},
    "pulso:release_settings_not_allowed": {"status": [422], "retryable": False, "routes": [DRY],
                                           "doc": "A change targets release_settings (default deny, CAP-23)."},
    "pulso:alias_unknown": {"status": [404], "retryable": False, "routes": [ALIAS],
                            "doc": "Alias not set for the agent."},
}

# `pulso:*` literals that exist in the source but never appear in an `/internal/v1` HTTP response: startup/config
# failures, tool-side errors returned to the model, and receipt `reason`s. Classified so a NEW literal is a decision.
NON_WIRE_CODES: dict[str, str] = {
    "pulso:adapter_missing": "startup", "pulso:demo_double_in_real_mode": "startup",
    "pulso:pin_symbol_drift": "startup", "pulso:runtime_config_invalid": "startup",
    "pulso:eval_db_misconfigured": "startup", "pulso:eval_db_not_isolated": "startup",
    "pulso:eval_db_unreachable": "startup", "pulso:exporter_overprivileged": "exporter",
    "pulso:schema_drift": "exporter",
    "pulso:artifact_digest_mismatch": "tool", "pulso:artifact_final_locked": "tool",
    "pulso:artifact_malformed": "tool", "pulso:artifact_too_large": "tool",
    "pulso:authorization_denied": "tool", "pulso:binding_cas_lost": "tool", "pulso:binding_failed": "receipt_reason",
    "pulso:binding_unconfirmed": "receipt_reason", "pulso:broker_": "tool", "pulso:broker_timeout": "tool",
    "pulso:broker_unavailable": "tool", "pulso:commitment_mismatch": "tool",
    "pulso:context_mismatch": "tool", "pulso:context_missing": "tool", "pulso:evaluation": "tool",
    "pulso:evaluation_context_ref_invalid": "tool", "pulso:evaluation_exception": "tool",
    "pulso:evaluation_gate_unavailable": "tool", "pulso:executor_exception": "tool",
    "pulso:lab_query_failed": "tool", "pulso:lab_query_timeout": "tool", "pulso:lab_query_unknown": "tool",
    "pulso:no_extract_manifest": "tool", "pulso:no_memory_snapshot": "tool",
    "pulso:release_settings_not_allowed": "tool", "pulso:tool_internal_error": "tool",
    "pulso:write_without_key": "tool",
}
# Codes the source spells without the `pulso:` prefix (derived names, e.g. `Denied("budget_unknown", 403)`).
SCAN_SKIP_DIRS: tuple[str, ...] = ()

# Receipt `reason` values (not codes). Dynamic suffix families are listed with a `*`.
RECEIPT_REASONS = ["completed", "run_failed", "unexpected_outcome", "core_call_failed", "post_send_exception",
                   "projection_failed", "binding_unconfirmed", "binding_unproven", "core_idempotency_conflict",
                   "core_http_*", "core_rejected_*", "release_drift", "release_pin_unavailable", "release_revoked",
                   "output_missing", "output_too_large", "fact_schema_violation", "canary_detected"]
ARM_REASONS = ["broker_denied", "commitment_mismatch", "closure_invalid", "unknown_target_kind", "release_not_active",
               "proposal_not_frozen", "proposal_mismatch", "draft_plan_digest_mismatch", "base_release_missing",
               "candidate_invalid", "candidate_hash_mismatch", "target_load_failed", "manifest_missing",
               "manifest_empty", "budget_unknown", "mixed_world_rejected", "sandbox_timeout_after_send",
               "schema_violation", "action_without_readback", "interrupted", "runner_error"]


def route_id(method: str, path: str) -> str:
    return {("POST", "/core-tasks/invoke"): INV, ("GET", "/core-tasks/{task_id}"): RD,
            ("GET", "/core-state/aliases"): ALIAS, ("GET", "/core-state/aliases/{agent_id}/{alias}"): ALIAS,
            ("POST", "/core-authoring/dry-run"): DRY, ("GET", "/version"): VER,
            ("POST", "/core-credentials/issue"): CRED, ("POST", "/evaluation/admissions"): ADM,
            ("POST", "/evaluation/arms/run"): ARM, ("POST", "/evaluation/arms/{arm_id}/run"): ARM,
            ("GET", "/evaluation/arms/by-key/{key}"): READ_ARM,
            ("GET", "/evaluation/arms/{arm_id}"): READ_ARM}[(method, path)]


def scan_source_codes() -> set[str]:
    found: set[str] = set()
    for py in SRC.rglob("*.py"):
        if py.relative_to(SRC).parts[0] in SCAN_SKIP_DIRS:
            continue
        text = py.read_text(encoding="utf-8")
        found |= set(re.findall(r"pulso:[a-z_]+", text))
        # AdmissionDenied/ArmDenied spell the bare name; routes prefix it with `pulso:`.
        found |= {f"pulso:{m}" for m in re.findall(r"(?:Admission|Arm)Denied\(\s*\"([a-z_]+)\"", text)}
    return found


def scan_statuses() -> dict[str, set[int]]:
    """Statuses the source attaches to a code (best effort over the spellings the source actually uses)."""
    out: dict[str, set[int]] = {}
    pats = [r"BridgeError\(\s*\"(pulso:[a-z_]+)\"\s*,\s*(\d{3})", r"(?:Admission|Arm)Denied\(\s*\"([a-z_]+)\"\s*,\s*(\d{3})",
            r"_err\(\s*\"(pulso:[a-z_]+)\"\s*,\s*(\d{3})", r"envelope\(\s*\"(pulso:[a-z_]+)\"[^\n]*?status=(\d{3})"]
    for py in SRC.rglob("*.py"):
        if py.relative_to(SRC).parts[0] in SCAN_SKIP_DIRS:
            continue
        text = py.read_text(encoding="utf-8")
        for pat in pats:
            for name, status in re.findall(pat, text):
                code = name if name.startswith("pulso:") else f"pulso:{name}"
                out.setdefault(code, set()).add(int(status))
    return out


def check_codes() -> None:
    scanned = scan_source_codes()
    known = set(WIRE_CODES) | set(NON_WIRE_CODES)
    # task_unknown/task_in_progress are built with a ternary; always present in the scan
    missing = sorted(scanned - known)
    stale = sorted(known - scanned)
    if missing or stale:
        raise GenError(f"error-code classification drift. unclassified in src: {missing}; stale in gen.py: {stale}. "
                       "Classify the code in WIRE_CODES / NON_WIRE_CODES (gen.py).")
    for code, statuses in scan_statuses().items():
        if code in WIRE_CODES:
            extra = statuses - set(WIRE_CODES[code]["status"]) - ({404, 422, 400} if code == "pulso:run_not_found" else set())
            if extra:
                raise GenError(f"{code}: src uses statuses {sorted(extra)} not listed in gen.py")


# --------------------------------------------------------------------------------------------------------------
# Schemas
# --------------------------------------------------------------------------------------------------------------
def _schema(name: str, body: dict[str, Any], doc: str | None = None) -> dict[str, Any]:
    out = {"$schema": SCHEMA_DRAFT, "$id": f"{name}.schema.json", "title": name}
    if doc:
        out["description"] = doc
    out.update(body)
    return out


def _obj(required: list[str], props: dict[str, Any], *, extra: bool = False) -> dict[str, Any]:
    return {"type": "object", "additionalProperties": extra, "required": required, "properties": props}


S = {"type": "string"}
SV1 = {"const": "1", "type": "string"}
NULLABLE_S = {"type": ["string", "null"]}
SHA256_HEX = {"type": "string", "pattern": "^[0-9a-f]{64}$"}
REF = {"$ref": "ArtifactRef.schema.json"}


def _strip(schema: Any) -> Any:
    """Remove pydantic noise that carries no contract meaning (titles of every property)."""
    if isinstance(schema, dict):
        return {k: _strip(v) for k, v in schema.items() if k != "title" or isinstance(v, str) is False}
    if isinstance(schema, list):
        return [_strip(v) for v in schema]
    return schema


def _from_model(model: type, name: str, doc: str) -> dict[str, Any]:
    raw = model.model_json_schema(mode="validation", ref_template="#/$defs/{model}")
    body = {k: v for k, v in _strip(raw).items() if k != "description"}  # the contract text replaces the docstring
    return _schema(name, body, doc) | {"title": name}


def _inline_defs(schema: dict[str, Any]) -> dict[str, Any]:
    """Replaces `#/$defs/X` refs by the definitions (one level, no recursion) and drops `$defs`."""
    defs = schema.pop("$defs", {})

    def walk(node: Any) -> Any:
        if isinstance(node, dict):
            ref = node.get("$ref")
            if isinstance(ref, str) and ref.startswith("#/$defs/"):
                return walk(copy.deepcopy(defs[ref.rsplit("/", 1)[-1]]))
            return {k: walk(v) for k, v in node.items()}
        if isinstance(node, list):
            return [walk(v) for v in node]
        return node

    return dict(walk(schema))


def build_schemas() -> dict[str, dict[str, Any]]:
    from pulso_core_runtime.credentials.issuer import MAX_TTL, POLICY
    from pulso_core_runtime.evaluation.admission import CONTEXT_REF_RE, AdmissionState
    from pulso_core_runtime.evaluation.arms import ArmRequest, ArmStatus
    from pulso_core_runtime.evaluation.routes import AdmissionRequest
    from pulso_core_runtime.invoke.models import (
        DIGEST_EXCLUDED,
        MAX_INPUT_BYTES,
        STAGES,
        CoreTaskInvocation,
    )
    from pulso_core_runtime.invoke.projection import FACT_CAP, RESULT_CAP
    from pulso_core_runtime.invoke.routes import CredentialRequest
    from pulso_core_runtime.stages.catalog import CATALOG
    from pulso_core_runtime.store.receipts import STATES

    schemas: dict[str, dict[str, Any]] = {}

    schemas["ArtifactRef"] = _schema("ArtifactRef", _obj(["id", "digest", "media_type"], {
        "id": S, "digest": S, "media_type": S}), "Annex D.1 ArtifactRef: treated data only, never a path/DSN/URL.")

    codes = sorted(WIRE_CODES)
    schemas["ErrorEnvelope"] = _schema("ErrorEnvelope", _obj(
        ["schema_version", "code", "retryable", "trace_id", "details"], {
            "schema_version": SV1, "code": {"type": "string", "enum": codes}, "retryable": {"type": "boolean"},
            "trace_id": S, "details": {"type": "object"}}),
        "Annex D.1 private error envelope (DR-34); `code` is a closed list. Core problem+json never appears here.")

    # -- invoke request: the pydantic model plus the validators/catalogue rules JSON Schema cannot see ------------
    inv = _from_model(CoreTaskInvocation, "CoreTaskInvocation",
                      "CAP-25 invocation (generated from invoke.models.CoreTaskInvocation).")
    defs = inv.pop("$defs", {})
    commitment = defs.get("RegistryMutationCommitmentDTO")
    inv["properties"]["registry_mutation_commitment"] = {
        "oneOf": [{"$ref": "RegistryMutationCommitment.schema.json"}, {"type": "null"}], "default": None}
    inv["properties"]["stage"] = {"type": "string", "enum": list(STAGES)}
    inv["x-digest-excluded"] = list(DIGEST_EXCLUDED)
    inv["x-max-input-bytes"] = MAX_INPUT_BYTES
    rules: list[dict[str, Any]] = []
    for name, spec in CATALOG.items():
        rule: dict[str, Any] = {
            "if": {"properties": {"stage": {"const": name}}, "required": ["stage"]},
            "then": {"properties": {"agent_id": {"const": spec.agent_id},
                                    "input": {"type": "object", "additionalProperties": False,
                                              "properties": {s: {} for s in spec.input_slots}}}}}
        if name != "writer":
            rule["then"]["properties"]["registry_mutation_commitment"] = {"type": "null"}
        rules.append(rule)
    inv["allOf"] = rules
    schemas["CoreTaskInvocation"] = inv
    commit = _schema("RegistryMutationCommitment", _strip(copy.deepcopy(commitment or {})),
                     "Writer-only sealed commitment (D.2; inside the digested body).")
    commit.pop("title", None)
    commit["title"] = "RegistryMutationCommitment"
    schemas["RegistryMutationCommitment"] = commit

    # -- receipt / read ------------------------------------------------------------------------------------------
    fact_names = [spec.fact for spec in CATALOG.values()]
    schemas["ReadRunResult"] = _schema("ReadRunResult", _obj(
        ["schema_version", "core_run_id", "status", "outcome", "facts", "output_digest"], {
            "schema_version": SV1, "core_run_id": S, "status": S, "outcome": S,
            "facts": {"type": "object", "propertyNames": {"enum": fact_names},
                      "additionalProperties": _obj(["value", "source_kind", "digest"], {
                          "value": {}, "source_kind": S, "digest": SHA256_HEX})},
            "output_digest": SHA256_HEX}),
        f"CAP-28 projection: whitelisted facts only; each fact <= {FACT_CAP} B, result <= {RESULT_CAP} B (JCS).")
    schemas["CoreTaskReceiptDetail"] = _schema("CoreTaskReceiptDetail", _obj(
        ["schema_version", "core_run_id", "release_id", "outcome", "trace_id", "idempotency_key_digest",
         "request_digest", "task_binding_ref", "input_commitment", "output_refs", "output_digest", "audit_refs",
         "budget"], {
            "schema_version": SV1, "core_run_id": S, "release_id": S, "outcome": S, "trace_id": NULLABLE_S,
            "idempotency_key_digest": SHA256_HEX, "request_digest": {"type": "string"}, "task_binding_ref": S,
            "input_commitment": SHA256_HEX, "output_refs": {"type": "array"}, "output_digest": {
                "type": ["string", "null"]}, "audit_refs": {"type": "array"},
            "budget": _obj(["known"], {"known": {"type": "boolean"}})}),
        "CAP-33 CoreTaskReceipt detail stored with a terminal receipt (`receipt` of the state body).")
    schemas["CoreTaskReceipt"] = _schema("CoreTaskReceipt", _obj(
        ["schema_version", "state", "core_run_id", "reason", "outcome", "task_binding_ref"], {
            "schema_version": SV1, "state": {"type": "string", "enum": list(STATES)},
            "core_run_id": NULLABLE_S, "reason": NULLABLE_S, "outcome": NULLABLE_S, "task_binding_ref": S,
            "receipt": {"$ref": "CoreTaskReceiptDetail.schema.json"}, "result": {"$ref": "ReadRunResult.schema.json"},
            "proven_no_effect": {"type": "boolean"}, "adopted_writes": {"type": "array"},
            "code": {"type": "string", "enum": ["pulso:task_unknown", "pulso:task_in_progress"]},
            "trace_id": S}),
        "Body of invoke 200/202 and read 200. HTTP 202 = non-terminal (state in sent|binding_confirmed|unknown|"
        "manual_reconcile|prepared); 200 = terminal_ok|terminal_failed or a read. `code`/`trace_id` appear only on a "
        "non-terminal read.")
    schemas["CoreTaskReceipt"]["x-reasons"] = RECEIPT_REASONS
    for stage_spec in CATALOG.values():
        path = SRC / "facts" / "schemas" / f"{stage_spec.fact}.strict.json"
        doc = json.loads(path.read_text(encoding="utf-8"))
        name = f"Fact_{stage_spec.fact}"
        doc.pop("$id", None)
        doc.pop("$schema", None)
        schemas[name] = _schema(name, doc, f"D.2 fact `{stage_spec.fact}` of stage `{stage_spec.stage}` "
                                f"(facts/schemas/{stage_spec.fact}.strict.json).")
    schemas["FactPolicy"] = _schema("FactPolicy", {"type": "object", "const": {
        "stage_fact": {s.stage: s.fact for s in CATALOG.values()},
        "stage_input_slots": {s.stage: list(s.input_slots) for s in CATALOG.values()},
        "stage_agent": {s.stage: s.agent_id for s in CATALOG.values()}}},
        "Stage -> agent / fact / input slots as the catalogue defines them (informational constant).")

    # -- credentials / version -----------------------------------------------------------------------------------
    schemas["CoreCredentialIssueRequest"] = _from_model(CredentialRequest, "CoreCredentialIssueRequest",
                                                        "CAP-33 credential request.")
    schemas["CoreCredentialIssueRequest"]["x-policy"] = [
        {"purpose": p, "role": r, "signer": s} for (p, r), s in sorted(POLICY.items())]
    schemas["CoreCredentialIssue"] = _schema("CoreCredentialIssue", _obj(["jws", "kid", "exp"], {
        "jws": S, "kid": S, "exp": {"type": "integer"}}),
        f"Principal JWS (typ=principal+jws), TTL <= {int(MAX_TTL.total_seconds())} s, `Cache-Control: no-store`. "
        "Never persist or log `jws`.")
    schemas["CoreVersion"] = _schema("CoreVersion", _obj(
        ["agent_core_sha", "contracts_version", "pulso_sha", "image_digest", "runtime_profile", "doubles"], {
            "agent_core_sha": {"type": "string", "pattern": "^[0-9a-f]{40}$"}, "contracts_version": S,
            "pulso_sha": S, "image_digest": NULLABLE_S, "runtime_profile": S,
            "keys_reload_error": NULLABLE_S, "doubles": {"type": "array", "items": S}}, extra=True),
        "Plan 4.1: `GET /version`. The real runtime reports runtime_profile=agent_core_real; a mock reports its own.")

    # -- evaluation ----------------------------------------------------------------------------------------------
    adm = _from_model(AdmissionRequest, "EvaluationAdmissionRequest", "D.4 admission request.")
    adm["properties"]["evaluation_context_ref"] = {"type": "string", "pattern": f"^{CONTEXT_REF_RE.pattern}$"}
    schemas["EvaluationAdmissionRequest"] = adm
    schemas["EvaluationAdmission"] = _schema("EvaluationAdmission", _obj(
        ["schema_version", "evaluation_context_ref", "state"], {
            "schema_version": SV1, "evaluation_context_ref": S,
            "state": {"type": "string", "enum": list(get_args(AdmissionState))}}),
        "201 on first admission, 200 on an identical replay.")
    arm = _from_model(ArmRequest, "ArmRequest", "D.4 arm request (generated from evaluation.arms.ArmRequest).")
    arm["properties"]["idempotency_key"] = {"type": "string", "pattern": f"^{CONTEXT_REF_RE.pattern}$"}
    schemas["ArmRequest"] = arm
    arm_status = list(get_args(ArmStatus))
    schemas["ArmReport"] = _schema("ArmReport", _obj(["execution_id", "status"], {
        "execution_id": {"type": "string", "pattern": "^arm-[0-9a-f]{32}$"},
        "status": {"type": "string", "enum": arm_status}, "reason": NULLABLE_S, "detail": S,
        "case_ref": S, "arm": S, "repetition": {"type": "integer"}, "seed": {"type": ["integer", "string"]},
        "oracle_ref": NULLABLE_S, "target_commitment": NULLABLE_S, "event_refs": {"type": "array", "items": S},
        "effect_receipts": {"type": "array"}, "final_state_ref": NULLABLE_S, "initial_state_digest": NULLABLE_S,
        "usage": {"type": ["object", "null"]}, "cost_known": {"type": "boolean"}, "trace_id": S,
        "closed_early": {"type": "boolean"}, "closed_early_runs": {"type": "array", "items": S}}, extra=False),
        "ArmReport. `usage=null` is 'not available'; `cost_known=false` is not free. A denied/interrupted arm reads "
        "back as the minimal {execution_id,status,reason}.")
    schemas["ArmReport"]["x-reasons"] = ARM_REASONS

    # -- auth claims ---------------------------------------------------------------------------------------------
    schemas["ServiceJwtClaims"] = _schema("ServiceJwtClaims", _obj(
        ["iss", "aud", "sub", "tenant_id", "purpose", "iat", "exp", "jti"], {
            "iss": {"const": "control-api"}, "aud": {"const": "core-bridge"}, "sub": {"type": "string", "minLength": 1},
            "tenant_id": {"type": "string", "minLength": 1}, "purpose": S, "job_id": S, "iat": {"type": "integer"},
            "exp": {"type": "integer"}, "jti": {"type": "string", "minLength": 1}}, extra=False),
        "Rust -> bridge JWT payload (A03 class i). No `scope`. `tenant_id` is optional only on version_probe; "
        "`job_id` is required by evaluation_admit.")

    # -- exporter side ------------------------------------------------------------------------------------------
    schemas["ObservationEvent"] = _schema("ObservationEvent", _obj(
        ["kind", "level", "source_event", "native_event_id", "source_event_digest", "source_event_ref",
         "source_schema_ref", "source_run_ref", "source_sequence", "episode_ref", "goal_ref", "layer_mapping_ref",
         "observed_at", "trace_refs", "coverage_marker"], {
            "kind": {"const": "core_event"}, "level": {"enum": ["engine_event", "outbound_event"]},
            "source_event": {"type": ["object", "null"]}, "native_event_id": S,
            "source_event_digest": S, "source_event_ref": {"oneOf": [REF, {"type": "null"}]},
            "source_schema_ref": REF, "source_run_ref": NULLABLE_S,
            "source_sequence": {"type": ["integer", "null"], "minimum": 0}, "episode_ref": {"type": "null"},
            "goal_ref": {"type": "null"}, "layer_mapping_ref": {"type": "null"},
            "observed_at": {"type": "string", "pattern": r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$"},
            "trace_refs": {"type": "array"}, "coverage_marker": {"type": ["string", "null"]}}),
        "Exporter-side observation DTO (`pulso-observations-2`): audit rows carry source_event=null + "
        "source_event_digest(hash) + source_event_ref; registry/outbox rows carry the public projection.")
    schemas["ObservationBatch"] = _schema("ObservationBatch", _obj(
        ["contract_version", "source_id", "tenant_id", "partition", "scan_mode", "expected_cursor_revision",
         "from_seq", "to_seq", "cursor", "cut_ref", "events", "verification_receipts"], {
            "contract_version": {"const": "pulso-observations-2"}, "source_id": S, "tenant_id": S, "partition": S,
            "scan_mode": {"enum": ["fast_poll", "rescan"]},
            "expected_cursor_revision": {"type": ["integer", "null"]}, "from_seq": {"type": ["integer", "null"]},
            "to_seq": {"type": ["integer", "null"]}, "cursor": S, "cut_ref": {"type": "null"},
            "events": {"type": "array", "items": {"$ref": "ObservationEvent.schema.json"}},
            "verification_receipts": {"type": "array"}, "batch_digest": SHA256_HEX}),
        "POST /internal/v1/platform/observations body; `batch_digest` = sha256(JCS(body without it)); ACK only "
        "after commit.")
    schemas["AuditAgentStepFailedEvidence"] = _schema("AuditAgentStepFailedEvidence", _obj(
        ["event_id", "hash", "payload", "run_id", "seq", "type"], {
            "event_id": S, "hash": S, "run_id": S, "seq": {"type": "integer", "minimum": 0},
            "schema_version": {"type": ["string", "null"]}, "type": {"const": "agent_step"},
            "payload": _obj(["kind", "error_kind", "node_id", "step", "latency_ms"], {
                "kind": {"const": "failed"}, "error_kind": S, "node_id": S, "step": {"type": "integer"},
                "latency_ms": {"type": "integer"}}, extra=False)}),
        "CX-0073/0075: ONLY a correlated audit `agent_step` with kind=failed + error_kind classifies a model "
        "dependency failure; the task receipt keeps its own outcome. Prompts/inputs never appear.")

    # -- alias read / authoring dry-run (V3 CAP-08/16/17/33, annex D) ---------------------------------------------------------------------
    schemas["AliasState"] = _schema("AliasState", _obj(
        ["schema_version", "agent_id", "alias", "release_id"], {
            "schema_version": SV1, "agent_id": S, "alias": {"enum": ["staging", "prod"]},
            "release_id": NULLABLE_S, "status": NULLABLE_S, "source": S,
            "observed_at": {"type": "string", "pattern": r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$"},
            "runtime_profile": S}, extra=True),
        "V3 CAP-08/33 AliasState (read-only observation of an alias; no write path).")
    from pulso_core_runtime.authoring.service import ALIASES, DryRunRequest

    schemas["AliasState"]["properties"]["alias"] = {"enum": sorted(ALIASES)}
    dry = _from_model(DryRunRequest, "CoreAuthoringDryRunRequest",
                      "V3 CAP-16 L2 request (generated from authoring.service.DryRunRequest). "
                      "More than 50 changes / 262144 canonical bytes per entity is a REG-LIMIT violation in the "
                      "response, not a 4xx.")
    schemas["CoreAuthoringDryRunRequest"] = _inline_defs(dry)
    schemas["CoreAuthoringDryRun"] = _schema("CoreAuthoringDryRun", _obj(
        ["schema_version", "violations", "candidate_hash"], {
            "schema_version": SV1, "valid": {"type": "boolean"},
            "violations": {"type": "array", "items": _obj(["rule", "message"], {
                "rule": S, "path": NULLABLE_S, "flow": NULLABLE_S, "node_id": NULLABLE_S, "message": S}, extra=True)},
            "candidate_hash": {"type": ["string", "null"], "pattern": "^[0-9a-f]{64}$"},
            "release_hash": {"type": ["string", "null"]}, "release_id_preview": NULLABLE_S,
            "auto_bumped": {"type": "array"}, "new_versions": {"type": "array"}, "content_hashes": {"type": "object"},
            "request_digest": SHA256_HEX, "proposal_created": {"const": False}, "runtime_profile": S}, extra=True),
        "V3 CAP-16/17 dry-run result. HTTP 200 with non-empty `violations` is not success "
        "(candidate_hash null); `release_id_preview` = 'rel-' + candidate_hash[:16]; no proposal is created.")
    return schemas


# --------------------------------------------------------------------------------------------------------------
# contract.json
# --------------------------------------------------------------------------------------------------------------
def build_contract() -> dict[str, Any]:
    from pulso_core_runtime import CONTRACTS_VERSION, PIN_SHA
    from pulso_core_runtime.credentials.issuer import MAX_TTL, POLICY
    from pulso_core_runtime.internal.app import MAX_BODY_BYTES, ROUTES, TENANT_EXEMPT
    from pulso_core_runtime.internal.auth import MAX_TTL_S
    from pulso_core_runtime.invoke.models import MAX_INPUT_BYTES, STAGES
    from pulso_core_runtime.invoke.projection import FACT_CAP, RESULT_CAP
    from pulso_core_runtime.invoke.service import InvokeSettings
    from pulso_core_runtime.stages.catalog import CATALOG
    from pulso_core_runtime.store.receipts import ALLOWED_FROM, STATES, TERMINAL

    check_codes()
    routes = []
    for r in ROUTES:
        purposes = sorted(r.purposes)
        routes.append({"id": route_id(r.method, r.path), "method": r.method, "path": r.path, "audience": r.audience,
                       "purposes": purposes, "tenant_required": not r.purposes <= TENANT_EXEMPT})
    settings = InvokeSettings()
    return {
        "contract_revision": CONTRACT_REVISION, "schema_version": CONTRACT_SCHEMA_VERSION,
        "base_path": "/internal/v1",
        "pin": {"agent_core_sha": PIN_SHA, "contracts_version": CONTRACTS_VERSION},
        "routes": sorted(routes, key=lambda r: (r["path"], r["method"])),
        "auth": {
            "token": {"typ": "JWT", "alg": "EdDSA (verifier-fixed, never from payload)",
                      "header_exact_keys": ["alg", "kid", "typ"], "iss": "control-api", "aud": "core-bridge",
                      "claims_required": ["iss", "aud", "sub", "tenant_id", "purpose", "iat", "exp", "jti"],
                      "claims_optional": ["job_id"], "sub_pattern": "^worker:.+ (else 403 pulso:auth_denied reason sub_not_worker)",
                      "job_id_rule": "invoke: claim job_id must equal body job_id (403 pulso:auth_denied reason job_mismatch)", "scope_claim": "absent (class i has no scope)",
                      "max_ttl_seconds": MAX_TTL_S, "max_exp_skew_seconds": 30,
                      "key_binding": "each kid is bound to exactly one (iss, aud)",
                      "jti": "receiver-owned durable replay store; (iss, jti) consumed atomically AFTER all other "
                             "checks pass (a rejected token never burns its jti); every HTTP attempt uses a fresh jti"},
            "reasons": ["missing_token", "malformed", "bad_header", "unknown_kid", "bad_signature", "key_binding",
                        "wrong_audience", "missing_claims", "expired", "ttl_too_long", "tenant_required",
                        "purpose_denied", "sub_not_worker", "job_mismatch", "jti_replayed"],
            "reason_status": {"tenant_required": 403, "purpose_denied": 403, "sub_not_worker": 403,
                              "job_mismatch": 403, "*": 401},
            "purposes": sorted({p for r in ROUTES for p in r.purposes}),
            "tenant_exempt_purposes": sorted(TENANT_EXEMPT)},
        "idempotency": {
            "invoke": {"header": "Idempotency-Key", "formula": "sha256_hex('{tenant_id}|{job_id}|{stage}|{attempt}|"
                                                                  "{logical_key}')",
                       "request_digest": "sha256_hex(JCS(body minus request_digest, credentials, trace))",
                       "same_key_same_digest": "replay: stored receipt (200 terminal / 202 non-terminal), no second Core run",
                       "same_key_other_digest": "409 pulso:digest_conflict",
                       "wrong_header": "422 pulso:invalid_request details.fields=[Idempotency-Key]",
                       "task_binding_ref": "sha256_hex('{tenant_id}|{idempotency_key}')"},
            "admissions": {"key": "body.evaluation_context_ref", "replay": "200 same body; 409 pulso:idempotency_conflict "
                                                                         "when any bound field differs",
                           "first": "201"},
            "arms": {"key": "body.idempotency_key (the Idempotency-Key header is not read by the runtime)",
                     "execution_id": "'arm-' + sha256_hex('{tenant_id}|{idempotency_key}')[:32]",
                     "replay": "200 stored report; other body -> 409 pulso:idempotency_conflict",
                     "readback": ["GET /evaluation/arms/{execution_id}", "GET /evaluation/arms/by-key/{key}"]}},
        "limits": {"max_body_bytes": MAX_BODY_BYTES, "max_input_bytes": MAX_INPUT_BYTES, "fact_cap_bytes": FACT_CAP,
                   "result_cap_bytes": RESULT_CAP, "credential_max_ttl_seconds": int(MAX_TTL.total_seconds()),
                   "default_max_inflight": settings.max_inflight},
        "receipt_state_machine": {
            "states": list(STATES), "terminal": sorted(TERMINAL),
            "transitions": {t: sorted(src) for t, src in sorted(ALLOWED_FROM.items())},
            "http": {"terminal": 200, "non_terminal": 202, "note": "a timeout after `sent` is `unknown`, never `failed`; "
                                                                  "`unknown` never licenses repeating the effect"}},
        "stages": {s.stage: {"agent_id": s.agent_id, "fact": s.fact, "input_slots": list(s.input_slots),
                             "writer_commitment": s.stage == "writer"} for s in CATALOG.values()},
        "stage_order": list(STAGES),
        "credential_policy": [{"purpose": p, "role": r, "signer": s} for (p, r), s in sorted(POLICY.items())],
        "error_codes": {"wire": {c: {k: v for k, v in e.items()} for c, e in sorted(WIRE_CODES.items())},
                        "non_wire": dict(sorted(NON_WIRE_CODES.items()))},
        "receipt_reasons": RECEIPT_REASONS, "arm_reasons": ARM_REASONS,
        "exporter": {"contract_version": "pulso-observations-2", "audience": "control-api",
                     "scopes": ["binding", "observations"], "note": "producer side only; Rust implements the receiver"},
    }


# --------------------------------------------------------------------------------------------------------------
# OpenAPI 3.1
# --------------------------------------------------------------------------------------------------------------
def _rewrite_refs(node: Any) -> Any:
    if isinstance(node, dict):
        out = {}
        for k, v in node.items():
            if k in ("$schema", "$id"):
                continue
            if k == "$ref" and isinstance(v, str) and v.endswith(".schema.json"):
                out[k] = "#/components/schemas/" + v.removesuffix(".schema.json")
            else:
                out[k] = _rewrite_refs(v)
        return out
    if isinstance(node, list):
        return [_rewrite_refs(v) for v in node]
    return node


def _component(schema: dict[str, Any]) -> dict[str, Any]:
    comp = _rewrite_refs(schema)
    defs = comp.pop("$defs", None)
    assert defs is None or not defs, "unexpected $defs"
    return comp


def _responses(route: dict[str, Any], ok: dict[int, str | None]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for status, schema in sorted(ok.items()):
        out[str(status)] = {"description": "OK" if status < 300 else "Accepted" if status == 202 else "Success",
                            "content": {"application/json": {"schema": {"$ref": f"#/components/schemas/{schema}"}}}}
    err_statuses: dict[int, list[str]] = {}
    for code, entry in WIRE_CODES.items():
        if entry["routes"] != [ALL] and route["id"] not in entry["routes"]:
            continue
        for st in entry["status"]:
            if st >= 300:
                err_statuses.setdefault(st, []).append(code)
    for st, cs in sorted(err_statuses.items()):
        out.setdefault(str(st), {
            "description": "Error envelope. Possible codes: " + ", ".join(sorted(cs)),
            "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ErrorEnvelope"}}}})
    return out


def build_openapi(schemas: dict[str, dict[str, Any]], contract: dict[str, Any]) -> dict[str, Any]:
    ok = {INV: ({200: "CoreTaskReceipt", 202: "CoreTaskReceipt"}, "CoreTaskInvocation"),
          RD: ({200: "CoreTaskReceipt"}, None), ALIAS: ({200: "AliasState"}, None),
          DRY: ({200: "CoreAuthoringDryRun"}, "CoreAuthoringDryRunRequest"), VER: ({200: "CoreVersion"}, None),
          CRED: ({200: "CoreCredentialIssue"}, "CoreCredentialIssueRequest"),
          ADM: ({200: "EvaluationAdmission", 201: "EvaluationAdmission"}, "EvaluationAdmissionRequest"),
          ARM: ({200: "ArmReport"}, "ArmRequest"), READ_ARM: ({200: "ArmReport"}, None)}
    paths: dict[str, Any] = {}
    for r in contract["routes"]:
        responses, body = ok[r["id"]]
        op: dict[str, Any] = {
            "operationId": f"{r['method'].lower()}_{r['id']}" + ("_by_key" if "by-key" in r["path"] else "")
                           + ("_by_id" if r["id"] in (ARM, READ_ARM) and "{arm_id}" in r["path"] else ""),
            "summary": r["id"], "security": [{"serviceJwt": []}],
            "x-pulso-auth": {"audience": r["audience"], "purposes": r["purposes"],
                             "tenant_required": r["tenant_required"]},
            "parameters": [{"$ref": "#/components/parameters/Traceparent"}],
            "responses": _responses(r, responses)}
        if r["id"] == INV:
            op["parameters"].append({"$ref": "#/components/parameters/IdempotencyKey"})
        for name in re.findall(r"{(\w+)}", r["path"]):
            op["parameters"].append({"name": name, "in": "path", "required": True, "schema": {"type": "string"}})
        if body:
            op["requestBody"] = {"required": True, "content": {"application/json": {
                "schema": {"$ref": f"#/components/schemas/{body}"}}}}
        if r.get("x-status"):
            op["x-status"] = r["x-status"]
        paths.setdefault("/internal/v1" + r["path"], {})[r["method"].lower()] = op
    components = {n: _component(s) for n, s in sorted(schemas.items())}
    return {
        "openapi": "3.1.0",
        "info": {"title": "Pulso core-bridge /internal/v1", "version": CONTRACT_REVISION,
                 "description": "Generated by bridge-contract/gen.py from core-bridge/src. Do not edit by hand.",
                 "x-contract-revision": CONTRACT_REVISION, "x-pin": contract["pin"]},
        "jsonSchemaDialect": SCHEMA_DRAFT,
        "servers": [{"url": "http://127.0.0.1:8000"}],
        "paths": dict(sorted(paths.items())),
        "components": {
            "securitySchemes": {"serviceJwt": {
                "type": "http", "scheme": "bearer", "bearerFormat": "JWT",
                "description": "Compact JWS typ=JWT alg=EdDSA, header exactly {alg,kid,typ}; see contract.json auth."}},
            "parameters": {
                "IdempotencyKey": {"name": "Idempotency-Key", "in": "header", "required": True, "schema": {
                    "type": "string", "pattern": "^[0-9a-f]{64}$"},
                    "description": "sha256_hex(tenant|job|stage|attempt|logical_key)"},
                "Traceparent": {"name": "traceparent", "in": "header", "required": False, "schema": {"type": "string"}}},
            "schemas": components}}


# --------------------------------------------------------------------------------------------------------------
def build() -> dict[str, str]:
    """relative path -> file text (deterministic)."""
    schemas = build_schemas()
    contract = build_contract()
    files: dict[str, str] = {"contract.json": json.dumps(contract, indent=2, sort_keys=True) + "\n"}
    for name, schema in schemas.items():
        files[f"schemas/{name}.schema.json"] = json.dumps(schema, indent=2, sort_keys=True) + "\n"
    files["openapi/bridge-internal-v1.yaml"] = yaml.safe_dump(build_openapi(schemas, contract), sort_keys=True,
                                                              allow_unicode=True, width=100)
    return files


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="fail when checked-in artifacts differ from the source")
    args = ap.parse_args(argv)
    try:
        files = build()
    except GenError as exc:
        print(f"gen.py: {exc}", file=sys.stderr)
        return 2
    managed = {"contract.json", *(f"schemas/{p.name}" for p in (HERE / "schemas").glob("*.schema.json")),
               "openapi/bridge-internal-v1.yaml"}
    if args.check:
        drift = [rel for rel, text in files.items()
                 if not (HERE / rel).is_file() or (HERE / rel).read_text(encoding="utf-8") != text]
        drift += [f"{rel} (stale, no longer generated)" for rel in sorted(managed - set(files))]
        if drift:
            print("gen.py --check: contract drift in " + ", ".join(drift) + "\nrun: python bridge-contract/gen.py",
                  file=sys.stderr)
            return 1
        print(f"gen.py --check: {len(files)} artifacts up to date")
        return 0
    for rel in sorted(managed - set(files)):
        (HERE / rel).unlink()
    for rel, text in files.items():
        path = HERE / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")
    print(f"gen.py: wrote {len(files)} artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
