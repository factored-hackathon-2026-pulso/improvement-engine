"""PX0 RED: the product platform consumes a published release.

registry alias resolution (real wire shape of GET /v1/registry/aliases/{agent}/{alias}) -> release.published
event in event_log -> observation. Platform is simulated (product-consumer); catalog 1.1.0 keeps the type unknown."""
import json

import pytest
from fastapi.testclient import TestClient
from jsonschema import Draft202012Validator

from platform_live import PlatformLiveSim
from registry_mock import jws
from registry_mock.app import create_app
from registry_mock.sim_common import AGENT_ID, BASE_RELEASE_ID

from pathlib import Path

RC = Path(__file__).resolve().parents[3] / "platform-contract" / "release-contract"


@pytest.fixture()
def resolver():
    client = TestClient(create_app())
    hdr = {"Authorization": f"Bearer {jws.issue('bot')}"}

    def resolve(agent_id, alias):
        r = client.get(f"/v1/registry/aliases/{agent_id}/{alias}", headers=hdr)
        return r.status_code, r.json()
    return resolve


def test_resolved_alias_matches_contract_schema(resolver):
    code, body = resolver(AGENT_ID, "prod")
    assert code == 200 and body["release_id"] == BASE_RELEASE_ID
    schema = json.loads((RC / "alias_resolution.schema.json").read_text(encoding="utf-8"))
    assert list(Draft202012Validator(schema).iter_errors(body)) == []


def test_alias_to_release_event_to_observation(resolver):
    sim = PlatformLiveSim(seed=3)
    obs = sim.consume_release(resolver, AGENT_ID, "prod")
    row = sim.conn.execute("select event_type, entity_id, payload from event_log "
                           "where event_type='release.published'").fetchone()
    assert row[1] == BASE_RELEASE_ID and json.loads(row[2])["alias"] == "prod"
    schema = json.loads((RC / "release_event.schema.json").read_text(encoding="utf-8"))
    ev = {"event_type": row[0], "entity": "release", "entity_id": row[1], "payload": json.loads(row[2])}
    assert list(Draft202012Validator(schema).iter_errors(ev)) == []
    assert obs["release_id"] == BASE_RELEASE_ID and obs["event_type"] == "release.published"
    assert obs["catalog_class"] == "unknown"  # catalog 1.1.0, not admitted by the engine yet
    assert obs["labels"]["platform"] == "simulated(product-consumer)"


def test_unresolvable_alias_emits_nothing(resolver):
    sim = PlatformLiveSim(seed=3)
    before = sim.conn.execute("select count(*) from event_log").fetchone()[0]
    with pytest.raises(LookupError):
        sim.consume_release(resolver, AGENT_ID, "no-such-alias")
    assert sim.conn.execute("select count(*) from event_log").fetchone()[0] == before
