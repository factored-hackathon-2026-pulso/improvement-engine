"""Real Postgres 16 for the llm tests (set PULSO_TEST_PG_ADMIN, see runtime/conftest). Re-exports the fixture."""

from __future__ import annotations

import pytest
from runtime.conftest import PgDbs, pg  # noqa: F401  (fixture re-export)


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"
