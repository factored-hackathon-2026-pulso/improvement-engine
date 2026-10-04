"""DC0 gateway profile checks (pure stdlib): gw-e0 has no key and no external route; gw-hosted never takes E0."""

from __future__ import annotations

import ipaddress
import json
import re
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse

RESTRICTED = frozenset({"e0", "csv", "original-treated", "original_treated"})
_KEYISH = re.compile(r"key|token|secret|authorization|password|bearer", re.I)
_SECRET_VALUE = re.compile(r"(?:sk-[A-Za-z0-9_-]{16,}|Bearer\s+\S+|AIza[0-9A-Za-z_-]{20,})")
_INTERNAL_SUFFIXES = (".internal", ".local", ".localhost", ".lan")
_IGNORED_KEYS = frozenset({"description"})


@dataclass(frozen=True)
class Violation:
    profile: str
    rule: str


def _walk(node, prefix=""):
    if isinstance(node, dict):
        for k, v in node.items():
            if k in _IGNORED_KEYS:
                continue
            yield from _walk(v, f"{prefix}.{k}" if prefix else k)
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from _walk(v, f"{prefix}[{i}]")
    else:
        yield prefix, node


def collect_keys(profile: dict) -> list[str]:
    """Paths of credential-looking fields that carry a value."""
    return [p for p, v in _walk(profile) if _KEYISH.search(p.rsplit(".", 1)[-1]) and v not in (None, "", False)]


def collect_urls(profile: dict) -> list[str]:
    """Every string that is a URL or sits under a url/uri/endpoint/host/base key, wherever it is (incl. fallbacks)."""
    hostish = re.compile(r"(url|uri|endpoint|host|address|origin)", re.I)
    return [
        v
        for p, v in _walk(profile)
        if isinstance(v, str) and (re.match(r"^[A-Za-z][A-Za-z0-9+.-]*://", v) or hostish.search(p.rsplit(".", 1)[-1]))
    ]


def is_internal_url(url: str) -> bool:
    host = (urlparse(url).hostname or "").lower()
    if not host:
        return False
    try:
        ip = ipaddress.ip_address(host)
        return ip.is_private or ip.is_loopback
    except ValueError:
        pass
    return "." not in host or host.endswith(_INTERNAL_SUFFIXES)


def check_profile(profile: dict) -> list[Violation]:
    name = profile.get("name", "?")
    out: list[Violation] = []
    classes = {str(c).strip().lower() for c in profile.get("data_classes", [])}
    if any(re.sub(r"[\s_-]+", "-", c).startswith(("e0", "csv", "original")) for c in classes):
        classes |= RESTRICTED  # E0-derived, e0_treated, Original Treated: a restricted class by prefix
    body = json.dumps({k: v for k, v in profile.items() if k not in _IGNORED_KEYS})
    if _SECRET_VALUE.search(body):
        out.append(Violation(name, "secret_value"))
    if classes & RESTRICTED:
        if collect_keys(profile):
            out.append(Violation(name, "e0_has_key"))
        if any(not is_internal_url(u) for u in collect_urls(profile)):
            out.append(Violation(name, "e0_external_route"))
        if profile.get("network", {}).get("internal") is not True:
            out.append(Violation(name, "network_not_internal"))
    if profile.get("upstream", {}).get("kind") != "local":  # anything not explicitly local is third-party
        if classes & RESTRICTED:
            out.append(Violation(name, "hosted_accepts_restricted"))
        if not profile.get("api_key_env") and profile.get("upstream", {}).get("kind") == "hosted":
            out.append(Violation(name, "hosted_missing_key_ref"))
    return out


def check_all(profiles_dir: Path) -> list[Violation]:
    out: list[Violation] = []
    for p in sorted(profiles_dir.glob("*.json")):
        out += check_profile(json.loads(p.read_text("utf-8")))
    return out


def route_allowed(profile: dict, data_class: str) -> bool:
    allowed = {str(c).lower() for c in profile.get("data_classes", [])}
    return data_class.lower() in allowed and not check_profile(profile)


def compose_has_internal_network(text: str, name: str) -> bool:
    """True when `networks: <name>:` declares `internal: true` (line-based, no YAML dependency)."""
    block = re.search(rf"^  {re.escape(name)}:\s*\n((?:    .*\n?)*)", text, re.M)
    return bool(block and re.search(r"^\s+internal:\s*true\s*$", block.group(1), re.M))
