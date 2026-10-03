"""Assembles the REAL-results bundle the translators consume. Pure: takes already-produced analysis and Core outputs."""

from __future__ import annotations

from typing import Any

SCHEMA = "pulso-demo-results/1"
DOUBLES = [
    {"id": "stand_in_engine", "what": "Codex engine stand-in (e2e-core codex_standin): drives the REAL Core runtime; not the Rust engine",
     "until": "Rust engine HTTP API exists"},
    {"id": "scripted_llm", "what": "scripted llm-gateway double: tool calls/outputs are scripted from data-derived analysis, no real model"},
    {"id": "fixture_api", "what": "debug-console fixture API/SSE server fed from translated results; not the Rust control API"},
    {"id": "lab_query_rows", "what": "the e2e-fixtures lab double returns fixed rows for lab_query; the data-derived SQL results come from the "
                                    "demo's local sqlite dataset (recorded SQL and digests)"},
    {"id": "improvement_judge", "what": "demo's data-derived mechanism_proxy judge and reviser; stand-in for the Codex improvement judge"},
    {"id": "human_decision", "what": "approve/publish hook not connected: pending until a human decision flow runs"},
]


def human_doubles(decision: dict[str, Any] | None, offline: bool) -> list[dict[str, Any]]:
    """The human decision double(s), derived from what really happened (mode and stage), replacing the static `human_decision` entry."""
    if decision is None:
        return [d for d in DOUBLES if d["id"] == "human_decision"]
    if offline:
        return [{"id": "human_decision", "what": "SYNTHETIC offline approval flow (offline-* ids): no issuer, no Core; preview only"},
                {"id": "offline_human", "what": "synthetic registry/authorizer for --offline"}]
    if decision["simulated_human"]:
        who = "SIMULATED human supervisor: --human-mode scripted approves automatically; no person decided anything"
    else:
        who = "a real person approved through `pulso_demo.decide` (manual mode); identity is still the sandbox issuer's"
    return [{"id": "human_decision", "what": who},
            {"id": "human_issuer", "what": "local-identity human-issuer: sandbox-only DOUBLE (com.pulso.role=double), internal network only; Core trusts only its "
                                           "public key in the LOCAL staff set (never remote staging/prod)", "until": "real IdP/step-up"}]


def assemble(*, namespace: str, tenant: str, generated_at: str, scout: dict[str, Any], verify: dict[str, Any], alternatives: list[dict[str, Any]],
             attempts: list[dict[str, Any]], core: dict[str, Any], exporter: dict[str, Any], dataset: dict[str, Any],
             decision: dict[str, Any] | None = None, successor: dict[str, Any] | None = None) -> dict[str, Any]:
    doubles = [d for d in DOUBLES if d["id"] != "human_decision"] + human_doubles(decision, bool(core.get("synthetic")))
    out = {"schema": SCHEMA, "namespace": namespace, "tenant": tenant, "generated_at": generated_at, "dataset": dataset,
           "analysis": {"scout": scout, "verify": verify, "alternatives": alternatives, "attempts": attempts},
           "core": core, "exporter": exporter, "doubles": doubles}
    if decision is not None:
        out["decision"] = decision
    if successor is not None:
        out["successor"] = successor
    return out
