# Proposal dossier specification

Status: engine-side content contract; synthetic examples only. The support-
platform UI mapping was inspected only at immutable commit
`6ce2581aafad7282325badd9a333477f84b6f489`; it has not been verified against
the current remote default branch. The example proposals were not created,
evaluated, or released.

## Purpose

Design goal (not usability-tested): a supervisor should be able to answer
these questions quickly, with “about 30 seconds” as a future validation target,
not a verified performance claim:

1. What observed problem is this proposal responding to?
2. Which published evidence supports it, and what is the comparison baseline?
3. What exactly would change, and what is the expected direction of effect?
4. How would the proposed change be evaluated, and what does that evaluation
   not establish?
5. What risks exist, and what explicitly stays unchanged?

The dossier is an explanation of a candidate intervention, not a finding that
the intervention will work. Association in the bank snapshot does not identify
a cause, predict a lift, or establish that a proposed agent/tool/prompt is
effective. Evaluation evidence must be shown separately from descriptive
discovery evidence.

## Evidence boundary used by these examples

The three examples reuse only the audited aggregate below; they are fictional
proposals and their intervention details are not inferred from customer-level
records.

| Evidence | Audited aggregate | Interpretation limit |
| --- | --- | --- |
| Phone / Queja first-contact unresolved rate | 56.4% (54,418 / 96,521) | `was_resolved=false` in complete-month M1 aggregates; not proof that the issue was never resolved later |
| Phone / all other reasons, same measure | 16.6% (77,871 / 469,586) | same-channel descriptive comparator, not a causal control |
| Difference | +39.8 percentage points; naive binomial interval [39.5, 40.1] pp | descriptive association; interval does not account for clustering and is not an expected proposal lift |

These figures were recalculated from the aggregate-only M1 rows produced by `python scripts/aggregate/bank_cells.py --data-root D:\.codex\factored\data --out D:\.codex\factored\outcome-temp\bank-cells-for-opbench.ndjson` (SHA-256 `042cabffbd27c0d462ed51f6479647f4798af06f73f7546c49edd40d6987fe7d`). No customer-level records were used in the dossier. The artifact contains 35 complete months (July 2023–May 2026), excluding partial June 2023 and June 2026. These values are the combined discovery and holdout aggregates for the T1 sensor protocol. They are conceptually the same M1 outcome as OPBENCH V1, but not the same eligible-row population: OPBENCH V1 deduplicates exact `interaction_id` values and does not exclude partial months, whereas the T1 producer streams source rows and excludes those partial months; the two protocols also use different normalization and hash-split rules. OPBENCH V1's checked-in full-snapshot figures and pooled-complement effect therefore must not be substituted for or combined with this T1 same-channel comparison. The aggregate output and source data are local-only, so these values are not independently reproducible from the repository checkout alone. This is a same-channel descriptive comparison, not a matched control or causal estimate. The naive interval assumes independent observations and should be read cautiously because repeated customers/clustering are not represented.

The audit says the snapshot is stationary and does not tell us why Queja contacts
are unresolved. Consequently, all interventions below are explicitly
hypothesis-generating. They must not say that a specific PQR state, tool gap,
agent, channel, or customer behavior caused the observed difference.

## Agent Core contract mapping (checked-in wire snapshot)

The engine serializes dossier content into Agent Core's supported authoring
objects. This mapping is based on the checked-in
`core-bridge/wire/agent_core@c814c2b` contract snapshot; it is not a claim about
an unverified newer upstream schema. `VersionDocs` belongs to each
`EntityDraft.docs` inside the proposal's `changes[]`; it is not a set of
top-level `Proposal` fields. Therefore, a dossier currently has to travel with
at least one valid nonempty artifact change. The exact UI aggregation of those
per-change docs into a proposal-level dossier is not verified.

