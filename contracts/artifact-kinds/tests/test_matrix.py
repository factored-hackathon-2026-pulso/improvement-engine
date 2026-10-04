"""Artifact-kind capability matrix (BK0): coverage, Core drift, honesty and digest tests."""
import json
import os
import re
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import digest  # noqa: E402

VERBS = ["propose", "validate", "evaluate", "publish"]
VERDICTS = {"supported", "denied", "not_exercised", "blocked"}
LEVELS = {"real-core-live", "real-core-dry-run", "stand-in", "none"}
REF = Path(os.environ.get("AGENT_CORE_REF", "D:/.codex/factored/references/agent-core-c814c2b"))


def load(name):
    return json.loads((HERE / name).read_text(encoding="utf-8"))


M = load("matrix.json")
KINDS = {k["kind"]: k for k in M["kinds"]}


class CoverageTest(unittest.TestCase):
    def test_schema_valid(self):
        import jsonschema
        jsonschema.Draft202012Validator(load("matrix.schema.json")).validate(M)

    def test_every_kind_has_a_verb_row_per_verb(self):
        for name, k in KINDS.items():
            self.assertEqual(sorted(k["verbs"]), sorted(VERBS), name)
            for v, row in k["verbs"].items():
                self.assertIn(row["verdict"], VERDICTS, (name, v))
                self.assertIn(row["evidence_level"], LEVELS, (name, v))
                if row["verdict"] == "denied":
                    self.assertTrue(row.get("reason"), (name, v))
                    self.assertIn(row.get("denied_by"), {"bridge", "engine"}, (name, v))
                if row["verdict"] == "blocked":
                    self.assertTrue(row.get("blocked_on"), (name, v))

    def test_matrix_covers_core_entity_kinds_and_release_settings(self):
        self.assertEqual(set(KINDS), set(M["core"]["entity_kinds"]) | set(M["core"]["draft_only_kinds"]))
        self.assertIn("release_settings", KINDS)
        self.assertEqual(len(M["core"]["entity_kinds"]), 11)


class DriftTest(unittest.TestCase):
    def test_matches_pinned_agent_core_checkout(self):
        refs = REF / "agent_core/domain/refs.py"
        if not refs.exists():
            self.skipTest(f"agent-core reference not found at {REF}")
        body = refs.read_text(encoding="utf-8").split("class EntityKind", 1)[1].split("\n\n\n", 1)[0]
        core = re.findall(r'^\s+(\w+) = "\1"', body, re.M)
        self.assertEqual(sorted(core), sorted(M["core"]["entity_kinds"]))
        models = (REF / "agent_core/registry/models.py").read_text(encoding="utf-8")
        self.assertIn(f'RELEASE_SETTINGS = "{M["core"]["release_settings_kind"]}"', models)
        ents = (REF / "agent_core/registry/entities.py").read_text(encoding="utf-8")
        self.assertIn(f'SUITE_KIND = "{M["core"]["draft_only_kinds"][0]}"', ents)


class EvidenceTest(unittest.TestCase):
    def test_evidence_pointers_exist_and_are_required_above_none(self):
        for name, k in KINDS.items():
            for v, row in k["verbs"].items():
                if row["evidence_level"] == "none":
                    self.assertEqual(row.get("evidence", []), [], (name, v))
                    continue
                self.assertTrue(row["evidence"], (name, v))
                for ptr in row["evidence"]:
                    path, _, test = ptr.partition("::")
                    self.assertTrue((ROOT / path).is_file(), ptr)
                    if test:
                        self.assertIn(test.split("[")[0], (ROOT / path).read_text(encoding="utf-8"), ptr)

    def test_only_replace_prompt_and_add_eval_suite_are_live(self):
        live = {n for n, k in KINDS.items() if any(r["evidence_level"] == "real-core-live" for r in k["verbs"].values())}
        self.assertEqual(live, {"prompt", "eval_suite"})
        for n in live:
            self.assertTrue(all(KINDS[n]["verbs"][v]["verdict"] == "supported" for v in VERBS), n)
        self.assertEqual({f["id"] for f in M["change_families"] if f["verdict"] == "supported"},
                         {"replace_prompt", "add_eval_suite"})

    def test_supported_needs_evidence_or_is_a_labelled_stand_in(self):
        for n, k in KINDS.items():
            for v, row in k["verbs"].items():
                if row["verdict"] == "supported":
                    self.assertNotEqual(row["evidence_level"], "none", (n, v))
                if row["verdict"] == "not_exercised":
                    self.assertIn(row["evidence_level"], {"none", "real-core-dry-run"}, (n, v))

    def test_release_settings_denied_by_bridge_everywhere(self):
        for v in VERBS:
            row = KINDS["release_settings"]["verbs"][v]
            self.assertEqual((row["verdict"], row["reason"], row["denied_by"]),
                             ("denied", "release_settings_not_allowed", "bridge"), v)

    def test_denied_kind_list_matches_the_rows(self):
        listed = {d["kind"]: d for d in M["denied_kinds"]}
        for n, k in KINDS.items():
            p = k["verbs"]["propose"]
            if p["verdict"] == "denied":
                self.assertIn(n, listed, n)
                self.assertEqual(listed[n]["denied_by"], p["denied_by"], n)
                self.assertEqual(listed[n]["reason"], p["reason"], n)
        self.assertTrue(set(listed) <= set(KINDS))

    def test_jev_provider_is_blocked_not_supported(self):
        self.assertEqual(KINDS["decision_model"]["provider_notes"]["jev"]["verdict"], "blocked")


class DigestTest(unittest.TestCase):
    def test_digest_json_is_fresh(self):
        pub = load("DIGEST.json")
        self.assertEqual(pub, digest.compute(), "stale DIGEST.json: python digest.py --write")

    def test_digest_is_over_the_canonical_matrix(self):
        self.assertEqual(load("DIGEST.json")["digest"], digest.matrix_digest(M))

    def test_digest_changes_when_a_verdict_changes(self):
        import copy
        m2 = copy.deepcopy(M)
        m2["kinds"][0]["verbs"]["publish"]["verdict"] = "not_exercised" if m2["kinds"][0]["verbs"]["publish"]["verdict"] != "not_exercised" else "denied"
        self.assertNotEqual(digest.matrix_digest(m2), digest.matrix_digest(M))


if __name__ == "__main__":
    unittest.main()
