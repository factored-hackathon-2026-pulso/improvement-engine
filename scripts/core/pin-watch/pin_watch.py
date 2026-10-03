#!/usr/bin/env python3
"""pin-watch: detect upstream agent-core / llm-gateway drift against our committed pin and say what to do.

Read-only by design: GitHub is queried only through `gh api` GET calls (never remote-tracking branches); the
new SHA is inspected in a scratch checkout under the temp dir (never inside references/agent-core*); nothing is
pushed, committed or written outside --out-dir / the git-ignored state file / the scratch root. No secrets are read
or stored (gh holds its own auth).

Verdicts (exit code): no-change 0 | additive-safe 10 | needs-bump-work 20 | breaking 30 | tool error 1.
Stdlib only (runs under any Python >= 3.10).
"""
from __future__ import annotations

import argparse
import datetime as dt
import filecmp
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

AGENT_CORE_REPO = "pulso-factored/agent-core"
LLM_GATEWAY_REPO = "pulso-factored/llm-gateway"
VERDICTS = ("no-change", "additive-safe", "needs-bump-work", "breaking")
EXIT = {"no-change": 0, "additive-safe": 10, "needs-bump-work": 20, "breaking": 30}
SCHEMA_VERSION = 1
HERE = Path(__file__).resolve().parent


class PinWatchError(Exception):
    pass


class GhError(PinWatchError):
    pass


# ------------------------------------------------------------------ gh (read-only)
def real_gh(endpoint: str) -> Any:
    """`gh api <endpoint>` (implicit GET). Retries transient failures; never passes -X/-f/-F."""
    last = ""
    for _ in range(3):
        p = subprocess.run(["gh", "api", endpoint], capture_output=True, text=True, timeout=120,
                           env={**os.environ, "GH_PROMPT_DISABLED": "1", "NO_COLOR": "1"})
        if p.returncode == 0:
            try:
                return json.loads(p.stdout)
            except ValueError as e:
                raise GhError(f"gh api {endpoint}: invalid JSON ({e})") from e
        last = (p.stderr or p.stdout).strip()[:300]
    raise GhError(f"gh api {endpoint} failed: {last}")


def fixture_gh(directory: Path) -> Callable[[str], Any]:
    """Replay mode for tests/offline: endpoint `a/b?c` -> `<dir>/a_b_c.json`."""
    def run(endpoint: str) -> Any:
        f = directory / (endpoint.replace("/", "_").replace("?", "_") + ".json")
        if not f.exists():
            raise GhError(f"no fixture {f.name}")
        return json.loads(f.read_text(encoding="utf-8"))
    return run


# ------------------------------------------------------------------ pin
@dataclass
class Pin:
    sha: str
    contract_version: str
    source: str
    wire_dir: Path | None = None
    warnings: list[str] = field(default_factory=list)


def _hex40(s: object) -> bool:
    return isinstance(s, str) and re.fullmatch(r"[0-9a-f]{40}", s) is not None


def read_pin(repo_root: Path) -> Pin:
    """contracts/agent_core/pin.json when present, else the newest core-bridge/wire/*/MANIFEST.json. SHA-agnostic."""
    wire_root = repo_root / "core-bridge" / "wire"
    pin_json = repo_root / "contracts" / "agent_core" / "pin.json"
    if pin_json.exists():
        d = json.loads(pin_json.read_text(encoding="utf-8"))
        if not _hex40(d.get("sha")):
            raise PinWatchError(f"{pin_json}: `sha` must be 40 lowercase hex characters")
        wire = wire_root / f"agent_core@{d['sha'][:7]}"
        return Pin(d["sha"], str(d.get("contract_version", "")), str(pin_json), wire if wire.is_dir() else None)
    manifests = sorted(wire_root.glob("*/MANIFEST.json"), key=lambda p: (p.stat().st_mtime, p.parent.name))
    if not manifests:
        raise PinWatchError(f"no pin: neither {pin_json} nor {wire_root}/*/MANIFEST.json exists")
    m = manifests[-1]
    d = json.loads(m.read_text(encoding="utf-8"))
    if not _hex40(d.get("sha")):
        raise PinWatchError(f"{m}: `sha` must be 40 lowercase hex characters")
    warns = [f"{len(manifests)} wire dirs found; using newest {m.parent.name}"] if len(manifests) > 1 else []
    return Pin(d["sha"], str(d.get("contract_version", "")), str(m), m.parent, warns)