| Dossier information | Agent Core field / object | Contract status and guidance |
| --- | --- | --- |
| Proposal identity | `Proposal.proposal_id`, `agent_id`, `base_release_id`, `rev`, `candidate_hash`, `created_by`, `updated_at` | Structured registry metadata; platform-generated values are not dossier prose. `Proposal` has `updated_at`, not `created_at`; creation timestamps belong to draft-write/entity-version objects. |
| Why the engine created it | `Proposal.origin = auto_detect` | Supported enum value; acceptance should assert exact value |
| Human-readable heading | `Proposal.title` | Supported, max 200 characters in checked-in schema |
| Problem, observed evidence, denominator/baseline, interpretation limits, risk, unchanged behavior, measurement plan | `changes[].docs.description` (`EntityDraft.docs` uses `VersionDocs`) | Required nonempty string (max 4,000 chars); use labelled, concise prose until a structured dossier API exists |
| Rationale for this intervention | `changes[].docs.rationale` | Required string (max 4,000 chars); distinguish evidence from mechanism hypothesis and expected effect |
| Exact proposed delta | `changes[]` on draft write (`kind`, `content`, `docs`) | Agent Core's wire schema does not declare `minItems`; this engine's dossier-transport/acceptance invariant requires at least one changed `EntityDraft` so the dossier has a `docs` carrier. Content must use the schema for its artifact kind. The examples are conceptual synthetic deltas, not serialized schema-valid artifacts. |
| What changes from the prior version | `changes[].docs.changelog` | Required string (max 8,000 chars); list only the proposed delta, never imply it is already live |
| Evaluation outcome | Separate `EvalRun` / report object | Not a field in `Proposal` or `VersionDocs`. Show only when a real evaluation response exists; never fabricate a result for these examples |
| Lifecycle | `Proposal.state` | Draft/candidate/evaluated/approved/published are distinct. Engine authoring must stop at draft for this acceptance path; approval/publication/promotion belong to authorized platform actors |

Agent Core's proposal schema does **not** provide dedicated structured fields
for evidence, baseline, expected effect, risk, unchanged behavior, or
measurement plan. Until the platform offers a stable structured dossier
contract, encode them as labelled sections in each changed entity's
`changes[].docs.description` and keep `rationale` / `changelog` aligned. Do not
place these facts in registry IDs, titles, artifact identifiers, or free-form
fields on an unrelated entity.

## Supervisor-facing dossier content

Recommended `description` section order (use the same labels in every
language):

1. **Problema observado / Problema observado** — named population and outcome.
2. **Evidencia y comparación / Evidência e comparação** — numerator, denominator,
   rate, comparator rate, absolute difference, source, and time/snapshot scope.
3. **Qué cambiaría / O que mudaria** — bounded artifact delta, not broad intent.
4. **Efecto esperado / Efeito esperado** — directional hypothesis only unless a
   measured estimate exists; state `no medido` / `não medido` here.
5. **Cómo se evaluará / Como será avaliado** — baseline/candidate scenarios,
   metric, guardrails, and what a pass would and would not prove.
6. **Riesgo / Risco** — plausible harm and fail-safe/rollback behavior.
7. **Qué no cambia / O que não muda** — boundaries and existing behavior
   preserved.

Use counts only where the disclosure rule permits; this example has all counts
above 10. Never add customer records, source text, names, identifiers,
interaction references, or fabricated quotes. Keep the aggregate attached to
its metric definition: `first-contact unresolved` is not ultimate issue
resolution.

## Golden dossier 1 — prompt patch

Synthetic proposal to adjust an existing complaint-intake prompt. No new agent
or tool is assumed.

**Title (ES):** `Quejas: confirmar el motivo antes de proponer una respuesta`

**Title (PT-BR):** `Reclamações: confirmar o motivo antes de sugerir uma resposta`

**Supported Agent Core fields**

