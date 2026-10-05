# Demo loop runbook (DEMO1)

One command runs the whole Pulso value loop on a LOCAL stack and prints a readable result:
`scripts/demo-loop/run.ps1`. Windows PowerShell 5.1 and pwsh 7 both run it (both were exercised). Spanish talk track:
[`DEMO_LOOP.es.md`](DEMO_LOOP.es.md). Branch `claude/demo1-loop-runner` (from `claude/ann1-announce-call`), no PR.

```
treated cells --> cells sensor --> Scout --> independent Verifier --> Builder --> compile (real artifact of the registry)
   -(finding, effect)-                                                                  |
                         regression suite on agent-core: FAILS on the base, PASSES with the candidate  <--+
                                   |  proven                          |  not proven
                         draft in agent-core (origin auto_detect,     stays internal: not_announced:<verdict>
                         state draft) + dossier ES                    (dossier kept in the job record)
                                   |
                         platform announce (one notification per supervisor)   -->  a PERSON approves, publishes, promotes
```

The engine only proposes. Nothing in this script approves, publishes, promotes, rejects or revokes anything.

## Quick start

```
powershell -File scripts/demo-loop/run.ps1 -Up                        # once: own stack, about 80 s
powershell -File scripts/demo-loop/run.ps1 -Cells -Loop -Show         # bank cells, the loop, the dossier
powershell -File scripts/demo-loop/run.ps1 -Cells -Loop -Show -Announce -Synthetic -BuilderModel xiaomi/mimo-v2.6-pro
powershell -File scripts/demo-loop/run.ps1 -Down                      # free the memory (volumes kept; -Purge drops them)
```

`-All` is `-Up -Cells -Loop -Show`. Steps always run in this order, whatever the order of the switches: Up, Cells, Loop, Probes, Show,
Announce, Down. Exit code 0 ok, 1 a step failed, 2 usage error (nothing was started).

| Switch | What it does |
|---|---|
| `-Up` | Own stack, prefix `pulso-demo`: Postgres :55490, llm-gateway :8190, agent-core :8191 with the real demo agents (`disputas`, `consultas`, registry-e2e artifacts, baseline label `live-registry`). It never touches the shared `pulso-l3` stack or another lane's prefix. The gateway image of another lane is re-tagged instead of rebuilt. |
| `-Cells` | The cached treated bank cells (`.dev-stack/demo-loop/cells/bank_cells.ndjson`, output of `scripts/aggregate/bank_cells.py`) if present, else it runs the aggregator over the local bank data (about 8 min, aggregates only leave it). `-CellsFile F` uses a table you give. `-Synthetic` uses the labelled planted-cell table instead. |
| `-Loop` | Starts `pulso run` (loopback :4190, ephemeral admin token), posts ONE explicit trigger, waits for the job, prints one block per finding, stops the engine. `-MaxFindings N` (default 4, 1..50) bounds the model cost: the first N corroborated findings in sensor order. `-BuilderModel M` sets the Builder tier (default flash; see "Variance"). `-TimeoutMin` (default 45). |
| `-Probes` | Starts an own battery core (prefix `pulso-demo-bat`, extra containers), runs the agent battery and the scheduled probes once (`-ProbeReps`, default 3) and prints the probe findings. Run it twice to see candidates become corroborated. |
| `-Show` | Prints the dossier ES of every announced proposal and reads the registry state back (`state=draft`, `origin=auto_detect`). |
| `-Announce` | With `PULSO_PLATFORM_URL` and `PULSO_PLATFORM_SERVICE_TOKEN` (or `PULSO_PLATFORM_TOKEN`) set: the engine tells the platform during `-Loop`, or the script posts the announce body itself when `-Loop` is not in the command. Without them: prints exactly the body that would be sent. |
| `-Down` | Stops this script's containers and serve processes only. `-Purge` also drops its volume, image and local state. |

