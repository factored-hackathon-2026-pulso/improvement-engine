"""Run the registry mock: `python -m registry_mock.main --port 8601` (real HTTP process, in-memory state)."""

from __future__ import annotations

import argparse

import uvicorn

from registry_mock.app import create_app


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8601)
    a = ap.parse_args()
    uvicorn.run(create_app(), host=a.host, port=a.port, log_level="warning", access_log=False)


if __name__ == "__main__":
    main()
