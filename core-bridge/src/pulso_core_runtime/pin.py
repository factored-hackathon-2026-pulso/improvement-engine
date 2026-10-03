"""`PinnedRegistryPort` (plan 17.3.2, DR-28): `resolve_release` honours the signed `pin_release_id` attr."""

from __future__ import annotations

from typing import Any

from agent_core.domain import AgentSelector, EntityKind, EntityRef, Principal, RegistryEntity, Release
from agent_core.ports.registry import RegistryPort

PIN_ATTR = "pin_release_id"
PIN_UNAVAILABLE = "pulso:release_pin_unavailable"


class PinnedRegistryPort:
    """Wraps the Postgres registry. Principals without the attr fall through to the inner port; with it the
    exact release is loaded by id, must be active and must belong to the selected agent (and version)."""

    def __init__(self, inner: Any) -> None:  # PostgresRegistry (has .release(id))
        self._inner = inner

    def resolve_release(self, selector: AgentSelector, principal: Principal) -> Release:
        release_id = principal.attrs.get(PIN_ATTR)
        if release_id is None:
            return self._inner.resolve_release(selector, principal)
        try:
            if self._inner.release_status(release_id) != "active":
                raise KeyError(PIN_UNAVAILABLE)
            release: Release = self._inner.release(release_id)
        except KeyError:
            raise KeyError(PIN_UNAVAILABLE) from None
        version = release.entities.get(EntityKind.agent, {}).get(selector.id)
        if version is None or (selector.version is not None and version != selector.version):
            raise KeyError(PIN_UNAVAILABLE)
        pinned = release.model_copy(deep=True, update={"status": "active"})
        return pinned.model_copy(update={"id": release_id})

    def release_status(self, release_id: str) -> Any:
        return self._inner.release_status(release_id)

    def release(self, release_id: str) -> Release:
        return self._inner.release(release_id)

    def get[T: RegistryEntity](self, ref: EntityRef, kind: type[T]) -> T:
        return self._inner.get(ref, kind)


def _conforms(x: PinnedRegistryPort) -> RegistryPort:
    return x
