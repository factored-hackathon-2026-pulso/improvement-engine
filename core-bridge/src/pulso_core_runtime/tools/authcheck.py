"""`POST /internal/v1/broker/authorizations/check` before every broker call and every write.

`allowed=false`, 5xx, timeout or an expired `valid_until` -> denied. Positive answers are cached only until
`valid_until` and only for the exact (operation, resources, payload digest), never across resources."""

from __future__ import annotations

from pulso_core_runtime.tools.broker import BrokerClient, BrokerError, BrokerTimeout, BrokerUnavailable
from pulso_core_runtime.tools.context import InvocationContext, InvocationRegistry

AUTH_DENIED = "pulso:authorization_denied"


def check(contexts: InvocationRegistry, broker: BrokerClient, ic: InvocationContext, operation: str,
          resources: list[str], payload_digest: str | None = None) -> str | None:
    """None when allowed, else the denial code."""
    key = (operation, tuple(resources), payload_digest)
    if contexts.auth_cached(ic.binding_ref, key):
        return None
    try:
        decision = broker.authorization_check(ic.binding_ref, operation, resources, payload_digest)
    except (BrokerError, BrokerTimeout, BrokerUnavailable):
        return AUTH_DENIED
    if not decision.allowed or decision.valid_until is None or decision.valid_until <= contexts.now():
        return AUTH_DENIED
    contexts.auth_store(ic.binding_ref, key, decision.valid_until)
    return None
