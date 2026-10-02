# U10 — proveedor externo de modelo gobernado

## Límite y decisión

U10 es el consumidor gobernado del modelo, no un LLM gateway, runtime de
Agent Core ni almacén de secretos. `ModelCapability` fija un proveedor,
endpoint aprobado, modelo, revisión, alias no secreto y referencia opaca a
credencial. `ModelPolicy` fija propósito, timeout, redacción y límites de
unidades/costo. El primer endpoint permitido es OpenRouter
`https://openrouter.ai/api/v1`; no hay llamadas de red o pagadas en pruebas.

## Proyección, presupuesto y egress

El modelo no acepta un texto y un hash proporcionados por un llamador.
`ModelInvocation::from_verified` sólo recibe `VerifiedProjection`, emitida por
`ProjectionBrokerPort`. El broker de referencia de pruebas firma el compromiso
HMAC con tenant, job, grant, authority, policy y proyección tratada. La clave
nunca entra al adapter/receipt; en producción el puerto pertenece a la
autoridad de tratamiento y grants. Por tanto una cadena con forma de HMAC no
puede autorizar PII o una proyección inventada en la frontera del modelo.

`ModelBudgetPort` admite antes de egress y recibe scope, capability, policy,
commitment y límites. La request OpenAI-compatible lleva input tratado,
referencia opaca a credencial, timeout, máximos de output/costo y una key de
idempotencia ligada a tenant/job/grant/authority/attempt/policy/capability.
`ModelReceipt` conserva evidencia, uso/costo medido y settlement, sin prompt,
output, secreto ni provider request id en `Debug`.

La respuesta se rechaza como `BudgetExceededAfterDispatch` si sus unidades o
costo superan el límite, y como `OutputLimitExceededAfterDispatch` si excede el
techo físico de bytes (`max_output_units * 4`); entonces no se conserva digest
del output. El transporte real debe además imponer ese techo al leer el body.
La admisión debe ser idempotente por la identidad del intento: si el proceso
cae entre `admit` y el claim, la recuperación vuelve a obtener la misma
reserva, nunca una segunda. Si el claim devuelve un fallo manejable, U10 libera
la reserva antes de retornar y no hace egress.

## Durabilidad y concurrencia

La identidad es `(tenant_id, job_id, attempt_id)`. La migración
`0002_model_attempt_ledger.sql` crea `pulso_model_attempts` en el control plane
de U06; no cambia tablas fuente. `PostgresModelAttemptRepository` hace
insert/update con CAS `revision`, y `ModelAttemptRepository` permite la misma
costura en pruebas. Antes de egress el adapter persiste `Dispatching`; otro
worker o reinicio ve ese estado y no redespacha. `Unknown` pasa a
`ReconcileBeforeRetry`, no a retry automático. Cada `PreDispatchTransient`
persiste su incremento: el presupuesto de retry no se reinicia tras un crash;
al agotarse libera la reserva. Las transiciones con revisión obsoleta fallan.

## Evidencia TDD y gates

El RED inicial verificó que no existía el módulo. El RED de segunda ronda
introdujo el contrato/migración de intento durable antes de cablear adapter y
broker. Las regresiones cubren proyección tipada ligada a scope, filtración de
secretos/PII, binding de idempotencia, cuotas, retry durable, restart sin
redispatch de `Unknown`, CAS de ledger, y límites de respuesta.

Verificado localmente en Windows:

```powershell
cargo +1.98.1 fmt --all
cargo +1.98.1 test --workspace --features test-support
cargo +1.98.1 clippy -p improvement-engine-core --all-targets --features test-support -- -D warnings
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
git diff --check
```

La prueba PostgreSQL real sigue siendo un gate aislado de CI: requiere
`PULSO_TEST_POSTGRES_URL` y consentimiento destructivo explícito. Antes de
commit/PR falta la re-revisión independiente AI/seguridad.
