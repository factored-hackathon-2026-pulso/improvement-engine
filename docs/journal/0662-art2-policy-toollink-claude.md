# 0662 ART2 read-only tool link and tighten-only policy, end to end and live (UTC 2026-10-05, CLAUDE)

Branch `claude/art2-policy-toollink`. `reasoning::art2` (menu, preconditions, pass-through link compiler, monotone policy comparator), mapping rows
and `human_owned.policy` (hypothesis + deterministic tighten draft), Builder wiring (`edge_id`), suite mechanisms `tool_link` and `policy_threshold`,
dossier texts, proof outcome `needs_owner_ack`, value-loop statuses `policy_hypothesis`/`needs_owner_ack`.

Live (own stack pulso-art2, :8082, agent-core a84080e, aligned registry import, local staff admin stand-in credential): tool link on `consultas`
(`leer_pqr_cliente` at edge `consultar.ok`): base fails 6/6 finding cases, candidate passes, 14/14 GateItems, announced as an auto_detect draft with 4 changes.
Tighten-only `escalamiento-disputa-monto` 500 -> 250: base fails 6/6 window cases (251/499/500 es+pt), 6 boundary guards pass on base and candidate, 16/16 GateItems,
outcome `needs_owner_ack`, not delivered. Defects found live: exact `{id, spec}` entry-flow pin (REG-PIN until retargeted); `VersionDocs` rejects extra
fields (422); `demo_core.py` never rewrote `AGENTCORE_EVAL_DSN` nor created `agentcore_eval` (fixed, additive).
Not done: flows, agent-node link, live `GET /v1/tools`, LLM-driven end to end run (the live tests drive compile + proof with scripted proposals).
