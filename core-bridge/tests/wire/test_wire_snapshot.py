"""L1a first RED: the wire snapshot of the pinned agent-core must exist, be byte-exact and reproduce golden hashes."""

import hashlib
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
PIN_SHA = "c814c2bad9f154d10c092326558815dca9562be7"
WIRE = ROOT / "wire" / f"agent_core@{PIN_SHA[:7]}"
pytestmark = pytest.mark.wire


def manifest() -> dict:
    return json.loads((WIRE / "MANIFEST.json").read_text(encoding="utf-8"))


def test_manifest_pins_sha_and_contract_version() -> None:
    m = manifest()
    assert m["sha"] == PIN_SHA
    assert m["contract_version"] == "1.3.0"
    assert m["repo"]
    assert m["tool_versions"]["python"].startswith("3.12")


def test_every_manifest_file_exists_and_hashes() -> None:
    m = manifest()
    assert m["files"], "empty manifest"
    for entry in m["files"]:
        data = (WIRE / entry["path"]).read_bytes()
        assert hashlib.sha256(data).hexdigest() == entry["sha256"], entry["path"]


def test_no_unlisted_files() -> None:
    listed = {e["path"] for e in manifest()["files"]} | {"MANIFEST.json"}
    on_disk = {p.relative_to(WIRE).as_posix() for p in WIRE.rglob("*") if p.is_file()}
    assert on_disk == listed


def test_194_schemas_2_events_and_31_registry_schemas_byte_copied() -> None:
    files = [e for e in manifest()["files"] if not e.get("derived_by_pulso")]
    schemas = [e for e in files if e["path"].startswith("schemas/")]
    events = [e for e in files if e["path"].startswith("events/")]
    registry = [e for e in files if e["path"].startswith("registry/")]
    assert len(schemas) == 194  # +RunSummary (N-08)
    assert len(events) == 2
    assert len(registry) == 31  # N-01: contracts/registry/ published upstream
    assert "schemas/RunSummary.json" in {e["path"] for e in schemas}
    assert {"registry/AliasState.json", "registry/VersionSummary.json", "registry/ReleaseSettings.json",
            "registry/CreateProposalBody.json", "registry/PutDraftBody.json"} <= {e["path"] for e in registry}


def test_release_detail_carries_the_n03_fields() -> None:
    """N-03: both the upstream (serialization) and the derived (validation) ReleaseDetail have the four new fields."""
    for rel in ("registry/ReleaseDetail.json", "derived/ReleaseDetail.schema.json"):
        props = json.loads((WIRE / rel).read_text(encoding="utf-8"))["properties"]
        assert {"interrupts", "language_detection", "injection_ruleset", "max_input_chars"} <= set(props), rel


def test_derived_schemas_are_flagged_and_complete() -> None:
    derived = {Path(e["path"]).stem for e in manifest()["files"] if e.get("derived_by_pulso")}
    expected = {"EvalSuite", "EntityDraft", "VersionDocs", "Proposal", "ValidationReport", "CandidateView",
                "ReleaseDetail", "EntityVersion", "ReleaseDiff", "EvalReport", "ProposalDetail",
                "WriteRecord", "EvalRun", "registry_openapi"}
    assert expected <= {d.removesuffix(".schema") for d in derived}


def test_golden_hash_of_disputa_cargo_matches_v3_cap12() -> None:
    vectors = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
    entity = next(v for v in vectors["entities"] if v["ref"] == "flow:disputa-cargo@1.0.0")
    assert entity["content_hash"].startswith("6b5b579464f54370")
    # the canonical bytes hash to the content hash (independent re-derivation, no agent_core needed)
    assert hashlib.sha256(bytes.fromhex(entity["canonical_bytes_hex"])).hexdigest() == entity["content_hash"]


def test_golden_vectors_carry_the_seeded_release_detail_n03() -> None:
    v = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
    d = v["release_detail"]
    assert d["release_id"] == v["release_id"] == "rel-e26df0070f6be82f"
    assert d["max_input_chars"] == 4000 and d["language_detection"]["id"] == "lang-es-pt"
    assert d["injection_ruleset"]["id"] == "injection-rules"
    assert [i["id"] for i in d["interrupts"]] == ["fraude"]
    assert len(d["entities"]) == len(v["entities"])


def test_golden_release_demo_values() -> None:
    vectors = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
    assert vectors["release_id"] == "rel-e26df0070f6be82f"
    assert vectors["release_hash"].startswith("e26df0070f6be82f")
    assert len(vectors["entities"]) == 26
    assert vectors["candidate"]["candidate_hash"]


def test_every_entity_vector_rederives_independently() -> None:
    """All 26 vectors: sha256(canonical bytes) == content_hash, and the canonical bytes equal an independent
    sorted-key/compact/UTF-8 serialisation of the normalised dump (no agent_core involved)."""
    vectors = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
    refs = set()
    for v in vectors["entities"]:
        raw = bytes.fromhex(v["canonical_bytes_hex"])
        assert hashlib.sha256(raw).hexdigest() == v["content_hash"], v["ref"]
        independent = json.dumps(v["normalized_dump_json"], sort_keys=True, separators=(",", ":"),
                                 ensure_ascii=False).encode("utf-8")
        assert independent == raw, v["ref"]
        refs.add(v["ref"])
    assert len(refs) == len(vectors["entities"]) == 26


def test_candidate_vector_is_complete() -> None:
    cand = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))["candidate"]
    assert len(cand["candidate_hash"]) == 64 and int(cand["candidate_hash"], 16) >= 0


UPSTREAM_NAME = {"Create": "CreateProposalBody", "Draft": "PutDraftBody", "Evaluate": "EvaluateBody",
                 "Approve": "ApproveBody", "Promote": "PromoteBody", "Reason": "ReasonBody"}


def _schema(rel: str) -> dict:
    doc = json.loads((WIRE / rel).read_text(encoding="utf-8"))
    doc.pop("$comment", None)
    return doc


def test_derived_schemas_published_upstream_equal_the_upstream_ones() -> None:
    """N-01: upstream publishes input models (validation) and output models (serialization). A derived schema that has
    an upstream twin must BE that twin (modulo the generator `$comment`): a derived validation-mode output schema is
    looser (a Decimal metric validates as a number too) than what Core really serves."""
    compared = 0
    for path in sorted((WIRE / "derived").glob("*.schema.json")):
        name = path.name.removesuffix(".schema.json")
        twin = WIRE / "registry" / f"{UPSTREAM_NAME.get(name, name)}.json"
        if twin.exists():
            assert _schema(f"derived/{path.name}") == _schema(f"registry/{twin.name}"), name
            compared += 1
    assert compared >= 15
