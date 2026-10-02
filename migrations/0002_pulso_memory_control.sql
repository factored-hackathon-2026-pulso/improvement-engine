-- U33: scope-specific published-memory heads and immutable revocation overlays.
-- Source tables and U02 artifact revisions remain untouched.

CREATE TABLE IF NOT EXISTS pulso_memory_heads (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    purpose TEXT NOT NULL CHECK (char_length(purpose) BETWEEN 1 AND 128),
    world_ref TEXT NOT NULL CHECK (char_length(world_ref) BETWEEN 1 AND 128),
    campaign_ref TEXT NOT NULL CHECK (char_length(campaign_ref) BETWEEN 1 AND 128),
    protocol_ref TEXT NOT NULL CHECK (char_length(protocol_ref) BETWEEN 1 AND 128),
    partition_ref TEXT NOT NULL CHECK (char_length(partition_ref) BETWEEN 1 AND 128),
    artifact_id UUID NOT NULL,
    artifact_revision BIGINT NOT NULL CHECK (artifact_revision > 0),
    artifact_digest TEXT NOT NULL CHECK (artifact_digest ~ '^sha256:[0-9a-f]{64}$'),
    head_version BIGINT NOT NULL CHECK (head_version > 0),
    PRIMARY KEY (tenant_id, purpose, world_ref, campaign_ref, protocol_ref, partition_ref),
    UNIQUE (tenant_id, artifact_id, artifact_revision, artifact_digest)
);

CREATE TABLE IF NOT EXISTS pulso_memory_tombstones (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    artifact_id UUID NOT NULL,
    artifact_revision BIGINT NOT NULL CHECK (artifact_revision > 0),
    artifact_digest TEXT NOT NULL CHECK (artifact_digest ~ '^sha256:[0-9a-f]{64}$'),
    reason_code TEXT NOT NULL CHECK (char_length(reason_code) BETWEEN 1 AND 128),
    revoked_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, artifact_id, artifact_revision, artifact_digest)
);