# ------------------------------------------------------------------ analysis
def classify_path(path: str) -> str:
    p = path.replace("\\", "/")
    base = p.rsplit("/", 1)[-1]
    if p.startswith("contracts/"):
        return "contracts"
    if p.startswith("migrations/") or "/migrations/" in p or p.endswith(".sql"):
        return "migrations"
    if base.lower().startswith("dockerfile"):
        return "dockerfile"
    if p.startswith("docs/") or p.endswith((".md", ".rst")):
        return "docs"
    if p.startswith(("tests/", "testing/")) or base.startswith("test_"):
        return "tests"
    parts = p.split("/")
    if parts[0] == "agent_core" and len(parts) > 1:
        return parts[1].rsplit(".", 1)[0]
    if parts[0] == "agent_telemetry":
        return "telemetry"
    return "other"


SIG_NAMES = re.compile(r"\b(?:def\s+(?:build_api_deps|resolve_ports|create_app|registry_extension|load_identity_verifier)"
                       r"|class\s+(?:ServePorts|ApiDeps|DemoContext|RegistryService|RunExport))\b")
PORTS_SHAPE = re.compile(r"^[+-]\s*(?:async\s+def |def |class |\w+\s*:\s*[\w\[\]\"'| .,]+(?:=.*)?$)")
ENV_RE = re.compile(r"\b((?:AGENTCORE|AGENT_CORE|PULSO)_[A-Z0-9_]{2,})\b")
ENV_GET = re.compile(r"""(?:env\.get|environ\.get|getenv|environ\[|env\[)\(?\s*["']([A-Z][A-Z0-9_]{3,})["']""")
ROUTE_RE = re.compile(r"""(?:@\w+\.(get|post|put|patch|delete)\(\s*|add_api_route\(\s*()|Route\(\s*())["']([^"']+)["']""")
PROBLEM_CODE = re.compile(r"""["'][a-z][a-z0-9_.-]*:[a-z][a-z0-9_.-]+["']""")


def _lines(patch: str | None) -> tuple[list[str], list[str]]:
    added, removed = [], []
    for ln in (patch or "").splitlines():
        if ln.startswith("+") and not ln.startswith("+++"):
            added.append(ln[1:])
        elif ln.startswith("-") and not ln.startswith("---"):
            removed.append(ln[1:])
    return added, removed


def _routes(lines: list[str]) -> set[str]:
    out = set()
    for ln in lines:
        m = ROUTE_RE.search(ln)
        if m:
            out.add(f"{(m.group(1) or 'ANY').upper()} {m.group(4)}")
    return out


def analyze_files(files: list[dict]) -> dict:
    """Group touched files by area and flag breaking-ish signals from path + patch text."""
    areas: dict[str, int] = {}
    by_area: dict[str, list[str]] = {}
    signals: list[dict] = []
    seen: set[tuple] = set()

    def sig(kind: str, file: str, detail: str = "") -> None:
        key = (kind, file, detail)
        if key not in seen:
            seen.add(key)
            signals.append({"kind": kind, "file": file, "detail": detail})

    for f in files:
        path, status, patch = f.get("filename", ""), f.get("status", ""), f.get("patch")
        area = classify_path(path)
        areas[area] = areas.get(area, 0) + 1
        by_area.setdefault(area, []).append(path)
        base = path.rsplit("/", 1)[-1]
        added, removed = _lines(patch)
        if path == "contracts/VERSION":
            sig("contracts_version", path, " -> ".join(x.strip() for x in (removed[:1] + added[:1])))
        if base == "openapi.json":
            sig("openapi_changed", path)
        elif re.search(r"^contracts/(schemas|registry|events)/", path):
            sig("schema_changed", path, status)
        if area == "migrations" and status in ("added", "renamed"):
            sig("migration_added", path)
        if area not in ("docs", "tests"):
            changed = added + removed
            if any(SIG_NAMES.search(x) for x in changed) or (
                    re.search(r"(^|/)ports/|composition/serve", path)
                    and any(PORTS_SHAPE.match(("+" if x in added else "-") + x) for x in changed)):
                sig("signature_change", path, "removed" if removed else "added")
            if re.search(r"problem|errors?\b", path, re.I) or any(
                    re.search(r"problem", x, re.I) and PROBLEM_CODE.search(x) for x in changed):
                sig("problem_code_change", path)
            old_env = set(ENV_RE.findall("\n".join(removed))) | set(ENV_GET.findall("\n".join(removed)))
            for var in sorted((set(ENV_RE.findall("\n".join(added))) | set(ENV_GET.findall("\n".join(added)))) - old_env):
                sig("new_env_var", path, var)
            old_routes, new_routes = _routes(removed), _routes(added)
            for r in sorted(new_routes - old_routes):
                sig("new_route", path, r)
            for r in sorted(old_routes - new_routes):
                sig("route_removed", path, r)
    return {"areas": dict(sorted(areas.items())), "files_by_area": {k: sorted(v) for k, v in sorted(by_area.items())},
            "signals": signals}