Prerequisites: Podman machine `pulso-dev` (connection `pulso-dev-root`), `uv`, Python 3.11+ with PyYAML, a built
`pulso.exe` of this branch (found in `PULSO_EXE`, `CARGO_TARGET_DIR`, or `D:\cargo-targets\claude-demo1|claude-ann1`; the script never
builds), optional `steps_cli.exe` beside it (labels the cell and the effect of each finding; without it the report says so),
`agent-core.env` and `llm-gateway.env` next to the worktrees folder (or `PULSO_AGENT_CORE_ENV` / `PULSO_LLM_GATEWAY_ENV`), and the agent-core and
llm-gateway checkouts (`PULSO_AGENT_CORE_DIR`, default `worktrees/agent-core-claude-w15` = main + PR 50; `PULSO_LLM_GATEWAY_DIR`). It ran with about 2.8 GB of free RAM while other lanes'
stacks were up (Postgres, gateway container and one agent-core process per stack; the aggregator adds a few hundred MB for 8 minutes).

Credentials: read from the two env files into memory, handed to CHILD processes only (never set in the script's own environment, never
on a command line, never written to another file). Every line a child prints is masked first, including fragments of long values (a DSN password,
one token of a map); the Pester canary test dumps a child's whole environment and asserts no value appears. The engine's admin token is random per
run and masked too. Tokens of the local stack live in `.dev-stack/tokens.json` (gitignored).

## Reading the output

Per finding the loop prints:

| Field | Meaning |
|---|---|
| finding | `finding_<n>` = position in the sensor output; metric with its name (M1 contact_unresolved_rate, M4 pqr_open_rate, ...) |
| cell / effect | the dimensions of the cell and its rate against the rest of the metric in discovery, then in the holdout; an association, never a cause |
| status | what the reasoning roles did: `proposed / compiled`, `unlinked / <reason>` (no artifact moves it; descriptive, not a failure), `blocked / <reason>` (a role could not finish) |
| outcome | `announced`; `not_announced:<verdict>` (the proof did not hold: `not_fixed`, `non_discriminating`, `infra_failed`, ...); `unlinked (<reason>)`; `blocked (<reason>)` |
| slug | the artifact the patch targets (`estado_pqr` = `template:t/estado_pqr` of `consultas`) or the target of a new agent |
| proof | the regression verdict story in one line; for `infra_failed` the step and code of the first problem |
| cost | model cost of the finding in USD (Scout, Verifier, Builder; the proof is deterministic and costs no tokens) |

Files of a run live in `.dev-stack/demo-loop/` (gitignored): `work-<stamp>/value-loop/job-0.json` is the full record (aggregates, reason codes, the dossier
ES and PT of every proven proposal), `run-<stamp>.log` is the masked transcript, `engine-<stamp>.log` the engine log, `state.json` points `-Show` and
`-Announce` at the last result.

## What is real, recorded, simulated, not exercised

| Part | State |
|---|---|
| Models (Scout, Verifier, Builder) through the local llm-gateway, real prices | REAL (cost printed per finding) |
| agent-core registry with the real demo agents and their artifacts; `put_draft`, `validate`, `freeze`, `evaluate` as the engine `builder` principal | REAL, on our OWN local instance (not the shared Core); `AGENTCORE_ALLOW_DEMO` doubles for tools and classifier |
| Regression proof (suite fails on base, passes on candidate; wording probe) | REAL execution; a deterministic judge, `uncalibrated`; proves the suite discriminates, not that the change helps customers |
| Bank cells | REAL aggregates of the local bank dataset (snapshot 2026-06-18), k >= 10; when the cache exists they are RECORDED from an earlier aggregation (the run prints `bank-cached`) |
| `-Synthetic` cells | SYNTHETIC: invented counts, one planted category; labelled `synthetic-planted` in the report and in the dossier ("sintéticos") |
| Platform announce | Without `PULSO_PLATFORM_*`: the body is printed, nothing is sent. In the verified run it was sent to the ANN1 platform DOUBLE (`scripts/ann1/platform_double.py`, a stdlib mirror of the route), not to the real platform. The real PR 17 backend was exercised in ANN1 (`docs/journal/0657-*`), not in this run |
| Probes | REAL battery against our own battery core with seeded tool doubles; synthetic scenarios, never customers (`evidence_class: probe_synthetic`) |
| Sensor | `claude-standin` (real code, labelled stand-in) |
| Not exercised | outcome verdicts after a release (the estimator runs on a window after publication; on stationary data they are mostly `inconclusive`, and nothing is published here); approval, publication and promotion (human, by design) |

