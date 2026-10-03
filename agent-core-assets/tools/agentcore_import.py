"""Run inside the agent-core env (cwd = checkout): import the merged root into an in-memory registry.

Usage: agentcore_import.py <root>  -> JSON with release ids, re-import code, bot code, session-admin code.
Level (a2) evidence: real Core registry code, no Postgres, no LLM (V3 31.10).
"""

import json
import sys
from pathlib import Path

from agent_core.registry.errors import RegistryError
from agent_core.registry.memory import InMemoryRegistryStore
from agent_core.registry.service import RegistryService
from testing.fakes.clock import FakeClock
from testing.fakes.ids import FakeIds
from tests.registry.helpers import admin, bot, human
from tests.registry.service_world import FakeEvaluator


def code_of(fn) -> str | None:
    try:
        fn()
    except RegistryError as exc:
        return exc.code.value
    return None


def fresh() -> tuple[InMemoryRegistryStore, RegistryService]:
    store = InMemoryRegistryStore()
    return store, RegistryService(store, FakeEvaluator(), FakeClock(), FakeIds())


def main(root: Path) -> None:
    store, service = fresh()
    details = service.import_seed(admin(), root)
    ids = {d.agent_id: d.release_id for d in details}
    again = code_of(lambda: service.import_seed(admin(), root))
    _, s2 = fresh()
    bot_code = code_of(lambda: s2.import_seed(bot(), root))
    _, s3 = fresh()
    session_code = code_of(lambda: s3.import_seed(human("constructor", "aprobador", "admin", level="session"), root))
    with store.transaction() as tx:
        prod = sorted(a for a, _ in tx.aliases_named("prod"))
        staging = sorted(a for a, _ in tx.aliases_named("staging"))
    sys.stdout.write(json.dumps({"release_ids": ids, "reimport_code": again, "bot_code": bot_code,
                                 "session_admin_code": session_code, "prod": prod, "staging": staging},
                                sort_keys=True) + "\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
