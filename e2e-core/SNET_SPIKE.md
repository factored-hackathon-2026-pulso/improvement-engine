# SNET spike: container-to-host reachability, clock skew, long calls, timeouts

Status: PARTIAL. Probe and tests are done; the container-side measurement is blocked (see below).

Tool: `e2e-core/src/snet_probe.py` (stdlib only; tests in `e2e-core/tests/unit/test_snet_probe.py`, written first).
- Host: `python snet_probe.py serve --host 0.0.0.0 --port 18080` (shim with `/time` and `/sleep?s=N`).
- Container: `python snet_probe.py probe --base http://host.containers.internal:18080 [--quick] [--long]`.
- Exit 1 when the shim is unreachable. Calls measured: 1 s under a 60 s limit, 70 s under 60 s (expect typed
  `timeout`, i.e. the gateway `timeout_s: 60` case), 70 s under 600 s (ok), and with `--long` 300 s under 600 s.

## Findings so far
- Go builder image: `docker.io/library/golang:1.27` IS present in the `pulso-dev` connection (the plan assumed absent);
  the default connection `pulso-dev-root` and `pulso-codex` differ, so check with `--connection pulso-dev`. ASK-4 may
  be unnecessary; M1 should confirm the version matches the pinned llm-gateway `go.mod`.
- Gateway profile `timeout_s` is 60 (`agent-core-assets/.../pulso-evolution-structured@1.0.0.yaml`); the Core invoke
  timeout is now configurable (CLT0, `PULSO_CORE_INVOKE_TIMEOUT_S`, default 600), so a 70 s model call hits the
  gateway limit first and must surface as a typed timeout, then `unknown`, then reconcile.
- Host-side probe works against localhost (5 unit tests green, including unreachable shim fails).

## Blocked / not measured
- Container-to-host reachability, skew, and the 70 s and 300 s calls were not measured. Running a container on
  `pulso-dev` fails rootless (`crun: controller pids is not available`); the root connection runs only with
  `--cgroups=disabled --pids-limit=-1`, and from there `host.containers.internal:18080` was unreachable (URLError).
  Diagnosing the host IP seen from the machine needed `podman machine ssh`/root shell access, which was refused by the
  session safety check, so it was left for the owner. Next step for a human: find the Windows host address as seen
  from the WSL machine (default gateway of the container network), start the shim on 0.0.0.0, allow the port in the
  Windows firewall for the vEthernet/WSL profile, and run the probe with `--long`.
- Measured seconds per responder call needs the real responder (RPP/M0RP) and is out of this spike's reach.