- `origin`: `auto_detect`
- `state`: `draft`
- `changes[0].docs.description` (ES):

  > Problema observado: En las interacciones Phone del snapshot, 54.418 de 96.521 contactos clasificados como Queja quedaron sin resolución en el primer contacto (56,4%); es una asociación descriptiva, no una causa. Evidencia y comparación: la tasa fue 16,6% (77.871/469.586) para los demás motivos del mismo canal (+39,8 pp; intervalo binomial ingenuo [39,5; 40,1] pp); el intervalo no ajusta por repetición de clientes. Qué cambiaría: Parche propuesto para el prompt de recepción de quejas: resumir el motivo en una frase y pedir confirmación si la categoría es ambigua; no añade permisos ni cambia decisiones financieras. Efecto esperado: Hipótesis direccional, no estimación; el efecto sobre la resolución real no está medido. Cómo se evaluará: Comparar prompt base y candidato en escenarios sintéticos/adversariales; medir clasificación, pertinencia, escalamiento seguro y guardrails. Esto no demuestra reducción de la tasa bancaria. Riesgo: Una pregunta innecesaria podría retrasar un caso urgente; máximo una confirmación y derivación segura ante urgencia o ambigüedad persistente. Qué no cambia: Identidad, acceso a herramientas, elegibilidad, compensaciones, aprobaciones ni autoridad humana.

- `changes[0].docs.rationale` (ES): `La tasa agregada prioriza investigar el primer contacto por Queja. El dataset no explica el mecanismo; confirmar el motivo es una hipótesis de intervención que debe superar evaluación adversarial antes de cualquier aprobación.`
- `changes[0].docs.changelog` (ES): `Propuesta de cambio únicamente: añade una confirmación breve del motivo antes de responder. Aún no aprobado, publicado ni promovido.`
- `changes`: one synthetic prompt artifact delta, nonempty, with no source-derived text or identifiers.

**Português (variante equivalente para os mesmos campos)**

- `changes[0].docs.description` (PT-BR):

  > Problema observado: Nas interações Phone do snapshot, 54.418 de 96.521 contatos classificados como Reclamação ficaram sem resolução no primeiro contato (56,4%); é uma associação descritiva, não uma causa. Evidência e comparação: a taxa foi 16,6% (77.871/469.586) para os demais motivos no mesmo canal (+39,8 pp; intervalo binomial ingênuo [39,5; 40,1] pp); o intervalo não ajusta por repetição de clientes. O que mudaria: Patch proposto para o prompt de entrada de reclamações: resumir o motivo em uma frase e pedir confirmação quando a categoria for ambígua; não adiciona permissões nem altera decisões financeiras. Efeito esperado: Hipótese direcional, não estimativa; o efeito sobre a resolução real não foi medido. Como será avaliado: Comparar o prompt-base e o candidato em cenários sintéticos/adversariais; medir classificação, pertinência, escalonamento seguro e guardrails. Isso não demonstra redução da taxa observada no banco. Risco: Uma pergunta desnecessária pode atrasar um caso urgente; no máximo uma confirmação e encaminhamento seguro diante de urgência ou ambiguidade persistente. O que não muda: Identidade, acesso a ferramentas, elegibilidade, compensações, aprovações e autoridade humana.

- `changes[0].docs.rationale` (PT-BR): `A taxa agregada prioriza investigar o primeiro contato de Reclamação. O dataset não explica o mecanismo; confirmar o motivo é uma hipótese de intervenção que precisa passar por avaliação adversarial antes de qualquer aprovação.`
- `changes[0].docs.changelog` (PT-BR): `Apenas proposta: acrescenta uma confirmação breve do motivo antes da resposta. Ainda não aprovada, publicada nem promovida.`

## Golden dossier 2 — link to an existing tool

Synthetic proposal to make an already-existing read capability available in a
bounded complaint workflow. It does not assert that missing tool access caused
unresolved contacts.

**Title (ES):** `Quejas: consultar el estado de PQR antes de explicar el siguiente paso`

**Title (PT-BR):** `Reclamações: consultar o status da manifestação antes de explicar o próximo passo`