def decide_verdict(changed: bool, signals: list[dict], checks: dict[str, str]) -> str:
    if not changed:
        return "no-change"
    kinds = {s["kind"] for s in signals}
    ok = lambda k: checks.get(k) not in (None, "skipped")  # noqa: E731
    if kinds & {"contracts_version", "route_removed"} or checks.get("compat") == "fail" or checks.get("gen_wire") == "fail":
        return "breaking"
    if "signature_change" in kinds and not (ok("compat") and checks["compat"] == "pass"):
        return "breaking"
    if kinds or checks.get("wire_diff") == "drift" or checks.get("tests") == "fail":
        return "needs-bump-work"
    return "additive-safe"


def _worst(*vs: str) -> str:
    return max(vs, key=VERDICTS.index)


# ------------------------------------------------------------------ our files
RUNTIME = "core-bridge/src/pulso_core_runtime"
ALWAYS = ["contracts/agent_core/pin.json", "core-bridge/docs/adr/ (new pin ADR)",
          "core-bridge/src/pulso_core_runtime/compat.py", "platform-sim/**", "agent-core-assets/**"]
KIND_FILES = {
    "contracts_version": ["core-bridge/scripts/gen_wire.py", "core-bridge/scripts/gen-wire.ps1"],
    "openapi_changed": ["core-bridge/wire/agent_core@<new7>/openapi.json", "platform-sim/registry_mock/**", "e2e-core/**"],
    "schema_changed": ["core-bridge/wire/agent_core@<new7>/schemas|registry|events|derived/**", "platform-sim/**"],
    "signature_change": [f"{RUNTIME}/compat.py", f"{RUNTIME}/factories.py", f"{RUNTIME}/internal/app.py",
                         f"{RUNTIME}/main.py", f"{RUNTIME}/readiness.py"],
    "migration_added": [f"{RUNTIME}/store/**", "core-bridge/docker-entrypoint.sh", "core-bridge/Dockerfile"],
    "problem_code_change": [f"{RUNTIME}/errors.py", f"{RUNTIME}/internal/app.py"],
    "new_env_var": [f"{RUNTIME}/factories.py", f"{RUNTIME}/main.py", "core-bridge/Dockerfile",
                    "e2e-core/src/codex_standin/stack.py", "local/compose.yaml (Codex-owned: request change)"],
    "new_route": ["core-bridge/wire/agent_core@<new7>/openapi.json", "platform-sim/registry_mock/**", "e2e-core/**"],
    "route_removed": [f"{RUNTIME}/internal/app.py", "platform-sim/registry_mock/**", "e2e-core/**"],
}
AREA_FILES = {
    "registry": [f"{RUNTIME}/registry_service.py", f"{RUNTIME}/adapters.py", f"{RUNTIME}/pin.py"],
    "composition": [f"{RUNTIME}/factories.py", f"{RUNTIME}/main.py", f"{RUNTIME}/compat.py"],
    "api": [f"{RUNTIME}/internal/app.py"],
    "identity": [f"{RUNTIME}/internal/auth.py", f"{RUNTIME}/credentials/**"],
    "cli": ["core-bridge/Dockerfile", "core-bridge/docker-entrypoint.sh"],
    "dockerfile": ["core-bridge/Dockerfile"],
    "migrations": [f"{RUNTIME}/store/**"],
    "adapters": [f"{RUNTIME}/adapters.py", f"{RUNTIME}/store/**"],
}


def our_files_for(signals: list[dict], areas: dict[str, int], wire_drift: bool, new_sha: str | None) -> list[str]:
    out: list[str] = list(ALWAYS)
    for s in signals:
        out += KIND_FILES.get(s["kind"], [])
    for a in areas:
        out += AREA_FILES.get(a, [])
    if wire_drift:
        out.append("core-bridge/wire/agent_core@<new7>/** (regenerate)")
    seen, res = set(), []
    for f in out:
        f = f.replace("<new7>", (new_sha or "")[:7] or "<new7>")
        if f not in seen:
            seen.add(f)
            res.append(f)
    return res


# ------------------------------------------------------------------ scratch checkout + checks
def _git(*args: str, cwd: Path | None = None, url: str = "") -> subprocess.CompletedProcess:
    cmd = ["git"]
    if url.startswith("https://github.com"):
        cmd += ["-c", "credential.helper=!gh auth git-credential"]
    cmd += list(args)
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})


