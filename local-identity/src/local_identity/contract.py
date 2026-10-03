"""CAP-63 contract scan: a local test kid or `auth.simulated=true` must never appear in remote configuration.

`scan_remote_config(root)` walks the repository and inspects only files that look like remote (staging/prod)
configuration; local compose, tests, fixtures, docs and this package are exempt."""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from pathlib import Path

PATTERNS = {
    "test_kid": re.compile(r"local-sim-(?:human|session)-"),
    "auth_simulated": re.compile(r"auth\.simulated|[\"']?simulated[\"']?\s*[:=]\s*true", re.IGNORECASE),
}
EXEMPT_DIRS = frozenset(
    {
        ".git",
        "node_modules",
        "target",
        ".venv",
        "__pycache__",
        "local",
        "tests",
        "fixtures",
        "docs",
        "local-identity",
        "worktrees",
        "references",
        "e2e-core",
        "platform-sim",
        "core-bridge",
        "debug-console",
        "agent-core-assets",
        "crates",
        "migrations",
    }
)
REMOTE_DIRS = frozenset(
    {"terraform", "infra", "infrastructure", "deploy", "deployment", "remote", "ecs", "helm", "k8s", "cdk"}
)
REMOTE_NAME = re.compile(r"(staging|stage|prod|production|\.tfvars$|\.tf$|task-?def|ecs)", re.IGNORECASE)
TEXT_SUFFIXES = frozenset({".tf", ".tfvars", ".json", ".yaml", ".yml", ".toml", ".env", ".hcl", ".ini", ".cfg"})
MAX_BYTES = 1_000_000


@dataclass(frozen=True)
class Finding:
    path: Path
    rule: str


def _is_remote(path: Path, root: Path) -> bool:
    parts = set(path.relative_to(root).parts[:-1])
    return bool(parts & REMOTE_DIRS) or bool(REMOTE_NAME.search(path.name))


def scan_remote_config(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for current, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in EXEMPT_DIRS]
        for name in files:
            path = Path(current, name)
            if path.suffix.lower() not in TEXT_SUFFIXES and not name.startswith(".env") or not _is_remote(path, root):
                continue
            try:
                if path.stat().st_size > MAX_BYTES:
                    continue
                text = path.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue
            findings.extend(Finding(path, rule) for rule, pattern in PATTERNS.items() if pattern.search(text))
    return findings
