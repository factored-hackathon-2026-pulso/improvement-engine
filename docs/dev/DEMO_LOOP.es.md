# Guion de 10 líneas: qué es lo que vas a ver y por qué importa

1. Vas a ver un solo comando que recorre el lazo completo de Pulso en una máquina local: datos agregados, un hallazgo, una propuesta, una prueba y un aviso.
2. Los datos son agregados del banco (celdas con al menos 10 casos, sin ids ni texto libre); cuando el banco no produce una propuesta anunciable, lo decimos y corremos el modo sintético, rotulado como sintético.
3. El sensor busca una celda cuyo problema es persistentemente peor que el resto y se repite en la ventana de replicación; eso es una asociación, nunca una causa.
4. Un agente razona el hallazgo, otro modelo lo verifica de forma independiente, y un tercero escribe el cambio sobre un artefacto real del registro de agent-core (hoy, la plantilla del estado de una PQR).
5. Antes de avisar a nadie, el motor demuestra el cambio: una suite de regresión debe fallar con la base y pasar con el candidato; si no, la propuesta queda interna con su motivo (`not_announced:...`).
6. Solo lo demostrado se entrega a agent-core como borrador (origen `auto_detect`, estado `draft`) con un dossier en español que explica problema, evidencia, qué cambia y qué no se midió.
7. Después se avisa a la plataforma de soporte: una notificación por supervisor. El motor propone; aprobar, publicar y promover sigue siendo decisión humana.
8. Lo que es real: modelos, gateway, agent-core local con agentes reales y la evaluación. Lo que es grabado o simulado: el origen sintético cuando se usa, y los veredictos de resultado (outcome), casi siempre `inconclusive` porque los datos son estacionarios.
9. Lo que no se ejerció: agentes nuevos en producción (falta una credencial admin para los ajustes de release), el enrutamiento desde recepción (seguimiento humano) y los enlaces de evidencia de la plataforma, que son ids opacos y no casos reales.
10. Por qué importa: pasamos de "el tablero muestra un número feo" a "hay un cambio probado, explicado y reversible esperando a una persona", con costo en centavos y sin que el motor apruebe nada.
