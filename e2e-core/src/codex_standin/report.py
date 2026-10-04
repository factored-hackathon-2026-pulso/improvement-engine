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
    {"code": "jev_base_url_not_configurable",
     "detail": "agent_core HttpJevTransport uses a fixed DEFAULT_BASE_URL (no env), so decision-provider scenarios "
               "(atencion/disputas-suite) cannot be scripted in the real image; E2E uses LLM-only pulso agents.",
     "request": "UP (agent-core): AGENTCORE_JEV_BASE_URL (or inject JevTransport through a factory) for local runs."},
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


def declared_doubles(fx_pieces: list[str]) -> list[dict[str, str]]:
    """What this harness itself still declares as a double: the fixtures pieces (scripted llm, control-api, lab-broker,
    bank, ingest), and the engine stand-in."""
    declared = [{"piece": p, "kind": "fixture", "declared_by": "e2e-core/fixtures_app"} for p in fx_pieces]
    declared += [
        {"piece": "codex-standin", "kind": "engine_stand_in", "declared_by": "e2e-core/codex_standin"}]
    return declared


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
