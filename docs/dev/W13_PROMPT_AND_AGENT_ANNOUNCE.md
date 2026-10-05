# W13 prompt patches and new agents become announceable (plan W1-1 extension)

Builds on `W11_EVALUATE_BEFORE_ANNOUNCE.md` (evaluate before announce) and `REGRESSION_SUITES.md`. Branch
`claude/w13-prompt-and-agent-announce`, no PR (nothing here depends on a merge).

## What changed

* Prompt patch. agent-core `evaluate` ignored candidate prompts. agent-core PR 50 (https://github.com/pulso-factored/agent-core/pull/50,
  merged only locally on `scratch/w13-pr50-local`, never pushed) makes it read them. The proof therefore runs a CONTROL: a text-identical
  version bump of the base prompt evaluated natively. `native_binding` = `candidate_bound` (the control did not change the native result,
  the candidate is read) or `native_not_candidate_bound` (evaluate ignores candidate prompts: the harness wording probe decides alone,
  the story is labelled and NOT announced) or `unknown`. `scripts/regression/sample_probes.py` (the only script with network) collects 3
  real model samples of the prompt text under test from the local llm-gateway; `judge_story.py` judges them offline. A probe without
  samples is "not measured" and its case fails: a prompt is never announced on a probe nobody ran.
* Coverage. The verdict story and the dossier now carry `coverage`: `native`, `harness_probe`, `not_measured`, `assumptions`, rendered in
  ES/PT in the dossier section "Que se midio".
* New agent. `build_suite.py` generates the suite for a `new_agent:<x>` target; the suite runs on the NEW agent itself. The base has no
  such agent: `verdict: absent`, its finding cases fail by absence (not measured). `registry_writer::closure` adds the FULL donor closure
  (templates, decision model, tools, language detection, injection ruleset; REG-PIN fails without it) to the draft. The donor's release
  settings (fraude interrupt, injection ruleset, lang-es-pt, max_input_chars) are NOT written by the engine (INH1, below): both the
  evaluation drafts and the announced proposal carry `release_settings: {inherit_from: <donor release id>}`. The old way (explicit
  settings, evaluation only, labelled `release_settings_assumed`) survives only as a fallback for an older Core when the operator
  configured an explicit admin credential (`Config::admin_settings_fallback`). The recepcion routing change cannot be in the same
  proposal: human-owned follow-up, stated as not measured (`routing_recepcion_to_new_agent`, `traffic_stealing`).

## Live results (2026-10-05, own stack `pulso-w13`: agent-core + PR 50 local on :8203, gateway :8213, engine builder principal)

`tests/live_w13.rs` (3 tests, `#[ignore]`, run one by one with `--test-threads 1`; credentials only in the process environment).

| case | result |
|---|---|
| (i) prompt patch `p/resumen_radicado` (closing follow-up sentence) | ANNOUNCED. `regression_suite_proven`, `native_binding: candidate_bound`, 6/6 finding cases fail on the base and pass on the candidate, guards pass, 13/13 GateItems. Real samples: base "...Te mantendremos informado..." vs candidate "Un especialista ... tomara el seguimiento". Delivered as auto_detect draft with 2 changes (patch + eval_suite). 176 s. |
| (ii) text-identical prompt (no-op candidate) | NOT announced: `not_announced:not_fixed`. `candidate_bound`; native evaluate passes (as for any prompt), the wording probe fails 6/6: the harness probe is what separates a patch from a no-op. 241 s. |
| (iii) NEW agent `soporte-tecnico` (clone closure of the donor) | ANNOUNCED. Proven on itself, all finding cases and guards pass natively, base by absence. Delivered as auto_detect draft: 14 changes (agent, flow, 9 templates, decision_model, language_detection, eval_suite), NO release_settings. 9 s. |

### Blocker found at W13 (iii), resolved by INH1

agent-core `put_draft` refuses release `interrupts` unless the actor has the `admin` role (`forbidden_role`). The W13 live run used the local
staff admin credential as a labelled stand-in for the evaluation drafts; without it a new-agent proof was `not_announced:infra_failed`. INH1
removes the need (next section).

## INH1: the clone inherits the donor settings by server-side reference (2026-10-05)

