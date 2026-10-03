"""`exporter-report.json` evidence builder (plan 17.3.6): passed/failed/not_run, target, sha, doubles[]."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .config import PIN_CONTRACT_VERSION, PIN_SHA

CONTRACT_REVISION = "pulso-two-teams-1"


def build_report(outcomes: dict[str, str], *, target: str, sha: str, doubles: list[str]) -> dict[str, Any]:
    def pick(*kinds: str) -> list[str]:
        return sorted(n for n, o in outcomes.items() if o in kinds)

    return {"schema_version": "1", "suite": "exporter", "target": target, "sha": sha,
            "contract_revision": CONTRACT_REVISION,
            "agent_core_pin": {"sha": PIN_SHA, "contract": PIN_CONTRACT_VERSION},
            "passed": pick("passed"), "failed": pick("failed", "error"), "not_run": pick("skipped", "not_run"),
            "doubles": doubles}


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
