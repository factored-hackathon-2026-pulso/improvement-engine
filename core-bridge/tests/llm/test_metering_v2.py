"""WP-A first RED: metering v2 (D1-D5) on real PG16. Failed calls that carry usage are metered, the call that crosses
the cap is counted (never silently dropped), `usage_known` is honoured, spend is pre-reserved atomically and every
call outcome lands in `model_call_ledger` (tenant/job/binding scoped, no prompt or response content)."""

from __future__ import annotations

import threading
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from types import SimpleNamespace
from typing import Any

import psycopg
import pytest
from agent_core.domain.errors import GatewayError, GatewayErrorKind
from agent_core.ports import GenerationResult

from pulso_core_runtime.adapters import SpendMeteringGateway
from pulso_core_runtime.llm.policy import ModelPolicy, StageModelPolicy
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

pytestmark = [pytest.mark.pg]

T, J, S, A, REF = "t1", "j1", "scout", 1, "ref-1"
PRICE = SimpleNamespace(input_per_mtok=Decimal("0.10"), output_per_mtok=Decimal("0.32"))
PROFILE = SimpleNamespace(endpoint_alias="pulso-scout-llm", model="m", max_tokens=1000, price=PRICE)


class Inner:
    """Scripted inner gateway; entries are results or exceptions, the last one repeats."""

    def __init__(self, *script: Any, delay: threading.Event | None = None) -> None:
        self.script, self.calls, self.delay = list(script), 0, delay
        self.lock = threading.Lock()

    def generate(self, prompt: Any, inputs: Any, locale: Any, schema: Any = None) -> Any:
        with self.lock:
            item = self.script[min(self.calls, len(self.script) - 1)]
            self.calls += 1
        if self.delay is not None:
            self.delay.wait(5)
        if isinstance(item, Exception):
            raise item
        return item


def result(tin: int = 100, tout: int = 50, cost: str = "0.0003", known: bool = True) -> GenerationResult:
    return GenerationResult(output={}, tokens_in=tin, tokens_out=tout, cost_usd=Decimal(cost), model="m",
                            usage_known=known)


@pytest.fixture
def store(pg: Any) -> ReceiptStore:
    apply_l3(pg.runtime)
    s = ReceiptStore(pg.runtime)
    for ref, tenant in ((REF, T), ("ref-other", "t2")):
        s.save_context(ref, tenant, J, f"k-{ref}", {"budget": {"cost_usd_max": "1"}},
                       datetime.now(UTC) + timedelta(hours=1))
    return s


def gateway(store: ReceiptStore, inner: Any, *, cap: str | None = "1", policy: ModelPolicy | None = None,
            ref: str = REF, tenant: str = T, profile: Any = PROFILE) -> SpendMeteringGateway:
    if cap is not None:
        with psycopg.connect(store._dsn) as conn:
            conn.execute("UPDATE pulso_bridge.invocation_contexts SET context=jsonb_set(context,'{budget}',"
                         "%s::jsonb) WHERE task_binding_ref=%s", (f'{{"cost_usd_max": "{cap}"}}', ref))
    ic = SimpleNamespace(tenant_id=tenant, job_id=J, stage=S, attempt=A)
    contexts = SimpleNamespace(lookup=lambda r: ic)
    return SpendMeteringGateway(inner, contexts, store, current=lambda: ref, policy=policy,
                                profiles=(lambda prompt: profile) if profile is not None else None)


def call(gw: SpendMeteringGateway, inputs: Any = None) -> Any:
    return gw.generate("prompt@1.0.0", inputs or {"q": "x"}, "es", None)


def ledger(store: ReceiptStore, tenant: str = T) -> list[dict[str, Any]]:
    return store.ledger_rows(tenant, J, S, A)


def test_failed_call_with_usage_is_metered_and_reraised(store: ReceiptStore) -> None:  # D1 (the package's first RED)
    err = GatewayError(GatewayErrorKind.invalid_output, tokens_in=100, tokens_out=50, cost_usd=Decimal("0.0003"))
    with pytest.raises(GatewayError) as caught:
        call(gateway(store, Inner(err)))
    assert caught.value.kind is GatewayErrorKind.invalid_output
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and meter["calls"] == 1 and meter["tokens"] == 150
    assert [(r["outcome"], r["tokens_in"], r["tokens_out"]) for r in ledger(store)] == [("invalid_output", 100, 50)]


def test_call_crossing_the_cap_is_counted_not_dropped_and_next_call_is_refused_unsent(store: ReceiptStore) -> None:  # D2
    inner = Inner(result(cost="0.0008"), result(cost="0.0008"), result(cost="0.0008"))
    gw = gateway(store, inner, cap="0.001", profile=None)
    call(gw)
    with pytest.raises(GatewayError) as caught:
        call(gw)  # crosses: 0.0016 > 0.001
    assert caught.value.kind is GatewayErrorKind.refused
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and meter["cost_usd"] == Decimal("0.0016") and meter["calls"] == 2
    with pytest.raises(GatewayError):
        call(gw)  # exhausted: refused before the model is reached
    assert inner.calls == 2
    assert [r["outcome"] for r in ledger(store)] == ["ok", "over_cap", "budget_exhausted"]
    assert ledger(store)[1]["over_cap"] is True