**Supported Agent Core fields**

- `origin`: `auto_detect`
- `state`: `draft`
- `changes[0].docs.description` (ES):

  > Problema observado: En Phone, Queja tiene 56,4% sin resolución en el primer contacto (54.418/96.521); esto no identifica una causa ni permite unir una PQR concreta con un contacto. Evidencia y comparación: 16,6% (77.871/469.586) para los demás motivos del mismo canal (+39,8 pp; intervalo binomial ingenuo [39,5; 40,1] pp); no es un control causal y el intervalo no ajusta por repetición de clientes. Qué cambiaría: Propuesta sintética para que el agente existente de disputas use `obtener_pqr` solo para leer de vuelta una PQR recién creada mediante su clave de idempotencia, resumir el estado y ofrecer el siguiente paso permitido; no demuestra búsqueda histórica por cliente. Efecto esperado: Hipótesis no cuantificada: explicaciones más consistentes y menos respuestas sin respaldo; no se predice una reducción de 39,8 pp. Cómo se evaluará: Casos sintéticos de PQR abierta, cerrada, no encontrada, respuesta inválida y timeout; medir exactitud, abstención, privacidad y escalamiento, no resolución bancaria. Riesgo: Exposición de datos ajenos o interpretación errónea; mantener autorización contextual, no buscar por identificadores libres y escalar ante falta de acceso o ambigüedad. Qué no cambia: La herramienta sigue siendo de solo lectura; no modifica PQR, transacciones, dinero, compensaciones ni decisiones.

- `changes[0].docs.rationale` (ES): `El hallazgo señala una población con más contactos no resueltos al primer intento, pero no demuestra que falte consulta de estado. El enlace de herramienta es una hipótesis verificable para la explicación del siguiente paso.`
- `changes[0].docs.changelog` (ES): `Propuesta de enlace de obtener_pqr al flujo acotado para leer de vuelta una PQR recién creada por su clave de idempotencia; no consulta PQR históricas ni agrega operaciones de escritura.`
- `changes`: one synthetic, schema-specific draft linking the existing tool; exact serialized shape must follow the tool/agent artifact schema and remain nonempty.

**Português (variante equivalente para os mesmos campos)**

- `changes[0].docs.description` (PT-BR):

  > Problema observado: Em Phone, Reclamação tem 56,4% sem resolução no primeiro contato (54.418/96.521); isso não identifica uma causa nem permite vincular uma manifestação específica a um contato. Evidência e comparação: 16,6% (77.871/469.586) para os demais motivos no mesmo canal (+39,8 pp; intervalo binomial ingênuo [39,5; 40,1] pp); não é controle causal e o intervalo não ajusta por repetição de clientes. O que mudaria: Proposta sintética para que o agente existente de disputas use `obtener_pqr` somente para ler de volta uma manifestação recém-criada por sua chave de idempotência, resumir o status e oferecer o próximo passo permitido; não comprova busca histórica por cliente. Efeito esperado: Hipótese não quantificada: explicações de status mais consistentes e menos respostas sem respaldo; não se prevê uma redução de 39,8 pp. Como será avaliado: Casos sintéticos de manifestação aberta, fechada, não encontrada, resposta inválida e timeout; medir exatidão, abstenção, privacidade e escalonamento, não a resolução observada no banco. Risco: Expor informação alheia ou interpretar incorretamente um status; manter autorização contextual, não pesquisar por identificadores livres e escalar sem acesso ou diante de ambiguidade. O que não muda: A ferramenta continua somente de leitura; não altera manifestações, transações, dinheiro, compensações nem decisões.

- `changes[0].docs.rationale` (PT-BR): `O achado aponta uma população com mais contatos não resolvidos na primeira tentativa, mas não demonstra falta de consulta de status. O vínculo da ferramenta é uma hipótese verificável para explicar o próximo passo.`
- `changes[0].docs.changelog` (PT-BR): `Proposta de vínculo de obtener_pqr ao fluxo limitado para leitura de uma manifestação recém-criada por sua chave de idempotência; não consulta manifestações históricas nem adiciona operações de escrita.`

