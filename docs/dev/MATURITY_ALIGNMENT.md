# Maturity alignment with the platform (S21) - ALIGN1

Compares `seams/crates/maturity` with the support-platform S21 definitions (`backend/src/cc_platform/domain/ai/maturity.py`, `maturity_events.py`, `domain/cases/values.py`; platform `origin/main` 6ce2581, report `docs/reports-claude/SPA_S22_ALIGNMENT_2026-10-05.md` section 4). The platform stage strip is the platform's own record. The engine value is an "engine view": it is shown as such and never overwrites the platform stage.

## Stages and thresholds

| Step | Platform S21 rule | Engine default profile | Engine `platform_aligned` profile |
|---|---|---|---|
| names | stage 0..3 + `AgentStatus` none / ready / active | `Stage::S0..S3, Agent` + `agent_proposed` | same; `Maturity::platform_stage()` and `platform_agent_status()` give the platform names (S3 + `agent_proposed` = `ready`, `Agent` = `active`) |
| 0 to 1 | 10 cases of the type resolved by people (`resolved_cases_to_ask`) | any copilot question (`copilot_questions > 0`) | `resolved_cases >= 10` (`stage0_min_resolved`); a copilot question alone no longer lifts the type |
| 1 to 2 | 20 closed cases in which someone asked the copilot (`asked_cases_to_propose_tools`; counts cases, the platform keeps no question text) | the same question repeated over >= 20 cases (`repeat_q_cases`) | `copilot_cases >= 20` (`stage1_basis = cases_with_questions`); `repeat_q_cases` is ignored |
| 2 to 3 | tool used in >= 70 % of the closed cases where tools were proposed, at least 10 such cases | `tool_use_min` 0.7, `k_min` 10 | identical (already matched) |
| 3 to agent | >= 80 of the last 100 decided drafts sent as is or with minor edit (<= 150 permille edit distance) | `draft_accept_min` 0.8, window 100, accepted = not `discarded` | identical numbers; the platform `edited` outcome (change larger than minor) maps to `discarded` (not accepted) |

Signals differ in scope too: the platform counts them since the type reached its current stage; the engine uses the aggregate window of its input. The numbers can therefore differ even with the same rule.

## Using the profile

- Code: `Thresholds::platform_aligned()`; `Thresholds::default()` is unchanged (its JSON keeps the historical shape: no `profile`, `stage0_min_resolved` or `stage1_basis` key).
- JSON (`thresholds_from_json`): `{"profile": "platform_aligned"}` (or `"default"`), then any other key overlays it; `stage0_min_resolved` (integer >= 1) and `stage1_basis` (`repeated_questions` | `cases_with_questions`) can also be set alone. `thresholds_to_json` of the aligned profile round-trips.
- Input rows (`JsonSource`): new aggregate fields `resolved_cases` and `copilot_cases` (counts only); draft values are `as_is | minor | discarded | edited` and `edited` is read as `discarded`. A missing count is `not_computable` (`no_resolved_cases`, `no_copilot_cases`), never a guess; the stage then waits with `blocked_by`.
- Output (aligned only): `metrics.resolved_cases`, `thresholds.stage0_to_1`, and `thresholds.stage1_to_2.basis`. `repeat_q` then carries the cases-with-questions count.
- Tests: `seams/crates/maturity/tests/platform_aligned.rs` (one test per difference; the default profile is pinned).

## Case-type ids we need from the platform

The platform case types (`CaseType`, `domain/cases/values.py`; `none` never matures and is never sent). Our input rows must use these ids as `type_id`, otherwise the supervisor panorama cannot match them. The engine fixtures and the demo use Spanish aliases; the mapping to apply in the source adapter is:

| Platform id | Label (es) | Engine fixture alias |
|---|---|---|
| `unrecognized_charge` | Cargo no reconocido | `cargo_no_reconocido` |
| `undue_charge` | Cobro indebido | `cobro_indebido` |
| `app_issue` | Problema con app | `problema_app` |
| `branch_service` | Atencion en sucursal | `atencion_sucursal` |
| `service_quality` | Calidad de servicio | `calidad_servicio` |
| `virtual_card` | Tarjeta virtual (team-generated, not a dataset subcategory) | `tarjeta_virtual` |

What the engine needs the platform to provide per id (aggregates only, no ids or text): cases resolved by people (`resolved_cases`), closed cases with copilot questions (`copilot_cases`), closed cases where tools were proposed and where one was used (`tool_applicable`, `tool_used`), and the last decided drafts as `as_is` / `minor` / `edited` / `discarded` (events `copilot.tool_used`, `ai.stage_advanced`, `ai.agent_ready`, `ai.agent_activated`; `entity_id` is the case type id). Open question for the platform team: which endpoint or export exposes `StageSignals` per type (it is persisted on `case_type_maturity`).

## Decision pending

Whether the engine default should move to the aligned rule (0 to 1 on 10 resolved cases) is the user's call; for now only the opt-in profile exists.
