"""Verify the engine-owned optional local dependency stack and secret bootstrap."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
COMPOSE = ROOT / "local" / "compose.yaml"
INIT_LOCAL_ENV = ROOT / "scripts" / "init-local-env.ps1"


@unittest.skipUnless(os.environ.get("PULSO_RUN_CONTAINER_TESTS") == "1", "set PULSO_RUN_CONTAINER_TESTS=1 with a working Podman backend")
class LocalComposeContractTest(unittest.TestCase):
    def test_compose_defines_local_postgres_and_s3_emulator_without_persisting_secrets(self):
        environment = dict(os.environ, PULSO_POSTGRES_PASSWORD="test-only-not-persisted")
        result = subprocess.run(["podman", "compose", "-f", str(COMPOSE), "config"], cwd=ROOT, env=environment, capture_output=True, text=True, timeout=30, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("postgres:", result.stdout)
        self.assertIn("localstack:", result.stdout)
        self.assertIn("127.0.0.1", result.stdout)


@unittest.skipUnless(os.name == "nt", "local environment initializer is PowerShell for Windows-first development")
class LocalEnvironmentInitializationTest(unittest.TestCase):
    def test_initializer_defaults_to_the_sibling_local_environment_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "scripts" / "init-local-env.ps1"
            script.parent.mkdir()
            shutil.copy2(INIT_LOCAL_ENV, script)
            result = subprocess.run(["powershell", "-NoProfile", "-File", str(script)], capture_output=True, text=True, timeout=60, check=False)
            exists = (root / "local" / ".env").is_file()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(exists)

    def test_initializer_creates_an_ignored_secret_file_and_refuses_to_replace_it(self):
        with tempfile.TemporaryDirectory() as directory:
            environment_file = Path(directory) / ".env"
            first = subprocess.run(["powershell", "-NoProfile", "-File", str(INIT_LOCAL_ENV), "-Path", str(environment_file)], capture_output=True, text=True, timeout=60, check=False)
            contents = environment_file.read_text(encoding="utf-8")
            second = subprocess.run(["powershell", "-NoProfile", "-File", str(INIT_LOCAL_ENV), "-Path", str(environment_file)], capture_output=True, text=True, timeout=60, check=False)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertIn("PULSO_POSTGRES_PASSWORD=", contents)
        self.assertNotIn(contents.split("PULSO_POSTGRES_PASSWORD=", 1)[1].splitlines()[0], first.stdout)
        self.assertNotEqual(second.returncode, 0)
