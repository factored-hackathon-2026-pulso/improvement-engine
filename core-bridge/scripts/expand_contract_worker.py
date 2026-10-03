"""Worker of the expand/contract smoke (CAP-57 / N-11). Runs inside ONE agent-core interpreter (old or new pin).

    PULSO_CORE_CHECKOUT=<that pin's checkout> python expand_contract_worker.py <seed|exercise|fingerprint|engine> <dsn>

Prints one JSON line. It uses only that pin's public pieces (`PgRegistryStore`, `RegistryService`, `PostgresStore`)."""

from __future__ import annotations

import hashlib
import json
import os
import sys
from pathlib import Path

CHECKOUT = Path(os.environ["PULSO_CORE_CHECKOUT"])
sys.path[:0] = [str(CHECKOUT), str(Path(__file__).resolve().parents[2] / "platform-sim")]

import psycopg  # noqa: E402

from agent_core.adapters.system_clock import SystemClock  # noqa: E402
from agent_core.adapters.system_ids import SystemIds  # noqa: E402
from agent_core.registry import PgRegistryStore  # noqa: E402
from agent_core.registry.models import Origin  # noqa: E402
from agent_core.registry.service import RegistryService  # noqa: E402
from registry_mock.real_app import FixedPassEval, _admin  # noqa: E402


def service(dsn: str) -> RegistryService:
    store = PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False))
    return RegistryService(store, FixedPassEval(), SystemClock(), SystemIds())  # type: ignore[arg-type]


def fingerprint(dsn: str) -> str:
    """Hash of the whole relational shape (columns, types, nullability, defaults, indexes, constraints)."""
    queries = (
        "SELECT table_name, column_name, data_type, is_nullable, column_default FROM information_schema.columns "
        "WHERE table_schema = 'public' ORDER BY 1, 2",
        "SELECT tablename, indexname, indexdef FROM pg_indexes WHERE schemaname = 'public' ORDER BY 1, 2",
        "SELECT conrelid::regclass::text, conname, pg_get_constraintdef(oid) FROM pg_constraint "
        "WHERE connamespace = 'public'::regnamespace ORDER BY 1, 2",
    )
    h = hashlib.sha256()
    with psycopg.connect(dsn) as conn:
        for q in queries:
            for row in conn.execute(q).fetchall():
                h.update(("|".join(str(x) for x in row) + "\n").encode())
    return h.hexdigest()


def main() -> None:
    action, dsn = sys.argv[1], sys.argv[2]
    out: dict[str, object] = {"action": action}
    if action == "seed":
        seed = CHECKOUT / "tests" / "fixtures" / "registry-demo"
        out["releases"] = [r.release_id for r in service(dsn).import_seed(_admin(), seed)]
    elif action == "exercise":
        svc = service(dsn)
        base = svc.get_alias("atencion", "prod").release_id  # the seeded release id moves with every pin (Interrupt.locked in 894fa65)
        detail = svc.get_release(base).model_dump(mode="json")
        p = svc.create_proposal(_admin(), "atencion", Origin.manual, "expand-contract smoke")
        out.update(release_id=detail["release_id"], status=detail["status"], entities=len(detail["entities"]),
                   proposal_state=str(p.state.value))
    elif action == "fingerprint":
        out["fingerprint"] = fingerprint(dsn)
    elif action == "engine":
        from agent_core.adapters.postgres_uow import PostgresStore
        out["ping"] = bool(PostgresStore(dsn).ping())
    else:
        raise SystemExit(f"unknown action {action}")
    print(json.dumps(out, sort_keys=True))


if __name__ == "__main__":
    main()
