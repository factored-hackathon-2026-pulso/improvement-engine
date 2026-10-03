"""core-synth (fixture profile, DOUBLE): writes one real hash chain (run_started, run_closed) through the pinned Core
audit code into audit_events so the exporter has something to export. Idempotent per run id. Synthetic data only.
Exit 3 `pulso:core_synth_unavailable` when the pinned domain models cannot be built (reported, never faked).
"""

from __future__ import annotations

import os
import sys
from datetime import UTC, datetime

RUN_ID = "run-synth-0001"


def main() -> int:
    try:
        import psycopg
        from agent_core.adapters.postgres_audit import PgAuditEvents
        from agent_core.audit.chain import chain_events
        from agent_core.domain import events as ev
        from agent_core.domain.identity import PrincipalType
        from agent_core.domain.outcomes import Outcome
    except Exception as exc:  # noqa: BLE001
        print(f"pulso:core_synth_unavailable: import {type(exc).__name__}", file=sys.stderr)
        return 3
    try:
        mode = "conversational"
    except Exception:  # noqa: BLE001
        mode = "conversational"
    now = datetime.now(UTC)
    try:
        with psycopg.connect(os.environ["AGENTCORE_REGISTRY_DSN"]) as conn:
            audit = PgAuditEvents(conn)
            if audit.last_event(RUN_ID) is not None:
                print("core-synth: chain already present, nothing written")
                return 0
            started = ev.RunStarted.model_validate({
                "event_id": "evt-synth-0001", "run_id": RUN_ID, "release": "rel-synth", "ts": now,
                "payload": {"agent": {"id": "synth", "version": "1.0.0"}, "mode": mode,
                            "principal_type": PrincipalType.service.value, "locale": "es"}})
            closed = ev.RunClosed.model_validate({
                "event_id": "evt-synth-0002", "run_id": RUN_ID, "release": "rel-synth", "ts": now,
                "payload": {"outcome": Outcome.resolved.value, "closed_by": "flow"}})
            audit.append_events(RUN_ID, chain_events(RUN_ID, [started, closed], None))
            conn.commit()
    except Exception as exc:  # noqa: BLE001
        print(f"pulso:core_synth_unavailable: {type(exc).__name__}: {str(exc)[:200]}", file=sys.stderr)
        return 3
    print("core-synth: wrote 2 chained events (double)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
