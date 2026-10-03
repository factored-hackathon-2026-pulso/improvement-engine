"""Entrypoint: `python -m local_identity`. Exit 2 on configuration problems (names the piece, never values)."""

from __future__ import annotations

import os
import sys
from collections.abc import Callable, Mapping
from typing import Any, TextIO

from local_identity.app import build_app
from local_identity.config import ConfigError, load_config


def run(
    env: Mapping[str, str] | None = None, *, serve: Callable[..., Any] | None = None, stderr: TextIO | None = None
) -> int:
    env = os.environ if env is None else env
    stderr = stderr or sys.stderr
    try:
        config = load_config(env)
    except ConfigError as exc:
        print(str(exc), file=stderr)
        return 2
    app = build_app(config)
    if serve is None:
        import uvicorn

        serve = uvicorn.run
    serve(app, host=config.host, port=config.port, log_config=None, access_log=False)
    return 0


def main() -> None:
    raise SystemExit(run())


if __name__ == "__main__":
    main()
