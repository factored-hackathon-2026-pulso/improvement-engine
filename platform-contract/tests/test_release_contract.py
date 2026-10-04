"""PX0 RED: product consumption contract for a published release (alias resolution + release.* events).

Platform label: simulated (product-consumer) until product answers EXT-2. Catalog 1.1.0 is NOT changed."""
import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

import platform_contract as pc

ROOT = Path(__file__).resolve().parents[1]
RC = ROOT / "release-contract"


def _schema(name):
    return json.loads((RC / f"{name}.schema.json").read_text(encoding="utf-8"))


def _examples(kind):
    return sorted((RC / "examples" / kind).glob("*.json"))


@pytest.mark.parametrize("name", ["alias_resolution", "release_event"])
def test_schemas_are_valid_draft_2020_12(name):
    Draft202012Validator.check_schema(_schema(name))


@pytest.mark.parametrize("name", ["alias_resolution", "release_event"])
def test_golden_valid_examples_conform(name):
    files = [f for f in _examples("valid") if f.name.startswith(name)]
    assert files, "no golden valid examples"
    v = Draft202012Validator(_schema(name))
    for f in files:
        assert list(v.iter_errors(json.loads(f.read_text(encoding="utf-8")))) == [], f.name


def test_golden_invalid_examples_are_rejected():
    files = _examples("invalid")
    assert len(files) >= 3
    for f in files:
        name = "alias_resolution" if f.name.startswith("alias_resolution") else "release_event"
        v = Draft202012Validator(_schema(name))
        assert list(v.iter_errors(json.loads(f.read_text(encoding="utf-8")))), f.name


def test_release_payload_schema_matches_python_validator():
    from platform_contract import release_events as rel
    props = _schema("release_event")["properties"]["payload"]
    assert set(props["properties"]) == set(rel.REQUIRED + rel.OPTIONAL)
    assert props["additionalProperties"] is False and set(props["required"]) == set(rel.REQUIRED)


def test_release_event_types_stay_unknown_and_catalog_unchanged():
    assert pc.CONTRACT_VERSION == "1.1.0"
    for t in ("release.published", "release.rolled_back"):
        assert pc.classify_event_type(t) == "unknown"
    assert not any(e["event_type"].startswith("release.") for e in pc.build_event_catalog()["event_types"])


def test_readme_labels_simulated_and_documents_unknown_until_admitted():
    text = (RC / "README.md").read_text(encoding="utf-8")
    assert "simulated" in text and "EXT-2" in text and "unknown" in text and "1.1.0" in text
