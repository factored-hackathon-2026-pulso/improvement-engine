# 0040 — U02: payload canónico del snapshot de fuente

## Decisión

Para todos los artefactos U02 de tipo `SourceSnapshot`, el payload canónico
debe contener exactamente un campo de transporte de procedencia:

```json
{"source_snapshot_json":"<bytes UTF-8 crudos exactos del SourceSnapshot>"}
```

`source_snapshot_json` conserva el texto JSON tal como fue aprobado al crear
el snapshot. El sello `SourceSnapshot.binding_digest()` es SHA-256 de esos
bytes UTF-8 crudos, no del `Value` parseado, no de JSON con claves ordenadas y
no de una representación reserializada. El digest de contenido del artefacto
U02 es un dominio adicional: ata el payload inmutable completo, pero no puede
sustituir el sello de bytes usado por U04-B.

## Reglas de consumo

1. Un consumidor que necesita identidad U04-B lee el string
   `payload.source_snapshot_json` del artefacto U02 exacto, lo entrega sin
   transformar a `SourceSnapshot::from_json`, y compara su `binding_digest`
   con el perfil/replay correspondiente.
2. Está prohibido parsear el payload a `serde_json::Value` y reserializarlo
   antes de calcular o comparar ese sello. Cambiar espacios, orden de claves o
   escape de strings cambiaría legítimamente los bytes comprometidos.
3. Si falta el campo, no es string, no se puede parsear/validar como
   `SourceSnapshot`, o el tenant del JSON no coincide con la referencia U02,
   el consumidor falla cerrado. No hay fallback al digest de contenido del
   artefacto, a metadata semejante ni a un snapshot histórico reconstruido.
4. Los snapshots U02 históricos que no preservaron este campo no son
   elegibles para rutas snapshot-bound (U04-B, U08-E, U20/E0). Pueden conservar
   su historial, pero requieren una nueva revisión inmutable que incluya los
   bytes fuente; no se migran ni se reescriben en sitio.

## Alcance y seguimiento

Esta es una aclaración de contrato y no cambia la estructura inicial del
dataset ni muta artefactos existentes. U04-B ya exige V2 y una identidad de
bytes canónica para `from_snapshot`; U20/E0 resuelve el payload persistido y
mantiene ambos dominios de digest separados. Implementaciones posteriores que
introduzcan un adaptador de persistencia deben rechazar escrituras U02
`SourceSnapshot` sin este campo y añadir regresiones para bytes reordenados,
reserialización y snapshots históricos incompletos.

La decisión nació de una auditoría P2 documental. No afirma que se haya hecho
una migración de datos ni que exista un adaptador de producción adicional.
