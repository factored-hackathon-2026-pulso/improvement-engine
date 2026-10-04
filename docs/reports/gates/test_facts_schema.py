"""G0gp: schema test for facts.json. Every SHA must be 40 hex chars or the literal 'unknown'."""
import json
import pathlib
import re
import unittest

HERE = pathlib.Path(__file__).parent
SHA = re.compile(r"^[0-9a-f]{40}$")


def sha_ok(v):
    return v == "unknown" or (isinstance(v, str) and SHA.match(v) is not None)


def validate(f):
    errs = []
    for path in [
        ("improvement_engine", "main_sha"),
        ("agent_core", "pin_sha"),
        ("agent_core", "main_sha"),
        ("llm_gateway", "main_sha"),
        ("llm_gateway", "pin_sha"),
    ]:
        cur = f
        try:
            for k in path:
                cur = cur[k]
        except (KeyError, TypeError):
            errs.append("missing " + ".".join(path))
            continue
        if not sha_ok(cur):
            errs.append("bad sha " + ".".join(path))
    prs = f.get("agent_core", {}).get("prs", {})
    for n in ("23", "24", "28"):
        pr = prs.get(n)
        if not pr:
            errs.append("missing pr " + n)
            continue
        if pr.get("state") not in ("MERGED", "OPEN", "CLOSED", "unknown"):
            errs.append("bad state pr " + n)
        if not sha_ok(pr.get("head_sha")) or not sha_ok(pr.get("merge_sha")):
            errs.append("bad sha pr " + n)
    db = f.get("db_versions", {})
    for k in ("engine_ci", "engine_local_core", "agent_core_compose"):
        if not isinstance(db.get(k), str) or not db[k]:
            errs.append("missing db " + k)
    if not f.get("collected_at"):
        errs.append("missing collected_at")
    return errs


class FactsSchema(unittest.TestCase):
    def test_facts_file_valid(self):
        f = json.loads((HERE / "facts.json").read_text(encoding="utf-8"))
        self.assertEqual(validate(f), [])

    def test_missing_sha_detected(self):
        f = json.loads((HERE / "facts.json").read_text(encoding="utf-8"))
        del f["agent_core"]["pin_sha"]
        self.assertIn("missing agent_core.pin_sha", validate(f))

    def test_malformed_sha_detected(self):
        f = json.loads((HERE / "facts.json").read_text(encoding="utf-8"))
        f["agent_core"]["prs"]["28"]["merge_sha"] = "abc"
        self.assertIn("bad sha pr 28", validate(f))


if __name__ == "__main__":
    unittest.main()
