# Contratos de wire de Pulso

Esta carpeta contiene contratos JSON versionados que pertenecen al motor de
automejora. No contiene entidades ejecutables de Agent Core ni datos del banco.

`artifact-envelope.schema.json` define el sobre v1 de una revisión inmutable
propia de Pulso: versión del contrato, tenant opaco, identidad/digest/kind,
payload y referencias de lineage. `artifact-ref.schema.json`,
`source-ref.schema.json` y `source-snapshot.schema.json` fijan las referencias
mínimas hacia revisiones propias y la fuente original readonly. El `payload`
se especializa por `kind` en slices posteriores; ninguno de estos contratos
pretende sustituir schemas de Agent Core.

En SourceSnapshot v1, `file_digest` conserva siempre el SHA-256 del objeto de
fuente. El campo opcional `partition_inventory_digest` agrega un compromiso
distinto para adaptadores que leen particiones; snapshots antiguos sin ese
campo siguen válidos y los consumidores que necesiten particiones deben
rechazarlos si falta.

Las referencias llevan tenant para que los consumidores rechacen lineage entre
tenants antes de persistir. `sources/call_center_interactions.v1.json` es el
único ejemplo original de SourceContract en este corte: declara una vista
readonly y campos usados, sin copiar CSV ni modificar su esquema. Los fixtures
son completamente sintéticos y no contienen PII, credenciales ni datos fuente.

Validación disponible sin dependencias adicionales:

```powershell
python -m unittest discover -s tests -v
python contracts/validate_fixtures.py
```

El segundo comando prueba que el fixture positivo es aceptado y que cada
fixture negativo sigue siendo rechazado. No es una implementación de
JSON-Schema completa: Rust/consumidores posteriores deben validar el schema
publicado además de estas invariantes semánticas. La canonicalización RFC8785/
JCS y el cálculo de los digests no están implementados todavía; los digests de
fixtures sólo ejercitan forma y no acreditan bytes canónicos.
