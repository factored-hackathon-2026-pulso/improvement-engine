"""Loads the engine-steps contract schemas (contracts/engine-steps) and validates documents with its minischema."""
import json
import sys
from pathlib import Path

_STEPS = Path(__file__).resolve().parents[3] / "contracts" / "engine-steps"
if str(_STEPS) not in sys.path:
    sys.path.insert(0, str(_STEPS))
import minischema  # noqa: E402


def _schema(step: str, side: str) -> dict:
    return json.loads((_STEPS / "schemas" / f"{step}.{side}.schema.json").read_text("utf-8"))


def validate_in(step: str, doc: dict) -> list[str]:
    return minischema.validate(doc, _schema(step, "in"))


def validate_out(step: str, doc: dict) -> list[str]:
    return minischema.validate(doc, _schema(step, "out"))
