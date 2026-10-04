import json

import pytest

from product_stream.generator import ProductStream
from product_stream.scenarios import SCENARIO_NAMES

N = 12000
CELL = ("pt", "web_chat")


def run(name, seed=11, n=N, **kw):
    g = ProductStream(seed=seed, scenario=name, horizon_events=n, **kw)
    while g.last_sequence < n:
        g.next_batch(500)
    return g


def rate(g, key, lo, hi, cell=None):
    ev = [s for s, c in g.stats[key] if lo < s <= hi and (cell is None or c == cell)]
    base = [s for s, c in g.stats["cases"] if lo < s <= hi and (cell is None or c == cell)]
    return len(ev) / max(1, len(base))


def test_scenario_names():
    assert {"null", "escalation_rise", "recurrence_rise", "volume_drift"} <= set(SCENARIO_NAMES)


def test_unknown_scenario_rejected():
    with pytest.raises(ValueError):
        ProductStream(scenario="nope")


def test_manifest_is_machine_readable_and_labelled():
    g = run("escalation_rise")
    m = g.manifest()
    json.dumps(m)
    assert m["data_origin"] == "synthetic product-sim"
    assert m["scenario"] == "escalation_rise" and m["seed"] == 11 and m["horizon_events"] == N
    assert m["planted"]["effect"] == "reassignment_rate_rise"
    assert m["planted"]["cell"] == {"language": "pt", "channel": "web_chat"}
    assert 0 < m["planted"]["onset_sequence"] < m["windows"]["discovery"][1] < m["windows"]["holdout"][1]
    assert m["windows"]["discovery"][1] == m["windows"]["holdout"][0]
    assert "realised" in m and m["realised"]["last_sequence"] >= N


def test_null_manifest_plants_nothing():
    m = run("null").manifest()
    assert m["planted"] is None


def test_escalation_rise_present_in_discovery_and_holdout_for_target_cell_only():
    g = run("escalation_rise")
    m = g.manifest()
    on = m["planted"]["onset_sequence"]
    d_hi, h_hi = m["windows"]["discovery"][1], m["windows"]["holdout"][1]
    assert rate(g, "reassigns", on, d_hi, CELL) > 2.5 * rate(g, "reassigns", 0, on, CELL) - 0.001
    assert rate(g, "reassigns", d_hi, h_hi, CELL) > 2.5 * rate(g, "reassigns", 0, on, CELL) - 0.001
    other = ("es", "app_chat")
    assert rate(g, "reassigns", d_hi, h_hi, other) < 0.12


def test_null_has_no_effect_in_any_window():
    g = run("null")
    m = g.manifest()
    d_hi, h_hi = m["windows"]["discovery"][1], m["windows"]["holdout"][1]
    for lo, hi in ((0, d_hi), (d_hi, h_hi)):
        assert rate(g, "reassigns", lo, hi, CELL) < 0.12
        assert rate(g, "reopens", lo, hi, CELL) < 0.12


def test_recurrence_rise_raises_reopens():
    g = run("recurrence_rise")
    m = g.manifest()
    on, d_hi, h_hi = m["planted"]["onset_sequence"], m["windows"]["discovery"][1], m["windows"]["holdout"][1]
    assert rate(g, "reopens", d_hi, h_hi, CELL) > 2.5 * rate(g, "reopens", 0, on, CELL) - 0.001


def test_volume_drift_raises_events_per_sim_hour():
    g = run("volume_drift", n=8000)
    ev = g.event_times
    third = len(ev) // 3
    from datetime import datetime
    t = lambda s: datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()
    early = third / max(1, t(ev[third - 1]) - t(ev[0]))
    late = third / max(1, t(ev[-1]) - t(ev[-third]))
    assert late > 1.8 * early
    assert g.manifest()["planted"]["effect"] == "volume_drift"


def test_manifest_reports_realised_target_cell_rates_not_only_nominal():
    from product_stream.generator import ProductStream
    g = ProductStream(seed=1, scenario="escalation_rise", horizon_events=6000)
    while g.last_sequence < 6000:
        g.next_batch(500)
    r = g.manifest()["realised"]["target_cell"]
    assert set(r) == {"pre_onset", "post_onset"}
    post = r["post_onset"]
    assert post["cases"] > 0 and abs(post["reassign_rate"] - post["reassigns"] / post["cases"]) < 1e-9
    assert post["reassign_rate"] > r["pre_onset"]["reassign_rate"]
