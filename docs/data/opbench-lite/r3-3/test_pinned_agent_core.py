"""Optional offline validation against a read-only pinned Agent Core checkout.

Set PULSO_AGENT_CORE_DIR to the exact c814c2b checkout before running this module.
"""

import os
import json
import subprocess
import sys
import types
import unittest
from pathlib import Path

from generate_scenarios import build_pack


class PinnedAgentCoreTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        raw = os.environ.get("PULSO_AGENT_CORE_DIR")
        if not raw:
            raise unittest.SkipTest("set PULSO_AGENT_CORE_DIR to pinned c814c2b checkout")
        cls.core = Path(raw).resolve()
        revision = subprocess.run(["git", "-c", f"safe.directory={cls.core.as_posix()}",
                                   "-C", str(cls.core), "rev-parse", "--short", "HEAD"],
                                  capture_output=True, text=True, check=True).stdout.strip()
        if revision != "c814c2b":
            raise AssertionError(f"Agent Core checkout must be pinned at c814c2b, got {revision}")
        sys.path.insert(0, str(cls.core))
        import yaml
        from agent_core.domain import Agent
        # The registry package initializer imports every persistence backend (including psycopg).
        # We need only the pinned schema module; expose the package path without executing that
        # unrelated initializer so this isolated contract check stays dependency-minimal.
        registry = types.ModuleType("agent_core.registry")
        registry.__path__ = [str(cls.core / "agent_core" / "registry")]
        sys.modules["agent_core.registry"] = registry
        from agent_core.registry.suite import EvalSuite, suite_problems
        cls.yaml, cls.Agent, cls.EvalSuite = yaml, Agent, EvalSuite
        cls.suite_problems = staticmethod(suite_problems)

    def test_each_suite_and_agent_validate_with_pinned_models(self):
        pack = build_pack()
        agent_dir = self.core / "tests" / "fixtures" / "registry-e2e" / "agents"
        generator = Path(__file__).with_name("generate_scenarios.py")
        for agent_id, raw_suite in pack["suites"].items():
            with self.subTest(agent=agent_id):
                emitted = subprocess.run([sys.executable, str(generator), "--suite", agent_id],
                                         capture_output=True, text=True, check=True)
                emitted_suite = json.loads(emitted.stdout)
                self.assertEqual(raw_suite, emitted_suite)
                suite = self.EvalSuite.model_validate(emitted_suite)
                agent_doc = self.yaml.safe_load((agent_dir / f"{agent_id}@1.0.0.yaml").read_text(encoding="utf-8"))
                agent = self.Agent.model_validate(agent_doc)
                problems = [problem.model_dump(mode="json") for problem in self.suite_problems(agent, suite)]
                self.assertEqual([], problems)


if __name__ == "__main__":
    unittest.main()
