# Local agent-core stack and the `pulso-builder` agent

A LOCAL, non-shared instance of agent-core (head `f91ac44`, includes PR 34) plus llm-gateway, used to prove the
Builder path end to end: engine-signed `builder` principal -> `POST /v1/runs` -> task agent `pulso-builder` (ReAct
`agent` node on `xiaomi/mimo-v2.6-flash`) -> in-process registry tools as `constructor-bot` -> a validated
draft proposal. Only SYNTHETIC input is used. This is our own instance, not the shared Core.

## Prerequisites

- Windows, Podman with a machine named `pulso-dev` (connection `pulso-dev-root`, override with
  `PULSO_PODMAN_CONNECTION`), `uv`, `git`, Python 3.11+. No Go and no cargo needed.
- Env files with the dev values: `agent-core.env` and `llm-gateway.env` (default location: the folder two levels above
  the worktree; override with `PULSO_AGENT_CORE_ENV` / `PULSO_LLM_GATEWAY_ENV`). Values are loaded into child-process
  environments only. They are never printed, logged, written to another file or passed on a command line. Do not
  commit them or copy them into AWS.
- Optional: `PULSO_AGENT_CORE_DIR` / `PULSO_LLM_GATEWAY_DIR` to reuse existing clones; otherwise the script clones
  both repos into `.dev-stack/src/` (gitignored).

## Commands

```
python scripts/dev-stack/stack.py up            # postgres 16 :55432, llm-gateway :8080, migrate, import, serve :8001
python scripts/dev-stack/stack.py health
python scripts/dev-stack/invoke_builder.py      # POST /v1/runs with synthetic input; prints only end.output_map
python scripts/dev-stack/stack.py down [--purge]
```

`up --reset-db` is needed after changing any entity under `scripts/dev-stack/registry-pulso-builder/` (registry
versions are immutable by hash). The first `up` builds the gateway image (heavy: run nothing else meanwhile).

What `up` does: Postgres container (`pulso-l3-postgres`, DBs `agentcore` and `agentcore_eval`); `podman build` of the
llm-gateway Dockerfile and run (`pulso-l3-llm-gateway`); `uv sync`, `agentcore migrate`; `identity.py` writes public
keys (identity-keys, staff-keys) and mints two 12 h tokens into `.dev-stack/tokens.json` (gitignored): `admin` for
the registry import and `builder`, signed by a locally generated engine Ed25519 key (`kid pulso-engine-dev-1`);
`agentcore registry import` of `registry-pulso-builder/`; `agentcore serve --registry-api --port 8001` with the
file-driven field classifier (`field-overlay.json`, pulso fields public) as a detached process (log in
`.dev-stack/serve.log`).

## The agent

`registry-pulso-builder/`: agent `pulso-builder@1.0.0` (`mode: task`, `invocable_by: [builder]`, inputs `agente`,
`objetivo`, `evidencia`), flow `pulso-construir@1.0.0` (agent node reading `registry/get_entity` and
`registry/list_versions` -> `create_proposal` -> verify -> `put_draft` -> verify -> `validate` -> `end completed`),
prompt `p/pulso-builder@1.0.0`, model profile `pulso-mimo-flash@1.0.0` (prices from the OpenRouter list).
`end.output_map` returns only `proposal_id`, `rev` (rev at creation, the stored draft is rev 1) and `valid`.

## Findings (all verified live on this stack)

1. The full path works: 3 of 3 runs on the final config returned `completed` with a non-empty, token-free,
   Core-validated draft (a `prompt` change 1.0.1). Each run takes about 15 to 40 s (two to three model calls).
2. `AGENTCORE_LLM_GATEWAY_TOKEN` in `agent-core.env` does NOT equal `GATEWAY_TOKEN_AGENT_CORE` (HTTP 401 from the
   local gateway), and `llm-gateway.env` has that token blank. The script feeds the local gateway the token from
   `agent-core.env` and points the Core at the same pair, in memory. The shared gateway token is a different value.
3. Prompts cannot contain braces (rule G0-01, "a prompt has no variables"), so the step protocol must be described
   in words. Without a very explicit wrapper instruction the model answered the bare draft and the gateway returned
   `invalid_output` (502) because the agent step schema is `kind` + `output`.
4. Tool results go through the field classifier. With only the pulso overlay, registry fields (`id`, `spec`,
   `model_profile`) were tokenised as PII (`pii:2`) and the model copied the tokens into the draft; Core validation
   caught it (`REG-SCHEMA`). Marking registry structural fields public in the overlay fixed it. De-tokenisation only
   happens in tool-call args, never in the agent's final output.
5. An empty `changes` list is a valid draft (`valid: true`): treat `valid` plus a non-empty change list as success.
6. The agent node's output and `put_draft` result cannot be read in `output_map` (G0-22), so evidence of what was
   drafted is read back through the registry API (`GET /v1/registry/proposals/{id}`), as an admin/builder.
7. Quota: 10 proposals per 24 h. The serve process exited once without a log line during an invocation; a re-`up`
   recovered it (cause unknown).
8. `AGENTCORE_ALLOW_DEMO=1` makes tools, authz, transcript and calibration demo doubles (warning at startup). That
   is acceptable locally and must never reach production.
9. Podman rootful and rootless on this machine reject containers without `--pids-limit=0` (cgroup `pids`
   controller missing): the script passes it.
