"""Verify the engine-owned optional local dependency stack and secret bootstrap."""
import os
from pathlib import Path
import re
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


class LocalObservabilityComposeContractTest(unittest.TestCase):
    """Keep the opt-in LGTM backend local, bounded and isolated without extra Python packages."""

    @classmethod
    def setUpClass(cls):
        cls.compose = COMPOSE.read_text(encoding="utf-8")
        cls.env_example = (ROOT / "local" / ".env.example").read_text(encoding="utf-8")
        lines = cls.compose.splitlines()
        marker = "  otel-lgtm:"
        try:
            start = lines.index(marker)
        except ValueError:
            cls.service = ""
            return
        end = next((index for index in range(start + 1, len(lines)) if re.match(r"^  [a-z][a-z0-9_-]*:\s*$", lines[index])), len(lines))
        cls.service = "\n".join(lines[start:end])

    def test_lgtm_is_opt_in_and_digest_pinned(self):
        self.assertTrue(self.service, "compose must define an otel-lgtm service")
        self.assertRegex(self.service, r"(?m)^    profiles: \[observability\]\s*$")
        self.assertRegex(self.service, r"(?m)^    image: docker\.io/grafana/otel-lgtm:0\.33\.1@sha256:[0-9a-f]{64}\s*$")

    def test_grafana_and_otlp_listeners_bind_only_to_loopback(self):
        published_ports = re.findall(r"(?m)^      - ['\"]?(127\.0\.0\.1:[^'\"\s]+)['\"]?\s*$", self.service)
        self.assertCountEqual(published_ports, [
            "127.0.0.1:${PULSO_OTEL_GRAFANA_PORT:-3000}:3000",
            "127.0.0.1:${PULSO_OTEL_GRPC_PORT:-4317}:4317",
            "127.0.0.1:${PULSO_OTEL_HTTP_PORT:-4318}:4318",
        ])
        self.assertEqual(len(re.findall(r"(?m)^      - ", self.service)), 3)

    def test_lgtm_uses_internal_network_healthcheck_and_bounded_retention(self):
        self.assertRegex(self.service, r"(?m)^    networks: \[pulso-internal\]\s*$")
        self.assertRegex(self.compose, r"(?m)^networks:\s*\n  pulso-internal:\s*\n    internal: true\s*$")
        self.assertRegex(self.service, r"(?m)^    healthcheck:\s*$")
        self.assertIn("test -f /tmp/ready", self.service)
        self.assertIn("/otel-lgtm/docker/healthcheck.sh", self.service)
        self.assertRegex(self.service, r"(?m)^      start_period: 5m\s*$")
        self.assertIn("--storage.tsdb.retention.time=", self.service)
        self.assertRegex(self.service, r'(?m)^      LOKI_EXTRA_ARGS: "-store\.retention=\$\{PULSO_OTEL_LOGS_RETENTION:-24h\} -compactor\.retention-enabled=true -compactor\.delete-request-store=filesystem"$')
        self.assertIn("--compaction.block-retention=", self.service)
        self.assertIn("${PULSO_OTEL_METRICS_RETENTION:-24h}", self.service)
        self.assertIn("${PULSO_OTEL_LOGS_RETENTION:-24h}", self.service)
        self.assertIn("${PULSO_OTEL_TRACES_RETENTION:-24h}", self.service)

    def test_example_retention_values_are_single_supported_duration_tokens(self):
        duration = re.compile(r"^[1-9][0-9]*(?:ns|us|µs|ms|s|m|h)$")
        for name in (
            "PULSO_OTEL_METRICS_RETENTION",
            "PULSO_OTEL_LOGS_RETENTION",
            "PULSO_OTEL_TRACES_RETENTION",
        ):
            with self.subTest(name=name):
                match = re.search(rf"(?m)^{name}=([^\r\n]*)$", self.env_example)
                self.assertIsNotNone(match, f"{name} must be present in local/.env.example")
                self.assertRegex(match.group(1), duration)

    def test_lgtm_does_not_mount_host_data_or_secrets(self):
        self.assertNotRegex(self.service, r"(?m)^    volumes:\s*$")
        self.assertNotIn("/data", self.service)
        self.assertNotIn("/run/secrets", self.service)


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
