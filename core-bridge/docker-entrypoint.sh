#!/bin/sh
# Entrypoints: runtime | exporter | migrate | agentcore <args> (seeding is the local/core init job).
# Fargate injects secrets as env (DR-89, ADR 0009). Before exec this script materialises every key file the selected
# entrypoint needs from named env vars into tmpfs ($PULSO_KEYS_DIR, default /run/pulso-keys, mode 0400, owner = the
# app uid), validates shape/length, fails closed (exit 2, `pulso:runtime_config_invalid` naming the variable, never a
# value), and then unsets the secret env vars so neither the child environment nor /proc/<pid>/environ carries them.
# File mode (local/core mounts the files and sets the *_KEYS / *_SIGNER / PULSO_EXPORTER_KEY_* path vars) is unchanged.
set -eu
cmd="${1:-runtime}"; [ $# -gt 0 ] && shift
case "$cmd" in runtime|exporter|migrate|agentcore) ;; *) echo "pulso:entrypoint_unknown $cmd" >&2; exit 2 ;; esac

exports="$(PULSO_ENTRYPOINT_CMD="$cmd" python - <<'PY'
import base64, json, os, sys

CMD = os.environ["PULSO_ENTRYPOINT_CMD"]
KEYS_DIR = os.environ.get("PULSO_KEYS_DIR", "/run/pulso-keys")
E = os.environ


def b64(value):
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


def key32(text):
    return len(b64(text.strip())) == 32


def signer_ok(text):
    d = json.loads(text)
    return bool(d["kid"]) and key32(d["key"])


def keyset_ok(text):
    d = json.loads(text)
    return isinstance(d, dict) and bool(d)


def service_ok(text):
    keys = json.loads(text)["keys"]
    return bool(keys) and all(e["iss"] and e["aud"] and len(b64(e["key"])) == 32 for e in keys.values())


# (content env var, path env var, default file name, validator)
RUNTIME = [
    ("PULSO_BRIDGE_IDENTITY_SIGNER_JSON", "PULSO_BRIDGE_IDENTITY_SIGNER", "bridge-identity.json", signer_ok),
    ("PULSO_BRIDGE_STAFF_SIGNER_JSON", "PULSO_BRIDGE_STAFF_SIGNER", "bridge-staff.json", signer_ok),
    ("PULSO_BRIDGE_CALLBACK_SIGNER_JSON", "PULSO_BRIDGE_CALLBACK_SIGNER", "bridge-callback.json", signer_ok),
    ("PULSO_BRIDGE_EXECUTOR_SIGNER_JSON", "PULSO_BRIDGE_EXECUTOR_SIGNER", "bridge-executor.json", signer_ok),
    ("CORE_IDENTITY_KEYS_JSON", "PULSO_IDENTITY_KEYS", "identity.json", keyset_ok),
    ("CORE_STAFF_KEYS_JSON", "PULSO_STAFF_KEYS", "staff.json", keyset_ok),
    ("PULSO_SERVICE_KEYS_JSON", "PULSO_SERVICE_KEYS", "service.json", service_ok),
]
EXPORTER = [
    ("PULSO_EXPORTER_KEY_CONTROL_API_SEED", "PULSO_EXPORTER_KEY_CONTROL_API", "exporter-control-api.key", key32),
    ("PULSO_EXPORTER_KEY_LAB_BROKER_SEED", "PULSO_EXPORTER_KEY_LAB_BROKER", "exporter-lab-broker.key", key32),
]
SPEC = {"runtime": RUNTIME, "exporter": EXPORTER}.get(CMD, [])
if "AGENTCORE_ALLOW_DEMO" in E:  # the runtime itself refuses the demo flag (exit 2, pulso:demo_double_in_real_mode)
    SPEC = []


problems = []


def fail(message):
    problems.append(message)


def finish():
    if problems:
        sys.stderr.write("".join(f"pulso:runtime_config_invalid: {m}\n" for m in problems))
        sys.exit(2)


def materialise(name, content):
    os.makedirs(KEYS_DIR, mode=0o700, exist_ok=True)
    path = os.path.join(KEYS_DIR, name)
    if os.path.exists(path):
        os.chmod(path, 0o600)
        os.unlink(path)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400)
    with os.fdopen(fd, "w", encoding="ascii") as fh:
        fh.write(content)
    return path


resolved = {}
for var, path_var, name, valid in SPEC:
    content = E.get(var, "")
    if content.strip():
        try:
            if not valid(content):
                raise ValueError
        except Exception:
            fail(f"{var} is malformed (wrong shape or key length)")
            continue
        try:
            path = materialise(name, content)
        except OSError:
            fail(f"{var} cannot be materialised under {KEYS_DIR}")
            continue
        print(f"{path_var}={path}")
    else:
        path = E.get(path_var) or os.path.join(KEYS_DIR, name)  # file mode (local/core mounts)
        try:
            with open(path, encoding="ascii") as fh:
                text = fh.read()
            ok = bool(text.strip()) and valid(text)
        except Exception:
            ok = False
        if not ok:
            fail(f"{var} is missing or malformed (and no readable key file at {path_var})")
            continue
    resolved[var] = path

if CMD == "runtime" and SPEC and not problems:
    def key_of(var):
        return json.load(open(resolved[var], encoding="ascii"))["key"]
    if key_of("PULSO_BRIDGE_EXECUTOR_SIGNER_JSON") == key_of("PULSO_BRIDGE_CALLBACK_SIGNER_JSON"):
        fail("PULSO_BRIDGE_EXECUTOR_SIGNER_JSON must differ from PULSO_BRIDGE_CALLBACK_SIGNER_JSON")
finish()
PY
)" || exit $?
while IFS='=' read -r k v; do [ -n "$k" ] && export "$k=$v"; done <<EOF2
$exports
EOF2
unset PULSO_BRIDGE_IDENTITY_SIGNER_JSON PULSO_BRIDGE_STAFF_SIGNER_JSON PULSO_BRIDGE_CALLBACK_SIGNER_JSON \
  PULSO_BRIDGE_EXECUTOR_SIGNER_JSON CORE_IDENTITY_KEYS_JSON CORE_STAFF_KEYS_JSON PULSO_SERVICE_KEYS_JSON \
  PULSO_EXPORTER_KEY_CONTROL_API_SEED PULSO_EXPORTER_KEY_LAB_BROKER_SEED
case "$cmd" in
  runtime)  exec python -m pulso_core_runtime.main "$@" ;;
  migrate)  exec agentcore migrate "$@" ;;
  agentcore) exec agentcore "$@" ;;
  exporter) exec python -m pulso_core_runtime.exporter "$@" ;;
esac
