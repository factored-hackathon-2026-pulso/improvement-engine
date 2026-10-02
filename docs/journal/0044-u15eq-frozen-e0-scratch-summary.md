# U15-EQ — preparación scratch de resumen Frozen E0

## Propósito y límite

U15-EQ prepara un único resumen estático dentro del workspace efímero U15.
No publica una página, no actualiza un head, no emite `MemoryUse`, no crea un
artefacto, ni concluye valor, causalidad, outcome o elegibilidad. La eventual
publicación es responsabilidad de U33-E y cualquier enlace de uso gobernado
con U23-E sigue pendiente.

La única composición es crate-private. Recibe la calificación opaca U14-EQ,
el candidato admitido U13-A, reporte U14-E, replay opaco U04-B y el scope/
access ya emitido por composición confiable. Antes de montar U15 vuelve a
rehidratar y recomputar candidato, verificación y calificación; también exige
que tenant, mundo, cutoff, raw-source binding y availability profile del replay
coincidan con la proveniencia E0 sellada.

## Contrato de scratch

- Scope canónico: `investigation/world_a/e0_diagnostic/frozen/train` en la
  fixture, con `frozen` y `train` obligatorios; en producción mundo y tenant
  provienen del replay, no de texto del cliente.
- `WikiAccess` debe tener el mismo tenant, grant, run, scope y purpose, y
  `allowed_at_unix_seconds == replay.cutoff_at_unix_seconds`.
- Sólo existe una operación: `WikiTransformOperation::Create` en una ruta
  constante con contenido literal. No entra texto caller, filas, páginas,
  PII, outcomes, valores o afirmaciones causales al transform.
- La salida `PreparedFrozenE0MemorySummary` no implementa `Debug` y expone
  únicamente commitments/recibos derivados; nunca workspace, páginas, ruta o
  texto. Tampoco autoriza publicación o uso de memoria.
- Si ya existe la ruta canónica `prepared/frozen-e0-summary.md`, `Create`
  devuelve `PageAlreadyExists`: no hay output preparado, `Replace`, reutilización
  del texto anterior ni resultado de transform observable.

## RED → GREEN

El RED fue la ausencia de frontera U15-EQ. El GREEN agrega una prueba E2E que
parte del helper real U02 raw bytes → U04-B replay → U08 ledger → U12-E →
U13-E/U13-A → U14-E → U14-EQ y sólo entonces ejecuta el transform U15 real.
La companion replay no es una fixture nominal: se produce por el mismo helper
que construye el resultado U08. Negativos de access-time y protocolo usan un
port que hace panic si se intenta montar o transformar, comprobando que el
rechazo ocurre antes de cualquier efecto scratch. Dos doctests compile-fail
cierran la composición y cualquier `Debug` del output.

## Fuera de alcance

No hay U33 publish/head/artifact, U22 `MemoryUse`, U23 temporal admission,
U16, LLM/Jev, Agent Core, runtime, HTTP o release. La raíz de composición del
servicio debe conectar este recibo opaco con U33-E/U23-E sin abrir los
bytes ni cambiar las precondiciones Frozen.
