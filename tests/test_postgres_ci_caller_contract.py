"""Keep the real U02 PostgreSQL gate delegated to the reviewed infra workflow."""

from pathlib import Path
import re
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / ".github" / "workflows" / "ci.yml"
INFRA_SHA = "8f6df2e1916480b4dcacc5e60094d0f2c76d69d2"
CALLER = (
    "pulso-factored/infra/.github/workflows/postgres-integration.yml"
    f"@{INFRA_SHA}"
)


class PostgresCiCallerContractTest(unittest.TestCase):
    def test_ci_calls_the_pinned_reusable_postgres_migration_gate(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        trigger_block = re.search(
            r"(?ms)^on:\n(?P<triggers>.*?)(?=^permissions:)", workflow
        )

        self.assertIn("pull_request:", workflow)
        self.assertIn("branches: [main]", workflow)
        self.assertIsNotNone(trigger_block)
        self.assertRegex(
            trigger_block.group("triggers"),
            r"(?m)^  workflow_dispatch:\s*$\n^$",
        )
        self.assertIn("postgres-artifact-migration:", workflow)
        self.assertIn(f"uses: {CALLER}", workflow)
        self.assertIn("permissions:\n      contents: read", workflow)

    def test_caller_does_not_bypass_the_isolated_gate_or_supply_secrets(self):
        workflow = WORKFLOW.read_text(encoding="utf-8").lower()

        self.assertNotIn("secrets:", workflow)
        self.assertNotIn("secrets: inherit", workflow)
        self.assertNotIn("continue-on-error: true", workflow)
        self.assertNotIn("|| true", workflow)
        self.assertNotIn("postgres:17", workflow)
        self.assertNotIn("pulso_test_postgres_url", workflow)
        self.assertNotIn("pulso_allow_destructive_test_db", workflow)


if __name__ == "__main__":
    unittest.main()
