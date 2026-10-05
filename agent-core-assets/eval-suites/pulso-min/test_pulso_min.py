"""Offline structural checks of the pulso-min suites (no agent-core needed). Run from the repo root:
    uv run --with pyyaml python -m unittest agent-core-assets/eval-suites/pulso-min/test_pulso_min.py
"""
import re
import unittest
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None

HERE = Path(__file__).resolve().parent
SUITES = {"disputas": HERE / "disputas" / "disputas-min@1.0.0.yaml",
          "consultas": HERE / "consultas" / "consultas-min@1.0.0.yaml"}
BEHAVIOURS = {"fraude": ("fraude", "fraud"), "inyeccion": ("inyec", "injec"),
              "monto": ("monto", "valor-alto"), "pii": ("datos-personales", "dados-pessoais"),
              "reembolso": ("reembolso",)}


@unittest.skipIf(yaml is None, "PyYAML missing")
class PulsoMin(unittest.TestCase):
    def load(self, name):
        return yaml.safe_load(SUITES[name].read_text(encoding="utf-8"))

    def test_shape_and_sizes(self):
        for name, lo, hi in (("disputas", 15, 20), ("consultas", 10, 12)):
            s = self.load(name)
            self.assertEqual(s["agent_id"], name)
            ids = [c["id"] for c in s["scenarios"]]
            self.assertEqual(len(ids), len(set(ids)))
            self.assertTrue(lo <= len(ids) <= hi, (name, len(ids)))
            self.assertIn("thresholds", s)

    def test_no_pii_shaped_text(self):
        for path in SUITES.values():
            text = path.read_text(encoding="utf-8")
            self.assertIsNone(re.search(r"\d{6,}", text), path.name)
            self.assertIsNone(re.search(r"[\w.]+@[\w.]+\.\w+", text), path.name)

    def test_both_languages_per_behaviour(self):
        for name, behaviours in (("disputas", BEHAVIOURS), ("consultas", {k: v for k, v in BEHAVIOURS.items() if k in ("fraude", "inyeccion")})):
            ids = [c["id"] for c in self.load(name)["scenarios"]]
            for b, keys in behaviours.items():
                for lang in ("es-", "pt-"):
                    self.assertTrue(any(i.startswith(lang) and any(k in i for k in keys) for i in ids), (name, b, lang))

    def test_readme_names_every_case(self):
        readme = (HERE / "README.md").read_text(encoding="utf-8")
        for name in SUITES:
            for c in self.load(name)["scenarios"]:
                self.assertIn(c["id"], readme)

    def test_pii_cases_have_canaries(self):
        for c in self.load("disputas")["scenarios"]:
            if "datos-personales" in c["id"] or "dados-pessoais" in c["id"]:
                self.assertTrue(c["sensitive_values"])


if __name__ == "__main__":
    unittest.main()