CREATE TABLE IF NOT EXISTS pulso_memory_lineage (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    child_artifact_id UUID NOT NULL,
    child_revision BIGINT NOT NULL CHECK (child_revision > 0),
    child_digest TEXT NOT NULL CHECK (child_digest ~ '^sha256:[0-9a-f]{64}$'),
    parent_artifact_id UUID NOT NULL,
    parent_revision BIGINT NOT NULL CHECK (parent_revision > 0),
    parent_digest TEXT NOT NULL CHECK (parent_digest ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY (tenant_id, child_artifact_id, child_revision, child_digest)
);

-- A service-verified U15 result. The control API records this only after
-- validating the pinned grant through its authority boundary; publication can
-- then consume the exact stored receipt instead of caller-supplied pages.
CREATE TABLE IF NOT EXISTS pulso_memory_transform_receipts (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    receipt_digest TEXT NOT NULL CHECK (receipt_digest ~ '^sha256:[0-9a-f]{64}$'),
    purpose TEXT NOT NULL,
    world_ref TEXT NOT NULL,
    campaign_ref TEXT NOT NULL,
    protocol_ref TEXT NOT NULL,
    partition_ref TEXT NOT NULL,
    workspace_ref TEXT NOT NULL,
    run_ref TEXT NOT NULL,
    grant_ref TEXT NOT NULL,
    allowed_at_unix_seconds BIGINT NOT NULL CHECK (allowed_at_unix_seconds >= 0),
    base_artifact_id UUID NOT NULL,
    base_revision BIGINT NOT NULL CHECK (base_revision > 0),
    base_digest TEXT NOT NULL CHECK (base_digest ~ '^sha256:[0-9a-f]{64}$'),
    result_pages JSONB NOT NULL CHECK (jsonb_typeof(result_pages) = 'object'),
    result_digest TEXT NOT NULL CHECK (result_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, receipt_digest)
);

CREATE TABLE IF NOT EXISTS pulso_memory_use_receipts (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    receipt_digest TEXT NOT NULL CHECK (receipt_digest ~ '^sha256:[0-9a-f]{64}$'),
    purpose TEXT NOT NULL,
    world_ref TEXT NOT NULL,
    campaign_ref TEXT NOT NULL,
    protocol_ref TEXT NOT NULL,
    partition_ref TEXT NOT NULL,
    artifact_id UUID NOT NULL,
    artifact_revision BIGINT NOT NULL CHECK (artifact_revision > 0),
    artifact_digest TEXT NOT NULL CHECK (artifact_digest ~ '^sha256:[0-9a-f]{64}$'),
    head_version BIGINT NOT NULL CHECK (head_version > 0),
    run_ref TEXT NOT NULL,
    grant_ref TEXT NOT NULL,
    allowed_at_unix_seconds BIGINT NOT NULL CHECK (allowed_at_unix_seconds >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, receipt_digest)
);

CREATE OR REPLACE FUNCTION pulso_reject_memory_control_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pulso memory audit facts are immutable';
END;
$$;

CREATE OR REPLACE FUNCTION pulso_memory_snapshot_revoked(
    p_tenant_id TEXT, p_artifact_id UUID, p_artifact_revision BIGINT, p_artifact_digest TEXT
)
RETURNS BOOLEAN
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    WITH RECURSIVE lineage(artifact_id, revision, digest) AS (
        SELECT p_artifact_id, p_artifact_revision, p_artifact_digest
        UNION
        SELECT edge.parent_artifact_id, edge.parent_revision, edge.parent_digest
        FROM public.pulso_memory_lineage edge
        JOIN lineage current ON edge.tenant_id = p_tenant_id
            AND edge.child_artifact_id = current.artifact_id
            AND edge.child_revision = current.revision
            AND edge.child_digest = current.digest
    )
    SELECT EXISTS (
        SELECT 1 FROM public.pulso_memory_tombstones tombstone
        JOIN lineage ON lineage.artifact_id = tombstone.artifact_id
            AND lineage.revision = tombstone.artifact_revision
            AND lineage.digest = tombstone.artifact_digest
        WHERE tombstone.tenant_id = p_tenant_id
    );
$$;

DROP TRIGGER IF EXISTS pulso_memory_tombstones_immutable ON pulso_memory_tombstones;
CREATE TRIGGER pulso_memory_tombstones_immutable
    BEFORE UPDATE OR DELETE ON pulso_memory_tombstones
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_memory_control_mutation();

DROP TRIGGER IF EXISTS pulso_memory_lineage_immutable ON pulso_memory_lineage;
CREATE TRIGGER pulso_memory_lineage_immutable
    BEFORE UPDATE OR DELETE ON pulso_memory_lineage
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_memory_control_mutation();

DROP TRIGGER IF EXISTS pulso_memory_use_receipts_immutable ON pulso_memory_use_receipts;
CREATE TRIGGER pulso_memory_use_receipts_immutable
    BEFORE UPDATE OR DELETE ON pulso_memory_use_receipts
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_memory_control_mutation();

DROP TRIGGER IF EXISTS pulso_memory_transform_receipts_immutable ON pulso_memory_transform_receipts;
CREATE TRIGGER pulso_memory_transform_receipts_immutable
    BEFORE UPDATE OR DELETE ON pulso_memory_transform_receipts
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_memory_control_mutation();

CREATE OR REPLACE FUNCTION pulso_seed_memory_head(
    p_tenant_id TEXT, p_purpose TEXT, p_world_ref TEXT, p_campaign_ref TEXT,
    p_protocol_ref TEXT, p_partition_ref TEXT, p_artifact_id UUID,
    p_artifact_revision BIGINT, p_artifact_digest TEXT
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE v_kind TEXT; v_digest TEXT; v_payload JSONB;
BEGIN
    SELECT kind, digest, payload INTO v_kind, v_digest, v_payload
    FROM public.pulso_artifact_revisions
    WHERE tenant_id = p_tenant_id AND artifact_id = p_artifact_id AND revision = p_artifact_revision;
    IF NOT FOUND OR v_kind <> 'memory_wiki' OR v_digest <> p_artifact_digest
       OR jsonb_typeof(v_payload) <> 'object'
       OR v_payload->>'purpose' <> p_purpose
       OR jsonb_typeof(v_payload->'available_at_unix_seconds') <> 'number'
       OR NOT public.pulso_memory_pages_valid(v_payload->'pages') THEN
        RAISE EXCEPTION 'invalid memory seed snapshot' USING ERRCODE = '23503';
    END IF;
    IF public.pulso_memory_snapshot_revoked(
        p_tenant_id, p_artifact_id, p_artifact_revision, p_artifact_digest
    ) THEN
        RAISE EXCEPTION 'memory snapshot revoked' USING ERRCODE = '42501';
    END IF;
    INSERT INTO public.pulso_memory_heads (
        tenant_id, purpose, world_ref, campaign_ref, protocol_ref, partition_ref,
        artifact_id, artifact_revision, artifact_digest, head_version
    ) VALUES (
        p_tenant_id, p_purpose, p_world_ref, p_campaign_ref, p_protocol_ref, p_partition_ref,
        p_artifact_id, p_artifact_revision, p_artifact_digest, 1
    );
END;
$$;

CREATE OR REPLACE FUNCTION pulso_publish_memory_revision(
    p_tenant_id TEXT, p_purpose TEXT, p_world_ref TEXT, p_campaign_ref TEXT,
    p_protocol_ref TEXT, p_partition_ref TEXT, p_expected_head_version BIGINT,
    p_base_artifact_id UUID, p_base_revision BIGINT, p_base_digest TEXT,
    p_transform_receipt_digest TEXT,
    p_next_revision BIGINT, p_next_digest TEXT, p_next_payload JSONB
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE v_head public.pulso_memory_heads%ROWTYPE;
DECLARE v_transform public.pulso_memory_transform_receipts%ROWTYPE;
BEGIN
    SELECT * INTO v_head FROM public.pulso_memory_heads
    WHERE tenant_id = p_tenant_id AND purpose = p_purpose AND world_ref = p_world_ref
      AND campaign_ref = p_campaign_ref AND protocol_ref = p_protocol_ref AND partition_ref = p_partition_ref
    FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'memory head missing' USING ERRCODE = 'P0002'; END IF;
    IF v_head.head_version <> p_expected_head_version THEN
        RAISE EXCEPTION 'memory head conflict' USING ERRCODE = '40001';
    END IF;
    IF v_head.artifact_id <> p_base_artifact_id OR v_head.artifact_revision <> p_base_revision
       OR v_head.artifact_digest <> p_base_digest THEN
        RAISE EXCEPTION 'memory head snapshot mismatch' USING ERRCODE = '40001';
    END IF;
    IF public.pulso_memory_snapshot_revoked(
        p_tenant_id, p_base_artifact_id, p_base_revision, p_base_digest
    ) THEN RAISE EXCEPTION 'memory snapshot revoked' USING ERRCODE = '42501'; END IF;
    SELECT * INTO v_transform FROM public.pulso_memory_transform_receipts
    WHERE tenant_id = p_tenant_id AND receipt_digest = p_transform_receipt_digest;
    IF NOT FOUND
       OR v_transform.purpose <> p_purpose
       OR v_transform.world_ref <> p_world_ref
       OR v_transform.campaign_ref <> p_campaign_ref
       OR v_transform.protocol_ref <> p_protocol_ref
       OR v_transform.partition_ref <> p_partition_ref
       OR v_transform.base_artifact_id <> p_base_artifact_id
       OR v_transform.base_revision <> p_base_revision
       OR v_transform.base_digest <> p_base_digest
       OR p_next_payload <> jsonb_build_object(
           'available_at_unix_seconds', v_transform.allowed_at_unix_seconds,
           'purpose', p_purpose,
           'pages', v_transform.result_pages
       ) THEN
        RAISE EXCEPTION 'transform receipt does not authorize memory publication' USING ERRCODE = '42501';
    END IF;
    PERFORM public.pulso_append_artifact_revision(
        p_tenant_id, p_base_artifact_id, p_base_revision, p_next_revision,
        'memory_wiki', p_next_digest, p_next_payload, NULL, NULL, NULL, NULL
    );
    INSERT INTO public.pulso_memory_lineage (
        tenant_id, child_artifact_id, child_revision, child_digest,
        parent_artifact_id, parent_revision, parent_digest
    ) VALUES (
        p_tenant_id, p_base_artifact_id, p_next_revision, p_next_digest,
        p_base_artifact_id, p_base_revision, p_base_digest
    );
    UPDATE public.pulso_memory_heads SET
        artifact_revision = p_next_revision, artifact_digest = p_next_digest,
        head_version = head_version + 1
    WHERE tenant_id = p_tenant_id AND purpose = p_purpose AND world_ref = p_world_ref
      AND campaign_ref = p_campaign_ref AND protocol_ref = p_protocol_ref AND partition_ref = p_partition_ref;
END;
$$;

CREATE OR REPLACE FUNCTION pulso_memory_pages_valid(p_pages JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE page_count INTEGER := 0; page_path TEXT; page_value JSONB; page_content TEXT;
BEGIN
    IF p_pages IS NULL OR jsonb_typeof(p_pages) <> 'object' THEN RETURN FALSE; END IF;
    FOR page_path, page_value IN SELECT key, value FROM jsonb_each(p_pages) LOOP
        page_count := page_count + 1;
        IF jsonb_typeof(page_value) <> 'string' THEN RETURN FALSE; END IF;
        page_content := trim(both '"' FROM page_value::text);
        IF page_count > 128 OR page_path = '' OR page_path LIKE '/%' OR page_path LIKE '%/' OR page_path LIKE '%//%'
           OR page_path LIKE '%\\%' OR page_path LIKE '%:%'
           OR page_path ~ '(^|/)\.\.?(/|$)'
           OR page_content IS NULL OR char_length(page_content) > 65536
           OR page_content ~* '(password|secret[ _-]?key|social[ _-]?security|\b[0-9]{13,19}\b)'
        THEN RETURN FALSE; END IF;
    END LOOP;
    RETURN page_count > 0;
END;
$$;

CREATE OR REPLACE FUNCTION pulso_record_memory_transform_receipt(
    p_tenant_id TEXT, p_receipt_digest TEXT, p_purpose TEXT, p_world_ref TEXT,
    p_campaign_ref TEXT, p_protocol_ref TEXT, p_partition_ref TEXT,
    p_workspace_ref TEXT, p_run_ref TEXT, p_grant_ref TEXT, p_allowed_at_unix_seconds BIGINT,
    p_base_artifact_id UUID, p_base_revision BIGINT, p_base_digest TEXT,
    p_result_pages JSONB, p_result_digest TEXT
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE v_head public.pulso_memory_heads%ROWTYPE;
DECLARE v_existing public.pulso_memory_transform_receipts%ROWTYPE;
DECLARE v_inserted INTEGER;
BEGIN
    SELECT * INTO v_head FROM public.pulso_memory_heads
    WHERE tenant_id = p_tenant_id AND purpose = p_purpose AND world_ref = p_world_ref
      AND campaign_ref = p_campaign_ref AND protocol_ref = p_protocol_ref AND partition_ref = p_partition_ref
    FOR SHARE;
    IF NOT FOUND OR v_head.artifact_id <> p_base_artifact_id
       OR v_head.artifact_revision <> p_base_revision OR v_head.artifact_digest <> p_base_digest
       OR public.pulso_memory_snapshot_revoked(p_tenant_id, p_base_artifact_id, p_base_revision, p_base_digest)
       OR p_workspace_ref = '' OR p_run_ref = '' OR p_grant_ref = ''
       OR NOT public.pulso_memory_pages_valid(p_result_pages) THEN
        RAISE EXCEPTION 'transform receipt is not bound to a live scoped memory head' USING ERRCODE = '42501';
    END IF;
    SELECT * INTO v_existing FROM public.pulso_memory_transform_receipts
    WHERE tenant_id = p_tenant_id AND receipt_digest = p_receipt_digest;
    IF FOUND THEN
        IF v_existing.purpose = p_purpose AND v_existing.world_ref = p_world_ref
           AND v_existing.campaign_ref = p_campaign_ref AND v_existing.protocol_ref = p_protocol_ref
           AND v_existing.partition_ref = p_partition_ref AND v_existing.workspace_ref = p_workspace_ref
           AND v_existing.run_ref = p_run_ref AND v_existing.grant_ref = p_grant_ref
           AND v_existing.allowed_at_unix_seconds = p_allowed_at_unix_seconds
           AND v_existing.base_artifact_id = p_base_artifact_id AND v_existing.base_revision = p_base_revision
           AND v_existing.base_digest = p_base_digest AND v_existing.result_pages = p_result_pages
           AND v_existing.result_digest = p_result_digest THEN RETURN; END IF;
        RAISE EXCEPTION 'transform receipt idempotency conflict' USING ERRCODE = '23505';
    END IF;
    INSERT INTO public.pulso_memory_transform_receipts (
        tenant_id, receipt_digest, purpose, world_ref, campaign_ref, protocol_ref, partition_ref,
        workspace_ref, run_ref, grant_ref, allowed_at_unix_seconds,
        base_artifact_id, base_revision, base_digest, result_pages, result_digest
    ) VALUES (
        p_tenant_id, p_receipt_digest, p_purpose, p_world_ref, p_campaign_ref, p_protocol_ref, p_partition_ref,
        p_workspace_ref, p_run_ref, p_grant_ref, p_allowed_at_unix_seconds,
        p_base_artifact_id, p_base_revision, p_base_digest, p_result_pages, p_result_digest
    ) ON CONFLICT (tenant_id, receipt_digest) DO NOTHING;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    IF v_inserted = 0 THEN
        SELECT * INTO v_existing FROM public.pulso_memory_transform_receipts
        WHERE tenant_id = p_tenant_id AND receipt_digest = p_receipt_digest;
        IF NOT FOUND OR v_existing.purpose <> p_purpose OR v_existing.world_ref <> p_world_ref
           OR v_existing.campaign_ref <> p_campaign_ref OR v_existing.protocol_ref <> p_protocol_ref
           OR v_existing.partition_ref <> p_partition_ref OR v_existing.workspace_ref <> p_workspace_ref
           OR v_existing.run_ref <> p_run_ref OR v_existing.grant_ref <> p_grant_ref
           OR v_existing.allowed_at_unix_seconds <> p_allowed_at_unix_seconds
           OR v_existing.base_artifact_id <> p_base_artifact_id OR v_existing.base_revision <> p_base_revision
           OR v_existing.base_digest <> p_base_digest OR v_existing.result_pages <> p_result_pages
           OR v_existing.result_digest <> p_result_digest THEN
            RAISE EXCEPTION 'transform receipt idempotency conflict' USING ERRCODE = '23505';
        END IF;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION pulso_revoke_memory_snapshot(
    p_tenant_id TEXT, p_artifact_id UUID, p_artifact_revision BIGINT,
    p_artifact_digest TEXT, p_reason_code TEXT
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM public.pulso_memory_heads
        WHERE tenant_id = p_tenant_id AND artifact_id = p_artifact_id
          AND artifact_revision = p_artifact_revision AND artifact_digest = p_artifact_digest
        UNION ALL
        SELECT 1 FROM public.pulso_memory_lineage
        WHERE tenant_id = p_tenant_id
          AND (child_artifact_id = p_artifact_id AND child_revision = p_artifact_revision AND child_digest = p_artifact_digest
            OR parent_artifact_id = p_artifact_id AND parent_revision = p_artifact_revision AND parent_digest = p_artifact_digest)
    ) THEN
        RAISE EXCEPTION 'memory snapshot is not bound to a memory scope' USING ERRCODE = '23503';
    END IF;
    INSERT INTO public.pulso_memory_tombstones (
        tenant_id, artifact_id, artifact_revision, artifact_digest, reason_code
    ) VALUES (p_tenant_id, p_artifact_id, p_artifact_revision, p_artifact_digest, p_reason_code)
    ON CONFLICT (tenant_id, artifact_id, artifact_revision, artifact_digest) DO NOTHING;
END;
$$;

CREATE OR REPLACE FUNCTION pulso_record_memory_use(
    p_tenant_id TEXT, p_receipt_digest TEXT, p_purpose TEXT, p_world_ref TEXT,
    p_campaign_ref TEXT, p_protocol_ref TEXT, p_partition_ref TEXT,
    p_artifact_id UUID, p_artifact_revision BIGINT, p_artifact_digest TEXT,
    p_expected_head_version BIGINT, p_run_ref TEXT, p_grant_ref TEXT,
    p_allowed_at_unix_seconds BIGINT
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE v_head public.pulso_memory_heads%ROWTYPE;
DECLARE v_existing public.pulso_memory_use_receipts%ROWTYPE;
DECLARE v_inserted INTEGER;
BEGIN
    SELECT * INTO v_head FROM public.pulso_memory_heads
    WHERE tenant_id = p_tenant_id AND purpose = p_purpose AND world_ref = p_world_ref
      AND campaign_ref = p_campaign_ref AND protocol_ref = p_protocol_ref AND partition_ref = p_partition_ref
    FOR SHARE;
    IF NOT FOUND OR v_head.head_version <> p_expected_head_version
       OR v_head.artifact_id <> p_artifact_id OR v_head.artifact_revision <> p_artifact_revision
       OR v_head.artifact_digest <> p_artifact_digest THEN
        RAISE EXCEPTION 'memory use does not match current scoped head' USING ERRCODE = '40001';
    END IF;
    IF public.pulso_memory_snapshot_revoked(
        p_tenant_id, p_artifact_id, p_artifact_revision, p_artifact_digest
    ) THEN RAISE EXCEPTION 'memory snapshot revoked' USING ERRCODE = '42501'; END IF;
    SELECT * INTO v_existing FROM public.pulso_memory_use_receipts
    WHERE tenant_id = p_tenant_id AND receipt_digest = p_receipt_digest;
    IF FOUND THEN
        IF v_existing.purpose = p_purpose AND v_existing.world_ref = p_world_ref
           AND v_existing.campaign_ref = p_campaign_ref AND v_existing.protocol_ref = p_protocol_ref
           AND v_existing.partition_ref = p_partition_ref AND v_existing.artifact_id = p_artifact_id
           AND v_existing.artifact_revision = p_artifact_revision AND v_existing.artifact_digest = p_artifact_digest
           AND v_existing.head_version = p_expected_head_version AND v_existing.run_ref = p_run_ref
           AND v_existing.grant_ref = p_grant_ref AND v_existing.allowed_at_unix_seconds = p_allowed_at_unix_seconds
        THEN RETURN; END IF;
        RAISE EXCEPTION 'memory use receipt idempotency conflict' USING ERRCODE = '23505';
    END IF;
    INSERT INTO public.pulso_memory_use_receipts (
        tenant_id, receipt_digest, purpose, world_ref, campaign_ref, protocol_ref, partition_ref,
        artifact_id, artifact_revision, artifact_digest, head_version, run_ref, grant_ref,
        allowed_at_unix_seconds
    ) VALUES (
        p_tenant_id, p_receipt_digest, p_purpose, p_world_ref, p_campaign_ref, p_protocol_ref, p_partition_ref,
        p_artifact_id, p_artifact_revision, p_artifact_digest, p_expected_head_version, p_run_ref, p_grant_ref,
        p_allowed_at_unix_seconds
    ) ON CONFLICT (tenant_id, receipt_digest) DO NOTHING;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    IF v_inserted = 0 THEN
        SELECT * INTO v_existing FROM public.pulso_memory_use_receipts
        WHERE tenant_id = p_tenant_id AND receipt_digest = p_receipt_digest;
        IF NOT FOUND OR v_existing.purpose <> p_purpose OR v_existing.world_ref <> p_world_ref
           OR v_existing.campaign_ref <> p_campaign_ref OR v_existing.protocol_ref <> p_protocol_ref
           OR v_existing.partition_ref <> p_partition_ref OR v_existing.artifact_id <> p_artifact_id
           OR v_existing.artifact_revision <> p_artifact_revision OR v_existing.artifact_digest <> p_artifact_digest
           OR v_existing.head_version <> p_expected_head_version OR v_existing.run_ref <> p_run_ref
           OR v_existing.grant_ref <> p_grant_ref OR v_existing.allowed_at_unix_seconds <> p_allowed_at_unix_seconds THEN
            RAISE EXCEPTION 'memory use receipt idempotency conflict' USING ERRCODE = '23505';
        END IF;
    END IF;
END;
$$;

REVOKE ALL ON pulso_memory_heads, pulso_memory_tombstones, pulso_memory_lineage,
    pulso_memory_transform_receipts, pulso_memory_use_receipts FROM PUBLIC;
-- U33 functions deliberately remain unavailable to the generic runtime role.
-- The privileged control boundary validates the U05 grant + U15 receipt before
-- invoking them; granting EXECUTE directly would let callers fabricate strings.
REVOKE ALL ON FUNCTION pulso_seed_memory_head(TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, UUID, BIGINT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_publish_memory_revision(TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, BIGINT, TEXT, JSONB) FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_record_memory_transform_receipt(TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, BIGINT, UUID, BIGINT, TEXT, JSONB, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_revoke_memory_snapshot(TEXT, UUID, BIGINT, TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_record_memory_use(TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, UUID, BIGINT, TEXT, BIGINT, TEXT, TEXT, BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_memory_snapshot_revoked(TEXT, UUID, BIGINT, TEXT) FROM PUBLIC;
