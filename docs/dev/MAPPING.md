# MAP1: where the engine intervenes (the mapping table)

A finding is a cell: a metric, dimensions (reason x channel), a direction. The mapping says which real artifacts of agent-core a change could target for that cell, in the order the engine tries them. It is DATA, not code: `seams/crates/reasoning/fixtures/mapping_table.json` (label `hypothesis_table`), loaded and validated by `reasoning::mapping::Table`.

## What a mapping is, and is not

A mapping is a HYPOTHESIS of where to intervene. It is not a cause: the cells are associations, a contact is not linkable to a complaint (BANK_DATA_AUDIT), and the table says WHERE the problem sits, not WHY. This is written in the table's `notice` (en, es, pt), in the mandatory caveat `mapping_is_a_hypothesis_of_where_to_intervene` of every row (the loader refuses a row without it), in the Scout input (`mapping_claim`), in the proposal docs and `expected_effect.mapping.claim`, in the dossier risk section ("Hipotesis de donde intervenir, no una causa (candidato 1 de 5)") and on every record of the loop (`mapping_claim`).

## Shape of the table

* `topics`: what an existing agent covers (`covered_by`, empty = uncovered topic), with a `why` and an `evidence` pointer to the anatomy report.
* `rows`: `match` (metric list, `dims` that must equal one of the listed values, `dims_present`), `topic`, `link_grade` (`mechanism_proxy | unlinked`, never `same_outcome_linked` from aggregates), `guardrail`, `caveats`, and ranked `candidates`.
* a candidate: `rank` (1..n, contiguous), `target_ref` (`template:`, `prompt:` or `new_agent:`), `kind` (`patch | new_agent` only), `agent`, `placeholders`, `slugs` (new agent), `mechanisms`, `justification` (finding cell -> topic -> the existing agent or artifact whose scope covers it), `evidence` (anatomy section), `proof_support` (`suite:<mechanism>` = a regression suite generator exists in `scripts/regression/build_suite.py`, or `none`), `announceable_now`, optional `when_dims` (e.g. the advisor copilot only for Phone).
* `human_owned`: findings a person owns (see below).

Not representable, by construction (the loader refuses them): policy values, interrupts, injection rulesets, language detection, tools, `release_settings`, `constructor-chat` and `pulso-*` agents, model profiles, any kind other than `patch` and `new_agent`. A test pins that `proof_support` and `announceable_now` agree with the real suite generator.

## Order of candidates

1. The user's proposal priority (new agents, prompt changes, links to existing tools, policies, flows) is applied with one correction from the evidence: when the topic is covered by an existing agent, a patch of that agent's artifact comes before a new agent; a new agent is proposed first only for an UNCOVERED topic (Tecnico, Comercial, Retencion).
2. Among candidates, the ones that can be proven and announced NOW (`proof_support != none` and `announceable_now`, or a new agent when `PULSO_NEW_AGENT_ADMIN=1` says a human admin credential exists) go first; ties keep the table rank (`Row::ordered`).
3. At most `MAX_CANDIDATES = 2` are tried per finding.

For Queja (M1, any channel) the candidates are: 1 `template:t/estado_pqr` (consultas: the status answer is one static sentence that never reads the PQR state), 2 `prompt:p/resumen_radicado` (disputas: the closing message does not say who follows up), 3 `template:t/aclarar_cargo` (no suite generator), 4 `prompt:p/copiloto` (Phone only, advisor path, no suite generator), 5 `new_agent:consultas` slug `intake-quejas` (last resort). For Tecnico, Comercial, Retencion: 1 the new specialist (uncovered topic), 2 `prompt:p/copiloto` on Phone (no suite generator). M4/M5 by category: `t/estado_pqr`. E1 dispute: `p/copiloto`.

## The loop per finding

`pulso::run::value_loop`: for each corroborated finding the engine computes `candidate_plan`; it runs the roles restricted to ONE candidate (the Scout sees one `allowed_target`), proves the proposal with the existing W11 flow (regression suite fails on the base, passes on the candidate), and stops at the first PROVEN candidate. A candidate that is blocked or not proven moves to the next one (cap 2). The record of the finding is the proven attempt, else the first attempt that produced a proposal; it also has:

* `candidates`: the whole ranked list with `justification`, `evidence`, `proof_support`, `announceable_now`, `tried`, `not_tried_because` (`over_the_candidate_cap`, `an_earlier_candidate_was_proven_or_the_finding_stopped`);
* `attempts`: per tried candidate `target_ref`, `status`, `outcome`, `proof` verdict, `delivery`, `cost_usd`;
* `metering.cost_usd`: the sum over every attempt (cost is accounted, not only the winner's).

Nothing is approved, published or promoted.

## Human-owned findings

`human_owned` entries are checked BEFORE the rows: `policy_dispute_amount` (dims `reason_code = policy:escalamiento-disputa-monto`; the registry holds 500 USD, the team document 250 USD; owner riesgo) and `calibration_pt_thresholds` (dims `language = pt`; calibration thresholds exist only for es and are not a registry entity; owner plataforma). The finding ends `human_owned` with a short ES/PT note for a person; no model is called and the Builder proposes nothing. Metric M8 (consent) keeps its older `unlinked` reason `level_risk_human_owned`.

## Extending

Add a row or a candidate to the JSON; `cargo test -p reasoning --test mapping` validates it (ranks, kinds, mechanisms, deny-list, catalogue grounding, agreement with the suite generator). A new patch target also needs its anchors and protected markers in `catalog.rs` and, to be announceable, a mechanism in `scripts/regression/build_suite.py`.
