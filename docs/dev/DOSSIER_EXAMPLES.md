# Decision dossier examples (W1-2)

Rendered offline by `reason_cli dossier` from the two REG1 live results (`scripts/regression/results/`) and the invented-count finding `scripts/regression/fixtures/finding_m4_pqr_status.json` (source `synthetic`). Runtime label `real` (REG1 ran against the local stack and gateway); rubric not computed (no reasoning record). Proposals are the compiled shape rebuilt in `seams/crates/reasoning/fixtures/dossier/`. Spanish first, Portuguese variant second. Command: `reason_cli dossier --proposal P --finding F --verdict V --runtime real --format md`.

## Format for the SPA proposal screen (ALIGN1)

The support-platform proposal screen shows each change's `docs.description`, `rationale` and `changelog` as plain text (`whitespace-pre-line`, PR #17 of the platform adds the class). The dossier therefore writes:

- `description`: first the decision line (never dropped), then one short `Label: text` line per section, separated by a blank line. No markdown at all (no `**`, no `#`, no tables, no code fences). Each section keeps its own cap (problem 220, evidence 440, change 400, result 400, measured 560, effect 240, measurement 260, risk 300, unchanged 200, labels 300, next step 220); the whole stays under 3,800 characters (agent-core allows 4,000).
- `title`: at most 120 characters (`TITLE_MAX`). The platform announce route rejects longer titles and never truncates, so the engine cuts at a word boundary (ES and PT), marks the cut with an ellipsis and keeps the full text in `title_full` (<= 300) and as the second line of the description (`Título completo: ...`, only when a cut happened). The decision line stays the first line. The registry draft title (`[improvement-engine] <target> <key>`) obeys the same 120 cap, cutting the target and keeping the key suffix.
- `rationale` (<= 1,500) and `changelog` (<= 1,500): plain sentences; diff entries are joined with `; `, not `|`.
- End step: a proposal patches an EXISTING agent (recepcion, disputas), so the human step is `Pasar a producción` (publish to staging, then promote to production from the agent page), never `Activar` (the first activation of a type). Announced dossiers carry a `Siguiente paso humano:` line (PT: `Passar para produção`) and the top-level `end_step` field; unannounced drafts carry neither.

Example of a cut title (ES, target `template:t/estado_pqr_con_un_nombre_extraordinariamente_largo_...`, 120 characters at most): the text ends at a whole word followed by `…`; the line `Título completo:` right after the decision line holds the complete one.

## t/estado_pqr live result (proven: attempt 1 failed, attempt 2 passed)

### template:t/estado_pqr - M4: propuesta de cambio

DECISIÓN: proponer a supervisión (suite de regresión probada).

Problema observado: M4 es persistentemente más alto en [category=Queja, channel=Phone, locale=es] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación.

Evidencia y comparación: Celda: 96/240 = 40,0 % frente a 20,0 % de la base del mismo canal (+20,0 pp); IC95 % de la tasa de la celda [34,0; 46,3] (Wilson). Replicación (holdout): 90/236 = 38,0 % frente a 20,0 %. Es una asociación descriptiva, no una causa.

Qué cambiaría: Parche anclado sobre template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."

Resultado base vs candidato: Suite de regresión probada: falla en la base y pasa con el candidato. La base falla 8 de 8 casos del hallazgo (3 guardas); intento 1 falló (8 casos); intento 2 pasó. GateItems: 15/15 aprobados.

Qué se midió: Cobertura no registrada en este veredicto (anterior a W13): véase el resultado.

Efecto esperado: Hipótesis direccional (decrease), no medida: llevar la tasa de la celda (40,0 %) hacia la referencia (20,0 %); mejora mínima buscada 10,0 pp. No es una predicción de efecto.

Cómo se evaluará: Después del lanzamiento, sobre una ventana nueva con el mismo k-anonimato, el estimador de resultado clasifica: improved / no_detectable_change / worsened / inconclusive (improved = baja de al menos 10,0 pp hacia la referencia). Guardrail: no more human escal…

