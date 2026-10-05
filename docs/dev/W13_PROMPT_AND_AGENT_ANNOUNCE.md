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
  (templates, decision model, tools, language detection; REG-PIN fails without it) to the draft, and, for the evaluation only, the donor's
  release settings (fraude interrupt, injection ruleset, lang-es-pt), without which agent-core `evaluate` answers 500. These settings are
  labelled `release_settings_assumed` and are NOT delivered with the announced proposal. The recepcion routing change cannot be in the same
  proposal: human-owned follow-up, stated as not measured (`routing_recepcion_to_new_agent`, `traffic_stealing`).

## Live results (2026-10-05, own stack `pulso-w13`: agent-core + PR 50 local on :8203, gateway :8213, engine builder principal)

`tests/live_w13.rs` (3 tests, `#[ignore]`, run one by one with `--test-threads 1`; credentials only in the process environment).

| case | result |
|---|---|
| (i) prompt patch `p/resumen_radicado` (closing follow-up sentence) | ANNOUNCED. `regression_suite_proven`, `native_binding: candidate_bound`, 6/6 finding cases fail on the base and pass on the candidate, guards pass, 13/13 GateItems. Real samples: base "...Te mantendremos informado..." vs candidate "Un especialista ... tomara el seguimiento". Delivered as auto_detect draft with 2 changes (patch + eval_suite). 176 s. |
| (ii) text-identical prompt (no-op candidate) | NOT announced: `not_announced:not_fixed`. `candidate_bound`; native evaluate passes (as for any prompt), the wording probe fails 6/6: the harness probe is what separates a patch from a no-op. 241 s. |
| (iii) NEW agent `soporte-tecnico` (clone closure of the donor) | ANNOUNCED. Proven on itself, all finding cases and guards pass natively, base by absence. Delivered as auto_detect draft: 14 changes (agent, flow, 9 templates, decision_model, language_detection, eval_suite), NO release_settings. 9 s. |

### Blocker found and how it is carried (iii)

agent-core `put_draft` refuses release `interrupts` unless the actor has the `admin` role (`forbidden_role`, "cambiar las interrupciones de la
release exige el rol admin"). The engine builder cannot put the donor's fraude interrupt in the evaluation draft. The live run therefore used
the local staff admin credential (stand-in, labelled) ONLY for the manual-origin evaluation drafts; the announced proposal is delivered by the
builder and carries no settings. What the engine must carry for the real path: either an admin-role evaluation credential owned by a human, or
an agent-core change that lets evaluation drafts take the donor release settings by reference (agent-core scope, not ours). Until then a new
agent proof in production is `infra_failed` (not announced), never silently green.

### Bugs found live and fixed

* Re-running on the same stack replayed the frozen drafts of an earlier run (registry Idempotency-Key): the live tests take an optional
  `PULSO_LIVE_NONCE` that changes the candidate digest. The engine's persistent ProofStore avoids this in production.
* A wrong gateway token turns every probe into "not measured" (401) and the case fails: the token for the local gateway may be blank in
  `llm-gateway.env` and set in `agent-core.env`.

## Tests

registry-writer 32 offline + 24 proof (new w13::* cases), reasoning dossier golden tests (coverage section), pulso value_loop 12 (the test
double serves the donor closure; a new agent has one evaluation draft, not two), python scripts/regression and scripts/battery unittests.
Limits: samples are 3 per probe; the finding is synthetic; the Builder was scripted (no LLM) in the live proofs.
