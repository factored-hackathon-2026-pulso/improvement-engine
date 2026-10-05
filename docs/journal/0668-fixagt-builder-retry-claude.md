# 0668 FIXAGT: transient Builder failures are retried, new-agent slug shown apart, hops json written, case type hint (Claude)

## Findings closed (from AGT1, docs/dev/AGT1_NEW_AGENT_PATH.md)
- A1 retry: one gateway 504 used to stop the Builder as `blocked/model_unavailable` and the loop then tried the next mapping candidate (`p/copiloto`, no suite generator). Now `reasoning::pipeline::ask` retries a transient `Unavailable` (`gateway_http_5xx`, `gateway_http_429`, `gateway_io` timeouts, `gateway_unreachable`) up to 3 calls in total with jittered exponential backoff (base 400 ms, `set_transient_backoff_ms` for tests), only while the role's recorded cost is under `TRANSIENT_COST_BUDGET_USD` (0.50; the repo has no other per-run cap to reuse). Each call is its own model-call record (`attempt` advances, `retries` = attempt - 1, `outcome: unavailable` then `answered`), and `metering.transient_retries` counts them for the Langfuse story metadata. These retries do not use the 2 answer-format retries.
- A1 no silent fallback: `value_loop` ends the finding on `blocked/model_unavailable` (the outage says nothing about the candidate), so the record carries the real cause and `p/copiloto` is never tried because of it. Other blocked reasons still go to the next candidate.
- A2: the finding record of a new agent carries `agent_id` (its own slug) and `donor`; `scripts/demo-loop/run.lib.ps1` shows `slug: soporte-tecnico (new_agent of new_agent:consultas, donor consultas)`.
- A3: `run_story.ps1` failed with "Argument types do not match" because `@($hops)` over a `List[object]` throws in PowerShell 7 (reproduced; `@(...)` then `ConvertTo-Json`). New `Save-Hops` copies with `ToArray()`; Pester test added.
- B3 (engine side): `announce` adds the optional `caseTypeHint` (`registry-writer::announce::case_type_hint`): a platform `case_type` dimension wins, else reason category `Tecnico` maps to `app_issue`, else no field. A platform that predates the field answers 422 and the announcement is repeated once without the hint.

## Evidence
`cargo test -j 1`: reasoning `mapping` 18/18 (2 new), pulso `value_loop` 25/25 (1 new, 1 extended), registry-writer `announce` 22/22 (2 new, 2 adapted: the shared fixture is a Tecnico cell). Pester `scripts/demo-loop` 39/39 (1 new), `scripts/integrated-rig` 24/24 (1 new). The new tests were written with the change (not shown red on their own).

## Asks
- [ASK] Tecnico -> `app_issue` is the only topic mapping (a hypothesis of which platform case type serves it); confirm or extend with the platform team. Deploy the platform first, or rely on the 422 fallback.