Limits to say out loud:

* **New agents are not announced in production.** Bank findings of M1 map to a NEW specialist agent. Its evaluation draft needs the donor's release
  settings (fraud interrupt, injection ruleset), and agent-core `put_draft` refuses them without the `admin` role. The engine `builder` has none, so these
  findings end `not_announced:infra_failed (put_draft forbidden_role HTTP 403)`, never silently green. A human-owned admin credential (or an agent-core
  change) is the unblock. See `W13_PROMPT_AND_AGENT_ANNOUNCE.md`.
* **Routing from `recepcion` to a new agent is a human-owned follow-up**, not part of the proposal (`routing_recepcion_to_new_agent` is listed as not measured).
* **Evidence links in the platform announce are opaque ids** (`CASE-` + 26 characters hashed from the finding); they have the shape the route requires and name no real case.
* **Real bank data now yields announceable patches (MAP1).** The M1 Queja cells map first to the existing artifacts that cover complaint follow-up (`t/estado_pqr`, then `p/resumen_radicado`), not to a new agent; with real cells the loop announced the `t/estado_pqr` patch for Queja on Phone, Email, App, WhatsApp and Web Chat (see `MAPPING.md`, section live result). The uncovered reasons (Tecnico, Comercial, Retencion) still map to a new agent and still end `not_announced:infra_failed`. The mapping is a hypothesis of where to intervene, never a cause. The synthetic planted mode is no longer needed to show an announced proposal.
  no real trigger. That is why the synthetic planted mode exists; the report says which mode ran.
* **Variance.** The Builder is a model. With the default `flash` tier, 2 of 3 synthetic runs wrote the placeholder without its braces and the proof
  correctly refused them (`not_announced:not_fixed`, wording probe failed in 8 of 8 cases); with `-BuilderModel xiaomi/mimo-v2.6-pro` the one run made was announced (about
  2x cost, still under 0.002 USD; one sample, not a rate). For a live demo use the pro tier. A refusal by the proof is the system working, not a bug.
* Re-running the same finding on the same stack replays the earlier drafts (registry idempotency) and the proposal id stays the same; agent-core allows 10
  `auto_detect` proposals per 24 h. `-Down -Purge` and `-Up` give a clean registry.
* One bank run had a transient `blocked (model_unavailable)` on one finding (gateway); it is counted and shown, not retried.

## Own prefix and ports (MAP1)

`PULSO_STACK_PREFIX` and `PULSO_DEMO_PG_PORT`, `PULSO_DEMO_GW_PORT`, `PULSO_DEMO_CORE_PORT`, `PULSO_DEMO_ENGINE_PORT` give a lane its own stack (never `PULSO_CORE_PORT`/`PULSO_PG_PORT`: the first is an engine setting and breaks `pulso run`). The report now prints per finding the ranked candidate list (`*` = tried, at most 2), one `tried` line per attempt and the human-owned note.

## Troubleshooting

* `pulso.exe not found`: build it once (`cd seams; cargo build -j 1 -p pulso`) or pass `-PulsoExe`.
* `the stack is not answering`: run `-Up`; `python scripts/dev-stack/stack.py health` needs the same `PULSO_STACK_PREFIX=pulso-demo` and ports.
* `no cells table`: add `-Cells` (or `-Cells -Synthetic`).
* A run that is "still working" for minutes is the proof: `evaluate` runs the real engine and retries infrastructure errors 4 times with 20 s backoff.
* Port or prefix clashes with another lane: edit `Get-DemoStackSettings` defaults in `scripts/demo-loop/run.lib.ps1`.

## Tests

