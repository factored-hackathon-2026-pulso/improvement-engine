"""Pieces shared by the mock and the a2 harness: clock, ids, /_sim contract, fixtures digest."""

from __future__ import annotations

import hashlib
import itertools
from datetime import datetime, timedelta
from pathlib import Path

from registry_mock.jws import SIM_EPOCH

PIN_SHA = "86a767474042a566a0dbd6ed23588959f27ebdb3"
CONTRACT_VERSION = "1.3.0"
FIXTURES_DIR = Path(__file__).resolve().parents[1] / "fixtures" / "agent_core_wire" / PIN_SHA[:7]
BASE_RELEASE_ID = "rel-98130317a1003849"
AGENT_ID = "atencion"


def fixtures_digest(directory: Path = FIXTURES_DIR) -> str:
    """sha256 over `name:sha256(file)` lines of the recorded wire fixtures (empty dir -> digest of nothing)."""
    h = hashlib.sha256()
    if directory.is_dir():
        for p in sorted(directory.glob("*.json")):
            data = p.read_bytes().replace(b"\r\n", b"\n")  # git autocrlf must not change the digest
            h.update(f"{p.name}:{hashlib.sha256(data).hexdigest()}\n".encode())
    return h.hexdigest()


class SimClock:
    """Injectable clock: starts at a fixed instant and only moves through `advance` (never wall time)."""

    def __init__(self) -> None:
        self._now = SIM_EPOCH
        self._mono = 0

    def now(self) -> datetime:
        return self._now

    def monotonic_ns(self) -> int:
        self._mono += 1000
        return self._mono

    def advance(self, seconds: float) -> None:
        self._now = self._now + timedelta(seconds=seconds)


class SimIds:
    def __init__(self) -> None:
        self._n: dict[str, itertools.count] = {}

    def new_id(self, kind: object) -> str:
        name = getattr(kind, "value", str(kind))
        counter = self._n.setdefault(name, itertools.count(1))
        return f"{name}-{next(counter):04d}"

    def secret_token(self) -> str:
        return "sim" + "A" * 40