agent-core PR 51 (`feat/constructor-release-settings-eval-drafts`) adds an optional `release_settings.inherit_from: <release_id>`: for an agent
with NO base the server copies the donor release's interrupts, language detection, injection ruleset and `max_input_chars` into the candidate.
The caller never writes those values, so nothing can be removed, weakened, re-prioritised or forged, and it needs no admin; explicit
`interrupts` still do, REG-LOCKED applies against the donor, and approve/publish/promote still need the platform human with step-up.

* The donor release id is read live: `GET /v1/registry/aliases/consultas/prod` (then `staging`), active release only. No published donor
  release fails closed (`not_announced:suite_error`, reason `donor_without_published_release`) before any draft is opened. Two read-only routes
  were added to the allow-list (`GET aliases/{agent}/{alias}`, `GET releases/{id}`).
* `registry_writer::closure` puts `release_settings {inherit_from}` (and an unchanged copy of the donor injection ruleset entity, needed for
  the pin) in the evaluation drafts AND in the announced proposal: what is evaluated is what is announced. `Writer::prepare` accepts a
  `release_settings` change only when its content is exactly `{inherit_from: <safe id>}`; interrupts, ruleset or limits are refused.
* Closed reasons when Core refuses it (all `not_announced:suite_error`, nothing is announced): `core_without_inherit_from` (older Core: validate
  answers REG-SCHEMA "Extra inputs are not permitted" for the field), `inherit_from_rejected` (e.g. the agent has a base), `donor_release_unknown`
  (validate 404). Only with `Config::admin_settings_fallback` AND an older Core the proof reruns the old way; its story keeps
  `assumptions: [release_settings_assumed]`, the inherit path has `[settings_inherited]`.
* Dossier coverage (ES/PT): the safety settings (fraude interrupt, injection-rules, lang-es-pt, input limit) are inherited by the clone from
  the donor release by server-side reference (`inherit_from`); the engine writes no interrupt; what was evaluated is exactly what is announced;
  approval and publication still need the platform human with step-up. The judge input carries `settings: inherit_from|assumed`.
* Observation for the agent-core team: `ApprovalReview.release_changes` (what the approver sees) is computed from the explicit
  `release_settings` fields only, so an `inherit_from` draft shows NO inherited interrupts to the approver. The values are only visible by
  reading the donor release. Suggest `release_changes` also lists the resolved inherited fields.

### INH1 live result (own stack `pulso-inh1`, agent-core = origin/main + PR 50 + PR 51 on a local scratch branch, never pushed)

`tests/live_inh1.rs`, planted uncovered-topic cell Tecnico/Phone, engine `builder` principal ONLY (no admin stand-in, no fallback): ANNOUNCED in
12 s. `regression_suite_proven`: base absent (6/6 finding cases fail by absence), candidate passes every finding case and guard, 13/13
GateItems, one manual-origin evaluation draft. Delivered as `auto_detect` draft `soporte-tecnico` (state `draft`, created_by `pulso-engine`,
16 changes: agent, flow, 9 templates, decision_model, language_detection, injection_ruleset, release_settings, eval_suite). Registry read-back:
`release_settings` content `{"inherit_from":"rel-f18a4d2c61045bf7"}` (the donor `consultas` prod release, active); rebuilding the candidate
read-only from the DB gives interrupts `[fraude, priority 100, escalate to queue fraude critical]`, `lang-es-pt@1.0.0`,
`injection-rules@1.0.0`, `max_input_chars 4000`, all equal to the donor's. 21 requests, none to approve, publish, promote, reject or revoke;
DB afterwards: 0 approvals, 0 releases for the new agent, the proposal still `draft`.

### Bugs found live and fixed

* Re-running on the same stack replayed the frozen drafts of an earlier run (registry Idempotency-Key): the live tests take an optional
  `PULSO_LIVE_NONCE` that changes the candidate digest. The engine's persistent ProofStore avoids this in production.
* A wrong gateway token turns every probe into "not measured" (401) and the case fails: the token for the local gateway may be blank in
  `llm-gateway.env` and set in `agent-core.env`.

## Tests

registry-writer 32 offline + 24 proof (new w13::* cases), reasoning dossier golden tests (coverage section), pulso value_loop 12 (the test
double serves the donor closure; a new agent has one evaluation draft, not two), python scripts/regression and scripts/battery unittests.
Limits: samples are 3 per probe; the finding is synthetic; the Builder was scripted (no LLM) in the live proofs.