```
powershell -Command "Invoke-Pester -Path scripts/demo-loop/run.Tests.ps1"   # Pester 3.4, also under pwsh: arguments, formatting, redaction canary, announce body
python -m unittest scripts/demo-loop/test_planted_cells.py
```

## Last verified run

2026-10-05 (local time 04:28 to 05:08, UTC-5), scripts at branch `claude/demo1-loop-runner`, engine binary built from `claude/ann1-announce-call` (`f42e5afa`, the
only later change is a test file), agent-core `scratch/w15-main-pr50` (`4ff8744`), llm-gateway `bdd9a9d`, models Scout `xiaomi/mimo-v2.6-flash`, Verifier `xiaomi/mimo-v2.6-pro`.
Aggregates only: no customer row, no secret and no token is in any transcript (they are masked before printing).

| Step | Mode | Duration | Model cost |
|---|---|---|---|
| `-Up` | own stack | 82.3 s | - |
| `-Cells` | real bank, fresh aggregation of 6 tables (7,995 treated rows, 19 corroborated findings), later runs read the cache | 488.4 s (cached: 0.1 s) | - |
| `-Loop` bank, cap 4 | `bank-cached` | 213.0 s | USD 0.0025 |
| `-Loop` synthetic, flash Builder | `synthetic-planted` | 67.3 s announced; 61.8 s and 65.1 s `not_fixed` | USD 0.0010 to 0.0011 |
| `-Loop` synthetic, pro Builder | `synthetic-planted` | 73.4 s announced | USD 0.0018 |
| `-Probes` x2 | own battery core | 148.2 s (with the core start), 53.1 s | not metered by the script |
| `-Down` | stack and battery containers of this script removed, other lanes' containers still up | 2.4 s | - |
| `-Announce` to the platform double | double | 0.5 s, HTTP 200, 2 supervisors notified on the double | - |

**Mode that ran.** The real bank cells produced NO announced proposal: the 2 M1 findings that compiled are new-agent proposals and end `not_announced:infra_failed`
(no admin credential for the evaluation release settings), one finding was `unlinked` (`dependent_on_m1`), one `blocked` (`model_unavailable`). So the
documented SYNTHETIC planted-cell mode ran next, labelled synthetic, and produced an announced proposal.

### 1. `-Up`

```
Pulso demo loop  2026-10-05 04:29:38  steps: Up  mode: bank
log: D:\.codex\factored\worktrees\improvement-engine-claude-demo1\.dev-stack\demo-loop\run-20261005-042938.log

=== [Up] own stack 'pulso-demo': postgres :55490, llm-gateway :8190, agent-core :8191
postgres 16 up on 55490
llm-gateway up on 8190
registry: pulso-builder imported
agent-core up on 8191 (pid 8288); try: python scripts/dev-stack/invoke_builder.py
stack up. registry http://127.0.0.1:8191  gateway http://127.0.0.1:8190  (agents: disputas, consultas, real registry-e2e artifacts)

--- step durations
  Up        ok        82.3 s
total 82.3 s
rc=0
```

### 2. Real bank cells (`-Cells -Loop -Show -Announce`), no announced proposal

