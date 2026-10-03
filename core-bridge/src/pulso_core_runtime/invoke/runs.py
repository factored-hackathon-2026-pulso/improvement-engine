"""`RunReader` over the runtime DSN (read role, never the eval DSN): `load_run` for the fact projection and
`get_run_idempotency` for reconciliation. In-process only; nothing here is exposed over HTTP."""

from __future__ import annotations

from typing import Any

from agent_core.adapters.postgres_uow import PostgresStore
from agent_core.domain.identity import PrincipalKey, PrincipalType


class PgRunReader:
    def __init__(self, runtime_dsn: str) -> None:
        self._store = PostgresStore(runtime_dsn)

    def load_run(self, run_id: str) -> Any | None:
        with self._store.uow() as uow:
            return uow.load_run(run_id)

    def get_run_idempotency(self, principal_id: str, key: str) -> tuple[str, Any] | None:
        with self._store.uow() as uow:
            return uow.get_run_idempotency(PrincipalKey(type=PrincipalType.builder, id=principal_id), key)
