# AGT1: the new-agent path, live with a person deciding (2026-10-05)

Own rig `agt1` (`PULSO_STACK_PREFIX=agt1`, ports `PULSO_RIG_{PG,GW,CORE,ENGINE,PLATFORM,SPA}_PORT`), agent-core main (PR 65), platform PR 32 branch (dossier, Pasar a produccion), prebuilt engine of ART3. Profile `run_story.ps1 -Profile newagent -Cycle` (planted SYNTHETIC M1 Tecnico/Phone cell, `planted_cells.py --profile uncovered-topic`).

## What worked (a person clicked; the rig only watched)
Loop 80 s: finding M1 x Tecnico -> `new_agent:consultas` (clone `soporte-tecnico`, `inherit_from` the donor prod release), suite proven (`regression_suite_proven`, base absent fails 6/6), announced as `auto_detect` draft (16 changes incl. `release_settings {inherit_from}` and eval_suite), 22/22 platform checks (4 notifications, list source engine, dossier). The person evaluated, approved, published (staging) and activated (prod) in the SPA. Read-only facts afterwards: prod alias = this proposal's release, release carries interrupt fraude, lang-es-pt, injection-rules, max_input_chars 4000 (inherited), platform case type `undue_charge` (Cobro indebido) records agent soporte-tecnico.

## Human steps
SPA http://127.0.0.1:<SPA port>; Lucia Herrera / demo1234 / step-up 000000. Automatizacion > Propuestas > open the proposal; Probar; Aprobar; Publicar (staging); open it with `?type=Cobro%20indebido` (the only seeded type `ready`) > Activar agente (promotes prod, records the agent on the type).

## Breaks found
Engine (fixed here): (1) approver-facing `human_items` still said an admin must write fraude/injection settings (stale since INH1); (2) proposal title read `new_agent:consultas` (the donor) instead of the new agent; (3) rig had no per-lane ports/prefix, no new-agent profile, no post-human read-only activation facts.
Engine (open): one Builder gateway timeout (504, 60 s) blocks the candidate (`blocked/model_unavailable`, no retry) and the loop falls to `p/copiloto` (no suite generator): rerun worked. Loop record shows `slug: consultas` (donor) for a new agent. `run_story.ps1` ends with "Argument types do not match" after the hops table (hops json not written).
Platform asks: (a) the engine finding's topic (reason_category Tecnico) has no link to a platform case type; only "Cobro indebido" is ready, so the demo activates a tecnico agent for a dispute type; (b) `ReleaseSettingChange` drops agent-core's `inherited`/`inherited_from` (PR 52) so the approver does not see which values are inherited; (c) ADR 0009 PRs 30/31 (routing display, `agent_not_routable`) are open: activation does not change what customers get.
agent-core asks: none blocking.

## Routing check (customer simulator) - NOT conclusive
`/customer` simulator chats reach the assistant only for customers linked to a dataset customer (`CC_BANK_CUSTOMER_LINKS_FILE`); the synthetic seed has none, so every chat went to people. With synthetic links added, a `recepcion` run was created in agent-core (awaiting slot problema), but the follow-up turn failed `unavailable` because my agent-core process had died (rig restarts); no reply from soporte-tecnico was obtained. Also `recepcion` routes through the directory by routing card only; the rig serves registry-e2e agents (recepcion included) and tools are demo doubles.