```
Pulso demo loop  2026-10-05 04:53:50  steps: Cells Loop Show Announce  mode: bank
log: D:\.codex\factored\worktrees\improvement-engine-claude-demo1\.dev-stack\demo-loop\run-20261005-045350.log

=== [Cells] treated bank cells (aggregates only, k >= 10)
using the cached treated cells (delete D:\.codex\factored\worktrees\improvement-engine-claude-demo1\.dev-stack\demo-loop\cells\bank_cells.ndjson to re-aggregate)
cells: D:\.codex\factored\worktrees\improvement-engine-claude-demo1\.dev-stack\demo-loop\cells\bank_cells.ndjson  mode=bank-cached  rows=7995  metrics: M1=1358 M10=419 M2=374 M3=70 M4=350 M5=350 M6=1083 M6R=37 M6U=35 M7=1960 M8=512 M9=1447  sha256:739cde15ff17

=== [Loop] pulso run over the cells (trigger -> job -> sensor -> roles -> proof -> registry), cap 4 findings
engine: D:\cargo-targets\claude-ann1\debug\pulso.exe
engine ready on http://127.0.0.1:4190 (loopback, ephemeral admin token never printed)
trigger posted (explicit, key demo-loop-20261005-045350); waiting for the job (timeout 45 min)
  ... still working, 62 s
  ... still working, 122 s
  ... still working, 182 s

VALUE LOOP  mode: bank-cached   cells: bank_cells.ndjson
models: xiaomi/mimo-v2.6-flash   baseline: live-registry (29 live artifacts)   evaluate-before-announce: on
corroborated 19, reasoned 4, proposed 2, delivered 0, announced 0, not announced 2, unlinked 1, blocked 1, cost $0.002478

finding_1  M1 contact_unresolved_rate
  cell    : channel=Phone reason_category=Comercial
  effect  : 34.7% vs 22.3% rest (+12.3 pp); holdout 35.2% vs 22.4%
  status  : proposed / compiled
  outcome : not_announced:infra_failed
  slug    : consultas   (new_agent of new_agent:consultas)
  proof   : infra_failed: evaluate returned no verdict for a run (put_draft forbidden_role HTTP 403)
  cost    : $0.0012
finding_2  M1 contact_unresolved_rate
  cell    : channel=Phone reason_category=Queja
  effect  : 56.1% vs 16.6% rest (+39.5 pp); holdout 56.6% vs 16.6%
  status  : proposed / compiled
  outcome : not_announced:infra_failed
  slug    : consultas   (new_agent of new_agent:consultas)
  proof   : infra_failed: evaluate returned no verdict for a run (put_draft forbidden_role HTTP 403)
  cost    : $0.0011
finding_3  M10 handle_time_unresolved_share
  cell    : channel=Phone reason_category=Queja
  effect  : 56.3% vs 20.4% rest (+35.9 pp); holdout 56.8% vs 20.1%
  status  : unlinked / dependent_on_m1
  outcome : unlinked (dependent_on_m1)
  cost    : $0.0000
finding_4  M1 contact_unresolved_rate
  cell    : channel=Email reason_category=Queja
  effect  : 57.3% vs 17.2% rest (+40.1 pp); holdout 56.7% vs 19.0%
  status  : blocked / model_unavailable
  outcome : blocked (model_unavailable)
  cost    : $0.0002

=== [Show] dossier ES of every announced proposal + registry state read back (nothing is approved)
no announced proposal in the last run (mode: bank-cached). Unlinked or not-announced findings stay internal.
The engine only proposes. Approval, publication and promotion stay with the supervisors of the platform.

=== [Announce] support platform: one notification per announced proposal
no announced proposal in the last run: nothing to announce.

--- step durations
  Cells     ok         0.1 s
  Loop      ok       213.0 s
  Show      ok         0.0 s
  Announce  ok         0.0 s
total 214.0 s
rc=0
```

### 3. Synthetic planted cells with the pro Builder (`-Cells -Loop -Show -Announce -Synthetic -BuilderModel xiaomi/mimo-v2.6-pro`), announced

The synthetic mode is labelled everywhere it shows. The dossier below is shortened to its headline sections.