def make_scratch_checkout(source: str, sha: str, dest: Path) -> Path:
    """Clone `source` (URL or local path) into `dest` and detach at `sha`. Never touches `source` or references/."""
    dest = Path(dest)
    if any(p.lower().startswith("references") for p in dest.parts) and dest.name.lower().startswith("agent-core"):
        raise PinWatchError(f"refusing to use {dest} as scratch (references/agent-core* is read-only)")
    if not (dest / ".git").exists():
        dest.parent.mkdir(parents=True, exist_ok=True)
        extra = ["--filter=blob:none"] if source.startswith("https://") else []
        p = _git("clone", "--quiet", "--no-checkout", *extra, source, str(dest), url=source)
        if p.returncode:
            raise PinWatchError(f"git clone failed: {p.stderr.strip()[:300]}")
    if _git("cat-file", "-e", f"{sha}^{{commit}}", cwd=dest).returncode:
        p = _git("fetch", "--quiet", "origin", sha, cwd=dest, url=source)
        if p.returncode:
            raise PinWatchError(f"git fetch {sha[:7]} failed: {p.stderr.strip()[:300]}")
    p = _git("checkout", "--quiet", "--detach", "--force", sha, cwd=dest)
    if p.returncode:
        raise PinWatchError(f"git checkout {sha[:7]} failed: {p.stderr.strip()[:300]}")
    return dest


def venv_python(sha: str) -> Path:
    exe = "Scripts/python.exe" if os.name == "nt" else "bin/python"
    return Path(tempfile.gettempdir()) / f"pulso-wire-venv-{sha[:7]}" / exe


def ensure_venv(checkout: Path, sha: str) -> tuple[str | None, str]:
    """Venv for the NEW sha (same location/convention as gen-wire.ps1): reuse, else `uv sync --locked` from the
    scratch checkout into a temp venv outside the repo. Returns (python or None, note)."""
    py = venv_python(sha)
    if py.exists():
        return str(py), f"reused venv {py.parents[1].name}"
    env = {**os.environ, "UV_PROJECT_ENVIRONMENT": str(py.parents[1])}
    try:
        p = subprocess.run(["uv", "sync", "--locked", "--python", "3.12"], cwd=checkout, env=env, capture_output=True,
                           text=True, timeout=1200)
    except (OSError, subprocess.TimeoutExpired) as e:
        return None, f"uv sync failed to start: {e}"
    if p.returncode or not py.exists():
        return None, f"uv sync --locked failed: {_tail(p.stderr, 300)}"
    return str(py), f"built venv {py.parents[1].name} with uv sync --locked"


def find_python(pin_sha: str, override: str | None = None) -> str | None:
    if override or os.environ.get("PIN_WATCH_PYTHON"):
        return override or os.environ["PIN_WATCH_PYTHON"]
    tmp = Path(tempfile.gettempdir())
    exe = "Scripts/python.exe" if os.name == "nt" else "bin/python"
    cands = [tmp / f"pulso-wire-venv-{pin_sha[:7]}" / exe]
    cands += sorted(tmp.glob("pulso-wire-venv-*/" + exe), key=lambda p: p.stat().st_mtime, reverse=True)
    return next((str(c) for c in cands if c.exists()), None)


DEFAULT_COMPAT = ("import sys, agent_core; from pulso_core_runtime.compat import assert_compat; assert_compat(); "
                  "print('agent_core=' + agent_core.__file__)")


def _tail(text: str, n: int = 1200) -> str:
    return text.strip()[-n:]