Riesgo: The data says where the problem is, not why; a status line may not change the unresolved rate. Cascada: 1 entidad(es) pasan a la siguiente versión parche. Reversión: volver a la release base; solo staging.

Qué no cambia: Cláusulas protegidas del texto base intactas (el menú de anclas las excluye y el compilador verifica los marcadores); ningún presupuesto, modelo ni permiso cambia; solo se tocan los textos nombrados.

Etiquetas: Ejecución: real (stack local y gateway). Rúbrica: no calculado. Familia del juez: z-ai (not used here: deterministic checks only). Calibración: uncalibrated. Sensor: claude-standin. Reclamo: asociación, no causa. Datos: sintéticos (conteos inventados).

Siguiente paso humano: Pasar a producción: tras aprobar y publicar en staging, la persona de supervisión lo promueve a producción desde la página del agente. No es una activación inicial.

- rationale: The base sentence is static; the customer cannot tell the state of the PQR. Hipótesis de intervención sobre una asociación; debe superar la evaluación antes de cualquier aprobación.
- changelog: Solo propuesta, no aprobada ni publicada. Parche anclado sobre template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."
- announce: `true` (announce)

### template:t/estado_pqr - M4: proposta de mudança

DECISÃO: propor a supervisão (suite de regressão comprovada).

Problema observado: M4 é persistentemente mais alto em [category=Queja, channel=Phone, locale=es] do que em sua base de comparação; recorre na janela de descoberta e na de replicação.

Evidência e comparação: Célula: 96/240 = 40,0 % contra 20,0 % da base do mesmo canal (+20,0 pp); IC95 % da taxa da célula [34,0; 46,3] (Wilson). Replicação (holdout): 90/236 = 38,0 % contra 20,0 %. É uma associação descritiva, não uma causa.

O que mudaria: Patch ancorado em template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."

Resultado base vs candidato: Suite de regressão comprovada: falha na base e passa com o candidato. A base falha 8 de 8 casos do achado (3 guardas); tentativa 1 falhou (8 casos); tentativa 2 passou. GateItems: 15/15 aprovados.

O que foi medido: Cobertura não registrada neste veredito (anterior ao W13): veja o resultado.

Efeito esperado: Hipótese direcional (decrease), não medida: levar a taxa da célula (40,0 %) para a referência (20,0 %); melhora mínima buscada 10,0 pp. Não é uma previsão de efeito.

Como será avaliado: Após o lançamento, em uma janela nova com o mesmo k-anonimato, o estimador de resultado classifica: improved / no_detectable_change / worsened / inconclusive (improved = queda de ao menos 10,0 pp rumo à referência). Guardrail: no more human escalations than t…

Risco: The data says where the problem is, not why; a status line may not change the unresolved rate. Cascata: 1 entidade(s) passam para a próxima versão patch. Reversão: voltar a release base; somente staging.

O que não muda: Cláusulas protegidas do texto base intactas (o menú de âncoras as exclui e o compilador verifica os marcadores); nenhum orçamento, modelo ou permissão muda; so os textos nomeados sao tocados.

Rótulos: Execução: real (stack local e gateway). Rúbrica: não calculado. Família do juiz: z-ai (not used here: deterministic checks only). Calibração: uncalibrated. Sensor: claude-standin. Alegação: associação, não causa. Dados: sintéticos (contagens inventadas).

Próximo passo humano: Passar para produção: após aprovar e publicar em staging, a pessoa de supervisão promove para produção na página do agente. Não é uma ativação inicial.

- rationale: The base sentence is static; the customer cannot tell the state of the PQR. Hipótese de intervenção sobre uma associação; deve passar na avaliação antes de qualquer aprovação.
- changelog: Somente proposta, não aprovada nem publicada. Patch ancorado em template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."
- announce: `true` (announce)

## p/resumen_radicado live result (proven: attempt 1 failed, attempt 2 passed)

### prompt:p/resumen_radicado - M4: propuesta de cambio

DECISIÓN: proponer a supervisión (suite de regresión probada).

