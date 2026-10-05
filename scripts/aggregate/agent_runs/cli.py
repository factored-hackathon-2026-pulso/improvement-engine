"""Command-line entrypoint for privacy-gated Agent Core run aggregates."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Sequence

from scripts.aggregate.agent_runs.aggregation import (
    ExportContractError,
    aggregate_export,
    write_immutable_report,
)


_REPO_ROOT = Path(__file__).resolve().parents[3]


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Create an aggregate-only report from a saved Agent Core export."
    )
    parser.add_argument("--input", required=True, type=Path, help="saved JSON export envelope")
    parser.add_argument("--output", required=True, type=Path, help="new report path outside the repository")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    try:
        with args.input.open("r", encoding="utf-8") as source:
            export = json.load(source)
        report = aggregate_export(export)
        write_immutable_report(report, args.output, repo_root=_REPO_ROOT)
    except (OSError, json.JSONDecodeError, ExportContractError) as exc:
        # Avoid echoing source data, paths, or JSON parser snippets in errors.
        parser.error(f"cannot safely create aggregate report ({type(exc).__name__})")
    print("Aggregate-only report written.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
