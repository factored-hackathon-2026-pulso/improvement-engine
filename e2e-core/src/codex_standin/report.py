"""`e2e-report.json`: the honest evidence record (plan 17.3.8 `integration_report.json` shape + `gaps[]`/`effects`).

`target=real_local` is written ONLY if `/_sim/info` is absent on the Core URL, `agent_core_sha == pin`, the image
digest the runtime reports equals the digest recorded by start.ps1 for the image it started, and `/readyz` passes;
otherwise `evidence_target_unproven`. `runtime_profile` is copied from `/internal/v1/version`, never typed.
`doubles[]` = the runtime's own report UNION every running container labelled com.pulso.role=double UNION the
pieces this harness declares (scripted LLM, fixtures, stand-in, workarounds). Non-empty doubles never mean a
real-AWS success."""

from __future__ import annotations

import json
import subprocess
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx

from codex_standin import PIN_SHA
from codex_standin.stack import CONNECTION, PODMAN

REPO = Path(__file__).resolve().parents[3]


# Stack-level gaps found while building this E2E (each has an owner request). Always listed: they are properties of
# the stack as delivered, independent of which test ran.
KNOWN_STACK_GAPS: list[dict[str, str]] = [
    {"code": "core_app_engine_table_grants_missing",
     "detail": "`agentcore migrate --app-role core_app` grants only audit_events (+registry tables); local/core/init/*.sql "
               "never grants runs, run_idempotency, turn_leases, turn_results, usage, handoffs, outbox, so the first "
               "POST /v1/runs dies with InsufficientPrivilege (HTTP 500, invoke `unknown core_call_failed`).",
     "request": "L8: add GRANT SELECT,INSERT,UPDATE,DELETE on those tables (+ sequences) to core_app in "
                "local/core/init/10-exporter-grants.sql or a new 12-app-grants.sql. This harness applies it as the "
                "declared double `db_grants:workaround`."},
    {"code": "eval_sequences_grant_missing",
     "detail": "15-eval-grants.sql grants ALL ON ALL TABLES to core_eval_app but no sequence: the first evaluation run "
               "fails `permission denied for sequence runs_run_seq_seq` (arms `failed_infra infra_InsufficientPrivilege`).",
     "request": "L8: GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO core_eval_app (core_eval)."},
    {"code": "no_control_api_to_bridge_service_key",
     "detail": "core-keygen's service.json holds only exporter keys and smoke-probe (iss pulso-smoke); no key lets "
               "iss=control-api (A03 class i) call the bridge. The harness overlays PULSO_SERVICE_KEYS with its own "
               "test key (declared double `runtime-config-overlay`).",
     "request": "L8: emit a control-api -> core-bridge kid (iss control-api, aud core-bridge) and keep its seed "
                "readable by stand-ins (e.g. control-api-core-bridge.key), as done for the exporter."},
    {"code": "exporter_key_issuer_mismatch",
     "detail": "core-keygen registers exporter-* keys with iss `pulso-exporter`, but the exporter signs iss=core-bridge "
               "(A03); a receiver that trusts keygen's service.json rejects every exporter call (wrong_audience, "
               "exporter stuck in `deferred`).",
     "request": "L8/L6: make keygen's iss equal what the exporter signs (or the exporter's iss configurable); the "
               "fixture registers iss=core-bridge."},
    {"code": "llm_env_not_passed_through_compose",
     "detail": "compose.core.yaml core-runtime has no LLM_ENDPOINTS / key env, so a scripted OpenAI-compatible provider "
               "cannot be injected through start.ps1; the runtime image has no scripted-provider hook. Workaround: "
               "thin config overlay image (ENV only) passed via start.ps1 -Image.",
     "request": "L8: pass LLM_ENDPOINTS and the api_key_env variable names through core-runtime environment (names "
               "only, values from the env file). MINIMAL hook, no runtime code change needed."},
    {"code": "jev_base_url_not_configurable",
     "detail": "agent_core HttpJevTransport uses a fixed DEFAULT_BASE_URL (no env), so decision-provider scenarios "
               "(atencion/disputas-suite) cannot be scripted in the real image; E2E uses LLM-only pulso agents.",
     "request": "UP (agent-core): AGENTCORE_JEV_BASE_URL (or inject JevTransport through a factory) for local runs."},
    {"code": "core_state_aliases_not_implemented",
     "detail": "GET /internal/v1/core-state/aliases answers 501; `pin release` is verified against the manifest and "
               "reg_release_status only.",
     "request": "L3: implement alias_read or document it as not part of H4."},
    {"code": "windows_host_unreachable_from_containers",
     "detail": "Containers on pulso-dev cannot reach listeners on the Windows host (host.containers.internal refuses, "
               "172.17.128.1 times out: WSL NAT/firewall), so the control-api/broker/LLM doubles run as a labelled "
               "container on the stack network, not as host processes.",
     "request": "none for Claude (no firewall change allowed); Codex's assembly hosts the real services in the "
                "same network."},
]


