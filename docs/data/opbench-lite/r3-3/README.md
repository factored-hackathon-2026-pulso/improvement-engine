# OPBENCH-lite R3-3 — aggregate-inspired Agent Core regression probes

## Why these suites exist

The bank-derived OPBENCH-lite catalog identifies broad contact-resolution patterns. These suites convert three aggregate references into authored, synthetic probes for the current Agent Core artifacts. They do **not** reconstruct observed contacts, establish a root cause, or show that an agent currently passes.

The outcome metric is the unresolved-first-contact flag divided by contacts with a valid resolution flag; contrasts are within-channel and descriptive, not causal. All source evidence is from the synthetic hackathon snapshot, not production bank outcomes.

| Catalog entry | Aggregate fact (discovery → replication) | What the probes test | What they do not prove |
|---|---|---|---|
| `M1-01`, phone + complaint | 27,978/49,649 unresolved vs 39,937/241,862 baseline; contrast +39.8393 pp. Replication: 28,151/49,905 vs 40,253/241,834; +39.7643 pp. | Synthetic dispute flow: verified resolution, customer cancellation, low-information escalation, lookup failure, high-amount branch, and failed PQR read-back. | The aggregate does not say these complaints were duplicate/incorrect charges, identify a failure mechanism, or attribute a result to `disputas`. Those are authored behavior probes only. |
| `M1-13`, phone + technical | 13,004/43,586 unresolved vs 54,911/247,925 baseline; contrast +7.6870 pp. Replication: 13,321/43,849 vs 55,083/247,890; +8.1585 pp. | Synthetic status lookups in the existing PQR query flow, plus malformed-reference and tool-error escalations. | This does not test generic technical troubleshooting, show PQRs caused technical contacts, or prove why calls remained unresolved. The `consultas` artifact only supports the narrower PQR-status path. |
| `E1-01`, overall E0 signature | Discovery-selected leading query signature: 1,587/2,000 pooled (79.35%); discovery/replication difference +2.6111 pp, 95% interval [-3.5111, +8.7333]. Status `uncertain`; actionability `covered_existing_capability`. | Six synthetic reads of movement details as a capability regression control. | Not a new opportunity, causal impact, detector-suppression test, or proof of an advisor-specific run. The pinned evaluator builds a `customer` principal while `copiloto-asesor` is invocable by `advisor`; this suite is schema-valid but marked `not_evaluable` for advisor capability under the stock harness. |

Counts and contrasts are loaded from the checked-in `evidence_catalog.json` aggregate excerpt; `provenance.json` records both its exact byte hash and the full source artifact SHA-256. Scenario references carry a canonical hash over the complete row-free entry descriptor (definition, cell, discovery, replication, and snapshot), so a change to cited metrics changes the reference hash. The optional `PULSO_OPBENCH_CATALOG` source crosswalk verifies both the complete source digest and every copied excerpt field against the selected source entries. Only broad low-cardinality aggregates are included. Hashing is **not anonymization**. The source catalog declares aggregate-only output, minimum count 10, and no row data. No new subgroup breakdown is made here.

## Evidence levels and privacy

Keep these four layers separate:

1. **Source finding:** the aggregate entries and their descriptive status above.
2. **Probe:** freshly authored synthetic phrases and fake IDs, motivated by a broad finding and the current Agent Core capability catalog.
3. **Expected behavior:** a deterministic target encoded in each EvalSuite. It is a test expectation, not a result.
4. **Execution result:** all are `not_run` in the provenance sidecar until the suite is run against the pinned runtime. The encoded outcome is a fixed expectation; runtime route/tool behavior can vary by provider and execution environment.

No source row, customer identifier, name, contact detail, account/card/radicado number, transcript, or transformed customer phrase is included. Fake principal, movement and PQR references use `syn-*` namespaces. Scenario language-country combinations are intentionally synthetic and are not supported as country-language prevalence or business evidence by the aggregate source. Automated email/long-digit checks are only leakage tripwires; they are not a substitute for manual source/provenance review. Provenance never hashes source keys, customer IDs or raw text.

## Suite contract and execution boundary

