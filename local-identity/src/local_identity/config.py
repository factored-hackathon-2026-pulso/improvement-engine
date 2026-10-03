"""Configuration and the local-only guards. Fails closed (exit 2) outside the explicit local profile."""

from __future__ import annotations

import json
import re
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from local_identity import CALLER_AUDIENCE, HUMAN_KID_PREFIX, SESSION_KID_PREFIX
from local_identity.keys import KeyFileError, Signer, b64url_decode, load_signer

HUMAN_ROLES = frozenset({"constructor", "aprobador", "admin"})
REMOTE_ENV_MARKERS = (
    "AWS_EXECUTION_ENV",
    "AWS_LAMBDA_FUNCTION_NAME",
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    "ECS_CONTAINER_METADATA_URI",
    "ECS_CONTAINER_METADATA_URI_V4",
    "KUBERNETES_SERVICE_HOST",
    "K_SERVICE",
    "WEBSITE_SITE_NAME",
)
# Allowlist, not denylist: any other PULSO_ENV value (staging, qa, preprod, ...) is treated as non-local.
LOCAL_PULSO_ENVS = frozenset({"", "local", "dev", "development", "test"})
_BOT_SHAPED = re.compile(r"^(pulso-|builder:|bot[:_-])", re.IGNORECASE)
_ACTOR = re.compile(r"^[A-Za-z0-9._:\-]{1,200}$")
MAX_SKEW_S = 60
MAX_STEP_UP_TTL_S = 120
MAX_SESSION_TTL_S = 60


class ConfigError(Exception):
    """`code` is a stable `local_identity:*` identifier; `piece` names what is wrong, never a value."""

    def __init__(self, code: str, piece: str) -> None:
        super().__init__(f"{code}: {piece}")
        self.code, self.piece = code, piece


@dataclass(frozen=True)
class ServiceKey:
    iss: str
    aud: str
    key: Ed25519PublicKey


@dataclass(frozen=True)
class Identity:
    actor_ref: str
    tenant_id: str
    roles: tuple[str, ...]


@dataclass(frozen=True)
class Config:
    service_keys: Mapping[str, ServiceKey]
    session_signer: Signer
    human_signer: Signer
    identities: Mapping[str, Identity]
    replay_db: str = ":memory:"
    clock_skew_s: int = MAX_SKEW_S
    step_up_ttl_s: int = 60
    session_ttl_s: int = 60
    nonce_retention_s: int = 900
    host: str = "127.0.0.1"
    port: int = 8083
    extra: Mapping[str, str] = field(default_factory=dict)


def _path(env: Mapping[str, str], name: str) -> Path:
    value = env.get(name)
    if not value:
        raise ConfigError("local_identity:config_invalid", f"{name} not set")
    return Path(value)


def _int(env: Mapping[str, str], name: str, default: int, maximum: int) -> int:
    raw = env.get(name)
    if raw is None or raw == "":
        return default
    try:
        value = int(raw)
    except ValueError:
        raise ConfigError("local_identity:config_invalid", f"{name} not an integer") from None
    if not 1 <= value <= maximum:
        raise ConfigError("local_identity:config_invalid", f"{name} out of range 1..{maximum}")
    return value


def _service_keys(path: Path) -> dict[str, ServiceKey]:
    try:
        raw = json.loads(path.read_bytes())["keys"]
        out = {
            kid: ServiceKey(e["iss"], e["aud"], Ed25519PublicKey.from_public_bytes(b64url_decode(e["key"])))
            for kid, e in raw.items()
        }
    except (OSError, KeyError, TypeError, ValueError, AttributeError):
        raise ConfigError("local_identity:service_keys_invalid", f"unreadable ({path.name})") from None
    if not out or any(k.aud != CALLER_AUDIENCE for k in out.values()):
        raise ConfigError("local_identity:service_keys_invalid", "every key must be bound to aud=human-issuer")
    return out