```
Pulso demo loop  2026-10-05 05:02:22  steps: Cells Loop Show Announce  mode: synthetic-planted
log: D:\.codex\factored\worktrees\improvement-engine-claude-demo1\.dev-stack\demo-loop\run-20261005-050222.log

=== [Cells] SYNTHETIC planted-cell table (invented numbers, labelled synthetic)
{"rows": 350, "metric": "M4", "planted": "Cobro indebido", "label": "synthetic"}
cells: D:\.codex\factored\worktrees\improvement-engine-claude-demo1\.dev-stack\demo-loop\cells\planted.ndjson  mode=synthetic-planted  rows=350  metrics: M4=350  sha256:4e95e7a3ce6f

=== [Loop] pulso run over the cells (trigger -> job -> sensor -> roles -> proof -> registry), cap 4 findings
engine: D:\cargo-targets\claude-ann1\debug\pulso.exe
engine ready on http://127.0.0.1:4190 (loopback, ephemeral admin token never printed)
trigger posted (explicit, key demo-loop-20261005-050223); waiting for the job (timeout 45 min)
  ... still working, 63 s

VALUE LOOP  mode: synthetic-planted   cells: planted.ndjson
models: xiaomi/mimo-v2.6-flash   baseline: live-registry (29 live artifacts)   evaluate-before-announce: on
corroborated 1, reasoned 1, proposed 1, delivered 1, announced 1, not announced 0, unlinked 0, blocked 0, cost $0.001765

finding_1  M4 pqr_open_rate
  cell    : category=Cobro indebido
  effect  : 62.0% vs 32.7% rest (+29.2 pp); holdout 62.0% vs 33.0%
  status  : proposed / compiled
  outcome : announced  proposal 01a10b78-5097-7e24-866c-d62ab082a0ce
  slug    : estado_pqr   (patch of template:t/estado_pqr)
  proof   : regression_suite_proven: La base falla 8 de 8 casos de la suite; intento 1 paso. [regression_suite_proven]
  cost    : $0.0018

=== [Show] dossier ES of every announced proposal + registry state read back (nothing is approved)
=== finding_1  estado_pqr  proposal 01a10b78-5097-7e24-866c-d62ab082a0ce
TITLE: template:t/estado_pqr - M4: propuesta de cambio
DECISIÓN: proponer a supervisión (suite de regresión probada).
Problema observado: M4 es persistentemente más alto en [category=Cobro indebido] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación.
Evidencia y comparación: Celda: 4.095/6.610 = 62,0 % frente a 32,7 % de la base de comparación (+29,2 pp); IC95 % de la tasa de la celda [60,8; 63,1] (Wilson). Replicación (holdout): 4.034/6.508 = 62,0 % frente a 33,0 %. Ventanas R2: replicated. p ajustado <0,0001. Es una asociación descriptiva, no una causa.
Qué cambiaría: Parche anclado sobre template:t/estado_pqr (189 caracteres editados de 240): [es] replace es.a1: "Ya consulté tu PQR." -> "Ya consulté tu PQR; su estado actual es {{facts.pqr.value.status}}."; [pt] replace pt.a1: "Já consultei sua solicitação." -> "Já consultei sua solicitação; o status atual é {{facts.pqr.value.status}}."
Resultado base vs candidato: Suite de regresión probada: falla en la base y pasa con el candidato. La base falla 8 de 8 casos del hallazgo (3 guardas); intento 1 pasó. GateItems: 15/15 aprobados.
Siguiente paso humano: Pasar a producción: tras aprobar y publicar en staging, la persona de supervisión lo promueve a producción desde la página del agente. No es una activación inicial.
[... the other dossier sections (what was measured, expected effect, how it will be evaluated, risk, what does not change, labels) are omitted here ...]
REGISTRY (read back from agent-core): state=draft origin=auto_detect agent=consultas rev=1 changes=2 created_by=pulso-engine
  never approved, published or promoted by the engine; a person decides in the platform

The engine only proposes. Approval, publication and promotion stay with the supervisors of the platform.

=== [Announce] support platform: one notification per announced proposal
PULSO_PLATFORM_URL / PULSO_PLATFORM_SERVICE_TOKEN are not set: this is what WOULD be sent (POST /api/v1/internal/builder/proposals/announce):
  {
      "proposalId":  "01a10b78-5097-7e24-866c-d62ab082a0ce",
      "title":  "template:t/estado_pqr - M4: propuesta de cambio",
      "problem":  "M4 es persistentemente más alto en [category=Cobro indebido] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación.",
      "evidence":  "Celda: 4.095/6.610 = 62,0 % frente a 32,7 % de la base de comparación (+29,2 pp); IC95 % de la tasa de la celda [60,8; 63,1] (Wilson). Replicación (holdout): 4.034/6.508 = 62,0 % frente a 33,0 %. Ventanas R2: replicated. p ajustado <0,0001. Es una asociación descriptiva, no una causa.",
      "expectedEffect":  "Hipótesis direccional (decrease), no medida: llevar la tasa de la celda (62,0 %) hacia la referencia (33,0 %); mejora mínima buscada 15,0 pp. No es una predicción de efecto.",
      "evidenceLinks":  [
                            "CASE-45T9YMMXB4HCAYXMKYVZW185Q1"
                        ]
  }
  (evidenceLinks are opaque ids derived from the finding, not real case ids)

--- step durations
  Cells     ok         1.1 s
  Loop      ok        73.4 s
  Show      ok         0.1 s
  Announce  ok         0.1 s
total 75.5 s
rc=0
```

