"""codex-standin: a Python stand-in of the Rust engine (Codex side) for the Claude-side E2E precursor (plan 17.3.8,
annex D, A03). Everything here that plays the platform is a DOUBLE and is declared as such in the e2e report."""

from pathlib import Path

import yaml


def _pin_sha() -> str:
    """The agent-core pin comes from agent-core-assets/manifest.yaml (SHA-agnostic: a pin bump needs no harness edit)."""
    manifest = Path(__file__).resolve().parents[3] / "agent-core-assets" / "manifest.yaml"
    return str(yaml.safe_load(manifest.read_text(encoding="utf-8"))["pin"]["sha"])


PIN_SHA = _pin_sha()
