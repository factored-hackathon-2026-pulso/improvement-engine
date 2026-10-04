import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def read(p):
    return (ROOT / p).read_text(encoding="utf-8")


class BriefTest(unittest.TestCase):
    def test_codex_brief_states_data_class_rule(self):
        t = read("docs/agents/codex-onboarding.md").lower()
        for needle in ("data-class rule", "e0", "synthetic", "schemas", "digest",
                       "depth-1", "seams/", "bundle"):
            self.assertIn(needle, t)

    def test_governance_doc_has_tags_state_table_trailer(self):
        t = read("docs/agents/governance.md")
        for tag in ("[CONTRACT-PUBLISHED]", "[CONTRACT-CHANGE]", "[ASK]", "[DEP-ASK]",
                    "[BLOCKED]", "[DONE]", "[HANDOFF]", "[FINDING]", "Team:", "state table"):
            self.assertIn(tag, t)

    def test_agents_md_links_ownership_and_brief(self):
        t = read("AGENTS.md")
        for needle in ("OWNERS.md", "docs/agents/codex-onboarding.md",
                       "docs/agents/governance.md", "data-class"):
            self.assertIn(needle, t)


if __name__ == "__main__":
    unittest.main()
