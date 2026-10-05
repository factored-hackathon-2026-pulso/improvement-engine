# Proposal dossier specification

Status: engine-side content contract; synthetic examples only.  This document
does not claim that the support-platform UI has been inspected or that the
example proposals were created, evaluated, or released.

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
| Queja first-contact unresolved rate | 56.4% (66,000 / 117,021) | `was_resolved=false` at final extract; not proof that the issue was never resolved later |
| All other reasons, same measure | 16.6% | comparison population, not a causal control |
| Difference | +39.8 percentage points; interval [39.5, 40.1] pp | stable association in the supplied synthetic hackathon snapshot; not an expected proposal lift |

This aggregate is reported in the workspace audit `docs/reports-claude/BANK_DATA_AUDIT_2026-10-04.md` (section 0). That audit is not part of this engine repository, so a consumer of this file must treat the figures as a supplied, audited input and must not claim the audit is reproducible from this repository alone. The checked-in OPBENCH-lite catalog reports a *different estimand* (phone/complaint versus pooled complement); do not substitute or combine those values with this all-channel, reason-level comparison.

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

  > **Problema observado.** En el snapshot auditado, 66.000 de 117.021 contactos clasificados como Queja quedaron sin resolución en el primer contacto (56,4%); para los demás motivos, la tasa fue 16,6% (+39,8 pp; IC informado [39,5; 40,1] pp). Es una asociación descriptiva, no una causa.
  >
  > **Qué cambiaría.** Parche propuesto para el prompt de recepción de quejas: antes de sugerir una acción, resumir el motivo en una frase y pedir confirmación cuando la categoría sea ambigua. No añade permisos ni cambia decisiones financieras.
  >
  > **Efecto esperado.** Hipótesis direccional, no estimación: menos respuestas iniciales que no corresponden al motivo. El efecto sobre la resolución real no está medido.
  >
  > **Cómo se evaluará.** Comparar prompt base y candidato en escenarios sintéticos/adversariales etiquetados; medir clasificación correcta, respuesta pertinente, escalamiento seguro y cumplimiento de guardrails. Esto no demuestra una reducción de la tasa bancaria.
  >
  > **Riesgo.** Añadir una pregunta innecesaria o retrasar un caso urgente. Guardrail: máximo una confirmación y derivación segura ante urgencia o ambigüedad persistente.
  >
  > **Qué no cambia.** Identidad, acceso a herramientas, reglas de elegibilidad, compensaciones, aprobaciones y autoridad humana.

- `changes[0].docs.rationale` (ES): `La tasa agregada prioriza investigar el primer contacto por Queja. El dataset no explica el mecanismo; confirmar el motivo es una hipótesis de intervención que debe superar evaluación adversarial antes de cualquier aprobación.`
- `changes[0].docs.changelog` (ES): `Propuesta de cambio únicamente: añade una confirmación breve del motivo antes de responder. Aún no aprobado, publicado ni promovido.`
- `changes`: one synthetic prompt artifact delta, nonempty, with no source-derived text or identifiers.

**Português (variante equivalente para os mesmos campos)**

