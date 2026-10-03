"""The engine stand-in: plays the Rust engine's side of the pipeline against the real Core bridge.

pin release -> scout -> verifier (different agent) -> builder_design -> writer (create/put/freeze) -> evaluation
admission -> evaluate-only invocation / arms -> report -> exporter. Every model answer is scripted on the
`e2e-fixtures` double; every platform answer (binding, broker, bank, ingest) comes from that double too."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import httpx
import psycopg
import rfc8785
import yaml

from codex_standin.bridge import Bridge
from codex_standin.dto import admission, digest_json, invocation
from codex_standin.fixtures_app import lab_refs

REPO = Path(__file__).resolve().parents[3]
ASSETS = REPO / "agent-core-assets"
TENANT = "tenant-local"  # the tenant of the stack (PULSO_TENANT_ID); also the exporter's tenant
OTHER = "tenant-other"
RESEARCH = "research stage"  # system-prompt markers of the three LLM stages (agent-core-assets prompts)
VERIFY = "independent verification stage"
DESIGN = "design stage"


def manifest_releases() -> dict[str, str]:
    return dict(yaml.safe_load((ASSETS / "manifest.yaml").read_text(encoding="utf-8"))["release_ids"])


def put_draft_digest(proposal_id: str | None, expected_rev: int | None, changes: Any) -> str:
    return digest_json({"proposal_id": proposal_id, "expected_rev": expected_rev, "changes": changes})


def ref_of(tenant: str) -> dict[str, str]:
    r = lab_refs(tenant)
    return {"id": r["result"], "digest": r["digest"], "media_type": "application/json"}


def candidate_changes(version: str = "1.1.0") -> list[dict[str, Any]]:
    """Writer draft plan for agent `pulso-scout`: a new scout prompt and the guard-path smoke suite (agent-core-assets
    world `pulso-evolution`, suite bumped to the candidate's version)."""
    suite = yaml.safe_load((ASSETS / "worlds/pulso-evolution/eval_suites/pulso-smoke@1.0.0.yaml").read_text("utf-8"))
    suite["version"] = version
    docs = {"description": "e2e candidate", "rationale": "improve evidence discipline", "changelog": "e2e"}
    prompt = {"id": "p/scout_task", "version": version,
              "locales": {"en": "You are a research stage. Use only the tools you are given; cite only artifacts you "
                                "retrieved; never invent evidence."},
              "model_profile": "pulso-evolution-structured@1.0.0"}
    return [{"kind": "prompt", "content": prompt, "docs": docs}, {"kind": "eval_suite", "content": suite, "docs": docs}]


def suite_digest(changes: list[dict[str, Any]]) -> str:
    from agent_core.registry import EvalSuite
    from agent_core.registry.entities import content_hash

    return str(content_hash(EvalSuite.model_validate(changes[1]["content"])))


@dataclass
class Db:
    dsn: str = field(repr=False)

    def one(self, sql: str, *args: Any) -> Any:
        with psycopg.connect(self.dsn) as conn:
            row = conn.execute(sql, args).fetchone()
        return None if row is None else row[0]

    def rows(self, sql: str, *args: Any) -> list[tuple[Any, ...]]:
        with psycopg.connect(self.dsn) as conn:
            return list(conn.execute(sql, args).fetchall())


@dataclass
class Stage:
    key: str
    body: dict[str, Any]
    response: httpx.Response

    @property
    def out(self) -> dict[str, Any]:
        return self.response.json()  # type: ignore[no-any-return]


@dataclass
class Engine:
    bridge: Bridge
    fx: httpx.Client
    tenant: str
    releases: dict[str, str] = field(default_factory=manifest_releases)

    def configure(self, **cfg: Any) -> None:
        r = self.fx.post("/_e2e/config", json=cfg)
        assert r.status_code == 200, r.text

    def state(self) -> dict[str, Any]:
        return self.fx.get("/_e2e/state").json()  # type: ignore[no-any-return]

    def stage(self, stage: str, job: str, logical: str, agent_id: str, input: dict[str, Any], *,
              release: str | None = None, tenant: str | None = None, attempt: int = 1, **extra: Any) -> Stage:
        tenant = tenant or self.tenant
        key, body = invocation(tenant=tenant, job=job, stage=stage, agent_id=agent_id,
                               release_id=release or self.releases[agent_id], logical=logical, input=input,
                               attempt=attempt, **extra)
        return Stage(key, body, self.bridge.invoke(tenant, key, body))

    def facts(self, run_id: str) -> dict[str, Any]:
        r = self.bridge.read_task(self.tenant, run_id)
        assert r.status_code == 200, r.text
        return r.json()["result"]["facts"]  # type: ignore[no-any-return]

    def seal(self, art_id: str, content: Any) -> str:
        """Engine stand-in uploads a sealed artifact to the broker double; returns the artifact id."""
        self.configure(artifacts=[{"tenant": self.tenant, "id": art_id, "content": content}])
        return art_id

    # -- scripted model answers ---------------------------------------------------------------------------------
    def script_stage_model(self, rule_id: str, marker: str, final: dict[str, Any], *, query: bool = True) -> None:
        responses: list[dict[str, Any]] = []
        if query:  # the agent retrieves its own evidence through the real `pulso/lab_query` first
            responses.append({"kind": "tool_call", "tool": "pulso/lab_query@1.0.0", "args": {"sql": "select 1"}})
        responses.append({"kind": "final", "output": final})
        self.configure(llm_rules=[{"id": rule_id, "match": {"system_contains": marker}, "responses": responses}])


def hypotheses_output(tenant: str) -> dict[str, Any]:
    return {"schema_version": "1", "hypotheses": [{
        "id": "h1", "statement": "retention dips on day 7", "mechanism": "onboarding gap",
        "evidence_refs": [ref_of(tenant)], "counterevidence_refs": [], "missing_evidence": ["cohort split"],
        "next_queries": []}]}


def verification_output(tenant: str) -> dict[str, Any]:
    return {"schema_version": "1", "assessments": [{
        "hypothesis_id": "h1", "verdict": "supported", "evidence_refs": [ref_of(tenant)],
        "counterevidence_refs": [], "limitations": ["single cohort"]}]}


def change_spec_output(tenant: str) -> dict[str, Any]:
    return {"schema_version": "1", "change_spec": {"target": "pulso-scout", "kind": "prompt"},
            "rationale": "supported hypothesis h1", "evidence_refs": [ref_of(tenant)],
            "alternatives": [{"id": "alt-0", "kind": "do_nothing", "summary": "keep the current prompt"},
                             {"id": "alt-1", "kind": "proposed_change", "summary": "tighten evidence discipline"}]}


def sha_hex(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


_ = (rfc8785, admission)
