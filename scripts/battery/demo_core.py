#!/usr/bin/env python3
"""ISOLATED local agent-core instance that serves the DEMO agents (recepcion, disputas, consultas) for the battery.

    python scripts/battery/demo_core.py up|down [--purge]|health

scripts/dev-stack/stack.py only serves `pulso-builder` and is a shared singleton (fixed container names and ports:
another session running `up --reset-db` wipes the database under you; observed during EV2). This script therefore
starts its OWN containers (postgres :55442, llm-gateway :8090, built from the already-built gateway image) and its own
agent-core serve on :8002 with agent-core `tests/fixtures/registry-e2e` imported and the demo doubles plus the
battery tools (scripts/battery/battery_tools.py: seeded, recording tool double; AGENTCORE_ALLOW_DEMO=1, local only).
Secrets: loaded from the env files into child-process environments only; never printed or written elsewhere.
"""
from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import urllib.request
from pathlib import Path
from urllib.parse import urlparse, urlunparse

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("devstack", REPO / "scripts" / "dev-stack" / "stack.py")
ds = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ds)  # reuse load_env/pm/sh/wait only; stack.py main is guarded

STATE = ds.STATE / "battery"
# Own stack per session: PULSO_STACK_PREFIX names the containers, PULSO_STACK_PORT_{CORE,PG,GW} move the ports (defaults = EV2's).
PREFIX = os.environ.get("PULSO_STACK_PREFIX", "pulso-ev2")
PORT = int(os.environ.get("PULSO_STACK_PORT_CORE", "8002"))
DEMO_DB = "agentcore"
STATE = STATE if os.environ.get("PULSO_STACK_PREFIX", "pulso-ev2") == "pulso-ev2" else ds.STATE / f"battery-{os.environ['PULSO_STACK_PREFIX']}"
PG, GW = f"{PREFIX}-postgres", f"{PREFIX}-llm-gateway"
PG_PORT, GW_PORT = int(os.environ.get("PULSO_STACK_PORT_PG", "55442")), int(os.environ.get("PULSO_STACK_PORT_GW", "8090"))
AGENTS = "recepcion,disputas,consultas"


def ac_dir() -> Path:
    d = os.environ.get("PULSO_AGENT_CORE_DIR")
    if d:
        return Path(d)
    shared = REPO.parents[1] / "tmp" / "shared" / "agent-core"  # worktree layout: <factored>/tmp/shared/agent-core
    return shared if shared.exists() else ds.source_dir("PULSO_AGENT_CORE_DIR", "agent-core", ds.AGENT_CORE_REPO)


def core_env() -> dict[str, str]:
    env = ds.load_env(ds.env_path("PULSO_AGENT_CORE_ENV", "agent-core.env"))
    gw = ds.load_env(ds.env_path("PULSO_LLM_GATEWAY_ENV", "llm-gateway.env"))
    u = urlparse(env["AGENTCORE_REGISTRY_DSN"])
    env["AGENTCORE_REGISTRY_DSN"] = urlunparse(u._replace(netloc=u.netloc.rsplit(":", 1)[0] + f":{PG_PORT}",
                                                          path="/" + DEMO_DB))
    if env.get("AGENTCORE_EVAL_DSN"):  # the evaluation database lives in the SAME own Postgres (stack.py creates agentcore_eval too)
        e = urlparse(env["AGENTCORE_EVAL_DSN"])
        env["AGENTCORE_EVAL_DSN"] = urlunparse(e._replace(netloc=e.netloc.rsplit(":", 1)[0] + f":{PG_PORT}"))
    env["AGENTCORE_LLM_GATEWAY_URL"] = f"http://127.0.0.1:{GW_PORT}"
    tok = gw.get("GATEWAY_TOKEN_AGENT_CORE") or env.get("GATEWAY_TOKEN_AGENT_CORE")
    if tok:
        env["AGENTCORE_LLM_GATEWAY_TOKEN"] = tok
    env["AGENTCORE_ALLOW_DEMO"] = "1"
    env["BATTERY_SEED_FILE"] = str(STATE / "seeds.json")
    env["OTEL_SDK_DISABLED"] = "true"  # no local collector: avoid exporter retry noise/latency
    return env


