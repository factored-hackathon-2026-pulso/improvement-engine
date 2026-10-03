"""Sandbox-only local human issuer (plan 16.13.2 / 17.3.2, DR-08). Never deployed to staging or production."""

SERVICE_NAME = "human-issuer"
ISSUER_NAME = "human-issuer"
CALLER_ISSUER = "control-api"
CALLER_AUDIENCE = "human-issuer"
SESSION_AUDIENCE = "control-api"
SESSION_PURPOSE = "session_assertion"
COMMAND_PURPOSE = "command_authorization"
# Test kids carry these prefixes; the contract scan fails when one shows up in remote configuration.
HUMAN_KID_PREFIX = "local-sim-human-"
SESSION_KID_PREFIX = "local-sim-session-"
