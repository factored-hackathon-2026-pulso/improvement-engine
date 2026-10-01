"""Keep the real U02 PostgreSQL migration gate self-contained in engine CI."""

from pathlib import Path
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / ".github" / "workflows" / "ci.yml"
POSTGRES_MIGRATION_TEST = (
    Path(__file__).resolve().parents[1]
    / "crates"
    / "core"
    / "tests"
    / "postgres_artifact_migration.rs"
)
POSTGRES_IMAGE = "postgres:17@sha256:d74eeac9a635390a49bc21bd49fccd973de707e2a53a76ac49b552b8712ec46f"
class PostgresCiContractTest(unittest.TestCase):
    def test_ci_runs_the_isolated_postgres_migration_gate_locally(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("pull_request:", workflow)
        self.assertIn("branches: [main]", workflow)
        self.assertIn("postgres-artifact-migration:", workflow)
        self.assertIn("runs-on: ubuntu-latest", workflow)
        self.assertIn(f"image: {POSTGRES_IMAGE}", workflow)
        self.assertIn("ports:\n          - 5432:5432", workflow)
        self.assertIn("PULSO_TEST_POSTGRES_URL", workflow)
        self.assertIn("PULSO_ALLOW_DESTRUCTIVE_TEST_DB", workflow)
        self.assertIn(
            "cargo +1.98.1 test --locked --workspace migration_enforces_cas_immutability_and_source_snapshot_kind -- --ignored",
            workflow,
        )

    def test_local_gate_isolated_and_never_uses_cross_repo_or_secrets(self):
        workflow = WORKFLOW.read_text(encoding="utf-8").lower()

        self.assertNotIn("secrets:", workflow)
        self.assertNotIn("secrets: inherit", workflow)
        self.assertNotIn("continue-on-error: true", workflow)
        self.assertNotIn("|| true", workflow)
        self.assertNotIn("pulso-factored/infra", workflow)
        self.assertNotIn("uses: pulso-factored", workflow)

    def test_real_postgres_regression_binds_json_as_jsonb_not_a_rust_string(self):
        migration_test = POSTGRES_MIGRATION_TEST.read_text(encoding="utf-8")

        self.assertIn("let payload = json!({});", migration_test)
        self.assertIn("&payload", migration_test)
        self.assertNotIn('&"{}"', migration_test)


if __name__ == "__main__":
    unittest.main()
