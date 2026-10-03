#!/bin/sh
# Fargate injects secrets as env (DR-89; same pattern as core-bridge/docker-entrypoint.sh, ADR 0009). Before exec this
# script materialises the control-api audience key seed from PULSO_EXPORTER_KEY_CONTROL_API_SEED into tmpfs-like task
# storage ($PULSO_KEYS_DIR, default /run/pulso-keys, mode 0400, owner = the app uid), validates that it is a base64url
# 32-byte value, fails closed (exit 2, `pulso:runtime_config_invalid` naming the variable, never a value) and then
# unsets the seed variable so neither the child environment nor /proc/<pid>/environ carries it. File mode (the path
# variable PULSO_EXPORTER_KEY_CONTROL_API pointing at a mounted file) is accepted when the seed variable is unset.
# The exporter reads the key file and mints a fresh service JWT per HTTP attempt (PL-0009). The lab-broker audience key
# (artifacts route) is OPTIONAL: PULSO_EXPORTER_KEY_LAB_BROKER_SEED is materialised/validated only when provided.
set -eu

exports="$(python - <<'PY'
import base64, os, sys

KEYS_DIR = os.environ.get("PULSO_KEYS_DIR", "/run/pulso-keys")
E = os.environ


def key32(text):
    text = text.strip()
    return len(base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))) == 32


# (content env var, path env var, default file name, validator, optional)
SPEC = [("PULSO_EXPORTER_KEY_CONTROL_API_SEED", "PULSO_EXPORTER_KEY_CONTROL_API", "exporter-control-api.key", key32, False),
        ("PULSO_EXPORTER_KEY_LAB_BROKER_SEED", "PULSO_EXPORTER_KEY_LAB_BROKER", "exporter-lab-broker.key", key32, True)]
problems = []


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


for var, path_var, name, valid, optional in SPEC:
    content = E.get(var, "")
    if content.strip():
        try:
            if not valid(content):
                raise ValueError
        except Exception:
            problems.append(f"{var} is malformed (wrong shape or key length)")
            continue
        try:
            print(f"{path_var}={materialise(name, content)}")
        except OSError:
            problems.append(f"{var} cannot be materialised under {KEYS_DIR}")
    else:
        if optional and not E.get(path_var):
            continue
        path = E.get(path_var) or os.path.join(KEYS_DIR, name)  # file mode
        try:
            with open(path, encoding="ascii") as fh:
                ok = valid(fh.read())
        except Exception:
            ok = False
        if not ok:
            problems.append(f"{var} is missing or malformed (and no readable key file at {path_var})")
if problems:
    sys.stderr.write("".join(f"pulso:runtime_config_invalid: {m}\n" for m in problems))
    sys.exit(2)
PY
)" || exit $?
while IFS='=' read -r k v; do [ -n "$k" ] && export "$k=$v"; done <<EOF2
$exports
EOF2
unset PULSO_EXPORTER_KEY_CONTROL_API_SEED PULSO_EXPORTER_KEY_LAB_BROKER_SEED
exec python -m platform_exporter "$@"