def run_compat(checkout: Path, python: str, repo_root: Path, code: str = DEFAULT_COMPAT) -> dict:
    """OUR compat assertion against the scratch checkout via PYTHONPATH (checkout first)."""
    env = {**os.environ, "PYTHONPATH": os.pathsep.join([str(checkout), str(repo_root / "core-bridge" / "src")]),
           "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([python, "-W", "ignore", "-c", code], capture_output=True, text=True, env=env, timeout=300)
    return {"status": "pass" if p.returncode == 0 else "fail", "returncode": p.returncode,
            "output": _tail(p.stdout + "\n" + p.stderr)}


def make_shadow_repo(repo_root: Path, shadow: Path, sha: str, contract_version: str) -> Path:
    """gen_wire.py pins itself via <its repo>/contracts/agent_core/pin.json; a shadow copy lets us run the SAME
    generator against the new SHA without editing the real repo."""
    if shadow.exists():
        shutil.rmtree(shadow)
    (shadow / "core-bridge" / "scripts").mkdir(parents=True)
    shutil.copy2(repo_root / "core-bridge" / "scripts" / "gen_wire.py", shadow / "core-bridge" / "scripts" / "gen_wire.py")
    shutil.copytree(repo_root / "platform-sim", shadow / "platform-sim",
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc", ".pytest_cache", "evidence"))
    pin = shadow / "contracts" / "agent_core"
    pin.mkdir(parents=True)
    (pin / "pin.json").write_text(json.dumps({"sha": sha, "contract_version": contract_version or "1.3.0"}), encoding="utf-8")
    return shadow


def run_gen_wire(shadow: Path, checkout: Path, out: Path, python: str, sha: str) -> dict:
    if out.exists():
        shutil.rmtree(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([python, "-W", "ignore", str(shadow / "core-bridge" / "scripts" / "gen_wire.py"), "--checkout",
                        str(checkout), "--out", str(out), "--expected-sha", sha], capture_output=True, text=True,
                       env=env, timeout=900)
    return {"status": "pass" if p.returncode == 0 else "fail", "returncode": p.returncode,
            "output": _tail(p.stdout + "\n" + p.stderr)}


WIRE_CATS = ("schemas", "registry", "events", "derived", "golden")


def _wire_files(root: Path) -> dict[str, Path]:
    return {f.relative_to(root).as_posix(): f for f in root.rglob("*") if f.is_file() and f.name != "MANIFEST.json"}


def _cat(rel: str) -> str:
    top = rel.split("/", 1)[0]
    return top if top in WIRE_CATS else ("openapi" if rel == "openapi.json" else "other")


def _route_table(wire: Path) -> set[str]:
    f = wire / "openapi.json"
    if not f.exists():
        return set()
    paths = json.loads(f.read_text(encoding="utf-8")).get("paths", {})
    return {f"{m.upper()} {p}" for p, ops in paths.items() for m in ops
            if m.lower() in ("get", "post", "put", "patch", "delete", "head", "options")}


def diff_wire(committed: Path, fresh: Path) -> dict:
    """Byte-diff of the generated wire dir vs the committed one (MANIFEST sha/tool churn ignored) + route table."""
    a, b = _wire_files(committed), _wire_files(fresh)
    res: dict[str, Any] = {"added": {}, "removed": {}, "changed": {}}
    for rel in sorted(b.keys() - a.keys()):
        res["added"].setdefault(_cat(rel), []).append(rel)
    for rel in sorted(a.keys() - b.keys()):
        res["removed"].setdefault(_cat(rel), []).append(rel)
    for rel in sorted(a.keys() & b.keys()):
        if not filecmp.cmp(a[rel], b[rel], shallow=False):
            res["changed"].setdefault(_cat(rel), []).append(rel)
    ra, rb = _route_table(committed), _route_table(fresh)
    res["routes_added"], res["routes_removed"] = sorted(rb - ra), sorted(ra - rb)
    res["drift"] = bool(res["added"] or res["removed"] or res["changed"])
    return res


def run_tests(repo_root: Path, checkout: Path, python: str) -> dict:
    env = {**os.environ, "PYTHONPATH": os.pathsep.join([str(checkout), str(repo_root / "core-bridge" / "src")]),
           "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([python, "-W", "ignore", "-m", "pytest", "-q", "-x", "-p", "no:cacheprovider", "-k",
                        "not pg and not postgres and not integration", "tests"], cwd=repo_root / "core-bridge",
                       capture_output=True, text=True, env=env, timeout=1800)
    return {"status": "pass" if p.returncode == 0 else "fail", "returncode": p.returncode, "output": _tail(p.stdout, 800)}


# ------------------------------------------------------------------ orchestration
def _commit_rows(commits: list[dict], limit: int = 100) -> list[dict]:
    rows = []
    for c in commits[-limit:]:
        cm = c.get("commit", {})
        rows.append({"sha": c.get("sha", ""), "subject": cm.get("message", "").splitlines()[0] if cm.get("message") else "",
                     "author": cm.get("author", {}).get("name", ""), "date": cm.get("author", {}).get("date", "")})
    return rows


def _load_state(path: Path | None) -> dict:
    if path and path.exists():
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except ValueError:
            return {}
    return {}


def run_watch(repo_root: Path, gh: Callable[[str], Any], *, run_checks: bool = True, with_tests: bool = False,
              state_path: Path | None = None, since_last: bool = False, update_state: bool = False,
              scratch_root: Path | None = None, out_dir: Path | None = None, clone_source: str | None = None,
              python: str | None = None, llm_baseline: str | None = None, include_prs: bool = False,
              max_prs: int = 30, sync_venv: bool = True, agent_core_repo: str = AGENT_CORE_REPO, llm_repo: str = LLM_GATEWAY_REPO) -> dict:
    repo_root = Path(repo_root)
    scratch_root = Path(scratch_root or Path(tempfile.gettempdir()) / "pin-watch-scratch")
    pin = read_pin(repo_root)
    state = _load_state(state_path)
    notes: list[str] = list(pin.warnings)

    main_sha = gh(f"repos/{agent_core_repo}/commits/main")["sha"]
    pulls = gh(f"repos/{agent_core_repo}/pulls?state=open&per_page=100")
    llm_main = None
    try:
        llm_main = gh(f"repos/{llm_repo}/commits/main")["sha"]
    except GhError as e:
        notes.append(f"llm-gateway unavailable: {e}")
    pr_heads = {str(p["number"]): p["head"]["sha"] for p in pulls}

    report: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION, "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "pin": {"sha": pin.sha, "contract_version": pin.contract_version, "source": pin.source,
                "wire_dir": str(pin.wire_dir) if pin.wire_dir else None},
        "since_last": {"enabled": since_last, "skipped": False, "previous_verdict": state.get("last_verdict")},
        "notes": notes, "checks": {}, "check_details": {},
    }

    def save(verdict: str) -> None:
        if state_path and (since_last or update_state):
            state_path.parent.mkdir(parents=True, exist_ok=True)
            state_path.write_text(json.dumps({
                "pin_sha": pin.sha, "agent_core_main": main_sha, "llm_gateway_main": llm_main or state.get("llm_gateway_main"),
                "open_pr_heads": pr_heads, "last_verdict": verdict, "updated_at": report["generated_at"]},
                indent=2, sort_keys=True), encoding="utf-8")

    unchanged = (state.get("pin_sha") == pin.sha and state.get("agent_core_main") == main_sha
                 and state.get("llm_gateway_main") == llm_main and state.get("open_pr_heads") == pr_heads)
    if since_last and unchanged:
        report["since_last"]["skipped"] = True
        report.update({"verdict": "no-change", "agent_core": {"repo": agent_core_repo, "main_sha": main_sha,
                       "changed": main_sha != pin.sha, "commits": [], "open_prs": []},
                       "llm_gateway": {"repo": llm_repo, "main_sha": llm_main, "changed": False, "baseline": llm_main},
                       "our_files_likely_to_change": []})
        report["exit_code"] = 0
        notes.append("upstream unchanged since last run; re-run without --since-last to force a full analysis")
        return report

    # --- agent-core main vs pin
    changed = main_sha != pin.sha
    ac: dict[str, Any] = {"repo": agent_core_repo, "main_sha": main_sha, "changed": changed, "ahead_by": 0,
                          "commits": [], "areas": {}, "files_by_area": {}, "signals": [], "files_truncated": False}
    analysis = {"areas": {}, "files_by_area": {}, "signals": []}
    if changed:
        cmp_ = gh(f"repos/{agent_core_repo}/compare/{pin.sha}...{main_sha}?per_page=100")
        files = cmp_.get("files", [])
        analysis = analyze_files(files)
        ac.update(ahead_by=cmp_.get("ahead_by", len(cmp_.get("commits", []))), commits=_commit_rows(cmp_.get("commits", [])),
                  files=len(files), files_truncated=len(files) >= 300, **analysis)
        if cmp_.get("status") == "behind":
            notes.append("upstream main is BEHIND the pin (pin is ahead of main): nothing to adopt")
        if ac["files_truncated"]:
            notes.append("GitHub compare returned >=300 files; file-level signals may be incomplete")
    # --- open PRs (advisory)
    prs = []
    for p in pulls[:max_prs]:
        row = {"number": p["number"], "title": p.get("title", ""), "url": p.get("html_url", ""),
               "draft": bool(p.get("draft")), "head_sha": p["head"]["sha"], "base": p.get("base", {}).get("ref", ""),
               "areas": {}, "signals": []}
        try:
            pa = analyze_files(gh(f"repos/{agent_core_repo}/pulls/{p['number']}/files?per_page=100"))
            row.update(areas=pa["areas"], signals=pa["signals"], verdict_hint=decide_verdict(True, pa["signals"], {}))
        except GhError as e:
            row["error"] = str(e)
        prs.append(row)
    ac["open_prs"] = prs

    # --- checks in a scratch checkout of the new SHA
    checks: dict[str, str] = {}
    wire_drift = False
    if run_checks and changed:
        py = python or os.environ.get("PIN_WATCH_PYTHON")
        co = None
        try:
            co = make_scratch_checkout(clone_source or f"https://github.com/{agent_core_repo}.git", main_sha,
                                       scratch_root / f"agent-core-{main_sha[:7]}")
            if not py and sync_venv:
                py, note = ensure_venv(co, main_sha)
                notes.append(note)
            py = py or find_python(pin.sha)
            if not py:
                notes.append("checks skipped: no venv python (set --python / PIN_WATCH_PYTHON)")
        except (PinWatchError, OSError) as e:
            checks["scratch"] = "fail"
            notes.append(f"scratch checkout failed: {e}")
        if py and co:
            try:
                comp = run_compat(co, py, repo_root)
                checks["compat"] = comp["status"]
                report["check_details"]["compat"] = comp
                shadow = make_shadow_repo(repo_root, scratch_root / "shadow", main_sha, pin.contract_version)
                out = scratch_root / "wire-out" / f"agent_core@{main_sha[:7]}"
                gw = run_gen_wire(shadow, co, out, py, main_sha)
                checks["gen_wire"] = gw["status"]
                report["check_details"]["gen_wire"] = gw
                if gw["status"] == "pass" and pin.wire_dir:
                    wd = diff_wire(pin.wire_dir, out)
                    checks["wire_diff"] = "drift" if wd["drift"] else "clean"
                    report["check_details"]["wire_diff"] = wd
                    wire_drift = wd["drift"]
                if with_tests:
                    t = run_tests(repo_root, co, py)
                    checks["tests"] = t["status"]
                    report["check_details"]["tests"] = t
            except (PinWatchError, subprocess.TimeoutExpired, OSError) as e:
                checks["scratch"] = "fail"
                notes.append(f"scratch checks aborted: {e}")
    report["checks"] = checks
    ac_verdict = decide_verdict(changed, analysis["signals"], checks)
    if checks.get("scratch") == "fail":
        ac_verdict = _worst(ac_verdict, "needs-bump-work")
    report["agent_core"] = ac
    pr_verdicts = [r["verdict_hint"] for r in prs if "verdict_hint" in r]

    # --- llm-gateway vs last-seen baseline (it has no pin of its own)
    baseline = llm_baseline or state.get("llm_gateway_main")
    lg: dict[str, Any] = {"repo": llm_repo, "main_sha": llm_main, "baseline": baseline, "changed": False,
                          "commits": [], "files": [], "areas": {}, "signals": [], "verdict": "no-change"}
    if llm_main and baseline and baseline != llm_main:
        try:
            c = gh(f"repos/{llm_repo}/compare/{baseline}...{llm_main}?per_page=100")
            la = analyze_files(c.get("files", []))
            interface = any(s["kind"] in ("openapi_changed", "schema_changed", "new_route", "route_removed",
                                          "new_env_var", "problem_code_change") for s in la["signals"]) or \
                any(a in la["areas"] for a in ("api", "contracts"))
            lg.update(changed=True, commits=_commit_rows(c.get("commits", [])), files=[f["filename"] for f in c.get("files", [])],
                      areas=la["areas"], signals=la["signals"], verdict="needs-bump-work" if interface else "additive-safe")
        except GhError as e:
            notes.append(f"llm-gateway compare failed: {e}")
    elif llm_main and not baseline:
        notes.append("llm-gateway: no baseline yet; run with --since-last (or --llm-baseline SHA) to start tracking")
    report["llm_gateway"] = lg

    verdict = _worst(ac_verdict, lg["verdict"], *(pr_verdicts if include_prs else []))
    report["verdict"], report["exit_code"] = verdict, EXIT[verdict]
    report["our_files_likely_to_change"] = (our_files_for(analysis["signals"] + lg["signals"], analysis["areas"],
                                                          wire_drift, main_sha) if verdict != "no-change" else [])
    save(verdict)
    return report


