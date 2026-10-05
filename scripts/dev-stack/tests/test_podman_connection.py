"""Which `podman` connection stack.py uses on each OS (same rules as Get-DevPodmanArgs in DevConfig.ps1). No podman, no network."""
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("stack", Path(__file__).resolve().parents[1] / "stack.py")
stack = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stack)


class PodmanConnArgs(unittest.TestCase):
    def test_explicit_connection_wins(self):
        env = {"PULSO_PODMAN_CONNECTION": "c1", "PULSO_PODMAN_MACHINE": "m"}
        self.assertEqual(stack.podman_conn_args(env, "posix"), ["--connection", "c1"])

    def test_machine_maps_to_root_connection(self):
        self.assertEqual(stack.podman_conn_args({"PULSO_PODMAN_MACHINE": "dev2"}, "posix"), ["--connection", "dev2-root"])

    def test_empty_machine_is_default_connection(self):
        self.assertEqual(stack.podman_conn_args({"PULSO_PODMAN_MACHINE": ""}, "nt"), [])

    def test_unset_windows_keeps_legacy_linux_is_native(self):
        self.assertEqual(stack.podman_conn_args({}, "nt"), ["--connection", "pulso-dev-root"])
        self.assertEqual(stack.podman_conn_args({}, "posix"), [])


if __name__ == "__main__":
    unittest.main()
