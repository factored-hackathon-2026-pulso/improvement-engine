# 0005 — I03: PostgreSQL CI autocontenido

## Comportamiento entregado

El job `postgres-artifact-migration` vive en `improvement-engine` y ejecuta
la única prueba PostgreSQL ignorada de U02 contra el digest inmutable de
`postgres:17`, un servicio efímero de GitHub Actions. Conserva checkout fijado,
credenciales persistentes
deshabilitadas, permisos globales mínimos (`contents: read`) y Rust 1.98.1.
La URL y el consentimiento destructivo se declaran sólo en el paso de la
prueba y apuntan al contenedor aislado; no hay secretos de GitHub ni workflow
reutilizable externo.

El adaptador PostgreSQL y la prueba de migración convierten sus UUID de texto
con `$n::text::uuid`. Así el driver enlaza parámetros `TEXT` y PostgreSQL hace
la conversión explícita: se corrige el fallo real donde un `&str` se intentaba
enviar a un parámetro tipado `UUID`.

La primera ejecución real también reveló que la helper directa de la prueba
intentaba enviar `&str` a `$7::jsonb`. Ahora materializa `serde_json::Value`
con `json!({})`, mientras el adaptador ya entrega su `Value` de payload. El
gate PostgreSQL prueba así los límites de binding UUID y JSONB del driver, no
sólo la sintaxis de la migración.

## Límite de propiedad

`improvement-engine` posee sus dependencias de desarrollo e integración:
servicios efímeros de CI, Compose/LocalStack y fixtures. El repositorio
`infra` posee Terraform, infraestructura AWS, despliegues y operación; no es
una dependencia para construir o validar el motor. Este cambio elimina el
acoplamiento a permisos entre repositorios y SHA externo.

## Evidencia de TDD

1. Se transformó primero el contrato Python del caller remoto al contrato de
   un servicio PostgreSQL local. Falló porque `ci.yml` todavía tenía `uses:
   pulso-factored/infra/...`.
2. El job local dejó el contrato verde y bloquea referencias a `infra`,
   secretos, bypasses y jobs opcionales.
3. La prueba de migración ignorada conserva el caso que reprodujo el error de
   binding UUID; su ejecución en GitHub Actions es la evidencia de la base
   PostgreSQL real. Las comprobaciones estructurales no lo sustituyen.

## Limitación

El servicio de CI valida la migración y el adaptador contra PostgreSQL real,
pero no despliega ni prueba recursos AWS. Esos recursos pertenecen a `infra`
y se validarán allí con Terraform y sus propios gates.
