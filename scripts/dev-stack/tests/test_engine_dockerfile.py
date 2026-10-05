"""Structural contract of the engine image (root Dockerfile + .dockerignore). No build, no network."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DOCKERFILE = (ROOT / "Dockerfile").read_text(encoding="utf-8")
IGNORE = (ROOT / ".dockerignore").read_text(encoding="utf-8").splitlines()


def logical_lines(text):
    out, cur = [], ""
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.endswith("\\"):
            cur += line[:-1] + " "
            continue
        out.append((cur + line).strip())
        cur = ""
    return out


LINES = logical_lines(DOCKERFILE)


def final_stage():
    idx = [i for i, l in enumerate(LINES) if l.upper().startswith("FROM ")]
    return LINES[idx[-1]:]


class EngineDockerfileContract(unittest.TestCase):
    def test_multi_stage_release_locked_build(self):
        self.assertGreaterEqual(sum(l.upper().startswith("FROM ") for l in LINES), 3)
        build = [l for l in LINES if l.startswith("RUN") and "cargo build" in l]
        self.assertEqual(len(build), 1)
        for flag in ("--release", "--locked", "-p pulso"):
            self.assertIn(flag, build[0])

    def test_runs_as_non_root_numeric_user(self):
        users = [l.split(None, 1)[1] for l in final_stage() if l.upper().startswith("USER ")]
        self.assertTrue(users, "final stage must set USER")
        uid = users[-1].split(":")[0]
        self.assertTrue(uid.isdigit() and int(uid) > 0, users[-1])

    def test_healthcheck_uses_pulso_healthcheck(self):
        hc = [l for l in final_stage() if l.upper().startswith("HEALTHCHECK")]
        self.assertEqual(len(hc), 1)
        self.assertIn("healthcheck", hc[0])
        self.assertIn("pulso", hc[0])
        self.assertNotRegex(hc[0], r"curl|wget")

    def test_entrypoint_is_pulso_with_run_default(self):
        ep = [l for l in final_stage() if l.startswith("ENTRYPOINT")]
        self.assertEqual(len(ep), 1)
        self.assertIn("/usr/local/bin/pulso", ep[0])
        cmd = [l for l in final_stage() if l.startswith("CMD")]
        self.assertEqual(cmd, ['CMD ["run"]'])

    def test_no_secrets_baked(self):
        bad_name = re.compile(r"TOKEN|SECRET|PASSWORD|PASSWD|API_?KEY|DSN|DATABASE_URL|CREDENTIAL", re.I)
        for l in LINES:
            if l.upper().startswith(("ENV ", "ARG ")):
                self.assertIsNone(bad_name.search(l), f"secret-like name in: {l.split('=')[0]}")
        self.assertNotRegex(DOCKERFILE, r"(?i)postgres(ql)?://[^\s:]+:[^\s@]+@")
        self.assertNotRegex(DOCKERFILE, r"AKIA[0-9A-Z]{12,}|sk-[A-Za-z0-9]{16,}")
        self.assertFalse(re.search(r"COPY\s+.*\.env", DOCKERFILE))

    def test_runtime_has_no_build_toolchain_and_env_driven(self):
        stage = " ".join(final_stage())
        self.assertNotIn("cargo", stage)
        self.assertIn("PULSO_CONSOLE_DIR", stage)
        self.assertIn("COPY --from=build", stage)

    def test_dockerignore_excludes_state_and_secrets(self):
        entries = {l.strip() for l in IGNORE if l.strip() and not l.startswith("#")}
        for must in (".git", "data", ".nexus-outbox", "tmp"):
            self.assertIn(must, entries)
        self.assertTrue({"**/target", "target"} & entries)
        self.assertTrue({"**/*.env", ".env*", "**/.env*"} & entries)


if __name__ == "__main__":
    unittest.main()
