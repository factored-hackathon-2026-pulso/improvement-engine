"""Validator for agent-core-assets (package L4; plan 17.3.4, V3 section 31.9 CAP-46/51).

Two tiers:
  * offline (PyYAML only): layout, manifest/expected-state drift, stage invariants (no precomputed finding),
    merge conflicts, layer mappings, secrets.
  * agent-core (uses the pinned checkout through `uv run --project`): `agentcore validate`, state computation,
    generated `registry/*` ToolDefs.

Python 3.12. CLI: `python assetcheck.py {check|validate|write-state} [--root DIR]`.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

PIN_SHA = "789d6c89b2fca90fc10e2abf157da51dc81c5d51"
PIN_CONTRACT_VERSION = "1.3.0"
EVOLUTION_WORLD = "pulso-evolution"
ENTITY_FOLDERS = (
    "agents", "flows", "policies", "templates", "prompts", "tools", "decision_models", "model_profiles",
    "language_detection", "injection_rulesets", "knowledge_snapshots", "eval_suites",
)
LAYERS = {"tree", "classifier", "ai1", "ai2", "human"}
LAYER_MATCH_KEYS = {"flow", "node_id", "node_type", "provider", "principal_type"}
# Node kinds that wait for a human or hand off: a task stage never uses them (G0-16, ADR 0019).
WAITING_NODES = {"collect", "confirm", "transfer", "await_approval"}
FORBIDDEN_KEY = re.compile(r"^(findings?|winning\w*|precomputed\w*|preloaded\w*|golden\w*|known_\w+|"
                           r"expected_finding\w*|seed_finding\w*)$", re.I)
FORBIDDEN_TEXT = re.compile(r"\b(winning|pre-?computed|pre-?loaded|known finding|ganador)\b", re.I)
FORBIDDEN_SCHEMA_KEYS = {"const", "default", "examples", "example"}
PATH_ROOTS = ("slots.", "facts.", "decisions.", "readback.")
SECRET_KEY = re.compile(r"^(api_key|secret|password|passwd|token|authorization|bearer)$", re.I)
SECRET_VALUE = re.compile(r"(sk-[A-Za-z0-9]{16,}|AKIA[0-9A-Z]{16}|-----BEGIN [A-Z ]*PRIVATE KEY-----)")


@dataclass(frozen=True)
class Violation:
    code: str
    path: str
    message: str

    def __str__(self) -> str:
        return f"{self.code} {self.path}: {self.message}"


# --------------------------------------------------------------------------------------------- helpers

def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(obj: Any) -> bytes:
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _norm(data: bytes) -> bytes:
    return data.replace(b"\r\n", b"\n")


def world_files(world: Path) -> list[Path]:
    return sorted(p for p in world.rglob("*") if p.is_file())


def files_digest(world: Path) -> str:
    lines = [f"{p.relative_to(world).as_posix()}\0{sha256_hex(_norm(p.read_bytes()))}\n"
             for p in world_files(world)]
    return sha256_hex("".join(lines).encode("utf-8"))


def worlds_of(root: Path) -> list[Path]:
    base = root / "worlds"
    return sorted(p for p in base.iterdir() if p.is_dir()) if base.is_dir() else []


def load_yaml(path: Path) -> Any:
    return yaml.safe_load(path.read_text("utf-8"))


def yaml_files(world: Path, folder: str) -> list[Path]:
    base = world / folder
    return sorted(base.rglob("*.yaml")) if base.is_dir() else []


def walk(node: Any, path: str = ""):
    """Yield (path, key_or_None, value) for every mapping value and list item."""
    if isinstance(node, dict):
        for k, v in node.items():
            yield f"{path}/{k}", k, v
            yield from walk(v, f"{path}/{k}")
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield f"{path}/{i}", None, v
            yield from walk(v, f"{path}/{i}")


def agent_ids(root: Path) -> dict[str, str]:
    """agent id -> world name, from the agents/ folders."""
    out: dict[str, str] = {}
    for world in worlds_of(root):
        for p in yaml_files(world, "agents"):
            out[load_yaml(p)["id"]] = world.name
    return out


def capability_catalog(root: Path) -> dict[str, Any]:
    """V3 31.9.4 capability catalog, derived from the seed (never hand-written)."""
    tools: dict[str, Any] = {}
    aliases: set[str] = set()
    for world in worlds_of(root):
        for p in yaml_files(world, "tools"):
            d = load_yaml(p)
            ex = ("pulso" if d["id"].startswith("pulso/")
                  else "builder" if d["id"].startswith("registry/") else "sandbox")
            tools[d["id"]] = {"version": d["version"], "risk_class": d["risk_class"], "executor": ex}
        for p in yaml_files(world, "model_profiles"):
            aliases.add(load_yaml(p)["endpoint_alias"])
    return {"tools": dict(sorted(tools.items())),
            "builder_tools": sorted(t for t in tools if t.startswith("registry/")),
            "model_aliases": sorted(aliases), "source": "seed"}


def expected_state_digest(state: Any) -> str:
    return sha256_hex(canonical(state))


# ------------------------------------------------------------------------------------- merge of worlds

def merge_worlds(root: Path, dest: Path) -> list[Violation]:
    """Copy every world into `dest`; the same path with different bytes is a conflict (V3 31.9.1 ii)."""
    problems: list[Violation] = []
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    for world in worlds_of(root):
        for p in world_files(world):
            rel = p.relative_to(world)
            target = dest / rel
            data = _norm(p.read_bytes())
            if target.exists():
                if target.read_bytes() != data:
                    problems.append(Violation("merge_conflict", rel.as_posix(),
                                              f"world {world.name} redefines this entity with other content"))
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
    return problems


# ------------------------------------------------------------------------------------------ layout

def check_layout(root: Path) -> list[Violation]:
    out: list[Violation] = []
    for required in ("manifest.yaml", "expected-state.json", "worlds", "layer_mappings"):
        if not (root / required).exists():
            out.append(Violation("layout_missing", required, "required by CAP-46"))
    worlds = {w.name for w in worlds_of(root)}
    for name in ("attention-demo", EVOLUTION_WORLD):
        if name not in worlds:
            out.append(Violation("layout_missing", f"worlds/{name}", "world required by CAP-49"))
    for world in worlds_of(root):
        per_agent_suites: dict[str, int] = {}
        for p in yaml_files(world, "eval_suites"):
            s = load_yaml(p)
            per_agent_suites[s["agent_id"]] = per_agent_suites.get(s["agent_id"], 0) + 1
        for a, n in per_agent_suites.items():
            if n > 1:
                out.append(Violation("suite_count", f"{world.name}/eval_suites", f"{a} has {n} suites, max 1"))
        release_agents: list[str] = []
        for p in yaml_files(world, "releases"):
            r = load_yaml(p)
            rel = f"{world.name}/releases/{p.name}"
            if r.get("id") != p.stem:
                out.append(Violation("release_shape", rel, "release id must equal the file name"))
            entries = r.get("agents", [])
            if len(entries) != 1 or len(entries[0].get("aliases", [])) != 1:
                out.append(Violation("release_shape", rel, "one agent and exactly one alias per release"))
            release_agents += [e["agent"].split("@")[0] for e in entries]
        for p in yaml_files(world, "agents"):
            a = load_yaml(p)
            if a["id"] not in release_agents:
                out.append(Violation("release_shape", f"{world.name}/agents/{p.name}",
                                     f"agent {a['id']} has no release declaration"))
        for folder in ENTITY_FOLDERS:
            for p in yaml_files(world, folder):
                d = load_yaml(p)
                expect = f"{d.get('id')}@{d.get('version')}.yaml"
                # ids with a slash (t/x, p/y, pulso/z) live in a subfolder named by the id prefix
                if p.name != expect.split("/")[-1]:
                    out.append(Violation("entity_path", f"{world.name}/{p.relative_to(world).as_posix()}",
                                         f"file name does not match {expect}"))
        for p in world_files(world):
            relp = f"{world.name}/{p.relative_to(world).as_posix()}"
            if SECRET_VALUE.search(p.read_text("utf-8", errors="ignore")):
                out.append(Violation("secret", relp, "secret-looking value"))
            if p.suffix == ".yaml":
                for where, key, val in walk(load_yaml(p)):
                    if key and SECRET_KEY.match(str(key)) and isinstance(val, str):
                        out.append(Violation("secret", f"{relp}#{where}", "credential-like key with a literal value"))
    out += check_layer_mappings(root)
    return out


def check_layer_mappings(root: Path) -> list[Violation]:
    out: list[Violation] = []
    known = set(agent_ids(root))
    base = root / "layer_mappings"
    for p in sorted(base.glob("*.yaml")) if base.is_dir() else []:
        m = load_yaml(p)
        rel = f"layer_mappings/{p.name}"
        if p.name != f"{m.get('agent_id')}@{m.get('version')}.yaml":
            out.append(Violation("layer_mapping", rel, "file name must be <agent_id>@<version>.yaml"))
        if m.get("agent_id") not in known:
            out.append(Violation("layer_mapping", rel, "unknown agent_id"))
        if m.get("default") != "unknown":
            out.append(Violation("layer_mapping", rel, "default must be 'unknown'"))
        for i, rule in enumerate(m.get("rules", [])):
            if rule.get("layer") not in LAYERS:
                out.append(Violation("layer_mapping", f"{rel}#/rules/{i}", "layer not in tree|classifier|ai1|ai2|human"))
            if not rule.get("evidence_event"):
                out.append(Violation("layer_mapping", f"{rel}#/rules/{i}", "evidence_event required"))
            extra = set(rule.get("match", {})) - LAYER_MATCH_KEYS
            if extra or not rule.get("match"):
                out.append(Violation("layer_mapping", f"{rel}#/rules/{i}", f"bad match keys {sorted(extra)}"))
    return out


# ------------------------------------------------------------------------------------------- stages

def check_stages(root: Path) -> list[Violation]:
    """Stages are generic and take inputs: nothing precomputed, no waiting nodes, entry is bind_context."""
    out: list[Violation] = []
    world = root / "worlds" / EVOLUTION_WORLD
    if not world.is_dir():
        return [Violation("layout_missing", f"worlds/{EVOLUTION_WORLD}", "stage world missing")]

    def rel(p: Path) -> str:
        return f"{EVOLUTION_WORLD}/{p.relative_to(world).as_posix()}"

    for p in sorted(world.rglob("*.yaml")):
        if p.relative_to(world).parts[:2] == ("tools", "registry"):
            continue  # generated from BUILDER_TOOL_DEFS; compared byte-for-byte against the generator instead
        for where, key, val in walk(load_yaml(p)):
            if key and FORBIDDEN_KEY.match(str(key)):
                out.append(Violation("precomputed_finding", f"{rel(p)}#{where}", f"forbidden key {key!r}"))
            if isinstance(val, str) and FORBIDDEN_TEXT.search(val):
                out.append(Violation("precomputed_finding", f"{rel(p)}#{where}", "text claims a precomputed result"))
    for p in yaml_files(world, "agents"):
        a = load_yaml(p)
        if (a.get("mode") != "task" or a.get("invocable_by") != ["builder"] or a.get("subject_kinds") != []
                or a.get("min_auth_level") != "session"):
            out.append(Violation("stage_shape", rel(p), "stage agent must be mode=task, invocable_by=[builder], "
                                 "subject_kinds=[], min_auth_level=session"))
    for p in yaml_files(world, "flows"):
        nodes = load_yaml(p).get("nodes", [])
        first = nodes[0] if nodes else {}
        if not (first.get("type") == "tool"
                and str(first.get("config", {}).get("tool", "")).startswith("pulso/bind_context@")):
            out.append(Violation("stage_entry", rel(p), "first node must be tool pulso/bind_context"))
        for i, n in enumerate(nodes):
            at = f"{rel(p)}#/nodes/{i}"
            cfg = n.get("config", {})
            if n.get("type") in WAITING_NODES or (n.get("type") == "respond" and cfg.get("await")):
                out.append(Violation("stage_shape", at, f"node type {n.get('type')} waits on a principal"))
            if n.get("type") == "end":
                for k, v in (cfg.get("output_map") or {}).items():
                    if not (isinstance(v, str) and v.startswith(PATH_ROOTS)):
                        out.append(Violation("precomputed_finding", f"{at}/config/output_map/{k}",
                                             "output must map from a fact or slot path, not a literal"))
            if n.get("type") == "tool":
                for k, v in (cfg.get("args") or {}).items():
                    if isinstance(v, str) and not v.startswith(PATH_ROOTS) and (" " in v or len(v) > 40):
                        out.append(Violation("precomputed_finding", f"{at}/config/args/{k}",
                                             "literal sentence passed as a tool argument"))
            if n.get("type") == "agent":
                for where, key, val in walk(cfg.get("output_schema", {})):
                    if key in FORBIDDEN_SCHEMA_KEYS:
                        out.append(Violation("precomputed_finding", f"{at}/config/output_schema{where}",
                                             f"output_schema must not carry {key}"))
                    if (isinstance(val, dict) and isinstance(val.get("enum"), list) and len(val["enum"]) == 1
                            and not where.endswith("/schema_version")):
                        out.append(Violation("precomputed_finding", f"{at}/config/output_schema{where}",
                                             "single-valued enum pins an answer"))
    return out


# ------------------------------------------------------------------------------------------ manifest

def check_manifest(root: Path) -> list[Violation]:
    out: list[Violation] = []
    mpath, spath = root / "manifest.yaml", root / "expected-state.json"
    if not mpath.is_file() or not spath.is_file():
        return [Violation("layout_missing", "manifest.yaml|expected-state.json", "both files are required")]
    manifest, state = load_yaml(mpath), json.loads(spath.read_text("utf-8"))
    pin = manifest.get("pin", {})
    if pin.get("sha") != PIN_SHA or pin.get("contract_version") != PIN_CONTRACT_VERSION:
        out.append(Violation("pin_drift", "manifest.yaml#/pin", f"pin must be {PIN_SHA} / {PIN_CONTRACT_VERSION}"))
    actual = {w.name: files_digest(w) for w in worlds_of(root)}
    declared = {k: v.get("files_digest") for k, v in manifest.get("worlds", {}).items()}
    if set(actual) != set(declared):
        out.append(Violation("manifest_drift", "manifest.yaml#/worlds",
                             f"worlds differ: {sorted(set(actual) ^ set(declared))}"))
    for name, digest in actual.items():
        if declared.get(name) != digest:
            out.append(Violation("manifest_drift", f"manifest.yaml#/worlds/{name}",
                                 "files_digest does not match the world"))
    if manifest.get("capability_catalog_digest") != sha256_hex(canonical(capability_catalog(root))):
        out.append(Violation("manifest_drift", "manifest.yaml#/capability_catalog_digest", "catalog digest differs"))
    if set(state) != set(agent_ids(root)):
        out.append(Violation("expected_state_drift", "expected-state.json", "agents differ from the worlds"))
    ids = {a: s.get("release_id") for a, s in state.items()}
    if manifest.get("release_ids") != ids:
        out.append(Violation("expected_state_drift", "manifest.yaml#/release_ids", "differs from expected-state.json"))
    for a, s in state.items():
        if s.get("release_id") != "rel-" + str(s.get("release_hash", ""))[:16]:
            out.append(Violation("expected_state_drift", f"expected-state.json#/{a}",
                                 "release_id != rel-<release_hash[:16]>"))
    if manifest.get("expected_state_digest") != expected_state_digest(state):
        out.append(Violation("expected_state_drift", "manifest.yaml#/expected_state_digest",
                             "expected-state.json changed without the manifest"))
    return out


def check_all(root: Path) -> list[Violation]:
    out = check_layout(root) + check_stages(root) + check_manifest(root)
    with tempfile.TemporaryDirectory() as tmp:
        out += merge_worlds(root, Path(tmp) / "merged")
    return out


# ---------------------------------------------------------------------------------- agent-core tier

def checkout_path() -> Path:
    env = os.environ.get("AGENT_CORE_CHECKOUT")
    if env:
        return Path(env)
    for parent in Path(__file__).resolve().parents:
        cand = parent / "references" / "agent-core"
        if cand.is_dir():
            return cand
    raise FileNotFoundError("agent-core checkout not found; set AGENT_CORE_CHECKOUT")


def checkout_head(checkout: Path) -> str:
    return subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()


def agentcore_cmd(checkout: Path, *args: str) -> list[str]:
    return ["uv", "run", "--python", "3.12", "--locked", "--project", str(checkout), *args]


def _env() -> dict[str, str]:
    env = dict(os.environ)
    default_venv = Path(env.get("TEMP", tempfile.gettempdir())) / "agentcore-assets-venv"
    env.setdefault("UV_PROJECT_ENVIRONMENT", str(default_venv))
    env.setdefault("UV_LINK_MODE", "copy")
    return env


def agentcore_validate(checkout: Path, root: Path) -> tuple[int, dict[str, Any]]:
    proc = subprocess.run(agentcore_cmd(checkout, "agentcore", "validate", str(root), "--json"),
                          capture_output=True, text=True, env=_env(), cwd=str(checkout))
    return proc.returncode, json.loads(proc.stdout)


def _run_script(checkout: Path, script: str, *args: str) -> str:
    env = _env()
    env["PYTHONPATH"] = str(checkout)  # `testing` and `tests` helpers of the pinned checkout
    proc = subprocess.run(agentcore_cmd(checkout, "python", str(Path(__file__).with_name(script)), *args),
                          capture_output=True, text=True, env=env, cwd=str(checkout))
    if proc.returncode != 0:
        raise RuntimeError(f"{script} failed: {proc.stderr[-2000:]}")
    return proc.stdout


def compute_state(checkout: Path, merged_root: Path) -> dict[str, Any]:
    return json.loads(_run_script(checkout, "agentcore_state.py", str(merged_root)))


def import_in_memory(checkout: Path, merged_root: Path) -> dict[str, Any]:
    return json.loads(_run_script(checkout, "agentcore_import.py", str(merged_root)))


def render_registry_tooldefs(checkout: Path, out: Path) -> list[str]:
    return json.loads(_run_script(checkout, "gen_builder_tooldefs.py", str(out)))


def write_state(root: Path, checkout: Path) -> None:
    """Recompute expected-state.json and manifest.yaml from the worlds (the sanctioned way to change them)."""
    with tempfile.TemporaryDirectory() as tmp:
        merged = Path(tmp) / "merged"
        problems = merge_worlds(root, merged)
        if problems:
            raise SystemExit("\n".join(map(str, problems)))
        state = compute_state(checkout, merged)
    (root / "expected-state.json").write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", "utf-8")
    manifest = {
        "pin": {"repo": "agent-core", "sha": PIN_SHA, "contract_version": PIN_CONTRACT_VERSION},
        "worlds": {w.name: {"files_digest": files_digest(w)} for w in worlds_of(root)},
        "release_ids": {a: s["release_id"] for a, s in sorted(state.items())},
        "capability_catalog_digest": sha256_hex(canonical(capability_catalog(root))),
        "expected_state_digest": expected_state_digest(state),
    }
    (root / "manifest.yaml").write_text(
        "# Generated by `python tools/assetcheck.py write-state`; change it in the same PR as the asset.\n"
        + yaml.safe_dump(manifest, sort_keys=True), "utf-8")


def main(argv: list[str]) -> int:
    cmd = argv[0] if argv else "check"
    root = Path(argv[argv.index("--root") + 1]) if "--root" in argv else Path(__file__).resolve().parents[1]
    if cmd == "check":
        violations = check_all(root)
        for v in violations:
            print(v)
        return 1 if violations else 0
    if cmd == "write-state":
        write_state(root, checkout_path())
        return 0
    if cmd == "validate":
        ck, rc = checkout_path(), 0
        with tempfile.TemporaryDirectory() as tmp:
            targets = {w.name: w for w in worlds_of(root)}
            merged = Path(tmp) / "merged"
            merge_worlds(root, merged)
            targets["merged"] = merged
            for name, path in targets.items():
                code, doc = agentcore_validate(ck, path)
                print(name, "ok" if code == 0 else f"FAIL {doc.get('counts')}")
                rc |= code
        return rc
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
