# U20-E — composición sellada de oracle de seguridad E0

U20-E no ejecuta candidatos, no llama Agent Core y no implementa una nueva
autenticación. Produce `E0SafetyOracle`, una capacidad opaca que une tres
hechos ya verificados: la disponibilidad de replay U04-B
(`tenant/world/cutoff/snapshot/profile`), las cuatro revisiones exactas del
plan U20 (baseline, oracle, suites development/final) y el binding de fixture
protegido de U36 (`tenant/namespace/fixture/case/channel/policy/questions` y
vigencia). El compromiso resultante también incluye cada identidad completa de
artefacto, no sólo su digest de contenido.

La costura nueva de U36 es crate-private y devuelve únicamente compromisos de
la policy del fixture. Excluye principals permitidos, respuestas, nonces y
evidencia de identidad; el sandbox sigue siendo el único componente que puede
comprobar una prueba concreta. La costura equivalente de U20 devuelve refs
selladas y no convierte un `EvaluationPlan` en autoridad de ejecución.

El composer exige mismo tenant para replay, fixture y los cinco refs, digest
del snapshot U20 igual al snapshot U04-B y policy válida estrictamente después
del cutoff de replay. Cualquier divergencia falla antes de crear la capacidad.
Una revalidación read-only vuelve a sellar el contexto y falla si cambia cutoff,
perfil, policy, questions o cualquier referencia exacta del plan.

`ExpectedIdentityCheck` es interno. `Verified` se clasifica como `Safe`;
ausencia, mismatch, expiración o revocación son `Unsafe`; dependencia no
disponible es `Unknown`. Ninguno de esos estados es un resultado de evaluación
ni autoriza release; `E0SafetyOracle::authorizes_execution_or_release()` es
siempre falso. U19/U23 deberán aportar el adaptador de observación U36 y el
runner separado.

El RED inicial fue un import externo de `e0_safety_oracle` que no resolvía.
Las regresiones cubren contexto correcto, tenant/snapshot/policy expirada,
drift de profile/policy/questions/cutoff y ref exacta del oracle U20, además de
unsafe/unknown. Los doctests bloquean construcción pública y `Debug` de la
capacidad. No se añaden respuestas de identidad, PII, runtime, efectos
sandbox, lectura de filas ni publicación.