Problema observado: M4 es persistentemente más alto en [category=Queja, channel=Phone, locale=es] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación.

Evidencia y comparación: Celda: 96/240 = 40,0 % frente a 20,0 % de la base del mismo canal (+20,0 pp); IC95 % de la tasa de la celda [34,0; 46,3] (Wilson). Replicación (holdout): 90/236 = 38,0 % frente a 20,0 %. Es una asociación descriptiva, no una causa.

Qué cambiaría: Parche anclado sobre prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."

Resultado base vs candidato: Suite de regresión probada: falla en la base y pasa con el candidato. La base falla 6 de 6 casos del hallazgo (3 guardas); intento 1 falló (6 casos); intento 2 pasó. GateItems: 13/13 aprobados.

Qué se midió: Cobertura no registrada en este veredicto (anterior a W13): véase el resultado.

Efecto esperado: Hipótesis direccional (decrease), no medida: llevar la tasa de la celda (40,0 %) hacia la referencia (20,0 %); mejora mínima buscada 10,0 pp. No es una predicción de efecto.

Cómo se evaluará: Después del lanzamiento, sobre una ventana nueva con el mismo k-anonimato, el estimador de resultado clasifica: improved / no_detectable_change / worsened / inconclusive (improved = baja de al menos 10,0 pp hacia la referencia). Guardrail: no more human escal…

Riesgo: The closing wording is generated by a model; the probe samples three generations per case. Cascada: 1 entidad(es) pasan a la siguiente versión parche. Reversión: volver a la release base; solo staging.

Qué no cambia: Cláusulas protegidas del texto base intactas (el menú de anclas las excluye y el compilador verifica los marcadores); ningún presupuesto, modelo ni permiso cambia; solo se tocan los textos nombrados.

Etiquetas: Ejecución: real (stack local y gateway). Rúbrica: no calculado. Familia del juez: z-ai (not used here: deterministic checks only). Calibración: uncalibrated. Sensor: claude-standin. Reclamo: asociación, no causa. Datos: sintéticos (conteos inventados).

Siguiente paso humano: Pasar a producción: tras aprobar y publicar en staging, la persona de supervisión lo promueve a producción desde la página del agente. No es una activación inicial.

- rationale: The closing message never says who follows the case; naming the follow-up closes the loop for the customer. Hipótesis de intervención sobre una asociación; debe superar la evaluación antes de cualquier aprobación.
- changelog: Solo propuesta, no aprobada ni publicada. Parche anclado sobre prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."
- announce: `true` (announce)

### prompt:p/resumen_radicado - M4: proposta de mudança

DECISÃO: propor a supervisão (suite de regressão comprovada).

Problema observado: M4 é persistentemente mais alto em [category=Queja, channel=Phone, locale=es] do que em sua base de comparação; recorre na janela de descoberta e na de replicação.

Evidência e comparação: Célula: 96/240 = 40,0 % contra 20,0 % da base do mesmo canal (+20,0 pp); IC95 % da taxa da célula [34,0; 46,3] (Wilson). Replicação (holdout): 90/236 = 38,0 % contra 20,0 %. É uma associação descritiva, não uma causa.

O que mudaria: Patch ancorado em prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."

Resultado base vs candidato: Suite de regressão comprovada: falha na base e passa com o candidato. A base falha 6 de 6 casos do achado (3 guardas); tentativa 1 falhou (6 casos); tentativa 2 passou. GateItems: 13/13 aprovados.

O que foi medido: Cobertura não registrada neste veredito (anterior ao W13): veja o resultado.

Efeito esperado: Hipótese direcional (decrease), não medida: levar a taxa da célula (40,0 %) para a referência (20,0 %); melhora mínima buscada 10,0 pp. Não é uma previsão de efeito.

Como será avaliado: Após o lançamento, em uma janela nova com o mesmo k-anonimato, o estimador de resultado classifica: improved / no_detectable_change / worsened / inconclusive (improved = queda de ao menos 10,0 pp rumo à referência). Guardrail: no more human escalations than t…