## Golden dossier 3 — new specialist agent

Synthetic proposal for a narrow specialist, with explicit fallback. The
aggregate supports prioritizing the complaint population only; it does not
support the proposed specialty taxonomy or establish that a new agent will
resolve contacts.

**Title (ES):** `Quejas: proponer un especialista acotado con escalamiento seguro`

**Title (PT-BR):** `Reclamações: propor um especialista limitado com escalonamento seguro`

**Supported Agent Core fields**

- `origin`: `auto_detect`
- `state`: `draft`
- `changes[0].docs.description` (ES):

  > Problema observado: En Phone, Queja sin resolución en el primer contacto fue 56,4% (54.418/96.521); esto justifica priorizar una investigación acotada, no demuestra que falte un especialista ni explica las causas. Evidencia y comparación: 16,6% (77.871/469.586) para los demás motivos del mismo canal (+39,8 pp; intervalo binomial ingenuo [39,5; 40,1] pp); el intervalo no ajusta por repetición de clientes y no es control causal. Qué cambiaría: Propuesta sintética de un agente especialista que resume la solicitud, verifica si hay datos suficientes para el flujo existente y deriva a la ruta actual fuera de alcance; no ejecuta acciones de cuenta ni resuelve disputas. Efecto esperado: Hipótesis direccional no medida: más consistencia de clasificación y derivación en escenarios cubiertos; no se estima mejora en resolución ni ahorro. Cómo se evaluará: Suite sintética/adversarial con casos dentro/fuera de alcance, ambigüedad, datos faltantes, inyección y necesidad de humano; comparar clasificación, abstención, derivación y guardrails, reportando solo resultados ejecutados. Riesgo: Enrutamiento incorrecto o confianza excesiva que retrase ayuda humana; alcance explícito, abstención y escalamiento ante incertidumbre, urgencia o falta de autoridad. Qué no cambia: Se conservan el punto de entrada y proceso humano; no se agregan permisos, políticas, herramientas mutables ni publicación automática.

- `changes[0].docs.rationale` (ES): `La tasa observada identifica Queja como población prioritaria, pero los datos no incluyen una causa raíz. Un especialista nuevo es una alternativa de diseño, no una conclusión del análisis; debe compararse con el parche de prompt, el uso de herramientas existentes y no hacer cambios.`
- `changes[0].docs.changelog` (ES): `Propuesta hipotética de nuevo agente especializado y limitado, con abstención/escalamiento; no desplegado.`
- `changes`: one synthetic specialist-agent artifact draft with no real customer examples, identifiers, or bank-specific policy claims.

**Português (variante equivalente para os mesmos campos)**

- `changes[0].docs.description` (PT-BR):

  > Problema observado: Em Phone, Reclamação sem resolução no primeiro contato foi 56,4% (54.418/96.521); isso justifica uma investigação limitada, não demonstra que falte especialista nem explica as causas. Evidência e comparação: 16,6% (77.871/469.586) para os demais motivos no mesmo canal (+39,8 pp; intervalo binomial ingênuo [39,5; 40,1] pp); o intervalo não ajusta por repetição de clientes e não é controle causal. O que mudaria: Proposta sintética de um agente especialista que resume a solicitação, verifica se há dados suficientes para o fluxo existente e encaminha casos fora do escopo; não executa ações de conta nem resolve disputas. Efeito esperado: Hipótese direcional não medida: mais consistência na classificação e encaminhamento dos cenários cobertos; não se estima ganho de resolução nem economia. Como será avaliado: Suíte sintética/adversarial com casos dentro/fora do escopo, ambiguidade, dados ausentes, injeção e necessidade de atendimento humano; comparar classificação, abstenção, encaminhamento e guardrails, relatando apenas resultados executados. Risco: Encaminhamento incorreto ou confiança excessiva que atrase atendimento humano; escopo explícito, abstenção e escalonamento diante de incerteza, urgência ou falta de autoridade. O que não muda: O ponto de entrada e processo humano são mantidos; não se adicionam permissões, políticas, ferramentas mutáveis nem publicação automática.

