# agent_roleplay shim: responder runbook

Status label: `model=agent_roleplay`, plumbing only, `quality_claims: forbidden`.

## Path
Core `HttpLLMGateway` -> real llm-gateway `POST /v1/generate` -> alias endpoint `POST /v1/chat/completions`
= this shim (`python -m roleplay_llm --queue <dir> --port 8640`). Point the gateway `LLM_ENDPOINTS` entry
`pulso-evolution-llm` at it (dummy `api_key_env`). The shim echoes the requested model string.

## Queue directory
- `requests/<key>.json`: written by the shim only after the treated-payload scanner (TPS) passed. Inputs are
  normalised (run, turn, session ids, uuids, binding/job/artifact ids replaced by ordinals).
- `responses/<key>.json`: written by a responder, atomically (write `.tmp`, rename).
- `ledger.jsonl`: events (queued, answered, scanner_rejected, responder_timeout, replay_miss, fault).
- `faults/next.json` or `faults/<key>.json`: scripted fault `{"status":502,"type":"invalid_output"}`, consumed once.
  Faults come only from this channel, never from a responder.

## Responder contract (one fresh-context subagent per request or stage batch)
1. Read only the one `requests/<key>.json`. Never the repo, other queue files, or the network.
2. Decide ONE step: a tool call or a final answer, short output.
3. Write only `responses/<key>.json`:
   `{"protocol":"roleplay-queue/1","key":"<key>","provenance":"agent_roleplay","quality_claims":"forbidden",
   "responder":{"id":"...","role":"scout|verifier|builder"},
   "content":{"kind":"tool_call","tool":"pulso/lab_query@1.0.0","args":{...}}}` or
   `"content":{"kind":"final","output":{...}}`.
   Any other field, a quality claim or a wrong label is rejected as `invalid_output` (file renamed `.rejected.json`).
4. Scout, verifier and builder use distinct responders and different model identities; none is the
   implementer or reviewer of the code under test (`roleplay_llm.protocol.check_assignments`).
5. Step caps: scout 5, verifier 5, builder 7 (`check_step_budget`). One agent slot at a time.

## Latency and budget
The shim holds a call 55 s, then returns typed `504 responder_timeout`; a late answer in `responses/` serves the
retry instantly. Record per live run: responder calls and wall minutes (to the run report).

## Replay
`--replay-only` serves only recorded `responses/`; a miss is `409 replay_miss` with the diff of normalised inputs
against the recorded request of the same stage and step. Raw-E0-derived queues stay in a git-ignored directory.

## Tests
`cd roleplay-llm && python -m unittest discover -s tests`
