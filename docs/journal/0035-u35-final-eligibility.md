# U35 — gate final de elegibilidad

U35 añade un gate puramente determinístico entre U20 y el futuro compiler U17.
`FinalEligibilityGate::decide` sólo consume `VerificationReport` (U14),
`WorkflowBridgeContract` (U16) y `EvaluationPlan` sellado (U20). Requiere
verificación `supported`, identidad U14/U16 completa (scope, snapshot,
candidate, provenance, commitment de input y receipt), bridge
`mechanism_proxy` con ruta candidata y plan enlazado al commitment/snapshot del
bridge. Las diferencias se devuelven como razones explícitas y fail-closed.

El resultado `Eligible` contiene únicamente commitments de bridge/plan para
que el builder posterior pueda enlazar el estado correcto. No crea candidato,
registro, ejecución, release ni afirmación de mismo outcome; sus predicados
de release/outcome permanecen falsos.

La primera prueba RED falló porque el módulo y las entidades U35 no existían.
La integración usa únicamente fixtures `cfg(test)` y `pub(crate)` para
componer la tripleta real U14 -> U16 -> U20: no agrega un factory o capability
público. La cobertura comprueba el caso compatible, determinismo sin efectos,
U14 `refuted`/`uncertain`, tenant/scope cruzado, snapshot cruzado y un plan U20
sellado para otro bridge U16. Todas esas incompatibilidades bloquean antes de
que un builder posterior pueda consumir el plan.
