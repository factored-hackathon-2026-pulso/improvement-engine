"""merge_keys.py: union, conflicts, requirements, and the rule that no key value is ever printed."""
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import merge_keys as mk  # noqa: E402

ENGINE = "pulso-engine-dev-1"
K = {n: "KEYVALUE-" + n * 12 for n in "abcdefg"}


def write(d: Path, name: str, doc: dict) -> None:
    (d / name).write_text(json.dumps(doc), encoding="utf-8")


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.state, self.plat = Path(self.tmp.name, "state"), Path(self.tmp.name, "plat")
        self.state.mkdir()
        self.plat.mkdir()
        write(self.state, "identity-keys.json", {"principal_keys": {"test-cust": K["a"], "test-staff": K["b"], ENGINE: K["c"]},
                                                 "delegation_keys": {"test-deleg": K["d"]}})
        write(self.state, "staff-keys.json", {"principal_keys": {"test-staff": K["b"], ENGINE: K["c"]}})
        write(self.plat, "identity-keys.json", {"principal_keys": {"cc-principal-env1": K["e"]}, "delegation_keys": {"cc-grant-env1": K["f"]}})
        write(self.plat, "staff-keys.json", {"principal_keys": {"cc-staff-env1": K["g"]}})


class MergeTests(Fixture):
    def test_union_holds_both_worlds(self):
        kids = mk.merge(self.state, self.plat, ENGINE)
        ident = json.loads((self.state / "identity-keys.json").read_text())
        staff = json.loads((self.state / "staff-keys.json").read_text())
        self.assertEqual(set(ident["principal_keys"]), {"test-cust", "test-staff", ENGINE, "cc-principal-env1"})
        self.assertEqual(set(ident["delegation_keys"]), {"test-deleg", "cc-grant-env1"})
        self.assertEqual(set(staff["principal_keys"]), {"test-staff", ENGINE, "cc-staff-env1"})
        self.assertIn("cc-staff-env1", kids["staff-keys.json"])

    def test_idempotent(self):
        mk.merge(self.state, self.plat, ENGINE)
        first = (self.state / "staff-keys.json").read_text()
        mk.merge(self.state, self.plat, ENGINE)
        self.assertEqual(first, (self.state / "staff-keys.json").read_text())

    def test_same_kid_other_key_is_refused_and_files_untouched(self):
        write(self.plat, "staff-keys.json", {"principal_keys": {ENGINE: K["g"]}})
        before = (self.state / "staff-keys.json").read_text()
        with self.assertRaises(mk.MergeError) as ctx:
            mk.merge(self.state, self.plat, ENGINE)
        self.assertIn(ENGINE, str(ctx.exception))
        self.assertNotIn("KEYVALUE", str(ctx.exception))
        self.assertEqual(before, (self.state / "staff-keys.json").read_text())

    def test_engine_kid_required(self):
        write(self.state, "staff-keys.json", {"principal_keys": {"test-staff": K["b"]}})
        with self.assertRaises(mk.MergeError):
            mk.merge(self.state, self.plat, ENGINE)

    def test_dev_admin_kid_required(self):
        write(self.state, "staff-keys.json", {"principal_keys": {ENGINE: K["c"]}})
        with self.assertRaises(mk.MergeError):
            mk.merge(self.state, self.plat, ENGINE)

    def test_missing_file_names_the_file_only(self):
        (self.plat / "staff-keys.json").unlink()
        with self.assertRaises(mk.MergeError) as ctx:
            mk.merge(self.state, self.plat, ENGINE)
        self.assertIn("staff-keys.json", str(ctx.exception))

    def test_malformed_entry_refused(self):
        write(self.plat, "staff-keys.json", {"principal_keys": {"cc-staff-env1": 5}})
        with self.assertRaises(mk.MergeError):
            mk.merge(self.state, self.plat, ENGINE)


class CanaryTests(Fixture):
    def test_cli_never_prints_a_key_value(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = mk.main(["--state-dir", str(self.state), "--platform-keys", str(self.plat)])
        self.assertEqual(rc, 0)
        write(self.plat, "staff-keys.json", {"principal_keys": {ENGINE: K["g"]}})
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            self.assertEqual(mk.main(["--state-dir", str(self.state), "--platform-keys", str(self.plat)]), 2)
        shown = out.getvalue() + err.getvalue()
        self.assertIn("kids", shown)
        for v in K.values():
            self.assertNotIn(v, shown)


if __name__ == "__main__":
    unittest.main()
