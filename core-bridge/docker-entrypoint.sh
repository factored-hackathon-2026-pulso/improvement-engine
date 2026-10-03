#!/bin/sh
# Entrypoints: runtime | exporter | seed | bootstrap | migrate | sweep | agentcore <args>.
# Fargate injects secrets as env (DR-89): materialise key files into tmpfs before exec.
set -eu
KEYS_DIR="${PULSO_KEYS_DIR:-/run/pulso-keys}"
if [ -d "$KEYS_DIR" ] && [ -w "$KEYS_DIR" ]; then
  umask 077
  [ -n "${CORE_IDENTITY_KEYS_JSON:-}" ] && printf '%s' "$CORE_IDENTITY_KEYS_JSON" > "$KEYS_DIR/identity.json"
  [ -n "${CORE_STAFF_KEYS_JSON:-}" ] && printf '%s' "$CORE_STAFF_KEYS_JSON" > "$KEYS_DIR/staff.json"
  [ -n "${PULSO_SERVICE_KEYS_JSON:-}" ] && printf '%s' "$PULSO_SERVICE_KEYS_JSON" > "$KEYS_DIR/service.json"
fi
unset CORE_IDENTITY_KEYS_JSON CORE_STAFF_KEYS_JSON PULSO_SERVICE_KEYS_JSON
cmd="${1:-runtime}"; [ $# -gt 0 ] && shift
case "$cmd" in
  runtime)  exec python -m pulso_core_runtime.main "$@" ;;
  migrate)  exec agentcore migrate "$@" ;;
  agentcore) exec agentcore "$@" ;;
  exporter|seed|bootstrap|sweep) exec python -m "pulso_core_runtime.$cmd" "$@" ;;
  *) echo "pulso:entrypoint_unknown $cmd" >&2; exit 2 ;;
esac
