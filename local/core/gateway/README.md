# local/core/gateway (M1)

Real llm-gateway overlay for the local Core stack, pinned to llm-gateway 63155b6. Alias `pulso-evolution-llm`
routes to the roleplay shim; spend is guarded in core-bridge (`llm/guard.py`, M2a). Steps: see
[`RUNBOOK.md`](RUNBOOK.md). `gen_gateway.py` writes the overlay and env example; `profile_check.py` validates
`profiles/*.json` (`gw-e0`: no key, no egress; `gw-hosted`).

Tests (verified; the live smoke is skipped unless `PULSO_GATEWAY_SMOKE=1`):

    uv run --python 3.12 --with pytest --with pyyaml python -m pytest local/core/gateway/tests -q -p no:cacheprovider

Data-class rules: secrets only by env reference, never in committed files; E0-class data is allowed only on the
no-egress profile. Honesty: without the Go image, list `gateway=stand-in` in `doubles[]`.
Owner lane: L-MODEL.