def target_of(*, sim_info_status: int, sha: str, reported_digest: str | None, expected_digest: str | None,
              ready: bool) -> str:
    ok = (sim_info_status == 404 and sha == PIN_SHA and bool(reported_digest) and reported_digest == expected_digest
          and ready)
    return "real_local" if ok else "evidence_target_unproven"


def double_containers(ns: str) -> list[dict[str, str]]:
    out = subprocess.run([PODMAN, "--connection", CONNECTION, "ps", "--filter", "label=com.pulso.role=double",
                          "--filter", f"label=com.pulso.namespace={ns}", "--format",
                          '{{.Label "com.docker.compose.service"}}'], capture_output=True, text=True).stdout.split()
    return [{"piece": f"container:{svc}", "kind": "double_container", "declared_by": "label com.pulso.role=double"}
            for svc in out]


def union_doubles(runtime_reported: list[str], containers: list[dict[str, str]],
                  declared: list[dict[str, str]]) -> list[dict[str, str]]:
    seen: set[str] = set()
    out: list[dict[str, str]] = []
    for d in [*({"piece": f"runtime:{r}", "kind": "runtime_stand_in", "declared_by": "runtime /internal/v1/version"}
                for r in runtime_reported), *containers, *declared]:
        if d["piece"] not in seen:
            seen.add(d["piece"])
            out.append(d)
    return out


def head_sha() -> str:
    return subprocess.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()


def build(*, env: dict[str, Any], version: dict[str, Any], runtime_url: str, declared: list[dict[str, str]],
          suites: list[dict[str, Any]], gaps: list[dict[str, str]], effects: dict[str, Any], commands: list[str],
          started: str, extra: dict[str, Any]) -> dict[str, Any]:
    sim = httpx.get(runtime_url + "/_sim/info", timeout=5).status_code
    ready = httpx.get(runtime_url + "/readyz", timeout=5).status_code == 200
    target = target_of(sim_info_status=sim, sha=version.get("agent_core_sha", ""),
                       reported_digest=version.get("image_digest"), expected_digest=env.get("expected_image_digest"),
                       ready=ready)
    manifest = REPO / "agent-core-assets" / "manifest.yaml"
    import hashlib
    return {
        "schema_version": 1, "target": target, "runtime_profile": version.get("runtime_profile"),
        "agent_core_sha": version.get("agent_core_sha"), "contracts_version": version.get("contracts_version"),
        "pin_manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest() if manifest.exists() else None,
        "image_digest": version.get("image_digest"), "expected_image_digest": env.get("expected_image_digest"),
        "head_sha": head_sha(), "namespace": env["namespace"], "machine": {"name": "pulso-dev",
                                                                          "connection": env["connection"]},
        "doubles": union_doubles(list(version.get("doubles", [])), double_containers(env["namespace"]), declared),
        "suites": suites, "gaps": gaps, "effects": effects, "commands": commands, "started_at": started,
        "finished_at": datetime.now(UTC).isoformat(), **extra}


def write(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
