"""FIRST RED of the e2e-core hardening pass: the harness no longer works around defects fixed upstream (db grants,
exporter key issuer, runtime service-key overlay, LLM overlay image) and stays SHA-agnostic."""

from __future__ import annotations

import json
from pathlib import Path

import yaml

from codex_standin import PIN_SHA, report, stack

REPO = Path(__file__).resolve().parents[3]


def test_pin_sha_is_read_from_the_manifest_never_hardcoded() -> None:
    manifest = yaml.safe_load((REPO / "agent-core-assets" / "manifest.yaml").read_text(encoding="utf-8"))
    assert PIN_SHA == manifest["pin"]["sha"]
    assert "86a7674" not in (REPO / "e2e-core" / "run.ps1").read_text(encoding="utf-8")


def test_grants_workaround_is_gone() -> None:
    assert not hasattr(stack, "apply_grants_workaround") and not hasattr(stack, "GRANTS_WORKAROUND")


def test_overlay_only_carries_the_eval_budgets_file_no_service_keys_no_llm(tmp_path: Path) -> None:
    keys = stack.prepare("claude-e2e-1", tmp_path)
    docker = (tmp_path / "context" / "Dockerfile").read_text(encoding="ascii")
    assert "PULSO_SERVICE_KEYS" not in docker and "LLM_ENDPOINTS" not in docker and "E2E_LLM_KEY" not in docker
    assert "PULSO_EVAL_BUDGETS" in docker
    assert not (tmp_path / "context" / "service.json").exists()
    assert "service_seed" not in keys  # the control-api seed is read from the stack volume after start


def test_llm_settings_go_through_the_env_file_with_pulso_llm_api_key(tmp_path: Path) -> None:
    lines = stack.core_env_lines()
    joined = "\n".join(lines)
    assert any(line.startswith("PULSO_LLM_API_KEY=") for line in lines)
    endpoints = next(line for line in lines if line.startswith("LLM_ENDPOINTS="))
    value = json.loads(endpoints.split("=", 1)[1].strip("'"))
    assert value["pulso-evolution-llm"]["api_key_env"] == "PULSO_LLM_API_KEY"
    assert value["pulso-evolution-llm"]["base_url"].startswith("http://e2e-fixtures:")
    assert "PULSO_TENANT_ID" not in joined or "tenant-local" in joined


def test_ring_trusts_the_stack_exporter_keys_as_issued_without_issuer_override() -> None:
    cb = {"kid": "cb", "key": stack.jwtsvc.seed_of(stack.jwtsvc.Ed25519PrivateKey.generate())}
    svc = {"keys": {"exporter-control-api": {"iss": "core-bridge", "aud": "control-api", "key": "AAAA"},
                    "control-api-core-bridge": {"iss": "control-api", "aud": "core-bridge", "key": "BBBB"}}}
    trust = {"keys": {"x-1": {"iss": "core-bridge", "aud": "lab-broker", "key": "CCCC"}}}
    out = stack.build_verify_keys(cb, svc, trust)
    assert out["ingest"]["keys"] == {"exporter-control-api": svc["keys"]["exporter-control-api"]}
    assert out["ring"]["x-1"] == ["core-bridge", "lab-broker", "CCCC"]


def test_report_lists_only_the_remaining_doubles_and_gaps() -> None:
    pieces = report.declared_doubles(["llm:scripted", "control-api", "lab-broker", "bank", "ingest"])
    kinds = {d["piece"] for d in pieces}
    assert kinds == {"llm:scripted", "control-api", "lab-broker", "bank", "ingest", "codex-standin",
                     "runtime-config-overlay"}
    assert not any("workaround" in d["piece"] for d in pieces)
    codes = {g["code"] for g in report.KNOWN_STACK_GAPS}
    assert not codes & {"core_app_engine_table_grants_missing", "eval_sequences_grant_missing",
                        "no_control_api_to_bridge_service_key", "exporter_key_issuer_mismatch",
                        "llm_env_not_passed_through_compose"}
    assert "eval_budgets_not_passed_through_compose" in codes
