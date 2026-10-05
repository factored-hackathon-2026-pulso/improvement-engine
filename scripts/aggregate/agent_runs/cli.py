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
from scripts.aggregate.agent_runs.cell_table import render_cell_ndjson


_REPO_ROOT = Path(__file__).resolve().parents[3]


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Create an aggregate-only report from a saved Agent Core export."
    )
    parser.add_argument("--input", required=True, type=Path, help="saved JSON export envelope")
    parser.add_argument("--output", required=True, type=Path, help="new report path outside the repository")
    parser.add_argument(
        "--format", choices=("json", "ndjson"), default="json",
        help="JSON envelope (default) or exact six-field bank-cell NDJSON rows",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    try:
        with args.input.open("r", encoding="utf-8") as source:
            export = json.load(source)
        report = aggregate_export(export)
        payload = report if args.format == "json" else render_cell_ndjson(report["cells"])
        if args.format == "json":
            write_immutable_report(payload, args.output, repo_root=_REPO_ROOT)
        else:
            # Keep the same immutable, outside-checkout destination guarantees.
            from scripts.aggregate.agent_runs.aggregation import write_immutable_bytes
            write_immutable_bytes(payload.encode("utf-8"), args.output, repo_root=_REPO_ROOT)
    except (OSError, json.JSONDecodeError, ExportContractError) as exc:
        # Avoid echoing source data, paths, or JSON parser snippets in errors.
        parser.error(f"cannot safely create aggregate report ({type(exc).__name__})")
    print("Aggregate-only report written.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
