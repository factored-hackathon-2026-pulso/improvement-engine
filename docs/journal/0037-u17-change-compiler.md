# U17 — compilador de cambios autorizados y sellados

U17 convierte exclusivamente un `AuthorizedChangeSpec` en un `EntityDraft`
inmutable. La frontera pública empieza en `UntrustedChangeSpec`: cualquiera
puede describir el cambio que propone, pero no puede fabricar la capacidad que
lo compila. `AuthorizedChangeSpec`, `CompilationAuthorization` y el
compositor `TrustedChangeAuthorizer` no exponen constructores públicos.

La composición confiable interna exige una decisión U35 `Eligible` y el bridge
U16 real con grado `MechanismProxy`. Sella tenant, scope completo,
`ArtifactReference` del snapshot, commitment de bridge y plan, ruta,
mecanismo, tipo/operación Core, identidad, versión y digest del cuerpo exactos
de entidad, y una precondición ejecutable tipada. Para `add` es
`EntityAbsent { kind, entity_id }`; para una futura sustitución existe el
contrato `RevisionEquals { kind, entity_id, entity_version, content_digest }`.
El digest de la precondición acompaña sus bytes canónicos como evidencia, pero
no sustituye el predicado que U18 debe evaluar atómicamente. El compilador
vuelve a cotejar esos valores
antes de materializar el draft; por tanto ni un cambio posterior a la
autorización ni un commitment público leído por un consumidor equivalen a
autoridad.

El RED inicial fue `crates/core/tests/change_compiler.rs`: el import de
`change_compiler` no resolvía porque el módulo no existía. Las regresiones
cubren tenant, scope, snapshot, ruta, mecanismo, tipo, operación, identidad,
versión, cuerpo Flow y precondición divergentes, además de una mutación interna
simulada después de sellar. Los doctests `compile_fail` prueban que un
consumidor no puede construir (ni por struct literal) el spec autorizado ni el
compositor confiable.

Este corte acepta una sola operación `add` sobre un `Flow` Core mínimo:
identificador canónico, semver acotado y canónico, prioridad no negativa y
nodos `end` únicos sin transiciones contradictorias. `replace`/`disable`, los
demás EntityKinds y la clausura completa de schemas Core son cortes futuros;
fallan cerrados ahora. U17 no escribe registry, no ejecuta Core, no evalúa ni
libera: U18 y cortes posteriores revalidarán atómicamente los metadatos
preservados en el draft antes de cualquier efecto mutable.

Decisión abierta explícita: el compositor crate-private es una frontera
confiable temporal hasta que exista el adaptador gobernado de política/autoridad
de U18. La capacidad ya es opaca para consumidores, pero la autoridad externa
persistida, el repositorio y la evaluación de `EntityAbsent`/`RevisionEquals`
en el mismo conditional write no pertenecen a este corte y no deben inferirse
de getters de commitments.
