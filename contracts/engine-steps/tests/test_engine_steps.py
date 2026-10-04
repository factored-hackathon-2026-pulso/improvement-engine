import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import minischema  # noqa: E402
import digest  # noqa: E402

STEPS = ["sensors", "recompute", "validation", "compile", "gate"]


def load(rel):
    return json.loads((HERE / rel).read_text(encoding="utf-8"))


class StepSchemaTest(unittest.TestCase):
    def test_sample_step_validates(self):
        for step in STEPS:
            for side in ("in", "out"):
                schema = load(f"schemas/{step}.{side}.schema.json")
                sample = load(f"samples/valid-{step}.{side}.json")
                self.assertEqual(minischema.validate(sample, schema), [], (step, side))

    def test_invalid_samples_are_rejected(self):
        bad = sorted((HERE / "samples").glob("invalid-*.json"))
        self.assertGreaterEqual(len(bad), 5)
        for p in bad:
            step, side = p.name.split("-")[1].split(".")[0], p.name.split(".")[-2]
            schema = load(f"schemas/{step}.{side}.schema.json")
            self.assertNotEqual(minischema.validate(json.loads(p.read_text("utf-8")), schema), [], p.name)

    def test_every_schema_declares_contract_version_and_data_class(self):
        for step in STEPS:
            for side in ("in", "out"):
                props = load(f"schemas/{step}.{side}.schema.json")["properties"]
                self.assertEqual(props["contract_version"]["const"], "engine-steps/0")
                self.assertIn("data_class", props)


class PortListTest(unittest.TestCase):
    def test_ports_cover_every_step(self):
        ports = load("ports.json")
        self.assertEqual(ports["contract_version"], "engine-steps/0")
        used = {s for p in ports["ports"] for s in p["steps"]}
        self.assertEqual(used, set(STEPS))
        names = [p["name"] for p in ports["ports"]]
        self.assertEqual(len(names), len(set(names)))
        for p in ports["ports"]:
            self.assertIn(p["direction"], ("driving", "driven"))
            self.assertTrue(p["owner_lane"].startswith(("L-", "X-")))


class MiniSchemaTest(unittest.TestCase):
    def test_validator_bites(self):
        s = {"type": "object", "required": ["a"], "additionalProperties": False,
             "properties": {"a": {"type": "integer", "minimum": 1}}}
        self.assertEqual(minischema.validate({"a": 1}, s), [])
        self.assertTrue(minischema.validate({}, s))
        self.assertTrue(minischema.validate({"a": 0}, s))
        self.assertTrue(minischema.validate({"a": 1, "b": 2}, s))
        self.assertTrue(minischema.validate({"a": True}, s))


class DigestTest(unittest.TestCase):
    def test_digest_is_published_and_matches(self):
        pub = load("DIGEST.json")
        self.assertEqual(pub["digest"], digest.compute()["digest"])
        self.assertTrue(pub["digest"].startswith("sha256:"))
        self.assertIn("schemas/gate.out.schema.json", pub["files"])

    def test_digest_changes_with_content(self):
        a = digest.digest_of({"x": "sha256:1"})
        b = digest.digest_of({"x": "sha256:2"})
        self.assertNotEqual(a, b)


if __name__ == "__main__":
    unittest.main()