def render_markdown(r: dict) -> str:
    ac, lg = r["agent_core"], r["llm_gateway"]
    L = [f"# pin-watch: {r['verdict']}", "",
         f"- pin: `{r['pin']['sha'][:12]}` (contracts {r['pin']['contract_version']}) from `{r['pin']['source']}`",
         f"- agent-core main: `{(ac['main_sha'] or '')[:12]}` ({'CHANGED' if ac['changed'] else 'same as pin'})",
         f"- llm-gateway main: `{(lg['main_sha'] or '')[:12]}` ({'CHANGED since ' + str(lg['baseline'])[:12] if lg['changed'] else 'no change / no baseline'})",
         f"- generated: {r['generated_at']}  exit code: {r['exit_code']}", ""]
    if r["since_last"].get("skipped"):
        L.append(f"Skipped (nothing new since last run; previous verdict: {r['since_last'].get('previous_verdict')}).")
    if ac.get("commits"):
        L += [f"## agent-core commits since pin ({ac.get('ahead_by')})"] + [f"- `{c['sha'][:7]}` {c['subject']}" for c in ac["commits"]] + [""]
    if ac.get("areas"):
        L += ["## Files by area"] + [f"- {a}: {n}" for a, n in ac["areas"].items()] + [""]
    if ac.get("signals"):
        L += ["## Breaking / adaptation signals"] + [f"- **{s['kind']}** `{s['file']}` {s['detail']}".rstrip() for s in ac["signals"]] + [""]
    if r.get("checks"):
        L += ["## Checks"] + [f"- {k}: {v}" for k, v in r["checks"].items()] + [""]
        wd = r["check_details"].get("wire_diff")
        if wd and wd.get("drift"):
            L += ["Wire drift: " + ", ".join(f"{k}={sum(len(v) for v in wd[k].values())}" for k in ("added", "removed", "changed")),
                  f"Routes added: {wd['routes_added']}  removed: {wd['routes_removed']}", ""]
    if ac.get("open_prs"):
        L += ["## Open agent-core PRs (advisory)"]
        for p in ac["open_prs"]:
            L.append(f"- #{p['number']} {p['title']} [{p.get('verdict_hint', '?')}] " +
                     ", ".join(sorted({s['kind'] for s in p["signals"]})))
        L.append("")
    if lg.get("changed"):
        L += [f"## llm-gateway ({lg['verdict']})"] + [f"- {f}" for f in lg["files"][:50]] + [""]
    if r.get("our_files_likely_to_change"):
        L += ["## Our files likely to change"] + [f"- `{f}`" for f in r["our_files_likely_to_change"]] + [""]
    if r.get("notes"):
        L += ["## Notes"] + [f"- {n}" for n in r["notes"]] + [""]
    return "\n".join(L)


