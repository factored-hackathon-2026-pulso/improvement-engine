# U34-F — fork de una corrida original

## Decisión

El fork de depuración es un reducer aislado: recibe únicamente `(tenant,
replay_of,idempotency_key,reason)`, localiza un padre previamente registrado y
crea una corrida nueva. No acepta snapshot, configuración, memoria ni cutoff
del operador. Los cuatro valores se copian del padre y quedan fijados junto a
`replay_of`; por tanto un fork no reescribe ni actualiza la evidencia original.

`RunForkStore` es el contrato ejecutable en memoria. El adaptador durable debe
hacer en una sola transacción el lookup de idempotencia, la comprobación de
disponibilidad/revocación y la inserción de corrida hija/recibo. Esta unidad no
afirma que ya existe el endpoint `/fork-replay`, autorización humana, PG ni un
replay E0: corresponden a U24/U34-FE y sus dependencias.

## Invariantes implementados

- La clave de idempotencia se ata al payload completo; la misma solicitud
  devuelve la misma hija, mientras que cambiar motivo o padre se rechaza.
- La inserción sólo ocurre tras encontrar el padre en el mismo tenant y tras
  comprobar snapshot/config/memoria disponibles. Un padre ajeno se ve como
  inexistente para no filtrar tenancy.
- Un snapshot/config/memoria revocado falla cerrado sin dejar corrida hija.
- El hijo conserva cutoff y referencias exactas del padre, incluye
  `replay_of`, y no ofrece API de mutación del padre.

## Evidencia RED → GREEN

1. El test inicial falló con `E0432` al no existir el módulo `run_fork`.
2. Se añadió el reducer mínimo y pasó la creación de hijo inmutable.
3. Se añadieron regresiones para retry/crash idempotente, payload alterado,
   padre desconocido/cross-tenant y referencia revocada.

Comando verificado:

```text
cargo +1.98.1 test -p improvement-engine-core --test run_fork
```

Resultado: 3 pruebas verdes. Antes de integración acumulativa, un revisor
independiente debe comprobar el contrato contra U03/U15/U33 y que el adaptador
durable conserva la atomicidad declarada.