- `changes[0].docs.rationale` (PT-BR): `A taxa observada identifica Reclamação como população prioritária, mas os dados não incluem causa-raiz. Um novo especialista é uma alternativa de desenho, não uma conclusão da análise; deve ser comparado com patch de prompt, uso de ferramentas existentes e não alterar nada.`
- `changes[0].docs.changelog` (PT-BR): `Proposta hipotética de novo agente especializado e limitado, com abstenção/escalonamento; ainda não implantado.`

## UI rendering: verified S22 snapshot and remaining platform asks

Read-only source inspection was performed against support-platform commit
`6ce2581aafad7282325badd9a333477f84b6f489` (S22, 2026-10-05), made available
in the shared local clone. The clone's checked-out `HEAD` is older
(`5e105a747b432a260cbbe56bb2de682ce1231e46`, 2026-10-04); the inspected commit
object is not contained by its checked-out refs. These findings are therefore
verified for the cited immutable commit, not independently confirmed as the
current GitHub default branch.

In that snapshot, the proposals list renders title, agent, state, source, and
updated time (`ProposalsScreen.tsx`); the detail view renders proposal title,
each changed artifact's kind/id/version, `docs.description`, `docs.rationale`,
and `docs.changelog`, plus agent tools/locales where applicable and gate-level
evaluation results (`ProposalScreen.tsx`, `proposals.ts`). Description is a
plain `<span>` without whitespace-preservation or Markdown rendering, so line
breaks and `**bold**` markers must not be relied on. For the first integration,
engine-produced docs must use concise plain text with explicit inline labels;
the dossier remains available through registry fields only if those fields are
actually populated. `ProposalScreen` does not render a dedicated
`ImprovementDossier` block in the inspected source; notification behavior was
not evaluated in this pass.

The snapshot represents `engine` as a supported proposal source. Evaluation is
blocked when neither the draft nor its base release supplies an `eval_suite`.
Approval and publication are human step-up actions. A proposal attached to an
existing agent should follow the agent's publish/promotion path; activating an
agent for a case type is a different action. The engine remains draft-only and
must never perform approve, publish, activation, or production promotion.

Remaining platform asks are: render the dossier as a structured, readable
section on proposal detail; preserve or section long-form descriptions; show
evidence provenance safely; and provide actor-attributed immutable lifecycle
history and a read-only rolling-24-hour quota observation contract. Claude's
inspection reports the latter two contracts absent from the platform and
agent-core APIs; the acceptance harness must retain `not_exercised` for them.
Do not add an engine-only shadow schema for UI limitations; coordinate any UI
change with the platform owner.

## Acceptance harness boundary and ownership

The T4 brief calls for a Python-standard-library black-box acceptance harness
under `scripts/acceptance/`, using HTTP against the running local stack and
recorded HTTP responses in tests; it must not import engine implementation
modules or inspect internal databases. Claude approved `scripts/acceptance/**`
and its tests as X-DOC-owned in shared journal entry CL-0069. Codex may add
that ownership glob in its consolidated PR; if an L-GOV ownership change lands
first, the identical union is safe.

When owned, the harness acceptance matrix should cover:

- Stack unavailable: exit nonzero with a concise endpoint/availability error;
- proposal present, response `proposal_id` matches the requested resource,
  `origin=auto_detect`, `state=draft`, and valid nonempty changes;
- proposal detail is fetched through the pinned read-only registry GET and its
  current state is `draft`; this proves current state only, not actor history;
