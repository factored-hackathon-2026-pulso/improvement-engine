# Journal PL-0005 (PL-L4): Sources view

- New hash route `#/sources` (nav "Fuentes"/"Sources"), `src/features/SourcesView.tsx`, demo model `sourcesModel.ts`, strings `src/i18n/sources.ts` (es-419 default + en, key parity tested).
- Everything is a `stand_in` demo model mirroring platform-exporter vocabulary (profile.py capabilities; finding/counter names unknown_event_type, denied_event_type, gap_suspected, late_event). Not read from a real API; banner says so.
- Added a pseudo-capability `event_log` (not in the exporter capability manifest) so operational families are `unsupported` on sources without events. Exporter has no `window_revisions` or `events_exported` counter by those names (PollReport has events_sent, late_events); the demo names are console-side. Reconcile when a real endpoint exists.
- Insight statuses `insufficient_core_target` / reason `no_core_artifact_in_phase1` are consumer proposals, not codes confirmed by Codex.
- View is read-only by construction (no buttons/links). The console has no role/permission pattern yet (session has `scopes` but nothing gates on them).

## Backoffice follow-ups (not done)
- Locale switch: only this view is bilingual; the rest of the console is es-419 only.
- Nav shell is a flat link row; a sectioned backoffice nav with scope-gated entries is needed.
- Role gating from session `scopes` for decision actions; this view needs none.
- Real endpoint for source profile/counters (contract with control-api) to replace the stand_in model.
- No lint script exists in package.json (typecheck/test/build only).