### 4. `-Announce` against the platform double (token and URL set in the environment of the command, never printed)

```
=== [Announce] support platform: one notification per announced proposal
announce 01a10b78-5097-7e24-866c-d62ab082a0ce: HTTP 200

--- step durations
  Announce  ok         0.5 s
total 0.5 s
rc=0
```

### 5. `-Probes` (first run: candidates; second run: corroborated, `action: triggered`)

```
=== [Probes] agent battery + scheduled probes, once (own battery core; synthetic scenarios, never customers)
gateway image: re-tagged pulso-w15-llm-gateway as pulso-demo-bat-llm-gateway (no rebuild)
registry: registry-e2e imported
demo agent-core up on 8193 (pid 11616); agents: recepcion,disputas,consultas
{"action": "none", "confirmed": 0, "counts": {"by_status": {"candidate": 5}, "total": 5}, "flaky": 1, "run_index": 1}
PROBES  action: none  run_index: 1  confirmed scenarios: 0  flaky: 1
  P1  agent=consultas scenario_family=attacker:language_switch  status=candidate  reason=no_previous_run  evidence_class=probe_synthetic
  P1  agent=consultas scenario_family=attacker:vague_customer  status=candidate  reason=no_previous_run  evidence_class=probe_synthetic
  P1  agent=disputas scenario_family=attacker:language_switch  status=candidate  reason=no_previous_run  evidence_class=probe_synthetic
  P2  agent=consultas  status=candidate  reason=no_previous_run  evidence_class=probe_synthetic
  P2  agent=disputas  status=candidate  reason=no_previous_run  evidence_class=probe_synthetic
  (probe cells are synthetic scenarios, never customers; a first run is always candidate, a second run can corroborate)

--- step durations
  Probes    ok       148.2 s
total 148.2 s
rc=0
```

```
PROBES  action: triggered  run_index: 2  confirmed scenarios: 3  flaky: 0
  P1  agent=consultas scenario_family=attacker:language_switch  status=corroborated  reason=replicated_in_previous_run  evidence_class=probe_synthetic
  P1  agent=consultas scenario_family=attacker:vague_customer  status=corroborated  reason=replicated_in_previous_run  evidence_class=probe_synthetic
  P1  agent=disputas scenario_family=attacker:language_switch  status=corroborated  reason=replicated_in_previous_run  evidence_class=probe_synthetic
  P1  agent=recepcion scenario_family=attacker:language_switch  status=candidate  reason=previous_run_above_floor  evidence_class=probe_synthetic
  P2  agent=consultas  status=corroborated  reason=replicated_in_previous_run  evidence_class=probe_synthetic
  P2  agent=disputas  status=corroborated  reason=replicated_in_previous_run  evidence_class=probe_synthetic
  P2  agent=recepcion  status=refuted  reason=pass_rate_at_or_above_floor  evidence_class=probe_synthetic
  (probe cells are synthetic scenarios, never customers; a first run is always candidate, a second run can corroborate)

--- step durations
  Probes    ok        53.1 s
total 53.1 s
rc=0
```

The probe findings are the real defects of the demo agents already documented in `PROBES.md` (`consultas` and `disputas` do not honour a Spanish to Portuguese switch,
`consultas` accepts any text as a case number); they are synthetic scenarios, never customers.
