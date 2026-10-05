"""Rubric judge hook for score_proposal.py backed by the local llm-gateway (POST /v1/generate).

Use: python score_proposal.py ... --judge gateway_judge:judge --builder-model xiaomi/mimo-v2.6-flash --judge-model z-ai/glm-5.3-flash

The judge model must be of ANOTHER family than the Builder (score_proposal enforces it). Env: PULSO_LLM_GATEWAY_ADDR (loopback host:port),
PULSO_LLM_GATEWAY_KEY (bearer, never printed), PULSO_JUDGE_MODEL (default z-ai/glm-5.3-flash). The judge sees the proposal and the base text
(treated, no ids of people) but never Builder reasoning (stripped upstream). It returns {R1,R2,R8,R9,R10,R12} in 0..2.
"""
from __future__ import annotations

import json
import os
import urllib.error
import urllib.request

DEFAULT_JUDGE_MODEL = "z-ai/glm-5.3-flash"
PRICE = {"z-ai/glm-5.3-flash": ("0.15", "0.5")}
CRITERIA = {
    "R1": "the target artifact is the right one for the stated signal",
    "R2": "hypothesis is falsifiable and alternatives (including doing nothing) were weighed",
    "R8": "expected effect is measurable: metric, current and reference rate, direction, decision rule",
    "R9": "side effects on dependent artifacts and sibling routing are considered",
    "R10": "uncertainty and the strength of the data-to-artifact link are stated honestly",
    "R12": "the change is minimal: no budget raised, no model call added",
}
SCHEMA = {"type": "object", "required": list(CRITERIA), "properties": {k: {"type": "integer", "enum": [0, 1, 2]} for k in CRITERIA}}
PROMPT = ("You are an independent reviewer of an improvement proposal for a bank's customer-service agents. Score each criterion 0 (fails), 1 (partly) or 2 (fully). "
          "Be strict; judge only what the proposal and base text show. Answer only the JSON object of scores. Criteria: " + json.dumps(CRITERIA))


def post(addr: str, key: str, body: dict, timeout: int = 150) -> dict:
    if not addr.split(":")[0] in ("127.0.0.1", "localhost", "::1"):
        raise RuntimeError("the judge gateway must be loopback")
    r = urllib.request.Request(f"http://{addr}/v1/generate", data=json.dumps(body).encode(), method="POST",
                               headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:  # noqa: S310 (loopback enforced)
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"gateway HTTP {e.code}") from None
    except OSError as e:
        raise RuntimeError(f"gateway unreachable: {type(e).__name__}") from None


def make_judge(send=post, env=os.environ):
    model = env.get("PULSO_JUDGE_MODEL") or DEFAULT_JUDGE_MODEL
    pin, pout = PRICE.get(model, ("0.15", "0.5"))

    def judge(request: dict) -> dict:
        body = {"prompt": PROMPT, "inputs": {"proposal": request.get("proposal"), "base": request.get("base")}, "schema": SCHEMA,
                "profile": {"endpoint_alias": env.get("PULSO_LLM_GATEWAY_ALIAS", "openrouter"), "model": model, "temperature": 0, "max_tokens": 6000, "timeout_s": 120,
                            "structured": "prompted", "price": {"input_per_mtok": pin, "output_per_mtok": pout}},
                "labels": {"agent": "pulso-rubric-judge"}}
        out = send(env.get("PULSO_LLM_GATEWAY_ADDR", ""), env.get("PULSO_LLM_GATEWAY_KEY", ""), body).get("output")
        if not isinstance(out, dict):
            raise RuntimeError("judge answer is not an object")
        return {k: out[k] for k in CRITERIA if k in out}

    return judge


def judge(request: dict) -> dict:
    return make_judge()(request)
