"""Fail-closed gateway configuration. The runtime has task stages, so a missing or malformed
`AGENTCORE_LLM_GATEWAY_URL` / `AGENTCORE_LLM_GATEWAY_TOKEN` is a startup error (exit 2, `pulso:runtime_config_invalid`
naming the piece), not a silent `UnconfiguredLLMGateway` that fails every call later. The only way out is the
explicit `PULSO_LLM_MODE=disabled` (fixture/test stacks), which is reported as a double in `/version`.

Messages name the piece and never echo a value (a URL may carry credentials, the token is a secret)."""

from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

from pulso_core_runtime.llm.policy import ModelPolicy

URL_ENV = "AGENTCORE_LLM_GATEWAY_URL"
TOKEN_ENV = "AGENTCORE_LLM_GATEWAY_TOKEN"
MODE_ENV = "PULSO_LLM_MODE"
POLICY_FILE_ENV = "PULSO_LLM_STAGE_POLICY"
POLICY_JSON_ENV = "PULSO_LLM_STAGE_POLICY_JSON"
POLICY_REQUIRED_ENV = "PULSO_LLM_POLICY_REQUIRED"
CEILING_ENV = "PULSO_LLM_SPEND_CEILING"
KILL_FILE_ENV = "PULSO_LLM_KILL_FILE"
UNLIMITED = "unlimited"
# Gateway mode without a configured ceiling is NOT unlimited: a conservative per-job default applies (USD). Opting out
# of the ceiling requires the explicit value `unlimited`. `PULSO_LLM_MODE=disabled` makes no calls, so it has none.
DEFAULT_CEILING_USD = Decimal("5.00")
MAX_CEILING_USD = Decimal("1000000")  # above this a value is a typo (e.g. 1e999), not a budget
MODES = ("gateway", "disabled")


@dataclass(frozen=True)
class LlmConfig:
    mode: str
    url: str | None = None
    token: str | None = field(default=None, repr=False)
    policy: ModelPolicy | None = None
    ceiling_usd: Decimal | None = None  # per-job spend ceiling (None = unlimited)
    kill_file: Path | None = None


def _url_ok(url: str) -> bool:
    try:
        parts = urlsplit(url)
        return parts.scheme in ("http", "https") and bool(parts.hostname)
    except ValueError:
        return False


def _policy(env: Mapping[str, str]) -> tuple[ModelPolicy | None, list[str]]:
    path, inline = env.get(POLICY_FILE_ENV, "").strip(), env.get(POLICY_JSON_ENV, "").strip()
    source, raw = None, ""
    if path and Path(path).is_file():
        source, raw = POLICY_FILE_ENV, Path(path).read_text(encoding="utf-8")
    elif inline:
        source, raw = POLICY_JSON_ENV, inline
    if source is None:
        required = env.get(POLICY_REQUIRED_ENV, "").strip().lower() in {"1", "true", "yes"}
        if required:
            return None, [f"{POLICY_FILE_ENV} or {POLICY_JSON_ENV} is required ({POLICY_REQUIRED_ENV}) and missing"]
        return None, []
    try:
        return ModelPolicy.from_json(json.loads(raw)), []
    except (ValueError, TypeError) as exc:
        detail = exc.args[0] if isinstance(exc, ValueError) and not isinstance(exc, json.JSONDecodeError) else (
            "not valid JSON")
        return None, [f"{source}: {detail}"]


def _ceiling(env: Mapping[str, str]) -> tuple[Decimal | None, list[str]]:
    raw = env.get(CEILING_ENV, "").strip()
    if not raw:
        return DEFAULT_CEILING_USD, []
    if raw.lower() == UNLIMITED:
        return None, []
    try:
        value = Decimal(raw)
    except InvalidOperation:
        value = Decimal("NaN")
    if not value.is_finite() or value < 0 or value > MAX_CEILING_USD:
        return DEFAULT_CEILING_USD, [f"{CEILING_ENV} must be a non-negative decimal USD amount (<= 1000000) or `{UNLIMITED}`"]
    return value, []


def parse_llm_config(env: Mapping[str, str]) -> tuple[LlmConfig | None, list[str]]:
    """Returns (config, problems). `config` is None when there are problems."""
    mode = env.get(MODE_ENV, "").strip().lower() or "gateway"
    if mode not in MODES:
        return None, [f"{MODE_ENV} must be one of {', '.join(MODES)}"]
    if mode == "disabled":
        return LlmConfig(mode="disabled"), []
    problems: list[str] = []
    url, token = env.get(URL_ENV, "").strip(), env.get(TOKEN_ENV, "").strip()
    if not url:
        problems.append(f"{URL_ENV} is empty (the runtime has task stages and no gateway to call; "
                        f"set {MODE_ENV}=disabled only for fixture stacks)")
    elif not _url_ok(url):
        problems.append(f"{URL_ENV} must be an http(s) URL with a host")
    if not token:
        problems.append(f"{TOKEN_ENV} is empty")
    elif token.lower() == "unset":
        problems.append(f"{TOKEN_ENV} is the placeholder `unset` (compose default); set a real consumer token")
    policy, policy_problems = _policy(env)
    problems += policy_problems
    ceiling, ceiling_problems = _ceiling(env)
    problems += ceiling_problems
    if problems:
        return None, problems
    return LlmConfig(mode="gateway", url=url, token=token, policy=policy, ceiling_usd=ceiling,
                     kill_file=Path(kill) if (kill := env.get(KILL_FILE_ENV, "").strip()) else None), []


def llm_doubles(cfg: LlmConfig) -> list[str]:
    """Stand-in/policy lines for `/internal/v1/version.doubles[]` (no URL, no token)."""
    if cfg.mode == "disabled":
        return [f"llm-gateway: disabled ({MODE_ENV}=disabled): every model call fails unavailable"]
    cap = ("unlimited (explicit opt-out)" if cfg.ceiling_usd is None else f"{cfg.ceiling_usd} USD per job")
    lines = [f"llm-spend-ceiling: {cap}"]
    if cfg.policy is None:
        return lines + [("llm-model-policy: unpinned (registry ModelProfile price is trusted; set "
                 f"{POLICY_JSON_ENV} to pin per-stage alias/model/price)")]
    return lines + [f"llm-model-policy: pinned stages {','.join(cfg.policy.stages)}"]
