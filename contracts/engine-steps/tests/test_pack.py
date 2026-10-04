import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
PACK = HERE / "pack"
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(PACK))
import minischema  # noqa: E402
import verify_pack  # noqa: E402

PARTS = ["c7_claim_next", "c8_schemas", "c9_transcript", "c10_gate_result",
         "builder_corpus", "arm_report_goldens", "bridge_goldens"]


def j(p):
    return json.loads(Path(p).read_text(encoding="utf-8"))


class PackTest(unittest.TestCase):
    def test_pack_verifies_clean(self):
        self.assertEqual(verify_pack.verify(PACK), [])

    def test_manifest_has_seven_named_parts_and_one_digest(self):
        m = j(PACK / "manifest.json")
        self.assertEqual([p["id"] for p in m["parts"]], PARTS)
        self.assertRegex(m["pack_digest"], r"^sha256:[0-9a-f]{64}$")
        self.assertEqual(m["pack_digest"], verify_pack.pack_digest(m["parts"]))

    def test_missing_golden_fails_digest_test(self):
        for victim in ("parts/arm_report_goldens/arms.json",
                       "parts/bridge_goldens/authoring.json",
                       "parts/bridge_goldens/writer_evaluation.json"):
            with tempfile.TemporaryDirectory() as t:
                tmp = Path(t) / "pack"
                shutil.copytree(PACK, tmp)
                (tmp / victim).unlink()
                errs = verify_pack.verify(tmp)
                self.assertTrue(any("missing" in e and victim in e for e in errs), (victim, errs))

    def test_tampered_golden_fails(self):
        with tempfile.TemporaryDirectory() as t:
            tmp = Path(t) / "pack"
            shutil.copytree(PACK, tmp)
            f = tmp / "parts/bridge_goldens/authoring.json"
            f.write_text(f.read_text(encoding="utf-8") + " ", encoding="utf-8")
            self.assertTrue(any("digest mismatch" in e for e in verify_pack.verify(tmp)))

    def test_c8_samples(self):
        d = PACK / "parts/c8_schemas"
        for name in ("ChangeSpec", "ScenarioCase"):
            schema = j(d / f"{name}.schema.json")
            self.assertEqual(minischema.validate(j(d / f"valid-{name}.json"), schema), [])
            self.assertNotEqual(minischema.validate(j(d / f"invalid-{name}.json"), schema), [])

    def test_c10_gate_result_samples(self):
        d = PACK / "parts/c10_gate_result"
        schema = j(d / "gate_result.schema.json")
        self.assertEqual(minischema.validate(j(d / "valid-pass.json"), schema), [])
        self.assertNotEqual(minischema.validate(j(d / "invalid-pass-with-failed-gate.json"), schema), [])

    def test_corpus_has_ten_outputs_with_expected_verdicts(self):
        d = PACK / "parts/builder_corpus"
        schema = j(d / "builder_output.schema.json")
        verdicts = j(d / "verdicts.json")["verdicts"]
        outs = sorted((d / "outputs").glob("*.json"))
        self.assertEqual(len(outs), 10)
        self.assertEqual({p.stem for p in outs}, set(verdicts))
        self.assertTrue({"valid", "unlinked", "not_evaluable", "invalid"} <= set(verdicts.values()))
        for p in outs:
            self.assertEqual(minischema.validate(j(p), schema), [], p.name)
            self.assertEqual(j(p)["data_class"], "synthetic")

    def test_transcript_seed_covers_ten_steps(self):
        t = j(PACK / "parts/c9_transcript/transcript.seed.json")
        self.assertEqual([e["step"] for e in t["events"]], list(range(1, 11)))
        for e in t["events"]:
            self.assertIn("status", e)
            self.assertIn("data_class", e)

    def test_claim_next_signature_and_traces(self):
        d = PACK / "parts/c7_claim_next"
        sig = j(d / "claim_next.v1_1.json")
        self.assertEqual(sig["trait"], "DurableJobRepository")
        self.assertEqual(sig["version"], "v1.1")
        self.assertIn("fn claim_next_job", sig["rust_signature"])
        traces = sorted((d / "traces").glob("*.json"))
        self.assertGreaterEqual(len(traces), 4)
        for p in traces:
            tr = j(p)
            self.assertIn("derived_from_test", tr)
            self.assertTrue(tr["steps"])

    def test_bridge_goldens_match_source_when_present(self):
        src = HERE.parents[1] / "bridge-contract/examples/flows"
        if not src.exists():
            self.skipTest("bridge-contract not present")
        for n in ("authoring.json", "writer_evaluation.json", "arms.json"):
            part = "bridge_goldens" if n != "arms.json" else "arm_report_goldens"
            self.assertEqual((PACK / "parts" / part / n).read_bytes().replace(b"\r\n", b"\n"),
                             (src / n).read_bytes().replace(b"\r\n", b"\n"), n)


if __name__ == "__main__":
    unittest.main()
