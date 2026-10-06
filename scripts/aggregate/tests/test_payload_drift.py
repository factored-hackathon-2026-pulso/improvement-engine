"""EVT3 drift test: the Rust feed writer (`seams/crates/sources/src/payload.rs`) cleans the payload of the platform events with an
allow-list; the Python aggregator (`platform_event_cells.py`) re-checks it. The Python module is the single source of truth: this test
parses the Rust source and fails when a key, an event type, a closed enum or an id prefix differs in either direction."""
import re
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import platform_event_cells as pec  # noqa: E402

PAYLOAD_RS = HERE.parents[2] / "seams" / "crates" / "sources" / "src" / "payload.rs"


def rust_text():
    return PAYLOAD_RS.read_text(encoding="utf-8")


def rust_str_list(body):
    return re.findall(r'"([^"]*)"', body)


def rust_const_list(src, name):
    m = re.search(r"const\s+" + name + r"\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];", src, re.S)
    assert m, f"const {name} not found in payload.rs"
    return rust_str_list(m.group(1))


def rust_payload_keys(src):
    m = re.search(r"pub const PAYLOAD_KEYS[^=]*=\s*&\[(.*?)\n\];", src, re.S)
    assert m, "PAYLOAD_KEYS not found in payload.rs"
    out = {}
    for t, keys in re.findall(r'\(\s*"([^"]+)"\s*,\s*&\[(.*?)\]\s*\)', m.group(1), re.S):
        assert t not in out, f"duplicate event type {t} in payload.rs"
        out[t] = tuple(rust_str_list(keys))
    return out


class PayloadDrift(unittest.TestCase):
    def test_the_rust_source_exists(self):
        self.assertTrue(PAYLOAD_RS.exists(), PAYLOAD_RS)

    def test_event_types_and_keys_match_the_python_payload_keys_exactly(self):
        rs, py = rust_payload_keys(rust_text()), {t: tuple(k) for t, k in pec.PAYLOAD_KEYS.items()}
        self.assertEqual(sorted(rs), sorted(py), "event types differ between payload.rs and PAYLOAD_KEYS")
        for t in py:
            self.assertEqual(sorted(rs[t]), sorted(py[t]), f"allow-listed keys of {t} differ")

    def test_closed_enums_match(self):
        src = rust_text()
        pairs = [("SUBJECTS", pec.SUBJECTS), ("DECISIONS", pec.DECISIONS), ("RESULTS", pec.ASSISTANT_RESULTS), ("CASE_TYPES", pec.CASE_TYPES)]
        for name, py in pairs:
            self.assertEqual(sorted(rust_const_list(src, name)), sorted(py), f"{name} differs")

    def test_id_prefixes_match_the_python_deny_list(self):
        rs = {p.rstrip("-").upper() for p in rust_const_list(rust_text(), "ID_PREFIXES")}
        self.assertTrue(all(p.endswith("-") for p in rust_const_list(rust_text(), "ID_PREFIXES")))
        py = set(re.match(r"\^\(([^)]*)\)-", pec.ID_PREFIX.pattern).group(1).split("|"))
        self.assertEqual(rs, py, "id prefix deny lists differ")

    def test_the_drift_check_sees_a_difference(self):
        """Mutation guard: a key added only on the Rust side, or an enum value dropped, must be detected by the same comparison."""
        src = rust_text().replace('("copilot.tool_used", &["tool"])', '("copilot.tool_used", &["tool", "analyst_id"])')
        self.assertNotEqual(sorted(rust_payload_keys(src)["copilot.tool_used"]), sorted(pec.PAYLOAD_KEYS["copilot.tool_used"]))
        src = rust_text().replace('"released"]', "]", 1)
        self.assertNotEqual(sorted(rust_const_list(src, "RESULTS")), sorted(pec.ASSISTANT_RESULTS))


if __name__ == "__main__":
    unittest.main()