- `changes[0].docs.description` (PT-BR):

  > **Problema observado.** No snapshot auditado, 66.000 de 117.021 contatos classificados como Reclamação ficaram sem resolução no primeiro contato (56,4%); para os demais motivos, a taxa foi 16,6% (+39,8 pp; intervalo informado [39,5; 40,1] pp). É uma associação descritiva, não uma causa.
  >
  > **O que mudaria.** Patch proposto para o prompt de entrada de reclamações: antes de sugerir uma ação, resumir o motivo em uma frase e pedir confirmação quando a categoria for ambígua. Não adiciona permissões nem altera decisões financeiras.
  >
  > **Efeito esperado.** Hipótese direcional, não estimativa: menos respostas iniciais incompatíveis com o motivo. O efeito sobre a resolução real não foi medido.
  >
  > **Como será avaliado.** Comparar o prompt-base e o candidato em cenários sintéticos/adversariais rotulados; medir classificação correta, resposta pertinente, escalonamento seguro e cumprimento das guardrails. Isso não demonstra redução da taxa observada no banco.
  >
  > **Risco.** Fazer uma pergunta desnecessária ou atrasar um caso urgente. Guardrail: no máximo uma confirmação e encaminhamento seguro diante de urgência ou ambiguidade persistente.
  >
  > **O que não muda.** Identidade, acesso a ferramentas, regras de elegibilidade, compensações, aprovações e autoridade humana.

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

  > **Problema observado.** La evidencia agregada es la misma del dossier 1: Queja tiene 56,4% sin resolución en el primer contacto (66.000/117.021), frente a 16,6% para los demás motivos (+39,8 pp). La asociación no identifica una causa ni permite unir una PQR concreta con un contacto.
  >
  > **Qué cambiaría.** Propuesta sintética: permitir que el agente existente de disputas use `obtener_pqr` únicamente para leer de vuelta una PQR recién creada mediante su clave de idempotencia, como hace el flujo actual; resumir solo el estado devuelto y ofrecer el siguiente paso permitido. El contrato revisado no demuestra una capacidad de búsqueda histórica por cliente.
  >
  > **Efecto esperado.** Hipótesis no cuantificada: explicaciones más consistentes del estado y menos respuestas sin respaldo. No se predice una reducción del 39,8 pp.
  >
  > **Cómo se evaluará.** Escenarios sintéticos con PQR abierta, cerrada, no encontrada, respuesta inválida y timeout. Comparar exactitud del resumen, abstención ante ausencia/error, privacidad y escalamiento. Ninguna salida de estos tests mide la resolución bancaria.
  >
  > **Riesgo.** Exponer información de otra persona o interpretar mal un estado. Guardrail: conservar autorización por contexto, no buscar por identificadores aportados libremente y escalar ante falta de acceso o respuesta ambigua.
  >
  > **Qué no cambia.** La herramienta sigue siendo de solo lectura; no modifica PQR, transacciones, dinero, compensaciones ni decisiones.

- `changes[0].docs.rationale` (ES): `El hallazgo señala una población con más contactos no resueltos al primer intento, pero no demuestra que falte consulta de estado. El enlace de herramienta es una hipótesis verificable para la explicación del siguiente paso.`
- `changes[0].docs.changelog` (ES): `Propuesta de enlace de obtener_pqr al flujo acotado para leer de vuelta una PQR recién creada por su clave de idempotencia; no consulta PQR históricas ni agrega operaciones de escritura.`
- `changes`: one synthetic, schema-specific draft linking the existing tool; exact serialized shape must follow the tool/agent artifact schema and remain nonempty.

**Português (variante equivalente para os mesmos campos)**

- `changes[0].docs.description` (PT-BR):

  > **Problema observado.** A evidência agregada é a mesma do dossiê 1: Reclamação tem 56,4% sem resolução no primeiro contato (66.000/117.021), contra 16,6% para os demais motivos (+39,8 pp). A associação não identifica uma causa nem permite vincular uma manifestação específica a um contato.
  >
  > **O que mudaria.** Proposta sintética: permitir que o agente existente de disputas use `obtener_pqr` somente para ler de volta uma manifestação recém-criada por sua chave de idempotência, como faz o fluxo atual; resumir apenas o status retornado e oferecer o próximo passo permitido. O contrato revisado não comprova capacidade de busca histórica por cliente.
  >
  > **Efeito esperado.** Hipótese não quantificada: explicações de status mais consistentes e menos respostas sem respaldo. Não se prevê uma redução de 39,8 pp.
  >
  > **Como será avaliado.** Cenários sintéticos com manifestação aberta, fechada, não encontrada, resposta inválida e timeout. Comparar exatidão do resumo, abstenção diante de ausência/erro, privacidade e escalonamento. Esses testes não medem a resolução observada no banco.
  >
  > **Risco.** Expor informação de outra pessoa ou interpretar incorretamente um status. Guardrail: manter autorização vinculada ao contexto, não pesquisar por identificadores fornecidos livremente e escalar sem acesso ou diante de resposta ambígua.
  >
  > **O que não muda.** A ferramenta continua somente de leitura; não altera manifestações, transações, dinheiro, compensações nem decisões.

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

  > **Problema observado.** En el agregado auditado, la tasa de Queja sin resolución en el primer contacto fue 56,4% (66.000/117.021), frente a 16,6% para los demás motivos. Esto justifica priorizar una investigación acotada; no demuestra que falte un especialista ni explica las causas.
  >
  > **Qué cambiaría.** Propuesta sintética de un agente especialista que resume la solicitud, identifica si hay datos suficientes para el flujo existente y deriva a la ruta actual cuando el caso no coincide con su alcance. No ejecuta acciones de cuenta ni resuelve por sí mismo una disputa.
  >
  > **Efecto esperado.** Hipótesis direccional no medida: más consistencia de clasificación y derivación dentro del conjunto de escenarios cubiertos. No se estima lift en resolución ni ahorro.
  >
  > **Cómo se evaluará.** Suite sintética/adversarial con casos dentro/fuera de alcance, motivos ambiguos, datos faltantes, inyección de instrucciones y necesidad de humano. Comparar con el agente/ruta base en clasificación, no-respuesta segura, derivación correcta y guardrails. Reportar únicamente los resultados realmente ejecutados.
  >
  > **Riesgo.** Enrutamiento incorrecto o confianza excesiva que retrase ayuda humana. Guardrail: alcance explícito, abstención y escalamiento humano ante incertidumbre, urgencia o falta de autoridad.
  >
  > **Qué no cambia.** No se reemplaza el punto de entrada ni el proceso humano; no se agregan permisos, políticas, herramientas mutables ni publicación automática.