Risco: The closing wording is generated by a model; the probe samples three generations per case. Cascata: 1 entidade(s) passam para a próxima versão patch. Reversão: voltar a release base; somente staging.

O que não muda: Cláusulas protegidas do texto base intactas (o menú de âncoras as exclui e o compilador verifica os marcadores); nenhum orçamento, modelo ou permissão muda; so os textos nomeados sao tocados.

Rótulos: Execução: real (stack local e gateway). Rúbrica: não calculado. Família do juiz: z-ai (not used here: deterministic checks only). Calibração: uncalibrated. Sensor: claude-standin. Alegação: associação, não causa. Dados: sintéticos (contagens inventadas).

Próximo passo humano: Passar para produção: após aprovar e publicar em staging, a pessoa de supervisão promove para produção na página do agente. Não é uma ativação inicial.

- rationale: The closing message never says who follows the case; naming the follow-up closes the loop for the customer. Hipótese de intervenção sobre uma associação; deve passar na avaliação antes de qualquer aprovação.
- changelog: Somente proposta, não aprovada nem publicada. Patch ancorado em prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."
- announce: `true` (announce)

## t/estado_pqr negative no-op (not_fixed: never announced)

### template:t/estado_pqr - M4: propuesta de cambio

DECISIÓN: NO SE ANUNCIA; queda como borrador interno.

Problema observado: M4 es persistentemente más alto en [category=Queja, channel=Phone, locale=es] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación.

Evidencia y comparación: Celda: 96/240 = 40,0 % frente a 20,0 % de la base del mismo canal (+20,0 pp); IC95 % de la tasa de la celda [34,0; 46,3] (Wilson). Replicación (holdout): 90/236 = 38,0 % frente a 20,0 %. Es una asociación descriptiva, no una causa.

Qué cambiaría: Parche anclado sobre template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."

Resultado base vs candidato: NO CORREGIDO: el candidato sigue fallando casos del hallazgo. No se anuncia. La base falla 8 de 8 casos del hallazgo (3 guardas); intento 1 falló (8 casos). GateItems: 15/15 aprobados.

Qué se midió: Cobertura no registrada en este veredicto (anterior a W13): véase el resultado.

Efecto esperado: Hipótesis direccional (decrease), no medida: llevar la tasa de la celda (40,0 %) hacia la referencia (20,0 %); mejora mínima buscada 10,0 pp. No es una predicción de efecto.

Cómo se evaluará: Después del lanzamiento, sobre una ventana nueva con el mismo k-anonimato, el estimador de resultado clasifica: improved / no_detectable_change / worsened / inconclusive (improved = baja de al menos 10,0 pp hacia la referencia). Guardrail: no more human escal…

Riesgo: The data says where the problem is, not why; a status line may not change the unresolved rate. Cascada: 1 entidad(es) pasan a la siguiente versión parche. Reversión: volver a la release base; solo staging.

Qué no cambia: Cláusulas protegidas del texto base intactas (el menú de anclas las excluye y el compilador verifica los marcadores); ningún presupuesto, modelo ni permiso cambia; solo se tocan los textos nombrados.

Etiquetas: Ejecución: real (stack local y gateway). Rúbrica: no calculado. Familia del juez: z-ai (not used here: deterministic checks only). Calibración: uncalibrated. Sensor: claude-standin. Reclamo: asociación, no causa. Datos: sintéticos (conteos inventados).

- rationale: The base sentence is static; the customer cannot tell the state of the PQR. Hipótesis de intervención sobre una asociación; debe superar la evaluación antes de cualquier aprobación.
- changelog: Solo propuesta, no aprobada ni publicada. Parche anclado sobre template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."
- announce: `false` (not_fixed)

### template:t/estado_pqr - M4: proposta de mudança

DECISÃO: NÃO E ANUNCIADA; fica como rascunho interno.

Problema observado: M4 é persistentemente mais alto em [category=Queja, channel=Phone, locale=es] do que em sua base de comparação; recorre na janela de descoberta e na de replicação.

