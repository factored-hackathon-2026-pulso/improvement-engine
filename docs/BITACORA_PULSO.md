
## 2026-10-03T14:51Z UTC - CLAUDE: run-input slots re-exposed as bind_context facts
- Core 1.3.0 keeps run-input slots claimed; resolve_path/rule data read only validated slots. `pulso/bind_context` now returns the stage-declared inputs as tool-origin facts; Flows read `facts.binding.value.*`. Writer Flow draft plan path fixed to `facts.draft_plan.value.content.*` (artifact_get nests content). InvocationContext.inputs added (invoke/service.py passes inv.input, one line). Scout and writer E2E on real Core + PG16 pass.