- `changes[0].docs.rationale` (ES): `La tasa observada identifica Queja como población prioritaria, pero los datos no incluyen una causa raíz. Un especialista nuevo es una alternativa de diseño, no una conclusión del análisis; debe compararse con el parche de prompt, el uso de herramientas existentes y no hacer cambios.`
- `changes[0].docs.changelog` (ES): `Propuesta hipotética de nuevo agente especializado y limitado, con abstención/escalamiento; no desplegado.`
- `changes`: one synthetic specialist-agent artifact draft with no real customer examples, identifiers, or bank-specific policy claims.

**Português (variante equivalente para os mesmos campos)**

- `changes[0].docs.description` (PT-BR):

  > **Problema observado.** No agregado auditado, a taxa de Reclamação sem resolução no primeiro contato foi 56,4% (66.000/117.021), contra 16,6% para os demais motivos. Isso justifica priorizar uma investigação limitada; não demonstra que falte um especialista nem explica as causas.
  >
  > **O que mudaria.** Proposta sintética de um agente especialista que resume a solicitação, identifica se há dados suficientes para o fluxo existente e encaminha para a rota atual quando o caso estiver fora do escopo. Ele não executa ações de conta nem resolve uma disputa por conta própria.
  >
  > **Efeito esperado.** Hipótese direcional não medida: mais consistência na classificação e no encaminhamento dentro dos cenários cobertos. Não se estima ganho de resolução nem economia.
  >
  > **Como será avaliado.** Suíte sintética/adversarial com casos dentro/fora do escopo, motivos ambíguos, dados ausentes, injeção de instruções e necessidade de atendimento humano. Comparar com o agente/rota-base em classificação, abstenção segura, encaminhamento correto e guardrails. Relatar apenas resultados realmente executados.
  >
  > **Risco.** Encaminhamento incorreto ou confiança excessiva que atrase o atendimento humano. Guardrail: escopo explícito, abstenção e escalonamento humano diante de incerteza, urgência ou falta de autoridade.
  >
  > **O que não muda.** O ponto de entrada e o processo humano são mantidos; não são adicionados permissões, políticas, ferramentas mutáveis nem publicação automática.

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
- each changed artifact has nonempty bodies for all seven labelled sections;
  evidence must contain at least two numeric values, an explicit comparison,
  and a source/snapshot marker. This is a mechanical completeness test, not
  claim verification or kind-specific artifact-schema validation;
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
  changes, the actual proposal-detail envelope, PII inside sibling artifact
  content, empty dossier sections, evidence without a comparator/source,
  prohibited lifecycle transitions, and quota boundary.

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
- No confirmed support-platform UI mapping.
- The read-only acceptance harness now exists with recorded-response tests.
  The proposal-detail GET exists and can verify current `state=draft`; it does
  not provide actor-attributed immutable lifecycle history. The pinned
  OpenAPI exposes no read-only quota endpoint. Even plausible supplied history
  and quota JSON remain `not_exercised` until a documented, proposal-/tenant-
  bound complete observation contract is available. Recorded fixtures never
  prove those runtime properties.
