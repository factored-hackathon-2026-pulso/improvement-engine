"""Worker of gen-wire.ps1. Runs INSIDE the scratch venv built from the pinned agent-core checkout.

Writes ONLY `<out>/` (the `agent_core@<sha7>` wire directory). Deterministic and fail-closed.
Usage: python gen_wire.py --checkout <path> --out <dir> [--expected-sha <sha>]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
from importlib import metadata
from pathlib import Path

PLATFORM_SIM = Path(__file__).resolve().parents[2] / "platform-sim"

DERIVED = ["EvalSuite", "EntityDraft", "VersionDocs", "Proposal", "ValidationReport", "CandidateView",
           "ReleaseDetail", "EntityVersion", "ReleaseDiff", "EvalReport", "ProposalDetail", "WriteRecord", "EvalRun"]
# Models Core SERVES (responses): schema in serialization mode, like upstream's `contracts/registry/` outputs (N-01). A
# validation-mode schema of an output is looser (a Decimal metric validates as a number too). Everything else in DERIVED
# plus BODY_MODELS is request input (validation mode).
OUTPUT_MODELS = {"Proposal", "ValidationReport", "CandidateView", "ReleaseDetail", "EntityVersion", "ReleaseDiff",
                 "EvalReport", "ProposalDetail", "WriteRecord", "EvalRun"}
BODY_MODELS = ["_Create", "_Draft", "_Evaluate", "_Approve", "_Promote", "_Reason"]


def die(msg: str) -> None:
    print(f"pulso:wire_gen_failed {msg}", file=sys.stderr)
    raise SystemExit(2)


FALLBACK_PIN = {"sha": "894fa65575d83420523f33ec1c6919b8965f7ebe", "contract_version": "1.3.0"}
SUPPORTED_CONTRACT = "1.3.0"


def resolve_pin(repo_root: Path) -> dict:
    """`contracts/agent_core/pin.json` when present (a malformed one fails closed, never falls back), else the
    built-in fallback pin. Mirrors gen-wire.ps1 so both entry points agree on the SHA."""
    pin_file = repo_root / "contracts" / "agent_core" / "pin.json"
    if not pin_file.exists():
        return {**FALLBACK_PIN, "source": "fallback"}
    try:
        pin = json.loads(pin_file.read_text(encoding="utf-8"))
    except ValueError:
        die(f"{pin_file} is not valid JSON")
    sha = pin.get("sha") if isinstance(pin, dict) else None
    if not (isinstance(sha, str) and len(sha) == 40 and all(c in "0123456789abcdef" for c in sha)):
        die(f"{pin_file}: `sha` must be 40 lowercase hex characters")
    if pin.get("contract_version") != SUPPORTED_CONTRACT:
        die(f"{pin_file}: contract_version {pin.get('contract_version')!r} != {SUPPORTED_CONTRACT}")
    return {**pin, "source": "pin.json"}


def check_manifest_digest(pin: dict, manifest_bytes: bytes) -> None:
    """pin.json `manifest_sha256` (when declared) must equal the digest of the generated MANIFEST.json."""
    declared = pin.get("manifest_sha256")
    if declared and declared != hashlib.sha256(manifest_bytes).hexdigest():
        die(f"pulso:wire_drift manifest_sha256 {hashlib.sha256(manifest_bytes).hexdigest()} != pin {declared}")


def _keys_to_str(obj: object) -> object:
    """YAML 1.1 turns bare true/false/yes/no keys into booleans; JSON keys are strings."""
    if isinstance(obj, dict):
        return {(str(k).lower() if isinstance(k, bool) else str(k)): _keys_to_str(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_keys_to_str(v) for v in obj]
    return obj


def dump(obj: object) -> bytes:
    obj = _keys_to_str(obj)
    return (json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=False, default=str) + "\n").encode("utf-8")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def preflight(checkout: Path, expected: str) -> str:
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True,
                          text=True).stdout.strip()
    if head != expected:
        die(f"checkout HEAD {head!r} != pin {expected}")
    version = (checkout / "contracts" / "VERSION").read_text().strip()
    if version != SUPPORTED_CONTRACT:
        # Not a gate: upstream did not move VERSION across N-01..N-11 (agent-core 789d6c8..894fa65). The pin is the SHA plus the
        # MANIFEST digest; a VERSION change is only flagged so a human reviews it.
        print(f"pulso:wire_note contracts/VERSION {version} != reviewed {SUPPORTED_CONTRACT}", file=sys.stderr)
    exe = Path(sys.executable).with_name("agentcore.exe")
    if not exe.exists():
        exe = Path(sys.executable).with_name("agentcore")
    rc = subprocess.run([str(exe), "contracts", "--check"], cwd=checkout, capture_output=True).returncode
    if rc != 0:
        die("agentcore contracts --check failed")
    return version


def copy_bytes(checkout: Path, out: Path, files: list[dict]) -> None:
    for sub in ("schemas", "events", "registry"):  # `registry/`: N-01 (published upstream since 1.3.0+N-batch)
        base = checkout / "contracts" / sub
        for src in sorted(base.rglob("*")):
            if src.is_file():
                rel = f"{sub}/{src.relative_to(base).as_posix()}"
                (out / rel).parent.mkdir(parents=True, exist_ok=True)
                data = src.read_bytes()
                (out / rel).write_bytes(data)
                files.append({"path": rel, "sha256": sha(data)})
    data = (checkout / "contracts" / "openapi.json").read_bytes()
    (out / "openapi.json").write_bytes(data)
    files.append({"path": "openapi.json", "sha256": sha(data)})


def derived(out: Path, files: list[dict], app) -> None:
    from pydantic import TypeAdapter

    from agent_core.registry import http as H
    from agent_core.registry import models as M
    from agent_core.registry import service as S
    from agent_core.registry import suite as SU
    from agent_core.registry.evaluation import report as R

    pool = {}
    for mod in (M, S, SU, R):
        for name in DERIVED:
            if hasattr(mod, name) and name not in pool:
                pool[name] = getattr(mod, name)
    for name in BODY_MODELS:
        pool[name] = getattr(H, name)
    missing = [n for n in DERIVED + BODY_MODELS if n not in pool]
    if missing:
        die(f"registry models not found: {missing}")
    (out / "derived").mkdir(parents=True, exist_ok=True)
    for name, model in sorted(pool.items()):
        mode = "serialization" if name in OUTPUT_MODELS else "validation"
        data = dump(TypeAdapter(model).json_schema(mode=mode, by_alias=True))
        rel = f"derived/{name.lstrip('_')}.schema.json"
        (out / rel).write_bytes(data)
        files.append({"path": rel, "sha256": sha(data), "derived_by_pulso": True})
    data = dump(app.openapi())
    (out / "derived/registry_openapi.json").write_bytes(data)
    files.append({"path": "derived/registry_openapi.json", "sha256": sha(data), "derived_by_pulso": True})


def seeded_release_id(tx, agent_id: str = "atencion", alias: str = "prod") -> str:
    """The seeded demo release is whatever `atencion:prod` points at. Its id is a content hash that moves on every pin
    touching release content (789d6c8 -> 894fa65 added `Interrupt.locked`), so it must never be hard-coded."""
    rel_id = tx.get_alias(agent_id, alias)
    if not rel_id:
        die(f"seeded alias {agent_id}:{alias} has no release")
    return rel_id


def golden(out: Path, files: list[dict], checkout: Path, world, client) -> None:
    import yaml

    from agent_core.registry.entities import content_hash, decode_entity, encode_entity
    from agent_core.registry.yaml_io import FOLDERS
    from registry_mock import jws

    seed = checkout / "tests" / "fixtures" / "registry-demo"
    entities = []
    with world.store.transaction() as tx:
        rel_id = seeded_release_id(tx)
        stored_rel = tx.get_release(rel_id)
        if stored_rel is None:
            die(f"seeded release {rel_id} missing")
        for ref in sorted(tx.release_refs(rel_id), key=lambda r: (r.kind, r.id, r.version)):
            sv = tx.get_version(ref)
            entity = decode_entity(ref.kind, tx.blobs.get(sv.content_hash))
            authored = yaml.safe_load((seed / FOLDERS[ref.kind] / f"{ref.id}@{ref.version}.yaml").read_bytes())
            entities.append({
                "ref": f"{ref.kind}:{ref.id}@{ref.version}",
                "authored_json": authored,
                "normalized_dump_json": entity.model_dump(mode="json", by_alias=True),
                "canonical_bytes_hex": encode_entity(entity).hex(),
                "content_hash": content_hash(entity),
            })
        release_hash = stored_rel.release_hash
    flow = next(e for e in entities if e["ref"] == "flow:disputa-cargo@1.0.0")

    draft = build_golden_draft(flow["normalized_dump_json"])
    h = {"authorization": "Bearer " + jws.issue("bot")}
    created = client.post("/v1/registry/proposals", json={"agent_id": "atencion", "origin": "auto_detect",
                                                          "title": "Avisar antes de escalar por monto"}, headers=h)
    pid = created.json()["proposal_id"]
    put = client.put(f"/v1/registry/proposals/{pid}/draft", json=draft, headers=h)
    val = client.post(f"/v1/registry/proposals/{pid}/validate", headers=h)
    if put.status_code != 200 or val.status_code != 200:
        die(f"golden draft rejected by a2: {put.status_code} {put.text[:300]} / {val.status_code} {val.text[:300]}")
    freeze = client.post(f"/v1/registry/proposals/{pid}/freeze", headers=h).json()
    vectors = {
        "_note": "Recorded from the real RegistryService at the pinned SHA (a2 in-memory). Reproducible by gen-wire.",
        "release_id": rel_id, "release_hash": release_hash, "entities": entities,
        # N-03: the seeded release exactly as `GET /v1/registry/releases/{id}` serves it (interrupts, language_detection,
        # injection_ruleset, max_input_chars). With the entity vectors above, `release_hash` is reconstructible locally.
        "release_detail": client.get(f"/v1/registry/releases/{rel_id}", headers=h).json(),
        "candidate": {"candidate_hash": freeze["candidate_hash"],
                      "release_id_preview": freeze["release_id_preview"], "validation": val.json(),
                      "new_versions": freeze["new_versions"], "auto_bumped": freeze["auto_bumped"]},
    }
    (out / "golden").mkdir(parents=True, exist_ok=True)
    for rel, obj in (("golden/hash_vectors.json", vectors), ("golden/golden_draft_disputa.json", draft)):
        data = dump(obj)
        (out / rel).write_bytes(data)
        files.append({"path": rel, "sha256": sha(data), "derived_by_pulso": True})


def build_golden_draft(flow: dict) -> dict:
    """V3 section 31.4.12: umbral.true -> aviso_monto -> esc_monto, new template, new suite."""
    flow = json.loads(json.dumps(flow))
    flow["version"] = "1.1.0"
    for node in flow["nodes"]:
        if node["id"] == "umbral":
            node["next"]["true"] = "aviso_monto"
    flow["nodes"].append({"id": "aviso_monto", "next": {"next": "esc_monto"}, "type": "respond",
                          "config": {"template_ref": {"id": "t/aviso_monto", "spec": "1.0.0"}, "generate": None,
                                     "await": False, "claims": []}})
    suite = {
        "id": "disputas-suite", "version": "1.0.0", "agent_id": "atencion", "repetitions": 1, "thresholds": {},
        "scenarios": [{
            "id": "resuelto", "principal": {"id": "cust-001", "attrs": {"country": "CO"}},
            "steps": [{"op": "start", "auth": "step_up"},
                      {"op": "turn", "text": "no reconozco un cargo de ciento veinte dolares", "auth": "step_up"},
                      {"op": "confirm", "answer": "yes", "auth": "step_up"}],
            "seed": {"tools": {
                "buscar_transacciones": [{"status": "ok", "result": [
                    {"transaction_id": "tx-1", "amount": "120.50", "currency": "USD"},
                    {"transaction_id": "tx-2", "amount": "30.00", "currency": "USD"}], "error": None}],
                "seleccionar": [{"status": "ok", "result": {"transaction_id": "tx-1", "amount": "120.50",
                                                           "currency": "USD"}, "error": None}],
                "convertir_moneda": [{"status": "ok", "result": "120.50", "error": None}],
                "radicar_pqr": [{"status": "ok", "result": {"status": "Open", "id": "pqr-demo-1"}, "error": None}],
                "obtener_pqr": [{"status": "ok", "result": {"status": "Open", "id": "pqr-demo-1"}, "error": None}]}},
            "expect": {"outcome": "resolved", "actions_verified": ["radicar_pqr"], "escalated": False}}]}
    return {"expected_rev": 0, "changes": [
        {"kind": "flow", "docs": {"description": "umbral.true pasa por aviso_monto antes de escalar",
                                  "rationale": "opp-demo-1", "changelog": "umbral.true -> aviso_monto -> esc_monto"},
         "content": flow},
        {"kind": "template", "docs": {"description": "Aviso es/pt previo a escalamiento por monto",
                                      "rationale": "opp-demo-1", "changelog": "Nuevo template"},
         "content": {"id": "t/aviso_monto", "version": "1.0.0",
                     "locales": {"es": "Por el monto, un especialista revisara tu disputa.",
                                 "pt": "Pelo valor, um especialista vai revisar sua contestacao."}, "reads": []}},
        {"kind": "eval_suite", "docs": {"description": "Suite baseline de disputas",
                                        "rationale": "suite del mundo demo", "changelog": "Version inicial"},
         "content": suite}]}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--checkout", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--expected-sha", help="default: resolved from contracts/agent_core/pin.json, else the fallback pin")
    a = ap.parse_args()
    checkout, out = Path(a.checkout).resolve(), Path(a.out)
    pin = resolve_pin(PLATFORM_SIM.parent)
    if a.expected_sha is None:
        a.expected_sha = pin["sha"]
    elif a.expected_sha != pin["sha"]:
        die(f"--expected-sha {a.expected_sha} != pin {pin['sha']} ({pin['source']})")
    version = preflight(checkout, a.expected_sha)
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    sys.path.insert(0, str(PLATFORM_SIM))
    sys.path.insert(0, str(checkout))
    os.environ["PULSO_CORE_CHECKOUT"] = str(checkout)
    from fastapi.testclient import TestClient

    from registry_mock.a2_app import World, build_app

    app = build_app(checkout)
    files: list[dict] = []
    copy_bytes(checkout, out, files)
    derived(out, files, app)
    with TestClient(app, raise_server_exceptions=False) as client:
        client.post("/_sim/reset")
        golden(out, files, checkout, World(checkout), client)
    tools = {"python": platform.python_version(), "agent_core_checkout_sha": a.expected_sha}
    for pkg in ("pydantic", "fastapi", "pyyaml", "rfc8785"):
        try:
            tools[pkg] = metadata.version(pkg)
        except metadata.PackageNotFoundError:
            pass
    manifest = {"repo": "agent-core", "sha": a.expected_sha, "contract_version": version,
                "files": sorted(files, key=lambda f: f["path"]), "tool_versions": tools}
    manifest_bytes = dump(manifest)
    check_manifest_digest(pin, manifest_bytes)
    (out / "MANIFEST.json").write_bytes(manifest_bytes)
    print(f"wire written: {out} ({len(files)} files; manifest_sha256={sha(dump(manifest))})")


if __name__ == "__main__":
    main()
