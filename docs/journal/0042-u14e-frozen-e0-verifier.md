# U14-E — verificador de consistencia Frozen E0

## Propósito y límite

U14-E es una ruta adicional a U14 genérico, no una sustitución. Consume sólo
un `VerifiedScoutCandidate` ya admitido por U13-A y vuelve a rehidratar el
registro canónico antes de aceptar su proveniencia E0. La capability resultante
es crate-private: no se puede formar desde un `ScoutCandidateDraft`, un receipt
arbitrario, `QueryResult`, strings de scope ni una policy aportada por caller.

El verificador `FrozenE0IndependentVerifier` ejecuta la policy sellada y
versionada `frozen_e0_provenance_consistency@1`. Confirma consistencia interna
de la evidencia Frozen: scope, snapshot, binding/profiling/cutoff,
ventana, agregados, tabla/campos, fuente/transform/proyección/evidencia y
receipts; además fija la semántica permitida de la métrica E0. La salida es un
`FrozenE0VerificationReport` opaco ligado a scope, candidate, proveniencia E0,
policy, input, evidencia y snapshot mediante commitments.

`Consistent` significa solamente "la proveniencia Frozen rehidratada fue
consistente". No significa `Supported`, causa, outcome, ahorro, calidad de
modelo, elegibilidad, aprobación, ejecución o release. No existe port
pluggable, storage, runtime, Agent Core ni efecto mutable en esta slice.

## RED → GREEN y pruebas

El RED inicial confirmó que el nuevo módulo necesitaba importar la referencia
de artefacto y serializar explícitamente las cuatro dimensiones de scope (el
scope no es serializable por contrato). El GREEN conserva el scope tipado en
memoria y lo compromete canónicamente por sus campos.

La prueba E2E usa la cadena real U02 raw JSON → U04-B V2 → U08 governed ledger
→ U12-E measurement → U13-E → U13-A → U14-E. Se verifica que repetir la
verificación sólo devuelve commitments idénticos (no hay escritura) y que el
status no es causal. Un candidato U13 genérico sin proveniencia E0 se rechaza;
los doctests compile-fail cubren construcción de report y receipt/port caller.

## Fuera de alcance

No produce el `VerificationReport` genérico, no llama Jev/LLM, no crea tests,
no registra resultados, no promueve artefactos ni conecta U16/U20/U35. La
corroboración independiente de hipótesis/causas sigue siendo responsabilidad
de U14 genérico y sus futuros adapters.