def test_usage_known_false_marks_the_meter_and_keeps_the_reservation(store: ReceiptStore) -> None:  # D4
    gw = gateway(store, Inner(result(0, 0, "0", known=False)))
    call(gw)
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and meter["usage_known"] is False and meter["reserved_usd"] > 0
    assert ledger(store)[0]["usage_known"] is False


def test_successful_known_call_releases_its_reservation(store: ReceiptStore) -> None:  # D3 settle
    call(gateway(store, Inner(result())))
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and meter["reserved_usd"] == 0 and meter["usage_known"] is True


@pytest.mark.parametrize(("kind", "keeps"), [(GatewayErrorKind.timeout, True), (GatewayErrorKind.unavailable, False),
                                             (GatewayErrorKind.rate_limited, False)])
def test_reservation_after_failure_kinds(store: ReceiptStore, kind: GatewayErrorKind, keeps: bool) -> None:
    with pytest.raises(GatewayError):
        call(gateway(store, Inner(GatewayError(kind))))
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and (meter["reserved_usd"] > 0) is keeps
    assert meter["usage_known"] is (not keeps)
    assert [r["outcome"] for r in ledger(store)] == [kind.value]


def test_concurrent_calls_cannot_exceed_the_cap(store: ReceiptStore) -> None:  # D3 atomic reservation
    gate = threading.Event()
    inner = Inner(result(cost="0.0003"), delay=gate)
    from pulso_core_runtime.llm.metering import estimate_reservation
    one = estimate_reservation(PRICE.input_per_mtok, PRICE.output_per_mtok, PROFILE.max_tokens, {"q": "x"})
    cap = one * 2 + one / 2  # fits exactly two reservations
    gw = gateway(store, inner, cap=format(cap, "f"))
    outcomes: list[str] = []

    def worker() -> None:
        try:
            call(gw)
            outcomes.append("ok")
        except GatewayError as exc:
            outcomes.append(exc.kind.value)

    threads = [threading.Thread(target=worker) for _ in range(8)]
    for t in threads:
        t.start()
    for _ in range(250):  # the six refusals return without reaching the model
        if len(outcomes) >= 6:
            break
        threading.Event().wait(0.02)
    gate.set()
    for t in threads:
        t.join(10)
    assert sorted(outcomes) == ["ok", "ok"] + ["refused"] * 6 and inner.calls == 2
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and meter["cost_usd"] + meter["reserved_usd"] <= cap


def test_store_reserve_is_atomic_under_contention(store: ReceiptStore) -> None:
    wins: list[bool] = []
    lock = threading.Lock()

    def reserve() -> None:
        got = store.meter_reserve(T, J, S, A, amount="0.4", cap_usd="1")
        with lock:
            wins.append(got)

    threads = [threading.Thread(target=reserve) for _ in range(16)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(10)
    assert wins.count(True) == 2


def test_ledger_is_scoped_and_stores_no_content(store: ReceiptStore, pg: Any) -> None:  # D5
    canary = "CANARY-PROMPT-CONTENT-0xBEEF"
    gw = gateway(store, Inner(result(), GatewayError(GatewayErrorKind.refused)))
    call(gw, {"q": canary})
    with pytest.raises(GatewayError):
        call(gw, {"q": canary})
    other = gateway(store, Inner(result()), ref="ref-other", tenant="t2")
    call(other)
    assert len(ledger(store)) == 2 and len(ledger(store, "t2")) == 1
    assert store.ledger_rows("t3", J, S, A) == []
    assert all(r["binding_ref"] == REF and r["tenant_id"] == T and r["job_id"] == J for r in ledger(store))
    with psycopg.connect(pg.runtime) as conn:
        cols = {r[0] for r in conn.execute("SELECT column_name FROM information_schema.columns WHERE "
                                           "table_name='model_call_ledger'").fetchall()}
        dump = str(conn.execute("SELECT * FROM pulso_bridge.model_call_ledger").fetchall())
    assert cols and not cols & {"prompt", "inputs", "output", "response", "text", "content", "message"}
    assert canary not in dump


def test_gateway_without_binding_passes_through_unmetered(store: ReceiptStore) -> None:
    gw = SpendMeteringGateway(Inner(result()), SimpleNamespace(), store, current=lambda: None)
    assert gw.generate("p", {}, "es").model == "m"


def test_policy_object_is_accepted(store: ReceiptStore) -> None:
    policy = ModelPolicy({S: StageModelPolicy("pulso-scout-llm", "m", Decimal("0.10"), Decimal("0.32"), 1000)})
    call(gateway(store, Inner(result()), policy=policy))
    assert ledger(store)[0]["outcome"] == "ok"
