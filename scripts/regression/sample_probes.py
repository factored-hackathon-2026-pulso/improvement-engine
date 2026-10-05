"""Collect the model samples of the `generated_contains` probes for the engine (W13): the only regression script with network.

    python scripts/regression/sample_probes.py < judge_input.json > generated.json

Input: the `w11.judge_input/1` document (bundle, base_artifacts, attempts[{changes}], optional `generated` already collected).
Output: `{"generated": {<probe_key>: {"samples": [...]} | {"error": "..."}}}`: one entry per distinct (prompt text, locale, inputs,
n) the judge will look up (the base text and every candidate text); entries already present are kept and not requested again.
The prompt text under test is sent to the LOCAL llm-gateway (`xiaomi/mimo-v2.6-flash`, the engine's generation model); an
unreachable gateway or an HTTP error is recorded as `error` and the judge then marks the case "not measured" (a failure, never a
pass). The consumer token is read from this process' environment (`GATEWAY_TOKEN_AGENT_CORE`) or `PULSO_PROBE_ENV_FILE` and is
only sent as a bearer header to `PULSO_PROBE_GATEWAY` (default http://127.0.0.1:8092); it is never printed. No clock, no files
written. Exit codes: 0 printed, 2 malformed input.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import prove_fails_on_base as prove  # noqa: E402


def needed(doc: dict) -> list[tuple[str, dict]]:
    """(text, probe) pairs the judge will request: base text + each attempt's candidate text, per generated probe."""
    bundle = doc["bundle"]
    base_artifacts = doc.get("base_artifacts") or json.loads(prove.BASE_ARTIFACTS.read_text(encoding="utf-8"))
    probes = [p for p in bundle.get("probes", []) if p["kind"] == "generated_contains"]
    out: list[tuple[str, dict]] = []
    variants = [[]] + [a.get("changes") or [] for a in doc.get("attempts") or []]
    for changes in variants:
        for p in probes:
            text = prove.texts_of(changes, base_artifacts, p["prompt_id"], "prompt").get(p["locale"])
            if text is not None:
                out.append((text, p))
    return out


def collect(doc: dict, generate) -> dict:
    have = dict(doc.get("generated") or {})
    for text, p in needed(doc):
        key = prove.probe_key(text, p["locale"], p["inputs"], p["samples"])
        if key in have and "samples" in have[key]:
            continue
        outs = generate(text, p["inputs"], p["locale"], p["samples"])
        have[key] = {"samples": outs} if outs else {"error": getattr(generate, "last_error", "") or "no samples"}
    return have


def main() -> int:
    try:
        doc = json.loads(sys.stdin.buffer.read().decode("utf-8"))
        env_file = Path(os.environ["PULSO_PROBE_ENV_FILE"]) if os.environ.get("PULSO_PROBE_ENV_FILE") else None
        generate = prove.make_generator(os.environ.get("PULSO_PROBE_GATEWAY", "http://127.0.0.1:8092"), env_file,
                                        os.environ.get("PULSO_PROBE_MODEL", prove.MODEL_POLICY["generation"]))
        result = {"generated": collect(doc, generate)}
    except (ValueError, KeyError, TypeError) as e:
        print(json.dumps({"error": "malformed_input", "why": type(e).__name__ + ": " + str(e)[:120]}))
        return 2
    sys.stdout.buffer.write(json.dumps(result, ensure_ascii=False).encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
