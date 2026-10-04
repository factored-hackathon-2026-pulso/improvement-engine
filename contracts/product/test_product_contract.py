"""FX22p: fixtures of the Product consumer contract (alias resolution + release.published goldens).

Platform label: simulated (product-consumer) until EXT-2. The schemas and the Python twin live in
platform-contract/release-contract; these goldens are what platform-sim consumes. Catalog 1.1.0 is unchanged:
release.* classify as unknown (quarantined by the ingest, batch still ACKed)."""
import json
import sys
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
RC = ROOT / "platform-contract" / "release-contract"
for p in (ROOT / "platform-contract", ROOT / "platform-sim"):
    sys.path.insert(0, str(p))

GOLDENS = {"alias_resolution": "alias_resolution.prod.json",
           "release_event": "release_event.published.json"}


def _load(name):
    return json.loads((HERE / name).read_text(encoding="utf-8"))


def _schema(kind):
    return json.loads((RC / f"{kind}.schema.json").read_text(encoding="utf-8"))


@pytest.mark.parametrize("kind", GOLDENS)
def test_product_goldens_conform_to_the_release_contract_schemas(kind):
    assert list(Draft202012Validator(_schema(kind)).iter_errors(_load(GOLDENS[kind]))) == []


def test_rolled_back_golden_conforms_and_goldens_match_the_release_contract_examples():
    assert list(Draft202012Validator(_schema("release_event")).iter_errors(_load("release_event.rolled_back.json"))) == []
    for name in (*GOLDENS.values(), "release_event.rolled_back.json"):
        assert _load(name) == json.loads((RC / "examples" / "valid" / name).read_text(encoding="utf-8")), name


def test_goldens_are_synthetic_and_carry_identity_only():
    ev = _load("release_event.published.json")
    assert set(ev["payload"]) <= {"release_id", "agent_id", "alias", "previous_release_id"}
    assert ev["entity_id"] == ev["payload"]["release_id"]


def test_platform_sim_consumes_the_alias_golden_and_emits_the_release_event_golden(tmp_path):
    from platform_live import PlatformLiveSim

    alias, event = _load("alias_resolution.prod.json"), _load("release_event.published.json")
    sim = PlatformLiveSim(seed=1, path=str(tmp_path / "sim.db"))
    try:
        obs = sim.consume_release(lambda a, al: (200, alias), alias["agent_id"], alias["alias"])
        assert obs["event_type"] == event["event_type"] and obs["release_id"] == alias["release_id"]
        assert obs["catalog_class"] == "unknown"  # catalog 1.1.0 is not changed by this fixture set
        row = sim.conn.execute("SELECT event_type, entity, entity_id, payload FROM event_log WHERE sequence=?",
                               (obs["source_sequence"],)).fetchone()
        payload = json.loads(row[3])
        assert (row[0], row[1]) == (event["event_type"], event["entity"])
        assert {k: payload[k] for k in ("agent_id", "alias")} == {k: event["payload"][k] for k in ("agent_id", "alias")}
        assert payload["release_id"] == row[2] == alias["release_id"]
    finally:
        sim.conn.close()
