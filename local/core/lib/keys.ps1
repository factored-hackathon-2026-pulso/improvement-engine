# Executor key (A03 iii) probe: runs INSIDE the runtime container (keys volume mounted read-only), prints a JSON verdict
# that never contains key material (kids and booleans only).
$script:ExecutorProbePy = @'
import json, os
d = os.environ.get("PULSO_KEYS_DIR", "/run/pulso-keys")
def load(n):
    try:
        return json.load(open(os.path.join(d, n), encoding="ascii"))
    except (OSError, ValueError):
        return None
ex, cb, tr = load("bridge-executor.json"), load("bridge-callback.json"), load("lab-broker-trust.json")
print(json.dumps({"executor_present": bool(ex), "callback_present": bool(cb),
    "distinct": bool(ex and cb and ex.get("kid") != cb.get("kid") and ex.get("key") != cb.get("key")),
    "trusted_by_lab_broker": bool(ex and tr and ex.get("kid") in tr.get("keys", {}))}))
'@

function Get-ExecutorKeyProbeArgs { @('python', '-c', $script:ExecutorProbePy) }

function Get-ExecutorKeyVerdict {
    [CmdletBinding()] param([string]$ProbeOutput)
    try { $p = $ProbeOutput | ConvertFrom-Json } catch { return [pscustomobject]@{ status = 'fail'; detail = 'executor key probe returned no JSON' } }
    if (-not $p.executor_present) { return [pscustomobject]@{ status = 'fail'; detail = 'bridge-executor.json missing (runtime fails closed without it)' } }
    if (-not $p.distinct) { return [pscustomobject]@{ status = 'fail'; detail = 'executor key equals the callback key (A03 iii)' } }
    if (-not $p.trusted_by_lab_broker) { return [pscustomobject]@{ status = 'fail'; detail = 'lab-broker trust file does not list the executor kid' } }
    [pscustomobject]@{ status = 'pass'; detail = 'distinct executor keypair, trusted by the lab-broker double' }
}