def _identities(path: Path) -> dict[str, Identity]:
    bad = ConfigError("local_identity:identities_invalid", f"invalid ({path.name})")
    try:
        raw = json.loads(path.read_bytes())["identities"]
        out: dict[str, Identity] = {}
        for ref, entry in raw.items():
            roles = entry["roles"]
            if (
                not isinstance(ref, str)
                or not _ACTOR.fullmatch(ref)
                or _BOT_SHAPED.match(ref)
                or entry.get("actor", "human") != "human"
                or set(entry) - {"tenant_id", "roles", "actor"}
                or not isinstance(roles, list)
                or not roles
                or not set(roles) <= HUMAN_ROLES
                or not isinstance(entry["tenant_id"], str)
                or not entry["tenant_id"]
            ):
                raise bad
            out[ref] = Identity(ref, entry["tenant_id"], tuple(roles))
    except ConfigError:
        raise
    except (OSError, KeyError, TypeError, ValueError, AttributeError):
        raise bad from None
    if not out:
        raise bad
    return out


def _bot_public_keys(path: Path) -> set[bytes]:
    try:
        raw = json.loads(path.read_bytes())
        entries = raw.get("keys") or raw.get("principal_keys") or {}
        return {b64url_decode(e["key"] if isinstance(e, dict) else e) for e in entries.values()}
    except (OSError, KeyError, TypeError, ValueError, AttributeError):
        raise ConfigError("local_identity:config_invalid", f"bot public keys unreadable ({path.name})") from None


def load_config(env: Mapping[str, str]) -> Config:
    if env.get("LOCAL_IDENTITY_PROFILE") != "local":
        raise ConfigError("local_identity:profile_not_local", "LOCAL_IDENTITY_PROFILE must be exactly 'local'")
    if any(env.get(m) for m in REMOTE_ENV_MARKERS) or env.get("PULSO_ENV", "").strip().lower() not in LOCAL_PULSO_ENVS:
        raise ConfigError("local_identity:remote_environment", "remote runtime indicators present")
    service_keys = _service_keys(_path(env, "LOCAL_IDENTITY_SERVICE_KEYS"))
    try:
        session = load_signer(_path(env, "LOCAL_IDENTITY_SESSION_SIGNER"))
        human = load_signer(_path(env, "LOCAL_IDENTITY_HUMAN_STAFF_SIGNER"))
    except KeyFileError as exc:
        raise ConfigError("local_identity:config_invalid", str(exc)) from None
    if not human.kid.startswith(HUMAN_KID_PREFIX):
        raise ConfigError("local_identity:kid_not_local", f"human staff kid must start with {HUMAN_KID_PREFIX}")
    if not session.kid.startswith(SESSION_KID_PREFIX):
        raise ConfigError("local_identity:kid_not_local", f"session kid must start with {SESSION_KID_PREFIX}")
    forbidden = {session.public_key} | {k.key.public_bytes_raw() for k in service_keys.values()}
    bot_file = env.get("LOCAL_IDENTITY_BOT_PUBLIC_KEYS")
    if bot_file:
        forbidden |= _bot_public_keys(Path(bot_file))
    if human.public_key in forbidden or session.public_key in {k.key.public_bytes_raw() for k in service_keys.values()}:
        raise ConfigError("local_identity:key_reuse", "human staff, session, service and bot keys must all differ")
    return Config(
        service_keys=service_keys,
        session_signer=session,
        human_signer=human,
        identities=_identities(_path(env, "LOCAL_IDENTITY_IDENTITIES")),
        replay_db=env.get("LOCAL_IDENTITY_REPLAY_DB") or ":memory:",
        clock_skew_s=_int(env, "LOCAL_IDENTITY_CLOCK_SKEW_S", MAX_SKEW_S, MAX_SKEW_S),
        step_up_ttl_s=_int(env, "LOCAL_IDENTITY_STEP_UP_TTL_S", 60, MAX_STEP_UP_TTL_S),
        session_ttl_s=_int(env, "LOCAL_IDENTITY_SESSION_TTL_S", 60, MAX_SESSION_TTL_S),
        host=env.get("LOCAL_IDENTITY_HOST") or "127.0.0.1",
        port=_int(env, "LOCAL_IDENTITY_PORT", 8083, 65535),
    )
