"""G2: agent-core pin-watch, bump-readiness report (initial lane).

Compares the engine's agent-core pin (facts.json, ADR 0012, wire MANIFEST) against agent-core main using read-only
`gh api` GETs. Rate-limit tolerant (backoff), offline mode replays the last-known facts.json and never claims
`ready`. A pin is only well-formed when its SHA AND the MANIFEST digest agree (ADR records the digest).
Complements scripts/core/pin-watch (deep checks); this one answers: is a bump ready to start?
No secrets: gh holds its own auth. This script writes nothing unless --out is given."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

AGENT_CORE = "pulso-factored/agent-core"
WATCHED_PRS = (23, 24, 28)
FACTS = Path("docs/reports/gates/facts.json")
ADR_GLOB = "core-bridge/docs/adr/*agent-core-pin*.md"
WIRE_GLOB = "core-bridge/wire/agent_core@*/MANIFEST.json"
DIGEST_RE = re.compile(r"MANIFEST digest:\**\s*`([0-9a-f]{64})`")


class GhError(Exception):
    pass


class RateLimited(GhError):
    pass


def sha256_file(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def real_gh(endpoint: str) -> Any:
    """`gh api <endpoint>` (implicit GET only)."""
    p = subprocess.run(["gh", "api", endpoint], capture_output=True, text=True, timeout=120)
    if p.returncode != 0:
        msg = (p.stderr or p.stdout).strip()[:300]
        if "rate limit" in msg.lower() or "HTTP 429" in msg or "secondary" in msg.lower():
            raise RateLimited(msg)
        raise GhError(f"gh api {endpoint}: {msg}")
    return json.loads(p.stdout)


def with_retry(gh: Callable[[str], Any], endpoint: str, *, sleep: Callable[[float], None] = time.sleep,
               attempts: int = 4, base: float = 2.0) -> Any:
    for i in range(attempts):
        try:
            return gh(endpoint)
        except RateLimited as e:
            if i == attempts - 1:
                raise GhError(f"rate limited after {attempts} attempts: {e}") from e
            sleep(base * (2 ** i))
    raise GhError("unreachable")


@dataclass
class Pin:
    sha: str
    manifest_digest: str | None
    adr_digest: str | None
    contract_version: str | None
    problems: list[str] = field(default_factory=list)


def read_facts(repo_root: Path) -> dict:
    return json.loads((repo_root / FACTS).read_text(encoding="utf-8"))


def read_pin(repo_root: Path) -> Pin:
    """Pin SHA from facts.json; manifest digest computed from the wire dir and compared with the one the ADR records."""
    facts = read_facts(repo_root)
    sha = facts["agent_core"]["pin_sha"]
    problems: list[str] = []
    manifest = next((m for m in sorted(repo_root.glob(WIRE_GLOB)) if m.parent.name == f"agent_core@{sha[:7]}"), None)
    digest = sha256_file(manifest) if manifest else None
    cv = None
    if manifest:
        try:
            cv = json.loads(manifest.read_text(encoding="utf-8")).get("contract_version")
        except ValueError:
            problems.append("wire MANIFEST is not valid JSON")
    else:
        problems.append(f"no wire MANIFEST for pin {sha[:7]}")
    cv = cv or facts["agent_core"].get("contract_version")
    adr_digest = None
    for adr in sorted(repo_root.glob(ADR_GLOB)):
        if sha[:7] in adr.name:
            m = DIGEST_RE.search(adr.read_text(encoding="utf-8"))
            adr_digest = m.group(1) if m else None
    if adr_digest is None:
        problems.append("manifest digest not recorded in the pin ADR")
    elif digest and adr_digest != digest:
        problems.append("manifest digest mismatch between ADR and wire MANIFEST")
    return Pin(sha, digest, adr_digest, cv, problems)


def _offline(facts: dict, pin: Pin, why: str) -> dict:
    ac = facts["agent_core"]
    prs = [{"number": int(n), "state": str(v.get("state", "unknown")).lower(), "merge_sha": v.get("merge_sha"),
            "on_main": str(v.get("state", "")).upper() == "MERGED"} for n, v in sorted(ac.get("prs", {}).items(), key=lambda kv: int(kv[0]))]
    return {"source": "offline-last-known", "offline_reason": why, "facts_collected_at": facts.get("collected_at"),
            "main_sha": ac.get("main_sha"), "commits_ahead": ac.get("main_ahead_of_pin_by"), "prs": prs,
            "contract_version": {"pin": pin.contract_version, "main": ac.get("contract_version"), "changed": None},
            "readiness": "unknown", "blockers": [f"offline, readiness not evaluated ({why})"] +
            [f"pin: {p}" for p in pin.problems]}


def build_report(repo_root: Path, gh: Callable[[str], Any] | None, *, sleep: Callable[[float], None] = time.sleep,
                 offline: bool = False) -> dict:
    pin = read_pin(repo_root)
    facts = read_facts(repo_root)
    head = {"platform": "n/a", "pin": {"sha": pin.sha, "manifest_digest": pin.manifest_digest,
                                       "adr_digest": pin.adr_digest, "problems": pin.problems}}
    if offline or gh is None:
        return {**head, **_offline(facts, pin, "offline mode")}
    try:
        call = lambda ep: with_retry(gh, ep, sleep=sleep)  # noqa: E731
        cmp = call(f"repos/{AGENT_CORE}/compare/{pin.sha}...main")
        main_sha = (cmp.get("commits") or [{}])[-1].get("sha") or facts["agent_core"].get("main_sha")
        prs = []
        for n in WATCHED_PRS:
            pr = call(f"repos/{AGENT_CORE}/pulls/{n}")
            msha = pr.get("merge_commit_sha") if pr.get("merged") else None
            on_main = False
            if msha:
                c = call(f"repos/{AGENT_CORE}/compare/{msha}...main")
                on_main = c.get("status") in ("identical", "ahead")
            prs.append({"number": n, "state": "merged" if pr.get("merged") else pr.get("state"),
                        "merge_sha": msha, "on_main": on_main, "title": pr.get("title")})
        v = call(f"repos/{AGENT_CORE}/contents/contracts/VERSION?ref=main")
        main_version = base64.b64decode(v["content"]).decode().strip()
    except GhError as e:
        return {**head, **_offline(facts, pin, str(e)[:200])}
    blockers = [f"pin: {p}" for p in pin.problems]
    blockers += [f"PR {p['number']} not on main (state {p['state']})" for p in prs if not p["on_main"]]
    changed = main_version != pin.contract_version
    if changed:
        blockers.append(f"contract VERSION changed {pin.contract_version} -> {main_version}: breaking review, not an auto bump")
    return {**head, "source": "online", "main_sha": main_sha, "commits_ahead": cmp.get("ahead_by"), "prs": prs,
            "contract_version": {"pin": pin.contract_version, "main": main_version, "changed": changed},
            "readiness": "blocked" if blockers else "ready", "blockers": blockers}


def render_markdown(r: dict) -> str:
    cv = r["contract_version"]
    out = [f"# agent-core bump readiness: {r['readiness']}", "",
           f"- source: {r['source']}" + (f" (facts from {r['facts_collected_at']}; {r['offline_reason']})" if r["source"] != "online" else ""),
           f"- pin: {r['pin']['sha'][:7]}  main: {str(r.get('main_sha'))[:7]}  commits ahead: {r['commits_ahead']}",
           f"- contract VERSION: pin {cv['pin']} / main {cv['main']}",
           f"- manifest digest: {r['pin']['manifest_digest']}", "", "## PRs"]
    out += [f"- PR {p['number']}: {p['state']}, on main: {p['on_main']}" for p in r["prs"]]
    out += ["", "## Blockers"] + ([f"- {b}" for b in r["blockers"]] or ["- none"])
    return "\n".join(out) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo-root", default=".")
    ap.add_argument("--offline", action="store_true", help="use the last-known facts.json, no gh calls")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--out", help="also write the report to this path")
    a = ap.parse_args(argv)
    try:
        r = build_report(Path(a.repo_root), None if a.offline else real_gh, offline=a.offline)
    except (OSError, KeyError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    text = json.dumps(r, indent=2) if a.json else render_markdown(r)
    if a.out:
        Path(a.out).write_text(text, encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
