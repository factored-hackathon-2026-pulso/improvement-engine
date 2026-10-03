"""`PulsoIds`: the single IdSource of the runtime (plan 17.4.2).

Ids are opaque UUIDv7 strings from the pinned `SystemIds` (the only randomness in the engine). Write keys of
the registry writer are derived by the L3 executor (A02 formula); this source only issues fresh ids, so a
crash before the run commit never reuses an action id."""

from __future__ import annotations

from agent_core.adapters.system_ids import SystemIds
from agent_core.ports.ids import IdKind


class PulsoIds:
    def __init__(self) -> None:
        self._inner = SystemIds()

    def new_id(self, kind: IdKind) -> str:
        return self._inner.new_id(kind)

    def secret_token(self) -> str:
        return self._inner.secret_token()

    def __repr__(self) -> str:  # never leak state
        return "PulsoIds()"
