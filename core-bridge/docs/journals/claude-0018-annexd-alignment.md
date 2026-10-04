# Journal claude-0018: Annex D alignment of the core-bridge runtime (branch claude/r3-annexd)

- First RED: `core-bridge/tests/annexd/test_service_jwt_claims.py` (9 failed: no `sub=worker:` rule, no job claim check);
  then `test_arm_request_annex.py` (8 failed), `test_admission_annex.py` (collection error: no `derive_context_ref`),
  `test_version_credentials_formats.py` (4 failed). Decisions and kept-runtime items: ADR 0011.
- Pre-existing: bridge-contract was stale after the 894fa65 pin bump (contract.json/openapi, goldens); regenerated through
  `gen.py` and re-recorded.
- Existing tests updated because the wire tightened: tokens now carry `sub=worker:*` and a matching `job_id`
  (tests/runtime/test_tenant_pin, l3a/test_routes, integration/conftest); admission tests use the derived ref,
  Z deadlines and hex digests (l5/test_routes, integration/test_writer); conformance admission tests and goldens.
- bridge_mock needed no change: it already had `schema_version`/`bridge_instance_id`; two new known-different cases
  (MOCK-WORKER-SUB) cover sub/job claims. Mock conformance: 6 passed, 29 skipped, 134 xfailed, no XPASS.
- Infra: throwaway PG16 `pulso-claude-a-pg` on pulso-dev (127.0.0.1:55481, --cgroups=disabled), removed afterwards.
  `jsonschema` was missing from the shared pinned venv and was installed into it.
