# Real llm-gateway in the local Core stack (M1)

Pin: llm-gateway `63155b600d4b108ac871125cfd633c2beb11b0b5` (main at the time of writing; bump together with
`docs/reports/gates/facts.json`).

1. `python local/core/gateway/gen_gateway.py` writes `compose.gateway-real.yaml` and `gateway.env.example`.
2. Copy the example to the git-ignored env file, fill `GATEWAY_TOKEN_PULSO_CORE` and `ROLEPLAY_LLM_API_KEY`
   (values never go in committed files). Core gets `AGENTCORE_LLM_GATEWAY_URL=http://llm-gateway:8080` and the same token.
3. Compose with `compose.core.yaml`, `compose.gateway.yaml` and `compose.gateway-real.yaml`. Gateway and shim sit on the
   internal `pulso-gw-e0` network; profile `profiles/gw-e0.json` (no key, no egress) governs E0 data.
4. Call profile: alias `pulso-evolution-llm`, model `external-reasoning-model`, price 1/4 (the registry profile is untouched).

Fallback: if the Go image cannot be built or is absent, the `fixtures_app` contract double answers and the engine run
must list `gateway=stand-in` in `doubles[]` (`gen_gateway.gateway_double(False)`). Never claim a real gateway then.

Live smoke (alias 404 -> 200): `PULSO_GATEWAY_SMOKE=1 GATEWAY_TOKEN_PULSO_CORE=... pytest local/core/gateway/tests`;
skipped otherwise. Not run on the RAM-constrained machine (no containers).