- actor-attributed immutable authoring history contains no engine approval,
  publish, or promotion action, only when a supported read-only endpoint
  guarantees complete history bound to this proposal and an authenticated
  actor vocabulary. The pinned OpenAPI does not expose that evidence contract;
  arbitrary JSON with `actions` is not sufficient, so report this live check
  as `not_exercised`;
- dossier content contains required evidence, baseline, expected-effect
  qualification, measurement plan, risks, and unchanged-behavior sections;
- proposal-detail parsing preserves the pinned `ProposalDetail` sibling
  `changes[]`, `last_eval`, and `review` fields beside the nested proposal
  summary; the checker scans both proposal metadata and artifact content;
- the acceptance check validates the generic pinned `EntityDraft` envelope
  (`kind`, nonempty JSON-object `content`, and bounded `docs`) without claiming
  to validate kind-specific artifact semantics; `ProposalDetail` must include
  its required `proposal`, `changes`, and `last_eval` fields;
- each changed artifact has nonempty bodies for all seven labelled sections;
  evidence must contain at least two numeric values, an explicit comparison,
  and a source/snapshot marker. This is a mechanical completeness test, not
  claim verification or kind-specific artifact-schema validation;
- the six golden descriptions are compact plain text with inline labels,
  matching the inspected S22 renderer; regression tests prevent Markdown
  emphasis markers or paragraph layout from being relied on;
- no fixture PII-token patterns or obvious email, phone, UUID, or long-numeric
  canaries are returned in proposal/change responses. These regex checks are
  regression canaries, not a general PII detector; do not claim they establish
  that arbitrary identifiers, source text, or tool payloads are safe;
- 24-hour proposal quota: verify the configured limit and HTTP-visible
  enforcement at the boundary. `docs/dev/LOCAL_STACK.md` documents a limit of
  10 proposals per rolling 24 hours, but the pinned registry OpenAPI snapshot
  does not expose a read-only quota endpoint. The harness must not invent one
  or issue proposal-creation requests against a shared running stack merely to
  test the limit. It should use an explicitly configured/approved HTTP
  observation endpoint when available, only if its contract binds tenant,
  proposal scope, rolling 24-hour window, and actual accepted/rejected boundary
  attempts. A syntactically valid object is not quota evidence. Otherwise report
  the live quota check as `not_exercised`; recorded-response tests cover local
  checker behavior only, not live enforcement.
- recorded-response tests exercise missing fields, wrong origin/state, empty
  changes, malformed generic entity drafts, the required proposal-detail
  envelope, PII inside sibling artifact content, render-ready dossier prose,
  empty sections, evidence without a comparator/source, prohibited lifecycle
  transitions, and quota boundary.

The quota limit comes from `docs/dev/LOCAL_STACK.md`; the pinned registry
OpenAPI exposes no read-only quota endpoint, so the live quota check remains
`not_exercised` unless an approved, scoped observation contract is added. A
proposal-detail `GET` verifies current draft state, not lifecycle history; a
release-detail `GET` is not an actor log. The
support-platform display questions above remain separate from this acceptance
path.

## Non-claims

- No claim that a prompt patch, tool link, or new agent was created in Agent
  Core, approved, evaluated, published, or promoted.
- No estimate of reduced unresolved contacts, increased sales, savings, or
  business lift.
- No causal explanation of the Queja result and no inferred customer/PQR
  linkage.
- UI field mapping is confirmed only for support-platform commit
  `6ce2581aafad7282325badd9a333477f84b6f489`; it has not been independently
  verified as the current remote default branch.
- The read-only acceptance harness now exists with recorded-response tests.
  The proposal-detail GET exists and can verify current `state=draft`; it does
  not provide actor-attributed immutable lifecycle history. The pinned
  OpenAPI exposes no read-only quota endpoint. Even plausible supplied history
  and quota JSON remain `not_exercised` until a documented, proposal-/tenant-
  bound complete observation contract is available. Recorded fixtures never
  prove those runtime properties.
