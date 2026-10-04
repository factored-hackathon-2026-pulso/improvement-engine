"""Digest-keyed drift gate for third-party contracts the engine is pinned to.

Version strings are not trusted: agent-core extended the `ProblemCode` enum without bumping `contracts/VERSION`.
This gate hashes CONTENT and compares it with the digests recorded in `pinned_digests.json`:

  * agent-core `contracts/` (openapi.json, schemas/, events/, registry/), VERSION reported separately;
  * support-platform schema sources (`PLATFORM_FILES`: openapi.json, tables, enums, audit/event catalogs).

Usage (read-only; never fetches, never writes unless --write is given):
  python drift_digest.py --agent-core <checkout|contracts dir> [--platform <checkout>] [--manifest <file>]
  python drift_digest.py ... --write        # re-pin: rewrites the `pinned` entries of the manifest (explicit act)

Exit codes: 0 no drift, 1 drift, 2 missing input / malformed manifest.
Digest = sha256 over sorted lines `<relpath>\\t<sha256(content with CRLF normalised to LF)>\\n`.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections.abc import Mapping
from pathlib import Path

MANIFEST = Path(__file__).resolve().parent / "pinned_digests.json"
AGENT_CORE_ROOTS = ("openapi.json", "schemas", "events", "registry")
PLATFORM_SRC = "backend/src/cc_platform"
PLATFORM_FILES = (
    "backend/openapi.json",
    f"{PLATFORM_SRC}/domain/cases/values.py",
    f"{PLATFORM_SRC}/infrastructure/persistence/sqlalchemy/tables.py",
    f"{PLATFORM_SRC}/application/audit/catalog.py",
    f"{PLATFORM_SRC}/domain/cases/events.py",
    f"{PLATFORM_SRC}/domain/ai/events.py",
)


class MissingInput(Exception):
    """A required file or directory is absent: never reported as a pass."""


def _norm(data: bytes) -> bytes:
    return data.replace(b"\r\n", b"\n")


def file_digests(files: Mapping[str, bytes]) -> dict[str, str]:
    return {k: hashlib.sha256(_norm(v)).hexdigest() for k, v in sorted(files.items())}


def digest_files(files: Mapping[str, bytes]) -> str:
    lines = "".join(f"{k}\t{h}\n" for k, h in file_digests(files).items())
    return hashlib.sha256(lines.encode("utf-8")).hexdigest()


def collect_agent_core(root: Path) -> dict[str, bytes]:
    """Content files under `root` (an agent-core `contracts/` dir or a wire snapshot with the same layout)."""
    out: dict[str, bytes] = {}
    for name in AGENT_CORE_ROOTS:
        p = root / name
        if p.is_file():
            out[name] = p.read_bytes()
        elif p.is_dir():
            for f in sorted(x for x in p.rglob("*") if x.is_file()):
                out[f.relative_to(root).as_posix()] = f.read_bytes()
        else:
            raise MissingInput(f"{p} not found")
    return out


def agent_core_contracts_dir(path: Path) -> Path:
    """Accepts a checkout root or its `contracts/` directory."""
    if (path / "contracts" / "openapi.json").is_file():
        return path / "contracts"
    if (path / "openapi.json").is_file():
        return path
    raise MissingInput(f"no agent-core contracts under {path}")


def agent_core_digest(contracts: Path) -> tuple[str, str]:
    version_file = contracts / "VERSION"
    if not version_file.is_file():
        raise MissingInput(f"{version_file} not found")
    return digest_files(collect_agent_core(contracts)), version_file.read_text("utf-8").strip()


def collect_platform(root: Path) -> dict[str, bytes]:
    out: dict[str, bytes] = {}
    for rel in PLATFORM_FILES:
        p = root / rel
        if not p.is_file():
            raise MissingInput(f"{p} not found")
        out[rel] = p.read_bytes()
    return out


def platform_digest(root: Path) -> str:
    return digest_files(collect_platform(root))


def _changed(old: Mapping[str, str], new: Mapping[str, str]) -> list[str]:
    return sorted(k for k in set(old) | set(new) if old.get(k) != new.get(k))


def check_agent_core(manifest: Mapping, path: Path) -> list[str]:
    pin = manifest["agent_core"]["pinned"]
    contracts = agent_core_contracts_dir(path)
    files = collect_agent_core(contracts)
    digest, version = agent_core_digest(contracts)
    if digest == pin["digest"]:
        return []
    problems = [f"agent-core contracts digest {digest} != pinned {pin['digest']} (pin {pin['sha'][:7]})"]
    for rel in _changed(pin.get("files", {}), file_digests(files)):
        problems.append(f"agent-core changed file: {rel}")
    if version == pin["contracts_version"]:
        problems.append(f"agent-core contracts changed without a VERSION change (still {version})")
    return problems


def check_platform(manifest: Mapping, path: Path) -> list[str]:
    pin = manifest["platform"]["pinned"]
    files = collect_platform(path)
    digest = digest_files(files)
    if digest == pin["digest"]:
        return []
    problems = [f"platform schema digest {digest} != pinned {pin['digest']} (pin {pin['sha'][:7]})"]
    problems += [f"platform changed file: {r}" for r in _changed(pin.get("files", {}), file_digests(files))]
    return problems


def write_pins(manifest: dict, agent_core: Path | None, platform: Path | None) -> dict:
    if agent_core is not None:
        c = agent_core_contracts_dir(agent_core)
        files = collect_agent_core(c)
        pin = manifest["agent_core"]["pinned"]
        pin.update(digest=digest_files(files), contracts_version=(c / "VERSION").read_text("utf-8").strip(),
                   files=file_digests(files))
    if platform is not None:
        files = collect_platform(platform)
        pin = manifest["platform"]["pinned"]
        pin.update(digest=digest_files(files), files=file_digests(files))
    return manifest


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--manifest", type=Path, default=MANIFEST)
    ap.add_argument("--agent-core", type=Path)
    ap.add_argument("--platform", type=Path)
    ap.add_argument("--write", action="store_true", help="re-pin: record the digests of the given checkouts")
    a = ap.parse_args(argv)
    if a.agent_core is None and a.platform is None:
        ap.error("give --agent-core and/or --platform")
    try:
        manifest = json.loads(a.manifest.read_text("utf-8"))
        if a.write:
            write_pins(manifest, a.agent_core, a.platform)
            a.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", "utf-8")
            print(f"re-pinned digests in {a.manifest}")
            return 0
        problems: list[str] = []
        if a.agent_core is not None:
            problems += check_agent_core(manifest, a.agent_core)
        if a.platform is not None:
            problems += check_platform(manifest, a.platform)
    except (MissingInput, KeyError, ValueError, OSError) as e:
        print(f"drift_digest: input error: {e}", file=sys.stderr)
        return 2
    for p in problems:
        print(f"drift: {p}")
    if not problems:
        print("drift_digest: no drift")
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
