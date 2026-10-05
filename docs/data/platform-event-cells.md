# Platform event cells (EVT1): what advisors and our own agents DO on the platform

Producer: `scripts/aggregate/platform_event_cells.py` (stdlib only). Consumer: the metric-generic cells sensor, `steps_cli cells_platform` (`steps::cells::run_platform`, `Config::platform()`). Mapping rows: `seams/crates/reasoning/fixtures/mapping_table.json`. Synthetic history: `scripts/aggregate/platform_event_synth.py` (platform-sim backbone), live run: `scripts/aggregate/platform_event_live.py`. Sources: `docs/reports-claude/PLATFORM_SIGNALS_2026-10-05.md` (4a, 4b, 4e), `DETECTION_GAP_ANALYSIS_2026-10-05.md` (item 8), platform `docs/platform/api/engine-signals.md`, contract 1.3.0 (PR 114).

Everything is an association over aggregates: no ids, no free text, k >= 10 on every published count, `claim: association`, never a cause.

## Families (every rate is "higher = worse": the sensor flags only an upward difference)

| Metric | Family | Numerator / denominator | Signatures (dims) | Platform events read |
|---|---|---|---|---|
| `P_DRAFT_REJECT` | draft_acceptance_rate (published as its complement: acceptance = 1 - rate) | reply drafts `discarded` or `ignored` / drafts decided `used`, `edited`, `discarded`, `ignored` (subject `reply`; `escalation` and `accepted` are a named discard) | case_type x channel; language x channel; release x agent | `copilot.suggestion_decided` |
| `P_DRAFT_HEAVY_EDIT` | draft_edit_distance bucket | decided `used`/`edited` with `edit_distance_permille` >= 500 (heavy) / all decided `used`/`edited` with a distance. Bucket edges fixed in the aggregator: light < 200, medium 200..499, heavy >= 500; only the heavy share is published (aggregate only, no per-draft value leaves) | same three | `copilot.suggestion_decided` |
| `P_SUGG_NONE` | suggestion_none_rate | `copilot.suggestion_none` / (`ready` + `none`) | same three | `copilot.suggestion_ready`, `copilot.suggestion_none` |
| `P_SUGG_FAILED` | suggestion_failed_rate (LEVEL RISK) | `copilot.suggestion_failed` / (`ready` + `none` + `failed`) | channel x language | `copilot.suggestion_*` |
| `P_TOOL_USE` | copilot tool_used mix | cases where the analyst used tool T at least once / cases with copilot activity | case_type x tool; channel x tool | `copilot.tool_used`, `copilot.suggestion_*` |
| `P_TYPE_REASSIGN` | case type reassignment rate | cases whose type was changed AWAY FROM A REAL TYPE / cases that had a real type. The first `none -> T` labelling is not a reassignment. The case_type dim is the type before the correction | case_type x channel; language x channel | `case.type_changed`, `cases.case_type` |
| `P_ASSIST_ESCALATION` | assistant escalation rate | `assistant.ended` result `escalated` / `assistant.ended` with a result | case_type x channel; language x channel; release x agent (release and agent from the case's `assistant.turn_answered`; `assistant.ended` has none) | `assistant.ended`, `assistant.turn_answered` |

Dim values: `case_type` (contract CASE_TYPES), `channel` (the 1.1.0 names `chat_app`/`chat_web` are folded into `app_chat`/`web_chat`), `language` (es/pt), `release` and `agent` (registry release id, AI agent id such as `copiloto-asesor@1.0.0`; never a person), `tool` (tool name). `analyst_id`, `run_id`, `trace_id`, `failure_code`, `turn_id`, case and customer ids are never read into a cell.

## Privacy rules (same family as `bank-cells-metrics.md`, DET1 reused)

- k: a row is published only when `denominator >= 10` and numerator and complement are each 0 or >= 10 (`bank_cells.k_ok`). One split (`bank_cells.split_half`, SHA-256 low bit) over the CUSTOMER (cases of one customer stay in one half; the customer id is used only to hash, never emitted).
- Periods: `ALL` (full period), `W1`, `W2` (first and second half of the export span, the aggregator's own R2 windows). No month rows, so there is no per-month leak surface. The sensor takes the half totals from `ALL` and R2 from `W1`/`W2`; the pooled support floor (`min_support`, platform profile 200) is applied to the POOLED discovery + holdout support (DET1).
- No margins: no row without both dims of its signature is ever emitted, so a withheld cell cannot be read off a published total. Hierarchy as in DET1: ALL withheld => both windows withheld; one window withheld => the other withheld.
- Cross-signature differencing (new here). The signatures of one metric are partitions of the same population, so they share marginals (the channel total, the grand total) and a reader could subtract a fully published signature from a partly withheld one. The aggregator therefore runs a differencing attacker (`find_leaks`: row space of the partition identities restricted to the withheld cells; every withheld cell and every sum of up to 3 withheld cells whose determined value fails k) and repairs each unsafe determined combination by withholding ONE more published cell (the smallest of the finest identity touching it), until none is left. Recovering a k-safe value is not a leak. Tests: `scripts/aggregate/tests/test_platform_event_suppression.py` run an independent attacker (sums of up to 4) over random tables (250 + 30 seeds), plus mutation checks that the attacker DOES find leaks when either rule is switched off.
- Named discards (counted, and published as null when the count is below k): `decision_not_a_reply_draft`, `edit_distance_missing`, `dimension_unknown:<metric>` (an event without release/agent is dropped from release signatures, never pooled into an `unknown` cell), `unsafe_payload_value:<type>:<key>` (a payload value that is not a bounded token or looks like a platform id), `event_without_case`, `event_case_not_in_cases`, `no_case_dimension`, `case_without_time`, `type_never_real`, `assistant_result_missing`, `duplicate_sequence`, `bad_event_time`. The sensor adds `k_violation`, `no_baseline`, `below_min_support`, `favourable_direction`, `level_below_min_support`, `holdout_without_discovery`.
- Hard gate: `assert_clean` raises (never silently filters) if a row has other than the six fields, a dim outside the closed list, an unsafe value, or breaks k.
- Honest limits: a cell with no event at all is indistinguishable from a withheld one; the attacker check covers sums of up to 3 withheld cells inside one metric x half (4 in the tests), not arbitrary subset sums; no formal differential privacy.

## Sensor (`Config::platform()`)

Same strict tests as the bank profile (two-proportion z, BH q 0.01 over ALL explored cells, 5 pp and ratio 1.25 floors, discovery/holdout replication, R2 windows, named discards) with three platform-specific rules:

1. Same-channel baseline by contrast dimension. A cell drops its first present dimension of `case_type > release > tool > agent > language` to form its comparison stratum: `case_type x channel` is compared with the same channel excluding its own case type (published full-period cells, minus the cell); `release x agent` with the other releases of the SAME agent; `case_type x tool` with the same tool in other case types; `language x channel` with the same channel excluding its own language. Bank and E0 tables are untouched (`platform: false`).
2. One comparison group per dimension signature. A metric published under several signatures describes the same population more than once; the signatures never mix in a baseline or in a level pool.
3. Level risk, pre-registered BEFORE data: `P_SUGG_FAILED` pooled failure rate above 0.05 (analyst policy parameter, not a legal threshold), minimum excess 0.03, one-sided z, Wilson 95% interval, holdout and R2 windows, Bonferroni over the registered family (1). The default (bank) config keeps its single M8 level test.

Findings of `P_SUGG_FAILED` as a contrast and level signals are never proposals (see the mapping): operational, human owned.

## Mapping rows (data, `mapping_table.json`)

| Finding | Row | Target | Mechanism | Outcome |
|---|---|---|---|---|
| `P_DRAFT_REJECT` with `case_type` | `copilot_low_acceptance` | `prompt:p/copiloto` (copiloto-asesor) | wording | candidate, proof support none (no suite generator for the copilot prompt) |
| `P_DRAFT_REJECT` with `release` | `copilot_release_regression` | `prompt:p/copiloto` | wording | candidate, same caveats |
| `P_DRAFT_HEAVY_EDIT` with `case_type` | `copilot_heavy_edits` | `prompt:p/copiloto` | wording | candidate |
| `P_SUGG_NONE` with `case_type` | `copilot_suggestion_none` | `prompt:p/copiloto` | uncovered_topic (knowledge gap hypothesis); caveat `tool_link_or_knowledge_source_is_a_human_decision` (tools are not an engine target) | candidate |
| `P_TOOL_USE` | `copilot_tool_mix` | `prompt:p/copiloto` | repeated_lookup | candidate |
| `P_ASSIST_ESCALATION` on `unrecognized_charge` / `undue_charge` | `assistant_escalation_dispute` | `template:t/aclarar_cargo` (disputas) | wording | candidate; other case types stay descriptive (no covering agent) |
| `P_TYPE_REASSIGN` | `case_type_taxonomy` | none | none | `human_owned` (owner: supervision) |
| `P_SUGG_FAILED` | `copilot_failures_platform` | none | none | `human_owned` (owner: plataforma) |

A language `pt` cell is human owned by the existing calibration entry (`calibration_pt_thresholds`), which is checked first. Every row carries `mapping_is_a_hypothesis_of_where_to_intervene`; none claims a cause. Findings carry `Source::Synthetic` for the synthetic history and `Source::PlatformTreated` (new, derived) for the real feed, so the derived-data opt-in applies to the latter.

## Data

- SYNTHETIC (this PR): `platform_event_synth.py` drives the engine's `platform-sim/platform_live` simulator (11-table platform model, language assignment, SLA, event_log with contiguous sequence) and emits the 1.3.0 events with the platform's published payload keys. Case types are the platform seed's complaint subcategories. Effects are planted and documented in the module (independent of the sensor); controls (language, release on rejection, `branch_service`) are flat. The history is read out through the EXPORTER's own access policy (`policy.select_sql` plus the engine-level SQLite authorizer): no column outside `ALLOWED_COLUMNS` can be read. Output is labelled `synthetic: true` (`SYNTHETIC.json`); nothing is committed.
- The data-lab `platform_sample` generator was inspected and NOT used: it builds E0 disputes from the bank warehouse (stage E0, no suggestions, no copilot decisions) and needs the real bronze data.
- REAL FEED, documented plan (not built here):
  1. Source of truth stays `platform-exporter`: `event_log` (sequence, event_type, case_id, event_time, payload) and `cases` (id, customer_id, channel, language, case_type, opened_at) through the allow-list; contract 1.3.0 already admits the 12 event types and `cases.case_type`. Free text is dropped client-side per type; the aggregator re-checks every payload key and value (allow-list, closed enums, bounded tokens, id-prefix deny list).
  2. Hand-off: the aggregator reads two ndjson files shaped like the synthetic ones. A package writer (monitor tick) must emit them with payload keys kept (the events sensor package carries no payload) and `cases.ndjson` extended with `case_type`, `opened_at` and a hashed customer key. NOT built (another lane's crate).
  3. Cadence and volume: a weekly rolling window; a cell is only tested when the pooled support reaches 200 decisions (cases for the case-level families), so a pilot with a few dozen decisions per day needs several weeks per cell. Below that every cell is a named `below_min_support` or `k_violation` discard, never a zero.
  4. `release` is null on events written before platform PR (engine-signals) and on `assistant.ended`: those rows are dropped from release signatures (named discard), not folded into an `unknown` release.
  5. AI-off periods (`platform.ai_toggled`) are not yet used to exclude intervals; a long off period would show as missing suggestions, not as a rate.

## Live run on the synthetic history (2026-10-05)

`python scripts/aggregate/platform_event_live.py --steps-cli <steps_cli> --out <dir> --cases 24000 --seed 7` (SYNTHETIC; 281,759 events, 909 cell rows, 180 cells explored, built `-j 1`): 13 corroborated findings, all on planted cells, none elsewhere. Lit: `P_DRAFT_REJECT` (service_quality in all 4 channels, 0.68-0.71 vs same-channel 0.37-0.42), `P_DRAFT_HEAVY_EDIT` (unrecognized_charge x phone_inbound 0.55 vs 0.21), `P_SUGG_NONE` (app_issue x web_chat 0.28 vs 0.06), `P_TOOL_USE` (undue_charge x consultar_cargos 0.67 vs 0.29), `P_TYPE_REASSIGN` (undue_charge in 4 channels 0.34-0.36 vs 0.03-0.04), `P_ASSIST_ESCALATION` (virtual_card in 2 chat channels 0.37-0.48 vs 0.10); level risk `P_SUGG_FAILED` corroborated (0.089, Wilson 0.083-0.095 vs the registered 0.05). Flat controls (language, release rel-a vs rel-b on rejection, branch_service) produce no finding; two `uncertain` cells (not significant after correction) are listed, not findings. Seed 11 repeats it. With 8,000 cases only 3 of the 6 contrast families light up (thin cells fall under the pooled floor or k): the families are volume gated, as designed.

## Not done

Month rows and per-month counts for these families; `platform.ai_toggled` interval exclusion; time-to-decision; escalation `reason_code` families (platform ask A5); handoff-quality labels (ask A4); wiring the findings of these families into `monitor::tick` / `pulso run` (the cells report is consumed like the bank one through `Finding::from_report`, but no run was made through the reasoning roles here); a regression-suite generator for `prompt:p/copiloto` (all new rows are proof support `none`, not announceable now).
