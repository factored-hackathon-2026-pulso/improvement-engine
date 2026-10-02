# U17 — compiler de cambios sellados

U17 introduce una frontera pura `change_compiler`: sólo convierte una
`FinalEligibilityDecision::Eligible` y un `ChangeSpec` con commitments exactos
de bridge/plan en `EntityDraft` inmutables. La identidad de readiness se coteja
antes de leer operaciones; una decisión inelegible o un commitment distinto
falla cerrada.

El primer RED fue `crates/core/tests/change_compiler.rs`: el import de
`change_compiler` no resolvía porque el módulo no existía. El primer GREEN
cubre el rechazo de readiness inelegible. La integración interna compone una
tripleta real U14→U16→U20→U35 y prueba que sólo esa readiness produce un draft
con digest determinista; una variación de commitment queda bloqueada.

Este corte sólo soporta operaciones `add` de un `Flow` Core mínimo: prioridad,
al menos un nodo `end` y un outcome permitido por el pin 0.5.0. Valida digest
de precondición, identidad de entidad y versión semver. `replace`/`disable`,
otros EntityKinds, resolver la clausura completa Core y validar el schema
upstream completo son extensiones que deben ser cortes posteriores; no se
aceptan silenciosamente. U17 no escribe al registry, no ejecuta Core, no evalúa
y no libera. Esas responsabilidades quedan en U18/U19/U21.
