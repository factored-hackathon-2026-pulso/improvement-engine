"""P2py first RED: release events, fast-forward clock, effect realism with author separation."""

import inspect
import json

import pytest

from platform_contract import release_events as rel
from platform_live import effects as fx
from platform_live import PlatformLiveSim


def _types(sim):
    return [r[0] for r in sim.conn.execute("select event_type from event_log order by sequence")]


def test_publish_emits_release_event():
    sim = PlatformLiveSim(seed=1)
    sim.publish_release("agent-a", "staging", "rel-0001")
    assert "release.published" in _types(sim)
    row = sim.conn.execute("select entity, entity_id, payload from event_log where event_type='release.published'").fetchone()
    assert row[0] == "release" and row[1] == "rel-0001"
    assert rel.validate_release_payload("release.published", json.loads(row[2])) == []


def test_release_payload_carries_no_effect_or_mechanism_fields():
    bad = {"release_id": "r", "agent_id": "a", "alias": "prod", "effect_delta_pct": -10}
    assert rel.validate_release_payload("release.published", bad)
    assert rel.validate_release_payload("release.published", {"release_id": "r", "agent_id": "a", "alias": "prod", "mechanism_id": "m"})


def test_release_types_are_unadmitted_in_catalog_1_1_0():
    import platform_contract as pc
    assert pc.classify_event_type("release.published") == "unknown"
    assert set(rel.RELEASE_EVENT_TYPES) == {"release.published", "release.rolled_back"}


def test_fast_forward_is_monotonic_and_runs_scheduled_jobs_in_order():
    sim = PlatformLiveSim(seed=1)
    start = sim.now
    seen = []
    sim.schedule(7200, lambda: seen.append("b"))
    sim.schedule(3600, lambda: seen.append("a"))
    sim.fast_forward(days=1)
    assert seen == ["a", "b"]
    assert (sim.now - start).total_seconds() == 86400
    with pytest.raises(ValueError):
        sim.fast_forward(seconds=-1)


def test_effect_series_labelled_simulated_and_follows_spec():
    spec = fx.EffectSpec(effect_id="e1", author="effects-author", metric="first_response_seconds",
                         baseline=300.0, delta_pct=-20.0, ramp_days=2, noise_sd=0.0, seed=5)
    s = fx.simulate_effect_series(spec, days=6, release_day=2)
    assert all(p["data_class"] == "simulated" for p in s)
    assert s[0]["value"] == 300.0 and s[1]["value"] == 300.0
    assert s[-1]["value"] == pytest.approx(240.0)


def test_author_separation_rejects_same_author_and_derivation():
    m = fx.PlantedMechanism(mechanism_id="m1", author="detector-author", kind="h1_starvation", params={"magnitude": 0.5})
    ok = fx.EffectSpec(effect_id="e", author="effects-author", metric="x", baseline=1.0, delta_pct=-5.0)
    fx.assert_author_separation(ok, m)
    with pytest.raises(fx.AuthorSeparationError):
        fx.assert_author_separation(fx.EffectSpec(effect_id="e", author="detector-author", metric="x", baseline=1.0, delta_pct=-5.0), m)
    with pytest.raises(fx.AuthorSeparationError):
        fx.assert_author_separation(fx.EffectSpec(effect_id="e", author="effects-author", metric="x", baseline=1.0,
                                                  delta_pct=-5.0, derived_from=("m1",)), m)


def test_effect_series_independent_of_planted_mechanism():
    params = list(inspect.signature(fx.simulate_effect_series).parameters)
    assert params == ["spec", "days", "release_day"]
    spec = fx.EffectSpec(effect_id="e1", author="effects-author", metric="m", baseline=100.0, delta_pct=10.0, noise_sd=3.0, seed=9)
    a = fx.simulate_effect_series(spec, days=10, release_day=3)
    # changing any planted mechanism must not move the series: the function cannot see it
    fx.PlantedMechanism(mechanism_id="m", author="d", kind="k", params={"magnitude": 99})
    assert a == fx.simulate_effect_series(spec, days=10, release_day=3)


def test_effects_module_does_not_reference_mechanism_internals():
    src = inspect.getsource(fx.simulate_effect_series) + inspect.getsource(fx.EffectSpec)
    assert "PlantedMechanism" not in src and "mechanism" not in src.replace("derived_from", "")


def test_release_with_effect_records_series_after_fast_forward_and_requires_separation():
    sim = PlatformLiveSim(seed=1)
    m = fx.PlantedMechanism(mechanism_id="m1", author="detector-author", kind="h1_starvation", params={})
    spec = fx.EffectSpec(effect_id="e1", author="effects-author", metric="first_response_seconds", baseline=300.0, delta_pct=-20.0)
    sim.publish_release("agent-a", "prod", "rel-0002", effect=spec, mechanism=m)
    sim.fast_forward(days=3)
    series = sim.effect_series("rel-0002")
    assert len(series) == 3 and series[0]["data_class"] == "simulated"
    bad = fx.EffectSpec(effect_id="e2", author="detector-author", metric="x", baseline=1.0, delta_pct=1.0)
    with pytest.raises(fx.AuthorSeparationError):
        sim.publish_release("agent-a", "prod", "rel-0003", effect=bad, mechanism=m)


def test_observation_labels_memory_and_successor_not_exercised():
    sim = PlatformLiveSim(seed=1)
    assert sim.observation_labels() == {"release_event": "simulated", "effect": "simulated",
                                        "memory": "not_exercised", "successor": "not_exercised"}


def test_schedule_after_partial_fast_forward_does_not_crash_or_lose_jobs():
    sim = PlatformLiveSim(seed=1)
    seen = []
    sim.schedule(100, lambda: seen.append("a"))
    sim.schedule(100000, lambda: seen.append("late"))
    sim.fast_forward(seconds=200)
    sim.schedule(10, lambda: seen.append("b"))
    sim.schedule(10, lambda: seen.append("c"))  # same due time, ids must not collide
    sim.fast_forward(days=2)
    assert seen == ["a", "b", "c", "late"]


def test_job_scheduled_inside_job_runs_within_window():
    sim = PlatformLiveSim(seed=1)
    seen = []
    sim.schedule(10, lambda: sim.schedule(10, lambda: seen.append("child")))
    sim.fast_forward(seconds=100)
    assert seen == ["child"]


def test_publish_rejects_non_release_event_type():
    sim = PlatformLiveSim(seed=1)
    with pytest.raises(ValueError):
        sim.publish_release("a", "prod", "rel-9", event_type="case.created")