Evidência e comparação: Célula: 96/240 = 40,0 % contra 20,0 % da base do mesmo canal (+20,0 pp); IC95 % da taxa da célula [34,0; 46,3] (Wilson). Replicação (holdout): 90/236 = 38,0 % contra 20,0 %. É uma associação descritiva, não uma causa.

O que mudaria: Patch ancorado em template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."

Resultado base vs candidato: NÃO CORRIGIDO: o candidato continua falhando casos do achado. Não e anunciada. A base falha 8 de 8 casos do achado (3 guardas); tentativa 1 falhou (8 casos). GateItems: 15/15 aprovados.

O que foi medido: Cobertura não registrada neste veredito (anterior ao W13): veja o resultado.

Efeito esperado: Hipótese direcional (decrease), não medida: levar a taxa da célula (40,0 %) para a referência (20,0 %); melhora mínima buscada 10,0 pp. Não é uma previsão de efeito.

Como será avaliado: Após o lançamento, em uma janela nova com o mesmo k-anonimato, o estimador de resultado classifica: improved / no_detectable_change / worsened / inconclusive (improved = queda de ao menos 10,0 pp rumo à referência). Guardrail: no more human escalations than t…

Risco: The data says where the problem is, not why; a status line may not change the unresolved rate. Cascata: 1 entidade(s) passam para a próxima versão patch. Reversão: voltar a release base; somente staging.

O que não muda: Cláusulas protegidas do texto base intactas (o menú de âncoras as exclui e o compilador verifica os marcadores); nenhum orçamento, modelo ou permissão muda; so os textos nomeados sao tocados.

Rótulos: Execução: real (stack local e gateway). Rúbrica: não calculado. Família do juiz: z-ai (not used here: deterministic checks only). Calibração: uncalibrated. Sensor: claude-standin. Alegação: associação, não causa. Dados: sintéticos (contagens inventadas).

- rationale: The base sentence is static; the customer cannot tell the state of the PQR. Hipótese de intervenção sobre uma associação; deve passar na avaliação antes de qualquer aprovação.
- changelog: Somente proposta, não aprovada nem publicada. Patch ancorado em template:t/estado_pqr (104 caracteres editados de 240): [es] replace s1: "Ya consulté tu PQR." -> "Ya consulté tu PQR. Su estado actual es: {{ facts.pqr.value.status }}."; [pt] replace s1: "Já consultei sua solicitação." -> "Já consultei sua solicitação. O estado atual é: {{ facts.pqr.value.status }}."
- announce: `false` (not_fixed)

## p/resumen_radicado native-only (non_discriminating: never announced)

### prompt:p/resumen_radicado - M4: propuesta de cambio

DECISIÓN: NO SE ANUNCIA; queda como borrador interno.

Problema observado: M4 es persistentemente más alto en [category=Queja, channel=Phone, locale=es] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación.

Evidencia y comparación: Celda: 96/240 = 40,0 % frente a 20,0 % de la base del mismo canal (+20,0 pp); IC95 % de la tasa de la celda [34,0; 46,3] (Wilson). Replicación (holdout): 90/236 = 38,0 % frente a 20,0 %. Es una asociación descriptiva, no una causa.

Qué cambiaría: Parche anclado sobre prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."

Resultado base vs candidato: NO DISCRIMINANTE: la base ya pasa todos los casos; la suite no captura el problema y no es una suite de regresión. No se anuncia. La base falla 0 de 6 casos del hallazgo (3 guardas); sin candidato evaluado. GateItems: 13/13 aprobados.

Qué se midió: Cobertura no registrada en este veredicto (anterior a W13): véase el resultado.

Efecto esperado: Hipótesis direccional (decrease), no medida: llevar la tasa de la celda (40,0 %) hacia la referencia (20,0 %); mejora mínima buscada 10,0 pp. No es una predicción de efecto.

Cómo se evaluará: Después del lanzamiento, sobre una ventana nueva con el mismo k-anonimato, el estimador de resultado clasifica: improved / no_detectable_change / worsened / inconclusive (improved = baja de al menos 10,0 pp hacia la referencia). Guardrail: no more human escal…

