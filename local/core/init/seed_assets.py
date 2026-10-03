"""core-seed: idempotent bootstrap of agent-core-assets into the Core registry (plan 17.3.8 / 17.3.2 `seed`).

Contract: the second run with the same asset digest WRITES NOTHING (it only reads the marker row). The digest covers
every file of the asset tree (path + bytes), so changing an asset re-seeds. Exit codes: 0 seeded/unchanged,
3 `pulso:core_seed_failed` (registry import refused or not importable). Never prints DSNs or key material.

`seed_once(conn, digest, importer)` is the pure core and is unit-tested (local/core/tests/test_seed_assets.py).
"""

from __future__ import annotations

import hashlib
import os
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any

MARKER = "agent-core-assets"
SKIP_DIRS = {"tests", "tools", "ci", "__pycache__", ".pytest_cache"}
SKIP_FILES = {"pytest.ini", "expected-state.json"}


def assets_digest(root: Path) -> str:
    h = hashlib.sha256()
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        rel = path.relative_to(root)
        if rel.parts[0] in SKIP_DIRS or path.name in SKIP_FILES or "__pycache__" in rel.parts:
            continue
        h.update(rel.as_posix().encode() + b"\0" + hashlib.sha256(path.read_bytes()).digest())
    return h.hexdigest()


def seed_once(conn: Any, digest: str, importer: Callable[[], list[str]]) -> tuple[str, list[str]]:
    """Returns ("unchanged"|"seeded", release_ids). The only write on the unchanged path is none."""
    row = conn.execute("SELECT assets_digest FROM pulso_seed_state WHERE name=%s", (MARKER,)).fetchone()
    if row is not None and row[0] == digest:
        return "unchanged", []
    releases = importer()
    conn.execute(
        "INSERT INTO pulso_seed_state(name, assets_digest) VALUES (%s, %s) "
        "ON CONFLICT (name) DO UPDATE SET assets_digest = EXCLUDED.assets_digest, seeded_at = now()", (MARKER, digest))
    conn.commit()
    return "seeded", releases


def _import_into_registry(dsn: str, root: Path) -> list[str]:  # pragma: no cover - needs the pinned Core + Postgres
    """Real import through the pinned RegistryService. `import_seed` only writes to the store (it never calls the
    evaluator), so the evaluation port is a stub that fails closed if it is ever used. The actor is the local bootstrap
    administrator (human, step-up), i.e. the authorised bootstrap runbook step, never a bot."""
    import psycopg
    from agent_core.adapters.system_clock import SystemClock
    from agent_core.registry import PgRegistryStore, RegistryError, RegistryErrorCode, RegistryService

    from pulso_core_runtime.ids import PulsoIds

    class _NoEval:
        def run(self, request: Any) -> Any:
            raise RuntimeError("seed never evaluates")

    # Worlds are merged first (same merge as `assetcheck.py`), then the merged root is imported.
    sys.path.insert(0, str(root / "tools"))
    import tempfile

    import assetcheck  # type: ignore[import-not-found]
    import yaml

    store = PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False))
    service = RegistryService(store, _NoEval(), SystemClock(), PulsoIds())
    with tempfile.TemporaryDirectory() as tmp:
        merged = Path(tmp) / "merged"
        conflicts = assetcheck.merge_worlds(root, merged)
        if conflicts:
            raise RuntimeError(f"assets_drift: {len(conflicts)} merge conflicts")
        try:
            details = service.import_seed(_seed_admin(), merged)
        except RegistryError as exc:
            if exc.code == RegistryErrorCode.illegal_transition:  # registry already holds releases: adopt, write nothing
                return []
            raise
    expected = yaml.safe_load((root / "manifest.yaml").read_text(encoding="utf-8"))["release_ids"]
    got = {d.agent_id: d.release_id for d in details}
    if got != expected:
        raise RuntimeError(f"assets_drift: release ids differ from manifest ({sorted(set(got) ^ set(expected))})")
    return sorted(got.values())

def _seed_admin() -> Any:  # pragma: no cover
    from datetime import UTC, datetime, timedelta

    from agent_core.domain.identity import AuthInfo, AuthLevel, Principal, PrincipalType

    now = datetime.now(UTC)
    return Principal(type=PrincipalType.builder, id="pulso-local-seed", roles=["constructor", "aprobador", "admin"],
                     attrs={"actor": "human"}, auth=AuthInfo(level=AuthLevel.step_up, at=now),
                     exp=now + timedelta(minutes=15))

def main() -> int:
    import psycopg

    dsn = os.environ.get("AGENTCORE_REGISTRY_DSN", "")
    root = Path(os.environ.get("PULSO_ASSETS_DIR", "/seed/agent-core-assets"))
    if not dsn or not root.is_dir():
        print("pulso:core_seed_failed: AGENTCORE_REGISTRY_DSN unset or assets dir missing", file=sys.stderr)
        return 3
    digest = assets_digest(root)
    try:
        with psycopg.connect(dsn) as conn:
            status, releases = seed_once(conn, digest, lambda: _import_into_registry(dsn, root))
    except Exception as exc:  # noqa: BLE001 - report the type only, never the DSN
        print(f"pulso:core_seed_failed: {type(exc).__name__}: {str(exc)[:200]}", file=sys.stderr)
        return 3
    print(f"core-seed: {status} digest={digest[:12]} releases={len(releases)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