`generate_scenarios.py --suite <agent>` emits one JSON document, which is valid JSON-as-YAML and matches the pinned `EvalSuite` shape. Each suite has six unique scenarios, exact current `agent_id`, scripted synthetic source, three repetitions, tool replies keyed by bare tool ID, explicit expected outcomes, and no steps after terminal confirmation. Seeds are keyed by tool ID and the stock EvalSuite assertions cannot validate tool-call arguments; these are contract/flow smoke probes, not tool-argument correctness tests. Provenance and evidence tags live only in the sidecar; the schema forbids arbitrary scenario annotations.

Validated locally with the read-only Agent Core checkout pinned at `c814c2b`, using its `EvalSuite.model_validate`, `Agent.model_validate`, and `suite_problems`. The minimal validator test bypasses only the broad `agent_core.registry` package initializer (which eagerly imports persistence backends); it imports the exact pinned `registry/suite.py` module and exact domain models. It does not modify that checkout.

The `copiloto-asesor` suite is kept as a non-runnable control artifact to document the capability and harness gap. Do not pass it to the stock live harness as evidence of advisor behavior: that harness currently constructs a customer principal, but the agent accepts advisors. No `copiloto-asesor` execution/pass rate is claimed. The two customer-facing suites are schema-valid and agent-binding-valid under the pinned `suite_problems` contract; this does not establish runtime path semantics. Cancellation and high-amount escalation require three read/selection/conversion calls before the target; stock metric predicates cannot distinguish a `radicar_pqr` write from those reads. Their provenance therefore requires a complementary runtime audit proving no write, and these packaged suites do not claim that audit has run. Live execution was not performed in this environment, so their actual outcomes remain unknown.

## Repetition and 30-point-effect protocol

Three repetitions × six authored scenario families yield 18 run instances per suite, but only six distinct prompts. Three repeats are a deterministic/regression smoke check—not 18 independent samples, not a power calculation, and not evidence for the aggregate contrast. The demo pack makes no statistical effect or lift claim.

To assess a 30-percentage-point change later, first declare the exact runtime endpoint, baseline rate, unit of analysis, repeated-run dependence, paired/randomized assignment, clustering, and multiplicity family. Then preregister an independent-family sample-size/power calculation from those inputs, execute baseline and candidate on matched independent cases (or the declared randomized design), preserve an untouched holdout, and report uncertainty. Repeated identical prompts cannot substitute for independent cases; with the current evidence, a defensible exact case count is not determined.

## Reproduce and validate

Use Python 3.12 and an isolated virtual environment. The minimum model-validation dependencies (not installed globally) are:

```powershell
py -3.12 -m venv .venv-r3-3
.\.venv-r3-3\Scripts\python.exe -m pip install "pydantic>=2.9,<3" "rfc8785>=0.1.4" "pyyaml>=6.0.2,<7"
$env:PULSO_AGENT_CORE_DIR = "<read-only path to agent-core commit c814c2b>"
$env:PYTHONPATH = $env:PULSO_AGENT_CORE_DIR
.\.venv-r3-3\Scripts\python.exe -m unittest -v test_scenarios.py test_pinned_agent_core.py
```

The suite generator is deterministic and writes only to stdout:

```powershell
python generate_scenarios.py --suite disputas
python generate_scenarios.py --suite consultas
python generate_scenarios.py --suite copiloto-asesor
python generate_scenarios.py --provenance
```

Local tests validate structural/privacy tripwires, target coverage, synthetic provenance, exact pinned Pydantic models, agent binding, and `suite_problems`. They do not execute agents, validate tool arguments, or prove that expected behavioral paths are semantically correct. A live check still requires an accessible local Agent Core/engine stack; none is claimed here. The stock live attachment helper may create/update a **local registry proposal draft**, then validate/freeze/evaluate it; never approve/publish/promote as part of this exercise. Do not run the Copilot suite through the current advisor-incompatible harness.

## Known follow-up

The merged R3-2 main tree still labels the first pass as blinded/human in `agreement.py`, despite the README and PR description being candid. A test-first follow-up corrects the module description and output provenance to `pass1_unblinded_codex_labels`; no label scores or agreement metrics change. The Portuguese/Spanish prompts and MX/CO/AR assignments are artificial probe examples, not localized customer research. Seeded replies are selected by tool ID and cannot establish that a tool call carried the correct transaction or PQR argument.
