# U29 — evidencia observable de plataforma

Se implementó el sobre tipado de `PlatformObservationBatch`, contratos
confiables de fuente, evidencia tipada de atención y OTel, verificación Core
por puerto, proyección temporal por tenant y persistencia transaccional PG
de eventos, receipts, cursores y blobs tratados por contenido.

La prueba RED inicial importó `platform_observations` inexistente. Rondas
posteriores introdujeron casos RED para fuente desconocida, telemetría sin
diagnóstico, cursor/tenant conflictivos, Core no verificado y corrección
tardía. Los tests de integración PG usan sólo una instancia local efímera y
una base declarada destructible; nunca un banco real.

Decisión: el blob store inline PG está limitado a 256 KiB y falla cerrado;
S3, transporte, verificador Core concreto, purga y U30 no están incluidos.
Consultar `docs/platform-observations.md` para el contrato y estos límites.

La revisión adversarial exigió que un denominador estuviera ligado al
evento/batch exacto y que la barrera PG no dependiera sólo del tenant pasado
al constructor. Se introdujo `ObservedEvent` + `BoundDenominator`, un puerto
de autorización de grant/purpose y FORCE RLS con entitlements por rol. Se
añadieron pruebas con rol PG restringido para tenant ajeno, grant/propósito
incorrectos, mutación de entitlements y ausencia de contexto de sesión.
La mutación directa de filas U29 dentro del tenant autorizado sigue siendo
posible para el rol de runtime que necesita DML; eso no es aislamiento de
escritura por API y queda documentado como límite.

En r3 se detectó que el antiguo mutador público de contratos permitía a un
caller de ingestión alterar la capacidad de cobertura. Se sustituyó por
un registro inmutable inyectado al construir el repositorio. La prueba de
regresión intenta declarar cobertura completa para una fuente OTel cuyo
contrato preinstalado sólo admite muestreo y recibe rechazo. El test PG usa
ahora un login restringido real y comprueba que `SET ROLE` a propietario o
rol ajeno falla; no se interpreta una sesión administradora con `SET ROLE`
como prueba suficiente de membresía.

Estado: revisión adversarial r4 aprobada. Las pruebas locales, el gate
PostgreSQL aislado y las comprobaciones de formato/lint pasaron tras
actualizar la rama con `main`; el PR queda pendiente de CI remoto.
