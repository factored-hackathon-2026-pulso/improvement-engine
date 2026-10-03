"""record-wire: write `fixtures/agent_core_wire/<sha7>/<case>.json` from a2 (CI) or from a real server (manual).

    python -m parity.record --target a2            # recorded from the real RegistryService, in memory
    python -m parity.record --target real --base-url http://...   # manual; never run against production
    python -m parity.record --target mock --only-mock-only          # mock_only cases (no upstream source)
"""

from __future__ import annotations

import argparse
import sys

import httpx

from parity import runner
from parity.servers import serve


def record(base_url: str, target: str, *, sim: bool, only_mock_only: bool) -> int:
    runner.FIXTURES.mkdir(parents=True, exist_ok=True)
    written = 0
    with httpx.Client(base_url=base_url, timeout=60) as client:
        for case in runner.load_cases():
            if (case.applies_to == "mock_only") != only_mock_only:
                continue
            if case.requires_sim and not sim:
                continue
            result = runner.run_case(client, case, sim=sim)
            runner.fixture_path(case.id).write_bytes(runner.dump_fixture(result).encode("utf-8"))  # LF on every OS
            written += 1
    return written


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", choices=["a2", "real", "mock"], required=True)
    ap.add_argument("--base-url")
    ap.add_argument("--only-mock-only", action="store_true")
    a = ap.parse_args()
    if a.target == "real":
        if not a.base_url:
            sys.exit("--base-url is required for --target real")
        print(record(a.base_url, "real", sim=False, only_mock_only=False), "fixtures")
        return
    with serve(a.target) as url:
        print(record(url, a.target, sim=True, only_mock_only=a.only_mock_only), "fixtures")


if __name__ == "__main__":
    main()