def containers() -> None:
    ac_env = ds.load_env(ds.env_path("PULSO_AGENT_CORE_ENV", "agent-core.env"))
    gw_env = ds.load_env(ds.env_path("PULSO_LLM_GATEWAY_ENV", "llm-gateway.env"))
    for k in ("GATEWAY_TOKEN_AGENT_CORE", "OPENROUTER_API_KEY"):
        if k not in gw_env and k in ac_env:
            gw_env[k] = ac_env[k]
    dsn = urlparse(ac_env["AGENTCORE_REGISTRY_DSN"])
    user = dsn.username or "agentcore"
    pg_env = {"POSTGRES_USER": user, "POSTGRES_PASSWORD": dsn.password or "", "POSTGRES_DB": DEMO_DB}
    if ds.pm("container", "exists", PG, check=False).returncode != 0:
        ds.pm("run", "-d", "--pids-limit=0", "--name", PG, "-e", "POSTGRES_USER", "-e", "POSTGRES_PASSWORD",
              "-e", "POSTGRES_DB", "-p", f"127.0.0.1:{PG_PORT}:5432", "docker.io/library/postgres:16-alpine", env=pg_env)
    else:
        ds.pm("start", PG, check=False)
    ds.wait(lambda: ds.pm("exec", PG, "pg_isready", "-U", user, check=False).returncode == 0, "postgres")
    ds.time.sleep(3)
    has_eval = ds.pm("exec", PG, "psql", "-U", user, "-d", DEMO_DB, "-tAc", "SELECT 1 FROM pg_database WHERE datname='agentcore_eval'", check=False).stdout
    if "1" not in (has_eval or ""):
        ds.pm("exec", PG, "psql", "-U", user, "-d", DEMO_DB, "-c", "CREATE DATABASE agentcore_eval", check=False)
    if ds.pm("image", "exists", ds.GW_IMAGE, check=False).returncode != 0:
        sys.exit("gateway image missing: run `python scripts/dev-stack/stack.py up` once (builds it)")
    ds.pm("rm", "-f", GW, check=False)
    names = [k for k in ("GATEWAY_CONSUMERS", "LLM_ENDPOINTS", "GATEWAY_TOKEN_AGENT_CORE", "OPENROUTER_API_KEY",
                         "JEV_API_KEY") if k in gw_env]
    ds.pm("run", "-d", "--pids-limit=0", "--name", GW, "-p", f"127.0.0.1:{GW_PORT}:8080",
          *[a for k in names for a in ("-e", k)], ds.GW_IMAGE, env=gw_env)
    ds.wait(lambda: ds.http_ok(f"http://127.0.0.1:{GW_PORT}/healthz"), "llm-gateway")


def up() -> None:
    containers()
    ac, env = ac_dir(), core_env()
    ds.sh(["uv", "sync", "--locked", "--quiet"], cwd=ac)
    ds.sh(["uv", "run", "agentcore", "migrate"], env=env, cwd=ac)
    STATE.mkdir(parents=True, exist_ok=True)
    out = ds.sh(["uv", "run", "python", "-m", "testing.demo_identities", "--public-keys", str(STATE / "identity-keys.json"),
                 "--staff-keys", str(STATE / "staff-keys.json")], env=env, cwd=ac).stdout
    admin = json.loads(out)["admin"]
    (STATE / "tokens.json").write_text(out, encoding="utf-8")  # local-stack identities for the live tests of this lane (gitignored state dir, never printed)
    imp = ds.sh(["uv", "run", "agentcore", "registry", "--verifier", "testing.registry_demo:demo_verifier", "import",
                 os.environ.get("PULSO_REGISTRY_DIR") or str(ac / "tests" / "fixtures" / "registry-e2e")], env={**env, "AGENTCORE_CREDENTIAL": admin}, cwd=ac,
                check=False)
    txt = (imp.stdout or "") + (imp.stderr or "")
    if imp.returncode != 0 and "ya tiene releases" not in txt:
        sys.exit("registry import failed:\n" + txt[-800:])
    print("registry:", "already imported" if imp.returncode != 0 else "registry-e2e imported")
    down(quiet=True)
    log = open(STATE / "serve.log", "ab")
    flags = (0x00000008 | 0x00000200) if os.name == "nt" else 0
    proc = subprocess.Popen(
        ["uv", "run", "agentcore", "serve", "--port", str(PORT), "--registry-api",
         "--identity-keys", str(STATE / "identity-keys.json"), "--staff-keys", str(STATE / "staff-keys.json"),
         "--lang-thresholds", str(ac / "scripts" / "e2e" / "lang-thresholds.json"), "--agents", AGENTS,
         "--tools", "battery_tools:tools", "--classifier", "testing.e2e_demo:classifier_provider",
         "--field-classifier", "testing.e2e_demo:field_classifier", "--calibration", "testing.e2e_demo:calibration"],
        cwd=ac, env={**os.environ, **env, "PYTHONPATH": str(Path(__file__).parent) + os.pathsep + os.environ.get("PYTHONPATH", "")}, stdout=log, stderr=log, creationflags=flags)
    (STATE / "serve.pid").write_text(str(proc.pid))
    ds.wait(lambda: ds.http_ok(f"http://127.0.0.1:{PORT}/healthz"), "demo agent-core serve", tries=60)
    print(f"demo agent-core up on {PORT} (pid {proc.pid}); agents: {AGENTS}")


def down(quiet: bool = False, purge: bool = False) -> None:
    pidf = STATE / "serve.pid"
    if pidf.exists():
        pid = pidf.read_text().strip()
        subprocess.run(["taskkill", "/PID", pid, "/T", "/F"] if os.name == "nt" else ["kill", pid], capture_output=True)
        pidf.unlink()
    if purge:
        ds.pm("rm", "-f", GW, PG, check=False)
    if not quiet:
        print("demo agent-core down" + (" (own containers removed)" if purge else " (own containers kept)"))


def health() -> int:
    try:
        ok = urllib.request.urlopen(f"http://127.0.0.1:{PORT}/readyz", timeout=3).status == 200
    except Exception:
        ok = False
    print("OK  " if ok else "FAIL", f"demo agent-core :{PORT}")
    return 0 if ok else 1


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "health"
    if cmd == "down":
        down(purge="--purge" in sys.argv)
    elif cmd == "up":
        up()
    else:
        sys.exit(health())
