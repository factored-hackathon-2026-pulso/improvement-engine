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
    {"id": "human_decision", "what": "approve/publish hook not connected: pending until local-identity is wired"},
]


def assemble(*, namespace: str, tenant: str, generated_at: str, scout: dict[str, Any], verify: dict[str, Any], alternatives: list[dict[str, Any]],
             attempts: list[dict[str, Any]], core: dict[str, Any], exporter: dict[str, Any], dataset: dict[str, Any]) -> dict[str, Any]:
    return {"schema": SCHEMA, "namespace": namespace, "tenant": tenant, "generated_at": generated_at, "dataset": dataset,
            "analysis": {"scout": scout, "verify": verify, "alternatives": alternatives, "attempts": attempts},
            "core": core, "exporter": exporter, "doubles": DOUBLES}
