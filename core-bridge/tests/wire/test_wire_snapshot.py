"""L1a first RED: the wire snapshot of the pinned agent-core must exist, be byte-exact and reproduce golden hashes."""

import hashlib
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
PIN_SHA = "86a767474042a566a0dbd6ed23588959f27ebdb3"
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


def test_193_schemas_and_2_events_byte_copied() -> None:
    files = [e for e in manifest()["files"] if not e.get("derived_by_pulso")]
    schemas = [e for e in files if e["path"].startswith("schemas/")]
    events = [e for e in files if e["path"].startswith("events/")]
    assert len(schemas) == 193
    assert len(events) == 2


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


def test_golden_release_demo_values() -> None:
    vectors = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
    assert vectors["release_id"] == "rel-98130317a1003849"
    assert vectors["release_hash"].startswith("98130317a1003849")
    assert len(vectors["entities"]) == 26
    assert vectors["candidate"]["candidate_hash"]
