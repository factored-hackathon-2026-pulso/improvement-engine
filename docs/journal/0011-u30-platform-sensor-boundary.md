# U30: sensor determinista de observaciones de plataforma

## Estado

U30 se inició con un test RED de contrato y, tras el merge de U29, fue
rebasado sobre `origin/main` `ba0d159`. El RED de comportamiento está en
`crates/core/tests/platform_sensor.rs`.

El test permanece RED hasta que exista U30. Un sensor U30 que reconstruyera
eventos, cobertura o persistencia para hacerlo verde duplicaría la frontera
de ingestión y podría convertir una fuente parcial en denominador.

## Contrato de integración mínimo solicitado a U29

U30 consume exclusivamente una `WindowProjection` obtenida por
`ObservationRepository::window_projection`. No consume batches, blobs ni
filas PostgreSQL directamente. Para producir una señal por capa, U29 debe
exponer una vista de lectura que preserve el commitment de la proyección y
permita, sin campos mutables ni deserialización externa:

1. identificar `target_system`, `evidence_kind`, `event_kind`, `layer`,
   `source_run_ref` y clocks de cada `ObservedEvent`;
2. obtener el `BoundDenominator` sólo por
   `CoverageEvidence::denominator_for(observed_event)`;
3. asociar el denominador a su `batch_digest` y commitment de cobertura,
   sin sumar poblaciones de batches solapados;
4. hacer visible la semántica declarada de `population_ref` o, si el contrato
   no acredita que la población es elegible para la métrica, devolver
   `insufficient_evidence`.

U29 valida que el denominador sea `Attention + PlatformAudit + complete` y
coincida exactamente en tenant/fuente/contrato/batch. U30 añadirá sólo los
getters/builder de lectura necesarios para consumir ese contrato; no añade
otro esquema de observaciones.

## Primer comportamiento tras la integración

La primera métrica implementada es `handoff_rate@1` por `Layer` para
`target_system=attention`. Cuenta únicamente eventos durables `PlatformAudit`
con capa conocida y deduplica un reintento por `source_run_ref`. Cada señal
conserva: numerador, denominador acreditado, missing, versión de métrica,
`projection_digest`, fuente/contrato, `batch_digest` y
`coverage_evidence_digest`, ventana y `received_as_of_ms`. El cutoff también
forma parte del commitment de `WindowProjection`, aun si dos lecturas muestran
los mismos bytes. `evolution`, CoreAudit, OTel muestreado,
cobertura `partial|unknown|degraded`, capa desconocida, batches superpuestos
o población semánticamente incompatible producen un resultado explícito
`insufficient_evidence`/`unsupported`; nunca una tasa cero o un fallo de
atención.

La revisión adversarial detectó tres límites de autoridad que el primer verde
no cubría: los campos públicos permitían reconstruir una señal con un digest
elegido por el llamador; `population_ref` no comprometía el digest; y una
fuente registrada podía aparentar ser elegible sin una semántica de métrica
revisada. La reparación deja `PlatformLayerSignal` sellada con getters de
lectura y un doctest `compile_fail`; incluye `population_ref` aun para un
resultado insuficiente; y configura el sensor desde un
`TrustedLayerMetricMapping` inmutable que liga
`metric_id/version/layer/population_ref` con exactamente un
`source_id/contract_ref`. Una fuente U29 registrada pero no mapeada no puede
medir. Un denominador completo igual a cero también resulta
`insufficient_evidence`, nunca una tasa medida.
El commitment de la señal también incluye `metric_mapping_digest`; por tanto,
un cambio de mapping de despliegue no puede producir la misma señal aunque la
proyección no contenga evidencia elegible.

La segunda revisión adversarial cerró dos huecos: no basta con llamar a un
mapping "trusted" si cualquier consumidor público puede crearlo, y un mapping
ausente no puede dejar el resultado sin commitment de decisión. La composición
es ahora privada: el único constructor público es
`PlatformLayerSensor::for_platform_audit()`, que carga el registro de despliegue
revisado. Un doctest `compile_fail` prueba que un consumidor externo no puede
registrar una fuente/contrato/población arbitraria. Además de conservar el
`metric_mapping_digest` opcional para el mapping resuelto, toda señal expone el
`mapping_resolution_digest` no opcional. Este receipt liga
`spec + projection_digest + registry_digest + mapped|missing + mapping_digest`;
así, una ausencia es explícita y un cambio de presencia o configuración del
registro cambia el receipt sin filtrar la configuración ni los eventos crudos.

La selección de una oportunidad, SQL, LLM, write de artefactos y promoción
no pertenecen a U30.

## Verificación ejecutada

- El RED anterior falló por `E0432` mientras U29 no estaba en `main`.
- El RED actual fija: deduplicación por run, exclusión de `evolution`,
  cobertura parcial/incompatible, y rechazo de dos denominadores completos
  potencialmente solapados. También fija versión positiva y reproducibilidad
  exacta de la señal. Regresiones adversariales añadidas: capa
  `Unknown` o una capa omitida en el mismo batch bloquea la tasa y dos cutoffs
  con idénticos eventos generan commitments distintos. La reparación añadió
  regresiones para fuente/contrato registrado pero no mapeado, denominador
  cero, dos especificaciones de población distintas sin evidencia y el
  compile-fail de no forja. La reparación P1 añadió dos pruebas unitarias: el
  receipt de mapping ausente cambia frente a un registro vacío, a un mapping
  de contrato distinto y a una resolución presente; el doctest ahora prueba
  que no se puede crear ni registrar un mapping externo. Pasaron:
  `cargo +1.98.1 fmt --all -- --check`, `cargo +1.98.1 test -p
  improvement-engine-core --lib platform_sensor` (2/2), `cargo +1.98.1 test
  -p improvement-engine-core --test platform_sensor` (9/9), `cargo +1.98.1
  test -p improvement-engine-core --doc` (2/2), y `cargo +1.98.1 clippy -p
  improvement-engine-core --all-targets -- -D warnings`. Falta una revisión
  adversarial independiente, la suite completa y rebase antes de integrar el
  commit en la PR acumulativa #44; no se abrirá una PR de feature.