# ------------------------------------------------------------------ CLI
def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo-root", default=str(HERE.parents[2]))
    ap.add_argument("--json", action="store_true", help="print the JSON report on stdout (default: markdown)")
    ap.add_argument("--since-last", action="store_true", help="skip analysis when upstream SHAs equal the last-seen state")
    ap.add_argument("--state-file", help="default: <here>/.state/last-seen.json (git-ignored)")
    ap.add_argument("--out-dir", help="default: <here>/out (git-ignored)")
    ap.add_argument("--no-checks", action="store_true", help="API-only: skip scratch checkout / compat / gen_wire")
    ap.add_argument("--run-tests", action="store_true", help="also run our non-PG core-bridge tests against the new SHA")
    ap.add_argument("--no-sync", action="store_true", help="do not build a venv for the new SHA (use the pin's venv)")
    ap.add_argument("--python", help="pinned-venv python for the checks (default: %%TEMP%%/pulso-wire-venv-<sha7>)")
    ap.add_argument("--clone-source", help="git URL/path to clone the new SHA from (default: GitHub over gh credentials)")
    ap.add_argument("--scratch-root", help="default: <tmp>/pin-watch-scratch")
    ap.add_argument("--llm-baseline", help="llm-gateway SHA to compare against when no state exists")
    ap.add_argument("--include-prs", action="store_true", help="let open-PR hints raise the overall verdict")
    ap.add_argument("--max-prs", type=int, default=30)
    ap.add_argument("--gh-fixtures", help="replay recorded gh JSON from this dir (tests/offline)")
    a = ap.parse_args(argv)
    repo_root = Path(a.repo_root).resolve()
    out_dir = Path(a.out_dir) if a.out_dir else HERE / "out"
    state = Path(a.state_file) if a.state_file else HERE / ".state" / "last-seen.json"
    try:
        gh = fixture_gh(Path(a.gh_fixtures)) if a.gh_fixtures else real_gh
        rep = run_watch(repo_root, gh, run_checks=not a.no_checks, with_tests=a.run_tests, state_path=state,
                        since_last=a.since_last, scratch_root=Path(a.scratch_root) if a.scratch_root else None,
                        clone_source=a.clone_source, python=a.python, llm_baseline=a.llm_baseline,
                        include_prs=a.include_prs, max_prs=a.max_prs, sync_venv=not a.no_sync)
    except (PinWatchError, OSError, KeyError, ValueError) as e:
        print(f"pin-watch error: {e}", file=sys.stderr)
        return 1
    out_dir.mkdir(parents=True, exist_ok=True)
    md = render_markdown(rep)
    (out_dir / "pin-watch-report.json").write_text(json.dumps(rep, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (out_dir / "pin-watch-report.md").write_text(md + "\n", encoding="utf-8")
    print(json.dumps(rep, indent=2, sort_keys=True) if a.json else md)
    return rep["exit_code"]


if __name__ == "__main__":
    sys.exit(main())
