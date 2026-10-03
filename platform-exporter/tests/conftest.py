"""Shared rig: SQLite platform fixture + ingest fixture double (platform-sim/ingest_fixture, in-process ASGI)."""

from __future__ import annotations

from collections.abc import Callable, Iterator
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest
from fastapi.testclient import TestClient
from ingest_fixture.app import IngestState, create_app

from platform_exporter import Exporter, ExporterConfig, ExporterState, SqliteSource
from tests.platform_db import make_db


class FakeClock:
    def __init__(self) -> None:
        self.now = datetime(2026, 3, 2, 12, 0, 0, tzinfo=UTC)

    def __call__(self) -> datetime:
        return self.now

    def advance(self, seconds: float) -> None:
        self.now += timedelta(seconds=seconds)


@dataclass
class Rig:
    db: Any
    db_path: Path
    ingest: IngestState
    client: TestClient
    clock: FakeClock
    tmp: Path
    make: Callable[..., Exporter]


@pytest.fixture
def rig(tmp_path: Path) -> Iterator[Rig]:
    db_path = tmp_path / "platform.db"
    db = make_db(db_path)
    ingest = IngestState()
    client = TestClient(create_app(ingest), base_url="http://ingest.fixture")
    clock = FakeClock()
    made: list[Exporter] = []

    def make(state_path: Path | None = None, **cfg_over: Any) -> Exporter:
        cfg_over.setdefault("gap_grace_seconds", 0.0)
        cfg = ExporterConfig(tenant_id="tenant-1", instance="plat-a", binding_ref="binding-1", **cfg_over)
        ex = Exporter(cfg, SqliteSource(db_path), ExporterState(state_path or tmp_path / "state.sqlite"), client,
                      clock=clock, sleep=lambda s: clock.advance(s))
        made.append(ex)
        return ex

    try:
        yield Rig(db, db_path, ingest, client, clock, tmp_path, make)
    finally:
        for ex in made:
            ex.close()
        db.close()