Riesgo: The closing wording is generated by a model; the probe samples three generations per case. Cascada: 1 entidad(es) pasan a la siguiente versión parche. Reversión: volver a la release base; solo staging.

Qué no cambia: Cláusulas protegidas del texto base intactas (el menú de anclas las excluye y el compilador verifica los marcadores); ningún presupuesto, modelo ni permiso cambia; solo se tocan los textos nombrados.

Etiquetas: Ejecución: real (stack local y gateway). Rúbrica: no calculado. Familia del juez: z-ai (not used here: deterministic checks only). Calibración: uncalibrated. Sensor: claude-standin. Reclamo: asociación, no causa. Datos: sintéticos (conteos inventados).

- rationale: The closing message never says who follows the case; naming the follow-up closes the loop for the customer. Hipótesis de intervención sobre una asociación; debe superar la evaluación antes de cualquier aprobación.
- changelog: Solo propuesta, no aprobada ni publicada. Parche anclado sobre prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."
- announce: `false` (non_discriminating)

### prompt:p/resumen_radicado - M4: proposta de mudança

DECISÃO: NÃO E ANUNCIADA; fica como rascunho interno.

Problema observado: M4 é persistentemente mais alto em [category=Queja, channel=Phone, locale=es] do que em sua base de comparação; recorre na janela de descoberta e na de replicação.

Evidência e comparação: Célula: 96/240 = 40,0 % contra 20,0 % da base do mesmo canal (+20,0 pp); IC95 % da taxa da célula [34,0; 46,3] (Wilson). Replicação (holdout): 90/236 = 38,0 % contra 20,0 %. É uma associação descritiva, não uma causa.

O que mudaria: Patch ancorado em prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."

Resultado base vs candidato: NÃO DISCRIMINANTE: a base ja passa todos os casos; a suite não captura o problema e não é uma suite de regressão. Não e anunciada. A base falha 0 de 6 casos do achado (3 guardas); sem candidato avaliado. GateItems: 13/13 aprovados.

O que foi medido: Cobertura não registrada neste veredito (anterior ao W13): veja o resultado.

Efeito esperado: Hipótese direcional (decrease), não medida: levar a taxa da célula (40,0 %) para a referência (20,0 %); melhora mínima buscada 10,0 pp. Não é uma previsão de efeito.

Como será avaliado: Após o lançamento, em uma janela nova com o mesmo k-anonimato, o estimador de resultado classifica: improved / no_detectable_change / worsened / inconclusive (improved = queda de ao menos 10,0 pp rumo à referência). Guardrail: no more human escalations than t…

Risco: The closing wording is generated by a model; the probe samples three generations per case. Cascata: 1 entidade(s) passam para a próxima versão patch. Reversão: voltar a release base; somente staging.

O que não muda: Cláusulas protegidas do texto base intactas (o menú de âncoras as exclui e o compilador verifica os marcadores); nenhum orçamento, modelo ou permissão muda; so os textos nomeados sao tocados.

Rótulos: Execução: real (stack local e gateway). Rúbrica: não calculado. Família do juiz: z-ai (not used here: deterministic checks only). Calibração: uncalibrated. Sensor: claude-standin. Alegação: associação, não causa. Dados: sintéticos (contagens inventadas).

- rationale: The closing message never says who follows the case; naming the follow-up closes the loop for the customer. Hipótese de intervenção sobre uma associação; deve passar na avaliação antes de qualquer aprovação.
- changelog: Somente proposta, não aprovada nem publicada. Patch ancorado em prompt:p/resumen_radicado (172 caracteres editados de 240): [es] insert_after s2: "que recibirá novedades." -> "Indica que un especialista del equipo le dará seguimiento a su caso hasta cerrarlo."; [pt] insert_after s2: "que receberá novidades." -> "Informe que um especialista da equipe fará o acompanhamento do caso até o encerramento."
- announce: `false` (non_discriminating)
